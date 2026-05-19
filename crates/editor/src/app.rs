//! eframe App. VSCode-like tabs (one per tree in any loaded `.tps.fab.json`),
//! tree panel + inspector + preview canvas. Holds the per-doc undo / redo
//! stacks, pending-op queue, selection, and per-tab view (pan/zoom) state.

use crate::action::Action;
use crate::command_palette::PaletteState;
use crate::doc::{Doc, LoadError, NodePath, SaveError};
use crate::inspector;
use crate::menubar::Menubar;
use crate::ops::{self, NewGraphic, TreeOp};
use crate::picker::Picker;
use crate::preferences::Preferences;
use crate::selection::Selection;
use crate::tree_panel;
use unity_sprite_author::manifest::{Graphic, Node, Output};

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
    /// Tracks repeat-clicks at the same world position so left/right-click
    /// rotates selection through overlapping parts (Photoshop convention).
    /// Reset when the cursor moves outside `CLICK_ROTATE_TOLERANCE_PX`.
    pub click_rotate: Option<ClickRotateState>,
    /// Photoshop-style horizontal + vertical guidelines, per `(doc, tree)`.
    /// Session-only; not persisted (would require per-file metadata or a
    /// sidecar). Empty on launch.
    pub guides: std::collections::HashMap<ViewKey, GuideSet>,
    /// Active drag of a guide line (new or existing). Owns the axis +
    /// optional index of the guide being mutated.
    pub guide_drag: Option<GuideDrag>,
    /// Source path of an in-flight tree-row drag (set on drag_started by the
    /// tree panel; consumed on mouse-up to emit a `MoveTo` op).
    pub tree_drag: Option<NodePath>,
    /// Live drop target, refreshed each frame while `tree_drag` is set.
    pub tree_drop_target: Option<TreeDropTarget>,
    /// Persistent pan/zoom per tab. Re-fit happens only on first open or via
    /// the "Fit" button — otherwise the canvas would rescale every time a
    /// part was dragged outside the current AABB.
    pub views: std::collections::HashMap<ViewKey, ViewState>,
    /// Persisted user preferences (view toggles + recent-files list). Loaded
    /// in `App::new` via `eframe::Storage` and written back by
    /// `eframe::App::save` — autosaves every 30 s + on shutdown.
    pub prefs: Preferences,
    /// Native menubar handle. On macOS, drives File/Edit/View via `muda`
    /// and suppresses the in-window egui menu (set up only after the
    /// NSApplication exists, i.e. inside `App::new`). On other platforms
    /// this is a no-op stub and the egui menu_bar at the top of the window
    /// remains the source of truth.
    pub menubar: Option<Menubar>,
    /// Command palette state. `Some` while the palette modal is open.
    pub palette: Option<PaletteState>,
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

/// One per `(doc, tree)`. Lines are world coords on the canvas — vertical
/// guides snap an `x` coord; horizontal guides snap a `y` coord.
#[derive(Debug, Clone, Default)]
pub struct GuideSet {
    pub vertical: Vec<f32>,   // x positions
    pub horizontal: Vec<f32>, // y positions
}

#[derive(Debug, Clone, Copy)]
pub enum GuideDrag {
    /// Cursor is dragging from the top ruler (creating a vertical line).
    AddVertical,
    /// Cursor is dragging from the left ruler (creating a horizontal line).
    AddHorizontal,
    /// Re-positioning an existing vertical guide at index `0`.
    MoveVertical(usize),
    MoveHorizontal(usize),
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

/// One tab = one open `.tps.fab.json` file. Trees inside that file appear as
/// top-level rows in the left tree panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TabId {
    pub doc: usize,
}

/// View-state key: per (doc, tree). Each combined sprite within a file has
/// its own pan/zoom, since their mesh AABBs differ.
pub type ViewKey = (usize, usize);

