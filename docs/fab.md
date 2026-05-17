# `.tps.fab.json` — Unified manifest for fabricated sprites + SMA meshes

> **Related:** [[CLAUDE.md]], [[TODO.md]], [[sma-migration.md]]

The `.tps.fab.json` sidecar declares **fabricated** sprites (combined from
multiple atlas parts) and **SMA meshes** (multi-mesh `.asset` outputs)
authored as a unified prefab-style tree. The crate emits the declared
outputs *instead of* their constituent parts; parts referenced inside any
tree are excluded from per-tpsheet emission.

Authoring is hand-written and/or LLM-assisted today. The historical
`CanvasSpriteAuthor` and `SpriteMeshAuthor` Unity prefabs that seeded the
corpus have been retired (meow-tower c23474b2ab40); new trees are
declared directly in the sidecar JSON.

## Location

Sibling of the `.tps`, named `<tps_path>.fab.json`. Discovery is implicit;
absent → behavior unchanged. No `pipeline::generate` parameter.

```
21_Collections/Boxes/33_Vampire/
├── 33_Vampire.tps
├── 33_Vampire.tps.fab.json
├── 33_Vampire.tpsheet
├── 33_Vampire.png
└── 33_Vampire/                  ← output sprite dir
    ├── BX_33_Armchair.asset     ← fabricated combined
    └── ...                      ← non-part per-tpsheet sprites
```

## Schema

```jsonc
{
  "version": 1,
  "combined": [
    {
      "name": "Silloutte1",              // output sprite filename stem (no .asset)
                                         //  — also the combined sprite's identity in errors.
      "mode": "ui",                      // "ui" (CSA) | "sma-canvas" | "sma-renderer"
                                         //  — drives canvas_scale_implicit (see below).
      // SMA-only fields (required when mode starts with "sma-"):
      "fileId":     -8704840387945618417,
      "outputPath": "out.asset",
      "keepVertices": true,
      "keepIndices":  true,

      "children": [
        // --- container (pure transform, no graphic) ---
        { "name": "Body", "pos": [0, 8.768],
          "children": [
            // --- sprite leaf (atlas-backed) ---
            { "type": "sprite", "sprite": "Mansion_Silloutte__1__B", "method": "MX",
              "pos": [0, -294.75], "scale": [-1, 1] }
          ] },
        // --- polygon leaf (solid-color quad) ---
        { "type": "polygon", "color": "32264DBD",      // → tpsheet entry `Color_32264DBD`
          "pos": [0, -22.25],
          "vertices": [[-106.25, -272.5], [106.25, -272.5], [-106.25, 272.5], [106.25, 272.5]],
          "triangles": [0, 2, 3, 3, 1, 0] }
      ]
    }
  ]
}
```

There is **one scale source per node**: `node.scale`. Sign carries flip,
magnitude carries the composed scale at that node. The historical CSA
chain (`tree.scale × leaf.uiScale`, typically `0.01 × 100 = 1.0`) is
collapsed — a leaf that previously had `uiScale: 53.125` under a CSA
tree (`tree.scale: 0.01`) now declares `scale: 0.53125`.

The canvas factor itself is **mode-implicit**:

| `mode` | `canvas_scale_implicit` |
| --- | --- |
| `ui` | `0.01` |
| `sma-canvas`, `sma-renderer` | `1.0` |

It is applied at exactly one seam — the bridge pre-multiplies each
leaf's anchored `pos` so `Part.offset` lands in world units. The
runtime per-vert chain therefore drops `× canvas_scale` entirely.

### Node fields (any depth)

