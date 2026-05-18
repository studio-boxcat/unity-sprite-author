//! Per-document atlas context: sibling `.tpsheet` (sprite enumeration), `.png`
//! (atlas image for thumbnails + preview), `.tps` (spriteScale), `.png.meta`
//! (PPU). Lazy — open on demand, cache decoded image.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use unity_sprite_author::tps;
use unity_sprite_author::tpsheet::{self, Sheet, SpriteEntry};

pub struct Atlas {
    #[allow(dead_code)]
    pub png_path: PathBuf,
    pub sheet: Sheet,
    pub image: egui::ColorImage,
    /// Sprite name → index into `sheet.sprites`. Built once at load.
    pub by_name: HashMap<String, usize>,
    /// Lazy thumbnail textures, keyed by sprite name.
    pub thumb_cache: HashMap<String, egui::TextureHandle>,
    /// Lazy full-atlas texture for the preview canvas.
    pub atlas_texture: Option<egui::TextureHandle>,
    /// Per-sprite `1 / spriteScale` from the sibling `.tps`. Without this the
    /// preview compresses sprites that TexturePacker downscaled at pack time
    /// (PB_PiggyBank_Closed renders at the wrong absolute size). Missing
    /// entries fall back to 1.0 (the .tps doesn't list spriteScale=1 lines).
    pub invert_scales: HashMap<String, f32>,
    /// PPU read from the sibling `.png.meta` (`spritePixelsToUnits`). Falls
    /// back to 100 if the field is missing — `combine::build_combined` only
    /// uses it to translate vertex world units into atlas-rect-pixel UVs, so
    /// a wrong PPU rescales the whole composite uniformly (visible but not
    /// catastrophic).
    pub ppu: f32,
}

#[derive(Debug)]
pub enum AtlasError {
    MissingTps(PathBuf),
    MissingTpsheetAfterPack { tpsheet: PathBuf },
    MissingPng(PathBuf),
    TpsheetIo(std::io::Error),
    TpsheetParse(tpsheet::ParseError),
    PngDecode(image::ImageError),
    PngIo(std::io::Error),
    /// TexturePackerCLI not in PATH (or whatever `cmd` was used).
    PackSpawn { cmd: String, err: std::io::Error },
    /// TexturePackerCLI exited non-zero.
    PackExit { cmd: String, status: std::process::ExitStatus },
}

impl std::fmt::Display for AtlasError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingTps(p) => write!(f, "missing sibling .tps: {}", p.display()),
            Self::MissingTpsheetAfterPack { tpsheet } => write!(
                f,
                "ran texturepacker but {} still missing — check .tps DataFormat / `--data` settings",
                tpsheet.display()
            ),
            Self::MissingPng(p) => write!(f, "missing sibling .png: {}", p.display()),
            Self::TpsheetIo(e) => write!(f, ".tpsheet read: {e}"),
            Self::TpsheetParse(e) => write!(f, ".tpsheet parse: {e:?}"),
            Self::PngDecode(e) => write!(f, ".png decode: {e}"),
            Self::PngIo(e) => write!(f, ".png read: {e}"),
            Self::PackSpawn { cmd, err } => write!(
                f,
                "couldn't run `{cmd}`: {err} — install TexturePackerCLI or set TEXTUREPACKER",
            ),
            Self::PackExit { cmd, status } => write!(f, "`{cmd}` exited with {status}"),
        }
    }
}

