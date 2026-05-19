//! Left panel: tree of the active doc's combined sprites. Section headers
//! per combined tree, alternating row backgrounds, inline thumbnails
//! (sprite leaves use the atlas crop; polygons use a color swatch;
//! containers stack up to 3 descendant previews). Right-click context
//! menu + mouse drag-to-reorder.

use crate::app::{App, TreeDropTarget};
use crate::ops::{NewGraphic, TreeOp};
use crate::doc::NodePath;
use std::collections::HashMap;
use unity_sprite_author::manifest::{Graphic, Node};

const THUMB_PX: f32 = 18.0;

/// Mutable state threaded through the recursive draw — keeps the alternating
/// row index in lockstep with what the user sees + carries the prefetched
/// sprite thumbnails so we don't re-borrow the atlas inside the recursion.
struct TreeCtx {
    row: usize,
    thumbs: HashMap<String, egui::TextureHandle>,
}

pub fn show(ui: &mut egui::Ui, app: &mut App) {
    let Some(tab) = app.active_tab() else {
        ui.label("(no tab — open a .tps.fab.json to start)");
        return;
    };

    // Snapshot doc state without holding a borrow across recursive draws.
    let trees: Vec<(String, usize, Vec<Node>)> = {
        let Some(doc) = app.docs.get(tab.doc) else { return; };
        doc.manifest.trees.iter().enumerate()
            .map(|(i, t)| (t.name.clone(), i, t.root.children.clone()))
            .collect()
    };

    // Prefetch thumbnails. Hitting the atlas requires &mut borrow which
    // clashes with the immutable `app` we pass into show_node; lifting the
    // map here keeps the recursive section pure.
    let thumbs = prefetch_thumbnails(ui.ctx(), app, &trees, tab.doc);

    app.tree_drop_target = None;
    let mut tcx = TreeCtx { row: 0, thumbs };

    for (i, (tree_name, tree_idx, children)) in trees.iter().enumerate() {
        if i > 0 { ui.separator(); }
        let root_path = NodePath::tree_root(tab.doc, *tree_idx);
        show_section_header(ui, app, tree_name, &root_path, children.len(), &mut tcx);
        for (ci, child) in children.iter().enumerate() {
            show_node(ui, app, child, root_path.child(ci), 1, &mut tcx);
        }
    }

    // Drop indicator + commit on release.
    let ctx = ui.ctx().clone();
    if let Some(target) = app.tree_drop_target.clone() {
        let panel_rect = ui.max_rect();
        ui.painter().line_segment(
            [egui::pos2(panel_rect.left(), target.line_y), egui::pos2(panel_rect.right(), target.line_y)],
            egui::Stroke::new(2.0, crate::theme::DROP_INDICATOR),
        );
    }
    if ctx.input(|i| i.pointer.any_released()) {
        if let (Some(src), Some(target)) = (app.tree_drag.take(), app.tree_drop_target.take()) {
            app.pending_ops.push(TreeOp::MoveTo {
                src,
                dst_parent: target.dst_parent,
                dst_idx: target.dst_idx,
            });
        }
        app.tree_drag = None;
    }
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.tree_drag = None;
    }
}

fn prefetch_thumbnails(
    ctx: &egui::Context,
    app: &mut App,
    trees: &[(String, usize, Vec<Node>)],
    doc_idx: usize,
) -> HashMap<String, egui::TextureHandle> {
    // Walk every leaf and collect sprite references first; then hit the atlas
    // exactly once per unique sprite.
    let mut names: Vec<String> = Vec::new();
    for (_, _, children) in trees {
        for c in children {
            collect_sprite_names(c, &mut names);
        }
    }
    names.sort();
    names.dedup();
    let mut out = HashMap::new();
    if let Some(doc) = app.docs.get_mut(doc_idx) {
        if let Ok(atlas) = doc.atlas_mut() {
            for n in names {
                if let Some(t) = atlas.thumbnail(ctx, &n) {
                    out.insert(n, t);
                }
            }
        }
    }
    out
}

fn collect_sprite_names(node: &Node, out: &mut Vec<String>) {
    match &node.graphic {
        Some(Graphic::Sprite { sprite, .. }) | Some(Graphic::SpriteRenderer { sprite, .. }) => {
            if !sprite.is_empty() { out.push(sprite.clone()); }
        }
        _ => {}
    }
    for c in &node.children { collect_sprite_names(c, out); }
}

