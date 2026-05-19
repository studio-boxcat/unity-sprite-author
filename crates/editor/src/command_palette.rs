//! Cmd+Shift+P-style command palette. Filters the static `Action` registry
//! and dispatches the chosen one through `App::dispatch`. Open via the
//! `Action::OpenPalette` action (bound to Cmd+Shift+P) — drops back to None
//! after a selection / Esc.

use crate::action::{filter_commands, Action};
use crate::app::App;

#[derive(Default)]
pub struct PaletteState {
    pub query: String,
    pub selected_idx: usize,
}

pub fn show(ctx: &egui::Context, app: &mut App) {
    if app.palette.is_none() {
        return;
    }
    let mut close = false;
    let mut dispatch: Option<Action> = None;

    egui::Window::new("Command Palette")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 80.0))
        .default_width(440.0)
        .show(ctx, |ui| {
            let p = app.palette.as_mut().unwrap();
            let edit = ui.add(
                egui::TextEdit::singleline(&mut p.query)
                    .hint_text("Type a command…")
                    .desired_width(420.0),
            );
            edit.request_focus();
            ui.separator();
            let filtered = filter_commands(&p.query);

            // Keep selected_idx in bounds across query changes.
            if filtered.is_empty() {
                p.selected_idx = 0;
            } else if p.selected_idx >= filtered.len() {
                p.selected_idx = filtered.len() - 1;
            }

            // Arrow nav + commit + cancel.
            let (down, up, enter, esc) = ctx.input(|i| {
                (
                    i.key_pressed(egui::Key::ArrowDown),
                    i.key_pressed(egui::Key::ArrowUp),
                    i.key_pressed(egui::Key::Enter),
                    i.key_pressed(egui::Key::Escape),
                )
            });
            if down && !filtered.is_empty() {
                p.selected_idx = (p.selected_idx + 1).min(filtered.len() - 1);
            }
            if up {
                p.selected_idx = p.selected_idx.saturating_sub(1);
            }
            if esc {
                close = true;
            }
            if enter {
                if let Some(cmd) = filtered.get(p.selected_idx) {
                    dispatch = Some((cmd.action_factory)());
                    close = true;
                }
            }

            egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                for (i, cmd) in filtered.iter().enumerate() {
                    let selected = i == p.selected_idx;
                    let display = match cmd.accelerator {
                        Some(a) => format!("{:<32}{}", cmd.label, a),
                        None => cmd.label.to_string(),
                    };
                    let row = ui.selectable_label(selected, egui::RichText::new(display).monospace());
                    if row.clicked() {
                        dispatch = Some((cmd.action_factory)());
                        close = true;
                    }
                }
            });
        });

    if close {
        app.palette = None;
    }
    if let Some(action) = dispatch {
        app.dispatch(action);
    }
}
