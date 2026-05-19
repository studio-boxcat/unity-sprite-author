//! Modal sprite + color pickers. Driven by `App.picker`; closed when the user
//! selects something or hits Esc.

use crate::app::App;
use crate::ops::{NewGraphic, NodeEdit, TreeOp};
use crate::doc::NodePath;
use crate::inspector::parse_color_hex;

pub struct Picker {
    pub kind: PickerKind,
    pub target: NodePath,
    pub filter: String,
    pub hex_buf: String,
    /// Currently-displayed color in the color picker (kept in sync with the
    /// hex_buf — egui's color widget mutates RGBA, the buf mirrors it).
    pub live_color: egui::Color32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PickerKind {
    Sprite,
    Color,
}

impl Picker {
    pub fn new(kind: PickerKind, target: NodePath) -> Self {
        Self {
            kind,
            target,
            filter: String::new(),
            hex_buf: String::new(),
            live_color: egui::Color32::WHITE,
        }
    }
}

/// Render `Color32` back to the hex form used in fab.json (`RRGGBB` when
/// alpha is fully opaque, `RRGGBBAA` otherwise — matches the storage
/// convention `manifest::resolve_color` decodes).
pub fn color_to_hex(c: egui::Color32) -> String {
    if c.a() == 255 {
        format!("{:02X}{:02X}{:02X}", c.r(), c.g(), c.b())
    } else {
        format!("{:02X}{:02X}{:02X}{:02X}", c.r(), c.g(), c.b(), c.a())
    }
}

pub fn show_modal(ctx: &egui::Context, app: &mut App) {
    let mut close = false;
    let mut pick_sprite: Option<String> = None;
    let mut pick_color_hex: Option<String> = None;

    let title = match app.picker.as_ref().map(|p| p.kind) {
        Some(PickerKind::Sprite) => "Pick sprite",
        Some(PickerKind::Color) => "Pick color",
        None => return,
    };

    egui::Window::new(title)
        .collapsible(false)
        .resizable(true)
        .default_size([520.0, 480.0])
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            let Some(picker) = &mut app.picker else { return; };
            ui.horizontal(|ui| {
                ui.label("filter");
                ui.add(egui::TextEdit::singleline(&mut picker.filter).hint_text("substring"));
                if ui.button("Cancel").clicked() {
                    close = true;
                }
            });
            ui.separator();
            let doc_idx = picker.target.doc;
            let kind = picker.kind;
            let Some(doc) = app.docs.get_mut(doc_idx) else { return; };
            let ctx_for_tex = ui.ctx().clone();
            let atlas = doc.atlas_mut();
            match atlas {
                Err(e) => {
                    ui.colored_label(egui::Color32::YELLOW, format!("atlas unavailable: {e}"));
                }
                Ok(atlas) => match kind {
                    PickerKind::Sprite => {
                        let filter = picker.filter.to_lowercase();
                        let entries: Vec<(String, (u32, u32))> = atlas
                            .sheet
                            .sprites
                            .iter()
                            .filter(|s| filter.is_empty() || s.name.to_lowercase().contains(&filter))
                            .map(|s| (s.name.clone(), (s.rect.w, s.rect.h)))
                            .collect();
                        ui.label(format!("{} match(es)", entries.len()));
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                for (name, size) in entries {
                                    if let Some(hex) = name.strip_prefix("Color_") {
                                        // Color_* entries are atlas swatches —
                                        // the actual color matters, not the
                                        // tiny thumbnail. Render as a
                                        // hex-driven swatch + rect-size label.
                                        let color = crate::inspector::parse_color_hex(hex);
                                        show_color_swatch_tile(ui, &name, hex, color, size, &mut pick_sprite);
                                    } else {
                                        // Polygon-clipped variant gives the
                                        // user a faithful sprite preview
                                        // instead of the bounding-box crop.
                                        let tex = atlas.thumbnail_clipped(&ctx_for_tex, &name);
                                        show_sprite_tile(ui, &name, tex.as_ref(), size, &mut pick_sprite);
                                    }
                                }
                            });
                        });
                    }
                    PickerKind::Color => {
                        // Seed the picker's working color from the current
                        // hex buffer (typed/edited) or from the leaf's color
                        // on first open.
                        let seed_color = parse_color_hex(&picker.hex_buf)
                            .unwrap_or(picker.live_color);
                        picker.live_color = seed_color;

                        ui.horizontal(|ui| {
                            // egui's color edit button opens the full color
                            // picker (RGB + HSV sliders, alpha, hex).
                            let mut rgba = [
                                seed_color.r() as f32 / 255.0,
                                seed_color.g() as f32 / 255.0,
                                seed_color.b() as f32 / 255.0,
                                seed_color.a() as f32 / 255.0,
                            ];
                            if ui.color_edit_button_rgba_unmultiplied(&mut rgba).changed() {
                                picker.live_color = egui::Color32::from_rgba_unmultiplied(
                                    (rgba[0] * 255.0).round() as u8,
                                    (rgba[1] * 255.0).round() as u8,
                                    (rgba[2] * 255.0).round() as u8,
                                    (rgba[3] * 255.0).round() as u8,
                                );
                                picker.hex_buf = color_to_hex(picker.live_color);
                            }
                            ui.label("hex");
                            // Editable hex; live-syncs to the swatch above.
                            let resp = ui.add(
                                egui::TextEdit::singleline(&mut picker.hex_buf)
                                    .desired_width(80.0)
                                    .hint_text("RRGGBB / RRGGBBAA"),
                            );
                            if resp.changed() {
                                if let Some(c) = parse_color_hex(&picker.hex_buf) {
                                    picker.live_color = c;
                                }
                            }
                            if ui.button("Apply").clicked() && !picker.hex_buf.is_empty() {
                                pick_color_hex = Some(picker.hex_buf.clone());
                            }
                        });
                        ui.separator();
                        ui.label("existing colors in this atlas:");
                        let filter = picker.filter.to_lowercase();
                        let colors: Vec<String> = atlas
                            .color_names()
                            .filter(|n| filter.is_empty() || n.to_lowercase().contains(&filter))
                            .map(|s| s.to_string())
                            .collect();
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                for name in colors {
                                    let hex = name.strip_prefix("Color_").unwrap_or(&name).to_string();
                                    let color = parse_color_hex(&hex);
                                    show_color_tile(ui, &hex, color, &mut pick_color_hex);
                                }
                            });
                        });
                    }
                },
            }
        });

    if let Some(name) = pick_sprite {
        if let Some(p) = app.picker.take() {
            // Color_* picked from the sprite picker → leaf doesn't want to
            // hold an atlas-sprite reference. Convert the leaf to a rect
            // with that color so the user gets the right graphic kind.
            let is_color = name.starts_with("Color_");
            let current_graphic = app
                .docs
                .get(p.target.doc)
                .and_then(|d| p.target.resolve(&d.manifest))
                .and_then(|n| n.graphic.clone());
            match (is_color, &current_graphic, p.kind) {
                (true, Some(unity_sprite_author::manifest::Graphic::Sprite { .. }), PickerKind::Sprite)
                | (true, Some(unity_sprite_author::manifest::Graphic::SpriteRenderer { .. }), PickerKind::Sprite) => {
                    app.pending_ops.push(TreeOp::SetGraphic {
                        path: p.target.clone(),
                        graphic: Some(NewGraphic::Rect),
                    });
                    app.pending_ops.push(TreeOp::Edit {
                        path: p.target,
                        edit: NodeEdit::PolygonColor(name),
                    });
                }
                (_, Some(unity_sprite_author::manifest::Graphic::Sprite { .. }), PickerKind::Sprite) => {
                    app.pending_ops.push(TreeOp::Edit {
                        path: p.target,
                        edit: NodeEdit::SpriteRef(name),
                    });
                }
                (_, Some(unity_sprite_author::manifest::Graphic::SpriteRenderer { .. }), PickerKind::Sprite) => {
                    app.pending_ops.push(TreeOp::Edit {
                        path: p.target,
                        edit: NodeEdit::SpriteRendererSprite(name),
                    });
                }
                _ => {}
            }
        }
    } else if let Some(hex) = pick_color_hex {
        if let Some(p) = app.picker.take() {
            let upper = hex.to_ascii_uppercase();
            app.pending_ops.push(TreeOp::Edit {
                path: p.target,
                edit: NodeEdit::PolygonColor(format!("Color_{upper}")),
            });
        }
    } else if close {
        app.picker = None;
    }

    // Close on Esc.
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.picker = None;
    }
}

