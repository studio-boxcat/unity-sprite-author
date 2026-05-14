# `.tps.fab.json` — Unified manifest for fabricated sprites + SMA meshes

> **Related:** [[CLAUDE.md]], [[TODO.md]], [[sma-migration.md]]

The `.tps.fab.json` sidecar declares **fabricated** sprites (combined from
multiple atlas parts) and **SMA meshes** (multi-mesh `.asset` outputs)
authored as a unified prefab-style tree. The crate emits the declared
outputs *instead of* their constituent parts; parts referenced inside any
tree are excluded from per-tpsheet emission.

The schema landed as part of the CSA / SMA Author migration: it replaces
the old flat `.tps.fab.json` (v1) and `.tps.mesh.json` (legacy) parsers,
which were retired alongside this rewrite. The unified tree mirrors the
authoring shape in Unity (`CanvasSpriteAuthor` / `SpriteMeshAuthor`
prefabs) so the migration tool emits canonical JSON directly.

Authoring is hand-written and/or LLM-assisted today; the dump-and-convert
path is in `examples/csa_to_fab.rs`.

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
  "trees": [
    {
      "name": "Silloutte1",              // output sprite filename stem (no .asset)
                                         //  — also the tree's identity in errors.
      "mode": "ui",                      // "ui" (CSA), "sma-canvas", or "sma-renderer"
      "scale": 0.01,                     // optional. Root's `scaleFactor` (CSA 0.01) or
                                         //   world-unit (SMA 1.0). Default 1.
      // SMA-only fields (required when mode starts with "sma-"):
      "fileId":     -8704840387945618417,
      "outputPath": "out.asset",
      "keepVertices": true,
      "keepIndices":  true,

      "children": [
        // --- container (pure transform, no graphic) ---
        { "name": "Body", "pos": [0, 8.768], "pivot": [0.5, 0.5],
          "children": [
            // --- sprite leaf (UIIcon / SpriteRenderer) ---
            { "type": "sprite", "sprite": "Mansion_Silloutte__1__B", "method": "MX",
              "pos": [0, -294.75], "scale": [-1, 1] }
          ] },
        // --- polygon leaf (UISolid background) ---
        { "type": "polygon", "color": "32264DBD",      // → tpsheet entry `Color_32264DBD`
          "pos": [0, -22.25],
          "vertices": [[-106.25, -272.5], [106.25, -272.5], [-106.25, 272.5], [106.25, 272.5]],
          "triangles": [0, 2, 3, 3, 1, 0] }
      ]
    }
  ]
}
```

### Node fields (any depth)

| Field | Meaning |
| --- | --- |
| `name` | Diagnostic only. Optional. |
| `pos` | `RectTransform.anchoredPosition`, canvas-pixel units. Default `(0, 0)`. |
| `sizeDelta` | `RectTransform.sizeDelta`. Consumed by size-fitted methods + SMA tiled mode. |
| `pivot` | `RectTransform.pivot` (0..1). Default `(0.5, 0.5)`. |
| `scale` | Per-node scale. Either `1.5` (uniform) or `[-1, 1]` (per-axis, for flips). Default `[1, 1]`. |
| `rotDeg` | Rotation in degrees. Default `0`. |
| `type` | Graphic discriminator: `"sprite"` \| `"polygon"` \| `"spriteRenderer"`. Absent → pure container. |

### Sprite / polygon / SpriteRenderer leaf fields

- **`type: "sprite"`** (UIIcon under CSA): `sprite` = tpsheet entry name;
  `method ∈ {ID, MX, MY, MXY, R*, MX_R*, MY_R*, MXY_R*, TX, TY, TX_MC3}`
  (geometric flips use negative `scale` instead of methods);
  `uiScale: f32` (default `100`, CSA's UIIcon `_scaleFactor`);
  `borderMult: f32`; `flipX`/`flipY: bool` (alternative to negative `scale`).
- **`type: "polygon"`** (UISolid): `color: "RRGGBB[AA]"` (hex, 6 or 8 digits)
  resolves to `Color_<UPPER>` in the tpsheet; `vertices: [[f32; 2]]` (canvas
  pixels); optional `triangles: [u16]` (default ear-clip; UISolid quads
  pass `[0,2,3,3,1,0]` explicitly).
- **`type: "spriteRenderer"`** (SMA): `sprite` + `drawMode: "simple" | "tiled"`;
  `size: [f32; 2]` (required when `tiled`, rejected when `simple`).

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
  require `sizeDelta`; omitting it is a parse error.
- `serde(deny_unknown_fields)` — typos and stale fields surface as parse
  errors instead of silent drops.

### Per-part transform

The full transform composed onto each part's local-frame vert is the f32
op sequence Unity uses, in this exact order:

```
v_world = ((R(rotDeg) · S(sx, sy) · v) × uiScale) × canvasScale
        + (offset × canvasScale)
        + (tx, ty)
```

`offset × canvasScale` is precomputed once per part (`offset_scaled` in
`combine::SliceCtx`) so the per-vert chain is a single two-step f32 op
(`v × canvasScale + offset_scaled`). Matches Unity's matrix-style emit.

Defaults `uiScale = canvasScale = 1`, `offset = (0, 0)` collapse the full
chain to the affine-only path `v' = T · R · S · v`. SpriteRenderer / Box
prefabs and any Canvas hierarchy whose root sits at origin run the same
op order without divergence.

For a geometric flip set `sx` or `sy` to `-1`. Method-style flips
(`FX`/`FY`/`FXY`) aren't supported.

### Polygon UV sampling

All polygon vertices sample the **center pixel** of the polygon's atlas
rect (matches `SolidUVCache.Get`). Bilinear-over-rect sampling is
listed in [[TODO.md]] as deferred.

### Part exclusion

Any sprite referenced by any tree is excluded from per-tpsheet emission;
pre-existing `.asset`s for excluded parts are preserved on disk (external
prefabs may reference the part GUID directly), but the part is not
emitted as a standalone sprite.

## Combined Sprite emission

Geometry build (per CSA tree, leaves walked in DFS order — see the order
rule above; each leaf's verts/tris are appended to the combined buffers,
so the resulting `m_IndexBuffer` runs back-to-front):

1. **Sprite leaf**: source verts/UVs/tris from the tpsheet entry; `method`
   produces local-frame output (slice / tile / mirror).
2. **Polygon leaf**: ear-clip `vertices` (or use `triangles` override);
   UVs all sample the polygon sprite's atlas-rect center.
3. Apply `combine::apply_transform` per vert — the full canvas chain (per-part
   affine → uiScale → canvasScale + offset_scaled).
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
