//! eframe App. VSCode-like tabs (one per tree in any loaded `.tps.fab.json`),
//! tree panel + inspector + preview canvas. Holds the per-doc undo / redo
//! stacks, pending-op queue, selection, and per-tab view (pan/zoom) state.

use crate::doc::{Doc, LoadError, NodePath, SaveError};
use crate::inspector;
use crate::picker::Picker;
use crate::selection::Selection;
use crate::tree_panel;
use unity_sprite_author::manifest::{DrawMode, Graphic, Node, Output, SpriteMethod, Tree};

#[derive(Default)]
pub struct App {
    pub docs: Vec<Doc>,
    /// Open tabs in display order. Each tab is `(doc_idx, tree_idx)`.
    pub tabs: Vec<TabId>,
    pub active_tab: Option<usize>,
    pub selection: Selection,
    pub status: Option<String>,
    pub pending_ops: Vec<TreeOp>,
    pub picker: Option<Picker>,
    /// Index of the polygon vertex currently being dragged (in the canvas).
    /// `None` between drags. Lives at app scope because the canvas runs per-
    /// frame and needs to remember which vertex the press started on.
    pub dragging_polygon_vertex: Option<usize>,
    /// Active 9-way handle in the preview canvas (NW/N/NE/W/E/SW/S/SE).
    pub dragging_size_handle: Option<crate::preview::SizeHandle>,
    /// Origin of an in-flight rect-marquee selection on the preview canvas.
    pub marquee_origin: Option<egui::Pos2>,
    /// Source path of an in-flight tree-row drag (set on drag_started by the
    /// tree panel; consumed on mouse-up to emit a `MoveTo` op).
    pub tree_drag: Option<NodePath>,
    /// Live drop target, refreshed each frame while `tree_drag` is set.
    pub tree_drop_target: Option<TreeDropTarget>,
    /// Persistent pan/zoom per tab. Re-fit happens only on first open or via
    /// the "Fit" button — otherwise the canvas would rescale every time a
    /// part was dragged outside the current AABB.
    pub views: std::collections::HashMap<TabId, ViewState>,
    /// Undo / redo snapshot stacks per doc index. Snapshots are entire
    /// `Manifest` clones; cheap for the manifests we see (≤ a few hundred
    /// nodes per file). Coalescing logic in `record_undo_for_op` keeps drag
    /// streams (Pos / PolygonVertex) collapsed into one snapshot per chain.
    pub undo: std::collections::HashMap<usize, Vec<unity_sprite_author::manifest::Manifest>>,
    pub redo: std::collections::HashMap<usize, Vec<unity_sprite_author::manifest::Manifest>>,
    /// `true` while we're inside a drag chain (continuous Pos / PolygonVertex
    /// edits). Reset by any non-drag op or by a drag_stopped signal from the
    /// preview canvas.
    pub in_drag_chain: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct ViewState {
    /// World coordinate at the screen center.
    pub center_world: [f32; 2],
    /// Screen pixels per world unit.
    pub zoom: f32,
    /// Cleared after the first `fit_to_mesh` runs. The canvas re-fits whenever
    /// this is true (initial open or explicit "Fit").
    pub needs_fit: bool,
}

impl Default for ViewState {
    fn default() -> Self {
        Self { center_world: [0.0, 0.0], zoom: 100.0, needs_fit: true }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TabId {
    pub doc: usize,
    pub tree: usize,
}

/// Edits emitted by the tree panel / inspector / pickers during a frame, then
/// applied at the end of `update` so we don't mutate during iteration.
#[derive(Debug, Clone)]
pub enum TreeOp {
    /// Append a new child under `parent` (empty container by default).
    AddChild { parent: NodePath, graphic: NewGraphic },
    Duplicate(NodePath),
    Delete(NodePath),
    /// Reorder a node within its parent's `children`.
    MoveSibling { path: NodePath, delta: i32 },
    /// Move `src` to land at `dst_parent.children[dst_idx]` (covers both
    /// in-parent reorder and reparenting). Tree-panel drag-and-drop uses this.
    MoveTo { src: NodePath, dst_parent: NodePath, dst_idx: usize },
    /// Replace the entire node's graphic discriminator (preserves transform).
    SetGraphic { path: NodePath, graphic: Option<NewGraphic> },
    /// Mutate the node via a closure. Used by inspector for inline edits.
    Edit { path: NodePath, edit: NodeEdit },
}

/// Where a dragged tree row would land. `dst_idx` is the insertion index
/// into `dst_parent.children` (post-removal of src if same parent).
#[derive(Debug, Clone, PartialEq)]
pub struct TreeDropTarget {
    pub dst_parent: NodePath,
    pub dst_idx: usize,
    /// Screen Y (f32) where the drop indicator should render. Already
    /// centered in the inter-row gap by `consider_drop_on_row`.
    pub line_y: f32,
}

#[derive(Debug, Clone)]
pub enum NewGraphic {
    Container,
    Sprite,
    /// Convenience: a Polygon with 4 verts + explicit quad indices, edited
    /// as a width × height rectangle. Serializes identically to a free
    /// polygon, but the inspector exposes a different UI for it.
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
    PolygonRemoveVertex(usize),
    PolygonTriangles(Option<Vec<u16>>),
    /// Update a rect-shape polygon's 4 vertices from width/height (centered).
    /// Used by the 9-way handles in the preview canvas.
    PolygonRectSize { width: f32, height: f32 },
    SpriteRendererSprite(String),
    SpriteRendererDrawMode(DrawMode),
}

impl App {
    fn open_dialog(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("fab manifest", &["json"])
            .set_title("Open .tps.fab.json")
            .pick_file()
        else {
            return;
        };
        match Doc::open(&path) {
            Ok(doc) => {
                let doc_idx = self.docs.len();
                let n_trees = doc.manifest.trees.len();
                self.status = Some(format!("loaded {n_trees} tree(s) from {}", doc.path.display()));
                self.docs.push(doc);
                let first_new_tab = self.tabs.len();
                for tree_idx in 0..n_trees {
                    self.tabs.push(TabId { doc: doc_idx, tree: tree_idx });
                }
                if n_trees > 0 {
                    self.active_tab = Some(first_new_tab);
                }
            }
            Err(e) => self.set_error(LoadError::to_string(&e)),
        }
    }

    fn save_active(&mut self) {
        let Some(tab_idx) = self.active_tab else { return; };
        let tab = self.tabs[tab_idx];
        let Some(doc) = self.docs.get_mut(tab.doc) else { return; };
        match doc.save() {
            Ok(()) => self.status = Some(format!("saved {}", doc.path.display())),
            Err(SaveError::Io(e)) => self.status = Some(format!("save io: {e}")),
            Err(SaveError::Validate(e)) => self.status = Some(format!("save rejected (would be invalid): {e}")),
        }
    }

    fn save_all(&mut self) {
        let mut saved = 0;
        let mut failed: Vec<String> = Vec::new();
        for doc in self.docs.iter_mut().filter(|d| d.dirty) {
            match doc.save() {
                Ok(()) => saved += 1,
                Err(e) => failed.push(format!("{}: {e}", doc.path.display())),
            }
        }
        if failed.is_empty() {
            self.status = Some(format!("saved {saved} file(s)"));
        } else {
            self.status = Some(format!("saved {saved}; failures: {}", failed.join("; ")));
        }
    }

    fn close_tab(&mut self, tab_idx: usize) {
        if tab_idx >= self.tabs.len() {
            return;
        }
        self.tabs.remove(tab_idx);
        // Adjust active_tab.
        self.active_tab = match self.active_tab {
            Some(active) if active == tab_idx => {
                if tab_idx < self.tabs.len() { Some(tab_idx) }
                else if !self.tabs.is_empty() { Some(self.tabs.len() - 1) }
                else { None }
            }
            Some(active) if active > tab_idx => Some(active - 1),
            other => other,
        };
        // Drop any selected paths that pointed into the closed tab's tree.
        let live_tabs: std::collections::HashSet<(usize, usize)> = self.tabs.iter().map(|t| (t.doc, t.tree)).collect();
        let kept: Vec<NodePath> = self.selection.iter()
            .filter(|p| live_tabs.contains(&(p.doc, p.tree)))
            .cloned()
            .collect();
        self.selection.replace_with(kept);
    }

    pub fn set_error(&mut self, msg: String) {
        self.status = Some(format!("error: {msg}"));
    }

    /// Add a child under the current selection (or the tree root if nothing
    /// is selected). Drives Cmd+N / Cmd+Shift+N.
    pub fn add_under_selection(&mut self, graphic: NewGraphic) {
        let parent = match self.selection.primary().cloned() {
            Some(p) => p,
            None => match self.active_tab() {
                Some(t) => NodePath::tree_root(t.doc, t.tree),
                None => return,
            },
        };
        self.pending_ops.push(TreeOp::AddChild { parent, graphic });
    }

    pub fn cycle_tab(&mut self, delta: i32) {
        if self.tabs.is_empty() { return; }
        let cur = self.active_tab.unwrap_or(0) as i32;
        let n = self.tabs.len() as i32;
        let next = ((cur + delta).rem_euclid(n)) as usize;
        self.active_tab = Some(next);
        self.selection.clear();
    }

    /// Move selection up/down by one visible tree row.
    pub fn move_selection(&mut self, delta: i32) {
        let Some(tab) = self.active_tab() else { return; };
        let Some(doc) = self.docs.get(tab.doc) else { return; };
        let Some(tree) = doc.manifest.trees.get(tab.tree) else { return; };
        let mut visible: Vec<NodePath> = Vec::new();
        let root_path = NodePath::tree_root(tab.doc, tab.tree);
        visible.push(root_path.clone());
        collect_visible(&tree.root, &root_path, &mut visible);
        let cur_idx = self.selection
            .primary()
            .and_then(|sel| visible.iter().position(|p| p == sel))
            .unwrap_or(0);
        let next = (cur_idx as i32 + delta).clamp(0, visible.len() as i32 - 1) as usize;
        self.selection.set_single(visible[next].clone());
    }

    pub fn collapse_or_parent(&mut self) {
        let Some(sel) = self.selection.primary().cloned() else { return; };
        if !sel.child_chain.is_empty() {
            if let Some(p) = sel.parent() {
                self.selection.set_single(p);
            }
        }
    }

    pub fn expand_or_first_child(&mut self) {
        let Some(sel) = self.selection.primary().cloned() else { return; };
        let Some(doc) = self.docs.get(sel.doc) else { return; };
        let resolved = if sel.child_chain.is_empty() {
            doc.manifest.trees.get(sel.tree).map(|t| &t.root)
        } else {
            sel.resolve(&doc.manifest)
        };
        if let Some(node) = resolved {
            if !node.children.is_empty() {
                self.selection.set_single(sel.child(0));
            }
        }
    }

    pub fn active_tab(&self) -> Option<TabId> {
        self.active_tab.and_then(|i| self.tabs.get(i).copied())
    }

    pub fn active_tree(&self) -> Option<(&Doc, usize, &Tree)> {
        let t = self.active_tab()?;
        let doc = self.docs.get(t.doc)?;
        let tree = doc.manifest.trees.get(t.tree)?;
        Some((doc, t.tree, tree))
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // ----- Keyboard shortcuts (handled before menu so menu state is fresh) -----
        let cmd_s = ctx.input(|i| i.modifiers.command && !i.modifiers.shift && i.key_pressed(egui::Key::S));
        let cmd_shift_s = ctx.input(|i| i.modifiers.command && i.modifiers.shift && i.key_pressed(egui::Key::S));
        let cmd_o = ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::O));
        let cmd_z = ctx.input(|i| i.modifiers.command && !i.modifiers.shift && i.key_pressed(egui::Key::Z));
        let cmd_shift_z = ctx.input(|i| i.modifiers.command && i.modifiers.shift && i.key_pressed(egui::Key::Z));
        if cmd_shift_s {
            self.save_all();
        } else if cmd_s {
            self.save_active();
        }
        if cmd_o {
            self.open_dialog();
        }
        if cmd_shift_z {
            self.redo();
        } else if cmd_z {
            self.undo();
        }

        // Cmd+N / Cmd+Shift+N — new sprite leaf / new container under current
        // selection (or under the tree root if nothing selected).
        let cmd_n = ctx.input(|i| i.modifiers.command && !i.modifiers.shift && i.key_pressed(egui::Key::N));
        let cmd_shift_n = ctx.input(|i| i.modifiers.command && i.modifiers.shift && i.key_pressed(egui::Key::N));
        if cmd_shift_n {
            self.add_under_selection(NewGraphic::Container);
        } else if cmd_n {
            self.add_under_selection(NewGraphic::Sprite);
        }

        // Cmd+D — duplicate the current selection.
        let cmd_d = ctx.input(|i| i.modifiers.command && !i.modifiers.shift && i.key_pressed(egui::Key::D));
        if cmd_d {
            for sel in self.selection.without_descendants_of_selected() {
                if !sel.child_chain.is_empty() {
                    self.pending_ops.push(TreeOp::Duplicate(sel));
                }
            }
        }

        // Cmd+Shift+[ / Cmd+Shift+] — prev/next tab (browser convention).
        let cmd_shift_lb = ctx.input(|i| i.modifiers.command && i.modifiers.shift && i.key_pressed(egui::Key::OpenBracket));
        let cmd_shift_rb = ctx.input(|i| i.modifiers.command && i.modifiers.shift && i.key_pressed(egui::Key::CloseBracket));
        if cmd_shift_lb {
            self.cycle_tab(-1);
        } else if cmd_shift_rb {
            self.cycle_tab(1);
        }

        // Tree arrow-key nav (only when no text widget has focus).
        if !ctx.wants_keyboard_input() {
            let up = ctx.input(|i| i.key_pressed(egui::Key::ArrowUp));
            let down = ctx.input(|i| i.key_pressed(egui::Key::ArrowDown));
            let left = ctx.input(|i| i.key_pressed(egui::Key::ArrowLeft));
            let right = ctx.input(|i| i.key_pressed(egui::Key::ArrowRight));
            if up { self.move_selection(-1); }
            if down { self.move_selection(1); }
            if left { self.collapse_or_parent(); }
            if right { self.expand_or_first_child(); }
        }

        // ----- Top menu -----
        egui::TopBottomPanel::top("menu").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("Edit", |ui| {
                    let can_undo = self.active_tab().map_or(false, |t| self.undo.get(&t.doc).map_or(false, |s| !s.is_empty()));
                    let can_redo = self.active_tab().map_or(false, |t| self.redo.get(&t.doc).map_or(false, |s| !s.is_empty()));
                    if ui.add_enabled(can_undo, egui::Button::new("Undo    Cmd+Z")).clicked() {
                        ui.close_menu();
                        self.undo();
                    }
                    if ui.add_enabled(can_redo, egui::Button::new("Redo    Cmd+Shift+Z")).clicked() {
                        ui.close_menu();
                        self.redo();
                    }
                });
                ui.menu_button("File", |ui| {
                    if ui.button("Open…    Cmd+O").clicked() {
                        ui.close_menu();
                        self.open_dialog();
                    }
                    let can_save = self.active_tab().is_some();
                    if ui.add_enabled(can_save, egui::Button::new("Save active    Cmd+S")).clicked() {
                        ui.close_menu();
                        self.save_active();
                    }
                    let any_dirty = self.docs.iter().any(|d| d.dirty);
                    if ui.add_enabled(any_dirty, egui::Button::new("Save all    Cmd+Shift+S")).clicked() {
                        ui.close_menu();
                        self.save_all();
                    }
                    ui.separator();
                    if ui.button("Quit").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
                ui.separator();
                ui.label(if let Some((doc, _, _)) = self.active_tree() {
                    let dirty = if doc.dirty { " *" } else { "" };
                    format!("{}{dirty}", doc.path.display())
                } else {
                    "(no file)".into()
                });
            });
        });

