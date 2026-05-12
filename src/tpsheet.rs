// Mirrors meow-tower/Packages/com.boxcat.libs/TexturePacker/SheetLoader.cs.
// Strict on :format= (C# is lenient; we hard-error per project policy).

use std::fmt;

pub const SUPPORTED_FORMAT: u32 = 40300;

#[derive(Debug, Clone, PartialEq)]
pub struct Sheet {
    pub format: u32,
    pub texture_name: String,
    pub tex: TexInfo,
    pub sprites: Vec<SpriteEntry>,
}

// Mirrors the C# `TexInfo` struct in SheetLoader.cs. Only `width` and
// `height` feed the Sprite `.asset` emit; the rest belong to the texture
// importer side (which our pipeline doesn't touch). Retained so the parser
// is a faithful 1:1 of the C# parse, which matters when diff-checking.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TexInfo {
    pub width: u32,
    pub height: u32,
    pub pivot_points_enabled: bool,
    /// Set to true when any sprite line carries polygon data; not currently
    /// consumed by emit (the per-sprite `Geometry` already encodes this).
    pub polygons_enabled: bool,
    /// Texture-importer flag, not consumed by Sprite emit.
    pub alpha_is_transparency: bool,
}

impl Default for TexInfo {
    fn default() -> Self {
        Self {
            width: 0,
            height: 0,
            pivot_points_enabled: true,
            polygons_enabled: false,
            alpha_is_transparency: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SpriteEntry {
    pub name: String,
    pub rect: Rect,
    pub pivot: Pivot,
    /// Computed via `pivot_to_alignment`. Sprite `.asset` files carry only
    /// the resolved pivot, not an alignment field — this is retained to
    /// mirror the SheetLoader.cs parse surface.
    pub alignment: SpriteAlignment,
    pub border: Border,
    pub geometry: Geometry,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pivot {
    pub x: f32,
    pub y: f32,
}

// Tokens appear in tpsheet order: bL, bR, bT, bB.
// Asset emission order is `{x: L, y: B, z: R, w: T}` per Unity Sprite.cs.
// Signed because real fixtures (e.g. OrgelGallery) carry negative borders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Border {
    pub left: i32,
    pub right: i32,
    pub top: i32,
    pub bottom: i32,
}

// Mirrors UnityEngine.SpriteAlignment. 9 = Custom (used when pivot is non-canonical).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum SpriteAlignment {
    Center = 0,
    TopLeft = 1,
    TopCenter = 2,
    TopRight = 3,
    LeftCenter = 4,
    RightCenter = 5,
    BottomLeft = 6,
    BottomCenter = 7,
    BottomRight = 8,
    Custom = 9,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vertex {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Geometry {
    pub vertices: Vec<Vertex>,
    pub triangles: Vec<u16>, // flat, len = 3 * triangle_count
}

impl Geometry {
    // Rect fallback when tpsheet line carries no polygon data.
    // Matches `SpriteGeometry.Rect` in SheetLoader.cs (same vertex /
    // index layout — 4 corner verts + the `0,2,1,1,2,3` triangle list).
    fn rect(w: u32, h: u32) -> Self {
        let w = w as f32;
        let h = h as f32;
        Self {
            vertices: vec![
                Vertex { x: 0.0, y: 0.0 },
                Vertex { x: w, y: 0.0 },
                Vertex { x: 0.0, y: h },
                Vertex { x: w, y: h },
            ],
            triangles: vec![0, 2, 1, 1, 2, 3],
        }
    }
}

#[derive(Debug)]
pub enum ParseError {
    Empty,
    UnsupportedFormat { found: u32 },
    MissingFormat,
    MalformedHeader { line: usize, content: String },
    MalformedSize { line: usize, value: String },
    ShortSpriteLine { line: usize, expected: usize, got: usize },
    BadNumber { line: usize, token: String, kind: &'static str },
    EmptyName { line: usize },
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "tpsheet is empty"),
            Self::UnsupportedFormat { found } => write!(
                f,
                "unsupported tpsheet format {found}; expected {SUPPORTED_FORMAT}"
            ),
            Self::MissingFormat => write!(f, "tpsheet missing :format= header"),
            Self::MalformedHeader { line, content } => {
                write!(f, "malformed header at line {line}: {content:?}")
            }
            Self::MalformedSize { line, value } => {
                write!(f, "malformed :size= at line {line}: {value:?}")
            }
            Self::ShortSpriteLine { line, expected, got } => write!(
                f,
                "short sprite line at line {line}: expected at least {expected} tokens, got {got}"
            ),
            Self::BadNumber { line, token, kind } => {
                write!(f, "bad {kind} at line {line}: {token:?}")
            }
            Self::EmptyName { line } => write!(f, "empty sprite name at line {line}"),
        }
    }
}

impl std::error::Error for ParseError {}

pub fn parse(input: &str) -> Result<Sheet, ParseError> {
    let lines: Vec<&str> = input.lines().collect();
    if lines.is_empty() {
        return Err(ParseError::Empty);
    }

    let mut i = 0usize;

    // Skip leading '#' comments.
    while i < lines.len() && lines[i].starts_with('#') {
        i += 1;
    }

    // Parse ':key=value' header.
    let mut format: Option<u32> = None;
    let mut texture_name = String::new();
    let mut tex = TexInfo::default();

    while i < lines.len() {
        let line = lines[i];
        if !line.starts_with(':') {
            break;
        }
        let body = &line[1..];
        let eq = body.find('=').ok_or_else(|| ParseError::MalformedHeader {
            line: i + 1,
            content: line.to_string(),
        })?;
        let key = &body[..eq];
        let value = &body[eq + 1..];

        match key {
            "format" => {
                format = Some(value.trim().parse().map_err(|_| ParseError::BadNumber {
                    line: i + 1,
                    token: value.to_string(),
                    kind: "format",
                })?);
            }
            "texture" => texture_name = value.to_string(),
            "size" => {
                let (w, h) = parse_size(value, i + 1)?;
                tex.width = w;
                tex.height = h;
            }
            "pivotpoints" => tex.pivot_points_enabled = is_enabled(value),
            "borders" => { /* ignored, mirrors C# */ }
            "alphahandling" => {
                let v = value.trim_start();
                tex.alpha_is_transparency =
                    !(v.starts_with("KeepTransparentPixels") || v.starts_with("PremultiplyAlpha"));
            }
            _ => { /* unknown header keys are ignored */ }
        }
        i += 1;
    }

    let format = format.ok_or(ParseError::MissingFormat)?;
    if format != SUPPORTED_FORMAT {
        return Err(ParseError::UnsupportedFormat { found: format });
    }

    // Skip blank lines between header and sprites.
    while i < lines.len() && lines[i].trim().is_empty() {
        i += 1;
    }

    // Parse sprite lines.
    let mut sprites = Vec::new();
    while i < lines.len() {
        let line = lines[i];
        if line.trim().is_empty() {
            i += 1;
            continue;
        }
        let entry = parse_sprite_line(line, i + 1, &mut tex)?;
        sprites.push(entry);
        i += 1;
    }

    Ok(Sheet {
        format,
        texture_name,
        tex,
        sprites,
    })
}

fn parse_size(value: &str, line: usize) -> Result<(u32, u32), ParseError> {
    let (w, h) = value.split_once('x').ok_or_else(|| ParseError::MalformedSize {
        line,
        value: value.to_string(),
    })?;
    let w = w.trim().parse().map_err(|_| ParseError::MalformedSize {
        line,
        value: value.to_string(),
    })?;
    let h = h.trim().parse().map_err(|_| ParseError::MalformedSize {
        line,
        value: value.to_string(),
    })?;
    Ok((w, h))
}

fn is_enabled(value: &str) -> bool {
    value.trim_start().starts_with("enabled")
}

fn parse_sprite_line(
    line: &str,
    line_no: usize,
    tex: &mut TexInfo,
) -> Result<SpriteEntry, ParseError> {
    let tokens: Vec<&str> = line.split(';').collect();
    // 11 tokens minimum either way: name + 4 rect + 2 pivot (consumed even when
    // pivotpoints=disabled — the C# side at the `ti += 2; // skip pivot` branch
    // in SheetLoader.cs still advances past the pivot tokens) + 4 border.
    const MIN_TOKENS: usize = 11;
    if tokens.len() < MIN_TOKENS {
        return Err(ParseError::ShortSpriteLine {
            line: line_no,
            expected: MIN_TOKENS,
            got: tokens.len(),
        });
    }

    let mut idx = 0;
    let name = tokens[idx].trim().to_string();
    idx += 1;
    if name.is_empty() {
        return Err(ParseError::EmptyName { line: line_no });
    }

    let rect = Rect {
        x: take_u32(&tokens, &mut idx, line_no, "rect.x")?,
        y: take_u32(&tokens, &mut idx, line_no, "rect.y")?,
        w: take_u32(&tokens, &mut idx, line_no, "rect.w")?,
        h: take_u32(&tokens, &mut idx, line_no, "rect.h")?,
    };

    let pivot = if tex.pivot_points_enabled {
        Pivot {
            x: take_f32(&tokens, &mut idx, line_no, "pivot.x")?,
            y: take_f32(&tokens, &mut idx, line_no, "pivot.y")?,
        }
    } else {
        idx += 2;
        Pivot { x: 0.5, y: 0.5 }
    };

    let alignment = if tex.pivot_points_enabled {
        pivot_to_alignment(pivot)
    } else {
        SpriteAlignment::Center
    };

    let border = Border {
        left: take_i32(&tokens, &mut idx, line_no, "border.L")?,
        right: take_i32(&tokens, &mut idx, line_no, "border.R")?,
        top: take_i32(&tokens, &mut idx, line_no, "border.T")?,
        bottom: take_i32(&tokens, &mut idx, line_no, "border.B")?,
    };

    let geometry = if idx < tokens.len() && !tokens[idx].trim().is_empty() {
        let vert_count = take_usize(&tokens, &mut idx, line_no, "vCount")?;
        let mut vertices = Vec::with_capacity(vert_count);
        for _ in 0..vert_count {
            vertices.push(Vertex {
                x: take_f32(&tokens, &mut idx, line_no, "vertex.x")?,
                y: take_f32(&tokens, &mut idx, line_no, "vertex.y")?,
            });
        }
        let tri_count = take_usize(&tokens, &mut idx, line_no, "triCount")?;
        let mut triangles = Vec::with_capacity(tri_count * 3);
        for _ in 0..(tri_count * 3) {
            triangles.push(take_u16(&tokens, &mut idx, line_no, "triangle")?);
        }
        tex.polygons_enabled = true;
        Geometry {
            vertices,
            triangles,
        }
    } else {
        Geometry::rect(rect.w, rect.h)
    };

    Ok(SpriteEntry {
        name,
        rect,
        pivot,
        alignment,
        border,
        geometry,
    })
}

fn take_u32(
    tokens: &[&str],
    idx: &mut usize,
    line: usize,
    kind: &'static str,
) -> Result<u32, ParseError> {
    let t = tokens.get(*idx).ok_or_else(|| ParseError::ShortSpriteLine {
        line,
        expected: *idx + 1,
        got: tokens.len(),
    })?;
    *idx += 1;
    t.trim().parse().map_err(|_| ParseError::BadNumber {
        line,
        token: t.to_string(),
        kind,
    })
}

fn take_usize(
    tokens: &[&str],
    idx: &mut usize,
    line: usize,
    kind: &'static str,
) -> Result<usize, ParseError> {
    take_u32(tokens, idx, line, kind).map(|n| n as usize)
}

fn take_i32(
    tokens: &[&str],
    idx: &mut usize,
    line: usize,
    kind: &'static str,
) -> Result<i32, ParseError> {
    let t = tokens.get(*idx).ok_or_else(|| ParseError::ShortSpriteLine {
        line,
        expected: *idx + 1,
        got: tokens.len(),
    })?;
    *idx += 1;
    t.trim().parse().map_err(|_| ParseError::BadNumber {
        line,
        token: t.to_string(),
        kind,
    })
}

fn take_u16(
    tokens: &[&str],
    idx: &mut usize,
    line: usize,
    kind: &'static str,
) -> Result<u16, ParseError> {
    let t = tokens.get(*idx).ok_or_else(|| ParseError::ShortSpriteLine {
        line,
        expected: *idx + 1,
        got: tokens.len(),
    })?;
    *idx += 1;
    t.trim().parse().map_err(|_| ParseError::BadNumber {
        line,
        token: t.to_string(),
        kind,
    })
}

fn take_f32(
    tokens: &[&str],
    idx: &mut usize,
    line: usize,
    kind: &'static str,
) -> Result<f32, ParseError> {
    let t = tokens.get(*idx).ok_or_else(|| ParseError::ShortSpriteLine {
        line,
        expected: *idx + 1,
        got: tokens.len(),
    })?;
    *idx += 1;
    t.trim().parse().map_err(|_| ParseError::BadNumber {
        line,
        token: t.to_string(),
        kind,
    })
}

// Mirrors SheetLoader.PivotToAlignment. Float comparisons follow the C# verbatim
// (exact equality, not epsilon). Non-canonical pivots → Custom.
fn pivot_to_alignment(p: Pivot) -> SpriteAlignment {
    let (x, y) = (p.x, p.y);
    if x == 0.0 && y == 0.0 {
        SpriteAlignment::BottomLeft
    } else if x == 0.5 && y == 0.0 {
        SpriteAlignment::BottomCenter
    } else if x == 1.0 && y == 0.0 {
        SpriteAlignment::BottomRight
    } else if x == 0.0 && y == 0.5 {
        SpriteAlignment::LeftCenter
    } else if x == 0.5 && y == 0.5 {
        SpriteAlignment::Center
    } else if x == 1.0 && y == 0.5 {
        SpriteAlignment::RightCenter
    } else if x == 0.0 && y == 1.0 {
        SpriteAlignment::TopLeft
    } else if x == 0.5 && y == 1.0 {
        SpriteAlignment::TopCenter
    } else if x == 1.0 && y == 1.0 {
        SpriteAlignment::TopRight
    } else {
        SpriteAlignment::Custom
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORGEL: &str = include_str!(
        "../tests/golden/orgel/Orgel.tpsheet"
    );

    #[test]
    fn orgel_header() {
        let sheet = parse(ORGEL).expect("parse Orgel.tpsheet");
        assert_eq!(sheet.format, 40300);
        assert_eq!(sheet.texture_name, "Orgel.png");
        assert_eq!(sheet.tex.width, 580);
        assert_eq!(sheet.tex.height, 580);
        assert!(sheet.tex.pivot_points_enabled);
        assert!(sheet.tex.polygons_enabled);
        // alphahandling=PremultiplyAlpha → false
        assert!(!sheet.tex.alpha_is_transparency);
    }

    #[test]
    fn orgel_sprite_count() {
        let sheet = parse(ORGEL).unwrap();
        // Matches the .asset count in the paired Orgel/ folder.
        assert_eq!(sheet.sprites.len(), 62);
    }

    #[test]
    fn cake_decoleft_fields() {
        let sheet = parse(ORGEL).unwrap();
        let s = sheet
            .sprites
            .iter()
            .find(|s| s.name == "Cake__DecoLeft")
            .expect("Cake__DecoLeft present");
        assert_eq!(
            s.rect,
            Rect {
                x: 204,
                y: 556,
                w: 34,
                h: 23,
            }
        );
        assert_eq!(s.pivot, Pivot { x: 0.5, y: 0.5 });
        assert_eq!(s.alignment, SpriteAlignment::Center);
        assert_eq!(s.border, Border::default());
        assert_eq!(s.geometry.vertices.len(), 7);
        // 5 triangles × 3 indices
        assert_eq!(s.geometry.triangles.len(), 15);
        assert_eq!(s.geometry.vertices[0], Vertex { x: 34.0, y: 16.0 });
        assert_eq!(s.geometry.vertices[6], Vertex { x: 19.0, y: 23.0 });
        assert_eq!(s.geometry.triangles[0], 4);
        assert_eq!(s.geometry.triangles[14], 6);
    }

    #[test]
    fn cake_base_polygon_shape() {
        let sheet = parse(ORGEL).unwrap();
        let s = sheet
            .sprites
            .iter()
            .find(|s| s.name == "Cake__Base")
            .unwrap();
        assert_eq!(s.geometry.vertices.len(), 22);
        assert_eq!(s.geometry.triangles.len(), 60);
    }

    #[test]
    fn unsupported_format_rejected() {
        let bad = ":format=99999\n:texture=foo.png\n:size=1x1\n\n";
        match parse(bad).unwrap_err() {
            ParseError::UnsupportedFormat { found: 99999 } => {}
            other => panic!("expected UnsupportedFormat, got {other:?}"),
        }
    }

    #[test]
    fn pivot_to_alignment_table() {
        assert_eq!(
            pivot_to_alignment(Pivot { x: 0.5, y: 0.5 }),
            SpriteAlignment::Center
        );
        assert_eq!(
            pivot_to_alignment(Pivot { x: 0.0, y: 0.0 }),
            SpriteAlignment::BottomLeft
        );
        assert_eq!(
            pivot_to_alignment(Pivot { x: 1.0, y: 1.0 }),
            SpriteAlignment::TopRight
        );
        assert_eq!(
            pivot_to_alignment(Pivot { x: 0.51, y: 0.5 }),
            SpriteAlignment::Custom
        );
    }
}
