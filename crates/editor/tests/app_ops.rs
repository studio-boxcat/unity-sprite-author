//! Headless integration tests for the App's pending-op pipeline. Drives the
//! same code path the UI uses (push to `pending_ops`, then `apply_pending`)
//! without needing eframe::App::update / a GPU context. Covers add/duplicate/
//! delete/move-to plus selection round-trip across these mutations.

#[path = "../src/action.rs"] mod action;
#[path = "../src/app.rs"] mod app;
#[path = "../src/atlas.rs"] mod atlas;
#[path = "../src/command_palette.rs"] mod command_palette;
#[path = "../src/doc.rs"] mod doc;
#[path = "../src/inspector.rs"] mod inspector;
#[path = "../src/menubar.rs"] mod menubar;
#[path = "../src/ops.rs"] mod ops;
#[path = "../src/picker.rs"] mod picker;
#[path = "../src/preferences.rs"] mod preferences;
#[path = "../src/preview.rs"] mod preview;
#[path = "../src/selection.rs"] mod selection;
#[path = "../src/serialize.rs"] mod serialize;
#[path = "../src/theme.rs"] mod theme;
#[path = "../src/tree_panel.rs"] mod tree_panel;

use crate::app::App;
use crate::ops::{NewGraphic, NodeEdit, TreeOp};
use crate::doc::{Doc, NodePath};
use std::path::PathBuf;
use unity_sprite_author::manifest::{Graphic, Manifest};

fn golden_silloutte() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../core/tests/golden/fab/silloutte/PremiumCat_Vampire_Popup.tps.fab.json")
}

fn app_with_silloutte() -> App {
    let doc = Doc::open(&golden_silloutte()).expect("open silloutte golden");
    let mut app = App::default();
    app.docs.push(doc);
    app.tabs.push(crate::app::TabId { doc: 0 });
    app.active_tab = Some(0);
    app
}

fn first_leaf_path(m: &Manifest) -> NodePath {
    // First child of tree 0's root, which the silloutte fixture has as a
    // pure container ("Body") — descend into its first child for a real leaf.
    let mut path = NodePath { doc: 0, tree: 0, child_chain: vec![0] };
    let body = path.resolve(m).expect("body");
    if !body.children.is_empty() {
        path = path.child(0);
    }
    path
}

/// Drain pending ops the same way `App::update` does at end of frame.
fn flush(app: &mut App) {
    let ops = std::mem::take(&mut app.pending_ops);
    for op in ops {
        app.apply_op(op);
    }
}

#[test]
fn add_then_delete_round_trips_manifest() {
    let mut app = app_with_silloutte();
    let before = app.docs[0].manifest.clone();
    let root = NodePath::tree_root(0, 0);
    app.pending_ops.push(TreeOp::AddChild { parent: root.clone(), graphic: NewGraphic::Container });
    flush(&mut app);
    let after_add = app.docs[0].manifest.clone();
    assert_ne!(before, after_add, "add should mutate");

    // The new child sits at the end of the tree-root's children list.
    let new_child = root.child(after_add.trees[0].root.children.len() - 1);
    app.pending_ops.push(TreeOp::Delete(new_child));
    flush(&mut app);
    assert_eq!(app.docs[0].manifest, before, "delete should undo the add");
}

#[test]
fn move_to_reorders_within_parent() {
    let mut app = app_with_silloutte();
    // Pick a tree with at least 2 children.
    let (tree_idx, n) = app.docs[0].manifest.trees.iter().enumerate()
        .find_map(|(i, t)| if t.root.children.len() >= 2 { Some((i, t.root.children.len())) } else { None })
        .expect("a tree with >=2 children in silloutte golden");
    assert!(n >= 2);
    let parent = NodePath::tree_root(0, tree_idx);
    let original_first = app.docs[0].manifest.trees[tree_idx].root.children[0].clone();
    app.pending_ops.push(TreeOp::MoveTo {
        src: parent.child(0),
        dst_parent: parent.clone(),
        dst_idx: n, // append to end
    });
    flush(&mut app);
    let after = &app.docs[0].manifest.trees[tree_idx].root;
    assert_eq!(after.children.len(), n, "child count preserved");
    assert_eq!(after.children.last().unwrap(), &original_first, "moved to end");
}

#[test]
fn edit_pos_then_undo_restores_value() {
    let mut app = app_with_silloutte();
    let path = first_leaf_path(&app.docs[0].manifest);
    let original_pos = path.resolve(&app.docs[0].manifest).unwrap().pos;
    let target = [original_pos[0] + 50.0, original_pos[1] - 25.0];
    app.pending_ops.push(TreeOp::Edit { path: path.clone(), edit: NodeEdit::Pos(target) });
    flush(&mut app);
    assert_eq!(path.resolve(&app.docs[0].manifest).unwrap().pos, target);
    app.undo();
    assert_eq!(path.resolve(&app.docs[0].manifest).unwrap().pos, original_pos);
    app.redo();
    assert_eq!(path.resolve(&app.docs[0].manifest).unwrap().pos, target);
}

#[test]
fn selection_survives_no_op_mutations() {
    let mut app = app_with_silloutte();
    let path = first_leaf_path(&app.docs[0].manifest);
    app.selection.set_single(path.clone());
    // Touch an unrelated edit; selection should still resolve.
    app.pending_ops.push(TreeOp::Edit { path: path.clone(), edit: NodeEdit::Rot(15.0) });
    flush(&mut app);
    let primary = app.selection.primary().cloned().expect("still selected");
    assert_eq!(primary, path);
    assert_eq!(primary.resolve(&app.docs[0].manifest).unwrap().rot_deg_ccw, 15.0);
}

#[test]
fn polygon_rect_size_sets_quad_vertices() {
    let mut app = app_with_silloutte();
    // Add a rect leaf so the polygon-rect-size edit has somewhere to land.
    let root = NodePath::tree_root(0, 0);
    app.pending_ops.push(TreeOp::AddChild { parent: root.clone(), graphic: NewGraphic::Rect });
    flush(&mut app);
    let new_child = root.child(app.docs[0].manifest.trees[0].root.children.len() - 1);
    app.pending_ops.push(TreeOp::Edit {
        path: new_child.clone(),
        edit: NodeEdit::PolygonRectSize { width: 8.0, height: 4.0 },
    });
    flush(&mut app);
    let node = new_child.resolve(&app.docs[0].manifest).unwrap();
    match &node.graphic {
        Some(Graphic::Polygon { vertices, .. }) => {
            assert_eq!(vertices.len(), 4);
            // Centered rect: corners at (±4, ±2).
            let mut xs: Vec<f32> = vertices.iter().map(|v| v[0]).collect();
            let mut ys: Vec<f32> = vertices.iter().map(|v| v[1]).collect();
            xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
            ys.sort_by(|a, b| a.partial_cmp(b).unwrap());
            assert_eq!((xs[0], xs[3]), (-4.0, 4.0));
            assert_eq!((ys[0], ys[3]), (-2.0, 2.0));
        }
        _ => panic!("expected polygon graphic"),
    }
}