        // ----- Tab strip -----
        egui::TopBottomPanel::top("tabs").show(ctx, |ui| {
            self.show_tab_strip(ui);
        });

        // ----- Status bar -----
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    self.status
                        .as_deref()
                        .unwrap_or("ready — File > Open to load a .tps.fab.json"),
                );
            });
        });

        // ----- Left: tree panel -----
        egui::SidePanel::left("tree").default_width(320.0).show(ctx, |ui| {
            ui.heading("Tree");
            ui.separator();
            egui::ScrollArea::vertical().show(ui, |ui| {
                tree_panel::show(ui, self);
            });
        });

        // ----- Right: inspector -----
        egui::SidePanel::right("inspector").default_width(380.0).show(ctx, |ui| {
            ui.heading("Inspector");
            ui.separator();
            egui::ScrollArea::vertical().show(ui, |ui| {
                inspector::show(ui, self);
            });
        });

        // ----- Center: preview placeholder -----
        egui::CentralPanel::default().show(ctx, |ui| {
            crate::preview::show(ui, self);
        });

        // ----- Modal: sprite/color picker -----
        if self.picker.is_some() {
            crate::picker::show_modal(ctx, self);
        }

        // ----- Apply deferred ops -----
        self.apply_pending();
    }
}

impl App {
    fn show_tab_strip(&mut self, ui: &mut egui::Ui) {
        if self.tabs.is_empty() {
            ui.label("(no tabs open)");
            return;
        }
        let mut close: Option<usize> = None;
        let mut activate: Option<usize> = None;
        ui.horizontal_wrapped(|ui| {
            for (i, tab) in self.tabs.iter().enumerate() {
                let Some(tree) = self.docs.get(tab.doc).and_then(|d| d.manifest.trees.get(tab.tree)) else {
                    continue;
                };
                let dirty = self.docs.get(tab.doc).map_or(false, |d| d.dirty);
                let active = self.active_tab == Some(i);
                let mark = if dirty { " *" } else { "" };
                let label = format!("{}{mark}", tree.name);
                let resp = ui.selectable_label(active, label);
                if resp.clicked() {
                    activate = Some(i);
                }
                if ui.small_button("×").on_hover_text("Close tab").clicked() {
                    close = Some(i);
                }
                ui.separator();
            }
        });
        if let Some(i) = activate {
            self.active_tab = Some(i);
            self.selection.clear();
        }
        if let Some(i) = close {
            self.close_tab(i);
        }
    }

