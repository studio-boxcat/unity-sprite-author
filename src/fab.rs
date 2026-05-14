// Typed IR for fabricated combined sprites.
//
// This module is now purely the typed AST; the v1 JSON parser was retired
// in favor of the v3 unified manifest (`crate::manifest`), which bridges
// directly to the `Combined` / `Part` types here via `to_fab_combined`.
// Downstream consumers (`combine`, `pipeline`) read this IR.

use std::fmt;

/// Parsed manifest: one per atlas. Lists fabricated combined sprites the
/// pipeline emits alongside per-tpsheet outputs. Produced by
/// `manifest::to_fab_combined` over each CSA tree.
#[derive(Debug, PartialEq)]
pub struct Manifest {
    pub combined: Vec<Combined>,
}

/// A single fabricated combined sprite. `name` becomes the output filename
/// stem (no `.asset`); `parts` are stitched in declared order into one mesh.
#[derive(Debug, PartialEq)]
pub struct Combined {
    pub name: String,
    pub pivot: [f32; 2],
    pub border: [f32; 4],
    /// Global multiplier applied to every part after its per-part affine +
    /// `ui_scale` + `offset` chain. `1.0` for SpriteRenderer / Box prefabs;
    /// `0.01` for `CanvasSpriteAuthor.Publish()` to undo the per-part `100×`.
    pub canvas_scale: f32,
    pub parts: Vec<Part>,
}

#[derive(Debug, PartialEq)]
pub enum Part {
    AtlasSprite {
        sprite: String,
        method: Method,
        /// Target rect, world units. `None` ⇒ native-scale (UIIconMeshGen
        /// path). `Some` ⇒ size-fitted (UISliceMeshGen path).
        size: Option<(f32, f32)>,
        /// Target-rect pivot in 0..1. Defaults to `(0.5, 0.5)`.
        part_pivot: [f32; 2],
        /// Slice-method border multiplier. Only meaningful for methods that
        /// declare a border in their source rect.
        border_mult: f32,
        affine: Affine,
        /// Per-part scale applied AFTER the affine, BEFORE `offset` + the
        /// combined `canvas_scale`. `1.0` default; `UIIcon._scaleFactor`
        /// (typically `100`) for CanvasSpriteAuthor reproduction.
        ui_scale: f32,
        /// Per-part canvas-pixel offset applied AFTER `ui_scale`, BEFORE
        /// the combined `canvas_scale`. For CanvasSpriteAuthor this is the
        /// part's `RectTransform.anchoredPosition`.
        offset: [f32; 2],
    },
    Polygon {
        polygon_sprite: String,
        vertices: Vec<[f32; 2]>,
        /// Optional explicit triangle indices. When absent, the combine path
        /// ear-clips `vertices`. When present, this overrides triangulation —
        /// e.g. UISolid quad: `(0, 2, 3, 3, 1, 0)`.
        triangles: Option<Vec<u16>>,
        affine: Affine,
        /// UISolid has no per-part scale (`ui_scale = 1`); SpriteRenderer /
        /// Box prefab callers leave both `ui_scale` and `offset` at identity.
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

/// Slice / tile / mirror dispatch for [`Part::AtlasSprite`]. Mirrors the
/// methods in `UISliceMeshGen.cs` and `Tiling.cs` (meow-tower); see
/// `combine::atlas_sprite_mesh` for the dispatch table.
///
/// Naming convention: `Id` = identity, `Mx/My/Mxy` = mirror duplicators,
/// `Tx/Ty` = tilers, `R<rows>c<cols>` = slice grids, `Nf` suffix = no
/// centre-fill, `Mx_R<...>` / `My_R<...>` = mirrored slice grids.
/// Geometric flips (FX/FY/FXY) aren't methods — express them as negative
/// `sx` / `sy` on the affine instead.
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
    /// True iff omitting `width`/`height` is a parse error. Slice grids and
    /// tilers always require a target rect; ID + mirror duplicators work
    /// both native-scale (no size) and size-fitted.
    pub fn requires_size(self) -> bool {
        !matches!(self, Method::Id | Method::Mx | Method::My | Method::Mxy)
    }

    /// True iff the method's mesh-gen math consumes the source sprite's border
    /// (and hence `borderMult` is a meaningful option).
    pub fn uses_border(self) -> bool {
        !matches!(self, Method::Id | Method::Mx | Method::My | Method::Mxy | Method::Tx | Method::Ty)
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