fn show_section_header(
    ui: &mut egui::Ui,
    app: &mut App,
    name: &str,
    path: &NodePath,
    n_children: usize,
    tcx: &mut TreeCtx,
) {
    // Pre-paint the row background (zebra-stripe parity even for headers
    // keeps the visual rhythm consistent).
    let bg_rect = paint_row_background(ui, tcx.row);
    tcx.row += 1;

    let selected = app.selection.is_selected(path);
    ui.horizontal(|ui| {
        ui.add_space(2.0);
        let label = egui::RichText::new(format!("◆ {name}")).strong();
        let resp = ui.selectable_label(selected, label)
            .on_hover_text(format!("{n_children} child node(s)"));
        if resp.clicked() || resp.is_pointer_button_down_on() {
            apply_click_modifier(app, path.clone(), &resp.ctx);
        }
        resp.context_menu(|ui| add_child_menu(ui, app, path));
        consider_drop_on_row(app, bg_rect, path, &[], &resp);
    });
}

fn show_node(
    ui: &mut egui::Ui,
    app: &mut App,
    node: &Node,
    path: NodePath,
    depth: usize,
    tcx: &mut TreeCtx,
) {
    let row_idx = tcx.row;
    tcx.row += 1;
    let bg_rect = paint_row_background(ui, row_idx);
    let selected = app.selection.is_selected(&path);

    let row_resp = if node.children.is_empty() {
        leaf_row(ui, app, node, &label_for(node), selected, depth, tcx)
    } else {
        let id = collapsing_id(&path);
        let mut header_resp_opt: Option<egui::Response> = None;
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true)
            .show_header(ui, |ui| {
                ui.add_space((depth as f32) * 12.0);
                header_resp_opt = Some(inline_row(ui, app, node, &label_for(node), selected, tcx));
            })
            .body(|ui| {
                for (i, child) in node.children.iter().enumerate() {
                    show_node(ui, app, child, path.child(i), depth + 1, tcx);
                }
            });
        header_resp_opt.expect("header response set")
    };

    if row_resp.is_pointer_button_down_on() {
        apply_click_modifier(app, path.clone(), &row_resp.ctx);
    }
    row_resp.context_menu(|ui| node_context_menu(ui, app, &path));
    if row_resp.drag_started() {
        app.tree_drag = Some(path.clone());
    }
    consider_drop_on_row(app, bg_rect, &path, &node.children, &row_resp);
}

fn leaf_row(
    ui: &mut egui::Ui,
    app: &mut App,
    node: &Node,
    label: &str,
    selected: bool,
    depth: usize,
    tcx: &TreeCtx,
) -> egui::Response {
    let mut row_resp = None;
    ui.horizontal(|ui| {
        ui.add_space((depth as f32) * 12.0);
        let r = inline_row(ui, app, node, label, selected, tcx);
        row_resp = Some(r);
    });
    row_resp.unwrap()
}

/// The actual label + thumbnail block. Caller is responsible for indent
/// padding (we don't know whether we're inside a collapsing header's row
/// or a manual leaf row).
fn inline_row(
    ui: &mut egui::Ui,
    _app: &App,
    node: &Node,
    label: &str,
    selected: bool,
    tcx: &TreeCtx,
) -> egui::Response {
    paint_leaf_icon(ui, node, tcx);
    let r = ui.selectable_label(selected, label);
    r.interact(egui::Sense::click_and_drag())
}

fn paint_leaf_icon(ui: &mut egui::Ui, node: &Node, tcx: &TreeCtx) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(THUMB_PX, THUMB_PX), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    let full_uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    match &node.graphic {
        None => paint_container_stack(node, tcx, &painter, rect, full_uv),
        Some(Graphic::Sprite { sprite, .. }) | Some(Graphic::SpriteRenderer { sprite, .. }) => {
            painter.rect_filled(rect, 1.0, egui::Color32::from_gray(40));
            if let Some(tex) = tcx.thumbs.get(sprite) {
                let img = tex.size_vec2();
                let s = (THUMB_PX / img.x.max(img.y)).min(1.0);
                let draw_size = img * s;
                let img_rect = egui::Rect::from_center_size(rect.center(), draw_size);
                painter.image(tex.id(), img_rect, full_uv, egui::Color32::WHITE);
            } else if !sprite.is_empty() {
                painter.text(rect.center(), egui::Align2::CENTER_CENTER, "?", egui::FontId::monospace(11.0), crate::theme::WARN_TEXT);
            }
        }
        Some(Graphic::Polygon { polygon_sprite, .. }) => {
            let hex = polygon_sprite.strip_prefix("Color_").unwrap_or(polygon_sprite);
            let color = crate::inspector::parse_color_hex(hex).unwrap_or(egui::Color32::DARK_GRAY);
            painter.rect_filled(rect.shrink(2.0), 1.0, color);
            painter.rect_stroke(rect.shrink(2.0), 1.0, egui::Stroke::new(0.5, egui::Color32::BLACK));
        }
    }
    ui.add_space(4.0);
}