    fn apply_pending(&mut self) {
        let ops = std::mem::take(&mut self.pending_ops);
        for op in ops {
            self.apply_op(op);
        }
    }

    /// Push a snapshot of the doc's current manifest to undo, clear redo.
    /// Idempotent within a drag chain (Pos / PolygonVertex edits) — only the
    /// first op in a chain records. The flag transition happens here, not in
    /// the drag handler — flipping it early would suppress the first record.
    fn record_undo_for_op(&mut self, op: &TreeOp) {
        let (doc_idx, is_drag_edit) = match op {
            TreeOp::Edit { path, edit } => (path.doc, matches!(edit, NodeEdit::Pos(_) | NodeEdit::PolygonVertex { .. })),
            TreeOp::AddChild { parent, .. } => (parent.doc, false),
            TreeOp::Duplicate(p) | TreeOp::Delete(p) | TreeOp::MoveSibling { path: p, .. } | TreeOp::SetGraphic { path: p, .. } => (p.doc, false),
            TreeOp::MoveTo { src, .. } => (src.doc, false),
        };
        if is_drag_edit && self.in_drag_chain {
            return;
        }
        if let Some(doc) = self.docs.get(doc_idx) {
            self.undo.entry(doc_idx).or_default().push(doc.manifest.clone());
            self.redo.entry(doc_idx).or_default().clear();
            let stack = self.undo.get_mut(&doc_idx).unwrap();
            if stack.len() > 200 {
                stack.remove(0);
            }
        }
        // Drag edit: arm the coalesce flag so subsequent Pos / PolygonVertex
        // ops in the same drag stream skip recording. Non-drag edits clear
        // it so the next drag chain starts fresh.
        self.in_drag_chain = is_drag_edit;
    }

