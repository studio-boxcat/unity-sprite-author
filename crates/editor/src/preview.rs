//! Center panel: 2D preview of the active tree's composed mesh + interactive
//! editing. Architecture:
//!
//!   build_view(...) → PreviewView { mesh, parts: [PartInfo] }
//!                                       ↑ uses combine::build_combined_with_ranges
//!   render(...)     → draws mesh + overlays + handles
//!   interact(...)   → reads input, emits TreeOps via app.pending_ops
//!
//! Persistent pan/zoom in `App.views`; rebuilt only when needed.

use crate::app::{App, GuideDrag, NodeEdit, TreeOp, ViewState};
use crate::doc::NodePath;
use crate::tree_panel;
use unity_sprite_author::combine::{self, AtlasSize, BuildOutput, CombinedMesh};
use unity_sprite_author::fab;
use unity_sprite_author::manifest::{self, Graphic, Node, Output};
use unity_sprite_author::tpsheet::{
    Border, Geometry, Pivot, Rect, SpriteAlignment, SpriteEntry,
};

const POS_SNAP: f32 = 0.25;
const HANDLE_R: f32 = 5.0;
const VERTEX_HANDLE_R: f32 = 5.0;
const RULER_PX: f32 = 18.0;
const GUIDE_HIT_PX: f32 = 4.0;
const GUIDE_SNAP_PX: f32 = 6.0;

pub fn show(ui: &mut egui::Ui, app: &mut App) {
    let Some(tab) = app.active_tab() else {
        ui.centered_and_justified(|ui| ui.label("(open a .tps.fab.json to start)"));
        return;
    };
    let doc_idx = tab.doc;
    // Tree to render = selection's primary tree, falling back to the active
    // doc's first tree. This decouples the preview from "the active tab" now
    // that tabs are per-file (a file can hold many combined trees).
    let tree_idx = app
        .selection
        .primary()
        .filter(|p| p.doc == doc_idx)
        .map(|p| p.tree)
        .unwrap_or(0);
    let view_key = (doc_idx, tree_idx);

    let (tree_clone, is_sma) = {
        let Some(doc) = app.docs.get(doc_idx) else { return; };
        let Some(tree) = doc.manifest.trees.get(tree_idx) else {
            ui.centered_and_justified(|ui| ui.label("(empty manifest — add a combined tree)"));
            return;
        };
        (tree.clone(), matches!(tree.output, Output::Sma { .. }))
    };

    if is_sma {
        ui.heading(format!("{}  ·  sma", tree_clone.name));
        ui.label("SMA preview not implemented yet — see CSA trees for live preview.");
        return;
    }

    let ctx = ui.ctx().clone();
    let view_built = match build_view(app, doc_idx, tree_idx, &tree_clone, &ctx) {
        Ok(v) => v,
        Err(msg) => {
            ui.colored_label(egui::Color32::YELLOW, msg);
            return;
        }
    };

    // ----- Toolbar -----
    let mut do_fit = false;
    ui.horizontal(|ui| {
        if ui.button("Fit").on_hover_text("Re-fit view to mesh AABB").clicked() {
            do_fit = true;
        }
        ui.separator();
        let view = app.views.entry(view_key).or_default();
        ui.label(format!("zoom: {:.1}x", view.zoom / 100.0));
        ui.label(format!("center: ({:.1}, {:.1})", view.center_world[0], view.center_world[1]));
    });

    // ----- Canvas -----
    let avail = ui.available_size();
    let (full_resp, painter) = ui.allocate_painter(avail, egui::Sense::click_and_drag());
    let full_rect = full_resp.rect;
    painter.rect_filled(full_rect, 0.0, egui::Color32::from_gray(28));
    // Carve out ruler strips along the top + left so canvas coords don't
    // collide with ruler hit-tests.
    let top_ruler = egui::Rect::from_min_max(
        full_rect.left_top() + egui::vec2(RULER_PX, 0.0),
        egui::pos2(full_rect.right(), full_rect.top() + RULER_PX),
    );
    let left_ruler = egui::Rect::from_min_max(
        full_rect.left_top() + egui::vec2(0.0, RULER_PX),
        egui::pos2(full_rect.left() + RULER_PX, full_rect.bottom()),
    );
    let corner = egui::Rect::from_min_max(
        full_rect.left_top(),
        full_rect.left_top() + egui::vec2(RULER_PX, RULER_PX),
    );
    let rect = egui::Rect::from_min_max(
        full_rect.left_top() + egui::vec2(RULER_PX, RULER_PX),
        full_rect.max,
    );
    painter.rect_filled(top_ruler, 0.0, egui::Color32::from_gray(40));
    painter.rect_filled(left_ruler, 0.0, egui::Color32::from_gray(40));
    painter.rect_filled(corner, 0.0, egui::Color32::from_gray(48));
    let resp = full_resp; // re-used; hit-tests below check the inset rect

    {
        let view = app.views.entry(view_key).or_default();
        if view.needs_fit || do_fit {
            *view = fit_view(&view_built.mesh, rect);
        }
    }
    let view = *app.views.get(&view_key).unwrap();
    let xform = ScreenTransform::from_view(&view, rect);

    paint_mesh(&painter, &view_built, &xform, &app.prefs);
    paint_world_overlays(&painter, &view_built.mesh, &xform, &app.prefs);
    if app.prefs.show_part_outlines {
        paint_part_outlines(&painter, &view_built, &xform, app);
    }
    if app.prefs.show_pivot_markers {
        paint_pivot_markers(&painter, &view_built, &xform, app);
    }

    handle_view_input(app, view_key, &resp, rect, &ctx);

    let cs = tree_clone.output.canvas_scale_implicit();
    paint_size_handles(&painter, &view_built, &xform, app, cs);
    paint_rulers(&painter, top_ruler, left_ruler, &xform);
    paint_guides(&painter, app, (doc_idx, tree_idx), rect, &xform);
    handle_guide_interaction(app, &resp, &painter, (doc_idx, tree_idx), top_ruler, left_ruler, rect, &xform, &ctx);

    // Mouse-down selection / marquee start / click rotation through stack.
    let primary_pressed = ctx.input(|i| i.pointer.primary_pressed());
    let secondary_pressed = ctx.input(|i| i.pointer.secondary_pressed());
    let hover_pos = ctx.input(|i| i.pointer.hover_pos());
    let hovered_part = hover_pos
        .filter(|p| rect.contains(*p))
        .and_then(|p| view_built.hit_test_part(p, &xform));
    let hovered_handle = hover_pos
        .filter(|p| rect.contains(*p))
        .and_then(|p| view_built.hit_test_handle(p, &xform, app, cs));

    // Click-rotate anchor invalidation: if cursor drifted past tolerance, drop.
    const CLICK_ROTATE_TOLERANCE_PX: f32 = 4.0;
    if let (Some(state), Some(hp)) = (app.click_rotate.as_ref(), hover_pos) {
        if (hp - state.anchor_screen).length() > CLICK_ROTATE_TOLERANCE_PX {
            app.click_rotate = None;
        }
    }

    if primary_pressed && rect.contains(hover_pos.unwrap_or(egui::Pos2::ZERO)) {
        if hovered_handle.is_none() && hovered_part.is_none() {
            let (cmd, shift) = ctx.input(|i| (i.modifiers.command, i.modifiers.shift));
            // Shift-click on empty canvas with a polygon leaf selected: if
            // the click is close to the polygon perimeter, insert a vertex
            // there instead of starting a marquee.
            let inserted = if shift {
                try_insert_polygon_vertex(app, &tree_clone, &view_built, hover_pos.unwrap(), &xform)
            } else { false };
            if !inserted {
                if !cmd && !shift {
                    app.selection.clear();
                }
                app.marquee_origin = hover_pos;
                app.click_rotate = None;
            }
        } else if let Some(pos) = hover_pos {
            let (cmd, shift) = ctx.input(|i| (i.modifiers.command, i.modifiers.shift));
            let path = click_rotate_pick(app, &view_built, pos, &xform, cmd, shift);
            if let Some(path) = path {
                tree_panel::apply_click_modifier_path(app, path, cmd, shift);
            }
        }
    }
    // Right-click: cycle through the z-stack at the cursor (Photoshop-style).
    if secondary_pressed && rect.contains(hover_pos.unwrap_or(egui::Pos2::ZERO)) {
        if let Some(pos) = hover_pos {
            // Right-click always advances (never starts a marquee, never
            // resets the rotation if we already have one at this spot).
            if let Some(path) = click_rotate_pick(app, &view_built, pos, &xform, false, false) {
                app.selection.set_single(path);
            }
        }
    }

    // Marquee rendering + commit on release.
    if let (Some(origin), Some(cur)) = (app.marquee_origin, hover_pos) {
        let r = egui::Rect::from_two_pos(origin, cur);
        painter.rect_filled(r, 0.0, egui::Color32::from_rgba_unmultiplied(0, 180, 255, 30));
        painter.rect_stroke(r, 0.0, egui::Stroke::new(1.0, egui::Color32::from_rgb(0, 180, 255)));
        if ctx.input(|i| i.pointer.primary_released()) {
            commit_marquee(app, &view_built, &xform, r, &ctx);
            app.marquee_origin = None;
        }
    } else if app.marquee_origin.is_some() && !resp.is_pointer_button_down_on() {
        app.marquee_origin = None;
    }

    // Drag interactions: handles → vertex → node, in priority order.
    handle_size_handle_drag(app, &resp, &xform, &view_built, &tree_clone, cs);
    handle_polygon_vertex_drag(app, &painter, &resp, &xform, &view_built, &tree_clone);
    handle_node_drag(app, &resp, &xform, cs, (doc_idx, tree_idx));

    if resp.drag_stopped() {
        app.in_drag_chain = false;
        app.dragging_size_handle = None;
        app.dragging_polygon_vertex = None;
    }

    paint_hud(&painter, rect, &tree_clone, &view_built.mesh);
}