| Field | Meaning |
| --- | --- |
| `name` | Diagnostic only. Optional. |
| `pos` | `RectTransform.anchoredPosition`, canvas-pixel units. Default `(0, 0)`. |
| `size` | `RectTransform.sizeDelta` for sprite/polygon, or `SpriteRenderer.size` for SMA tiled. Default: inherit sprite's natural rect (sprite leaves with a strictly-size-fitted method — `R*`, `MX_R*`, `MY_R*`, `MXY_R*`, `TX`, `TY`, `TX_MC3`); irrelevant for `ID`, `MX`/`MY`/`MXY` without size (native-scale path), polygons (uses its own `vertices`), and `simple`-mode SpriteRenderers. Omit when matching the natural value. Note: for `MX`/`MY`/`MXY` the presence vs. absence of `size` switches between slice-fitted and native-scale behavior; don't add a default value blindly. |
| `pivot` | `RectTransform.pivot` (0..1). Default: inherit sprite's tps `pivotPoint` (sprite leaves) / `(0.5, 0.5)` (containers, polygon). Omit when matching the natural value. |
| `scale` | Per-node scale. Either `1.5` (uniform) or `[-1, 1]` (per-axis). Sign = flip, magnitude = composed scale. Default `[1, 1]`. |
| `rotDegCCW` | Counter-clockwise rotation around Z, degrees (Unity UI canvas convention, Y-up). Default `0`. |
| `type` | Graphic discriminator: `"sprite"` \| `"polygon"` \| `"spriteRenderer"`. Absent → pure container. |

### Sprite / polygon / SpriteRenderer leaf fields

- **`type: "sprite"`** (atlas-backed leaf): `sprite` = tpsheet entry name;
  `method ∈ {ID, MX, MY, MXY, R*, MX_R*, MY_R*, MXY_R*, TX, TY, TX_MC3}`
  (geometric flips use negative `scale` instead of methods);
  `borderMult: f32`; `flipX`/`flipY: bool` (alternative to negative `scale`).
- **`type: "polygon"`** (solid-color quad): `color: "RRGGBB[AA]"` (hex, 6 or 8 digits)
  resolves to `Color_<UPPER>` in the tpsheet; `vertices: [[f32; 2]]` (canvas
  pixels); optional `triangles: [u16]` (default ear-clip; quads
  pass `[0,2,3,3,1,0]` explicitly).
- **`type: "spriteRenderer"`** (SMA): `sprite` + `drawMode: "simple" | "tiled"`.
  The tiled draw rect lives in the node-level `size` field (shared with the
  sprite/polygon RectTransform-size slot — same field, different role per leaf type).

### Rules enforced at parse time

