//! Centralized color + visual constants. Adding a new UI surface should pull
//! from here instead of inlining `Color32::from_rgb(...)` literals so the
//! whole editor reads as one consistent visual language. Spacing constants
//! also live here when they're shared across modules.

use egui::Color32;

// ---- Selection / highlight ----
pub const SELECTION: Color32 = Color32::from_rgb(255, 200, 0);
pub const SELECTION_DIM: Color32 = Color32::from_rgba_premultiplied(255, 200, 0, 80);
pub const PART_OUTLINE_UNSELECTED: Color32 = Color32::from_rgba_premultiplied(255, 255, 255, 48);

// ---- Canvas chrome ----
pub const CANVAS_BG: Color32 = Color32::from_gray(28);
pub const RULER_BG: Color32 = Color32::from_gray(40);
pub const RULER_CORNER_BG: Color32 = Color32::from_gray(48);
pub const RULER_TICK: Color32 = Color32::from_gray(160);
pub const RULER_LABEL: Color32 = Color32::from_gray(200);
pub const WORLD_AXIS: Color32 = Color32::from_gray(70);
pub const ATLAS_AABB: Color32 = Color32::from_gray(100);

// ---- Handles ----
pub const SIZE_HANDLE: Color32 = Color32::from_rgb(255, 200, 0);
pub const ROTATE_HANDLE: Color32 = Color32::from_rgb(80, 200, 120);
pub const HANDLE_STROKE: Color32 = Color32::BLACK;
pub const VERTEX_HANDLE: Color32 = Color32::from_rgb(0, 170, 255);
pub const VERTEX_HANDLE_ACTIVE: Color32 = Color32::from_rgb(0, 220, 255);

// ---- Guides ----
pub const GUIDE_LINE: Color32 = Color32::from_rgba_premultiplied(0, 200, 255, 200);
pub const GUIDE_PREVIEW: Color32 = Color32::from_rgba_premultiplied(0, 230, 255, 220);

// ---- Tree row alt + drop indicator ----
pub const ROW_ALT_BG: Color32 = Color32::from_rgba_premultiplied(255, 255, 255, 6);
pub const DROP_INDICATOR: Color32 = Color32::from_rgb(0, 180, 255);
pub const MARQUEE_FILL: Color32 = Color32::from_rgba_premultiplied(0, 180, 255, 30);
pub const MARQUEE_STROKE: Color32 = Color32::from_rgb(0, 180, 255);

// ---- Placeholders / warnings ----
pub const PLACEHOLDER_SPRITE: Color32 = Color32::from_rgba_premultiplied(255, 0, 220, 200);
pub const WARN_TEXT: Color32 = Color32::YELLOW;
