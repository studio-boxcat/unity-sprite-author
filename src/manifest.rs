// Unified `.tps.fab.json` / `.tps.mesh.json` manifest — v3 schema.
//
// Replaces the two flat-array schemas (`fab::Manifest` + `mesh_manifest::MeshManifest`)
// with a single tree-shaped schema mirroring pspec (`tools/pspec` in meow-tower).
// Each `Tree` is one authored output (a CSA-published Sprite or an SMA-published
// Mesh); children are GameObjects whose transforms compose down the tree.
//
// > **Related:** [[fab.md]], [[sma-migration.md]], pspec orientation
//
// JSON shape:
//
//   {
//     "version": 1,
//     "trees": [
//       {
//         "name": "Silloutte1",
//         "scale": 0.01,                       // root's `scaleFactor` (CSA) or 1.0 (SMA)
//         "rootAnchored": [141.8, 370.875],    // optional, only matters for FMA residue
//         "mode": "ui",                      // or { "type":"sma", "fileId":…, "outputPath":…, "usedInCanvas":… }
//         "children": [
//           {
//             "name": "Image",
//             "pos": [0, -22.25],
//             "sizeDelta": [212.5, 545],
//             "pivot": [0.5, 0.5],
//             "type":"polygon", "color":"32264DBD", "vertices":[[…]]
//           },
//           {
//             "name": "B",
//             "pos": [0, -294.75],
//             "sizeDelta": [212.5, 17.5],
//             "pivot": [0.5, 1],
//             "type":"sprite", "sprite":"Mansion_…__B", "method":"MX"
//           },
//           …
//         ]
//       }
//     ]
//   }
//
// Defaults per node:
//   pos    = [0, 0]
//   sizeDelta = [0, 0]   (only matters for size-fitted methods; SMA leaves it 0)
//   pivot  = [0.5, 0.5]
//   scale  = 1.0         (uniform; per-axis `[x, y]` for X/Y flip)
//   children = []
//   graphic = none (interior nodes carry only a transform)
//
// Defaults per graphic:
//   sprite:  method = "ID", uiScale = 100, drawMode = "simple"
//   polygon: triangles = ear-clip
//   sprite-renderer (SMA): drawMode = "simple", flipX/flipY = false

#![allow(dead_code)]

use std::collections::HashSet;
use std::fmt;

#[derive(Debug, PartialEq)]
pub struct Manifest {
    pub trees: Vec<Tree>,
}

#[derive(Debug, PartialEq)]
pub struct Tree {
    pub name: String,
    /// CSA `_scaleFactor` (0.01 typical) or SMA world-unit (1.0 typical).
    pub scale: f32,
    /// CSA root's `RectTransform.anchoredPosition`. Defaults to (0, 0). Only
    /// matters for FMA-residue reproduction when non-origin.
    pub root_anchored: [f32; 2],
    pub output: Output,
    pub root: Node,
}

#[derive(Debug, PartialEq)]
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

#[derive(Debug, PartialEq)]
pub struct Node {
    pub name: String,
    pub pos: [f32; 2],
    pub size_delta: [f32; 2],
    pub pivot: [f32; 2],
    /// Per-axis uniform/non-uniform scale. `[1, 1]` = identity.
    pub scale: [f32; 2],
    pub rot_deg: f32,
    pub graphic: Option<Graphic>,
    pub children: Vec<Node>,
}

