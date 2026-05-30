//! `unity-sprite-author watch` — re-pack atlases when their TexturePacker
//! sources change.
//!
//! Watches every `.tps` under `Assets/` **and the source folders/files each
//! one references** (TexturePacker `fileLists`). On a change it re-packs the
//! affected `.tps` (TexturePackerCLI) and re-authors via `pipeline::generate`
//! — see [`crate::author_tps`]. Unity's `TPSheetImporter` independently
//! handles the regenerated `.tpsheet → .asset` step; the `.hash` skip in
//! `pipeline::generate` keeps the two from doing duplicate work.
//!
//! Change detection rides on the shared [`unity_watch`] wire layer (the same
//! crate unity-assetdb's `refresh` uses). We hold one watchman **subscription**
//! — the daemon *pushes* a message the instant a matching file changes, so
//! there is no polling and no interval. The daemon keeps a single OS-level
//! watch on the project tree regardless of how many clients subscribe.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use tps_core::TpsDoc;
use unity_watch::{Change, Filter, WatchError};

use crate::{author_tps, rel};

const WATCH_USAGE: &str = "\
usage: unity-sprite-author watch [project-dir] [options]

  Continuously watches every .tps under Assets/ and the source folders each
  one references via a watchman subscription (push, not polling). On change,
  re-packs with TexturePackerCLI and re-authors the sprite .asset files.
  Unity's TPSheetImporter picks up the .tpsheet.

  project-dir           A path inside the Unity project. Default: cwd. The
                        project root (nearest ancestor with ProjectSettings/)
                        is resolved from it — so running inside a `wt go`
                        worktree watches that worktree's own root.

options:
  --texturepacker <CMD> TexturePackerCLI command. Default: \"texturepacker\".
  -h, --help            Show this help.

  Requires watchman (brew install watchman).
";

/// File suffixes worth re-packing for: the `.tps` itself plus the raster
/// source formats TexturePacker packs from. Output suffixes (`.tpsheet`,
/// `.asset`, the atlas `.png`) are intentionally absent — `.tpsheet` is
/// Unity's lane, and the atlas `.png`/`.asset`/`.hash` we write ourselves
/// are never in a `.tps`'s `fileLists`, so they can't re-trigger a pack.
const SUFFIXES: &[&str] = &[
    "tps", "png", "psd", "tga", "jpg", "jpeg", "tif", "tiff", "gif", "bmp", "exr",
];