// =============================================================================
// View build: walk + bridge + combine + build_combined_with_ranges
// =============================================================================

/// Resolved per-part info from a single `build_combined_with_ranges` call.
struct PartInfo {
    path: NodePath,
    range: (usize, usize),
    /// True when this part is a polygon leaf (used by the
    /// `prefs.show_polygon` toggle to hide them without affecting picking
    /// or per-part outlines).
    is_polygon: bool,
    /// `Some` when the part renders as a flat color (polygon or placeholder).
    color_override: Option<egui::Color32>,
    /// World-space pivot point — center of the per-part anchored position.
    pivot_world: [f32; 2],
    /// Composed leaf transform, needed for inverting screen drag back to local
    /// frame for polygon vertex / size handle math.
    affine: fab::Affine,
    /// World-space size of the part's rect bounding box (post-affine). `None`
    /// when the leaf has no explicit `size` field.
    rect_size_world: Option<[f32; 2]>,
}

struct PreviewView {
    mesh: CombinedMesh,
    parts: Vec<PartInfo>,
}

impl PreviewView {
    fn hit_test_part(&self, screen: egui::Pos2, xform: &ScreenTransform) -> Option<usize> {
        // Front-most first: iterate in reverse so a topmost part wins.
        for (i, info) in self.parts.iter().enumerate().rev() {
            if point_in_part(screen, &self.mesh, &info.range, xform) {
                return Some(i);
            }
        }
        None
    }

    /// All parts under `screen`, front-to-back (front = last in part order).
    /// Used by click-rotation to cycle through the stack.
    fn hit_test_parts_all(&self, screen: egui::Pos2, xform: &ScreenTransform) -> Vec<usize> {
        let mut hits = Vec::new();
        for (i, info) in self.parts.iter().enumerate().rev() {
            if point_in_part(screen, &self.mesh, &info.range, xform) {
                hits.push(i);
            }
        }
        hits
    }

    /// Returns `Some(handle_kind)` if the cursor is over a transform handle of
    /// the primary selection. Handle kinds index the 9-way rect handles.
    fn hit_test_handle(
        &self,
        screen: egui::Pos2,
        xform: &ScreenTransform,
        app: &App,
        cs: f32,
    ) -> Option<SizeHandle> {
        let primary = app.selection.primary()?;
        let info = self.parts.iter().find(|p| &p.path == primary)?;
        let rect_size = info.rect_size_world?;
        let center = info.pivot_world;
        let handles = handle_positions(center, rect_size, info.affine, cs);
        for (i, world) in handles.iter().enumerate() {
            let p = xform.world_to_screen(*world);
            if (p - screen).length() < HANDLE_R + 4.0 {
                return Some(SizeHandle::from_idx(i));
            }
        }
        None
    }
}

fn build_view(
    app: &mut App,
    doc_idx: usize,
    tree_idx: usize,
    tree: &manifest::Tree,
    ctx: &egui::Context,
) -> Result<BuiltView, String> {
    let Some(doc) = app.docs.get_mut(doc_idx) else { return Err("(stale tab)".into()); };
    let atlas_result = doc.atlas_mut();
    let (atlas_tex, atlas_size, sprite_lookup, invert_scales, ppu) = match atlas_result {
        Err(e) => return Err(format!("atlas unavailable: {e}")),
        Ok(atlas) => {
            let tex = atlas.atlas_texture(ctx);
            let size = AtlasSize {
                width: atlas.sheet.tex.width.max(1),
                height: atlas.sheet.tex.height.max(1),
            };
            let lookup: std::collections::HashMap<String, SpriteEntry> = atlas
                .sheet
                .sprites
                .iter()
                .map(|s| (s.name.clone(), s.clone()))
                .collect();
            let inv = atlas.invert_scales.clone();
            (tex, size, lookup, inv, atlas.ppu)
        }
    };
    let invert_scale_for = |name: &str| -> f32 {
        if let Some(s) = invert_scales.get(name) { return *s; }
        if let Some(idx) = name.rfind('-') {
            if let Some(s) = invert_scales.get(&name[idx + 1..]) { return *s; }
        }
        1.0
    };

    let combined = manifest::to_fab_combined(tree).map_err(|e| format!("bridge error: {e}"))?;
    if combined.parts.is_empty() {
        return Err("(empty tree — add a sprite or polygon child)".into());
    }

    let resolve = |name: &str| -> Option<(SpriteEntry, f32)> {
        if let Some(e) = sprite_lookup.get(name) { return Some((e.clone(), invert_scale_for(name))); }
        if name.is_empty() { return Some((fake_placeholder_entry(), 1.0)); }
        if name.starts_with("Color_") { return Some((fake_color_entry(name), 1.0)); }
        None
    };

    let output: BuildOutput = combine::build_combined_with_ranges(&combined, resolve, atlas_size, ppu)
        .map_err(|e| format!("combine error: {e}"))?;

    let leaves = manifest::walk(tree);
    let leaf_paths = leaf_node_paths(tree, doc_idx, tree_idx);
    debug_assert_eq!(leaves.len(), combined.parts.len());
    debug_assert_eq!(leaf_paths.len(), combined.parts.len());

    let cs = tree.output.canvas_scale_implicit();
    let mut parts = Vec::with_capacity(combined.parts.len());
    for (i, part) in combined.parts.iter().enumerate() {
        let range = output.part_ranges[i];
        let leaf = &leaves[i];
        let path = leaf_paths[i].clone();
        let color_override = match part {
            fab::Part::Polygon { polygon_sprite, .. } => {
                let hex = polygon_sprite.strip_prefix("Color_").unwrap_or(polygon_sprite);
                crate::inspector::parse_color_hex(hex)
            }
            fab::Part::AtlasSprite { sprite, .. } if sprite.is_empty() => {
                Some(egui::Color32::from_rgba_unmultiplied(255, 0, 220, 200))
            }
            _ => None,
        };
        let rect_size_world = match part {
            fab::Part::AtlasSprite { size: Some((w, h)), .. } => Some([*w, *h]),
            _ => None,
        };
        parts.push(PartInfo {
            path,
            range,
            is_polygon: matches!(part, fab::Part::Polygon { .. }),
            color_override,
            pivot_world: [leaf.world_pos[0] * cs, leaf.world_pos[1] * cs],
            affine: fab::Affine {
                tx: 0.0, ty: 0.0,
                sx: leaf.world_scale[0], sy: leaf.world_scale[1],
                rot_deg_ccw: leaf.world_rot_deg_ccw,
            },
            rect_size_world,
        });
    }

    Ok(BuiltView {
        view: PreviewView { mesh: output.mesh, parts },
        atlas_tex,
    })
}

struct BuiltView {
    view: PreviewView,
    atlas_tex: egui::TextureHandle,
}

impl std::ops::Deref for BuiltView {
    type Target = PreviewView;
    fn deref(&self) -> &PreviewView { &self.view }
}

