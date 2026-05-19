//! Right panel: per-node editor. Transform widgets + graphic-specific fields.
//! Mutations are enqueued as `TreeOp::Edit` so we don't borrow the doc mutably
//! while the inspector runs.

use crate::app::App;
use crate::ops::{NewGraphic, NodeEdit, TreeOp};
use crate::doc::NodePath;
use crate::picker::{Picker, PickerKind};
use crate::serialize;
use unity_sprite_author::manifest::{DrawMode, Graphic, Node, Output, SpriteMethod};

pub fn show(ui: &mut egui::Ui, app: &mut App) {
    let Some(path) = app.selection.primary().cloned() else {
        ui.label("(no selection)");
        return;
    };
    if app.selection.len() > 1 {
        ui.label(format!(
            "{} selected — inspecting primary ({})",
            app.selection.len(),
            crate::app::node_label(
                &path
                    .resolve(&app.docs.get(path.doc).unwrap().manifest)
                    .cloned()
                    .unwrap_or_else(|| empty_node())
            )
        ));
        ui.separator();
    }
    if path.child_chain.is_empty() {
        show_tree_inspector(ui, app, &path);
        return;
    }
    let Some(doc) = app.docs.get(path.doc) else {
        ui.label("(stale selection)");
        return;
    };
    let Some(node) = path.resolve(&doc.manifest) else {
        ui.label("(stale selection)");
        return;
    };
    let node = node.clone();
    show_transform(ui, app, &path, &node);
    ui.separator();
    show_graphic(ui, app, &path, &node);
}

fn show_tree_inspector(ui: &mut egui::Ui, app: &mut App, path: &NodePath) {
    let Some(doc) = app.docs.get(path.doc) else { return; };
    let Some(tree) = doc.manifest.trees.get(path.tree) else { return; };
    ui.heading(format!("{}  ·  {}", tree.name, crate::app::mode_label(&tree.output)));
    ui.label(format!("canvas_scale_implicit: {}", tree.output.canvas_scale_implicit()));
    if let Output::Sma {
        file_id, output_path, used_in_canvas, keep_vertices, keep_indices,
    } = &tree.output {
        ui.separator();
        ui.label(format!("fileId: {file_id}"));
        ui.label(format!("outputPath: {output_path}"));
        ui.label(format!("usedInCanvas: {used_in_canvas}"));
        ui.label(format!("keepVertices: {keep_vertices}"));
        ui.label(format!("keepIndices: {keep_indices}"));
    }
    ui.separator();
    ui.label(format!("{} child node(s)", tree.root.children.len()));
}