/// Per-click rotation state for "click again at same spot to advance through
/// overlapping parts". The `parts` vector is the z-order-stacked list of part
/// indices under the cursor on the first click; `cursor_index` is which we
/// last selected. Right-click + Cmd-click also advance.
#[derive(Debug, Clone)]
pub struct ClickRotateState {
    /// Screen-space anchor; further clicks within a small radius keep rotating.
    pub anchor_screen: egui::Pos2,
    pub parts: Vec<usize>,
    pub cursor_index: usize,
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


impl App {
    /// eframe creation hook — pulls persisted `Preferences` out of storage
    /// (or defaults if first run / settings file missing). Auto-reopens the
    /// most-recently-used file so the session picks up where the last one
    /// left off; stale entries (file deleted/renamed) are silently skipped.
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let prefs = Preferences::load(cc.storage);
        let menubar = Some(Menubar::install(&prefs));
        let mut app = Self {
            prefs,
            menubar,
            ..Default::default()
        };
        if let Some(path) = app.prefs.recent_files.first().cloned() {
            if path.exists() {
                app.open_path(path);
            }
        }
        app
    }

    /// Central action dispatcher. Every user-facing command goes through
    /// here — menus, keyboard shortcuts, command palette, right-click ops.
    /// Add a new feature: extend `Action`, add one `match` arm, optionally
    /// register a `CommandEntry` in `action::commands()`.
    pub fn dispatch(&mut self, action: Action) {
        match action {
            Action::OpenDialog => self.open_dialog(),
            Action::SaveActive => self.save_active(),
            Action::SaveAll => self.save_all(),
            Action::CloseActiveTab => {
                if let Some(i) = self.active_tab {
                    self.close_tab(i);
                }
            }
            Action::Undo => self.undo(),
            Action::Redo => self.redo(),
            Action::AddUnderSelection(g) => self.add_under_selection(g),
            Action::DuplicateSelection => {
                for sel in self.selection.without_descendants_of_selected() {
                    if !sel.child_chain.is_empty() {
                        self.pending_ops.push(TreeOp::Duplicate(sel));
                    }
                }
            }
            Action::DeleteSelection => {
                for sel in self.selection.without_descendants_of_selected() {
                    if !sel.child_chain.is_empty() {
                        self.pending_ops.push(TreeOp::Delete(sel));
                    }
                }
            }
            Action::Fit => {
                if let Some(tab) = self.active_tab() {
                    let primary_tree = self.selection.primary()
                        .filter(|p| p.doc == tab.doc)
                        .map(|p| p.tree)
                        .unwrap_or(0);
                    if let Some(v) = self.views.get_mut(&(tab.doc, primary_tree)) {
                        v.needs_fit = true;
                    }
                }
            }
            Action::ToggleShowPolygon => self.prefs.show_polygon = !self.prefs.show_polygon,
            Action::ToggleShowPivot => self.prefs.show_pivot_markers = !self.prefs.show_pivot_markers,
            Action::ToggleShowOutlines => self.prefs.show_part_outlines = !self.prefs.show_part_outlines,
            Action::ToggleShowAABB => self.prefs.show_atlas_aabb = !self.prefs.show_atlas_aabb,
            Action::NextTab => self.cycle_tab(1),
            Action::PrevTab => self.cycle_tab(-1),
            Action::OpenPalette => {
                self.palette = Some(PaletteState::default());
            }
        }
    }

    fn open_dialog(&mut self) {
        let mut dialog = rfd::FileDialog::new()
            .add_filter("fab manifest", &["json"])
            .set_title("Open .tps.fab.json");
        if let Some(dir) = &self.prefs.last_open_dir {
            dialog = dialog.set_directory(dir);
        }
        let Some(path) = dialog.pick_file() else { return; };
        self.open_path(path);
    }