// =============================================================================
// Coordinate transform
// =============================================================================

#[derive(Clone, Copy)]
struct ScreenTransform {
    scale: f32,
    origin: egui::Pos2,
}

impl ScreenTransform {
    fn from_view(view: &ViewState, screen_rect: egui::Rect) -> Self {
        let center = screen_rect.center();
        let origin = egui::pos2(
            center.x - view.center_world[0] * view.zoom,
            center.y + view.center_world[1] * view.zoom,
        );
        Self { scale: view.zoom, origin }
    }

    fn world_to_screen(&self, p: [f32; 2]) -> egui::Pos2 {
        egui::pos2(self.origin.x + p[0] * self.scale, self.origin.y - p[1] * self.scale)
    }

    fn screen_to_world(&self, p: egui::Pos2) -> [f32; 2] {
        [(p.x - self.origin.x) / self.scale, (self.origin.y - p.y) / self.scale]
    }

    fn screen_delta_to_world(&self, d: egui::Vec2) -> [f32; 2] {
        [d.x / self.scale, -d.y / self.scale]
    }
}

fn fit_view(mesh: &CombinedMesh, screen_rect: egui::Rect) -> ViewState {
    let (minx, miny, maxx, maxy) = aabb_2d(&mesh.verts);
    let w = (maxx - minx).max(1e-3);
    let h = (maxy - miny).max(1e-3);
    let padding = 32.0;
    let avail_w = (screen_rect.width() - padding * 2.0).max(1.0);
    let avail_h = (screen_rect.height() - padding * 2.0).max(1.0);
    let zoom = (avail_w / w).min(avail_h / h).max(1e-3);
    ViewState {
        center_world: [(minx + maxx) * 0.5, (miny + maxy) * 0.5],
        zoom,
        needs_fit: false,
    }
}

fn aabb_2d(verts: &[[f32; 2]]) -> (f32, f32, f32, f32) {
    let mut minx = f32::INFINITY;
    let mut miny = f32::INFINITY;
    let mut maxx = f32::NEG_INFINITY;
    let mut maxy = f32::NEG_INFINITY;
    for v in verts {
        minx = minx.min(v[0]);
        miny = miny.min(v[1]);
        maxx = maxx.max(v[0]);
        maxy = maxy.max(v[1]);
    }
    (minx, miny, maxx, maxy)
}

// =============================================================================
// Rendering
// =============================================================================

fn paint_mesh(
    painter: &egui::Painter,
    view: &BuiltView,
    xform: &ScreenTransform,
    prefs: &crate::preferences::Preferences,
) {
    // Set vertex colors. Suppressed parts (e.g. polygons hidden via the View
    // menu) get fully-transparent colors — the geometry still ships to GPU
    // but contributes nothing visible. Filtering triangles out instead would
    // also work; vertex-alpha is simpler and keeps part_ranges consistent for
    // picking + outlines (which we still want when polygons are hidden).
    let mut colors = vec![egui::Color32::WHITE; view.mesh.verts.len()];
    for info in &view.parts {
        let suppressed = info.is_polygon && !prefs.show_polygon;
        if suppressed {
            for v in &mut colors[info.range.0..info.range.1] {
                *v = egui::Color32::TRANSPARENT;
            }
            continue;
        }
        if let Some(c) = info.color_override {
            for v in &mut colors[info.range.0..info.range.1] {
                *v = c;
            }
        }
    }
    let mut egui_mesh = egui::Mesh::with_texture(view.atlas_tex.id());
    for (i, v) in view.mesh.verts.iter().enumerate() {
        let uv = view.mesh.uvs[i];
        egui_mesh.vertices.push(egui::epaint::Vertex {
            pos: xform.world_to_screen(*v),
            uv: egui::pos2(uv[0], 1.0 - uv[1]),
            color: colors[i],
        });
    }
    for chunk in view.mesh.tris.chunks(3) {
        if let [a, b, c] = *chunk {
            egui_mesh.indices.push(a as u32);
            egui_mesh.indices.push(b as u32);
            egui_mesh.indices.push(c as u32);
        }
    }
    painter.add(egui::Shape::mesh(egui_mesh));
}

fn paint_world_overlays(
    painter: &egui::Painter,
    mesh: &CombinedMesh,
    xform: &ScreenTransform,
    prefs: &crate::preferences::Preferences,
) {
    let origin = xform.world_to_screen([0.0, 0.0]);
    let stroke_axis = egui::Stroke::new(0.5, egui::Color32::from_gray(70));
    let len = 4000.0;
    painter.line_segment([origin - egui::vec2(len, 0.0), origin + egui::vec2(len, 0.0)], stroke_axis);
    painter.line_segment([origin - egui::vec2(0.0, len), origin + egui::vec2(0.0, len)], stroke_axis);

    if prefs.show_atlas_aabb {
        let (minx, miny, maxx, maxy) = aabb_2d(&mesh.verts);
        let aabb = egui::Rect::from_two_pos(
            xform.world_to_screen([minx, miny]),
            xform.world_to_screen([maxx, maxy]),
        );
        painter.rect_stroke(aabb, 0.0, egui::Stroke::new(0.5, egui::Color32::from_gray(100)));
    }
}

fn paint_part_outlines(painter: &egui::Painter, view: &BuiltView, xform: &ScreenTransform, app: &App) {
    for info in &view.parts {
        let is_selected = app.selection.is_selected(&info.path);
        let color = if is_selected {
            egui::Color32::from_rgb(255, 200, 0)
        } else {
            egui::Color32::from_rgba_unmultiplied(255, 255, 255, 48)
        };
        let stroke_w = if is_selected { 1.5 } else { 0.4 };
        let stroke = egui::Stroke::new(stroke_w, color);
        // Render only perimeter edges (edges that appear in exactly one
        // triangle of this part). Interior triangulation lines were
        // visually noisy and aren't meaningful to the user.
        let boundary = boundary_edges(&view.mesh.tris, info.range);
        for (a, b) in boundary {
            let p_a = xform.world_to_screen(view.mesh.verts[a]);
            let p_b = xform.world_to_screen(view.mesh.verts[b]);
            painter.line_segment([p_a, p_b], stroke);
        }
    }
}

/// Boundary edges = edges with odd incidence count across the part's
/// triangles. For a clean 2D mesh, that's edges shared by exactly one
/// triangle (the perimeter). Edges between two triangles cancel out.
fn boundary_edges(tris: &[u16], range: (usize, usize)) -> Vec<(usize, usize)> {
    use std::collections::HashMap;
    let (start, end) = range;
    let mut counts: HashMap<(usize, usize), i32> = HashMap::new();
    for tri in tris.chunks(3) {
        if let [a, b, c] = *tri {
            let ai = a as usize;
            if ai < start || ai >= end { continue; }
            let bi = b as usize;
            let ci = c as usize;
            for (u, v) in [(ai, bi), (bi, ci), (ci, ai)] {
                let key = if u < v { (u, v) } else { (v, u) };
                *counts.entry(key).or_insert(0) += 1;
            }
        }
    }
    counts.into_iter().filter(|(_, n)| *n == 1).map(|(e, _)| e).collect()
}

fn paint_pivot_markers(painter: &egui::Painter, view: &BuiltView, xform: &ScreenTransform, app: &App) {
    for info in &view.parts {
        let is_selected = app.selection.is_selected(&info.path);
        let (r, color) = if is_selected {
            (5.0, egui::Color32::from_rgb(255, 200, 0))
        } else {
            (3.0, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 140))
        };
        let p = xform.world_to_screen(info.pivot_world);
        let stroke = egui::Stroke::new(1.0, color);
        painter.line_segment([p - egui::vec2(r, 0.0), p + egui::vec2(r, 0.0)], stroke);
        painter.line_segment([p - egui::vec2(0.0, r), p + egui::vec2(0.0, r)], stroke);
        painter.circle_stroke(p, r * 0.4, egui::Stroke::new(0.75, color));
    }
}

