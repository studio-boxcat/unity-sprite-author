//! Tree mutation operations + per-node edits. Extracted from `app.rs` so the
//! type surface stays separable from the eframe App state. `App::apply_op`
//! consumes these (record undo, dispatch the mutation).
//!
//! Every UI surface that wants to mutate the manifest produces one of these
//! and pushes onto `App.pending_ops`. The end of frame drains the queue and
//! applies in order. This indirection is what lets us (a) coalesce drag
//! chains for undo, (b) iterate the tree while collecting mutations without
//! borrow conflicts, and (c) unit-test ops headlessly.

use crate::doc::NodePath;
use unity_sprite_author::manifest::{DrawMode, Graphic, Node, SpriteMethod};

#[derive(Debug, Clone)]
pub enum TreeOp {
    AddChild { parent: NodePath, graphic: NewGraphic },
    Duplicate(NodePath),
    Delete(NodePath),
    MoveSibling { path: NodePath, delta: i32 },
    /// Move `src` to `dst_parent.children[dst_idx]` — covers in-parent
    /// reorder and reparenting. Drag-and-drop in the tree panel uses this.
    MoveTo { src: NodePath, dst_parent: NodePath, dst_idx: usize },
    /// Replace the entire node's graphic discriminator (transform preserved).
    SetGraphic { path: NodePath, graphic: Option<NewGraphic> },
    /// Mutate a specific node field. The granular variants give us
    /// fine-grained undo coalescing for drag chains.
    Edit { path: NodePath, edit: NodeEdit },
}

#[derive(Debug, Clone)]
pub enum NewGraphic {
    Container,
    Sprite,
    /// 4-vert axis-aligned quad polygon with explicit `[0,2,3,3,1,0]` indices.
    /// Edited via the rect inspector + 9-way canvas handles.
    Rect,
    Polygon,
    SpriteRenderer,
}

#[derive(Debug, Clone)]
pub enum NodeEdit {
    Name(String),
    Pos([f32; 2]),
    Size(Option<[f32; 2]>),
    Pivot(Option<[f32; 2]>),
    Scale([f32; 2]),
    Rot(f32),
    SpriteRef(String),
    SpriteMethod(SpriteMethod),
    SpriteBorderMult(f32),
    SpriteFlipX(bool),
    SpriteFlipY(bool),
    PolygonColor(String),
    PolygonVertex { idx: usize, value: [f32; 2] },
    PolygonAddVertex,
    /// Insert a vertex at `idx` (shifts existing vertices [idx..] up by 1).
    PolygonInsertVertex { idx: usize, value: [f32; 2] },
    PolygonRemoveVertex(usize),
    PolygonTriangles(Option<Vec<u16>>),
    /// Update a rect-shape polygon's 4 vertices from width/height (centered).
    PolygonRectSize { width: f32, height: f32 },
    SpriteRendererSprite(String),
    SpriteRendererDrawMode(DrawMode),
}

/// Build a fresh `Node` for a newly-added child. Position + transform fields
/// default to identity; the graphic is whatever `NewGraphic` selected.
pub fn new_node(g: NewGraphic) -> Node {
    Node {
        name: String::new(),
        pos: [0.0, 0.0],
        size: None,
        pivot: None,
        scale: [1.0, 1.0],
        rot_deg_ccw: 0.0,
        graphic: default_graphic(g),
        children: Vec::new(),
    }
}

/// Initial graphic body for a node when a kind is selected.
pub fn default_graphic(g: NewGraphic) -> Option<Graphic> {
    match g {
        NewGraphic::Container => None,
        NewGraphic::Sprite => Some(Graphic::Sprite {
            sprite: String::new(),
            method: SpriteMethod::Id,
            border_mult: 1.0,
            flip_x: false,
            flip_y: false,
        }),
        NewGraphic::Rect => Some(Graphic::Polygon {
            polygon_sprite: "Color_FFFFFF".into(),
            vertices: vec![[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]],
            triangles: Some(vec![0, 2, 3, 3, 1, 0]),
        }),
        NewGraphic::Polygon => Some(Graphic::Polygon {
            polygon_sprite: "Color_FFFFFF".into(),
            vertices: vec![[0.0, 1.0], [-1.0, -1.0], [1.0, -1.0]],
            triangles: None,
        }),
        NewGraphic::SpriteRenderer => Some(Graphic::SpriteRenderer {
            sprite: String::new(),
            draw_mode: DrawMode::Simple,
        }),
    }
}

