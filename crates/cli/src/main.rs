//! `unity-sprite-author` CLI — pack one `.tps` with TexturePackerCLI, then
//! run `pipeline::generate` on the result. Missing `.tps.meta` / `.png.meta`
//! are synthesized via `unity-assetdb` so first-time packs outside the
//! Unity Editor don't error on the atlas-GUID read. Missing
//! `Color_*.png` swatches referenced by a sibling `.tps.fab.json` are
//! synthesized into the .tps's source dir before the pack (see
//! `color_synth`).
//!
//! See `CLAUDE.md` for the rlib's contract; this binary is a thin
//! orchestrator that mirrors what `TPSheetPostprocessor.cs` does inside
//! Unity.

mod color_png;
mod color_synth;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::Duration;

use unity_assetdb::register::{self, ImporterKind, RegisterOptions};
use unity_sprite_author::meta::read_tps_prefix;
use unity_sprite_author::pipeline::{self, GenerateInputs, StandardLayout};

const USAGE: &str = "\
usage: unity-sprite-author <atlas.tps> [options]

  Packs the .tps with TexturePackerCLI, then authors Unity Sprite
  .asset files from the resulting .tpsheet. Missing .tps.meta and
  .png.meta are minted via unity-assetdb.

options:
  --prefix <STR>        Sprite filename prefix. Default: TPSImporter
                        `_prefix` from .tps.meta if present, else \"\".
  --sprite-dir <DIR>    Output dir for sprite .asset files. Default:
                        <tps-parent>/<tps-stem>/.
  --skip-pack           Don't run TexturePackerCLI; assume .tpsheet
                        and .png are already up to date.
  --texturepacker <CMD> TexturePackerCLI command. Default: \"texturepacker\".
  -h, --help            Show this help.
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cli = match Cli::parse(&args) {
        Ok(c) => c,
        Err(msg) => {
            eprintln!("{msg}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    if cli.help {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

struct Cli {
    tps_path: PathBuf,
    prefix: Option<String>,
    sprite_dir: Option<PathBuf>,
    skip_pack: bool,
    texturepacker: String,
    help: bool,
}

fn take_value<'a>(
    iter: &mut std::slice::Iter<'a, String>,
    flag: &'static str,
) -> Result<&'a String, String> {
    let v = iter.next().ok_or_else(|| format!("{flag} needs a value"))?;
    if v.starts_with("--") {
        return Err(format!("{flag} expected a value but got `{v}`"));
    }
    Ok(v)
}

impl Cli {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut tps_path: Option<PathBuf> = None;
        let mut prefix: Option<String> = None;
        let mut sprite_dir: Option<PathBuf> = None;
        let mut skip_pack = false;
        let mut texturepacker = "texturepacker".to_string();
        let mut help = false;
        let mut iter = args.iter();
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "-h" | "--help" => help = true,
                "--skip-pack" => skip_pack = true,
                "--prefix" => prefix = Some(take_value(&mut iter, "--prefix")?.clone()),
                "--sprite-dir" => {
                    sprite_dir = Some(PathBuf::from(take_value(&mut iter, "--sprite-dir")?));
                }
                "--texturepacker" => {
                    texturepacker = take_value(&mut iter, "--texturepacker")?.clone();
                }
                s if s.starts_with("--") => return Err(format!("unknown flag: {s}")),
                _ => {
                    if tps_path.is_some() {
                        return Err(format!("unexpected positional arg: {arg}"));
                    }
                    tps_path = Some(PathBuf::from(arg));
                }
            }
        }
        if help {
            return Ok(Self {
                tps_path: PathBuf::new(),
                prefix,
                sprite_dir,
                skip_pack,
                texturepacker,
                help: true,
            });
        }
        Ok(Self {
            tps_path: tps_path.ok_or("missing <atlas.tps> argument")?,
            prefix,
            sprite_dir,
            skip_pack,
            texturepacker,
            help: false,
        })
    }
}

