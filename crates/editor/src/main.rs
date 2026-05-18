//! Visual editor for `.tps.fab.json` (see `docs/fab.md`). VSCode-style tabs
//! (one per combined-sprite tree), tree + inspector panels, atlas-backed
//! sprite/color pickers, live composed-mesh preview canvas.

mod app;
mod atlas;
mod doc;
mod inspector;
mod picker;
mod preview;
mod selection;
mod serialize;
mod tree_panel;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1440.0, 880.0])
            .with_title("unity-sprite-author editor"),
        ..Default::default()
    };
    eframe::run_native(
        "unity-sprite-author-editor",
        options,
        Box::new(|_cc| Ok(Box::new(app::App::default()))),
    )
}