/// Apply a granular field edit to `node` in place. Edits that don't match the
/// node's graphic kind are silently no-op (the UI is supposed to only emit
/// edits compatible with the current kind).
pub fn apply_edit(node: &mut Node, edit: NodeEdit) {
    match edit {
        NodeEdit::Name(s) => node.name = s,
        NodeEdit::Pos(v) => node.pos = v,
        NodeEdit::Size(v) => node.size = v,
        NodeEdit::Pivot(v) => node.pivot = v,
        NodeEdit::Scale(v) => node.scale = v,
        NodeEdit::Rot(v) => node.rot_deg_ccw = v,
        NodeEdit::SpriteRef(name) => {
            if let Some(Graphic::Sprite { sprite, .. }) = &mut node.graphic {
                *sprite = name;
            }
        }
        NodeEdit::SpriteMethod(m) => {
            if let Some(Graphic::Sprite { method, .. }) = &mut node.graphic {
                *method = m;
            }
        }
        NodeEdit::SpriteBorderMult(b) => {
            if let Some(Graphic::Sprite { border_mult, .. }) = &mut node.graphic {
                *border_mult = b;
            }
        }
        NodeEdit::SpriteFlipX(b) => {
            if let Some(Graphic::Sprite { flip_x, .. }) = &mut node.graphic {
                *flip_x = b;
            }
        }
        NodeEdit::SpriteFlipY(b) => {
            if let Some(Graphic::Sprite { flip_y, .. }) = &mut node.graphic {
                *flip_y = b;
            }
        }
        NodeEdit::PolygonColor(name) => {
            if let Some(Graphic::Polygon { polygon_sprite, .. }) = &mut node.graphic {
                *polygon_sprite = name;
            }
        }
        NodeEdit::PolygonVertex { idx, value } => {
            if let Some(Graphic::Polygon { vertices, .. }) = &mut node.graphic {
                if let Some(v) = vertices.get_mut(idx) {
                    *v = value;
                }
            }
        }
        NodeEdit::PolygonAddVertex => {
            if let Some(Graphic::Polygon { vertices, .. }) = &mut node.graphic {
                let last = vertices.last().copied().unwrap_or([0.0, 0.0]);
                vertices.push(last);
            }
        }
        NodeEdit::PolygonInsertVertex { idx, value } => {
            if let Some(Graphic::Polygon { vertices, triangles, .. }) = &mut node.graphic {
                let i = idx.min(vertices.len());
                vertices.insert(i, value);
                // Inserting a vertex invalidates explicit triangle indices;
                // fall back to ear-clip until the user authors new tris.
                *triangles = None;
            }
        }
        NodeEdit::PolygonRemoveVertex(idx) => {
            if let Some(Graphic::Polygon { vertices, .. }) = &mut node.graphic {
                if idx < vertices.len() && vertices.len() > 3 {
                    vertices.remove(idx);
                }
            }
        }
        NodeEdit::PolygonTriangles(t) => {
            if let Some(Graphic::Polygon { triangles, .. }) = &mut node.graphic {
                *triangles = t;
            }
        }
        NodeEdit::PolygonRectSize { width, height } => {
            if let Some(Graphic::Polygon { vertices, triangles, .. }) = &mut node.graphic {
                let hw = width * 0.5;
                let hh = height * 0.5;
                *vertices = vec![[-hw, -hh], [hw, -hh], [hw, hh], [-hw, hh]];
                *triangles = Some(vec![0, 2, 3, 3, 1, 0]);
            }
        }
        NodeEdit::SpriteRendererSprite(name) => {
            if let Some(Graphic::SpriteRenderer { sprite, .. }) = &mut node.graphic {
                *sprite = name;
            }
        }
        NodeEdit::SpriteRendererDrawMode(m) => {
            if let Some(Graphic::SpriteRenderer { draw_mode, .. }) = &mut node.graphic {
                *draw_mode = m;
            }
        }
    }
}

/// True when the op is part of a drag chain — coalesces for undo. Pos and
/// PolygonVertex edits fire many times per second during a drag and share
/// one snapshot.
pub fn is_drag_edit(op: &TreeOp) -> bool {
    matches!(op, TreeOp::Edit { edit: NodeEdit::Pos(_) | NodeEdit::PolygonVertex { .. }, .. })
}

/// Doc index the op targets — used by the undo recorder to scope snapshots.
pub fn op_doc(op: &TreeOp) -> usize {
    match op {
        TreeOp::AddChild { parent, .. } => parent.doc,
        TreeOp::Duplicate(p) | TreeOp::Delete(p) | TreeOp::MoveSibling { path: p, .. }
            | TreeOp::SetGraphic { path: p, .. } => p.doc,
        TreeOp::MoveTo { src, .. } => src.doc,
        TreeOp::Edit { path, .. } => path.doc,
    }
}