fn run(cli: &Cli) -> Result<(), Box<dyn std::error::Error>> {
    // Canonicalize before TexturePackerCLI runs — texturepacker resolves
    // relative paths in the .tps against its CWD, so we cd into the .tps's
    // dir for the pack step regardless.
    let tps_path = cli
        .tps_path
        .canonicalize()
        .map_err(|e| format!("canonicalize {}: {e}", cli.tps_path.display()))?;
    let layout = StandardLayout::from_tps(&tps_path)?;
    let tps_dir = layout.tps_path.parent().unwrap().to_path_buf();
    let tpsheet_path = layout.tpsheet_path.clone();
    let png_path = layout.atlas_png_path.clone();
    let sprite_dir = cli.sprite_dir.clone().unwrap_or_else(|| layout.sprite_dir.clone());

    // Pre-pack: synthesize any missing `Color_*.png` swatches referenced
    // by a sibling `.tps.fab.json`. No-op when no fab.json exists or all
    // referenced swatches are already on disk.
    if !cli.skip_pack {
        let synth = color_synth::synthesize_for_tps(&tps_path)?;
        for p in &synth.written_paths {
            eprintln!("synth color: {}", rel(p, &tps_dir));
        }
        pack(&cli.texturepacker, &tps_path)?;
    }
    if !tpsheet_path.exists() {
        return Err(format!(
            "{} not produced by pack — does the .tps emit a .tpsheet?",
            tpsheet_path.display()
        )
        .into());
    }
    if !png_path.exists() {
        return Err(format!(
            "{} not produced by pack — does the .tps emit a .png?",
            png_path.display()
        )
        .into());
    }

    let project_root = find_project_root(&tps_dir).ok_or_else(|| {
        format!(
            "no Unity project root above {} (needs ProjectSettings/)",
            tps_dir.display()
        )
    })?;
    let out_dir = project_root.join("Library").join("unity-assetdb");

    ensure_meta(&tps_path, &project_root, &out_dir, None)?;
    ensure_meta(
        &png_path,
        &project_root,
        &out_dir,
        Some(ImporterKind::Texture),
    )?;

    let prefix = match &cli.prefix {
        Some(p) => p.clone(),
        None => read_tps_prefix(&tps_path).unwrap_or_default(),
    };

    std::fs::create_dir_all(&sprite_dir)
        .map_err(|e| format!("create sprite dir {}: {e}", sprite_dir.display()))?;

    eprintln!(
        "authoring sprites: tpsheet={} png={} sprite_dir={} prefix={:?}",
        rel(&tpsheet_path, &project_root),
        rel(&png_path, &project_root),
        rel(&sprite_dir, &project_root),
        prefix,
    );
    let out = pipeline::generate(&GenerateInputs {
        tpsheet_path: &tpsheet_path,
        tps_path: &tps_path,
        atlas_png_path: &png_path,
        sprite_dir: &sprite_dir,
        prefix: &prefix,
    })?;

    eprintln!(
        "written: {}  deleted: {}  warnings: {}",
        out.written_paths.len(),
        out.deleted_paths.len(),
        out.warnings.len()
    );
    for p in &out.written_paths {
        println!("W\t{}", rel(p, &project_root));
    }
    for p in &out.deleted_paths {
        println!("D\t{}", rel(p, &project_root));
    }
    for w in &out.warnings {
        eprintln!("warn: {w}");
    }
    Ok(())
}

fn pack(cmd: &str, tps: &Path) -> Result<(), String> {
    let dir = tps.parent().expect("tps has parent (canonicalized)");
    let name = tps.file_name().expect("tps has filename");
    let status = Command::new(cmd)
        .arg(name)
        .current_dir(dir)
        .status()
        .map_err(|e| format!("spawn `{cmd} {}`: {e}", name.to_string_lossy()))?;
    if !status.success() {
        return Err(format!(
            "`{cmd} {}` exited with {status}",
            name.to_string_lossy()
        ));
    }
    Ok(())
}

fn ensure_meta(
    asset: &Path,
    project_root: &Path,
    out_dir: &Path,
    importer: Option<ImporterKind>,
) -> Result<(), String> {
    let outcome = register::register(&RegisterOptions {
        project_root: project_root.to_path_buf(),
        out_dir: out_dir.to_path_buf(),
        target: asset.to_path_buf(),
        importer_override: importer,
        lock_timeout: Duration::from_secs(10),
    })
    .map_err(|e| format!("register {}: {e}", asset.display()))?;
    if outcome.created_meta {
        eprintln!(
            "minted .meta for {} (guid={:032x})",
            rel(asset, project_root),
            outcome.guid
        );
    }
    Ok(())
}

/// Walk up `start` until a directory containing `ProjectSettings/` is
/// found. `Assets/` is implied — we're walking up from a `.tps` already
/// inside it. `unity_assetdb::walk::resolve_project_root` with `Some(p)`
/// requires `p` to already be the project root, so we do the climb here.
fn find_project_root(start: &Path) -> Option<PathBuf> {
    let mut cur = Some(start);
    while let Some(p) = cur {
        if p.join("ProjectSettings").is_dir() {
            return Some(p.to_path_buf());
        }
        cur = p.parent();
    }
    None
}

fn rel<'a>(p: &'a Path, root: &Path) -> std::borrow::Cow<'a, str> {
    p.strip_prefix(root)
        .map(|r| r.to_string_lossy().into_owned().into())
        .unwrap_or_else(|_| p.to_string_lossy())
}