- `name` is non-empty + bare (no `/`, `\`); duplicates across trees → error.
- `parts` (sprite-tree children) order is significant — render order
  (first = back, last = front), matching Unity's DFS hierarchy walk.
- Every `sprite` / `color` resolves to an entry on the same `.tpsheet`;
  unresolved names listed in one error.
- `flipX`/`flipY` and negative `scale` are mutually exclusive expressions;
  the bridge folds whichever is present into `Part::AtlasSprite.affine.sx/sy`.
- SMA-only fields (`fileId`, `outputPath`, `keepVertices`, `keepIndices`)
  must be present when `mode` starts with `sma-` and rejected otherwise.
- Slice grids (`R*`, `MX_*`, `MY_*`, `MXY_*`) and tilers (`TX*`, `TY*`)
  require a non-zero target rect — omit `size` to inherit the sprite's
  natural rect, or pass an explicit `[w, h]` to override. Explicit
  `[0, 0]` is a parse error (slice math would divide by zero).
- `serde(deny_unknown_fields)` — typos and stale fields surface as parse
  errors instead of silent drops. A pre-c23474b2 tree-level `scale` or
  leaf `uiScale` therefore lands as a hard parse failure naming the
  offending field; no silent migration.

### Per-part transform

The full transform composed onto each part's local-frame vert in f32:

```
v_world = R(rotDegCCW) · S(sx, sy) · v + offset + (tx, ty)
```

`offset` here is the bridge-precomputed world-unit translation
(`leaf.pos × canvas_scale_implicit`, stored on `fab::Part`). The
per-vert chain is therefore one multiply (the per-part affine scale),
one optional rotate, and one add. For SpriteRenderer / Box prefabs
`offset = (0, 0)` and the chain collapses to `T · R · S · v`.

For a geometric flip set `sx` or `sy` to `-1`. Method-style flips
(`FX`/`FY`/`FXY`) aren't supported.

Composition through container hierarchies follows the standard
multiplicative walk: a leaf's effective `(sx, sy)` is its own `scale`
multiplied by every ancestor's `scale`. Because magnitude is
front-loaded into the leaf, container nodes typically declare just
`[±1, ±1]` for flips and identity scaling.

`rotDegCCW` cascades the same way and *also* rotates child anchored
positions, not just the per-vert R. A child at local `pos (-28.4, -16)`
under a parent rotated `-45°` in Z lands at world `(-31.4, +8.8)`, not
at the unrotated `(-28.4, -16)`. Pre-walker-rotation fix, container
rotations only affected the leaf's own per-vert R and broke composites
like the Spider Ghost sticker (4 child verts shifted by `(+2.97,
-24.76)` world units when the `-45°` Body parent's rot wasn't applied
to its child position).

### Polygon UV sampling

All polygon vertices sample the **center pixel** of the polygon's atlas
rect (matches `SolidUVCache.Get`). Bilinear-over-rect sampling is
listed in [[TODO.md]] as deferred.

### Part exclusion

Any sprite referenced by any tree is excluded from per-tpsheet emission;
pre-existing `.asset`s for excluded parts are preserved on disk (external
prefabs may reference the part GUID directly), but the part is not
emitted as a standalone sprite.

A combined tree's `name` may equal one of its own part sprite names —
the combined emit owns the `.asset` for that name and inherits the
preserved GUID via the staged `.asset.meta` (see `PB_PiggyBank_Open`,
which composites itself with sibling sprites). The per-tpsheet loop
skips name pre-registration in this case so the combined loop can
register and emit without tripping `DuplicateSpriteName`.

## Combined Sprite emission

Geometry build (per CSA tree, leaves walked in DFS order — see the order
rule above; each leaf's verts/tris are appended to the combined buffers,
so the resulting `m_IndexBuffer` runs back-to-front):

1. **Sprite leaf**: source verts/UVs/tris from the tpsheet entry; `method`
   produces local-frame output (slice / tile / mirror).
2. **Polygon leaf**: ear-clip `vertices` (or use `triangles` override);
   UVs all sample the polygon sprite's atlas-rect center.
3. Apply `combine::apply_transform` per vert — per-part affine then add the
   bridge-precomputed world-unit `offset`.
4. Append verts/uvs/tris into the combined buffer with index offset.

Sprite field mapping follows the tpsheet → Sprite `.asset` field-map table
in [[CLAUDE.md]] with these deltas:

- `m_Rect = (0, 0, w*ppu, h*ppu)` where `(w, h)` is the combined mesh's
  vertex-AABB size in world units (`combine::calc_rect_and_pivot`).
- `m_Pivot = (-AABB.min.x / w, -AABB.min.y / h)` — mesh origin within the
  AABB, normalized.
- `_typelessdata` positions come from combined verts (pivot-relative world
  units post-affine); UVs are atlas-normalized at the per-part emit step.
- `m_IndexBuffer` is the combined triangle list (u16 LE).
- `atlasRectOffset = (-1, -1)`; `m_Rect.{w, h}` are f32 (sub-pixel-able)
  via `emit::SpriteAsset { source: Fabricated }`.
- `textureRect == m_Rect` always. Existing `.asset`s with divergent
  `textureRect.{w,h}` (legacy Tight + `spriteMode:Multiple`) are
  overwritten and warned about non-fatally.

## Pipeline integration

Slots into the two-phase commit (see "Public Rust API" / "Invariants" in
[[CLAUDE.md]]):

- `pipeline::generate` looks for `<tps_path>.fab.json` next to the `.tps`.
  Present → `manifest::parse` (`ManifestError` propagates as
  `pipeline::Error::Manifest`); absent → behavior unchanged.
- For each CSA tree: `manifest::to_fab_combined` → `combine::build_combined`
  → `calc_rect_and_pivot` → `render_data::build_fabricated` →
  `emit::SpriteAsset { source: Fabricated }` → bytes appended to writes.
  `BridgeError` / `CombineError` propagate as
  `pipeline::Error::Bridge` / `Error::Combine`.
- For each SMA tree: `manifest::to_mesh_combined` → `mesh_emit::build_mesh`
  → bytes appended; multiple SMA trees pointing at the same `outputPath`
  are grouped into one multi-mesh `.asset`.
- Sprites referenced by any tree are skipped in per-tpsheet emission;
  pre-existing `.asset`s for parts are preserved (the orphan-prune path
  still applies to sprites no tree refers to).