fn show_transform(ui: &mut egui::Ui, app: &mut App, path: &NodePath, node: &Node) {
    ui.label(egui::RichText::new("Transform").strong());

    // name
    let mut name = node.name.clone();
    ui.horizontal(|ui| {
        ui.label("name");
        if ui.text_edit_singleline(&mut name).changed() {
            app.pending_ops.push(TreeOp::Edit { path: path.clone(), edit: NodeEdit::Name(name.clone()) });
        }
    });

    // pos
    let mut pos = node.pos;
    ui.horizontal(|ui| {
        ui.label("pos");
        let dx = ui.add(egui::DragValue::new(&mut pos[0]).speed(0.1));
        let dy = ui.add(egui::DragValue::new(&mut pos[1]).speed(0.1));
        if dx.changed() || dy.changed() {
            app.pending_ops.push(TreeOp::Edit { path: path.clone(), edit: NodeEdit::Pos(pos) });
        }
    });

    // size (optional). Toggle-on while value is (0, 0) pre-fills from the
    // sprite's natural atlas rect — both knob and slice-method math depend
    // on a non-zero size, and "inherit then customize" is the common edit.
    let mut has_size = node.size.is_some();
    let mut size = node.size.unwrap_or([0.0, 0.0]);
    ui.horizontal(|ui| {
        ui.label("size");
        let toggled = ui.checkbox(&mut has_size, "");
        if toggled.changed() {
            let new_size = if has_size {
                let nat = natural_size_for_sprite(app, path, &node);
                if size == [0.0, 0.0] { Some(nat.unwrap_or([16.0, 16.0])) }
                else { Some(size) }
            } else { None };
            app.pending_ops.push(TreeOp::Edit {
                path: path.clone(),
                edit: NodeEdit::Size(new_size),
            });
        }
        if has_size {
            let dx = ui.add(egui::DragValue::new(&mut size[0]).speed(0.5));
            let dy = ui.add(egui::DragValue::new(&mut size[1]).speed(0.5));
            if dx.changed() || dy.changed() {
                app.pending_ops.push(TreeOp::Edit { path: path.clone(), edit: NodeEdit::Size(Some(size)) });
            }
        } else {
            ui.label("(inherit)");
        }
    });

    // pivot (optional)
    let mut has_pivot = node.pivot.is_some();
    let mut pivot = node.pivot.unwrap_or([0.5, 0.5]);
    ui.horizontal(|ui| {
        ui.label("pivot");
        let toggled = ui.checkbox(&mut has_pivot, "");
        if toggled.changed() {
            app.pending_ops.push(TreeOp::Edit {
                path: path.clone(),
                edit: NodeEdit::Pivot(if has_pivot { Some(pivot) } else { None }),
            });
        }
        if has_pivot {
            let dx = ui.add(egui::DragValue::new(&mut pivot[0]).speed(0.01).range(0.0..=1.0));
            let dy = ui.add(egui::DragValue::new(&mut pivot[1]).speed(0.01).range(0.0..=1.0));
            if dx.changed() || dy.changed() {
                app.pending_ops.push(TreeOp::Edit { path: path.clone(), edit: NodeEdit::Pivot(Some(pivot)) });
            }
        } else {
            ui.label("(inherit)");
        }
    });

    let mut scale = node.scale;
    ui.horizontal(|ui| {
        ui.label("scale");
        let dx = ui.add(egui::DragValue::new(&mut scale[0]).speed(0.01));
        let dy = ui.add(egui::DragValue::new(&mut scale[1]).speed(0.01));
        if dx.changed() || dy.changed() {
            app.pending_ops.push(TreeOp::Edit { path: path.clone(), edit: NodeEdit::Scale(scale) });
        }
        if ui.small_button("Reset").on_hover_text("Set to (1, 1)").clicked() && scale != [1.0, 1.0] {
            app.pending_ops.push(TreeOp::Edit { path: path.clone(), edit: NodeEdit::Scale([1.0, 1.0]) });
        }
    });

    // rotation
    let mut rot = node.rot_deg_ccw;
    ui.horizontal(|ui| {
        ui.label("rot (deg CCW)");
        let r = ui.add(egui::DragValue::new(&mut rot).speed(1.0).suffix("°"));
        if r.changed() {
            app.pending_ops.push(TreeOp::Edit { path: path.clone(), edit: NodeEdit::Rot(rot) });
        }
        if ui.small_button("Reset").clicked() && rot != 0.0 {
            app.pending_ops.push(TreeOp::Edit { path: path.clone(), edit: NodeEdit::Rot(0.0) });
        }
    });
}

/// Look up the sprite's natural atlas-rect size (in canvas-pixel units, as
/// the manifest's `size` field expects) for the node's leaf, if any. Returns
/// `None` for polygons / containers / unresolved sprites. Used by the size
/// toggle to pre-fill instead of writing a useless `(0, 0)`.
fn natural_size_for_sprite(
    app: &mut App,
    path: &NodePath,
    node: &unity_sprite_author::manifest::Node,
) -> Option<[f32; 2]> {
    let sprite_name = match &node.graphic {
        Some(Graphic::Sprite { sprite, .. }) | Some(Graphic::SpriteRenderer { sprite, .. }) => sprite.clone(),
        _ => return None,
    };
    if sprite_name.is_empty() { return None; }
    let doc = app.docs.get_mut(path.doc)?;
    let atlas = doc.atlas_mut().as_mut().ok()?;
    let entry = atlas.sprite(&sprite_name)?;
    // Canvas-pixel size — `size` in the manifest pre-bridge units. With ppu=100
    // and canvas_scale=0.01, this equals the atlas rect dimensions; with other
    // ratios it's `rect.w / ppu / canvas_scale` but we don't have `canvas_scale`
    // here (would need the tree's Output). Atlas-rect dimensions are a sane
    // first-pick value that the user can then tweak.
    Some([entry.rect.w as f32, entry.rect.h as f32])
}

