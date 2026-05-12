// Parser and validator for `.tps.fab.json` fabrication manifests.
//
// See docs/fab.md for the v1 contract. This module is the JSON → typed-AST
// boundary; downstream modules (combine.rs, pipeline.rs) consume the typed
// `Manifest`. Cross-references against the actual .tpsheet (atlas-uniqueness
// of part names) happen in pipeline integration, not here.

use std::collections::HashSet;
use std::fmt;

// ---------------------------------------------------------------------------
// Public typed AST. JSON shape lives in the `raw` submodule below; we
// validate-and-translate into these types so downstream code never touches
// the raw JSON layer.

#[derive(Debug, PartialEq)]
pub struct Manifest {
    pub combined: Vec<Combined>,
}

#[derive(Debug, PartialEq)]
pub struct Combined {
    pub name: String,
    pub pivot: [f32; 2],
    pub border: [f32; 4],
    /// Global multiplier applied to every part after its per-part affine +
    /// `ui_scale`/`offset` chain. Defaults to 1.0 (identity). For
    /// `CanvasSpriteAuthor.Publish()` reproduction, set to 0.01 — matches
    /// the per-author `_scaleFactor` field. Necessary for byte-exact f32
    /// reproduction of the ×100/×0.01 round-trip Unity does.
    pub canvas_scale: f32,
    /// CanvasSpriteAuthor root's `RectTransform.anchoredPosition` (in canvas
    /// pixels). Defaults to `(0, 0)`. Only matters for reproducing Unity's
    /// `Mesh.CombineMeshes` op chain when the root sits at non-origin: the
    /// per-`CombineInstance` matrix carries an FMA-fused residual through
    /// `m13 = fma(canvas_scale, root + child_offset, -canvas_scale * root)`
    /// that the algebraically-equivalent `offset * canvas_scale` form
    /// doesn't capture. For SpriteRenderer / Box prefabs and any Canvas
    /// hierarchy whose root sits at origin, leave at default.
    pub root_anchored: [f32; 2],
    pub parts: Vec<Part>,
}

#[derive(Debug, PartialEq)]
pub enum Part {
    AtlasSprite {
        sprite: String,
        method: Method,
        /// Target rect, world units. `None` ⇒ native-scale (UIIconMeshGen
        /// path). `Some` ⇒ size-fitted (UISliceMeshGen path). Rejected on
        /// methods that don't accept the opposite shape.
        size: Option<(f32, f32)>,
        /// Target-rect pivot in 0..1. Defaults to `(0.5, 0.5)`. For
        /// SpriteMeshAuthor-tree fixtures (Box prefabs) this stays at the
        /// default; CanvasSpriteAuthor-style hierarchies have per-
        /// RectTransform pivots like `(0, 0.5)` or `(0.5, 0)` that shift
        /// each part's mesh relative to its anchored position.
        part_pivot: [f32; 2],
        /// Slice-method border multiplier. Only used by methods that
        /// declare a border in their source rect; rejected on ID / mirror /
        /// tile (non-MC3) methods at parse time.
        border_mult: f32,
        affine: Affine,
        /// Per-part scale applied AFTER the affine, BEFORE `offset` + the
        /// combined `canvas_scale`. Default `1.0`. For CanvasSpriteAuthor
        /// reproduction, set to `UIIcon._scaleFactor` (typically 100).
        ui_scale: f32,
        /// Per-part canvas-pixel offset applied AFTER `ui_scale`, BEFORE
        /// the combined `canvas_scale`. Default `(0, 0)`. For
        /// CanvasSpriteAuthor, this is the part's
        /// `RectTransform.anchoredPosition`.
        offset: [f32; 2],
    },
    Polygon {
        polygon_sprite: String,
        vertices: Vec<[f32; 2]>,
        /// Optional explicit triangle indices. When absent, the combine
        /// path ear-clips `vertices` via the Triangulator. When present,
        /// this overrides triangulation — useful for matching Unity's
        /// specific index patterns (e.g. UISolid quad: `(0, 2, 3, 3, 1, 0)`).
        triangles: Option<Vec<u16>>,
        affine: Affine,
        /// Canvas-chain transform, same as atlas-sprite parts. Polygons
        /// under CanvasSpriteAuthor (e.g. UISolid backgrounds) need the
        /// matrix-style `v × canvas_scale + offset × canvas_scale` op
        /// order because anchored positions comparable to half-extents
        /// would otherwise round 1+ ULP differently from Unity's emit.
        /// UISolid itself has no per-part scale factor, so `ui_scale`
        /// defaults to `1.0`; SpriteRenderer / Box prefab callers keep
        /// all three at identity.
        ui_scale: f32,
        offset: [f32; 2],
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Affine {
    pub tx: f32,
    pub ty: f32,
    pub sx: f32,
    pub sy: f32,
    pub rot_deg: f32,
}

impl Default for Affine {
    fn default() -> Self {
        Self { tx: 0.0, ty: 0.0, sx: 1.0, sy: 1.0, rot_deg: 0.0 }
    }
}

// Slice/tile/mirror dispatch. Mirrors UISliceMeshGen.cs in meow-tower; see
// docs/fab.md for which constraints fire on which variants.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Method {
    Id,
    Mx, My, Mxy,
    Tx, Ty, TxMc3,
    R1c3, R3c3, R3c3Nf,
    MxR1c3, MxR1c4, MxR3c2, MxR3c3, MxR3c4, MxR3c6,
    MyR2c2, MyR2c3, MyR3c1, MyR3c2, MyR3c3,
    MxyR3c3, MxyR3c3Nf,
}

impl Method {
    /// True iff omitting `width`/`height` is a parse error.
    ///
    /// ID and the mirror duplicators (MX/MY/MXY) work both with and without
    /// a target rect:
    /// - Without size: native scale (`UIIconMeshGen.cs` — SpriteRenderer /
    ///   UIIcon use case). Source verts drawn 1:1, per-vert affine.
    /// - With size: slice-fitted (`UISliceMeshGen.cs` — UISlice use case).
    ///   Source verts stretched to fit the declared target rect.
    ///
    /// Slice grids and tilers always require a target rect — they exist to
    /// fit a region.
    fn requires_size(self) -> bool {
        !matches!(self, Method::Id | Method::Mx | Method::My | Method::Mxy)
    }