    pub fn undo(&mut self) {
        let Some(tab) = self.active_tab() else { return; };
        let doc_idx = tab.doc;
        let Some(stack) = self.undo.get_mut(&doc_idx) else {
            self.status = Some("nothing to undo".into());
            return;
        };
        let Some(prev) = stack.pop() else {
            self.status = Some("nothing to undo".into());
            return;
        };
        if let Some(doc) = self.docs.get_mut(doc_idx) {
            let current = std::mem::replace(&mut doc.manifest, prev);
            self.redo.entry(doc_idx).or_default().push(current);
            doc.dirty = true;
        }
        self.in_drag_chain = false;
        self.selection.clear();
        self.status = Some("undo".into());
    }

    pub fn redo(&mut self) {
        let Some(tab) = self.active_tab() else { return; };
        let doc_idx = tab.doc;
        let Some(stack) = self.redo.get_mut(&doc_idx) else {
            self.status = Some("nothing to redo".into());
            return;
        };
        let Some(next) = stack.pop() else {
            self.status = Some("nothing to redo".into());
            return;
        };
        if let Some(doc) = self.docs.get_mut(doc_idx) {
            let current = std::mem::replace(&mut doc.manifest, next);
            self.undo.entry(doc_idx).or_default().push(current);
            doc.dirty = true;
        }
        self.in_drag_chain = false;
        self.selection.clear();
        self.status = Some("redo".into());
    }