pub fn run(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let opts = Opts::parse(args)?;
    if opts.help {
        print!("{WATCH_USAGE}");
        return Ok(());
    }

    // Pin WATCHMAN_SOCK before any tokio runtime / threads exist, so every
    // connect skips the get-sockname fork. Safe: the CLI is single-threaded
    // at this point.
    unity_watch::init_socket_env();

    let start = match &opts.project_dir {
        Some(d) => d
            .canonicalize()
            .map_err(|e| format!("canonicalize {}: {e}", d.display()))?,
        None => std::env::current_dir()?,
    };
    let project_root = unity_assetdb::walk::find_project_root(&start)?
        .canonicalize()
        .map_err(|e| format!("canonicalize project root: {e}"))?;

    let filter = Filter::new(&["Assets"], SUFFIXES);

    // Full root path in the log so it's unambiguous which checkout/worktree
    // is being watched.
    eprintln!("watch: project root {}", project_root.display());

    // Outer loop owns the subscription lifecycle: a server-side cancellation
    // (watch-del / daemon shutdown) or a dropped connection breaks the inner
    // loop and we re-subscribe. Only a first-attempt "watchman missing" is
    // fatal — once we've subscribed, transient drops just retry.
    let mut entries: Vec<TpsEntry>;
    let mut subscribed_once = false;
    loop {
        let mut watcher = match unity_watch::subscribe(&project_root, &filter) {
            Ok(w) => w,
            Err(WatchError::Unavailable) if !subscribed_once => {
                return Err(
                    "watchman unavailable; install it (brew install watchman) to use `watch`"
                        .into(),
                );
            }
            Err(e) => {
                eprintln!("watch: subscribe failed: {e}; retrying in 2s");
                std::thread::sleep(Duration::from_secs(2));
                continue;
            }
        };
        subscribed_once = true;
        entries = discover(&project_root);
        eprintln!("watch: subscribed — tracking {} .tps (Ctrl-C to stop)", entries.len());

        loop {
            match watcher.next() {
                // The first push after subscribing is the fresh-instance
                // snapshot (all current matches). Re-scan the .tps set but
                // don't repack everything — a watch start isn't a rebuild.
                // (The `.hash` skip would make a redundant *generate* cheap,
                // but `author_tps` always re-packs, so authoring every atlas
                // here would spawn TexturePackerCLI per atlas. Use the
                // one-shot CLI for a full catch-up.)
                Ok(Change::Files { is_fresh: true, .. }) => entries = discover(&project_root),
                Ok(Change::Files { paths, is_fresh: false }) => {
                    if !paths.is_empty() {
                        handle_changes(&project_root, &mut entries, &paths, &opts.texturepacker);
                    }
                }
                Ok(Change::Canceled) => {
                    eprintln!("watch: subscription canceled by watchman; resubscribing");
                    break;
                }
                Err(WatchError::Unavailable) => {
                    eprintln!("watch: watchman connection lost; resubscribing");
                    break;
                }
                Err(WatchError::Query(e)) => {
                    eprintln!("watch: {e}; resubscribing");
                    break;
                }
            }
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

/// Map the changed paths to the affected `.tps` set and re-author each. A
/// `.tps` hint re-scans the whole set first so added/removed atlases and
/// edited `fileLists` are reflected before matching.
fn handle_changes(
    project_root: &Path,
    entries: &mut Vec<TpsEntry>,
    hints: &[String],
    texturepacker: &str,
) {
    if hints.iter().any(|h| h.ends_with(".tps")) {
        *entries = discover(project_root);
    }

    let mut affected: Vec<PathBuf> = Vec::new();
    for hint in hints {
        for e in entries.iter() {
            if e.matches(hint) && !affected.contains(&e.tps) {
                affected.push(e.tps.clone());
            }
        }
    }

    for tps in &affected {
        eprintln!("watch: change → repack {}", rel(tps, project_root));
        match author_tps(tps, texturepacker, false, None, None) {
            Ok((out, _)) => eprintln!(
                "watch: {} → written {} deleted {} warnings {}",
                rel(tps, project_root),
                out.written_paths.len(),
                out.deleted_paths.len(),
                out.warnings.len(),
            ),
            Err(e) => eprintln!("watch: error authoring {}: {e}", rel(tps, project_root)),
        }
    }
}

/// One watched `.tps` and the project-relative path prefixes that should
/// trigger a re-pack of it: its own `.tps` path plus every entry in its
/// TexturePacker `fileLists` (source files match exactly; source dirs match
/// any path beneath them).
struct TpsEntry {
    tps: PathBuf,
    refs: Vec<String>,
}

impl TpsEntry {
    fn matches(&self, hint: &str) -> bool {
        self.refs
            .iter()
            .any(|r| hint == r || hint.starts_with(&format!("{r}/")))
    }
}

/// Walk `Assets/` for every `.tps` and read its `fileLists`.
fn discover(project_root: &Path) -> Vec<TpsEntry> {
    let mut tps_paths = Vec::new();
    collect_tps(&project_root.join("Assets"), &mut tps_paths);
    let mut entries = Vec::new();
    for tps in tps_paths {
        match build_entry(project_root, &tps) {
            Ok(e) => entries.push(e),
            Err(e) => eprintln!("watch: skip {}: {e}", rel(&tps, project_root)),
        }
    }
    entries
}

/// Recursively collect `*.tps` files, skipping hidden entries (`.foo`) and
/// Unity-hidden source dirs (`foo~` — these hold the packed PSD/PNG sources,
/// never the `.tps` itself, and can be large).
fn collect_tps(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') {
            continue;
        }
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() {
            if name.ends_with('~') {
                continue;
            }
            collect_tps(&path, out);
        } else if ft.is_file() && path.extension().is_some_and(|e| e == "tps") {
            out.push(path);
        }
    }
}

fn build_entry(project_root: &Path, tps: &Path) -> Result<TpsEntry, String> {
    let tps = tps.canonicalize().map_err(|e| e.to_string())?;
    let tps_dir = tps.parent().ok_or("no parent dir")?.to_path_buf();
    let doc = TpsDoc::load(&tps).map_err(|e| e.to_string())?;
    let file_lists = doc.list_file_lists().map_err(|e| e.to_string())?;

    let mut refs = Vec::new();
    if let Some(r) = relpath(project_root, &tps) {
        refs.push(r);
    }
    for entry in &file_lists {
        let p = Path::new(entry);
        let abs = if p.is_absolute() { p.to_path_buf() } else { tps_dir.join(p) };
        // Canonicalize when the source exists (resolves symlinks so it lines
        // up with the canonical project_root); fall back to the lexical join
        // for a source dir that isn't on disk yet.
        let abs = abs.canonicalize().unwrap_or(abs);
        if let Some(r) = relpath(project_root, &abs) {
            refs.push(r);
        }
    }
    Ok(TpsEntry { tps, refs })
}

/// `path` relative to `root` as a `/`-separated string, or `None` if `path`
/// is outside `root`.
fn relpath(root: &Path, path: &Path) -> Option<String> {
    path.strip_prefix(root)
        .ok()
        .map(|r| r.to_string_lossy().replace('\\', "/"))
}

struct Opts {
    project_dir: Option<PathBuf>,
    texturepacker: String,
    help: bool,
}

impl Opts {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut project_dir = None;
        let mut texturepacker = "texturepacker".to_string();
        let mut help = false;
        let mut iter = args.iter();
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "-h" | "--help" => help = true,
                "--texturepacker" => {
                    texturepacker = iter
                        .next()
                        .ok_or("--texturepacker needs a value")?
                        .clone();
                }
                s if s.starts_with("--") => return Err(format!("unknown flag: {s}")),
                _ => {
                    if project_dir.is_some() {
                        return Err(format!("unexpected positional arg: {arg}"));
                    }
                    project_dir = Some(PathBuf::from(arg));
                }
            }
        }
        Ok(Self { project_dir, texturepacker, help })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(refs: &[&str]) -> TpsEntry {
        TpsEntry {
            tps: PathBuf::from("Assets/Atlas.tps"),
            refs: refs.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn matches_exact_ref() {
        // Own `.tps` and an exact source-file ref both match by equality.
        let e = entry(&["Assets/Atlas.tps", "Assets/src/icon.png"]);
        assert!(e.matches("Assets/Atlas.tps"));
        assert!(e.matches("Assets/src/icon.png"));
    }

    #[test]
    fn matches_dir_prefix() {
        // A source-dir ref matches the dir itself and anything beneath it.
        let e = entry(&["Assets/src"]);
        assert!(e.matches("Assets/src"));
        assert!(e.matches("Assets/src/a.png"));
        assert!(e.matches("Assets/src/sub/b.png"));
    }

    #[test]
    fn matches_rejects_non_prefix() {
        let e = entry(&["Assets/src"]);
        assert!(!e.matches("Assets/src2/a.png")); // sibling dir, not under src/
        assert!(!e.matches("Assets/other.png"));
        // A `.tps.meta` hint must not match the `.tps` ref (exact + `/` only).
        assert!(!entry(&["Assets/Atlas.tps"]).matches("Assets/Atlas.tps.meta"));
    }

    #[test]
    fn relpath_strips_root_or_returns_none() {
        let root = Path::new("/proj");
        assert_eq!(
            relpath(root, Path::new("/proj/Assets/x.png")).as_deref(),
            Some("Assets/x.png"),
        );
        assert_eq!(relpath(root, Path::new("/proj")).as_deref(), Some("")); // root itself
        assert_eq!(relpath(root, Path::new("/other/x.png")), None); // outside the root
    }

    fn args(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn opts_parse_defaults() {
        let o = Opts::parse(&[]).unwrap();
        assert!(o.project_dir.is_none());
        assert_eq!(o.texturepacker, "texturepacker");
        assert!(!o.help);
    }

    #[test]
    fn opts_parse_positional_and_flags() {
        let o = Opts::parse(&args(&["/proj", "--texturepacker", "tp"])).unwrap();
        assert_eq!(o.project_dir, Some(PathBuf::from("/proj")));
        assert_eq!(o.texturepacker, "tp");
        assert!(Opts::parse(&args(&["-h"])).unwrap().help);
    }

    #[test]
    fn opts_parse_errors() {
        assert!(Opts::parse(&args(&["--nope"])).is_err()); // unknown flag
        assert!(Opts::parse(&args(&["--texturepacker"])).is_err()); // missing value
        assert!(Opts::parse(&args(&["a", "b"])).is_err()); // two positionals
    }
}
