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
    /// Same as `thumb_cache` but with the sprite's polygon geometry masked
    /// in (pixels outside the polygon are transparent). Used by the sprite
    /// picker where users scrutinize each tile.
    pub thumb_clipped_cache: HashMap<String, egui::TextureHandle>,
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
            thumb_clipped_cache: HashMap::new(),
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

    /// Like [`Self::thumbnail`] but masks pixels outside the sprite's
    /// polygon geometry to transparent. Falls back to the unclipped crop
    /// when the sprite has no polygon (geometry is empty), so the picker
    /// always gets a thumbnail.
    pub fn thumbnail_clipped(&mut self, ctx: &egui::Context, name: &str) -> Option<egui::TextureHandle> {
        if let Some(tex) = self.thumb_clipped_cache.get(name) {
            return Some(tex.clone());
        }
        let mut img = self.crop_sprite(name)?;
        let s = self.sprite(name)?;
        let geom = &s.geometry;
        let w = img.size[0];
        let h = img.size[1];
        // Only mask when the sprite actually has polygon geometry — sprites
        // packed without `polygons_enabled` carry a rect-fallback in the
        // tpsheet that would mask nothing out anyway, but we skip explicitly
        // for clarity.
        if !geom.triangles.is_empty() && !geom.vertices.is_empty() {
            apply_polygon_mask(&mut img, &geom.vertices, &geom.triangles, w, h);
        }
        let tex = ctx.load_texture(format!("thumb-clip:{name}"), img, egui::TextureOptions::LINEAR);
        self.thumb_clipped_cache.insert(name.to_string(), tex.clone());
        Some(tex)
    }
}

/// Rasterize the sprite's polygon (rect-local atlas-pixel coords, bottom-up
/// Y) into an alpha mask + zero out crop pixels outside it. We modify in
/// place rather than returning a new image to avoid a second allocation.
fn apply_polygon_mask(
    img: &mut egui::ColorImage,
    vertices: &[unity_sprite_author::tpsheet::Vertex],
    triangles: &[u16],
    w: usize,
    h: usize,
) {
    if w == 0 || h == 0 { return; }
    let mut mask = vec![false; w * h];
    let h_f = h as f32;
    let to_pixel = |v: &unity_sprite_author::tpsheet::Vertex| -> [f32; 2] {
        // Bottom-up tpsheet Y → top-down pixel Y.
        [v.x, h_f - v.y]
    };
    for tri in triangles.chunks(3) {
        if let [a, b, c] = tri {
            let pa = to_pixel(&vertices[*a as usize]);
            let pb = to_pixel(&vertices[*b as usize]);
            let pc = to_pixel(&vertices[*c as usize]);
            rasterize_triangle(&mut mask, w, h, pa, pb, pc);
        }
    }
    for (i, px) in img.pixels.iter_mut().enumerate() {
        if !mask[i] { *px = egui::Color32::TRANSPARENT; }
    }
}

fn rasterize_triangle(mask: &mut [bool], w: usize, h: usize, a: [f32; 2], b: [f32; 2], c: [f32; 2]) {
    let minx = a[0].min(b[0]).min(c[0]).floor().max(0.0) as usize;
    let maxx = a[0].max(b[0]).max(c[0]).ceil().min(w as f32 - 1.0) as usize;
    let miny = a[1].min(b[1]).min(c[1]).floor().max(0.0) as usize;
    let maxy = a[1].max(b[1]).max(c[1]).ceil().min(h as f32 - 1.0) as usize;
    for y in miny..=maxy {
        for x in minx..=maxx {
            let p = [x as f32 + 0.5, y as f32 + 0.5];
            if point_in_triangle(p, a, b, c) {
                mask[y * w + x] = true;
            }
        }
    }
}

fn point_in_triangle(p: [f32; 2], a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> bool {
    let edge = |v0: [f32; 2], v1: [f32; 2], q: [f32; 2]| (q[0] - v0[0]) * (v1[1] - v0[1]) - (q[1] - v0[1]) * (v1[0] - v0[0]);
    let d0 = edge(a, b, p);
    let d1 = edge(b, c, p);
    let d2 = edge(c, a, p);
    let neg = d0 < 0.0 || d1 < 0.0 || d2 < 0.0;
    let pos = d0 > 0.0 || d1 > 0.0 || d2 > 0.0;
    !(neg && pos)
}

#[cfg(test)]
mod tests {
    use super::*;
    use unity_sprite_author::tpsheet::Vertex;

    #[test]
    fn rasterize_triangle_covers_interior() {
        let mut mask = vec![false; 16];
        // Triangle covering the right half of a 4×4 grid.
        rasterize_triangle(&mut mask, 4, 4, [4.0, 0.0], [4.0, 4.0], [0.0, 2.0]);
        // The center pixel (2, 2) is inside.
        assert!(mask[2 * 4 + 2]);
        // Corner (0, 0) is outside.
        assert!(!mask[0]);
        // Corner (3, 0) on the right edge — covered.
        assert!(mask[3]);
    }

    #[test]
    fn polygon_mask_zeros_pixels_outside_polygon() {
        let mut img = egui::ColorImage::new([4, 4], egui::Color32::RED);
        // Right-triangle polygon covering the bottom-right of the rect, in
        // bottom-up tpsheet coords: verts (0,0), (4,0), (4,4) → pixel verts
        // (0,4), (4,4), (4,0). The triangle is the bottom + right edge.
        let verts = vec![Vertex { x: 0.0, y: 0.0 }, Vertex { x: 4.0, y: 0.0 }, Vertex { x: 4.0, y: 4.0 }];
        let tris = vec![0u16, 1, 2];
        apply_polygon_mask(&mut img, &verts, &tris, 4, 4);
        // Top-left pixel (x=0, y=0 in image) is outside the triangle.
        assert_eq!(img.pixels[0], egui::Color32::TRANSPARENT);
        // Bottom-right pixel (x=3, y=3) is inside.
        assert_eq!(img.pixels[3 * 4 + 3], egui::Color32::RED);
    }
}

/// Shell out to TexturePackerCLI in the `.tps`'s parent dir. Command resolved
/// by `usa_pack::texturepacker_cmd` (`$TEXTUREPACKER`, else the platform's
/// canonical install path) — the same source the CLI and bridge watch use.
fn run_texturepacker(tps_path: &Path) -> Result<(), AtlasError> {
    let cmd = usa_pack::texturepacker_cmd();
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