    /// True iff the method's mesh-gen math consumes the source sprite's border
    /// (and hence `borderMult` is a meaningful option). ID, the mirror
    /// duplicators (MX/MY/MXY), and the plain tilers (TX/TY) don't have a
    /// border concept; only TX_MC3 and the slice grids do.
    fn uses_border(self) -> bool {
        !matches!(self, Method::Id | Method::Mx | Method::My | Method::Mxy | Method::Tx | Method::Ty)
    }
}

// ---------------------------------------------------------------------------
// Errors.

#[derive(Debug)]
pub enum FabError {
    Json(serde_json::Error),
    UnsupportedVersion(u32),
    EmptyName,
    NameWithSeparator(String),
    DuplicateName(String),
    EmptyParts(String),
    PartShape { combined: String, reason: &'static str },
    UnknownMethod { combined: String, sprite: String, method: String },
    GeometricFlipMethod { combined: String, sprite: String, method: String },
    MissingSize { combined: String, sprite: String, method: Method },
    NonPositiveSize { combined: String, sprite: String, w: f32, h: f32 },
    PolygonTooFewVertices { combined: String, sprite: String, n: usize },
    UnusedOption { combined: String, sprite: String, method: Method, option: &'static str },
}

impl fmt::Display for FabError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(e) => write!(f, "fab.json parse: {e}"),
            Self::UnsupportedVersion(v) => write!(f, "unsupported fab.json version: {v} (expected 1)"),
            Self::EmptyName => write!(f, "combined.name must be non-empty"),
            Self::NameWithSeparator(n) => write!(f, "combined.name must be a bare filename: {n:?}"),
            Self::DuplicateName(n) => write!(f, "duplicate combined.name: {n:?}"),
            Self::EmptyParts(n) => write!(f, "combined {n:?} has no parts"),
            Self::PartShape { combined, reason } => write!(
                f, "combined {combined:?} part: {reason}",
            ),
            Self::UnknownMethod { combined, sprite, method } => write!(
                f, "combined {combined:?} part {sprite:?}: unknown method {method:?}",
            ),
            Self::GeometricFlipMethod { combined, sprite, method } => write!(
                f, "combined {combined:?} part {sprite:?}: method {method:?} is unsupported; \
                    use negative sx/sy for geometric flip",
            ),
            Self::MissingSize { combined, sprite, method } => write!(
                f, "combined {combined:?} part {sprite:?}: method {method} requires width and height",
            ),
            Self::NonPositiveSize { combined, sprite, w, h } => write!(
                f, "combined {combined:?} part {sprite:?}: width and height must be > 0, got ({w}, {h})",
            ),
            Self::PolygonTooFewVertices { combined, sprite, n } => write!(
                f, "combined {combined:?} polygon {sprite:?}: needs ≥ 3 vertices, got {n}",
            ),
            Self::UnusedOption { combined, sprite, method, option } => write!(
                f, "combined {combined:?} part {sprite:?} (method {method}): \
                    option {option:?} is not applicable",
            ),
        }
    }
}

impl fmt::Display for Method {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Method::Id => "ID",
            Method::Mx => "MX", Method::My => "MY", Method::Mxy => "MXY",
            Method::Tx => "TX", Method::Ty => "TY", Method::TxMc3 => "TX_MC3",
            Method::R1c3 => "R1C3", Method::R3c3 => "R3C3", Method::R3c3Nf => "R3C3_NF",
            Method::MxR1c3 => "MX_R1C3", Method::MxR1c4 => "MX_R1C4",
            Method::MxR3c2 => "MX_R3C2", Method::MxR3c3 => "MX_R3C3",
            Method::MxR3c4 => "MX_R3C4", Method::MxR3c6 => "MX_R3C6",
            Method::MyR2c2 => "MY_R2C2", Method::MyR2c3 => "MY_R2C3",
            Method::MyR3c1 => "MY_R3C1", Method::MyR3c2 => "MY_R3C2",
            Method::MyR3c3 => "MY_R3C3",
            Method::MxyR3c3 => "MXY_R3C3", Method::MxyR3c3Nf => "MXY_R3C3_NF",
        };
        f.write_str(s)
    }
}