fn paint_size_handles(
    painter: &egui::Painter,
    view: &BuiltView,
    xform: &ScreenTransform,
    app: &App,
    _cs: f32,
) {
    let Some(primary) = app.selection.primary() else { return; };
    let Some(info) = view.parts.iter().find(|p| &p.path == primary) else { return; };
    let Some(rect_size) = info.rect_size_world else { return; };
    let positions = handle_positions(info.pivot_world, rect_size, info.affine, 1.0);
    let stroke = egui::Stroke::new(1.0, egui::Color32::BLACK);
    for (i, w) in positions.iter().enumerate() {
        let p = xform.world_to_screen(*w);
        let handle = SizeHandle::from_idx(i);
        if handle == SizeHandle::Mid { continue; }
        let r = if handle == SizeHandle::Rotate { 4.5 } else { 4.0 };
        let fill = if handle == SizeHandle::Rotate {
            egui::Color32::from_rgb(80, 200, 120)
        } else {
            egui::Color32::from_rgb(255, 200, 0)
        };
        painter.circle_filled(p, r, fill);
        painter.circle_stroke(p, r, stroke);
        // Visual tether: line from top-center to rotation handle so the user
        // recognizes its origin.
        if handle == SizeHandle::Rotate {
            let top_center_screen = xform.world_to_screen(positions[1]);
            painter.line_segment([top_center_screen, p], egui::Stroke::new(0.5, egui::Color32::from_gray(120)));
        }
    }
}

/// Render pixel ruler ticks along the top + left edges of the canvas.
/// Tick step in world units auto-scales so the screen-space spacing stays
/// near the target band (`MIN_TICK_PX`..`MAX_TICK_PX`).
fn paint_rulers(
    painter: &egui::Painter,
    top: egui::Rect,
    left: egui::Rect,
    xform: &ScreenTransform,
) {
    const MIN_TICK_PX: f32 = 50.0;
    let step = nice_tick_step(MIN_TICK_PX / xform.scale.max(1e-3));
    let tick_color = egui::Color32::from_gray(160);
    let label_color = egui::Color32::from_gray(200);
    let stroke = egui::Stroke::new(0.5, tick_color);

    // Top ruler: ticks at world x = k*step, drawn at screen x = world_to_screen([x, 0]).x.
    let world_left = xform.screen_to_world(top.left_top())[0];
    let world_right = xform.screen_to_world(top.right_top())[0];
    let first = (world_left / step).floor() as i64;
    let last = (world_right / step).ceil() as i64;
    for k in first..=last {
        let x_world = k as f32 * step;
        let sp = xform.world_to_screen([x_world, 0.0]);
        let x = sp.x;
        if x < top.left() || x > top.right() { continue; }
        let major = k.rem_euclid(5) == 0;
        let h = if major { RULER_PX * 0.7 } else { RULER_PX * 0.35 };
        painter.line_segment([egui::pos2(x, top.bottom() - h), egui::pos2(x, top.bottom())], stroke);
        if major {
            painter.text(
                egui::pos2(x + 2.0, top.top()),
                egui::Align2::LEFT_TOP,
                format!("{x_world:.0}"),
                egui::FontId::monospace(9.0),
                label_color,
            );
        }
    }
    // Left ruler.
    let world_top = xform.screen_to_world(left.left_top())[1];
    let world_bottom = xform.screen_to_world(left.left_bottom())[1];
    let first = (world_bottom / step).floor() as i64;
    let last = (world_top / step).ceil() as i64;
    for k in first..=last {
        let y_world = k as f32 * step;
        let sp = xform.world_to_screen([0.0, y_world]);
        let y = sp.y;
        if y < left.top() || y > left.bottom() { continue; }
        let major = k.rem_euclid(5) == 0;
        let w = if major { RULER_PX * 0.7 } else { RULER_PX * 0.35 };
        painter.line_segment([egui::pos2(left.right() - w, y), egui::pos2(left.right(), y)], stroke);
        if major {
            painter.text(
                egui::pos2(left.left() + 1.0, y - 1.0),
                egui::Align2::LEFT_BOTTOM,
                format!("{y_world:.0}"),
                egui::FontId::monospace(9.0),
                label_color,
            );
        }
    }
}

/// "Nice" tick step for the ruler — a power-of-10 multiple of {1, 2, 5}
/// just above `min_step` so the visible numbers stay round.
fn nice_tick_step(min_step: f32) -> f32 {
    if !min_step.is_finite() || min_step <= 0.0 { return 1.0; }
    let exp = min_step.log10().floor();
    let base = 10f32.powf(exp);
    for m in [1.0_f32, 2.0, 5.0, 10.0] {
        if base * m >= min_step {
            return base * m;
        }
    }
    base * 10.0
}