fn show_graphic(ui: &mut egui::Ui, app: &mut App, path: &NodePath, node: &Node) {
    ui.label(egui::RichText::new("Graphic").strong());

    // Distinguish rect-shape polygons (4 verts, quad triangles) from
    // free polygons in the dropdown so the user sees the intended shape.
    let current_idx = match &node.graphic {
        None => 0,
        Some(Graphic::Sprite { .. }) => 1,
        Some(Graphic::Polygon { vertices, triangles, .. }) => {
            if is_rect_shape(vertices, triangles.as_deref()) { 2 } else { 3 }
        }
        Some(Graphic::SpriteRenderer { .. }) => 4,
    };
    let kinds = [
        ("container", None),
        ("sprite", Some(NewGraphic::Sprite)),
        ("rect", Some(NewGraphic::Rect)),
        ("polygon", Some(NewGraphic::Polygon)),
        ("spriteRenderer", Some(NewGraphic::SpriteRenderer)),
    ];
    egui::ComboBox::from_id_salt(("graphic_kind", path))
        .selected_text(kinds[current_idx].0)
        .show_ui(ui, |ui| {
            for (i, (label, kind)) in kinds.iter().enumerate() {
                if ui.selectable_label(i == current_idx, *label).clicked() && i != current_idx {
                    app.pending_ops.push(TreeOp::SetGraphic {
                        path: path.clone(),
                        graphic: kind.clone(),
                    });
                }
            }
        });

    match &node.graphic {
        None => {
            ui.label("(pure transform container)");
        }
        Some(Graphic::Sprite { sprite, method, border_mult, flip_x, flip_y }) => {
            show_sprite_fields(ui, app, path, sprite, *method, *border_mult, *flip_x, *flip_y);
        }
        Some(Graphic::Polygon { polygon_sprite, vertices, triangles }) => {
            show_polygon_fields(ui, app, path, polygon_sprite, vertices, triangles.as_deref());
        }
        Some(Graphic::SpriteRenderer { sprite, draw_mode }) => {
            show_sprite_renderer_fields(ui, app, path, sprite, *draw_mode);
        }
    }
}

fn show_sprite_fields(
    ui: &mut egui::Ui,
    app: &mut App,
    path: &NodePath,
    sprite: &str,
    method: SpriteMethod,
    border_mult: f32,
    flip_x: bool,
    flip_y: bool,
) {
    // Sprite reference + thumbnail.
    ui.horizontal(|ui| {
        ui.label("sprite");
        let label = if sprite.is_empty() { "(unset)" } else { sprite };
        if ui.button(label).clicked() {
            app.picker = Some(Picker::new(PickerKind::Sprite, path.clone()));
        }
    });
    show_thumbnail(ui, app, path.doc, sprite);

    // Method dropdown.
    ui.horizontal(|ui| {
        ui.label("method");
        egui::ComboBox::from_id_salt(("method", path))
            .selected_text(serialize::method_str(method))
            .show_ui(ui, |ui| {
                for m in serialize::ALL_METHODS {
                    if ui.selectable_label(*m == method, serialize::method_str(*m)).clicked() && *m != method {
                        app.pending_ops.push(TreeOp::Edit { path: path.clone(), edit: NodeEdit::SpriteMethod(*m) });
                    }
                }
            });
    });
    if method.requires_size() && app.docs.get(path.doc).and_then(|d| path.resolve(&d.manifest)).map_or(false, |n| n.size.is_none()) {
        ui.colored_label(crate::theme::WARN_TEXT, format!("⚠ {} requires `size` or inherits from sprite's natural rect", serialize::method_str(method)));
    }

    // borderMult.
    let mut b = border_mult;
    ui.horizontal(|ui| {
        ui.label("borderMult");
        if ui.add(egui::DragValue::new(&mut b).speed(0.05)).changed() {
            app.pending_ops.push(TreeOp::Edit { path: path.clone(), edit: NodeEdit::SpriteBorderMult(b) });
        }
    });

    // flipX / flipY.
    ui.horizontal(|ui| {
        let mut fx = flip_x;
        if ui.checkbox(&mut fx, "flipX").changed() {
            app.pending_ops.push(TreeOp::Edit { path: path.clone(), edit: NodeEdit::SpriteFlipX(fx) });
        }
        let mut fy = flip_y;
        if ui.checkbox(&mut fy, "flipY").changed() {
            app.pending_ops.push(TreeOp::Edit { path: path.clone(), edit: NodeEdit::SpriteFlipY(fy) });
        }
    });
}

