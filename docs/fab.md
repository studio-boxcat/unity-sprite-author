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
          "mirrorX": false, "mirrorY": false,// UV-mirror duplication; honored only by slice methods
          "width":  null, "height": null,    // required for non-ID methods (target rect, world units)
          "borderMult": 1.0                  // optional, default 1; ignored when method has no border
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
- For non-ID methods, `width` and `height` are required (replace
  `RectTransform.rect.size` from the C# port).
- The source-rect constraints asserted by each slice method in
  `Core/Runtime/UI/MeshGeneration/UISliceMeshGen.cs` (meow-tower) are
  preserved — violations are parse errors naming the offending part.
  Method list and constraints live in that file; this crate is a faithful port.
- Polygon parts must not declare atlas-sprite-only fields
  (`method`, `width`, `height`, `borderMult`, `mirrorX`, `mirrorY`).
  Mixing the two shapes signals a typo; the parser rejects rather than
  silently dropping fields.

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
of [[CLAUDE.md]]). Phase-1 deltas only:

- Load `<tps_path>.fab.json` if present and validate.
- Emit one `(path, bytes)` pair per combined entry, alongside the per-tpsheet
  pairs.
- Remove part sprites from the per-tpsheet emission set; their on-disk
  predecessors (if any) get orphan-pruned in phase 2.

## Phasing

Each phase ends with cargo test green and a golden fixture covering the new
path.

1. `src/fab.rs` + serde model + schema validation. No pipeline change.
2. Triangulator + polygon-only combine path.
3. Affine + `ID` method for atlas-sprite parts; multi-part combine; pivot;
   combined `m_Rect`.
4. `MX` / `MY` / `MXY` mirror duplication.
5. `TX` / `TY` / `TX_MC3` tiling.
6. Slice-grid family — `R1C3`, `R3C3*`, then `MX_*`, `MY_*`, `MXY_*`.
7. Part exclusion in pipeline; end-to-end orphan-prune verification.

Phases 4–6 share dispatch + grid-index tables and may collapse if cohesive.