fn paint_guides(
    painter: &egui::Painter,
    app: &App,
    key: crate::app::ViewKey,
    rect: egui::Rect,
    xform: &ScreenTransform,
) {
    let Some(set) = app.guides.get(&key) else { return; };
    let stroke = egui::Stroke::new(0.7, egui::Color32::from_rgba_unmultiplied(0, 200, 255, 200));
    for x in &set.vertical {
        let sx = xform.world_to_screen([*x, 0.0]).x;
        if sx < rect.left() || sx > rect.right() { continue; }
        painter.line_segment([egui::pos2(sx, rect.top()), egui::pos2(sx, rect.bottom())], stroke);
    }
    for y in &set.horizontal {
        let sy = xform.world_to_screen([0.0, *y]).y;
        if sy < rect.top() || sy > rect.bottom() { continue; }
        painter.line_segment([egui::pos2(rect.left(), sy), egui::pos2(rect.right(), sy)], stroke);
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_guide_interaction(
    app: &mut App,
    resp: &egui::Response,
    painter: &egui::Painter,
    key: crate::app::ViewKey,
    top: egui::Rect,
    left: egui::Rect,
    canvas: egui::Rect,
    xform: &ScreenTransform,
    ctx: &egui::Context,
) {
    let Some(cursor) = ctx.input(|i| i.pointer.hover_pos()) else { return; };
    let primary_pressed = ctx.input(|i| i.pointer.primary_pressed());
    let primary_down = ctx.input(|i| i.pointer.primary_down());
    let primary_released = ctx.input(|i| i.pointer.primary_released());

    // Start a new drag.
    if primary_pressed && app.guide_drag.is_none() {
        if top.contains(cursor) {
            app.guide_drag = Some(GuideDrag::AddVertical);
        } else if left.contains(cursor) {
            app.guide_drag = Some(GuideDrag::AddHorizontal);
        } else if canvas.contains(cursor) {
            // Hit-test existing guides.
            if let Some((axis, idx)) = hit_test_guide(app, key, cursor, xform) {
                app.guide_drag = Some(match axis {
                    GuideAxis::Vertical => GuideDrag::MoveVertical(idx),
                    GuideAxis::Horizontal => GuideDrag::MoveHorizontal(idx),
                });
            }
        }
    }

    // While dragging, show a live preview line + update the position.
    if let Some(drag) = app.guide_drag {
        let preview_stroke = egui::Stroke::new(0.7, egui::Color32::from_rgba_unmultiplied(0, 230, 255, 220));
        let world = xform.screen_to_world(cursor);
        match drag {
            GuideDrag::AddVertical | GuideDrag::MoveVertical(_) => {
                let x = world[0];
                let sx = xform.world_to_screen([x, 0.0]).x;
                painter.line_segment([egui::pos2(sx, canvas.top()), egui::pos2(sx, canvas.bottom())], preview_stroke);
            }
            GuideDrag::AddHorizontal | GuideDrag::MoveHorizontal(_) => {
                let y = world[1];
                let sy = xform.world_to_screen([0.0, y]).y;
                painter.line_segment([egui::pos2(canvas.left(), sy), egui::pos2(canvas.right(), sy)], preview_stroke);
            }
        }
    }

    // Commit / delete on release.
    if primary_released {
        if let Some(drag) = app.guide_drag.take() {
            let world = xform.screen_to_world(cursor);
            let set = app.guides.entry(key).or_default();
            let out_of_canvas = !canvas.contains(cursor);
            match drag {
                GuideDrag::AddVertical => {
                    if canvas.contains(cursor) { set.vertical.push(world[0]); }
                }
                GuideDrag::AddHorizontal => {
                    if canvas.contains(cursor) { set.horizontal.push(world[1]); }
                }
                GuideDrag::MoveVertical(i) => {
                    if out_of_canvas {
                        if i < set.vertical.len() { set.vertical.remove(i); }
                    } else if let Some(v) = set.vertical.get_mut(i) {
                        *v = world[0];
                    }
                }
                GuideDrag::MoveHorizontal(i) => {
                    if out_of_canvas {
                        if i < set.horizontal.len() { set.horizontal.remove(i); }
                    } else if let Some(v) = set.horizontal.get_mut(i) {
                        *v = world[1];
                    }
                }
            }
        }
    } else if !primary_down {
        // Mouse left the drag without a press registered (e.g. click outside).
        // Don't leak the drag state.
        // Note: don't clear here — pointer_released path covers normal cases.
    }
    let _ = resp;
}

#[derive(Copy, Clone)]
enum GuideAxis { Vertical, Horizontal }

fn hit_test_guide(
    app: &App,
    key: crate::app::ViewKey,
    screen: egui::Pos2,
    xform: &ScreenTransform,
) -> Option<(GuideAxis, usize)> {
    let set = app.guides.get(&key)?;
    for (i, x) in set.vertical.iter().enumerate() {
        let sx = xform.world_to_screen([*x, 0.0]).x;
        if (screen.x - sx).abs() < GUIDE_HIT_PX {
            return Some((GuideAxis::Vertical, i));
        }
    }
    for (i, y) in set.horizontal.iter().enumerate() {
        let sy = xform.world_to_screen([0.0, *y]).y;
        if (screen.y - sy).abs() < GUIDE_HIT_PX {
            return Some((GuideAxis::Horizontal, i));
        }
    }
    None
}

/// Snap `world_pos` to the nearest guide on each axis if within
/// `GUIDE_SNAP_PX` (screen-space). Returns the possibly-snapped pos.
fn snap_to_guides(
    app: &App,
    key: crate::app::ViewKey,
    world_pos: [f32; 2],
    xform: &ScreenTransform,
) -> [f32; 2] {
    let Some(set) = app.guides.get(&key) else { return world_pos; };
    let mut out = world_pos;
    let screen = xform.world_to_screen(world_pos);
    for x in &set.vertical {
        let sx = xform.world_to_screen([*x, 0.0]).x;
        if (screen.x - sx).abs() < GUIDE_SNAP_PX {
            out[0] = *x;
            break;
        }
    }
    for y in &set.horizontal {
        let sy = xform.world_to_screen([0.0, *y]).y;
        if (screen.y - sy).abs() < GUIDE_SNAP_PX {
            out[1] = *y;
            break;
        }
    }
    out
}

fn paint_hud(painter: &egui::Painter, rect: egui::Rect, tree: &manifest::Tree, mesh: &CombinedMesh) {
    let text = format!(
        "{} · {} parts · {} verts · {} tris",
        tree.name,
        mesh.tris.len() / 3,
        mesh.verts.len(),
        mesh.tris.len() / 3,
    );
    painter.text(
        rect.left_top() + egui::vec2(8.0, 8.0),
        egui::Align2::LEFT_TOP,
        text,
        egui::FontId::monospace(11.0),
        egui::Color32::LIGHT_GRAY,
    );
}

// =============================================================================
// Pan / zoom
// =============================================================================

fn handle_view_input(app: &mut App, view_key: crate::app::ViewKey, resp: &egui::Response, rect: egui::Rect, ctx: &egui::Context) {
    if !resp.hovered() { return; }
    let hover_pos = ctx.input(|i| i.pointer.hover_pos());
    let zoom_delta = ctx.input(|i| i.zoom_delta());
    let scroll = ctx.input(|i| i.smooth_scroll_delta);
    let modifiers = ctx.input(|i| i.modifiers);

    let view = app.views.entry(view_key).or_default();
    let mut z = view.zoom;
    let mut c = view.center_world;

    if (zoom_delta - 1.0).abs() > 1e-4 {
        zoom_around(&mut z, &mut c, rect, hover_pos, zoom_delta);
    }
    if scroll != egui::Vec2::ZERO {
        if modifiers.command {
            let factor = (1.0 + scroll.y * 0.005).clamp(0.5, 2.0);
            zoom_around(&mut z, &mut c, rect, hover_pos, factor);
        } else {
            c[0] -= scroll.x / z;
            c[1] += scroll.y / z;
        }
    }
    view.zoom = z;
    view.center_world = c;
}

fn zoom_around(z: &mut f32, c: &mut [f32; 2], rect: egui::Rect, hover: Option<egui::Pos2>, factor: f32) {
    if let Some(cursor) = hover {
        let xf = ScreenTransform {
            scale: *z,
            origin: egui::pos2(rect.center().x - c[0] * *z, rect.center().y + c[1] * *z),
        };
        let world = xf.screen_to_world(cursor);
        *z = (*z * factor).clamp(0.5, 50_000.0);
        let new_origin = egui::pos2(cursor.x - world[0] * *z, cursor.y + world[1] * *z);
        c[0] = (rect.center().x - new_origin.x) / *z;
        c[1] = (new_origin.y - rect.center().y) / *z;
    } else {
        *z = (*z * factor).clamp(0.5, 50_000.0);
    }
}

/// Shift-click vertex insertion. Returns `true` if a vertex was inserted, so
/// the caller can suppress the marquee that would otherwise start.
fn try_insert_polygon_vertex(
    app: &mut App,
    tree: &manifest::Tree,
    view: &BuiltView,
    screen: egui::Pos2,
    xform: &ScreenTransform,
) -> bool {
    let Some(path) = app.selection.primary().cloned() else { return false; };
    let Some(info) = view.parts.iter().find(|p| p.path == path) else { return false; };
    let Some(node) = resolve_in_tree(tree, &path) else { return false; };
    let Some(Graphic::Polygon { vertices, .. }) = &node.graphic else { return false; };
    if vertices.len() < 3 { return false; }

    let world = xform.screen_to_world(screen);
    // Convert click → polygon local frame.
    let local_world = [world[0] - info.pivot_world[0], world[1] - info.pivot_world[1]];
    let local = invert_affine_delta(local_world, &info.affine);

    let (idx, projected) = match project_onto_polygon_perimeter(local, vertices) {
        Some(x) => x,
        None => return false,
    };

    // Reject inserts too far from the actual perimeter to avoid accidental
    // adds. Tolerance scales with zoom so the threshold feels consistent.
    let d_local = ((projected[0] - local[0]).powi(2) + (projected[1] - local[1]).powi(2)).sqrt();
    let max_screen_px = 16.0;
    let max_local = max_screen_px / xform.scale / info.affine.sx.abs().max(1e-3);
    if d_local > max_local { return false; }

    app.pending_ops.push(TreeOp::Edit {
        path,
        edit: NodeEdit::PolygonInsertVertex { idx, value: projected },
    });
    true
}

/// Pick the next part under `pos` using the click-rotation state in `app`.
/// First click at a fresh location picks the front-most; subsequent clicks
/// within `CLICK_ROTATE_TOLERANCE_PX` cycle through underlying parts.
fn click_rotate_pick(
    app: &mut App,
    view: &BuiltView,
    pos: egui::Pos2,
    xform: &ScreenTransform,
    _cmd: bool,
    _shift: bool,
) -> Option<crate::doc::NodePath> {
    let hits = view.hit_test_parts_all(pos, xform);
    if hits.is_empty() {
        app.click_rotate = None;
        return None;
    }
    let new_state = match app.click_rotate.take() {
        Some(prev) if prev.parts == hits => {
            // Same stack — advance.
            let next = (prev.cursor_index + 1) % hits.len();
            crate::app::ClickRotateState { anchor_screen: pos, parts: hits.clone(), cursor_index: next }
        }
        _ => {
            // Fresh location — start at front.
            crate::app::ClickRotateState { anchor_screen: pos, parts: hits.clone(), cursor_index: 0 }
        }
    };
    let part_idx = new_state.parts[new_state.cursor_index];
    let path = view.parts.get(part_idx).map(|p| p.path.clone());
    app.click_rotate = Some(new_state);
    path
}

// =============================================================================
// Marquee multi-select
// =============================================================================

fn commit_marquee(app: &mut App, view: &BuiltView, xform: &ScreenTransform, marquee: egui::Rect, ctx: &egui::Context) {
    let (cmd, shift) = ctx.input(|i| (i.modifiers.command, i.modifiers.shift));
    let mut paths_under = Vec::new();
    for info in &view.parts {
        if part_intersects_marquee(info, &view.mesh, xform, marquee) {
            paths_under.push(info.path.clone());
        }
    }
    if cmd {
        for p in paths_under { app.selection.toggle(p); }
    } else if shift {
        for p in paths_under { app.selection.extend(p); }
    } else {
        app.selection.replace_with(paths_under);
    }
}

fn part_intersects_marquee(info: &PartInfo, mesh: &CombinedMesh, xform: &ScreenTransform, marquee: egui::Rect) -> bool {
    for v in &mesh.verts[info.range.0..info.range.1] {
        if marquee.contains(xform.world_to_screen(*v)) {
            return true;
        }
    }
    // Conversely, any marquee corner inside the part's AABB also counts.
    let (minx, miny, maxx, maxy) = aabb_2d(&mesh.verts[info.range.0..info.range.1]);
    let part_screen = egui::Rect::from_two_pos(
        xform.world_to_screen([minx, miny]),
        xform.world_to_screen([maxx, maxy]),
    );
    part_screen.intersects(marquee)
}

// =============================================================================
// 9-way rect handles
// =============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizeHandle {
    Nw, N,  Ne,
    W,  Mid, E,
    Sw, S,  Se,
    /// Sits above the top-center handle. Dragging rotates the leaf about its
    /// pivot. Shift snaps to 15°.
    Rotate,
}

impl SizeHandle {
    fn from_idx(i: usize) -> Self {
        use SizeHandle::*;
        const TABLE: [SizeHandle; 10] = [Nw, N, Ne, W, Mid, E, Sw, S, Se, Rotate];
        TABLE[i]
    }

    /// (dx_left, dy_top, dx_right, dy_bottom) edge mask: which edges this
    /// handle drags. `Mid` / `Rotate` return all-false (size-unchanged).
    fn edge_mask(self) -> (bool, bool, bool, bool) {
        use SizeHandle::*;
        match self {
            Nw  => (true,  true,  false, false),
            N   => (false, true,  false, false),
            Ne  => (false, true,  true,  false),
            W   => (true,  false, false, false),
            Mid => (false, false, false, false),
            E   => (false, false, true,  false),
            Sw  => (true,  false, false, true),
            S   => (false, false, false, true),
            Se  => (false, false, true,  true),
            Rotate => (false, false, false, false),
        }
    }
}

/// World positions of the 10 handles for a part's rect — the 9-way grid in
/// row-major (NW, N, NE, W, MID, E, SW, S, SE) order, plus the rotation
/// handle at index 9, sitting above the top-center handle.
fn handle_positions(pivot_world: [f32; 2], rect_size: [f32; 2], affine: fab::Affine, cs: f32) -> [[f32; 2]; 10] {
    let hw = rect_size[0] * 0.5 * cs;
    let hh = rect_size[1] * 0.5 * cs;
    // Rotation handle offset in local frame: above the top-center, offset
    // by a screen-relative-ish stub. Using local units keeps it consistent
    // with the rest of the handle layout under rotation.
    let rotate_offset = (hh + (rect_size[1].abs().max(rect_size[0].abs()) * cs * 0.25).max(0.5)).max(hh + 0.5);
    let local = [
        [-hw,  hh], [0.0,  hh], [hw,  hh],
        [-hw, 0.0], [0.0, 0.0], [hw, 0.0],
        [-hw, -hh], [0.0, -hh], [hw, -hh],
        [0.0, rotate_offset],
    ];
    let mut out = [[0.0; 2]; 10];
    let r = affine.rot_deg_ccw.to_radians();
    let (s, c) = r.sin_cos();
    for (i, p) in local.iter().enumerate() {
        let sx = p[0] * affine.sx.signum();
        let sy = p[1] * affine.sy.signum();
        let rx = sx * c - sy * s;
        let ry = sx * s + sy * c;
        out[i] = [pivot_world[0] + rx, pivot_world[1] + ry];
    }
    out
}

fn handle_size_handle_drag(
    app: &mut App,
    resp: &egui::Response,
    xform: &ScreenTransform,
    view: &BuiltView,
    tree: &manifest::Tree,
    cs: f32,
) {
    if resp.drag_started() && app.dragging_size_handle.is_none() {
        if let Some(pos) = resp.hover_pos() {
            if let Some(h) = view.hit_test_handle(pos, xform, app, cs) {
                if h != SizeHandle::Mid {
                    app.dragging_size_handle = Some(h);
                }
            }
        }
    }
    let Some(handle) = app.dragging_size_handle else { return; };
    if !resp.dragged() { return; }
    let delta = resp.drag_delta();
    if delta == egui::Vec2::ZERO { return; }

    let Some(path) = app.selection.primary().cloned() else { return; };
    let Some(info) = view.parts.iter().find(|p| p.path == path) else { return; };

    // Rotation handle drives `rot_deg_ccw` directly, not size.
    if handle == SizeHandle::Rotate {
        let Some(cursor) = resp.hover_pos() else { return; };
        let cursor_world = xform.screen_to_world(cursor);
        let dx = cursor_world[0] - info.pivot_world[0];
        let dy = cursor_world[1] - info.pivot_world[1];
        if dx.abs() < 1e-6 && dy.abs() < 1e-6 { return; }
        // Handle's local frame is "up" (+Y). After rot of θ, up maps to
        // (-sin θ, cos θ). We want that to point at the cursor → sin θ = -dx_n,
        // cos θ = dy_n  ⇒  θ = atan2(-dx, dy).
        let mut new_rot = (-dx).atan2(dy).to_degrees();
        let shift = resp.ctx.input(|i| i.modifiers.shift);
        if shift {
            // Snap to 15° increments.
            new_rot = (new_rot / 15.0).round() * 15.0;
        }
        app.pending_ops.push(TreeOp::Edit {
            path,
            edit: NodeEdit::Rot(new_rot),
        });
        return;
    }

    let Some(cur_size) = info.rect_size_world else { return; };
    // Drag delta in unrotated local frame (un-rotate then scale-canvas back).
    let world_d = xform.screen_delta_to_world(delta);
    let r = (-info.affine.rot_deg_ccw).to_radians();
    let (sr, cr) = r.sin_cos();
    let local_dx = (world_d[0] * cr - world_d[1] * sr) / cs;
    let local_dy = (world_d[0] * sr + world_d[1] * cr) / cs;

    let (dl, dt, dr_, db) = handle.edge_mask();
    let (cmd, shift, alt) = resp.ctx.input(|i| (i.modifiers.command, i.modifiers.shift, i.modifiers.alt));
    let snap = !shift && !cmd; // Shift = aspect-lock, Cmd = no snap.
    let from_center = alt;

    let mut w = cur_size[0] / cs;
    let mut h = cur_size[1] / cs;

    // Apply edge contributions. Local frame: +x = right, +y = up (world Y).
    // Edges: top edge moves with +y; bottom with -y; right with +x; left with -x.
    // Increasing w when right edge moves right (dr=+local_dx) or left moves left (-local_dx).
    let dw = (if dr_ { local_dx } else { 0.0 }) - (if dl { local_dx } else { 0.0 });
    let dh = (if dt { local_dy } else { 0.0 }) - (if db { local_dy } else { 0.0 });
    w += dw * if from_center { 2.0 } else { 1.0 };
    h += dh * if from_center { 2.0 } else { 1.0 };

    if shift {
        // Aspect-lock: derive scale from the dominant axis.
        let r0 = cur_size[0] / cur_size[1].max(1e-6);
        if w.abs() > h.abs() {
            h = w / r0;
        } else {
            w = h * r0;
        }
    }

    w = w.max(0.0);
    h = h.max(0.0);

    if snap {
        w = snap_to(w, POS_SNAP);
        h = snap_to(h, POS_SNAP);
    }

    // Determine which node field to update: sprite leaf → Size; rect-shape
    // polygon → vertex set; otherwise no-op.
    let edit = match resolve_leaf_kind(tree, &path) {
        Some(LeafKind::Sprite) | Some(LeafKind::SpriteRenderer) => Some(NodeEdit::Size(Some([w, h]))),
        Some(LeafKind::PolygonRect) => Some(NodeEdit::PolygonRectSize { width: w, height: h }),
        _ => None,
    };
    if let Some(edit) = edit {
        app.pending_ops.push(TreeOp::Edit { path, edit });
    }
}

enum LeafKind {
    Sprite,
    SpriteRenderer,
    PolygonRect,
    PolygonFree,
    Container,
}

fn resolve_leaf_kind(tree: &manifest::Tree, path: &NodePath) -> Option<LeafKind> {
    let node = resolve_in_tree(tree, path)?;
    match &node.graphic {
        Some(Graphic::Sprite { .. }) => Some(LeafKind::Sprite),
        Some(Graphic::SpriteRenderer { .. }) => Some(LeafKind::SpriteRenderer),
        Some(Graphic::Polygon { vertices, triangles, .. }) => {
            if crate::inspector::is_rect_shape(vertices, triangles.as_deref()) {
                Some(LeafKind::PolygonRect)
            } else {
                Some(LeafKind::PolygonFree)
            }
        }
        None => Some(LeafKind::Container),
    }
}

fn resolve_in_tree<'a>(tree: &'a manifest::Tree, path: &NodePath) -> Option<&'a Node> {
    let mut node = &tree.root;
    for &i in &path.child_chain {
        node = node.children.get(i)?;
    }
    Some(node)
}