/// Container icon: stack up to 3 descendant leaf previews diagonally to
/// suggest "group". Falls back to a plain rect stroke when the container has
/// no sprite/polygon descendants yet.
fn paint_container_stack(node: &Node, tcx: &TreeCtx, painter: &egui::Painter, rect: egui::Rect, full_uv: egui::Rect) {
    let mut samples: Vec<DescendantSample> = Vec::with_capacity(3);
    collect_descendant_samples(node, &mut samples, 3);
    if samples.is_empty() {
        painter.rect_stroke(rect.shrink(2.0), 1.0, egui::Stroke::new(0.8, egui::Color32::from_gray(140)));
        return;
    }
    // Draw back-to-front so the most-relevant first sample lands on top.
    for (i, sample) in samples.iter().enumerate().rev() {
        let offset = egui::vec2(i as f32 * 1.5, -(i as f32 * 1.5));
        let tile = egui::Rect::from_center_size(rect.center() + offset, egui::vec2(THUMB_PX * 0.7, THUMB_PX * 0.7));
        match sample {
            DescendantSample::Sprite(name) => {
                if let Some(tex) = tcx.thumbs.get(name) {
                    let img = tex.size_vec2();
                    let s = (tile.width() / img.x.max(img.y)).min(1.0);
                    let img_rect = egui::Rect::from_center_size(tile.center(), img * s);
                    painter.rect_filled(tile, 1.0, egui::Color32::from_gray(50));
                    painter.image(tex.id(), img_rect, full_uv, egui::Color32::WHITE);
                } else {
                    painter.rect_filled(tile, 1.0, egui::Color32::from_gray(50));
                }
            }
            DescendantSample::Color(c) => {
                painter.rect_filled(tile, 1.0, *c);
            }
        }
        painter.rect_stroke(tile, 1.0, egui::Stroke::new(0.4, egui::Color32::from_gray(110)));
    }
}

enum DescendantSample {
    Sprite(String),
    Color(egui::Color32),
}

fn collect_descendant_samples(node: &Node, out: &mut Vec<DescendantSample>, cap: usize) {
    for c in &node.children {
        if out.len() >= cap { return; }
        match &c.graphic {
            Some(Graphic::Sprite { sprite, .. }) | Some(Graphic::SpriteRenderer { sprite, .. }) => {
                if !sprite.is_empty() { out.push(DescendantSample::Sprite(sprite.clone())); }
            }
            Some(Graphic::Polygon { polygon_sprite, .. }) => {
                let hex = polygon_sprite.strip_prefix("Color_").unwrap_or(polygon_sprite);
                if let Some(c) = crate::inspector::parse_color_hex(hex) {
                    out.push(DescendantSample::Color(c));
                }
            }
            None => {}
        }
        collect_descendant_samples(c, out, cap);
    }
}

/// Paint a faint alt-row background. Returns the row's predicted rect so
/// drop-target hit-testing can use it.
fn paint_row_background(ui: &mut egui::Ui, row_idx: usize) -> egui::Rect {
    let height = ui.spacing().interact_size.y.max(THUMB_PX);
    let cursor = ui.cursor();
    let rect = egui::Rect::from_min_size(
        egui::pos2(cursor.min.x, cursor.min.y),
        egui::vec2(ui.available_width(), height),
    );
    if row_idx % 2 == 1 {
        ui.painter().rect_filled(rect, 0.0, crate::theme::row_alt_bg());
    }
    rect
}

fn label_for(node: &Node) -> String {
    crate::app::node_label(node)
}

/// Stable Id for a node's collapsing header — see `is_node_open`.
pub fn collapsing_id(path: &NodePath) -> egui::Id {
    egui::Id::new(("usa-tree-node", path))
}

