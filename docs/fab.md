# `.tps.fab.json` — Fabricated combined sprites

> **Related:** [[CLAUDE.md]], [[TODO.md]]

A `<atlas>.tps.fab.json` sidecar declares **fabricated** sprites that the crate
emits *instead of* their constituent parts. Saves atlas size by re-using small
part rects (incl. solid `Color_RRGGBBFF` tiles) to compose larger visuals as a
single Sprite whose mesh samples multiple atlas regions.

The crate consumes the manifest. Authoring is hand-written and/or
LLM-assisted today; a standalone exporter from the meow-tower prefab tree
(`BoxAuthor` / `SpriteMeshAuthor`) may follow.

## Location

Sibling of the `.tps`, named `<tps_path>.fab.json`. Discovery is implicit; if
absent, behavior is unchanged. No new `pipeline::generate` parameter.

```
21_Collections/Boxes/33_Vampire/
├── 33_Vampire.tps
├── 33_Vampire.tps.fab.json
├── 33_Vampire.tpsheet
├── 33_Vampire.png
└── 33_Vampire/                  ← output sprite dir
    ├── BX_33_Armchair.asset     ← fabricated
    └── ...                      ← non-part per-tpsheet sprites
```

## Schema (v1)

```jsonc
{
  "version": 1,
  "combined": [
    {
      "name":   "BX_33_Armchair",            // output sprite filename stem (no .asset)
      "pivot":  [0.5, 0.5],                  // optional, default [0.5, 0.5]
      "border": [0, 0, 0, 0],                // optional [L, B, R, T] in atlas pixels, default zeros
      "canvasScale": 1.0,                    // optional. CanvasSpriteAuthor._scaleFactor (post-mul).
                                             //   Default 1 (SpriteRenderer / Box prefab path).
                                             //   Canvas hierarchies set 0.01 to undo UIIcon's 100×.
      "rootAnchored": [0, 0],                // optional. CanvasSpriteAuthor root's
                                             //   RectTransform.anchoredPosition. Only matters
                                             //   for reproducing the FMA-fused residual Unity's
                                             //   Mesh.CombineMeshes leaves on each per-instance
                                             //   matrix when the root isn't at origin. Default
                                             //   [0, 0] collapses the FMA chain to the plain
                                             //   `offset × canvas_scale` form.
      "parts": [
        // --- atlas-sprite part ---
        {
          "sprite":  "Armchair__Cushion__T", // tpsheet entry name on the same atlas
          "method":  "ID",                   // see "Slice methods" below; default "ID"
          "tx": 0, "ty": 0,                  // translation, world units (post-PPU, pre-canvasScale)
          "sx": 1, "sy": 1,                  // scale; negative ⇒ geometric flip (replaces FX/FY/FXY)
          "rotDeg": 0,
          "uiScale": 1.0,                    // optional. UIIcon._scaleFactor (per-part pre-mul).
                                             //   Default 1. Canvas hierarchies set 100.
          "offset": [0, 0],                  // optional. RectTransform.anchoredPosition, applied
                                             //   between uiScale and canvasScale. Default [0, 0].
          "width":  null, "height": null,    // target rect, world units. Absent ⇒ native-scale
                                             //   (UIIconMeshGen path). Present ⇒ slice-fitted
                                             //   (UISliceMeshGen path). Rejected on ID.
          "partPivot": [0.5, 0.5],           // target-rect pivot, normalized 0..1. Default [0.5,0.5].
                                             //   Box prefabs leave it default; UI hierarchies (e.g.
                                             //   CanvasSpriteAuthor) carry mixed pivots like (0,0.5).
          "borderMult": 1.0                  // optional, default 1. Rejected on ID / MX / MY / MXY /
                                             //   TX / TY (none of them consume a sprite border).
        },
        // --- polygon part (custom verts over a solid sprite) ---
        {
          "polygonSprite": "Color_3F314EFF", // tpsheet entry, axis-aligned quad
          "vertices": [[-0.41,-0.385],[0.41,-0.385],[0.41,0.385],[-0.41,0.385]],
          "triangles": [0, 2, 3, 3, 1, 0],   // optional. Index list override; default ear-clip.
                                             //   Required for UISolid quads (BL,BR,TL,TR order is
                                             //   self-crossing for the ear-clipper).
          "tx": 0, "ty": 0, "sx": 1, "sy": 1, "rotDeg": 0
        }
      ]
    }
  ]
}
```