impl std::error::Error for FabError {}

// ---------------------------------------------------------------------------
// Entry point.

pub fn parse(json: &str) -> Result<Manifest, FabError> {
    let raw: raw::Manifest = serde_json::from_str(json).map_err(FabError::Json)?;
    translate(raw)
}

// ---------------------------------------------------------------------------
// Raw shape — direct mirror of the JSON. Translation enforces all semantic
// rules (default values, exclusivity of sprite vs polygonSprite, method
// constraints, etc.) — serde stays unconfigured for these checks so the
// error messages are uniform and the trigger conditions live in one place.

mod raw {
    use serde::Deserialize;

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct Manifest {
        pub version: u32,
        #[serde(default)]
        pub combined: Vec<Combined>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    pub struct Combined {
        pub name: String,
        pub pivot: Option<[f32; 2]>,
        pub border: Option<[f32; 4]>,
        pub canvas_scale: Option<f32>,
        pub root_anchored: Option<[f32; 2]>,
        #[serde(default)]
        pub parts: Vec<Part>,
    }

    // Both atlas-sprite and polygon shapes share affine fields; serde
    // `untagged` would force exclusive-or between two structs, but the
    // ergonomics for missing-field error messages are poor. Single struct
    // with `Option` discriminators is easier to error-message well.
    // `deny_unknown_fields` so e.g. a stale `mirrorX` from an older schema
    // surfaces as a parse error instead of being silently dropped.
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    pub struct Part {
        // atlas-sprite discriminator
        pub sprite: Option<String>,
        pub method: Option<String>,
        pub width: Option<f32>,
        pub height: Option<f32>,
        pub border_mult: Option<f32>,
        pub part_pivot: Option<[f32; 2]>,
        pub ui_scale: Option<f32>,
        pub offset: Option<[f32; 2]>,

        // polygon discriminator
        pub polygon_sprite: Option<String>,
        pub vertices: Option<Vec<[f32; 2]>>,
        pub triangles: Option<Vec<u16>>,

        // shared affine
        pub tx: Option<f32>,
        pub ty: Option<f32>,
        pub sx: Option<f32>,
        pub sy: Option<f32>,
        pub rot_deg: Option<f32>,
    }
}

fn translate(raw: raw::Manifest) -> Result<Manifest, FabError> {
    if raw.version != 1 {
        return Err(FabError::UnsupportedVersion(raw.version));
    }

    let mut seen_names: HashSet<String> = HashSet::with_capacity(raw.combined.len());
    let mut combined: Vec<Combined> = Vec::with_capacity(raw.combined.len());

    for c in raw.combined {
        if c.name.is_empty() {
            return Err(FabError::EmptyName);
        }
        if c.name.contains('/') || c.name.contains('\\') {
            return Err(FabError::NameWithSeparator(c.name));
        }
        if !seen_names.insert(c.name.clone()) {
            return Err(FabError::DuplicateName(c.name));
        }
        if c.parts.is_empty() {
            return Err(FabError::EmptyParts(c.name));
        }

        let mut parts: Vec<Part> = Vec::with_capacity(c.parts.len());
        for raw_part in c.parts {
            parts.push(translate_part(&c.name, raw_part)?);
        }

        combined.push(Combined {
            name: c.name,
            pivot: c.pivot.unwrap_or([0.5, 0.5]),
            border: c.border.unwrap_or([0.0; 4]),
            canvas_scale: c.canvas_scale.unwrap_or(1.0),
            root_anchored: c.root_anchored.unwrap_or([0.0, 0.0]),
            parts,
        });
    }

    Ok(Manifest { combined })
}

fn translate_part(combined: &str, p: raw::Part) -> Result<Part, FabError> {
    let affine = Affine {
        tx: p.tx.unwrap_or(0.0),
        ty: p.ty.unwrap_or(0.0),
        sx: p.sx.unwrap_or(1.0),
        sy: p.sy.unwrap_or(1.0),
        rot_deg: p.rot_deg.unwrap_or(0.0),
    };

    match (p.sprite, p.polygon_sprite) {
        (Some(_), Some(_)) => Err(FabError::PartShape {
            combined: combined.to_string(),
            reason: "must declare either `sprite` or `polygonSprite`, not both",
        }),
        (None, None) => Err(FabError::PartShape {
            combined: combined.to_string(),
            reason: "must declare either `sprite` or `polygonSprite`",
        }),
        (Some(sprite), None) => {
            let method_str = p.method.as_deref().unwrap_or("ID");
            let method = parse_method(method_str).ok_or_else(|| {
                if matches!(method_str, "FX" | "FY" | "FXY") {
                    FabError::GeometricFlipMethod {
                        combined: combined.to_string(),
                        sprite: sprite.clone(),
                        method: method_str.to_string(),
                    }
                } else {
                    FabError::UnknownMethod {
                        combined: combined.to_string(),
                        sprite: sprite.clone(),
                        method: method_str.to_string(),
                    }
                }
            })?;

            let size = match (p.width, p.height) {
                (Some(w), Some(h)) => {
                    if w <= 0.0 || h <= 0.0 {
                        return Err(FabError::NonPositiveSize {
                            combined: combined.to_string(),
                            sprite,
                            w, h,
                        });
                    }
                    Some((w, h))
                }
                (None, None) => None,
                _ => return Err(FabError::PartShape {
                    combined: combined.to_string(),
                    reason: "width and height must be declared together",
                }),
            };
            if method.requires_size() && size.is_none() {
                return Err(FabError::MissingSize {
                    combined: combined.to_string(),
                    sprite,
                    method,
                });
            }
            // ID never accepts a target rect.
            if matches!(method, Method::Id) && size.is_some() {
                return Err(FabError::UnusedOption {
                    combined: combined.to_string(),
                    sprite,
                    method,
                    option: "width/height (ID method ignores target rect)",
                });
            }
            // borderMult only applies to methods that consume a border:
            // TX_MC3 and the slice grids (R*, MX_*, MY_*, MXY_*).
            if p.border_mult.is_some() && !method.uses_border() {
                return Err(FabError::UnusedOption {
                    combined: combined.to_string(),
                    sprite,
                    method,
                    option: "borderMult (method has no border concept)",
                });
            }

            Ok(Part::AtlasSprite {
                sprite,
                method,
                size,
                part_pivot: p.part_pivot.unwrap_or([0.5, 0.5]),
                border_mult: p.border_mult.unwrap_or(1.0),
                affine,
                ui_scale: p.ui_scale.unwrap_or(1.0),
                offset: p.offset.unwrap_or([0.0, 0.0]),
            })
        }
        (None, Some(polygon_sprite)) => {
            let vertices = p.vertices.ok_or(FabError::PartShape {
                combined: combined.to_string(),
                reason: "polygon part needs `vertices`",
            })?;
            if vertices.len() < 3 {
                return Err(FabError::PolygonTooFewVertices {
                    combined: combined.to_string(),
                    sprite: polygon_sprite,
                    n: vertices.len(),
                });
            }
            // Atlas-sprite-only fields on a polygon part signal a shape
            // mix-up (typo, schema confusion). Reject explicitly so the
            // author sees the issue. `uiScale` / `offset` ARE accepted —
            // polygons under CanvasSpriteAuthor need the same canvas chain
            // as atlas-sprite parts for byte-exact f32 op order.
            if p.method.is_some() || p.width.is_some() || p.height.is_some()
                || p.border_mult.is_some() || p.part_pivot.is_some()
            {
                return Err(FabError::PartShape {
                    combined: combined.to_string(),
                    reason: "polygon parts cannot declare \
                             method/width/height/borderMult/partPivot",
                });
            }
            // Validate explicit triangles when given: must be a multiple of 3
            // and every index must be in range.
            if let Some(tris) = &p.triangles {
                if tris.len() % 3 != 0 {
                    return Err(FabError::PartShape {
                        combined: combined.to_string(),
                        reason: "polygon `triangles` length must be a multiple of 3",
                    });
                }
                let n = vertices.len() as u16;
                if tris.iter().any(|&i| i >= n) {
                    return Err(FabError::PartShape {
                        combined: combined.to_string(),
                        reason: "polygon `triangles` index out of range",
                    });
                }
            }
            Ok(Part::Polygon {
                polygon_sprite,
                vertices,
                triangles: p.triangles,
                affine,
                ui_scale: p.ui_scale.unwrap_or(1.0),
                offset: p.offset.unwrap_or([0.0, 0.0]),
            })
        }
    }
}

fn parse_method(s: &str) -> Option<Method> {
    Some(match s {
        "ID" => Method::Id,
        "MX" => Method::Mx, "MY" => Method::My, "MXY" => Method::Mxy,
        "TX" => Method::Tx, "TY" => Method::Ty, "TX_MC3" => Method::TxMc3,
        "R1C3" => Method::R1c3, "R3C3" => Method::R3c3, "R3C3_NF" => Method::R3c3Nf,
        "MX_R1C3" => Method::MxR1c3, "MX_R1C4" => Method::MxR1c4,
        "MX_R3C2" => Method::MxR3c2, "MX_R3C3" => Method::MxR3c3,
        "MX_R3C4" => Method::MxR3c4, "MX_R3C6" => Method::MxR3c6,
        "MY_R2C2" => Method::MyR2c2, "MY_R2C3" => Method::MyR2c3,
        "MY_R3C1" => Method::MyR3c1, "MY_R3C2" => Method::MyR3c2,
        "MY_R3C3" => Method::MyR3c3,
        "MXY_R3C3" => Method::MxyR3c3, "MXY_R3C3_NF" => Method::MxyR3c3Nf,
        _ => return None,
    })
}

// ===========================================================================
// Tests

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_ok(json: &str) -> Manifest {
        parse(json).unwrap_or_else(|e| panic!("expected ok, got: {e}"))
    }