    /// Open by explicit path (also used by the recent-files submenu). Updates
    /// the persisted recent-files list on success.
    pub fn open_path(&mut self, path: std::path::PathBuf) {
        // If the file's already open, focus its tab instead of duplicating.
        if let Some(existing) = self.tabs.iter().position(|t| self.docs.get(t.doc).map_or(false, |d| d.path == path)) {
            self.active_tab = Some(existing);
            self.selection.clear();
            return;
        }
        match Doc::open(&path) {
            Ok(doc) => {
                let doc_idx = self.docs.len();
                let n_trees = doc.manifest.trees.len();
                self.status = Some(format!("loaded {n_trees} tree(s) from {}", doc.path.display()));
                self.prefs.note_open(doc.path.clone());
                self.docs.push(doc);
                let tab_idx = self.tabs.len();
                self.tabs.push(TabId { doc: doc_idx });
                self.active_tab = Some(tab_idx);
                self.selection.clear();
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
        // If the closing tab's doc is dirty, ask before dropping changes.
        let tab = self.tabs[tab_idx];
        let dirty = self.docs.get(tab.doc).map_or(false, |d| d.dirty);
        if dirty {
            let doc_path = self.docs[tab.doc].path.display().to_string();
            match rfd::MessageDialog::new()
                .set_level(rfd::MessageLevel::Warning)
                .set_title("Unsaved changes")
                .set_description(&format!("Save changes to {doc_path} before closing?"))
                .set_buttons(rfd::MessageButtons::YesNoCancel)
                .show()
            {
                rfd::MessageDialogResult::Yes => {
                    if let Some(doc) = self.docs.get_mut(tab.doc) {
                        if doc.save().is_err() {
                            self.status = Some(format!("save failed; close cancelled"));
                            return;
                        }
                    }
                }
                rfd::MessageDialogResult::No => { /* discard */ }
                _ => return, // cancel
            }
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
        // Drop any selected paths that pointed into the closed tab's doc.
        let live_docs: std::collections::HashSet<usize> = self.tabs.iter().map(|t| t.doc).collect();
        let kept: Vec<NodePath> = self.selection.iter()
            .filter(|p| live_docs.contains(&p.doc))
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
                Some(t) => NodePath::tree_root(t.doc, 0),
                None => return,
            },
        };
        self.pending_ops.push(TreeOp::AddChild { parent, graphic });
    }

    /// Cycle through open tabs by `delta`. Wired only on non-macOS where the
    /// egui top menu uses Cmd+Shift+[ / ]; macOS gets the same via muda.
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    pub fn cycle_tab(&mut self, delta: i32) {
        if self.tabs.is_empty() { return; }
        let cur = self.active_tab.unwrap_or(0) as i32;
        let n = self.tabs.len() as i32;
        let next = ((cur + delta).rem_euclid(n)) as usize;
        self.active_tab = Some(next);
        self.selection.clear();
    }

    /// Move selection up/down by one visible tree row. Walks every combined
    /// tree in the active doc in order, skipping descendants of collapsed
    /// nodes so the cursor mirrors what the user sees in the panel.
    pub fn move_selection(&mut self, ctx: &egui::Context, delta: i32) {
        let Some(tab) = self.active_tab() else { return; };
        let Some(doc) = self.docs.get(tab.doc) else { return; };
        let mut visible: Vec<NodePath> = Vec::new();
        for (tree_idx, tree) in doc.manifest.trees.iter().enumerate() {
            let root_path = NodePath::tree_root(tab.doc, tree_idx);
            visible.push(root_path.clone());
            collect_visible_open(ctx, &tree.root, &root_path, &mut visible);
        }
        if visible.is_empty() { return; }
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

    #[cfg_attr(target_os = "macos", allow(dead_code))]
    pub fn active_doc(&self) -> Option<&Doc> {
        let t = self.active_tab()?;
        self.docs.get(t.doc)
    }
}

impl eframe::App for App {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.prefs.save_to(storage);
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // ----- Native menubar dispatch (macOS) -----
        // muda fires accelerators natively. Drain its event channel and run
        // the action through the same dispatcher the egui menu uses.
        if let Some(menubar) = self.menubar.take() {
            for action in menubar.poll() {
                self.dispatch(action);
            }
            menubar.sync_to_prefs(&self.prefs);
            self.menubar = Some(menubar);
        }

        // ----- Keyboard shortcuts (in-window egui menu path) -----
        // On macOS, muda owns the accelerator namespace — letting egui also
        // see Cmd+S etc. would double-fire (menu activation + this handler).
        #[cfg(not(target_os = "macos"))]
        {
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
        } // end cfg(not(target_os = "macos")) keyboard shortcuts

        // Cmd+Shift+P — open the command palette. Cross-platform because the
        // native menubar doesn't claim this combo (and the in-window egui
        // menu doesn't define it). Single binding for all platforms.
        let cmd_shift_p = ctx.input(|i| i.modifiers.command && i.modifiers.shift && i.key_pressed(egui::Key::P));
        if cmd_shift_p {
            self.dispatch(Action::OpenPalette);
        }

        // Tree arrow-key nav (only when no text widget has focus). Not a Cmd
        // accelerator, so it stays cross-platform.
        if !ctx.wants_keyboard_input() {
            let up = ctx.input(|i| i.key_pressed(egui::Key::ArrowUp));
            let down = ctx.input(|i| i.key_pressed(egui::Key::ArrowDown));
            let left = ctx.input(|i| i.key_pressed(egui::Key::ArrowLeft));
            let right = ctx.input(|i| i.key_pressed(egui::Key::ArrowRight));
            if up { self.move_selection(ctx, -1); }
            if down { self.move_selection(ctx, 1); }
            if left { self.collapse_or_parent(); }
            if right { self.expand_or_first_child(); }
        }

        // ----- Top menu -----
        // On macOS the native menubar (muda) replaces the in-window menu.
        // Other platforms keep this so File/Edit/View stay reachable.
        #[cfg(not(target_os = "macos"))]
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
                    let recent: Vec<std::path::PathBuf> = self.prefs.recent_files.clone();
                    ui.add_enabled_ui(!recent.is_empty(), |ui| {
                        ui.menu_button("Recent files", |ui| {
                            for p in &recent {
                                if ui.button(p.display().to_string()).clicked() {
                                    ui.close_menu();
                                    self.open_path(p.clone());
                                }
                            }
                            if !recent.is_empty() {
                                ui.separator();
                                if ui.button("Clear recent").clicked() {
                                    ui.close_menu();
                                    self.prefs.recent_files.clear();
                                }
                            }
                        });
                    });
                    ui.separator();
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
                ui.menu_button("View", |ui| {
                    // Toggles are mutated directly on `self.prefs` — eframe
                    // autosaves them periodically; nothing else to do.
                    ui.checkbox(&mut self.prefs.show_polygon, "Show polygons");
                    ui.checkbox(&mut self.prefs.show_pivot_markers, "Show pivot markers");
                    ui.checkbox(&mut self.prefs.show_part_outlines, "Show part outlines");
                    ui.checkbox(&mut self.prefs.show_atlas_aabb, "Show atlas AABB");
                });
                ui.separator();
                ui.label(if let Some(doc) = self.active_doc() {
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

        // ----- Modal: command palette -----
        if self.palette.is_some() {
            crate::command_palette::show(ctx, self);
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
                let Some(doc) = self.docs.get(tab.doc) else { continue; };
                let active = self.active_tab == Some(i);
                let label = format!(
                    "{}{}",
                    doc.path.file_name().and_then(|n| n.to_str()).unwrap_or("?"),
                    if doc.dirty { " *" } else { "" },
                );
                let resp = ui.selectable_label(active, &label)
                    .on_hover_text(doc.path.display().to_string());
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
        let doc_idx = ops::op_doc(op);
        let is_drag_edit = ops::is_drag_edit(op);
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

    pub fn apply_op(&mut self, op: TreeOp) {
        self.record_undo_for_op(&op);
        match op {
            TreeOp::AddChild { parent, graphic } => {
                let Some(doc) = self.docs.get_mut(parent.doc) else { return; };
                let Some(parent_node) = parent.resolve_mut(&mut doc.manifest) else { return; };
                parent_node.children.push(ops::new_node(graphic));
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
                node.graphic = graphic.and_then(ops::default_graphic);
                doc.dirty = true;
            }
            TreeOp::Edit { path, edit } => {
                let Some(doc) = self.docs.get_mut(path.doc) else { return; };
                let Some(node) = path.resolve_mut(&mut doc.manifest) else { return; };
                ops::apply_edit(node, edit);
                doc.dirty = true;
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
            let hex = polygon_sprite.strip_prefix("Color_").unwrap_or(polygon_sprite.as_str());
            if node.name.is_empty() { format!("polygon #{hex}") }
            else { format!("{name} · #{hex}") }
        }
    }
}

/// Visible-row DFS that honors the tree panel's collapsed state. A
/// collapsed parent's subtree is hidden — arrow nav skips it. Leaves
/// (children-less nodes) have no collapsing state and are always visible.
pub fn collect_visible_open(
    ctx: &egui::Context,
    node: &Node,
    path: &NodePath,
    out: &mut Vec<NodePath>,
) {
    for (i, c) in node.children.iter().enumerate() {
        let cp = path.child(i);
        out.push(cp.clone());
        if !c.children.is_empty() && crate::tree_panel::is_node_open(ctx, &cp) {
            collect_visible_open(ctx, c, &cp, out);
        }
    }
}