    fn apply_op(&mut self, op: TreeOp) {
        self.record_undo_for_op(&op);
        match op {
            TreeOp::AddChild { parent, graphic } => {
                let Some(doc) = self.docs.get_mut(parent.doc) else { return; };
                let Some(parent_node) = parent.resolve_mut(&mut doc.manifest) else { return; };
                parent_node.children.push(new_node(graphic));
                doc.dirty = true;
            }
            TreeOp::Duplicate(path) => {
                let Some(parent_path) = path.parent() else { return; };
                let Some(doc) = self.docs.get_mut(path.doc) else { return; };
                let Some(parent) = parent_path.resolve_mut(&mut doc.manifest) else { return; };
                let last = *path.child_chain.last().unwrap();
                if let Some(src) = parent.children.get(last).cloned() {
                    parent.children.insert(last + 1, src);
                    doc.dirty = true;
                }
            }
            TreeOp::Delete(path) => {
                let Some(parent_path) = path.parent() else { return; };
                let Some(doc) = self.docs.get_mut(path.doc) else { return; };
                let Some(parent) = parent_path.resolve_mut(&mut doc.manifest) else { return; };
                let last = *path.child_chain.last().unwrap();
                if last < parent.children.len() {
                    parent.children.remove(last);
                    doc.dirty = true;
                    self.selection.clear();
                }
            }
            TreeOp::MoveSibling { path, delta } => {
                let Some(parent_path) = path.parent() else { return; };
                let Some(doc) = self.docs.get_mut(path.doc) else { return; };
                let Some(parent) = parent_path.resolve_mut(&mut doc.manifest) else { return; };
                let last = *path.child_chain.last().unwrap();
                let new_idx = (last as i32 + delta).clamp(0, parent.children.len() as i32 - 1) as usize;
                if new_idx != last && last < parent.children.len() {
                    let node = parent.children.remove(last);
                    parent.children.insert(new_idx, node);
                    doc.dirty = true;
                    // Update selection to follow the moved node.
                    self.selection.set_single(parent_path.child(new_idx));
                }
            }
            TreeOp::MoveTo { src, dst_parent, dst_idx } => {
                if src == dst_parent || dst_parent.child_chain.starts_with(&src.child_chain) && src.tree == dst_parent.tree && src.doc == dst_parent.doc {
                    return; // would orphan into own subtree
                }
                let Some(src_parent_path) = src.parent() else { return; };
                let Some(doc) = self.docs.get_mut(src.doc) else { return; };
                let src_last = *src.child_chain.last().unwrap();
                let Some(src_parent) = src_parent_path.resolve_mut(&mut doc.manifest) else { return; };
                if src_last >= src_parent.children.len() { return; }
                let node = src_parent.children.remove(src_last);
                // After removal, indices in the SAME parent past src_last shift down by 1.
                let same_parent = src_parent_path == dst_parent;
                let adjusted_idx = if same_parent && dst_idx > src_last { dst_idx - 1 } else { dst_idx };
                let Some(dst) = dst_parent.resolve_mut(&mut doc.manifest) else {
                    // Re-insert at original spot to avoid losing the node.
                    let _ = src_parent_path.resolve_mut(&mut doc.manifest).map(|n| n.children.insert(src_last, node));
                    return;
                };
                let final_idx = adjusted_idx.min(dst.children.len());
                dst.children.insert(final_idx, node);
                doc.dirty = true;
                self.selection.set_single(dst_parent.child(final_idx));
            }
            TreeOp::SetGraphic { path, graphic } => {
                let Some(doc) = self.docs.get_mut(path.doc) else { return; };
                let Some(node) = path.resolve_mut(&mut doc.manifest) else { return; };
                node.graphic = graphic.and_then(default_graphic);
                doc.dirty = true;
            }
            TreeOp::Edit { path, edit } => {
                let Some(doc) = self.docs.get_mut(path.doc) else { return; };
                let Some(node) = path.resolve_mut(&mut doc.manifest) else { return; };
                apply_edit(node, edit);
                doc.dirty = true;
            }
        }
    }
}

