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
absent, behavior is unchanged. No FFI parameter, no `abi_version` bump.

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
      "parts": [
        // --- atlas-sprite part ---
        {
          "sprite":  "Armchair__Cushion__T", // tpsheet entry name on the same atlas
          "method":  "ID",                   // see "Slice methods" below; default "ID"
          "tx": 0, "ty": 0,                  // translation, world units (post-PPU)
          "sx": 1, "sy": 1,                  // scale; negative ⇒ geometric flip (replaces FX/FY/FXY)
          "rotDeg": 0,
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

```
v' = T(tx, ty) · R(rotDeg) · S(sx, sy) · v
```

Applied to the part's local-frame verts *after* the slice/polygon emitter
runs. `mirrorX` / `mirrorY` are slice-method UV semantics, not geometry —
for a plain flip set `sx` or `sy` to `-1`.

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
2. **Polygon part**: triangulate `vertices` via ear-clipping; UVs all sample
   the polygon sprite's atlas-rect center.
3. Apply `T · R · S` to verts.
4. Append verts/uvs/tris into the combined buffer with index offset.

Sprite field mapping follows the existing tpsheet → Sprite `.asset` field-map
table in [[CLAUDE.md]] with these deltas:

- `m_Rect` = AABB on atlas of the union of all parts' atlas rects.
- `_typelessdata` positions come from the combined verts (mapped through
  `pivot` and `m_Rect`'s pixel size), not from a single tpsheet entry.
- `_typelessdata` UVs are already atlas-normalized from the per-part emit
  step — no further uv transform.
- `m_IndexBuffer` is the combined triangle list, u16 LE.
- `textureRect == m_Rect` always. The on-disk preserve branch is being dropped
  crate-wide (see [[TODO.md]]); any sprite whose existing `.asset` has
  divergent `textureRect.{w,h}` fails loud out of `generate()`.

## Pipeline integration

Slots into the existing two-phase commit (see the C# ↔ Rust contract section
of [[CLAUDE.md]]) — landed. Phase-1 deltas:

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
6. Slice-grid family — `R1C3` / `R3C3` / `R3C3_NF` / `MX_R1C3` /
   `MX_R3C3` / `MY_R3C3` ✓. `MX_R1C4` / `MX_R3C2` / `MX_R3C4` /
   `MX_R3C6` / `MY_R2C2` / `MY_R2C3` / `MY_R3C1` / `MY_R3C2` /
   `MXY_R3C3` / `MXY_R3C3_NF` still pending.
7. Pipeline integration: fab.json discovery, combined emission, part
   exclusion, orphan-prune verification. ✓