// =============================================================================
// Polygon vertex + node drag
// =============================================================================

fn handle_polygon_vertex_drag(
    app: &mut App,
    painter: &egui::Painter,
    resp: &egui::Response,
    xform: &ScreenTransform,
    view: &BuiltView,
    tree: &manifest::Tree,
) {
    let Some(path) = app.selection.primary().cloned() else { return; };
    let Some(info) = view.parts.iter().find(|p| p.path == path) else { return; };
    let Some(node) = resolve_in_tree(tree, &path) else { return; };
    let Some(Graphic::Polygon { vertices, triangles, .. }) = &node.graphic else { return; };
    // Rect-shape polygons get 9-way handles instead of per-vertex handles.
    if crate::inspector::is_rect_shape(vertices, triangles.as_deref()) { return; }

    let offset_world = info.pivot_world;

    let hover_pos = resp.hover_pos();
    let mut hovered_vert: Option<usize> = None;
    for (i, v) in vertices.iter().enumerate() {
        let world = apply_affine(*v, &info.affine, offset_world);
        let p = xform.world_to_screen(world);
        let is_active = app.dragging_polygon_vertex == Some(i);
        let color = if is_active {
            egui::Color32::from_rgb(0, 220, 255)
        } else {
            egui::Color32::from_rgb(0, 170, 255)
        };
        painter.circle_filled(p, VERTEX_HANDLE_R, color);
        painter.circle_stroke(p, VERTEX_HANDLE_R, egui::Stroke::new(1.0, egui::Color32::BLACK));
        if let Some(hp) = hover_pos {
            if (hp - p).length() < VERTEX_HANDLE_R + 4.0 {
                hovered_vert = Some(i);
            }
        }
    }

    if resp.drag_started() && hovered_vert.is_some() && app.dragging_size_handle.is_none() {
        app.dragging_polygon_vertex = hovered_vert;
    }
    if let Some(i) = app.dragging_polygon_vertex {
        let delta = resp.drag_delta();
        if delta == egui::Vec2::ZERO { return; }
        let snap = !resp.ctx.input(|i| i.modifiers.shift);
        let world_d = xform.screen_delta_to_world(delta);
        let local_d = invert_affine_delta(world_d, &info.affine);
        let cur = vertices[i];
        let mut new = [cur[0] + local_d[0], cur[1] + local_d[1]];
        if snap {
            new[0] = snap_to(new[0], POS_SNAP);
            new[1] = snap_to(new[1], POS_SNAP);
        }
        app.pending_ops.push(TreeOp::Edit {
            path: path.clone(),
            edit: NodeEdit::PolygonVertex { idx: i, value: new },
        });
    }
}