fn default_graphic(g: NewGraphic) -> Option<Graphic> {
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
            // Default 2×2 (canvas-pixel) rect centered at origin. Quad index
            // layout matches `combine::polygon_mesh_with_tris`'s expected
            // CCW ordering for an axis-aligned rect.
            vertices: vec![[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]],
            triangles: Some(vec![0, 2, 3, 3, 1, 0]),
        }),
        NewGraphic::Polygon => Some(Graphic::Polygon {
            polygon_sprite: "Color_FFFFFF".into(),
            // Start as a free triangle so it's visibly distinct from a rect.
            vertices: vec![[0.0, 1.0], [-1.0, -1.0], [1.0, -1.0]],
            triangles: None,
        }),
        NewGraphic::SpriteRenderer => Some(Graphic::SpriteRenderer {
            sprite: String::new(),
            draw_mode: DrawMode::Simple,
        }),
    }
}

fn new_node(g: NewGraphic) -> Node {
    let graphic = default_graphic(g);
    Node {
        name: String::new(),
        pos: [0.0, 0.0],
        size: None,
        pivot: None,
        scale: [1.0, 1.0],
        rot_deg_ccw: 0.0,
        graphic,
        children: Vec::new(),
    }
}

fn apply_edit(node: &mut Node, edit: NodeEdit) {
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
        NodeEdit::PolygonRemoveVertex(idx) => {
            if let Some(Graphic::Polygon { vertices, .. }) = &mut node.graphic {
                if idx < vertices.len() && vertices.len() > 3 {
                    vertices.remove(idx);
                }
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
        NodeEdit::PolygonTriangles(t) => {
            if let Some(Graphic::Polygon { triangles, .. }) = &mut node.graphic {
                *triangles = t;
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

pub fn mode_label(o: &Output) -> &'static str {
    match o {
        Output::Csa => "ui",
        Output::Sma { used_in_canvas: true, .. } => "sma-canvas",
        Output::Sma { used_in_canvas: false, .. } => "sma-renderer",
    }
}

pub fn node_label(node: &Node) -> String {
    let name = if node.name.is_empty() { "(unnamed)" } else { node.name.as_str() };
    match &node.graphic {
        None => format!("{name} · container"),
        Some(Graphic::Sprite { sprite, .. }) | Some(Graphic::SpriteRenderer { sprite, .. }) => {
            if sprite.is_empty() { format!("{name} · (no sprite)") }
            else if node.name.is_empty() { sprite.clone() }
            else { format!("{name} · {sprite}") }
        }
        Some(Graphic::Polygon { polygon_sprite, .. }) => {
            let hex = polygon_sprite.strip_prefix("Color_").unwrap_or(polygon_sprite);
            if node.name.is_empty() { format!("polygon #{hex}") }
            else { format!("{name} · #{hex}") }
        }
    }
}

/// Flatten the tree into a visible-row list in DFS order. Walks all nodes
/// regardless of collapsed state — egui's CollapsingHeader keeps state in
/// its own memory; we treat all nodes as visible for arrow-key navigation
/// purposes (simpler and matches the common file-tree expectation).
pub fn collect_visible(node: &Node, path: &NodePath, out: &mut Vec<NodePath>) {
    for (i, c) in node.children.iter().enumerate() {
        let cp = path.child(i);
        out.push(cp.clone());
        collect_visible(c, &cp, out);
    }
}