fn show_polygon_fields(
    ui: &mut egui::Ui,
    app: &mut App,
    path: &NodePath,
    polygon_sprite: &str,
    vertices: &[[f32; 2]],
    triangles: Option<&[u16]>,
) {
    // Color picker — opens modal that lists Color_* sprites + RGB hex editor.
    ui.horizontal(|ui| {
        ui.label("color");
        let hex = polygon_sprite.strip_prefix("Color_").unwrap_or(polygon_sprite);
        if ui.button(hex).clicked() {
            app.picker = Some(Picker::new(PickerKind::Color, path.clone()));
        }
        // Color preview swatch.
        if let Some(c) = parse_color_hex(hex) {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::hover());
            ui.painter().rect_filled(rect, 2.0, c);
        }
    });

    // Vertices.
    ui.label(format!("vertices ({})", vertices.len()));
    let mut to_remove: Option<usize> = None;
    for (i, v) in vertices.iter().enumerate() {
        let mut vv = *v;
        ui.horizontal(|ui| {
            ui.label(format!("{i:>3}"));
            let dx = ui.add(egui::DragValue::new(&mut vv[0]).speed(0.5));
            let dy = ui.add(egui::DragValue::new(&mut vv[1]).speed(0.5));
            if dx.changed() || dy.changed() {
                app.pending_ops.push(TreeOp::Edit {
                    path: path.clone(),
                    edit: NodeEdit::PolygonVertex { idx: i, value: vv },
                });
            }
            if ui.small_button("−").on_hover_text("Remove vertex (min 3)").clicked() {
                to_remove = Some(i);
            }
        });
    }
    if let Some(i) = to_remove {
        app.pending_ops.push(TreeOp::Edit { path: path.clone(), edit: NodeEdit::PolygonRemoveVertex(i) });
    }
    if ui.button("+ vertex").clicked() {
        app.pending_ops.push(TreeOp::Edit { path: path.clone(), edit: NodeEdit::PolygonAddVertex });
    }

    // Triangles override.
    ui.horizontal(|ui| {
        let mut has_t = triangles.is_some();
        if ui.checkbox(&mut has_t, "explicit triangles").changed() {
            let new = if has_t {
                // Default to a quad ear-clip when toggled on.
                if vertices.len() == 4 { Some(vec![0, 2, 3, 3, 1, 0]) }
                else { Some(Vec::new()) }
            } else {
                None
            };
            app.pending_ops.push(TreeOp::Edit { path: path.clone(), edit: NodeEdit::PolygonTriangles(new) });
        }
    });
    if let Some(t) = triangles {
        ui.label(format!("indices: {} ({} triangles)", t.len(), t.len() / 3));
        ui.label(egui::RichText::new(format!("{:?}", t)).monospace().size(10.0));
    } else {
        ui.label("(ear-clip auto)");
    }
}

