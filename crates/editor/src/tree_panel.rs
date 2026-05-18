//! Left panel: tree of the active tab's nodes. Right-click context ops +
//! mouse drag-to-reorder. Selection follows the cursor on mouse-down for
//! responsiveness (matches the canvas convention).

use crate::app::{App, NewGraphic, TreeDropTarget, TreeOp};
use crate::doc::NodePath;
use unity_sprite_author::manifest::Node;

pub fn show(ui: &mut egui::Ui, app: &mut App) {
    let Some(tab) = app.active_tab() else {
        ui.label("(no tab — open a .tps.fab.json to start)");
        return;
    };
    let (tree_name, n_children, children) = {
        let Some(doc) = app.docs.get(tab.doc) else { return; };
        let Some(tree) = doc.manifest.trees.get(tab.tree) else { return; };
        (tree.name.clone(), tree.root.children.len(), tree.root.children.clone())
    };
    let root_path = NodePath::tree_root(tab.doc, tab.tree);

    // Reset drop target each frame; it'll get repopulated by hover during the recurse.
    app.tree_drop_target = None;

    show_root(ui, app, tree_name, &root_path, n_children);
    for (i, child) in children.iter().enumerate() {
        show_node(ui, app, child, root_path.child(i));
    }

    // Render drop indicator + commit drop on mouse-up.
    let ctx = ui.ctx().clone();
    let released = ctx.input(|i| i.pointer.any_released());
    if let Some(target) = app.tree_drop_target.clone() {
        let line_y = target.line_y;
        let painter = ui.painter();
        let panel_rect = ui.max_rect();
        painter.line_segment(
            [egui::pos2(panel_rect.left(), line_y), egui::pos2(panel_rect.right(), line_y)],
            egui::Stroke::new(2.0, egui::Color32::from_rgb(0, 180, 255)),
        );
    }
    if released {
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

fn show_root(ui: &mut egui::Ui, app: &mut App, name: String, path: &NodePath, n_children: usize) {
    let selected = app.selection.is_selected(path);
    let label = format!("◆ {name}  ({n_children})");
    let resp = ui.selectable_label(selected, label);
    if resp.clicked() || resp.is_pointer_button_down_on() {
        apply_click_modifier(app, path.clone(), &resp.ctx);
    }
    resp.context_menu(|ui| add_child_menu(ui, app, path));
    // The root accepts drops as "append to end" when the cursor hovers it.
    consider_drop_on_row(app, resp.rect, path, &[], &resp);
}

fn show_node(ui: &mut egui::Ui, app: &mut App, node: &Node, path: NodePath) {
    let label = crate::app::node_label(node);
    let selected = app.selection.is_selected(&path);

    let row_resp = if node.children.is_empty() {
        leaf_row(ui, &label, selected)
    } else {
        let id = ui.make_persistent_id(("node", &path));
        let mut header_resp_opt: Option<egui::Response> = None;
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true)
            .show_header(ui, |ui| {
                header_resp_opt = Some(ui.selectable_label(selected, &label));
            })
            .body(|ui| {
                for (i, child) in node.children.iter().enumerate() {
                    show_node(ui, app, child, path.child(i));
                }
            });
        header_resp_opt.expect("header response set")
    };

    // Mouse-down selection (responsive). Cmd-click toggles, Shift-click extends.
    if row_resp.is_pointer_button_down_on() {
        apply_click_modifier(app, path.clone(), &row_resp.ctx);
    }
    row_resp.context_menu(|ui| node_context_menu(ui, app, &path));

    // Drag source.
    if row_resp.drag_started() {
        app.tree_drag = Some(path.clone());
    }
    // Drop target detection — populates app.tree_drop_target when this row
    // is hovered during an active drag.
    consider_drop_on_row(app, row_resp.rect, &path, &node.children, &row_resp);
}

fn leaf_row(ui: &mut egui::Ui, label: &str, selected: bool) -> egui::Response {
    let resp = ui.selectable_label(selected, label);
    // Promote to click_and_drag so drag_started fires from a row click.
    resp.interact(egui::Sense::click_and_drag())
}

fn consider_drop_on_row(
    app: &mut App,
    rect: egui::Rect,
    path: &NodePath,
    own_children: &[Node],
    resp: &egui::Response,
) {
    if app.tree_drag.is_none() { return; }
    let cursor_pos = match resp.ctx.input(|i| i.pointer.hover_pos()) {
        Some(p) => p,
        None => return,
    };
    // Use the row's actual rect — no expansion. The gap between rows is
    // intentionally a dead zone; expanding both rows into it makes both fire
    // and the lower row wins, which is what drove the visual offset.
    if !rect.contains(cursor_pos) { return; }

    // Suppress drop-onto-self.
    let drag = app.tree_drag.clone().unwrap();
    if &drag == path { return; }
    if path.child_chain.starts_with(&drag.child_chain)
        && path.tree == drag.tree
        && path.doc == drag.doc
        && path.child_chain.len() >= drag.child_chain.len()
    {
        return;
    }

    // The indicator line lives in the gap between rows. Center it on the row
    // edge plus half the panel's item_spacing so it visually sits between
    // this row and the next/previous row.
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
        // Middle of a row with children → drop into it as last child.
        (path.clone(), own_children.len(), rect.bottom() + half_gap)
    } else {
        // Leaf row + middle: fall back to "after this sibling".
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

/// Single-click select / Cmd-click toggle / Shift-click extend. Shared by
/// canvas and tree-row click handling so the modifier semantics stay uniform.
pub fn apply_click_modifier(app: &mut App, path: NodePath, ctx: &egui::Context) {
    let (cmd, shift) = ctx.input(|i| (i.modifiers.command, i.modifiers.shift));
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