#[derive(Debug, PartialEq)]
pub enum Graphic {
    /// UIIcon / UISlice (CSA hierarchy) or atlas-sprite under SMA.
    Sprite {
        sprite: String,
        method: SpriteMethod,
        ui_scale: f32,
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
    SpriteRenderer {
        sprite: String,
        draw_mode: DrawMode,
        size: Option<[f32; 2]>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SpriteMethod {
    Id,
    Mx, My, Mxy,
    Tx, Ty, TxMc3,
    R1c3, R3c3, R3c3Nf,
    MxR1c3, MxR1c4, MxR3c2, MxR3c3, MxR3c4, MxR3c6,
    MyR2c2, MyR2c3, MyR3c1, MyR3c2, MyR3c3,
    MxyR3c3, MxyR3c3Nf,
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
            size_delta: [0.0, 0.0],
            pivot: [0.5, 0.5],
            scale: [1.0, 1.0],
            rot_deg: 0.0,
            graphic: None,
            children,
        };
        // Default tree-level scale is 1.0 across modes. CSA prefabs that
        // need the historical 0.01 canvas factor declare `scale: 0.01`
        // explicitly. (Earlier mode-dependent default was confusing —
        // unify to a single neutral default.)
        trees.push(Tree {
            name: t.name,
            scale: t.scale.unwrap_or(1.0),
            root_anchored: t.root_anchored.unwrap_or([0.0, 0.0]),
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
        size_delta: raw.size_delta.unwrap_or([0.0, 0.0]),
        pivot: raw.pivot.unwrap_or([0.5, 0.5]),
        scale,
        rot_deg: raw.rot_deg.unwrap_or(0.0),
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
                ui_scale: raw.ui_scale.unwrap_or(100.0),
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
                size: raw.size,
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
        pub trees: Vec<Tree>,
    }

    /// Per-tree input. Replaces the old `{ output, root: { children } }`
    /// shape with flat `{ mode, children }` plus optional SMA fields at
    /// the tree level. `scale` is in 0..1 range (CSA canvasScale: 0.01;
    /// SMA worldScale: 1.0).
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    pub struct Tree {
        pub name: String,
        pub mode: String, // "ui" | "sma-canvas" | "sma-renderer"
        pub scale: Option<f32>,
        pub root_anchored: Option<[f32; 2]>,
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
        pub size_delta: Option<[f32; 2]>,
        pub pivot: Option<[f32; 2]>,
        pub scale: Option<ScaleSpec>,
        pub rot_deg: Option<f32>,

        // Graphic discriminator + fields:
        #[serde(rename = "type")]
        pub kind: Option<String>,
        // sprite + sprite-renderer
        pub sprite: Option<String>,
        // sprite only
        pub method: Option<String>,
        pub ui_scale: Option<f32>,
        pub border_mult: Option<f32>,
        pub flip_x: Option<bool>,
        pub flip_y: Option<bool>,
        // polygon
        pub color: Option<String>,
        pub vertices: Option<Vec<[f32; 2]>>,
        pub triangles: Option<Vec<u16>>,
        // sprite-renderer
        pub draw_mode: Option<String>,
        pub size: Option<[f32; 2]>,

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
    /// Composed rotation in degrees (sum of node `rot_deg` along the path).
    pub world_rot_deg: f32,
    /// The leaf node's `size_delta` (untransformed). Size-fitted methods
    /// (`R*`, `MX_R*`, `MY_R*`, tilers) consume this directly.
    pub size_delta: [f32; 2],
    /// The leaf node's `pivot` (untransformed).
    pub pivot: [f32; 2],
    pub graphic: &'a Graphic,
}

/// Walk a tree, composing each node's transform with its ancestors' so the
/// leaves end up with root-local coords. Mirrors Unity's RectTransform
/// cascade for the common center-anchored case — each child's world pos is
/// its parent's world pos plus a center-of-rect offset
/// `(0.5 − parent.pivot) × parent.size_delta` (the "Body shift" seen on
/// AlbumSticker_Ghost1), plus the child's own anchored `pos`. Returns
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
    parent_size_delta: [f32; 2],
    parent_pivot: [f32; 2],
    out: &mut Vec<ResolvedLeaf<'a>>,
) {
    // Center offset: where parent's rect-center sits, relative to parent's
    // local origin (which is the pivot point in Unity's RectTransform). Only
    // applies when the node has a parent with a meaningful size_delta.
    let parent_center_offset = [
        (0.5 - parent_pivot[0]) * parent_size_delta[0],
        (0.5 - parent_pivot[1]) * parent_size_delta[1],
    ];
    let world_pos = [
        parent_world_pos[0] + parent_center_offset[0] + node.pos[0],
        parent_world_pos[1] + parent_center_offset[1] + node.pos[1],
    ];
    let world_scale = [
        parent_world_scale[0] * node.scale[0],
        parent_world_scale[1] * node.scale[1],
    ];
    let world_rot = parent_world_rot + node.rot_deg;

    if let Some(g) = &node.graphic {
        out.push(ResolvedLeaf {
            world_pos,
            world_scale,
            world_rot_deg: world_rot,
            size_delta: node.size_delta,
            pivot: node.pivot,
            graphic: g,
        });
    }
    for c in &node.children {
        walk_node(
            c,
            world_pos,
            world_scale,
            world_rot,
            node.size_delta,
            node.pivot,
            out,
        );
    }
}

// ---------------------------------------------------------------------------
// Bridge: v3 Tree → existing flat `fab::Combined` / `mesh_emit::MeshCombined`
// so the byte-exact emit pipeline keeps consuming a single typed AST.

#[derive(Debug)]
pub enum BridgeError {
    /// Tree's `output` doesn't match the requested adapter (e.g. SMA tree
    /// fed into `to_fab_combined`).
    OutputMismatch { tree: String, expected: &'static str },
    /// Tree contains a graphic incompatible with the output (e.g. polygon
    /// under SMA, or sprite-renderer under CSA).
    GraphicMismatch { tree: String, reason: &'static str },
    /// Method requires `size` (R*/MX_R*/MY_R*/MXY_R*/TX*/TY*) but the
    /// node's `size_delta` is zero. v3 manifests must declare size_delta
    /// on size-fitted UISlice leaves.
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
                "tree {tree:?} sprite {sprite:?}: size-fitted method requires non-zero size_delta"
            ),
        }
    }
}

impl std::error::Error for BridgeError {}

/// Convert a `Tree` with `output: "csa"` into a `fab::Combined`, the
/// flat-shape struct the existing `combine::build_combined` consumes.
pub fn to_fab_combined(tree: &Tree) -> Result<crate::fab::Combined, BridgeError> {
    use crate::fab;

    if tree.output != Output::Csa {
        return Err(BridgeError::OutputMismatch {
            tree: tree.name.clone(),
            expected: "csa",
        });
    }

    let leaves = walk(tree);
    let mut parts: Vec<fab::Part> = Vec::with_capacity(leaves.len());
    for leaf in leaves {
        match leaf.graphic {
            Graphic::Sprite {
                sprite,
                method,
                ui_scale,
                border_mult,
                ..
            } => {
                let fab_method = map_method(*method);
                let size = if fab_method.requires_size_v3() {
                    let sd = leaf.size_delta;
                    if sd[0] == 0.0 || sd[1] == 0.0 {
                        return Err(BridgeError::ZeroSizeForSliceMethod {
                            tree: tree.name.clone(),
                            sprite: sprite.clone(),
                        });
                    }
                    Some((sd[0], sd[1]))
                } else {
                    None
                };
                parts.push(fab::Part::AtlasSprite {
                    sprite: sprite.clone(),
                    method: fab_method,
                    size,
                    part_pivot: leaf.pivot,
                    border_mult: *border_mult,
                    affine: fab::Affine {
                        tx: 0.0,
                        ty: 0.0,
                        sx: leaf.world_scale[0],
                        sy: leaf.world_scale[1],
                        rot_deg: leaf.world_rot_deg,
                    },
                    ui_scale: *ui_scale,
                    offset: leaf.world_pos,
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
                    affine: fab::Affine {
                        tx: 0.0,
                        ty: 0.0,
                        sx: leaf.world_scale[0],
                        sy: leaf.world_scale[1],
                        rot_deg: leaf.world_rot_deg,
                    },
                    // UISolid under CanvasSpriteAuthor takes no per-part
                    // scale-factor, but the canvas-chain still uses
                    // ui_scale = 1.0 (identity) so the matrix-style op order
                    // preserves the FMA residue. Mirrors the v1 schema's
                    // polygon emit.
                    ui_scale: 1.0,
                    offset: leaf.world_pos,
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
        canvas_scale: tree.scale,
        root_anchored: tree.root_anchored,
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

// Mirror of `fab::Method::requires_size` so the bridge can decide
// whether to thread size_delta through. Kept private to manifest.rs.
trait MethodCapV3 {
    fn requires_size_v3(self) -> bool;
}
impl MethodCapV3 for crate::fab::Method {
    fn requires_size_v3(self) -> bool {
        use crate::fab::Method as M;
        !matches!(self, M::Id | M::Mx | M::My | M::Mxy)
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

    let leaves = walk(tree);
    let mut renderers: Vec<mm::MeshRenderer> = Vec::with_capacity(leaves.len());
    for leaf in leaves {
        let g = match leaf.graphic {
            Graphic::SpriteRenderer {
                sprite,
                draw_mode,
                size,
            } => (sprite, draw_mode, size),
            _ => {
                return Err(BridgeError::GraphicMismatch {
                    tree: tree.name.clone(),
                    reason: "sma output only accepts sprite-renderer graphics",
                })
            }
        };
        let (sprite, draw_mode, size) = g;
        let dm = match draw_mode {
            DrawMode::Simple => mm::DrawMode::Simple,
            DrawMode::Tiled => mm::DrawMode::Tiled,
        };
        // localToRoot = composed 2D affine row-major [m00, m01, m02, m03, m10, m11, m12, m13].
        // The walker gives us world_scale (composed across ancestors), world_pos
        // (composed translation), and world_rot_deg (composed rotation). Build
        // the matrix accordingly. Flip is folded into world_scale's sign.
        let (sin_t, cos_t) = leaf.world_rot_deg.to_radians().sin_cos();
        let m00 = cos_t * leaf.world_scale[0];
        let m01 = -sin_t * leaf.world_scale[1];
        let m10 = sin_t * leaf.world_scale[0];
        let m11 = cos_t * leaf.world_scale[1];
        let l2r = [m00, m01, 0.0, leaf.world_pos[0], m10, m11, 0.0, leaf.world_pos[1]];

        renderers.push(mm::MeshRenderer {
            sprite: sprite.clone(),
            // flipX/Y is folded into world_scale's sign by the walker, so the
            // mesh_manifest level keeps these false. mesh_emit's `build_mesh`
            // applies flip BEFORE the matrix, but if the matrix already
            // carries the sign in m00/m11, that's a double-flip. We undo
            // here: extract sign from world_scale, but pass false for flip
            // and absorb sign into l2r.
            flip_x: false,
            flip_y: false,
            draw_mode: dm,
            size: *size,
            local_to_root: l2r,
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
              "trees": [{
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
        // Tree-level scale defaults to 1.0 across modes; CSA prefabs that
        // need the historical canvasScale declare `scale: 0.01` explicitly.
        assert_eq!(t.scale, 1.0);
        assert_eq!(t.root.children.len(), 1);
        match &t.root.children[0].graphic.as_ref().unwrap() {
            Graphic::Sprite { method, ui_scale, .. } => {
                assert_eq!(*method, SpriteMethod::Id);
                assert_eq!(*ui_scale, 100.0);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn parse_polygon_color_6_hex() {
        let m = parse(
            r#"{ "version": 1, "trees": [{
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
            r#"{ "version": 1, "trees": [{
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
            r#"{ "version": 1, "trees": [{
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
            Graphic::SpriteRenderer { sprite, draw_mode, size } => {
                assert_eq!(sprite, "foo");
                assert_eq!(*draw_mode, DrawMode::Simple);
                assert!(size.is_none());
            }
            _ => panic!(),
        }
    }

    #[test]
    fn parse_scale_uniform_and_per_axis() {
        let m = parse(
            r#"{ "version": 1, "trees": [{
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
            r#"{ "version": 1, "trees": [{
              "name": "X", "mode": "ui",
              "children": [{
                "name": "Body",
                "pivot": [0.5, 0.4515571],
                "sizeDelta": [154, 181],
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
        assert_eq!(body.pivot, [0.5, 0.4515571]);
        assert_eq!(body.children.len(), 1);
        let sp = &body.children[0];
        assert_eq!(sp.pos, [-69.8, -36.7]);
    }

    #[test]
    fn parse_rejects_unknown_top_level_field() {
        let m = parse(r#"{ "version": 1, "trees": [], "extra": 0 }"#);
        assert!(matches!(m, Err(ManifestError::Json(_))));
    }

    #[test]
    fn parse_rejects_bad_color() {
        let m = parse(
            r#"{ "version":1, "trees":[{
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
            r#"{ "version":1, "trees":[{
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
            r#"{ "version":1, "trees":[{
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
            r#"{ "version":1, "trees":[{
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
            r#"{ "version":1, "trees":[{
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
            r#"{ "version":1, "trees":[{
              "name":"X", "mode": "ui",
              "children":[{
                "name":"Body",
                "pivot":[0.5, 0.4515571],
                "sizeDelta":[154, 181],
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
            r#"{ "version":1, "trees":[{
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
            r#"{ "version":1, "trees":[{
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
            r#"{ "version":1, "trees":[{
              "name":"X","mode": "ui",
              "children":[
                {"type":"sprite","sprite":"foo"}
              ]
            }]}"#,
        );
        let c = to_fab_combined(&m.trees[0]).unwrap();
        assert_eq!(c.name, "X");
        // Default canvas_scale is 1.0 — CSA prefabs set 0.01 explicitly.
        assert_eq!(c.canvas_scale, 1.0);
        assert_eq!(c.parts.len(), 1);
        match &c.parts[0] {
            crate::fab::Part::AtlasSprite { sprite, method, ui_scale, size, offset, .. } => {
                assert_eq!(sprite, "foo");
                assert_eq!(*method, crate::fab::Method::Id);
                assert_eq!(*ui_scale, 100.0);
                assert!(size.is_none());
                assert_eq!(*offset, [0.0, 0.0]);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn bridge_to_fab_polygon_with_offset() {
        let m = parse_single(
            r#"{ "version":1, "trees":[{
              "name":"X","mode": "ui","scale":0.01,
              "children":[{
                "pos":[10, 20],
                "type":"polygon","color":"32264D","vertices":[[0,0],[1,0],[1,1]]
              }]
            }]}"#,
        );
        let c = to_fab_combined(&m.trees[0]).unwrap();
        assert_eq!(c.canvas_scale, 0.01);
        match &c.parts[0] {
            crate::fab::Part::Polygon { polygon_sprite, offset, .. } => {
                assert_eq!(polygon_sprite, "Color_32264D");
                assert_eq!(*offset, [10.0, 20.0]);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn bridge_propagates_scale_as_sx_sy() {
        // scale: [-1, 1] should flow into Affine.sx/sy.
        let m = parse_single(
            r#"{ "version":1, "trees":[{
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
    fn bridge_size_fitted_method_takes_size_delta() {
        let m = parse_single(
            r#"{ "version":1, "trees":[{
              "name":"X","mode": "ui",
              "children":[{
                "sizeDelta":[200, 150],
                "type":"sprite","sprite":"a","method":"R3C3"
              }]
            }]}"#,
        );
        let c = to_fab_combined(&m.trees[0]).unwrap();
        match &c.parts[0] {
            crate::fab::Part::AtlasSprite { method, size, .. } => {
                assert_eq!(*method, crate::fab::Method::R3c3);
                assert_eq!(*size, Some((200.0, 150.0)));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn bridge_rejects_size_fitted_with_zero_size() {
        let m = parse_single(
            r#"{ "version":1, "trees":[{
              "name":"X","mode": "ui",
              "children":[
                {"type":"sprite","sprite":"a","method":"R3C3"}
              ]
            }]}"#,
        );
        let err = to_fab_combined(&m.trees[0]).unwrap_err();
        assert!(matches!(err, BridgeError::ZeroSizeForSliceMethod { .. }));
    }

    #[test]
    fn bridge_rejects_sprite_renderer_in_csa() {
        let m = parse_single(
            r#"{ "version":1, "trees":[{
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
            r#"{ "version":1, "trees":[{
              "name":"X",
              "mode":"sma-canvas", "fileId":1,"outputPath":"o.asset",
              "children":[{"type":"spriteRenderer","sprite":"a"}]
            }]}"#,
        );
        let err = to_fab_combined(&m.trees[0]).unwrap_err();
        assert!(matches!(err, BridgeError::OutputMismatch { .. }));
    }

    #[test]
    fn bridge_silloutte3_root_anchored_threads_through() {
        // The Silloutte3 case: root has non-origin anchored position, threads
        // into Combined.root_anchored so compute_m13_axis fires the FMA-fused
        // residual computation downstream.
        let m = parse_single(
            r#"{ "version":1, "trees":[{
              "name":"Silloutte3","mode": "ui","scale":0.01,
              "rootAnchored":[141.8, 370.875],
              "children":[
                {"type":"sprite","sprite":"a","method":"MX"}
              ]
            }]}"#,
        );
        let c = to_fab_combined(&m.trees[0]).unwrap();
        assert_eq!(c.root_anchored, [141.8, 370.875]);
    }

    #[test]
    fn bridge_to_mesh_minimal_simple() {
        let m = parse_single(
            r#"{ "version":1, "trees":[{
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
        assert_eq!(r.sprite, "foo");
        assert_eq!(r.draw_mode, crate::mesh_manifest::DrawMode::Simple);
        assert_eq!(r.local_to_root, [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    }

    #[test]
    fn bridge_to_mesh_translation_into_l2r() {
        let m = parse_single(
            r#"{ "version":1, "trees":[{
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
            r#"{ "version":1, "trees":[{
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
            r#"{ "version":1, "trees":[{
              "name":"X",
              "mode":"sma-renderer", "fileId":1,"outputPath":"o.asset",
              "children":[
                {"type":"spriteRenderer","sprite":"brick","drawMode":"tiled","size":[4.05,1.0]}
              ]
            }]}"#,
        );
        let mc = to_mesh_combined(&m.trees[0]).unwrap();
        let r = &mc.renderers[0];
        assert_eq!(r.draw_mode, crate::mesh_manifest::DrawMode::Tiled);
        assert_eq!(r.size, Some([4.05, 1.0]));
    }

    #[test]
    fn bridge_to_mesh_rejects_csa_tree() {
        let m = parse_single(
            r#"{ "version":1, "trees":[{
              "name":"X","mode": "ui",
              "children":[{"type":"sprite","sprite":"a"}]
            }]}"#,
        );
        let err = to_mesh_combined(&m.trees[0]).unwrap_err();
        assert!(matches!(err, BridgeError::OutputMismatch { .. }));
    }

    #[test]
    fn bridge_to_mesh_rejects_polygon_in_sma() {
        let m = parse_single(
            r#"{ "version":1, "trees":[{
              "name":"X",
              "mode":"sma-canvas", "fileId":1,"outputPath":"o.asset",
              "children":[
                {"type":"polygon","color":"112233","vertices":[[0,0],[1,0],[1,1]]}
              ]
            }]}"#,
        );
        let err = to_mesh_combined(&m.trees[0]).unwrap_err();
        assert!(matches!(err, BridgeError::GraphicMismatch { .. }));
    }

    #[test]
    fn parse_rejects_duplicate_tree_name() {
        let m = parse(
            r#"{ "version":1, "trees":[
              {"name":"X","mode": "ui","children":[{"type":"sprite","sprite":"a"}]},
              {"name":"X","mode": "ui","children":[{"type":"sprite","sprite":"b"}]}
            ]}"#,
        );
        assert!(matches!(m, Err(ManifestError::DuplicateName(n)) if n == "X"));
    }
}