impl Atlas {
    /// Load from `<stem>.tps.fab.json` → derive sibling paths
    /// `<stem>.tps`, `<stem>.tpsheet`, `<stem>.png`. Auto-packs via
    /// TexturePackerCLI when `.tpsheet` is missing (`pipeline::generate`
    /// deletes the `.tpsheet` on successful Unity import, so a checked-in
    /// atlas usually has only `.tps` + `.png` + per-sprite `.asset`s).
    pub fn load_for_fab_json(fab_path: &Path) -> Result<Self, AtlasError> {
        // Strip `.tps.fab.json` to get the atlas stem.
        let stem = fab_path
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.strip_suffix(".tps.fab.json"))
            .unwrap_or_else(|| {
                fab_path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
            });
        let dir = fab_path.parent().unwrap_or(Path::new("."));
        let tps_path = dir.join(format!("{stem}.tps"));
        let tpsheet_path = dir.join(format!("{stem}.tpsheet"));
        let png_path = dir.join(format!("{stem}.png"));
        if !tpsheet_path.exists() {
            if !tps_path.exists() {
                return Err(AtlasError::MissingTps(tps_path));
            }
            run_texturepacker(&tps_path)?;
            if !tpsheet_path.exists() {
                return Err(AtlasError::MissingTpsheetAfterPack { tpsheet: tpsheet_path });
            }
        }
        if !png_path.exists() {
            return Err(AtlasError::MissingPng(png_path));
        }
        let tpsheet_text = fs::read_to_string(&tpsheet_path).map_err(AtlasError::TpsheetIo)?;
        let sheet = tpsheet::parse(&tpsheet_text).map_err(AtlasError::TpsheetParse)?;
        let bytes = fs::read(&png_path).map_err(AtlasError::PngIo)?;
        let img = image::load_from_memory(&bytes).map_err(AtlasError::PngDecode)?;
        let rgba = img.to_rgba8();
        let (w, h) = rgba.dimensions();
        let image = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], rgba.as_raw());

        let by_name = sheet
            .sprites
            .iter()
            .enumerate()
            .map(|(i, s)| (s.name.clone(), i))
            .collect();

        // Per-sprite `1 / spriteScale` (anti-tps scale). Best-effort: an
        // unreadable .tps falls back to empty (every sprite gets 1.0).
        let invert_scales = tps::parse(&tps_path).map(|d| d.invert_scales).unwrap_or_default();

        // PPU from `<stem>.png.meta` (`spritePixelsToUnits: N`). Best-effort
        // string scan — meta.rs doesn't expose this field today, so we read
        // it ourselves.
        let png_meta_path = dir.join(format!("{stem}.png.meta"));
        let ppu = fs::read_to_string(&png_meta_path)
            .ok()
            .and_then(|text| {
                text.lines()
                    .find_map(|line| line.trim().strip_prefix("spritePixelsToUnits:"))
                    .and_then(|v| v.trim().parse::<f32>().ok())
            })
            .unwrap_or(100.0);

        Ok(Self {
            png_path,
            sheet,
            image,
            by_name,
            thumb_cache: HashMap::new(),
            atlas_texture: None,
            invert_scales,
            ppu,
        })
    }

    /// Lazy upload of the full atlas as a `TextureHandle`. Pixels are kept in
    /// PNG order (top-down rows); preview-side rendering compensates by
    /// emitting UVs as `(u, 1.0 - v)` so the bottom-up tpsheet UV convention
    /// renders right-side up.
    pub fn atlas_texture(&mut self, ctx: &egui::Context) -> egui::TextureHandle {
        if let Some(t) = &self.atlas_texture {
            return t.clone();
        }
        let tex = ctx.load_texture("atlas", self.image.clone(), egui::TextureOptions::LINEAR);
        self.atlas_texture = Some(tex.clone());
        tex
    }

    pub fn sprite(&self, name: &str) -> Option<&SpriteEntry> {
        self.by_name.get(name).and_then(|&i| self.sheet.sprites.get(i))
    }

    /// All `Color_*` sprite names (the polygon-color palette for this atlas).
    pub fn color_names(&self) -> impl Iterator<Item = &str> {
        self.sheet
            .sprites
            .iter()
            .map(|s| s.name.as_str())
            .filter(|n| n.starts_with("Color_"))
    }

    /// Crop a sprite's atlas rect into its own `ColorImage`. Used for both
    /// thumbnails (small) and full-size previews. Y-flips because the atlas
    /// .png uses top-down rows while tpsheet rects are bottom-up.
    pub fn crop_sprite(&self, name: &str) -> Option<egui::ColorImage> {
        let s = self.sprite(name)?;
        let atlas_h = self.image.size[1] as u32;
        let atlas_w = self.image.size[0] as u32;
        let rx = s.rect.x.min(atlas_w);
        let rw = s.rect.w.min(atlas_w - rx);
        let rh = s.rect.h.min(atlas_h);
        // Bottom-up rect.y → top-down png row.
        let top_y = atlas_h.saturating_sub(s.rect.y + rh);
        let mut out = egui::ColorImage::new([rw as usize, rh as usize], egui::Color32::TRANSPARENT);
        for row in 0..rh {
            let src_row = (top_y + row) as usize;
            let dst_row = row as usize;
            for col in 0..rw {
                let src = src_row * atlas_w as usize + (rx + col) as usize;
                let dst = dst_row * rw as usize + col as usize;
                out.pixels[dst] = self.image.pixels[src];
            }
        }
        Some(out)
    }

    pub fn thumbnail(&mut self, ctx: &egui::Context, name: &str) -> Option<egui::TextureHandle> {
        if let Some(tex) = self.thumb_cache.get(name) {
            return Some(tex.clone());
        }
        let img = self.crop_sprite(name)?;
        let tex = ctx.load_texture(format!("thumb:{name}"), img, egui::TextureOptions::LINEAR);
        self.thumb_cache.insert(name.to_string(), tex.clone());
        Some(tex)
    }
}

/// Shell out to TexturePackerCLI in the `.tps`'s parent dir. Mirrors the
/// `pack` fn in `unity-sprite-author-cli`. Override the command via the
/// `TEXTUREPACKER` env var (default: `texturepacker`).
fn run_texturepacker(tps_path: &Path) -> Result<(), AtlasError> {
    let cmd = std::env::var("TEXTUREPACKER").unwrap_or_else(|_| "texturepacker".to_string());
    let dir = tps_path.parent().unwrap_or(Path::new("."));
    let name = tps_path.file_name().unwrap_or_default();
    let status = Command::new(&cmd)
        .arg(name)
        .current_dir(dir)
        .status()
        .map_err(|e| AtlasError::PackSpawn { cmd: cmd.clone(), err: e })?;
    if !status.success() {
        return Err(AtlasError::PackExit { cmd, status });
    }
    Ok(())
}
