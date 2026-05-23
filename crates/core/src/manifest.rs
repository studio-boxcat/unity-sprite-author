// Unified `.tps.fab.json` manifest — the single supported schema.
//
// Tree-shaped, mirroring pspec (`tools/pspec` in meow-tower). Each entry in
// `combined[]` is one authored output (a CSA-published Sprite or an SMA-
// published Mesh); children are GameObjects whose transforms compose down
// the tree. `parse` → `to_fab_combined` / `to_mesh_combined` lowers into
// the runtime IR (`fab::Combined` / `mesh_manifest::MeshCombined`).
//
// Schema, field tables, and the per-part transform formula live in
// [[fab.md]]. The "why this collapse exists" rationale is in
// [[fab.md#per-part-transform]] — don't duplicate here.
//
// > **Related:** [[fab.md]], [[sma-migration.md]]

use std::collections::HashSet;
use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub struct Manifest {
    /// One entry per authored output sprite/mesh (CSA Sprite or SMA Mesh).
    /// Field is `combined` in JSON — each entry is a "combined" of multiple
    /// atlas parts. The in-memory `Combined` name doubles as the runtime IR
    /// type elsewhere (`fab::Combined`), so the manifest layer wraps it in
    /// `manifest::Tree` to stay focused on the authoring shape.
    pub trees: Vec<Tree>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tree {
    pub name: String,
    pub output: Output,
    pub root: Node,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Output {
    /// CSA-published Sprite. Output path / GUID are derived from the
    /// `_sprite` reference at migration time, not declared here.
    Csa,
    /// SMA-published Mesh. Output path + Mesh sub-asset fileId + canvas-vs-
    /// sprite-renderer layout declared explicitly.
    Sma {
        file_id: i64,
        output_path: String,
        used_in_canvas: bool,
        keep_vertices: bool,
        keep_indices: bool,
    },
}

impl Output {
    /// Mode-implicit canvas factor applied at the bridge seam (`to_fab_combined`
    /// / `to_mesh_combined`) to translate canvas-pixel `pos` into world units.
    /// Single source of truth — keeps the c23474b2 "silent default" class of
    /// regressions impossible (no scattered `match mode { … 0.01 … }` literals).
    pub fn canvas_scale_implicit(&self) -> f32 {
        match self {
            Output::Csa => 0.01,
            Output::Sma { .. } => 1.0,
        }
    }
}

impl Manifest {
    /// Sorted, deduplicated list of every `Color_*` tpsheet entry name
    /// referenced by a polygon leaf across all trees. Used by the CLI's
    /// pre-pack step to synthesize missing 1×1 color PNGs into the
    /// TexturePacker source dir.
    pub fn polygon_sprite_names(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for tree in &self.trees {
            collect_polygon_names(&tree.root, &mut out);
        }
        out.sort();
        out.dedup();
        out
    }
}

fn collect_polygon_names(node: &Node, out: &mut Vec<String>) {
    if let Some(Graphic::Polygon { polygon_sprite, .. }) = &node.graphic {
        out.push(polygon_sprite.clone());
    }
    for c in &node.children {
        collect_polygon_names(c, out);
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub name: String,
    pub pos: [f32; 2],
    /// `None` for non-size-respecting leaves. For sprite leaves with a
    /// size-fitted method, `None` ⇒ default to the sprite's natural rect
    /// size at `combine::build_combined`. SpriteRenderer + tiled draw mode
    /// also stores its world-unit draw rect here.
    pub size: Option<[f32; 2]>,
    /// `None` ⇒ defer to the runtime default. For sprite leaves the bridge
    /// passes this through as `Part::AtlasSprite.part_pivot = None`, which
    /// `combine::build_combined` resolves against the sprite's tps `pivotPoint`.
    /// For container nodes (no graphic) and the synthesized root the cascade
    /// uses `[0.5, 0.5]` as the cascade default.
    pub pivot: Option<[f32; 2]>,
    /// Per-axis uniform/non-uniform scale. `[1, 1]` = identity.
    pub scale: [f32; 2],
    /// Counter-clockwise rotation around Z in degrees (Unity UI canvas
    /// convention, Y-up). Matches `apply_transform`'s 2D rotation matrix.
    pub rot_deg_ccw: f32,
    pub graphic: Option<Graphic>,
    pub children: Vec<Node>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Graphic {
    /// UIIcon / UISlice (CSA hierarchy) or atlas-sprite under SMA.
    /// The historical UIIcon `_scaleFactor` (× CSA `_scaleFactor`) is folded
    /// into the node's `scale` magnitude — there is no per-leaf scale knob.
    Sprite {
        sprite: String,
        method: SpriteMethod,
        border_mult: f32,
        flip_x: bool,
        flip_y: bool,
    },
    /// UISolid or color-quad polygon.
    Polygon {
        /// Resolved tpsheet entry name (e.g. `Color_32264DBDFF` for color `"32264DBD"`,
        /// `Color_32264D` for `"32264D"`).
        polygon_sprite: String,
        vertices: Vec<[f32; 2]>,
        triangles: Option<Vec<u16>>,
    },
    /// SMA SpriteRenderer leaf — different VBO layout, tile mode option.
    /// World-unit draw rect lives in `Node.size` (tiled mode only).
    SpriteRenderer {
        sprite: String,
        draw_mode: DrawMode,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SpriteMethod {
    Id,
    Mx, My, Mxy,
    Tx, Ty, TxMc3,
    R1c3, R3c3, R3c3Nf,
    MxR1c3, MxR1c4, MxR3c2, MxR3c3, MxR3c4, MxR3c6,
    MyR2c2, MyR2c3, MyR3c1, MyR3c2, MyR3c3,
    MxyR3c3, MxyR3c3Nf,
}

impl SpriteMethod {
    /// True iff omitting `size` would be a parse error (slice grids and tilers
    /// need a target rect). Mirrors `fab::Method::requires_size` 1:1.
    pub fn requires_size(self) -> bool {
        !matches!(self, Self::Id | Self::Mx | Self::My | Self::Mxy)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DrawMode {
    Simple,
    Tiled,
}

#[derive(Debug)]
pub enum ManifestError {
    Json(serde_json::Error),
    UnsupportedVersion(u32),
    EmptyName,
    DuplicateName(String),
    BadColor { tree: String, color: String },
    UnknownMethod { tree: String, method: String },
    UnknownDrawMode { tree: String, mode: String },
    PolygonTooFewVertices { tree: String, n: usize },
    GraphicShape { tree: String, reason: &'static str },
    OutputShape { tree: String, reason: &'static str },
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(e) => write!(f, "json: {e}"),
            Self::UnsupportedVersion(v) => write!(f, "unsupported version: {v}"),
            Self::EmptyName => write!(f, "tree.name must be non-empty"),
            Self::DuplicateName(n) => write!(f, "duplicate tree.name: {n:?}"),
            Self::BadColor { tree, color } => write!(
                f,
                "tree {tree:?} polygon color {color:?}: expected 6 or 8 hex chars"
            ),
            Self::UnknownMethod { tree, method } => {
                write!(f, "tree {tree:?}: unknown sprite method {method:?}")
            }
            Self::UnknownDrawMode { tree, mode } => {
                write!(f, "tree {tree:?}: unknown drawMode {mode:?}")
            }
            Self::PolygonTooFewVertices { tree, n } => write!(
                f,
                "tree {tree:?} polygon: need ≥ 3 vertices, got {n}"
            ),
            Self::GraphicShape { tree, reason } => {
                write!(f, "tree {tree:?} graphic: {reason}")
            }
            Self::OutputShape { tree, reason } => {
                write!(f, "tree {tree:?} output: {reason}")
            }
        }
    }
}

impl std::error::Error for ManifestError {}

pub fn parse(json: &str) -> Result<Manifest, ManifestError> {
    let raw: raw::Manifest = serde_json::from_str(json).map_err(ManifestError::Json)?;
    if raw.version != 1 {
        return Err(ManifestError::UnsupportedVersion(raw.version));
    }
    let mut seen: HashSet<String> = HashSet::with_capacity(raw.trees.len());
    let mut trees: Vec<Tree> = Vec::with_capacity(raw.trees.len());
    for t in raw.trees {
        if t.name.is_empty() {
            return Err(ManifestError::EmptyName);
        }
        if !seen.insert(t.name.clone()) {
            return Err(ManifestError::DuplicateName(t.name));
        }
        let output = translate_mode(&t)?;
        let mut children = Vec::with_capacity(t.children.len());
        for c in t.children {
            children.push(translate_node(&t.name, c)?);
        }
        // Wrap the flat children list back into the `root: Node` form that
        // the bridge consumes downstream. The `root` is a pure container
        // (no graphic) carrying the original children.
        let root = Node {
            name: String::new(),
            pos: [0.0, 0.0],
            size: None,
            pivot: None,
            scale: [1.0, 1.0],
            rot_deg_ccw: 0.0,
            graphic: None,
            children,
        };
        trees.push(Tree {
            name: t.name,
            output,
            root,
        });
    }
    Ok(Manifest { trees })
}

fn translate_mode(t: &raw::Tree) -> Result<Output, ManifestError> {
    let sma_fields_present = t.file_id.is_some()
        || t.output_path.is_some()
        || t.keep_vertices.is_some()
        || t.keep_indices.is_some();
    match t.mode.as_str() {
        "ui" => {
            if sma_fields_present {
                return Err(ManifestError::OutputShape {
                    tree: t.name.clone(),
                    reason: "mode=ui must not carry fileId/outputPath/keepVertices/keepIndices",
                });
            }
            Ok(Output::Csa)
        }
        "sma-canvas" | "sma-renderer" => {
            let file_id = t.file_id.ok_or(ManifestError::OutputShape {
                tree: t.name.clone(),
                reason: "mode=sma-* requires `fileId`",
            })?;
            let output_path = t.output_path.clone().ok_or(ManifestError::OutputShape {
                tree: t.name.clone(),
                reason: "mode=sma-* requires `outputPath`",
            })?;
            Ok(Output::Sma {
                file_id,
                output_path,
                used_in_canvas: t.mode == "sma-canvas",
                keep_vertices: t.keep_vertices.unwrap_or(true),
                keep_indices: t.keep_indices.unwrap_or(true),
            })
        }
        _ => Err(ManifestError::OutputShape {
            tree: t.name.clone(),
            reason: "mode must be \"ui\" | \"sma-canvas\" | \"sma-renderer\"",
        }),
    }
}

fn translate_node(tree: &str, raw: raw::Node) -> Result<Node, ManifestError> {
    let scale = raw
        .scale
        .as_ref()
        .map(raw::ScaleSpec::resolve)
        .unwrap_or([1.0, 1.0]);
    let graphic = match raw.kind.as_deref() {
        None => None,
        Some(k) => Some(translate_graphic_flat(tree, k, &raw)?),
    };
    let mut children = Vec::with_capacity(raw.children.len());
    for c in raw.children {
        children.push(translate_node(tree, c)?);
    }
    Ok(Node {
        name: raw.name.unwrap_or_default(),
        pos: raw.pos.unwrap_or([0.0, 0.0]),
        size: raw.size,
        pivot: raw.pivot,
        scale,
        rot_deg_ccw: raw.rot_deg_ccw.unwrap_or(0.0),
        graphic,
        children,
    })
}

fn translate_graphic_flat(tree: &str, kind: &str, raw: &raw::Node) -> Result<Graphic, ManifestError> {
    match kind {
        "sprite" => {
            let sprite = raw
                .sprite
                .clone()
                .ok_or(ManifestError::GraphicShape {
                    tree: tree.to_string(),
                    reason: "type=sprite requires `sprite`",
                })?;
            let method_str = raw.method.as_deref().unwrap_or("ID");
            let method = parse_method(method_str).ok_or_else(|| ManifestError::UnknownMethod {
                tree: tree.to_string(),
                method: method_str.to_string(),
            })?;
            Ok(Graphic::Sprite {
                sprite,
                method,
                border_mult: raw.border_mult.unwrap_or(1.0),
                flip_x: raw.flip_x.unwrap_or(false),
                flip_y: raw.flip_y.unwrap_or(false),
            })
        }
        "polygon" => {
            let color = raw.color.clone().ok_or(ManifestError::GraphicShape {
                tree: tree.to_string(),
                reason: "type=polygon requires `color`",
            })?;
            let polygon_sprite = resolve_color(tree, &color)?;
            let vertices = raw.vertices.clone().unwrap_or_default();
            if vertices.len() < 3 {
                return Err(ManifestError::PolygonTooFewVertices {
                    tree: tree.to_string(),
                    n: vertices.len(),
                });
            }
            Ok(Graphic::Polygon {
                polygon_sprite,
                vertices,
                triangles: raw.triangles.clone(),
            })
        }
        "spriteRenderer" | "sprite-renderer" => {
            let sprite = raw
                .sprite
                .clone()
                .ok_or(ManifestError::GraphicShape {
                    tree: tree.to_string(),
                    reason: "type=spriteRenderer requires `sprite`",
                })?;
            let draw_mode = match raw.draw_mode.as_deref().unwrap_or("simple") {
                "simple" => DrawMode::Simple,
                "tiled" => DrawMode::Tiled,
                other => {
                    return Err(ManifestError::UnknownDrawMode {
                        tree: tree.to_string(),
                        mode: other.to_string(),
                    })
                }
            };
            Ok(Graphic::SpriteRenderer {
                sprite,
                draw_mode,
            })
        }
        _ => Err(ManifestError::GraphicShape {
            tree: tree.to_string(),
            reason: "type must be \"sprite\" | \"polygon\" | \"spriteRenderer\"",
        }),
    }
}

fn resolve_color(tree: &str, c: &str) -> Result<String, ManifestError> {
    let len = c.len();
    let valid = (len == 6 || len == 8) && c.bytes().all(|b| b.is_ascii_hexdigit());
    if !valid {
        return Err(ManifestError::BadColor {
            tree: tree.to_string(),
            color: c.to_string(),
        });
    }
    // Atlas entry names match the input verbatim — 6-hex when alpha is
    // implicit FF, 8-hex when alpha is explicit. The corpus
    // convention is to drop the redundant `FF` alpha when fully opaque,
    // so `"FFFFFF"` (6-hex) → `Color_FFFFFF` (the actual sprite name in
    // the tpsheet), not `Color_FFFFFFFF`.
    let upper = c.to_ascii_uppercase();
    Ok(format!("Color_{upper}"))
}

fn parse_method(s: &str) -> Option<SpriteMethod> {
    Some(match s {
        "ID" => SpriteMethod::Id,
        "MX" => SpriteMethod::Mx,
        "MY" => SpriteMethod::My,
        "MXY" => SpriteMethod::Mxy,
        "TX" => SpriteMethod::Tx,
        "TY" => SpriteMethod::Ty,
        "TX_MC3" => SpriteMethod::TxMc3,
        "R1C3" => SpriteMethod::R1c3,
        "R3C3" => SpriteMethod::R3c3,
        "R3C3_NF" => SpriteMethod::R3c3Nf,
        "MX_R1C3" => SpriteMethod::MxR1c3,
        "MX_R1C4" => SpriteMethod::MxR1c4,
        "MX_R3C2" => SpriteMethod::MxR3c2,
        "MX_R3C3" => SpriteMethod::MxR3c3,
        "MX_R3C4" => SpriteMethod::MxR3c4,
        "MX_R3C6" => SpriteMethod::MxR3c6,
        "MY_R2C2" => SpriteMethod::MyR2c2,
        "MY_R2C3" => SpriteMethod::MyR2c3,
        "MY_R3C1" => SpriteMethod::MyR3c1,
        "MY_R3C2" => SpriteMethod::MyR3c2,
        "MY_R3C3" => SpriteMethod::MyR3c3,
        "MXY_R3C3" => SpriteMethod::MxyR3c3,
        "MXY_R3C3_NF" => SpriteMethod::MxyR3c3Nf,
        _ => return None,
    })
}

mod raw {
    use serde::Deserialize;

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct Manifest {
        pub version: u32,
        #[serde(default)]
        // JSON key is `combined` (each entry IS a fabricated combined sprite).
        // Internal Rust name stays `trees` to avoid colliding with `fab::Combined`,
        // the lowered runtime IR — see `manifest::Manifest.trees` docs.
        #[serde(rename = "combined")]
        pub trees: Vec<Tree>,
    }

    /// Per-tree input. Flat `{ mode, children }` with optional SMA fields at
    /// the tree level. There is no tree-level `scale` — the canvas factor
    /// is mode-implicit (see `Output::canvas_scale_implicit`).
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    pub struct Tree {
        pub name: String,
        pub mode: String, // "ui" | "sma-canvas" | "sma-renderer"
        // SMA fields — required when mode starts with "sma-", rejected otherwise.
        pub file_id: Option<i64>,
        pub output_path: Option<String>,
        pub keep_vertices: Option<bool>,
        pub keep_indices: Option<bool>,
        #[serde(default)]
        pub children: Vec<Node>,
    }

    /// Per-leaf input. Graphic + RectTransform fields are flattened
    /// onto the node directly. `type` discriminates the graphic kind
    /// ("sprite" | "polygon" | "spriteRenderer"); absent = pure container.
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    pub struct Node {
        // Transform fields:
        pub name: Option<String>,
        pub pos: Option<[f32; 2]>,
        pub size: Option<[f32; 2]>,
        pub pivot: Option<[f32; 2]>,
        pub scale: Option<ScaleSpec>,
        // Force `rotDegCCW` (all-caps CCW) — serde's camelCase would lower
        // the second C, giving `rotDegCcw` which buries the abbreviation.
        #[serde(rename = "rotDegCCW")]
        pub rot_deg_ccw: Option<f32>,

        // Graphic discriminator + fields:
        #[serde(rename = "type")]
        pub kind: Option<String>,
        // sprite + sprite-renderer
        pub sprite: Option<String>,
        // sprite only
        pub method: Option<String>,
        pub border_mult: Option<f32>,
        pub flip_x: Option<bool>,
        pub flip_y: Option<bool>,
        // polygon
        pub color: Option<String>,
        pub vertices: Option<Vec<[f32; 2]>>,
        pub triangles: Option<Vec<u16>>,
        // sprite-renderer (consumes `size` above when drawMode=tiled)
        pub draw_mode: Option<String>,

        #[serde(default)]
        pub children: Vec<Node>,
    }

    #[derive(Deserialize)]
    #[serde(untagged)]
    pub enum ScaleSpec {
        Uniform(f32),
        PerAxis([f32; 2]),
    }
    impl ScaleSpec {
        pub fn resolve(&self) -> [f32; 2] {
            match self {
                Self::Uniform(s) => [*s, *s],
                Self::PerAxis(arr) => *arr,
            }
        }
    }
}

/// One graphic leaf with its world-frame transform after the tree walk.
/// Equivalent to what the Unity-side dumper used to capture pre-flattened
/// (`rel_m03/m13` etc.); now computed in Rust from the tree shape.
#[derive(Debug, PartialEq)]
pub struct ResolvedLeaf<'a> {
    /// Composed position in the tree's root-local frame (world units for
    /// SpriteRenderer / SMA, canvas-pixel units for CSA pre-`scale`).
    pub world_pos: [f32; 2],
    /// Composed per-axis scale (sign carries flip).
    pub world_scale: [f32; 2],
    /// Composed rotation in degrees (sum of node `rot_deg_ccw` along the path).
    pub world_rot_deg_ccw: f32,
    /// The leaf node's authored `size` (untransformed). `None` ⇒ runtime
    /// uses the sprite's natural rect (sprite leaves, size-fitted methods)
    /// or the field is irrelevant (native-scale sprite, polygon).
    pub size: Option<[f32; 2]>,
    /// The leaf node's authored `pivot` (untransformed). `None` ⇒ runtime
    /// uses the sprite's tps pivotPoint (sprite leaves) or `(0.5, 0.5)`
    /// (polygon hardcode); see `Part::AtlasSprite.part_pivot`.
    pub pivot: Option<[f32; 2]>,
    pub graphic: &'a Graphic,
}

/// Walk a tree, composing each node's transform with its ancestors' so the
/// leaves end up with root-local coords. Mirrors Unity's RectTransform
/// cascade for the common center-anchored case — each child's world pos is
/// its parent's world pos plus a center-of-rect offset
/// `(0.5 − parent.pivot) × parent.size` (the "Body shift" seen on
/// AlbumSticker_Ghost1), plus the child's own anchored `pos`, with the
/// composed (offset + pos) rotated by the parent's world rotation
/// (Spider's `-45°` Body propagates into child SP world pos). Returns
/// leaves in DFS order — same order the previous flat `parts: [...]`
/// schema declared.
pub fn walk<'a>(tree: &'a Tree) -> Vec<ResolvedLeaf<'a>> {
    let mut out = Vec::new();
    walk_node(
        &tree.root,
        [0.0, 0.0],
        [1.0, 1.0],
        0.0,
        [0.0, 0.0],
        [0.5, 0.5],
        &mut out,
    );
    out
}

fn walk_node<'a>(
    node: &'a Node,
    parent_world_pos: [f32; 2],
    parent_world_scale: [f32; 2],
    parent_world_rot: f32,
    parent_size: [f32; 2],
    parent_pivot: [f32; 2],
    out: &mut Vec<ResolvedLeaf<'a>>,
) {
    // Center offset: where parent's rect-center sits, relative to parent's
    // local origin (which is the pivot point in Unity's RectTransform). Only
    // applies when the parent has a meaningful size; absent size = (0, 0)
    // collapses the offset to zero (correct for pure-transform containers).
    let parent_center_offset = [
        (0.5 - parent_pivot[0]) * parent_size[0],
        (0.5 - parent_pivot[1]) * parent_size[1],
    ];
    // Apply parent's composed world_scale + world_rot to the child's local
    // pos. Matches Unity's RectTransform/Transform cascade:
    //   - localScale: a child at local pos (-504, 0) under parent
    //     localScale (-1, 1) lands at world (+504, 0). Without this
    //     multiply, mirror containers (R sibling of L) collapse onto L.
    //   - localRotation: a child at local pos (-28.4, -16) under parent
    //     rotated -45° in Z lands at world (-31.4, 8.8). Without the
    //     rotate, child positions are off by the rotation delta — see
    //     Spider golden where 4 verts shift by (+2.97, -24.76) world
    //     units when the -45° Body parent's rot isn't propagated.
    let p = (parent_center_offset[0] + node.pos[0]) * parent_world_scale[0];
    let q = (parent_center_offset[1] + node.pos[1]) * parent_world_scale[1];
    let r = parent_world_rot.to_radians();
    let (sin_r, cos_r) = r.sin_cos();
    let world_pos = [
        parent_world_pos[0] + p * cos_r - q * sin_r,
        parent_world_pos[1] + p * sin_r + q * cos_r,
    ];
    let world_scale = [
        parent_world_scale[0] * node.scale[0],
        parent_world_scale[1] * node.scale[1],
    ];
    let world_rot = parent_world_rot + node.rot_deg_ccw;

    if let Some(g) = &node.graphic {
        out.push(ResolvedLeaf {
            world_pos,
            world_scale,
            world_rot_deg_ccw: world_rot,
            size: node.size,
            pivot: node.pivot,
            graphic: g,
        });
    }
    // Cascade-default container pivots to (0.5, 0.5) — there's no
    // sprite-pivot fallback at the GameObject level for pure-transform
    // nodes. (Sprite-leaf children get their own pivot resolved
    // downstream from the tps entry.)
    let cascade_pivot = node.pivot.unwrap_or([0.5, 0.5]);
    let cascade_size = node.size.unwrap_or([0.0, 0.0]);
    for c in &node.children {
        walk_node(
            c,
            world_pos,
            world_scale,
            world_rot,
            cascade_size,
            cascade_pivot,
            out,
        );
    }
}

// ---------------------------------------------------------------------------
// Bridge: `Tree` → `fab::Combined` (sprites) / `mesh_manifest::MeshCombined`
// (meshes). The downstream emit pipelines (combine.rs, mesh_emit.rs) consume
// these IRs directly.

#[derive(Debug)]
pub enum BridgeError {
    /// Tree's `output` doesn't match the requested adapter (e.g. SMA tree
    /// fed into `to_fab_combined`).
    OutputMismatch { tree: String, expected: &'static str },
    /// Tree contains a graphic incompatible with the output (e.g. polygon
    /// under SMA, or sprite-renderer under CSA).
    GraphicMismatch { tree: String, reason: &'static str },
    /// Method requires `size` (R*/MX_R*/MY_R*/MXY_R*/TX*/TY*) but the
    /// node's `size` is zero. Either omit `size` (defaults to the
    /// sprite's natural rect) or pass a non-zero `[w, h]`.
    ZeroSizeForSliceMethod { tree: String, sprite: String },
}

impl fmt::Display for BridgeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutputMismatch { tree, expected } => write!(
                f,
                "tree {tree:?} output mismatch: expected {expected}"
            ),
            Self::GraphicMismatch { tree, reason } => {
                write!(f, "tree {tree:?} graphic: {reason}")
            }
            Self::ZeroSizeForSliceMethod { tree, sprite } => write!(
                f,
                "tree {tree:?} sprite {sprite:?}: size-fitted method requires non-zero size"
            ),
        }
    }
}

impl std::error::Error for BridgeError {}

/// Convert a `Tree` with `output: "csa"` into a `fab::Combined`, the
/// flat-shape struct the existing `combine::build_combined` consumes.
///
/// The mode-implicit canvas factor (`Output::canvas_scale_implicit`) is
/// applied here at one seam: each leaf's anchored `pos` (canvas-pixel
/// units) is pre-multiplied so `Part.offset` lands in world units. The
/// runtime per-vert chain therefore drops `× canvas_scale` entirely.
pub fn to_fab_combined(tree: &Tree) -> Result<crate::fab::Combined, BridgeError> {
    use crate::fab;

    if tree.output != Output::Csa {
        return Err(BridgeError::OutputMismatch {
            tree: tree.name.clone(),
            expected: "csa",
        });
    }
    let cs = tree.output.canvas_scale_implicit();

    let leaves = walk(tree);
    let mut parts: Vec<fab::Part> = Vec::with_capacity(leaves.len());
    for leaf in leaves {
        let offset = [leaf.world_pos[0] * cs, leaf.world_pos[1] * cs];
        let base_affine = fab::Affine {
            tx: 0.0,
            ty: 0.0,
            sx: leaf.world_scale[0],
            sy: leaf.world_scale[1],
            rot_deg_ccw: leaf.world_rot_deg_ccw,
        };
        match leaf.graphic {
            Graphic::Sprite {
                sprite,
                method,
                border_mult,
                flip_x,
                flip_y,
            } => {
                // flipX/Y folds into the affine sign — matches the SMA path
                // and the doc spec on `Graphic::Sprite`. Doing it here keeps
                // the runtime per-vert chain `affine · v + offset` correct
                // (no separate flip stage needed downstream).
                let mut affine = base_affine;
                if *flip_x { affine.sx = -affine.sx; }
                if *flip_y { affine.sy = -affine.sy; }
                let fab_method = map_method(*method);
                // Pre-scale `size` by canvas_scale_implicit, mirroring the
                // `offset` pattern. The slice/tile mesh-gen math in
                // `combine::slice_*` divides by sprite_bound_size (already
                // in world units), so target_size must be in world units
                // too — without pre-scaling, CSA leaves emit verts 100×
                // too big (the regression class this refactor is fixing).
                //
                // Strictly-size-fitted methods (R*, MX_R*, MY_R*, MXY_R*,
                // TX, TY, TX_MC3) reject explicit zero — slice math would
                // divide by zero. MX/MY/MXY tolerate a zero axis: CSA-era
                // prefabs used `[0, h]` with stretch anchors that the rlib
                // doesn't resolve, but the value is a faithful echo.
                let size = match leaf.size {
                    Some([w, h]) if fab_method.requires_size() && (w == 0.0 || h == 0.0) => {
                        return Err(BridgeError::ZeroSizeForSliceMethod {
                            tree: tree.name.clone(),
                            sprite: sprite.clone(),
                        });
                    }
                    Some([w, h]) => Some((w * cs, h * cs)),
                    None => None,
                };
                parts.push(fab::Part::AtlasSprite {
                    sprite: sprite.clone(),
                    method: fab_method,
                    size,
                    part_pivot: leaf.pivot,  // None ⇒ default to tps pivotPoint at build_combined
                    border_mult: *border_mult,
                    affine,
                    offset,
                });
            }
            Graphic::Polygon {
                polygon_sprite,
                vertices,
                triangles,
            } => {
                parts.push(fab::Part::Polygon {
                    polygon_sprite: polygon_sprite.clone(),
                    vertices: vertices.clone(),
                    triangles: triangles.clone(),
                    affine: base_affine,
                    offset,
                });
            }
            Graphic::SpriteRenderer { .. } => {
                return Err(BridgeError::GraphicMismatch {
                    tree: tree.name.clone(),
                    reason: "sprite-renderer graphic incompatible with CSA output",
                });
            }
        }
    }

    Ok(fab::Combined {
        name: tree.name.clone(),
        pivot: [0.5, 0.5],
        border: [0.0; 4],
        parts,
    })
}

fn map_method(m: SpriteMethod) -> crate::fab::Method {
    use crate::fab::Method as F;
    match m {
        SpriteMethod::Id => F::Id,
        SpriteMethod::Mx => F::Mx,
        SpriteMethod::My => F::My,
        SpriteMethod::Mxy => F::Mxy,
        SpriteMethod::Tx => F::Tx,
        SpriteMethod::Ty => F::Ty,
        SpriteMethod::TxMc3 => F::TxMc3,
        SpriteMethod::R1c3 => F::R1c3,
        SpriteMethod::R3c3 => F::R3c3,
        SpriteMethod::R3c3Nf => F::R3c3Nf,
        SpriteMethod::MxR1c3 => F::MxR1c3,
        SpriteMethod::MxR1c4 => F::MxR1c4,
        SpriteMethod::MxR3c2 => F::MxR3c2,
        SpriteMethod::MxR3c3 => F::MxR3c3,
        SpriteMethod::MxR3c4 => F::MxR3c4,
        SpriteMethod::MxR3c6 => F::MxR3c6,
        SpriteMethod::MyR2c2 => F::MyR2c2,
        SpriteMethod::MyR2c3 => F::MyR2c3,
        SpriteMethod::MyR3c1 => F::MyR3c1,
        SpriteMethod::MyR3c2 => F::MyR3c2,
        SpriteMethod::MyR3c3 => F::MyR3c3,
        SpriteMethod::MxyR3c3 => F::MxyR3c3,
        SpriteMethod::MxyR3c3Nf => F::MxyR3c3Nf,
    }
}

/// Convert a `Tree` with `Output::Sma { … }` into the mesh-emit-ready shape:
/// extract the SMA output config + every `Graphic::SpriteRenderer` leaf with
/// its composed 2D affine (`localToRoot`). Returns the per-tree atomic unit
/// the existing `mesh_emit::build_mesh` consumes — except `build_mesh`
/// currently keys off the flat `MeshCombined { fileId, name, output_path,
/// usedInCanvas, keep*, renderers: [...] }` shape, so we hand-build that
/// from the walker output.
pub fn to_mesh_combined(
    tree: &Tree,
) -> Result<crate::mesh_manifest::MeshCombined, BridgeError> {
    use crate::mesh_manifest as mm;

    let (file_id, output_path, used_in_canvas, keep_vertices, keep_indices) = match &tree.output {
        Output::Sma {
            file_id,
            output_path,
            used_in_canvas,
            keep_vertices,
            keep_indices,
        } => (*file_id, output_path.clone(), *used_in_canvas, *keep_vertices, *keep_indices),
        _ => {
            return Err(BridgeError::OutputMismatch {
                tree: tree.name.clone(),
                expected: "sma",
            })
        }
    };

    // Mode-implicit canvas factor — required to be 1.0 for SMA today
    // (SpriteRenderer.size is already in world units). Asserted to keep the
    // single-seam invariant honest: a future SMA mode introducing cs ≠ 1.0
    // would have to thread it through here too.
    debug_assert_eq!(
        tree.output.canvas_scale_implicit(), 1.0,
        "to_mesh_combined assumes SMA canvas_scale_implicit == 1.0",
    );

    let leaves = walk(tree);
    let mut renderers: Vec<mm::MeshRenderer> = Vec::with_capacity(leaves.len());
    for leaf in leaves {
        let content = match leaf.graphic {
            Graphic::SpriteRenderer { sprite, draw_mode } => {
                let dm = match draw_mode {
                    DrawMode::Simple => mm::DrawMode::Simple,
                    DrawMode::Tiled => mm::DrawMode::Tiled,
                };
                // SpriteRenderer.size lives on the node (`leaf.size`) —
                // tiled mode consumes a world-unit draw rect; simple
                // mode leaves it unset.
                mm::MeshRendererContent::Sprite {
                    sprite: sprite.clone(),
                    draw_mode: dm,
                    size: leaf.size,
                }
            }
            Graphic::Polygon { polygon_sprite, vertices, triangles } => {
                mm::MeshRendererContent::Polygon {
                    polygon_sprite: polygon_sprite.clone(),
                    vertices: vertices.clone(),
                    triangles: triangles.clone(),
                }
            }
            Graphic::Sprite { .. } => {
                return Err(BridgeError::GraphicMismatch {
                    tree: tree.name.clone(),
                    reason: "sma output rejects CSA `sprite` graphics; use `spriteRenderer`",
                })
            }
        };
        // localToRoot = composed 2D affine row-major [m00, m01, m02, m03, m10, m11, m12, m13].
        // The walker gives us world_scale (composed across ancestors), world_pos
        // (composed translation), and world_rot_deg_ccw (composed rotation). Build
        // the matrix accordingly. SpriteRenderer flipX/Y is folded into
        // world_scale's sign; polygons have no flip semantic.
        let (sin_t, cos_t) = leaf.world_rot_deg_ccw.to_radians().sin_cos();
        let m00 = cos_t * leaf.world_scale[0];
        let m01 = -sin_t * leaf.world_scale[1];
        let m10 = sin_t * leaf.world_scale[0];
        let m11 = cos_t * leaf.world_scale[1];
        let l2r = [m00, m01, 0.0, leaf.world_pos[0], m10, m11, 0.0, leaf.world_pos[1]];

        renderers.push(mm::MeshRenderer {
            // SpriteRenderer: flipX/Y is folded into world_scale's sign by
            // the walker, so the IR keeps these false (the matrix carries
            // the sign in m00/m11; a true here would double-flip).
            // Polygon: no flip semantic at the Graphic level — also false.
            // The fields stay on `MeshRenderer` for callers that bypass
            // the walker (direct IR authoring in tests).
            flip_x: false,
            flip_y: false,
            local_to_root: l2r,
            content,
        });
    }

    Ok(mm::MeshCombined {
        file_id,
        name: tree.name.clone(),
        output_path,
        used_in_canvas,
        keep_vertices,
        keep_indices,
        renderers,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_csa_tree() {
        let m = parse(
            r#"{
              "version": 1,
              "combined": [{
                "name": "X",
                "mode": "ui",
                "children": [
                    { "type":"sprite", "sprite":"foo" }
                  ]
              }]
            }"#,
        )
        .unwrap();
        assert_eq!(m.trees.len(), 1);
        let t = &m.trees[0];
        assert_eq!(t.output, Output::Csa);
        assert_eq!(t.output.canvas_scale_implicit(), 0.01);
        assert_eq!(t.root.children.len(), 1);
        match &t.root.children[0].graphic.as_ref().unwrap() {
            Graphic::Sprite { method, .. } => {
                assert_eq!(*method, SpriteMethod::Id);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn parse_polygon_color_6_hex() {
        let m = parse(
            r#"{ "version": 1, "combined": [{
              "name": "X", "mode": "ui",
              "children": [{
                "type":"polygon", "color":"32264D", "vertices":[[0,0],[1,0],[1,1]]
              }]
            }]}"#,
        )
        .unwrap();
        match &m.trees[0].root.children[0].graphic.as_ref().unwrap() {
            Graphic::Polygon { polygon_sprite, .. } => assert_eq!(polygon_sprite, "Color_32264D"),
            _ => panic!(),
        }
    }

    #[test]
    fn parse_polygon_color_8_hex() {
        let m = parse(
            r#"{ "version": 1, "combined": [{
              "name": "X", "mode": "ui",
              "children": [{
                "type":"polygon", "color":"DEADBEEF", "vertices":[[0,0],[1,0],[1,1]]
              }]
            }]}"#,
        )
        .unwrap();
        match &m.trees[0].root.children[0].graphic.as_ref().unwrap() {
            Graphic::Polygon { polygon_sprite, .. } => assert_eq!(polygon_sprite, "Color_DEADBEEF"),
            _ => panic!(),
        }
    }

    #[test]
    fn parse_sma_output() {
        let m = parse(
            r#"{ "version": 1, "combined": [{
              "name": "X",
              "mode":"sma-renderer", "fileId":-1234, "outputPath":"!Output/X.asset",
              "children": [{
                "type":"spriteRenderer", "sprite":"foo"
              }]
            }]}"#,
        )
        .unwrap();
        match &m.trees[0].output {
            Output::Sma { file_id, output_path, used_in_canvas, keep_vertices, .. } => {
                assert_eq!(*file_id, -1234);
                assert_eq!(output_path, "!Output/X.asset");
                assert!(!*used_in_canvas);
                assert!(*keep_vertices); // default
            }
            _ => panic!(),
        }
        match &m.trees[0].root.children[0].graphic.as_ref().unwrap() {
            Graphic::SpriteRenderer { sprite, draw_mode } => {
                assert_eq!(sprite, "foo");
                assert_eq!(*draw_mode, DrawMode::Simple);
                assert!(m.trees[0].root.children[0].size.is_none());
            }
            _ => panic!(),
        }
    }

    #[test]
    fn parse_scale_uniform_and_per_axis() {
        let m = parse(
            r#"{ "version": 1, "combined": [{
              "name": "X", "mode": "ui",
              "children": [
                { "scale": 2.5, "type":"sprite","sprite":"a" },
                { "scale": [-1, 1], "type":"sprite","sprite":"b" }
              ]
            }]}"#,
        )
        .unwrap();
        let kids = &m.trees[0].root.children;
        assert_eq!(kids[0].scale, [2.5, 2.5]);
        assert_eq!(kids[1].scale, [-1.0, 1.0]);
    }

    #[test]
    fn parse_nested_children() {
        let m = parse(
            r#"{ "version": 1, "combined": [{
              "name": "X", "mode": "ui",
              "children": [{
                "name": "Body",
                "pivot": [0.5, 0.4515571],
                "size": [154, 181],
                "type":"sprite","sprite":"body",
                "children": [{
                  "name": "SP",
                  "pos": [-69.8, -36.7],
                  "type":"sprite","sprite":"sp"
                }]
              }]
            }]}"#,
        )
        .unwrap();
        let body = &m.trees[0].root.children[0];
        assert_eq!(body.name, "Body");
        assert_eq!(body.pivot, Some([0.5, 0.4515571]));
        assert_eq!(body.children.len(), 1);
        let sp = &body.children[0];
        assert_eq!(sp.pos, [-69.8, -36.7]);
    }

    #[test]
    fn parse_rejects_unknown_top_level_field() {
        let m = parse(r#"{ "version": 1, "combined": [], "extra": 0 }"#);
        assert!(matches!(m, Err(ManifestError::Json(_))));
    }

    #[test]
    fn parse_rejects_bad_color() {
        let m = parse(
            r#"{ "version":1, "combined":[{
              "name":"X", "mode": "ui",
              "children": [{
                "type":"polygon","color":"NOPE","vertices":[[0,0],[1,0],[1,1]]
              }]
            }]}"#,
        );
        assert!(matches!(m, Err(ManifestError::BadColor { .. })));
    }

    #[test]
    fn parse_rejects_unknown_method() {
        let m = parse(
            r#"{ "version":1, "combined":[{
              "name":"X", "mode": "ui",
              "children": [{
                "type":"sprite","sprite":"a","method":"BOGUS"
              }]
            }]}"#,
        );
        assert!(matches!(m, Err(ManifestError::UnknownMethod { .. })));
    }

    #[test]
    fn parse_rejects_sma_as_tag() {
        let m = parse(
            r#"{ "version":1, "combined":[{
              "name":"X", "mode": "sma-tag",
              "children": [{ "type":"spriteRenderer","sprite":"a" }]
            }]}"#,
        );
        assert!(matches!(m, Err(ManifestError::OutputShape { .. })));
    }

    fn parse_single(s: &str) -> Manifest {
        parse(s).unwrap()
    }

    #[test]
    fn walk_single_root_child_at_origin() {
        let m = parse_single(
            r#"{ "version":1, "combined":[{
              "name":"X", "mode": "ui",
              "children":[{"type":"sprite","sprite":"a"}]
            }]}"#,
        );
        let leaves = walk(&m.trees[0]);
        assert_eq!(leaves.len(), 1);
        assert_eq!(leaves[0].world_pos, [0.0, 0.0]);
        assert_eq!(leaves[0].world_scale, [1.0, 1.0]);
    }

    #[test]
    fn walk_child_translated() {
        let m = parse_single(
            r#"{ "version":1, "combined":[{
              "name":"X", "mode": "ui",
              "children":[
                {"pos":[10,20],"type":"sprite","sprite":"a"}
              ]
            }]}"#,
        );
        let leaves = walk(&m.trees[0]);
        assert_eq!(leaves[0].world_pos, [10.0, 20.0]);
    }

    #[test]
    fn walk_nested_pivot_center_offset() {
        // Mirrors AlbumSticker_Ghost1: Body has non-center pivot, so SP's
        // world.y picks up the (0.5 - pivot.y) * sizeDelta.y offset.
        //   Body: pos=(0,0), pivot=(0.5, 0.4515571), sizeDelta=(154, 181)
        //   SP:   pos=(-69.8, -36.7), graphic
        // Expected SP world.y = (0.5 - 0.4515571) * 181 + (-36.7)
        //                     = 0.0484429 * 181 - 36.7
        //                     = 8.768165 - 36.7
        //                     ≈ -27.931835
        let m = parse_single(
            r#"{ "version":1, "combined":[{
              "name":"X", "mode": "ui",
              "children":[{
                "name":"Body",
                "pivot":[0.5, 0.4515571],
                "size":[154, 181],
                "type":"sprite","sprite":"body",
                "children":[{
                  "name":"SP",
                  "pos":[-69.8, -36.7],
                  "type":"sprite","sprite":"sp"
                }]
              }]
            }]}"#,
        );
        let leaves = walk(&m.trees[0]);
        assert_eq!(leaves.len(), 2);
        // Body itself is a leaf with graphic.
        assert_eq!(leaves[0].world_pos, [0.0, 0.0]);
        // SP picks up Body's center-offset shift.
        let sp = &leaves[1];
        // Body's center offset y = (0.5 - 0.4515571) * 181 ≈ 8.768165
        let expected_y = (0.5 - 0.4515571_f32) * 181.0 + (-36.7);
        assert!(
            (sp.world_pos[1] - expected_y).abs() < 1e-4,
            "world.y = {} vs expected {}",
            sp.world_pos[1],
            expected_y
        );
        assert!((sp.world_pos[0] - (-69.8)).abs() < 1e-4);
    }

    #[test]
    fn walk_composes_scale_through_descendants() {
        let m = parse_single(
            r#"{ "version":1, "combined":[{
              "name":"X", "mode": "ui",
              "children":[{
                "scale":[-1, 1],
                "type":"sprite","sprite":"parent",
                "children":[{
                  "scale":[2, 3],
                  "type":"sprite","sprite":"child"
                }]
              }]
            }]}"#,
        );
        let leaves = walk(&m.trees[0]);
        assert_eq!(leaves[0].world_scale, [-1.0, 1.0]);
        assert_eq!(leaves[1].world_scale, [-2.0, 3.0]);
    }

    #[test]
    fn walk_skips_interior_nodes_with_no_graphic() {
        // A pure-transform "group" node with no `graphic` doesn't produce
        // a leaf; only its descendants with graphics do.
        let m = parse_single(
            r#"{ "version":1, "combined":[{
              "name":"X", "mode": "ui",
              "children":[{
                "pos":[5, 10],
                "children":[{
                  "pos":[1, 2],
                  "type":"sprite","sprite":"a"
                }]
              }]
            }]}"#,
        );
        let leaves = walk(&m.trees[0]);
        assert_eq!(leaves.len(), 1);
        assert_eq!(leaves[0].world_pos, [6.0, 12.0]);
    }

    #[test]
    fn bridge_to_fab_csa_minimal() {
        let m = parse_single(
            r#"{ "version":1, "combined":[{
              "name":"X","mode": "ui",
              "children":[
                {"type":"sprite","sprite":"foo"}
              ]
            }]}"#,
        );
        let c = to_fab_combined(&m.trees[0]).unwrap();
        assert_eq!(c.name, "X");
        assert_eq!(c.parts.len(), 1);
        match &c.parts[0] {
            crate::fab::Part::AtlasSprite { sprite, method, size, offset, .. } => {
                assert_eq!(sprite, "foo");
                assert_eq!(*method, crate::fab::Method::Id);
                assert!(size.is_none());
                // Origin leaf under CSA: pos×0.01 = (0,0) regardless.
                assert_eq!(*offset, [0.0, 0.0]);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn bridge_to_fab_polygon_with_offset() {
        let m = parse_single(
            r#"{ "version":1, "combined":[{
              "name":"X","mode": "ui",
              "children":[{
                "pos":[10, 20],
                "type":"polygon","color":"32264D","vertices":[[0,0],[1,0],[1,1]]
              }]
            }]}"#,
        );
        let c = to_fab_combined(&m.trees[0]).unwrap();
        match &c.parts[0] {
            crate::fab::Part::Polygon { polygon_sprite, offset, .. } => {
                assert_eq!(polygon_sprite, "Color_32264D");
                // Mode-implicit canvas_scale (CSA=0.01) pre-applied: 10×0.01 ≈ 0.1
                // (one ULP off in f32; 0.01 is non-representable binary fraction).
                assert!((offset[0] - 0.1).abs() < 1e-6, "{:?}", offset);
                assert!((offset[1] - 0.2).abs() < 1e-6, "{:?}", offset);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn bridge_flipx_folds_into_negative_sx() {
        // The doc spec (line 686-687) and the SMA path both say flipX/flipY
        // fold into the affine sign. CSA's `to_fab_combined` historically
        // dropped them — guard against the regression.
        let m = parse_single(
            r#"{ "version":1, "combined":[{
              "name":"X","mode": "ui",
              "children":[
                {"type":"sprite","sprite":"a","flipX":true}
              ]
            }]}"#,
        );
        let c = to_fab_combined(&m.trees[0]).unwrap();
        match &c.parts[0] {
            crate::fab::Part::AtlasSprite { affine, .. } => {
                assert!(affine.sx < 0.0, "flipX should negate sx, got {}", affine.sx);
                assert!(affine.sy > 0.0, "sy unaffected by flipX, got {}", affine.sy);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn bridge_flipy_folds_into_negative_sy() {
        let m = parse_single(
            r#"{ "version":1, "combined":[{
              "name":"X","mode": "ui",
              "children":[
                {"type":"sprite","sprite":"a","flipY":true}
              ]
            }]}"#,
        );
        let c = to_fab_combined(&m.trees[0]).unwrap();
        match &c.parts[0] {
            crate::fab::Part::AtlasSprite { affine, .. } => {
                assert!(affine.sx > 0.0);
                assert!(affine.sy < 0.0, "flipY should negate sy, got {}", affine.sy);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn bridge_flipxy_combines_with_scale_sign() {
        // flipX + composed negative scale: signs cancel to positive.
        let m = parse_single(
            r#"{ "version":1, "combined":[{
              "name":"X","mode": "ui",
              "children":[
                {"scale":[-1, 1],"type":"sprite","sprite":"a","flipX":true}
              ]
            }]}"#,
        );
        let c = to_fab_combined(&m.trees[0]).unwrap();
        match &c.parts[0] {
            crate::fab::Part::AtlasSprite { affine, .. } => {
                assert!(affine.sx > 0.0, "flipX × scale -1 should restore positive sx");
            }
            _ => panic!(),
        }
    }

    #[test]
    fn bridge_propagates_scale_as_sx_sy() {
        // scale: [-1, 1] should flow into Affine.sx/sy.
        let m = parse_single(
            r#"{ "version":1, "combined":[{
              "name":"X","mode": "ui",
              "children":[
                {"scale":[-1,1],"type":"sprite","sprite":"a"}
              ]
            }]}"#,
        );
        let c = to_fab_combined(&m.trees[0]).unwrap();
        match &c.parts[0] {
            crate::fab::Part::AtlasSprite { affine, .. } => {
                assert_eq!(affine.sx, -1.0);
                assert_eq!(affine.sy, 1.0);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn bridge_size_fitted_method_takes_size_delta_world_units() {
        // CSA mode pre-scales canvas-pixel `size` by canvas_scale_implicit
        // (= 0.01) so the slice mesh-gen math runs in world units. JSON
        // `[200, 150]` lands on the Part as `(2.0, 1.5)`.
        let m = parse_single(
            r#"{ "version":1, "combined":[{
              "name":"X","mode": "ui",
              "children":[{
                "size":[200, 150],
                "type":"sprite","sprite":"a","method":"R3C3"
              }]
            }]}"#,
        );
        let c = to_fab_combined(&m.trees[0]).unwrap();
        match &c.parts[0] {
            crate::fab::Part::AtlasSprite { method, size, .. } => {
                assert_eq!(*method, crate::fab::Method::R3c3);
                assert_eq!(*size, Some((2.0, 1.5)));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn bridge_rejects_size_fitted_with_explicit_zero_size() {
        // Explicit [0, 0] is still rejected (slice methods would divide by
        // zero). Missing size defaults to the sprite's natural rect in
        // build_combined — see the v2_schema integration test.
        let m = parse_single(
            r#"{ "version":1, "combined":[{
              "name":"X","mode": "ui",
              "children":[
                {"type":"sprite","sprite":"a","method":"R3C3","size":[0,0]}
              ]
            }]}"#,
        );
        let err = to_fab_combined(&m.trees[0]).unwrap_err();
        assert!(matches!(err, BridgeError::ZeroSizeForSliceMethod { .. }));
    }

    #[test]
    fn bridge_rejects_sprite_renderer_in_csa() {
        let m = parse_single(
            r#"{ "version":1, "combined":[{
              "name":"X","mode": "ui",
              "children":[
                {"type":"spriteRenderer","sprite":"a"}
              ]
            }]}"#,
        );
        let err = to_fab_combined(&m.trees[0]).unwrap_err();
        assert!(matches!(err, BridgeError::GraphicMismatch { .. }));
    }

    #[test]
    fn bridge_rejects_sma_tree_into_fab_adapter() {
        let m = parse_single(
            r#"{ "version":1, "combined":[{
              "name":"X",
              "mode":"sma-canvas", "fileId":1,"outputPath":"o.asset",
              "children":[{"type":"spriteRenderer","sprite":"a"}]
            }]}"#,
        );
        let err = to_fab_combined(&m.trees[0]).unwrap_err();
        assert!(matches!(err, BridgeError::OutputMismatch { .. }));
    }

    #[test]
    fn bridge_to_mesh_minimal_simple() {
        let m = parse_single(
            r#"{ "version":1, "combined":[{
              "name":"X",
              "mode":"sma-canvas", "fileId":-1234,"outputPath":"o.asset",
              "children":[
                {"type":"spriteRenderer","sprite":"foo"}
              ]
            }]}"#,
        );
        let mc = to_mesh_combined(&m.trees[0]).unwrap();
        assert_eq!(mc.file_id, -1234);
        assert_eq!(mc.name, "X");
        assert_eq!(mc.output_path, "o.asset");
        assert!(mc.used_in_canvas);
        assert!(mc.keep_vertices); // default
        assert_eq!(mc.renderers.len(), 1);
        let r = &mc.renderers[0];
        match &r.content {
            crate::mesh_manifest::MeshRendererContent::Sprite { sprite, draw_mode, size } => {
                assert_eq!(sprite, "foo");
                assert_eq!(*draw_mode, crate::mesh_manifest::DrawMode::Simple);
                assert!(size.is_none());
            }
            other => panic!("expected Sprite content, got {other:?}"),
        }
        assert_eq!(r.local_to_root, [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    }

    #[test]
    fn bridge_to_mesh_translation_into_l2r() {
        let m = parse_single(
            r#"{ "version":1, "combined":[{
              "name":"X",
              "mode":"sma-renderer", "fileId":1,"outputPath":"o.asset",
              "children":[
                {"pos":[10,20],"type":"spriteRenderer","sprite":"a"}
              ]
            }]}"#,
        );
        let mc = to_mesh_combined(&m.trees[0]).unwrap();
        let l = mc.renderers[0].local_to_root;
        assert_eq!(l[3], 10.0); // m03
        assert_eq!(l[7], 20.0); // m13
    }

    #[test]
    fn bridge_to_mesh_flip_folds_into_l2r_diagonal() {
        // scale: [-1, 1] is the walker-composed equivalent of flipX=true.
        // The bridge folds it into matrix m00 = -1, m11 = 1, and clears the
        // separate flip_x/flip_y bits so mesh_emit doesn't double-apply.
        let m = parse_single(
            r#"{ "version":1, "combined":[{
              "name":"X",
              "mode":"sma-renderer", "fileId":1,"outputPath":"o.asset",
              "children":[
                {"scale":[-1,1],"type":"spriteRenderer","sprite":"a"}
              ]
            }]}"#,
        );
        let mc = to_mesh_combined(&m.trees[0]).unwrap();
        let r = &mc.renderers[0];
        assert!(!r.flip_x);
        assert!(!r.flip_y);
        assert_eq!(r.local_to_root[0], -1.0); // m00
        assert_eq!(r.local_to_root[5], 1.0); // m11
    }

    #[test]
    fn bridge_to_mesh_tiled_threads_size() {
        let m = parse_single(
            r#"{ "version":1, "combined":[{
              "name":"X",
              "mode":"sma-renderer", "fileId":1,"outputPath":"o.asset",
              "children":[
                {"type":"spriteRenderer","sprite":"brick","drawMode":"tiled","size":[4.05,1.0]}
              ]
            }]}"#,
        );
        let mc = to_mesh_combined(&m.trees[0]).unwrap();
        let r = &mc.renderers[0];
        match &r.content {
            crate::mesh_manifest::MeshRendererContent::Sprite { sprite, draw_mode, size } => {
                assert_eq!(sprite, "brick");
                assert_eq!(*draw_mode, crate::mesh_manifest::DrawMode::Tiled);
                assert_eq!(*size, Some([4.05, 1.0]));
            }
            other => panic!("expected Sprite content, got {other:?}"),
        }
    }

    #[test]
    fn bridge_to_mesh_rejects_csa_tree() {
        let m = parse_single(
            r#"{ "version":1, "combined":[{
              "name":"X","mode": "ui",
              "children":[{"type":"sprite","sprite":"a"}]
            }]}"#,
        );
        let err = to_mesh_combined(&m.trees[0]).unwrap_err();
        assert!(matches!(err, BridgeError::OutputMismatch { .. }));
    }

    #[test]
    fn bridge_to_mesh_accepts_polygon() {
        // SMA trees now compose with polygon leaves (resolved to a
        // `Color_*` tpsheet entry). The bridge preserves the raw
        // vertices + optional triangle override into the
        // `MeshRendererContent::Polygon` variant; the matrix carries
        // the leaf's composed scale/rot/pos same as SpriteRenderer.
        let m = parse_single(
            r#"{ "version":1, "combined":[{
              "name":"X",
              "mode":"sma-canvas", "fileId":1,"outputPath":"o.asset",
              "children":[
                {
                  "pos":[7,3],
                  "type":"polygon","color":"112233",
                  "vertices":[[-1,-1],[1,-1],[1,1],[-1,1]],
                  "triangles":[0,2,3,3,1,0]
                }
              ]
            }]}"#,
        );
        let mc = to_mesh_combined(&m.trees[0]).unwrap();
        assert_eq!(mc.renderers.len(), 1);
        let r = &mc.renderers[0];
        match &r.content {
            crate::mesh_manifest::MeshRendererContent::Polygon {
                polygon_sprite, vertices, triangles,
            } => {
                assert_eq!(polygon_sprite, "Color_112233");
                assert_eq!(vertices.len(), 4);
                assert_eq!(triangles.as_deref(), Some(&[0u16, 2, 3, 3, 1, 0][..]));
            }
            other => panic!("expected Polygon content, got {other:?}"),
        }
        // Bridge composes leaf.pos into m03/m13 — same path as SpriteRenderer.
        assert_eq!(r.local_to_root[3], 7.0);
        assert_eq!(r.local_to_root[7], 3.0);
    }

    #[test]
    fn bridge_to_mesh_rejects_csa_sprite_graphic() {
        // SMA accepts only `spriteRenderer` and `polygon` leaves. A
        // CSA-style `sprite` (with `method` / slice grids) belongs in a
        // `ui` tree and is rejected here.
        let m = parse_single(
            r#"{ "version":1, "combined":[{
              "name":"X",
              "mode":"sma-canvas", "fileId":1,"outputPath":"o.asset",
              "children":[
                {"type":"sprite","sprite":"a","method":"ID"}
              ]
            }]}"#,
        );
        let err = to_mesh_combined(&m.trees[0]).unwrap_err();
        assert!(matches!(err, BridgeError::GraphicMismatch { .. }));
    }

    #[test]
    fn polygon_sprite_names_sorted_deduped_across_trees() {
        let m = parse(
            r#"{ "version":1, "combined":[
              {"name":"A","mode":"ui","children":[
                {"type":"polygon","color":"FF0000","vertices":[[0,0],[1,0],[1,1]]},
                {"type":"polygon","color":"00ff00","vertices":[[0,0],[1,0],[1,1]]}
              ]},
              {"name":"B","mode":"ui","children":[
                {"type":"polygon","color":"FF0000","vertices":[[0,0],[1,0],[1,1]]},
                {"name":"nested","children":[
                  {"type":"polygon","color":"DEADBEEF","vertices":[[0,0],[1,0],[1,1]]}
                ]}
              ]}
            ]}"#,
        )
        .unwrap();
        assert_eq!(
            m.polygon_sprite_names(),
            vec!["Color_00FF00", "Color_DEADBEEF", "Color_FF0000"],
        );
    }

    #[test]
    fn polygon_sprite_names_empty_when_no_polygons() {
        let m = parse(
            r#"{ "version":1, "combined":[
              {"name":"A","mode":"ui","children":[{"type":"sprite","sprite":"x"}]}
            ]}"#,
        )
        .unwrap();
        assert!(m.polygon_sprite_names().is_empty());
    }

    #[test]
    fn parse_rejects_duplicate_tree_name() {
        let m = parse(
            r#"{ "version":1, "combined":[
              {"name":"X","mode": "ui","children":[{"type":"sprite","sprite":"a"}]},
              {"name":"X","mode": "ui","children":[{"type":"sprite","sprite":"b"}]}
            ]}"#,
        );
        assert!(matches!(m, Err(ManifestError::DuplicateName(n)) if n == "X"));
    }
}