fn show_sprite_tile(
    ui: &mut egui::Ui,
    name: &str,
    tex: Option<&egui::TextureHandle>,
    size: (u32, u32),
    pick: &mut Option<String>,
) {
    let resp = ui.allocate_response(egui::vec2(96.0, 96.0), egui::Sense::click());
    let rect = resp.rect;
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, egui::Color32::from_gray(40));
    if let Some(tex) = tex {
        let img_size = tex.size_vec2();
        let max = 70.0;
        let s = (max / img_size.x.max(img_size.y)).min(1.0);
        let draw_size = img_size * s;
        let center = rect.center();
        let img_rect = egui::Rect::from_center_size(center - egui::vec2(0.0, 12.0), draw_size);
        painter.image(tex.id(), img_rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
    }
    painter.text(
        egui::pos2(rect.center().x, rect.bottom() - 18.0),
        egui::Align2::CENTER_CENTER,
        truncate(name, 14),
        egui::FontId::proportional(10.0),
        egui::Color32::LIGHT_GRAY,
    );
    painter.text(
        egui::pos2(rect.center().x, rect.bottom() - 6.0),
        egui::Align2::CENTER_CENTER,
        format!("{}×{}", size.0, size.1),
        egui::FontId::monospace(9.0),
        egui::Color32::from_gray(150),
    );
    if resp.hovered() {
        painter.rect_stroke(rect, 2.0, egui::Stroke::new(2.0, egui::Color32::WHITE));
    }
    resp.on_hover_text(format!("{name}  ({}×{})", size.0, size.1));
    if ui.input(|i| i.pointer.primary_pressed()) && rect.contains(ui.input(|i| i.pointer.hover_pos().unwrap_or_default())) {
        *pick = Some(name.to_string());
    }
}

