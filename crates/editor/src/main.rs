//! Visual editor for `.tps.fab.json` (see `docs/fab.md`). VSCode-style tabs
//! (one per combined-sprite tree), tree + inspector panels, atlas-backed
//! sprite/color pickers, live composed-mesh preview canvas.

mod action;
mod app;
mod atlas;
mod command_palette;
mod doc;
mod inspector;
mod menubar;
mod ops;
mod picker;
mod preferences;
mod preview;
mod selection;
mod serialize;
mod theme;
mod tree_panel;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1440.0, 880.0])
            .with_title("unity-sprite-author editor"),
        // `persistence` lets eframe round-trip window geometry + our
        // `Preferences` blob to disk between launches.
        persist_window: true,
        ..Default::default()
    };
    eframe::run_native(
        "unity-sprite-author-editor",
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
    )
}