### Rules enforced at parse time

- `name` is a non-empty bare filename (no `/`, no `\`); duplicates and
  collisions with non-pruned per-tpsheet sprites → error.
- `parts` is non-empty; **order is significant** — it's the render order
  (first = back, last = front), matching the DFS hierarchy walk that
  `SpriteMeshBuilder.ExtractMeshDataByHierarchicalOrder` (meow-tower) does
  over a prefab's `SpriteRenderer` children. The crate preserves the JSON
  array order verbatim: no sort, no dedup, repeats are emitted as repeats.
- Every `sprite` / `polygonSprite` resolves to an entry in the same
  `.tpsheet`; unresolved names listed in one error.
- `method ∈ {FX, FY, FXY}` is rejected; use negative `sx`/`sy` instead.
- **Two strategies based on `width`/`height` presence:**
  - **Native scale** (no `width`/`height`): the source sprite is drawn at
    its native size (pivot-relative world units = pixel-rect / PPU). Affine
    applies per-vertex. Matches `UIIconMeshGen.cs` (SpriteRenderer / UIIcon
    use case). Allowed methods: `ID`, `MX`, `MY`, `MXY`.
  - **Size-fitted** (`width`/`height` declared): source verts stretch to the
    declared target rect via the slice/tile math. Matches `UISliceMeshGen.cs`
    (UISlice use case). Allowed methods: all except `ID`. The
    constraints asserted by each method in
    `Core/Runtime/UI/MeshGeneration/UISliceMeshGen.cs` are preserved —
    violations are parse errors naming the offending part.
  - Slice grids (`R*`, `MX_*`, `MY_*`, `MXY_*`) and tilers (`TX*`, `TY*`)
    only make sense size-fitted; omitting `width`/`height` on them is a
    parse error.
- Polygon parts must not declare atlas-sprite-only fields
  (`method`, `width`, `height`, `borderMult`, `partPivot`). Mixing the
  two shapes signals a typo; the parser rejects rather than silently
  dropping fields.
- Unknown JSON fields anywhere in the manifest are rejected
  (`deny_unknown_fields`). Typos and stale schema fields surface as
  parse errors instead of being silently dropped.
- Options that don't apply to the resolved method are rejected:
  `width`/`height` on `ID`; `borderMult` on methods that don't consume
  the sprite's border (`ID`, `MX`, `MY`, `MXY`, `TX`, `TY`). Surface
  area kept tight so authoring tools (LLMs included) get clear
  feedback instead of silent data loss.

### Per-part transform

The full transform composed onto each part's local-frame vert is the f32
op sequence Unity uses, in this exact order:

```
v_world = ((R(rotDeg) · S(sx, sy) · v) × uiScale) × canvasScale + m13
        + (tx, ty)

where m13 = fma(canvasScale, rootAnchored + offset, −canvasScale × rootAnchored)
```

`m13` is precomputed once per part with FMA-fused rounding (the f64-
promotion form in `combine::compute_m13_axis`) to reproduce the residual
Unity's `Mesh.CombineMeshes` carries on each per-`CombineInstance`
matrix. The per-vert chain itself uses regular two-step f32 (no FMA at
that step) — matches Unity's `Matrix4x4.MultiplyPoint`.

For `rootAnchored = (0, 0)` the FMA chain collapses to
`canvasScale × offset` exactly, so SpriteRenderer / Box prefab callers
plus any Canvas hierarchy whose root sits at origin are bit-stable.
For non-origin roots (e.g. Silloutte3 at `(141.8, 370.875)`) the FMA
residual is what makes the byte-exact match work.

Defaults `uiScale = canvasScale = 1`, `offset = (0, 0)`, `rootAnchored = (0, 0)`
collapse the full chain to the affine-only path `v' = T · R · S · v`.

For a plain geometric flip set `sx` or `sy` to `-1` — `FX`/`FY`/`FXY`
methods are rejected at parse time in favor of negative scale.

### Polygon UV sampling

All polygon vertices sample the **center pixel** of the `polygonSprite`'s atlas
rect (matches `SolidUVCache.Get`). Only mode in v1. Bilinear-over-rect mode is
listed in [[TODO.md]] as deferred.

### Part exclusion

Any sprite referenced by any combined entry is excluded from per-tpsheet
emission. Pre-existing `.asset`s for excluded parts are pruned via the
existing orphan path. No allowlist in v1.

## Combined Sprite emission

Geometry build (per combined entry, parts walked in declared order — see
the order rule above; each part's verts/tris are appended to the combined
buffers, so the resulting `m_IndexBuffer` runs back-to-front):

1. **Atlas-sprite part**: source verts/UVs/tris from the tpsheet entry; the
   `method` produces local-frame output (slice / tile / mirror).
2. **Polygon part**: triangulate `vertices` via ear-clipping (or use the
   `triangles` override); UVs all sample the polygon sprite's atlas-rect center.
3. Apply `combine::apply_transform` to each vert — the full canvas chain
   from "Per-part transform" above (per-part affine → uiScale → canvasScale + m13),
   not just `T · R · S`.
4. Append verts/uvs/tris into the combined buffer with index offset.

Sprite field mapping follows the existing tpsheet → Sprite `.asset` field-map
table in [[CLAUDE.md]] with these deltas:

- `m_Rect` = `(0, 0, w*ppu, h*ppu)` where `(w, h)` is the combined mesh's
  vertex-AABB size in world units. Matches `SpriteFactory.CreateFromMesh`'s
  derivation (ported in `combine::calc_rect_and_pivot`).
- `m_Pivot` = `(-AABB.min.x / w, -AABB.min.y / h)` — the mesh origin's
  position within the AABB, normalized.
- `_typelessdata` positions come from the combined verts (already in
  pivot-relative world units post-affine), not from a single tpsheet entry.
- `_typelessdata` UVs are already atlas-normalized from the per-part emit
  step — no further uv transform.
- `m_IndexBuffer` is the combined triangle list, u16 LE.
- `atlasRectOffset = (-1, -1)` (Unity's sentinel for non-SpriteAtlas sprites,
  matches both `Sprite.Create` and TexturePacker outputs) and `m_Rect.{w, h}`
  are f32 (sub-pixel-able) — the fabricated sprite branch in
  `emit::SpriteAsset` (`source: Fabricated`).
- `textureRect == m_Rect` always. The on-disk preserve branch was dropped
  crate-wide (see [[TODO.md]]); any sprite whose existing `.asset` has
  divergent `textureRect.{w,h}` fails loud out of `generate()`.

## Pipeline integration

Slots into the existing two-phase commit (see the "Public Rust API" /
"Invariants" sections of [[CLAUDE.md]]) — landed. Phase-1 deltas:

- `pipeline::generate` looks for `<tps_path>.fab.json` next to the `.tps`.
  Present → `fab::parse` (`FabError` propagates as `pipeline::Error::Fab`);
  absent → behavior unchanged.
- For each `combined` entry: `combine::build_combined` → `calc_rect_and_pivot`
  → `render_data::build_fabricated` → `emit::SpriteAsset { source: Fabricated }`
  → bytes appended to writes. `CombineError` propagates as
  `pipeline::Error::Combine`.
- Sprites referenced by any combined entry are skipped in the per-tpsheet
  emission loop. Their on-disk `.asset` predecessors (if any) are pruned as
  orphans in phase 2 — no special-case code needed.

## Phasing

Each phase ends with cargo test green and a unit fixture covering the new
path.

1. `src/fab.rs` + serde model + schema validation. ✓
2. Triangulator + polygon-only combine path. ✓
3. Affine + `ID` method for atlas-sprite parts; multi-part combine; pivot;
   combined `m_Rect`. ✓
4. `MX` / `MY` / `MXY` mirror duplication (dual-strategy: native-scale via
   `UIIconMeshGen` or slice-fitted via `UISliceMeshGen` depending on `width`/
   `height` presence). ✓
5. `TX` / `TY` / `TX_MC3` tiling. ✓
6. Slice-grid family — all 17 methods ✓
   (`R1C3` / `R3C3` / `R3C3_NF` / `MX_R1C3` / `MX_R1C4` / `MX_R3C2` /
   `MX_R3C3` / `MX_R3C4` / `MX_R3C6` / `MY_R3C1` / `MY_R2C2` /
   `MY_R2C3` / `MY_R3C2` / `MY_R3C3` / `MXY_R3C3` / `MXY_R3C3_NF`).
7. Pipeline integration: fab.json discovery, combined emission, part
   exclusion, orphan-prune verification. ✓
