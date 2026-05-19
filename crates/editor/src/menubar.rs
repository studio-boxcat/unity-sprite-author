//! Native macOS menubar via `muda`. Other platforms keep the in-window egui
//! menu (see `app.rs`). The menubar struct's only job is to translate native
//! `MenuEvent`s into `MenuAction` variants — `App::dispatch_menu_action`
//! turns those into the same calls that the egui menu uses, keeping a single
//! source of truth for "what each menu item does".

use crate::preferences::Preferences;

#[derive(Debug, Clone, Copy)]
pub enum MenuAction {
    Open,
    Save,
    SaveAll,
    CloseTab,
    Undo,
    Redo,
    NewSprite,
    NewContainer,
    Duplicate,
    ToggleShowPolygon,
    ToggleShowPivot,
    ToggleShowOutlines,
    ToggleShowAABB,
}

#[cfg(target_os = "macos")]
pub use macos::Menubar;

#[cfg(not(target_os = "macos"))]
pub use stub::Menubar;

// ---------------------------------------------------------------------------
// macOS implementation
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
mod macos {
    use super::{MenuAction, Preferences};
    use muda::accelerator::{Accelerator, Code, Modifiers};
    use muda::{CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu};

    /// Holds the native menu tree (must stay alive for the install to remain
    /// valid) plus the IDs of the dispatchable items, so polling can route
    /// events back to `MenuAction`s. `CheckMenuItem` handles need to live
    /// here too so `sync_to_prefs` can flip their checkmarks.
    pub struct Menubar {
        _menu: Menu,
        items: Vec<(MenuId, MenuAction)>,
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

            // ----- App menu (macOS's first submenu becomes the named app menu) -----
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

            // ----- File -----
            let open = item("Open…", accel(Modifiers::SUPER, Code::KeyO));
            let save = item("Save", accel(Modifiers::SUPER, Code::KeyS));
            let save_all = item("Save All", accel(Modifiers::SUPER | Modifiers::SHIFT, Code::KeyS));
            let close_tab = item("Close Tab", accel(Modifiers::SUPER, Code::KeyW));
            let file_menu = Submenu::new("File", true);
            file_menu
                .append_items(&[
                    &open,
                    &PredefinedMenuItem::separator(),
                    &save,
                    &save_all,
                    &PredefinedMenuItem::separator(),
                    &close_tab,
                ])
                .expect("file menu");
            menu.append(&file_menu).expect("append file");

            // ----- Edit -----
            let undo = item("Undo", accel(Modifiers::SUPER, Code::KeyZ));
            let redo = item("Redo", accel(Modifiers::SUPER | Modifiers::SHIFT, Code::KeyZ));
            let new_sprite = item("New Sprite", accel(Modifiers::SUPER, Code::KeyN));
            let new_container = item(
                "New Container",
                accel(Modifiers::SUPER | Modifiers::SHIFT, Code::KeyN),
            );
            let duplicate = item("Duplicate", accel(Modifiers::SUPER, Code::KeyD));
            let edit_menu = Submenu::new("Edit", true);
            edit_menu
                .append_items(&[
                    &undo,
                    &redo,
                    &PredefinedMenuItem::separator(),
                    &new_sprite,
                    &new_container,
                    &duplicate,
                ])
                .expect("edit menu");
            menu.append(&edit_menu).expect("append edit");

            // ----- View -----
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
                (open.id().clone(), MenuAction::Open),
                (save.id().clone(), MenuAction::Save),
                (save_all.id().clone(), MenuAction::SaveAll),
                (close_tab.id().clone(), MenuAction::CloseTab),
                (undo.id().clone(), MenuAction::Undo),
                (redo.id().clone(), MenuAction::Redo),
                (new_sprite.id().clone(), MenuAction::NewSprite),
                (new_container.id().clone(), MenuAction::NewContainer),
                (duplicate.id().clone(), MenuAction::Duplicate),
                (show_polygon.id().clone(), MenuAction::ToggleShowPolygon),
                (show_pivot.id().clone(), MenuAction::ToggleShowPivot),
                (show_outlines.id().clone(), MenuAction::ToggleShowOutlines),
                (show_aabb.id().clone(), MenuAction::ToggleShowAABB),
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

        /// Drain pending menu events into a list of high-level actions.
        /// Returns one action per click since the last poll.
        pub fn poll(&self) -> Vec<MenuAction> {
            let mut out = Vec::new();
            while let Ok(event) = MenuEvent::receiver().try_recv() {
                if let Some((_, a)) = self.items.iter().find(|(id, _)| id == &event.id) {
                    out.push(*a);
                }
            }
            out
        }

        /// Reflect `prefs` toggles into the menu's check states so the
        /// menu mirrors the current preferences after non-menu mutations
        /// (e.g. shortcut hotkeys without menu involvement).
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
    use super::{MenuAction, Preferences};

    pub struct Menubar;

    impl Menubar {
        pub fn install(_prefs: &Preferences) -> Self { Self }
        pub fn poll(&self) -> Vec<MenuAction> { Vec::new() }
        pub fn sync_to_prefs(&self, _prefs: &Preferences) {}
    }
}
