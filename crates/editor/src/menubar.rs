//! Native macOS menubar via `muda`. Other platforms keep the in-window egui
//! menu (see `app.rs`). Each menu item carries an `Action` directly; polling
//! drains pending menu events and returns the corresponding `Action`s, which
//! `App::dispatch` consumes — same path as keyboard / palette / egui-menu.

use crate::action::Action;
use crate::ops::NewGraphic;
use crate::preferences::Preferences;

#[cfg(target_os = "macos")]
pub use macos::Menubar;

#[cfg(not(target_os = "macos"))]
pub use stub::Menubar;

// ---------------------------------------------------------------------------
// macOS implementation
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
mod macos {
    use super::{Action, NewGraphic, Preferences};
    use muda::accelerator::{Accelerator, Code, Modifiers};
    use muda::{CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu};

    /// Holds the native menu tree (must stay alive for the install to remain
    /// valid) plus the item ID → action map for dispatch. `CheckMenuItem`
    /// handles live here too so `sync_to_prefs` can flip their checkmarks.
    pub struct Menubar {
        _menu: Menu,
        items: Vec<(MenuId, Action)>,
        show_polygon: CheckMenuItem,
        show_pivot: CheckMenuItem,
        show_outlines: CheckMenuItem,
        show_aabb: CheckMenuItem,
    }

    impl Menubar {
        /// Build the menu tree and install it as the app's main menu. Must be
        /// called on the main thread after winit has initialized NSApp (i.e.
        /// inside `eframe::CreationContext` callback).
        pub fn install(prefs: &Preferences) -> Self {
            let menu = Menu::new();

            let app_menu = Submenu::new("unity-sprite-author editor", true);
            app_menu
                .append_items(&[
                    &PredefinedMenuItem::about(Some("unity-sprite-author editor"), None),
                    &PredefinedMenuItem::separator(),
                    &PredefinedMenuItem::services(None),
                    &PredefinedMenuItem::separator(),
                    &PredefinedMenuItem::hide(None),
                    &PredefinedMenuItem::hide_others(None),
                    &PredefinedMenuItem::show_all(None),
                    &PredefinedMenuItem::separator(),
                    &PredefinedMenuItem::quit(None),
                ])
                .expect("app menu");
            menu.append(&app_menu).expect("append app");

            let open = item("Open…", accel(Modifiers::SUPER, Code::KeyO));
            let save = item("Save", accel(Modifiers::SUPER, Code::KeyS));
            let save_all = item("Save All", accel(Modifiers::SUPER | Modifiers::SHIFT, Code::KeyS));
            let close_tab = item("Close Tab", accel(Modifiers::SUPER, Code::KeyW));
            let file_menu = Submenu::new("File", true);
            file_menu
                .append_items(&[&open, &PredefinedMenuItem::separator(), &save, &save_all, &PredefinedMenuItem::separator(), &close_tab])
                .expect("file menu");
            menu.append(&file_menu).expect("append file");

            let undo = item("Undo", accel(Modifiers::SUPER, Code::KeyZ));
            let redo = item("Redo", accel(Modifiers::SUPER | Modifiers::SHIFT, Code::KeyZ));
            let new_sprite = item("New Sprite", accel(Modifiers::SUPER, Code::KeyN));
            let new_container = item("New Container", accel(Modifiers::SUPER | Modifiers::SHIFT, Code::KeyN));
            let duplicate = item("Duplicate", accel(Modifiers::SUPER, Code::KeyD));
            let edit_menu = Submenu::new("Edit", true);
            edit_menu
                .append_items(&[&undo, &redo, &PredefinedMenuItem::separator(), &new_sprite, &new_container, &duplicate])
                .expect("edit menu");
            menu.append(&edit_menu).expect("append edit");

            let show_polygon = check("Show Polygons", prefs.show_polygon);
            let show_pivot = check("Show Pivot Markers", prefs.show_pivot_markers);
            let show_outlines = check("Show Part Outlines", prefs.show_part_outlines);
            let show_aabb = check("Show Atlas AABB", prefs.show_atlas_aabb);
            let view_menu = Submenu::new("View", true);
            view_menu
                .append_items(&[&show_polygon, &show_pivot, &show_outlines, &show_aabb])
                .expect("view menu");
            menu.append(&view_menu).expect("append view");

            let items = vec![
                (open.id().clone(), Action::OpenDialog),
                (save.id().clone(), Action::SaveActive),
                (save_all.id().clone(), Action::SaveAll),
                (close_tab.id().clone(), Action::CloseActiveTab),
                (undo.id().clone(), Action::Undo),
                (redo.id().clone(), Action::Redo),
                (new_sprite.id().clone(), Action::AddUnderSelection(NewGraphic::Sprite)),
                (new_container.id().clone(), Action::AddUnderSelection(NewGraphic::Container)),
                (duplicate.id().clone(), Action::DuplicateSelection),
                (show_polygon.id().clone(), Action::ToggleShowPolygon),
                (show_pivot.id().clone(), Action::ToggleShowPivot),
                (show_outlines.id().clone(), Action::ToggleShowOutlines),
                (show_aabb.id().clone(), Action::ToggleShowAABB),
            ];

            // Install last — once `init_for_nsapp` runs, item appends start
            // mutating the live system menubar.
            menu.init_for_nsapp();

            Self {
                _menu: menu,
                items,
                show_polygon,
                show_pivot,
                show_outlines,
                show_aabb,
            }
        }

        /// Drain pending menu events into the corresponding actions.
        pub fn poll(&self) -> Vec<Action> {
            let mut out = Vec::new();
            while let Ok(event) = MenuEvent::receiver().try_recv() {
                if let Some((_, a)) = self.items.iter().find(|(id, _)| id == &event.id) {
                    out.push(a.clone());
                }
            }
            out
        }

        /// Reflect `prefs` toggles into the menu's check states so the
        /// menu mirrors the current preferences after non-menu mutations.
        pub fn sync_to_prefs(&self, prefs: &Preferences) {
            self.show_polygon.set_checked(prefs.show_polygon);
            self.show_pivot.set_checked(prefs.show_pivot_markers);
            self.show_outlines.set_checked(prefs.show_part_outlines);
            self.show_aabb.set_checked(prefs.show_atlas_aabb);
        }
    }

    fn item(label: &str, accelerator: Accelerator) -> MenuItem {
        MenuItem::new(label, true, Some(accelerator))
    }

    fn check(label: &str, checked: bool) -> CheckMenuItem {
        CheckMenuItem::new(label, true, checked, None)
    }

    fn accel(modifiers: Modifiers, code: Code) -> Accelerator {
        Accelerator::new(Some(modifiers), code)
    }
}

// ---------------------------------------------------------------------------
// Non-macOS stub (the in-window egui menu_bar covers Windows / Linux).
// ---------------------------------------------------------------------------

#[cfg(not(target_os = "macos"))]
mod stub {
    use super::{Action, Preferences};

    pub struct Menubar;

    impl Menubar {
        pub fn install(_prefs: &Preferences) -> Self { Self }
        pub fn poll(&self) -> Vec<Action> { Vec::new() }
        pub fn sync_to_prefs(&self, _prefs: &Preferences) {}
    }
}