fn handle_node_drag(
    app: &mut App,
    resp: &egui::Response,
    xform: &ScreenTransform,
    cs: f32,
    guide_key: crate::app::ViewKey,
) {
    if !resp.dragged() { return; }
    if app.dragging_polygon_vertex.is_some() || app.dragging_size_handle.is_some() { return; }
    if app.marquee_origin.is_some() { return; }
    let delta = resp.drag_delta();
    if delta == egui::Vec2::ZERO { return; }

    let snap = !resp.ctx.input(|i| i.modifiers.shift);
    let world_d = xform.screen_delta_to_world(delta);

    // Move all selected node leaves by the same delta. Skip the synthesized
    // root (empty child_chain).
    let to_move: Vec<NodePath> = app.selection.iter().cloned().filter(|p| !p.child_chain.is_empty()).collect();
    if to_move.is_empty() { return; }
    for path in to_move {
        let cur_pos = match app.docs.get(path.doc).and_then(|d| path.resolve(&d.manifest)) {
            Some(n) => n.pos,
            None => continue,
        };
        let mut new_pos = [cur_pos[0] + world_d[0] / cs, cur_pos[1] + world_d[1] / cs];
        if snap {
            new_pos[0] = snap_to(new_pos[0], POS_SNAP);
            new_pos[1] = snap_to(new_pos[1], POS_SNAP);
        }
        // Guide snap operates in world units; pos is in canvas-pixel units
        // for CSA. Convert, snap, convert back. Skip when Shift is held
        // (Shift already opts out of snapping).
        if snap {
            let world_pos = [new_pos[0] * cs, new_pos[1] * cs];
            let snapped = snap_to_guides(app, guide_key, world_pos, xform);
            new_pos = [snapped[0] / cs, snapped[1] / cs];
        }
        app.pending_ops.push(TreeOp::Edit { path, edit: NodeEdit::Pos(new_pos) });
    }
}

// =============================================================================
// Small helpers
// =============================================================================

pub fn snap_to(v: f32, step: f32) -> f32 {
    (v / step).round() * step
}

/// Project a point onto the polygon's perimeter, returning the insertion
/// index + the projected vertex in local coordinates. The insertion index
/// is the position where the new vertex slots in to lie between
/// vertices[idx-1] and vertices[idx] (or at the end when the closest
/// edge wraps from `n-1` back to `0`, in which case `idx == n`).
/// Returns `None` for polygons with < 3 vertices.
pub fn project_onto_polygon_perimeter(point: [f32; 2], vertices: &[[f32; 2]]) -> Option<(usize, [f32; 2])> {
    if vertices.len() < 3 { return None; }
    let n = vertices.len();
    let mut best: Option<(f32, usize, [f32; 2])> = None;
    for i in 0..n {
        let a = vertices[i];
        let b = vertices[(i + 1) % n];
        let proj = project_onto_segment(point, a, b);
        let d2 = (proj[0] - point[0]).powi(2) + (proj[1] - point[1]).powi(2);
        if best.map_or(true, |(bd2, _, _)| d2 < bd2) {
            best = Some((d2, i + 1, proj));
        }
    }
    best.map(|(_, idx, p)| (idx, p))
}

fn project_onto_segment(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    let ab = [b[0] - a[0], b[1] - a[1]];
    let denom = ab[0] * ab[0] + ab[1] * ab[1];
    if denom < 1e-12 { return a; }
    let ap = [p[0] - a[0], p[1] - a[1]];
    let t = ((ap[0] * ab[0] + ap[1] * ab[1]) / denom).clamp(0.0, 1.0);
    [a[0] + ab[0] * t, a[1] + ab[1] * t]
}

fn apply_affine(v: [f32; 2], a: &fab::Affine, offset: [f32; 2]) -> [f32; 2] {
    let sx = v[0] * a.sx;
    let sy = v[1] * a.sy;
    let r = a.rot_deg_ccw.to_radians();
    let (sin_r, cos_r) = r.sin_cos();
    let rx = sx * cos_r - sy * sin_r;
    let ry = sx * sin_r + sy * cos_r;
    [rx + offset[0] + a.tx, ry + offset[1] + a.ty]
}

fn invert_affine_delta(world_d: [f32; 2], a: &fab::Affine) -> [f32; 2] {
    let r = (-a.rot_deg_ccw).to_radians();
    let (sin_r, cos_r) = r.sin_cos();
    let rx = world_d[0] * cos_r - world_d[1] * sin_r;
    let ry = world_d[0] * sin_r + world_d[1] * cos_r;
    let sx = if a.sx.abs() < 1e-6 { 1e-6 * a.sx.signum().max(1.0) } else { a.sx };
    let sy = if a.sy.abs() < 1e-6 { 1e-6 * a.sy.signum().max(1.0) } else { a.sy };
    [rx / sx, ry / sy]
}

fn point_in_part(screen: egui::Pos2, mesh: &CombinedMesh, range: &(usize, usize), xform: &ScreenTransform) -> bool {
    let world = xform.screen_to_world(screen);
    let (start, end) = *range;
    for tri in mesh.tris.chunks(3) {
        if let [a, b, c] = *tri {
            let ai = a as usize;
            if ai < start || ai >= end { continue; }
            let pa = mesh.verts[ai];
            let pb = mesh.verts[b as usize];
            let pc = mesh.verts[c as usize];
            if point_in_triangle(world, pa, pb, pc) {
                return true;
            }
        }
    }
    false
}