pub fn is_node_open(ctx: &egui::Context, path: &NodePath) -> bool {
    egui::collapsing_header::CollapsingState::load(ctx, collapsing_id(path))
        .map(|s| s.is_open())
        .unwrap_or(true)
}

fn consider_drop_on_row(
    app: &mut App,
    rect: egui::Rect,
    path: &NodePath,
    own_children: &[Node],
    resp: &egui::Response,
) {
    if app.tree_drag.is_none() { return; }
    let Some(cursor_pos) = resp.ctx.input(|i| i.pointer.hover_pos()) else { return; };
    if !rect.contains(cursor_pos) { return; }

    let drag = app.tree_drag.clone().unwrap();
    if &drag == path { return; }
    if path.child_chain.starts_with(&drag.child_chain)
        && path.tree == drag.tree
        && path.doc == drag.doc
        && path.child_chain.len() >= drag.child_chain.len()
    {
        return;
    }

    let half_gap = resp.ctx.style().spacing.item_spacing.y * 0.5;
    let y_rel = (cursor_pos.y - rect.top()) / rect.height();
    let (dst_parent, dst_idx, line_y) = if path.child_chain.is_empty() {
        let n = app.docs.get(path.doc)
            .and_then(|d| d.manifest.trees.get(path.tree))
            .map(|t| t.root.children.len())
            .unwrap_or(0);
        (path.clone(), n, rect.bottom() + half_gap)
    } else if y_rel < 0.25 {
        let parent = path.parent().unwrap();
        let idx = *path.child_chain.last().unwrap();
        (parent, idx, rect.top() - half_gap)
    } else if y_rel > 0.75 {
        let parent = path.parent().unwrap();
        let idx = *path.child_chain.last().unwrap() + 1;
        (parent, idx, rect.bottom() + half_gap)
    } else if !own_children.is_empty() {
        (path.clone(), own_children.len(), rect.bottom() + half_gap)
    } else {
        let parent = path.parent().unwrap();
        let idx = *path.child_chain.last().unwrap() + 1;
        (parent, idx, rect.bottom() + half_gap)
    };
    app.tree_drop_target = Some(TreeDropTarget { dst_parent, dst_idx, line_y });
}

fn node_context_menu(ui: &mut egui::Ui, app: &mut App, path: &NodePath) {
    ui.menu_button("Add child", |ui| add_child_buttons(ui, app, path));
    if ui.button("Duplicate").clicked() {
        app.pending_ops.push(TreeOp::Duplicate(path.clone()));
        ui.close_menu();
    }
    if ui.button("Move up").clicked() {
        app.pending_ops.push(TreeOp::MoveSibling { path: path.clone(), delta: -1 });
        ui.close_menu();
    }
    if ui.button("Move down").clicked() {
        app.pending_ops.push(TreeOp::MoveSibling { path: path.clone(), delta: 1 });
        ui.close_menu();
    }
    ui.separator();
    if ui.button("Delete").clicked() {
        app.pending_ops.push(TreeOp::Delete(path.clone()));
        ui.close_menu();
    }
}

fn add_child_menu(ui: &mut egui::Ui, app: &mut App, parent: &NodePath) {
    ui.menu_button("Add child", |ui| add_child_buttons(ui, app, parent));
}

pub fn apply_click_modifier(app: &mut App, path: NodePath, ctx: &egui::Context) {
    let (cmd, shift) = ctx.input(|i| (i.modifiers.command, i.modifiers.shift));
    apply_click_modifier_path(app, path, cmd, shift);
}

pub fn apply_click_modifier_path(app: &mut App, path: NodePath, cmd: bool, shift: bool) {
    if cmd {
        app.selection.toggle(path);
    } else if shift {
        app.selection.extend(path);
    } else {
        app.selection.set_single(path);
    }
}

fn add_child_buttons(ui: &mut egui::Ui, app: &mut App, parent: &NodePath) {
    let kinds = [
        ("Container", NewGraphic::Container),
        ("Sprite", NewGraphic::Sprite),
        ("Rect", NewGraphic::Rect),
        ("Polygon", NewGraphic::Polygon),
        ("SpriteRenderer (SMA)", NewGraphic::SpriteRenderer),
    ];
    for (label, kind) in kinds {
        if ui.button(label).clicked() {
            app.pending_ops.push(TreeOp::AddChild {
                parent: parent.clone(),
                graphic: kind,
            });
            ui.close_menu();
        }
    }
}
