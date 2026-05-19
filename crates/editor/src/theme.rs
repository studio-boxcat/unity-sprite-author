//! Centralized color + visual constants. Opaque colors are `const`; the
//! semi-transparent ones are `fn` because `Color32::from_rgba_unmultiplied`
//! isn't `const` and hand-computing premultiplied values is error-prone.

use egui::Color32;

// ---- Selection / highlight (opaque) ----
pub const SELECTION: Color32 = Color32::from_rgb(255, 200, 0);

// ---- Canvas chrome ----
pub const CANVAS_BG: Color32 = Color32::from_gray(28);
pub const RULER_BG: Color32 = Color32::from_gray(40);
pub const RULER_CORNER_BG: Color32 = Color32::from_gray(48);
pub const RULER_TICK: Color32 = Color32::from_gray(160);
pub const RULER_LABEL: Color32 = Color32::from_gray(200);
pub const WORLD_AXIS: Color32 = Color32::from_gray(70);
pub const ATLAS_AABB: Color32 = Color32::from_gray(100);

// ---- Handles. Size handles reuse `SELECTION` (same yellow); rotation
// handle gets its own green to distinguish "this changes angle, not size". ----
pub const ROTATE_HANDLE: Color32 = Color32::from_rgb(80, 200, 120);
pub const VERTEX_HANDLE: Color32 = Color32::from_rgb(0, 170, 255);
pub const VERTEX_HANDLE_ACTIVE: Color32 = Color32::from_rgb(0, 220, 255);

// ---- Drop indicator + marquee outline ----
pub const DROP_INDICATOR: Color32 = Color32::from_rgb(0, 180, 255);
pub const MARQUEE_STROKE: Color32 = Color32::from_rgb(0, 180, 255);

// ---- Modal tile chrome (sprite picker, color picker, tree thumbnails) ----
pub const TILE_BG: Color32 = Color32::from_gray(40);
pub const TILE_BG_DARK: Color32 = Color32::from_gray(30);
pub const TILE_LABEL_SUBTLE: Color32 = Color32::from_gray(150);
pub const TILE_STROKE: Color32 = Color32::from_gray(80);

// ---- Tree + canvas line accents ----
pub const CONTAINER_GLYPH: Color32 = Color32::from_gray(140);
pub const HANDLE_TETHER: Color32 = Color32::from_gray(120);

// ---- Warnings ----
pub const WARN_TEXT: Color32 = Color32::YELLOW;

// ---- Semi-transparent colors (runtime constructors) ----
pub fn part_outline_unselected() -> Color32 { Color32::from_rgba_unmultiplied(255, 255, 255, 48) }
pub fn pivot_marker_unselected() -> Color32 { Color32::from_rgba_unmultiplied(255, 255, 255, 140) }
pub fn guide_line() -> Color32 { Color32::from_rgba_unmultiplied(0, 200, 255, 200) }
pub fn guide_preview() -> Color32 { Color32::from_rgba_unmultiplied(0, 230, 255, 220) }
pub fn row_alt_bg() -> Color32 { Color32::from_rgba_unmultiplied(255, 255, 255, 6) }
pub fn marquee_fill() -> Color32 { Color32::from_rgba_unmultiplied(0, 180, 255, 30) }
pub fn placeholder_sprite() -> Color32 { Color32::from_rgba_unmultiplied(255, 0, 220, 200) }
/// Faint vertical line per indent level in the tree panel.
pub fn indent_guide() -> Color32 { Color32::from_rgba_unmultiplied(255, 255, 255, 24) }