fn point_in_triangle(p: [f32; 2], a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> bool {
    let d1 = sign(p, a, b);
    let d2 = sign(p, b, c);
    let d3 = sign(p, c, a);
    let neg = (d1 < 0.0) || (d2 < 0.0) || (d3 < 0.0);
    let pos = (d1 > 0.0) || (d2 > 0.0) || (d3 > 0.0);
    !(neg && pos)
}

fn sign(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    (p[0] - b[0]) * (a[1] - b[1]) - (a[0] - b[0]) * (p[1] - b[1])
}

fn leaf_node_paths(tree: &manifest::Tree, doc_idx: usize, tree_idx: usize) -> Vec<NodePath> {
    let mut out = Vec::new();
    fn walk(node: &Node, path: &NodePath, out: &mut Vec<NodePath>) {
        if node.graphic.is_some() {
            out.push(path.clone());
        }
        for (i, c) in node.children.iter().enumerate() {
            walk(c, &path.child(i), out);
        }
    }
    let root_path = NodePath::tree_root(doc_idx, tree_idx);
    for (i, c) in tree.root.children.iter().enumerate() {
        walk(c, &root_path.child(i), &mut out);
    }
    out
}

fn fake_color_entry(name: &str) -> SpriteEntry {
    SpriteEntry {
        name: name.to_string(),
        rect: Rect { x: 0, y: 0, w: 1, h: 1 },
        pivot: Pivot { x: 0.5, y: 0.5 },
        alignment: SpriteAlignment::Center,
        border: Border::default(),
        geometry: Geometry { vertices: Vec::new(), triangles: Vec::new() },
    }
}

fn fake_placeholder_entry() -> SpriteEntry {
    SpriteEntry {
        name: String::new(),
        rect: Rect { x: 0, y: 0, w: 32, h: 32 },
        pivot: Pivot { x: 0.5, y: 0.5 },
        alignment: SpriteAlignment::Center,
        border: Border::default(),
        geometry: Geometry { vertices: Vec::new(), triangles: Vec::new() },
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snap_to_quarter() {
        // Midpoint between 0 and 0.25 is 0.125.
        assert_eq!(snap_to(0.12, 0.25), 0.0);
        assert_eq!(snap_to(0.13, 0.25), 0.25);
        assert_eq!(snap_to(-0.13, 0.25), -0.25);
        assert_eq!(snap_to(1.49, 0.25), 1.5);
        assert_eq!(snap_to(1.51, 0.25), 1.5);
        assert_eq!(snap_to(100.0, 0.25), 100.0);
    }

    #[test]
    fn invert_affine_delta_is_inverse_of_apply() {
        let a = fab::Affine { tx: 0.0, ty: 0.0, sx: 2.0, sy: 3.0, rot_deg_ccw: 30.0 };
        let local = [1.5, -0.8];
        let world = apply_affine(local, &a, [0.0, 0.0]);
        let back = invert_affine_delta(world, &a);
        assert!((back[0] - local[0]).abs() < 1e-4, "{back:?} vs {local:?}");
        assert!((back[1] - local[1]).abs() < 1e-4);
    }

    #[test]
    fn screen_transform_round_trip() {
        let xform = ScreenTransform {
            scale: 100.0,
            origin: egui::pos2(400.0, 300.0),
        };
        let world = [3.5, -2.1];
        let screen = xform.world_to_screen(world);
        let back = xform.screen_to_world(screen);
        assert!((back[0] - world[0]).abs() < 1e-4);
        assert!((back[1] - world[1]).abs() < 1e-4);
    }

    #[test]
    fn handle_positions_centered_unrotated() {
        let pos = handle_positions([0.0, 0.0], [4.0, 2.0], fab::Affine::default(), 1.0);
        // Index layout: NW, N, NE, W, MID, E, SW, S, SE, ROTATE.
        assert_eq!(pos[0], [-2.0, 1.0]); // NW
        assert_eq!(pos[2], [2.0, 1.0]);  // NE
        assert_eq!(pos[4], [0.0, 0.0]);  // MID
        assert_eq!(pos[6], [-2.0, -1.0]); // SW
        assert_eq!(pos[8], [2.0, -1.0]); // SE
        // Rotation handle sits above the top-center handle.
        assert!(pos[9][0].abs() < 1e-4 && pos[9][1] > 1.0, "rotate at {:?}", pos[9]);
    }

    #[test]
    fn nice_tick_step_picks_125_pattern() {
        // min_step in [1, 2) → step = 2.
        assert_eq!(nice_tick_step(1.5), 2.0);
        // [2, 5) → 5.
        assert_eq!(nice_tick_step(3.0), 5.0);
        // [5, 10) → 10.
        assert_eq!(nice_tick_step(7.0), 10.0);
        // Sub-1 inputs.
        assert!((nice_tick_step(0.07) - 0.1).abs() < 1e-6);
    }

    #[test]
    fn rotation_handle_drag_math_matches_atan2() {
        // Sanity: dragging the rotation handle to a cursor at (1, 0) world
        // (right of pivot) should set rot such that local up → right, i.e.
        // -90° (clockwise quarter turn).
        let dx: f32 = 1.0;
        let dy: f32 = 0.0;
        let rot = (-dx).atan2(dy).to_degrees();
        assert!((rot - -90.0).abs() < 1e-4, "{rot}");
        // Cursor at (0, 1) → no rotation.
        let rot = (-(0.0_f32)).atan2(1.0).to_degrees();
        assert!(rot.abs() < 1e-4);
        // Cursor at (-1, 0) → +90°.
        let rot = (-(-1.0_f32)).atan2(0.0).to_degrees();
        assert!((rot - 90.0).abs() < 1e-4);
    }

    #[test]
    fn polygon_projection_finds_nearest_edge() {
        // Square at (±1, ±1). Click at (0.95, 0.0) — closest is right edge.
        let verts = [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]];
        let (idx, projected) = project_onto_polygon_perimeter([0.95, 0.0], &verts).unwrap();
        // Right edge is from verts[1] → verts[2]; insertion idx = 2.
        assert_eq!(idx, 2);
        assert!((projected[0] - 1.0).abs() < 1e-4);
        assert!(projected[1].abs() < 1e-4);
    }

    #[test]
    fn polygon_projection_clamps_to_segment_ends() {
        // Click beyond the right edge's end → project to the corner, not
        // past it. Tie-broken by iteration order (top edge wins on equal
        // distance from (2, 1) — it goes verts[2] → verts[3]).
        let verts = [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]];
        let (_, projected) = project_onto_polygon_perimeter([2.0, 1.0], &verts).unwrap();
        assert_eq!(projected, [1.0, 1.0]);
    }

    #[test]
    fn polygon_projection_returns_none_for_degenerate() {
        let verts = [[0.0, 0.0], [1.0, 0.0]];
        assert!(project_onto_polygon_perimeter([0.5, 0.5], &verts).is_none());
    }

    #[test]
    fn boundary_edges_finds_only_perimeter() {
        // Single quad triangulated 0-1-2, 0-2-3 → perimeter is 0-1, 1-2, 2-3,
        // 3-0; the shared 0-2 diagonal cancels.
        let tris = [0u16, 1, 2, 0, 2, 3];
        let mut got = boundary_edges(&tris, (0, 4));
        got.sort();
        assert_eq!(got.len(), 4);
        let mut want: Vec<(usize, usize)> = vec![(0, 1), (1, 2), (2, 3), (0, 3)];
        want.sort();
        assert_eq!(got, want);
    }

    #[test]
    fn boundary_edges_respects_range() {
        // Two disconnected quads at (0..4) and (4..8). Only the first's
        // perimeter when range = (0, 4).
        let tris = [0u16, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7];
        let got = boundary_edges(&tris, (0, 4));
        assert_eq!(got.len(), 4, "{got:?}");
        for (a, b) in &got {
            assert!(*a < 4 && *b < 4);
        }
    }

    #[test]
    fn handle_edge_mask_corners_drag_both_axes() {
        assert_eq!(SizeHandle::Nw.edge_mask(), (true, true, false, false));
        assert_eq!(SizeHandle::Se.edge_mask(), (false, false, true, true));
        assert_eq!(SizeHandle::N.edge_mask(),  (false, true, false, false));
        assert_eq!(SizeHandle::Mid.edge_mask(),(false, false, false, false));
    }
}