fn show_sprite_renderer_fields(
    ui: &mut egui::Ui,
    app: &mut App,
    path: &NodePath,
    sprite: &str,
    draw_mode: DrawMode,
) {
    ui.horizontal(|ui| {
        ui.label("sprite");
        let label = if sprite.is_empty() { "(unset)" } else { sprite };
        if ui.button(label).clicked() {
            app.picker = Some(Picker::new(PickerKind::Sprite, path.clone()));
        }
    });
    show_thumbnail(ui, app, path.doc, sprite);

    ui.horizontal(|ui| {
        ui.label("drawMode");
        for mode in [DrawMode::Simple, DrawMode::Tiled] {
            let label = serialize::draw_mode_str(mode);
            if ui.selectable_label(mode == draw_mode, label).clicked() && mode != draw_mode {
                app.pending_ops.push(TreeOp::Edit { path: path.clone(), edit: NodeEdit::SpriteRendererDrawMode(mode) });
            }
        }
    });
}

fn show_thumbnail(ui: &mut egui::Ui, app: &mut App, doc_idx: usize, sprite: &str) {
    if sprite.is_empty() { return; }
    let Some(doc) = app.docs.get_mut(doc_idx) else { return; };
    let ctx = ui.ctx().clone();
    match doc.atlas_mut() {
        Ok(atlas) => {
            if let Some(tex) = atlas.thumbnail(&ctx, sprite) {
                let size = tex.size_vec2();
                let max = 120.0;
                let scale = (max / size.x.max(size.y)).min(1.0);
                ui.image((tex.id(), size * scale));
            } else {
                ui.colored_label(crate::theme::WARN_TEXT, "⚠ sprite not in atlas");
            }
        }
        Err(e) => {
            ui.colored_label(crate::theme::WARN_TEXT, format!("⚠ {e}"));
        }
    }
}

fn empty_node() -> unity_sprite_author::manifest::Node {
    unity_sprite_author::manifest::Node {
        name: String::new(),
        pos: [0.0, 0.0],
        size: None,
        pivot: None,
        scale: [1.0, 1.0],
        rot_deg_ccw: 0.0,
        graphic: None,
        children: Vec::new(),
    }
}

/// Detect "rect-shaped" Polygon: exactly 4 vertices forming an axis-aligned
/// quad with the standard `[0,2,3,3,1,0]` index layout. Used to switch the
/// inspector kind dropdown between "rect" (this case) and "polygon" (general).
pub fn is_rect_shape(vertices: &[[f32; 2]], triangles: Option<&[u16]>) -> bool {
    if vertices.len() != 4 { return false; }
    let want_tris: &[u16] = &[0, 2, 3, 3, 1, 0];
    if triangles.map_or(false, |t| t == want_tris) {
        // Verify axis-aligned: x's are {min, max} pair, y's are {min, max} pair.
        let xs: std::collections::BTreeSet<i32> = vertices.iter().map(|v| (v[0] * 1000.0).round() as i32).collect();
        let ys: std::collections::BTreeSet<i32> = vertices.iter().map(|v| (v[1] * 1000.0).round() as i32).collect();
        return xs.len() == 2 && ys.len() == 2;
    }
    false
}

pub fn parse_color_hex(hex: &str) -> Option<egui::Color32> {
    let h = hex.trim();
    let bytes = match h.len() {
        6 => {
            let r = u8::from_str_radix(&h[0..2], 16).ok()?;
            let g = u8::from_str_radix(&h[2..4], 16).ok()?;
            let b = u8::from_str_radix(&h[4..6], 16).ok()?;
            [r, g, b, 255]
        }
        8 => {
            let r = u8::from_str_radix(&h[0..2], 16).ok()?;
            let g = u8::from_str_radix(&h[2..4], 16).ok()?;
            let b = u8::from_str_radix(&h[4..6], 16).ok()?;
            let a = u8::from_str_radix(&h[6..8], 16).ok()?;
            [r, g, b, a]
        }
        _ => return None,
    };
    Some(egui::Color32::from_rgba_unmultiplied(bytes[0], bytes[1], bytes[2], bytes[3]))
}