fn show_color_swatch_tile(
    ui: &mut egui::Ui,
    name: &str,
    hex: &str,
    color: Option<egui::Color32>,
    size: (u32, u32),
    pick: &mut Option<String>,
) {
    let resp = ui.allocate_response(egui::vec2(96.0, 96.0), egui::Sense::click());
    let rect = resp.rect;
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, egui::Color32::from_gray(30));
    // Swatch area takes the top ~60% of the tile.
    let swatch = egui::Rect::from_min_max(
        rect.left_top() + egui::vec2(8.0, 8.0),
        rect.right_top() + egui::vec2(-8.0, 56.0),
    );
    painter.rect_filled(swatch, 2.0, color.unwrap_or(egui::Color32::DARK_GRAY));
    painter.rect_stroke(swatch, 2.0, egui::Stroke::new(0.5, egui::Color32::from_gray(80)));
    painter.text(
        egui::pos2(rect.center().x, rect.bottom() - 18.0),
        egui::Align2::CENTER_CENTER,
        hex,
        egui::FontId::monospace(10.0),
        egui::Color32::LIGHT_GRAY,
    );
    painter.text(
        egui::pos2(rect.center().x, rect.bottom() - 6.0),
        egui::Align2::CENTER_CENTER,
        format!("{}×{}", size.0, size.1),
        egui::FontId::monospace(9.0),
        egui::Color32::from_gray(150),
    );
    if resp.hovered() {
        painter.rect_stroke(rect, 2.0, egui::Stroke::new(2.0, egui::Color32::WHITE));
    }
    resp.on_hover_text(format!("{name}  ({}×{})", size.0, size.1));
    if ui.input(|i| i.pointer.primary_pressed()) && rect.contains(ui.input(|i| i.pointer.hover_pos().unwrap_or_default())) {
        *pick = Some(name.to_string());
    }
}

fn show_color_tile(ui: &mut egui::Ui, hex: &str, color: Option<egui::Color32>, pick: &mut Option<String>) {
    let resp = ui.allocate_response(egui::vec2(72.0, 72.0), egui::Sense::click());
    let rect = resp.rect;
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, color.unwrap_or(egui::Color32::DARK_GRAY));
    painter.text(
        egui::pos2(rect.center().x, rect.bottom() - 8.0),
        egui::Align2::CENTER_CENTER,
        hex,
        egui::FontId::monospace(10.0),
        egui::Color32::BLACK,
    );
    if resp.hovered() {
        painter.rect_stroke(rect, 2.0, egui::Stroke::new(2.0, egui::Color32::WHITE));
    }
    resp.on_hover_text(hex);
    if ui.input(|i| i.pointer.primary_pressed()) && rect.contains(ui.input(|i| i.pointer.hover_pos().unwrap_or_default())) {
        *pick = Some(hex.to_string());
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max - 1).collect();
        out.push('…');
        out
    }
}