    fn parse_err(json: &str) -> FabError {
        parse(json).expect_err("expected err")
    }

    #[test]
    fn root_anchored_defaults_to_zero() {
        let m = parse_ok(r#"{
            "version": 1,
            "combined": [{ "name": "BX_Foo", "parts": [{ "sprite": "Body" }] }]
        }"#);
        assert_eq!(m.combined[0].root_anchored, [0.0, 0.0]);
    }

    #[test]
    fn root_anchored_parses_explicit_value() {
        let m = parse_ok(r#"{
            "version": 1,
            "combined": [{
                "name": "Silloutte3",
                "rootAnchored": [141.8, 370.875],
                "parts": [{ "sprite": "Body" }]
            }]
        }"#);
        assert_eq!(m.combined[0].root_anchored, [141.8, 370.875]);
    }

    #[test]
    fn polygon_part_accepts_canvas_chain_fields() {
        // UISolid backgrounds under CanvasSpriteAuthor share the canvas
        // chain with atlas-sprite parts; the parser used to reject these
        // fields on polygons. Regression guard.
        let m = parse_ok(r#"{
            "version": 1,
            "combined": [{
                "name": "X",
                "canvasScale": 0.01,
                "parts": [{
                    "polygonSprite": "Color_FFFFFFFF",
                    "vertices": [[-1, -1], [1, -1], [-1, 1], [1, 1]],
                    "uiScale": 1.0,
                    "offset": [10.5, -20.25]
                }]
            }]
        }"#);
        match &m.combined[0].parts[0] {
            Part::Polygon { ui_scale, offset, .. } => {
                assert_eq!(*ui_scale, 1.0);
                assert_eq!(*offset, [10.5, -20.25]);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn polygon_explicit_triangles_must_be_multiple_of_three() {
        // 4 indices isn't a valid triangle list. The parser must catch this
        // up front rather than letting the combiner emit a half-triangle.
        let err = parse_err(r#"{
            "version": 1,
            "combined": [{
                "name": "X",
                "parts": [{
                    "polygonSprite": "P",
                    "vertices": [[-1, -1], [1, -1], [1, 1], [-1, 1]],
                    "triangles": [0, 1, 2, 3]
                }]
            }]
        }"#);
        assert!(
            matches!(&err, FabError::PartShape { reason, .. }
                if reason.contains("multiple of 3")),
            "{err:?}"
        );
    }

    #[test]
    fn polygon_explicit_triangles_index_in_range() {
        // Indices must reference declared vertices.
        let err = parse_err(r#"{
            "version": 1,
            "combined": [{
                "name": "X",
                "parts": [{
                    "polygonSprite": "P",
                    "vertices": [[-1, -1], [1, -1], [0, 1]],
                    "triangles": [0, 1, 5]
                }]
            }]
        }"#);
        assert!(
            matches!(&err, FabError::PartShape { reason, .. }
                if reason.contains("out of range")),
            "{err:?}"
        );
    }

    #[test]
    fn polygon_explicit_triangles_uisolid_quad_pattern_accepted() {
        // BL/BR/TL/TR + (0, 2, 3, 3, 1, 0) — the exact pattern UISolid
        // emits and that powers Silloutte1/2/3 byte-exactness. Validates
        // the *valid* path the other two tests exercise the *invalid*
        // path of.
        let m = parse_ok(r#"{
            "version": 1,
            "combined": [{
                "name": "X",
                "parts": [{
                    "polygonSprite": "P",
                    "vertices": [[-1, -1], [1, -1], [-1, 1], [1, 1]],
                    "triangles": [0, 2, 3, 3, 1, 0]
                }]
            }]
        }"#);
        match &m.combined[0].parts[0] {
            Part::Polygon { triangles: Some(tris), .. } => {
                assert_eq!(tris, &vec![0, 2, 3, 3, 1, 0]);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn polygon_part_rejects_atlas_sprite_fields() {
        // Atlas-sprite-only fields on a polygon part still signal a typo
        // and must be rejected (uiScale/offset are NOT in this list since
        // polygons now use them too).
        for bad in [
            r#""method": "MX""#,
            r#""width": 10"#,
            r#""height": 10"#,
            r#""borderMult": 2.0"#,
            r#""partPivot": [0, 0]"#,
        ] {
            let json = format!(
                r#"{{
                    "version": 1,
                    "combined": [{{
                        "name": "X",
                        "parts": [{{
                            "polygonSprite": "P",
                            "vertices": [[-1, -1], [1, -1], [0, 1]],
                            {bad}
                        }}]
                    }}]
                }}"#
            );
            let err = parse_err(&json);
            assert!(
                matches!(err, FabError::PartShape { .. }),
                "expected PartShape error for `{bad}`, got: {err:?}"
            );
        }
    }

    #[test]
    fn minimal_id_part() {
        let m = parse_ok(r#"{
            "version": 1,
            "combined": [
                { "name": "BX_Foo", "parts": [{ "sprite": "Body" }] }
            ]
        }"#);
        assert_eq!(m.combined.len(), 1);
        let c = &m.combined[0];
        assert_eq!(c.name, "BX_Foo");
        assert_eq!(c.pivot, [0.5, 0.5]);
        assert_eq!(c.border, [0.0, 0.0, 0.0, 0.0]);
        match &c.parts[0] {
            Part::AtlasSprite { sprite, method, size, part_pivot, affine, border_mult, ui_scale, offset } => {
                assert_eq!(sprite, "Body");
                assert_eq!(*method, Method::Id);
                assert_eq!(*size, None);
                assert_eq!(*part_pivot, [0.5, 0.5]);
                assert_eq!(*affine, Affine::default());
                assert_eq!(*border_mult, 1.0);
                assert_eq!(*ui_scale, 1.0);
                assert_eq!(*offset, [0.0, 0.0]);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn polygon_part() {
        let m = parse_ok(r#"{
            "version": 1,
            "combined": [{
                "name": "BX_Foo",
                "parts": [{
                    "polygonSprite": "Color_3F314EFF",
                    "vertices": [[-0.41,-0.385],[0.41,-0.385],[0.41,0.385],[-0.41,0.385]]
                }]
            }]
        }"#);
        match &m.combined[0].parts[0] {
            Part::Polygon { polygon_sprite, vertices, affine, .. } => {
                assert_eq!(polygon_sprite, "Color_3F314EFF");
                assert_eq!(vertices.len(), 4);
                assert_eq!(*affine, Affine::default());
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn affine_defaults_applied_per_field() {
        let m = parse_ok(r#"{
            "version": 1,
            "combined": [{ "name": "X", "parts": [
                { "sprite": "A", "tx": 1.5, "rotDeg": 45.0 }
            ] }]
        }"#);
        match &m.combined[0].parts[0] {
            Part::AtlasSprite { affine, .. } => {
                assert_eq!(affine.tx, 1.5);
                assert_eq!(affine.ty, 0.0);
                assert_eq!(affine.sx, 1.0);
                assert_eq!(affine.sy, 1.0);
                assert_eq!(affine.rot_deg, 45.0);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn pivot_and_border_overridden() {
        let m = parse_ok(r#"{
            "version": 1,
            "combined": [{
                "name": "X",
                "pivot": [0.0, 1.0],
                "border": [2, 3, 4, 5],
                "parts": [{ "sprite": "A" }]
            }]
        }"#);
        assert_eq!(m.combined[0].pivot, [0.0, 1.0]);
        assert_eq!(m.combined[0].border, [2.0, 3.0, 4.0, 5.0]);
    }

    #[test]
    fn rejects_version_other_than_1() {
        let e = parse_err(r#"{ "version": 2, "combined": [] }"#);
        assert!(matches!(e, FabError::UnsupportedVersion(2)), "got {e:?}");
    }

    #[test]
    fn empty_combined_is_ok() {
        // No fabricated entries is the same as no manifest present.
        let m = parse_ok(r#"{ "version": 1, "combined": [] }"#);
        assert!(m.combined.is_empty());
    }

    #[test]
    fn rejects_empty_parts() {
        let e = parse_err(r#"{ "version": 1, "combined": [{ "name": "X", "parts": [] }] }"#);
        assert!(matches!(e, FabError::EmptyParts(ref n) if n == "X"), "got {e:?}");
    }

    #[test]
    fn rejects_duplicate_combined_name() {
        let e = parse_err(r#"{ "version": 1, "combined": [
            { "name": "X", "parts": [{ "sprite": "A" }] },
            { "name": "X", "parts": [{ "sprite": "B" }] }
        ] }"#);
        assert!(matches!(e, FabError::DuplicateName(ref n) if n == "X"), "got {e:?}");
    }

    #[test]
    fn rejects_empty_name() {
        let e = parse_err(r#"{ "version": 1, "combined": [{ "name": "", "parts": [{ "sprite": "A" }] }] }"#);
        assert!(matches!(e, FabError::EmptyName), "got {e:?}");
    }

    #[test]
    fn part_pivot_defaults_and_overrides() {
        let m = parse_ok(r#"{ "version": 1, "combined": [{ "name": "X", "parts": [
            { "sprite": "A" },
            { "sprite": "B", "partPivot": [0, 0.5] }
        ] }] }"#);
        match &m.combined[0].parts[0] {
            Part::AtlasSprite { part_pivot, .. } => assert_eq!(*part_pivot, [0.5, 0.5]),
            _ => panic!(),
        }
        match &m.combined[0].parts[1] {
            Part::AtlasSprite { part_pivot, .. } => assert_eq!(*part_pivot, [0.0, 0.5]),
            _ => panic!(),
        }
    }

    #[test]
    fn rejects_polygon_with_atlas_sprite_fields() {
        // Mixing an atlas-sprite-only field onto a polygon part is a shape
        // mix-up — surface it instead of silently dropping the field.
        for extra in [r#""method": "ID""#, r#""width": 1, "height": 1"#,
                      r#""borderMult": 0.5"#, r#""partPivot": [0, 0.5]"#] {
            let json = format!(
                r#"{{ "version": 1, "combined": [{{ "name": "X", "parts": [
                    {{ "polygonSprite": "C", "vertices": [[0,0],[1,0],[0,1]], {extra} }}
                ] }}] }}"#
            );
            let e = parse_err(&json);
            assert!(matches!(e, FabError::PartShape { .. }), "got {e:?} for {extra}");
        }
    }

    #[test]
    fn rejects_unknown_fields_anywhere() {
        // deny_unknown_fields catches typos and stale schema fields (e.g.
        // mirrorX/mirrorY, which were removed when no implementation used
        // them). Validates the parser falls in the "fail loud" camp rather
        // than silently dropping options the user wrote.
        for json in [
            // unknown at top level
            r#"{ "version": 1, "combined": [], "extra": true }"#,
            // unknown at combined level
            r#"{ "version": 1, "combined": [{ "name": "X", "parts": [{ "sprite": "A" }], "foo": 1 }] }"#,
            // unknown on a Part (mirrorX is a stale schema example)
            r#"{ "version": 1, "combined": [{ "name": "X", "parts": [{ "sprite": "A", "mirrorX": true }] }] }"#,
        ] {
            let e = parse_err(json);
            assert!(matches!(e, FabError::Json(_)), "got {e:?} for {json}");
        }
    }

    #[test]
    fn rejects_width_height_on_id_method() {
        // ID never takes a target rect; supplying one is an unused option.
        let e = parse_err(r#"{ "version": 1, "combined": [{ "name": "X", "parts": [
            { "sprite": "A", "method": "ID", "width": 1, "height": 1 }
        ] }] }"#);
        assert!(matches!(e, FabError::UnusedOption { method: Method::Id, .. }), "got {e:?}");
    }

    #[test]
    fn rejects_border_mult_on_methods_without_border() {
        for method in ["ID", "MX", "MY", "MXY", "TX", "TY"] {
            let extra = if matches!(method, "ID" | "MX" | "MY" | "MXY") { "" }
                        else { r#", "width": 4, "height": 1"# };
            let json = format!(
                r#"{{ "version": 1, "combined": [{{ "name": "X", "parts": [
                    {{ "sprite": "A", "method": "{method}", "borderMult": 0.5{extra} }}
                ] }}] }}"#
            );
            let e = parse_err(&json);
            assert!(matches!(e, FabError::UnusedOption { .. }), "got {e:?} for {method}");
        }
    }

    #[test]
    fn accepts_border_mult_on_methods_with_border() {
        let m = parse_ok(r#"{ "version": 1, "combined": [{ "name": "X", "parts": [
            { "sprite": "A", "method": "R3C3", "width": 4, "height": 4, "borderMult": 0.5 }
        ] }] }"#);
        match &m.combined[0].parts[0] {
            Part::AtlasSprite { border_mult, .. } => assert_eq!(*border_mult, 0.5),
            _ => panic!(),
        }
    }

    #[test]
    fn rejects_name_with_path_separator() {
        // JSON-source has to escape the backslash so we hand the parser the
        // 3-char string a\b (not the 2-char a-then-backspace).
        for json in [
            r#"{ "version": 1, "combined": [{ "name": "a/b",  "parts": [{ "sprite": "A" }] }] }"#,
            r#"{ "version": 1, "combined": [{ "name": "a\\b", "parts": [{ "sprite": "A" }] }] }"#,
        ] {
            let e = parse_err(json);
            assert!(matches!(e, FabError::NameWithSeparator(_)), "got {e:?} for {json}");
        }
    }

    #[test]
    fn rejects_fx_fy_fxy_with_specific_error() {
        for bad in ["FX", "FY", "FXY"] {
            let json = format!(
                r#"{{ "version": 1, "combined": [{{ "name": "X", "parts": [
                    {{ "sprite": "A", "method": "{bad}", "width": 1, "height": 1 }}
                ] }}] }}"#
            );
            let e = parse_err(&json);
            assert!(
                matches!(&e, FabError::GeometricFlipMethod { method, .. } if method == bad),
                "got {e:?} for {bad}"
            );
        }
    }

    #[test]
    fn rejects_unknown_method() {
        let e = parse_err(r#"{ "version": 1, "combined": [{ "name": "X", "parts": [
            { "sprite": "A", "method": "NONESUCH", "width": 1, "height": 1 }
        ] }] }"#);
        assert!(matches!(&e, FabError::UnknownMethod { method, .. } if method == "NONESUCH"), "got {e:?}");
    }

    #[test]
    fn slice_grid_methods_require_size() {
        // TX/TY/TX_MC3/R*/MX_*/MY_*/MXY_* always need a target rect.
        let e = parse_err(r#"{ "version": 1, "combined": [{ "name": "X", "parts": [
            { "sprite": "A", "method": "TX" }
        ] }] }"#);
        assert!(matches!(e, FabError::MissingSize { .. }), "got {e:?}");
    }

    #[test]
    fn mirror_methods_size_optional() {
        // MX/MY/MXY without size dispatch to UIIconMeshGen-style (native scale).
        // The parser accepts both shapes.
        for method in ["MX", "MY", "MXY"] {
            let json = format!(
                r#"{{ "version": 1, "combined": [{{ "name": "X", "parts": [
                    {{ "sprite": "A", "method": "{method}" }}
                ] }}] }}"#
            );
            let m = parse_ok(&json);
            match &m.combined[0].parts[0] {
                Part::AtlasSprite { size, .. } => assert_eq!(*size, None, "method={method}"),
                _ => panic!(),
            }
        }
    }

    #[test]
    fn rejects_half_specified_size() {
        let e = parse_err(r#"{ "version": 1, "combined": [{ "name": "X", "parts": [
            { "sprite": "A", "width": 1 }
        ] }] }"#);
        assert!(matches!(e, FabError::PartShape { .. }), "got {e:?}");
    }

    #[test]
    fn rejects_non_positive_size() {
        for (w, h) in [(0.0, 1.0), (1.0, 0.0), (-1.0, 1.0)] {
            let json = format!(
                r#"{{ "version": 1, "combined": [{{ "name": "X", "parts": [
                    {{ "sprite": "A", "method": "MX", "width": {w}, "height": {h} }}
                ] }}] }}"#
            );
            let e = parse_err(&json);
            assert!(matches!(e, FabError::NonPositiveSize { .. }), "got {e:?} for ({w},{h})");
        }
    }

    #[test]
    fn rejects_polygon_with_fewer_than_three_verts() {
        let e = parse_err(r#"{ "version": 1, "combined": [{ "name": "X", "parts": [
            { "polygonSprite": "C", "vertices": [[0,0],[1,1]] }
        ] }] }"#);
        assert!(matches!(e, FabError::PolygonTooFewVertices { n: 2, .. }), "got {e:?}");
    }

    #[test]
    fn rejects_part_with_both_sprite_and_polygon() {
        let e = parse_err(r#"{ "version": 1, "combined": [{ "name": "X", "parts": [
            { "sprite": "A", "polygonSprite": "C", "vertices": [[0,0],[1,0],[0,1]] }
        ] }] }"#);
        assert!(matches!(e, FabError::PartShape { .. }), "got {e:?}");
    }

    #[test]
    fn rejects_part_with_neither_sprite_nor_polygon() {
        let e = parse_err(r#"{ "version": 1, "combined": [{ "name": "X", "parts": [
            { "tx": 1.0 }
        ] }] }"#);
        assert!(matches!(e, FabError::PartShape { .. }), "got {e:?}");
    }

    #[test]
    fn all_methods_parse() {
        // Every supported method string in docs/fab.md round-trips through
        // parse_method + Display. Adding a new variant requires updating
        // this list and the Display impl; the test catches half-done edits.
        let pairs: &[(&str, Method)] = &[
            ("ID", Method::Id),
            ("MX", Method::Mx), ("MY", Method::My), ("MXY", Method::Mxy),
            ("TX", Method::Tx), ("TY", Method::Ty), ("TX_MC3", Method::TxMc3),
            ("R1C3", Method::R1c3), ("R3C3", Method::R3c3), ("R3C3_NF", Method::R3c3Nf),
            ("MX_R1C3", Method::MxR1c3), ("MX_R1C4", Method::MxR1c4),
            ("MX_R3C2", Method::MxR3c2), ("MX_R3C3", Method::MxR3c3),
            ("MX_R3C4", Method::MxR3c4), ("MX_R3C6", Method::MxR3c6),
            ("MY_R2C2", Method::MyR2c2), ("MY_R2C3", Method::MyR2c3),
            ("MY_R3C1", Method::MyR3c1), ("MY_R3C2", Method::MyR3c2),
            ("MY_R3C3", Method::MyR3c3),
            ("MXY_R3C3", Method::MxyR3c3), ("MXY_R3C3_NF", Method::MxyR3c3Nf),
        ];
        for (s, m) in pairs {
            assert_eq!(parse_method(s), Some(*m), "parse {s}");
            assert_eq!(format!("{m}"), *s, "display {s}");
        }
    }

    #[test]
    fn parts_order_preserved_verbatim() {
        // Order is significant (docs/fab.md). The parser must not sort,
        // dedup, or reorder.
        let m = parse_ok(r#"{ "version": 1, "combined": [{ "name": "X", "parts": [
            { "sprite": "Z" },
            { "sprite": "A" },
            { "sprite": "Z" }
        ] }] }"#);
        let names: Vec<&str> = m.combined[0]
            .parts
            .iter()
            .map(|p| match p {
                Part::AtlasSprite { sprite, .. } => sprite.as_str(),
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(names, ["Z", "A", "Z"]);
    }
}
