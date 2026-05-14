# Silloutte fab fixtures

Two `CanvasSpriteAuthor`-published sprites originally captured from
`meow-tower/Assets/21_Collections/PremiumCatEvents/33_Vampire/Elements/Ornaments/`
(the source dir was rewritten upstream; the `.prefab` here is now the
self-contained source-of-truth). The `.asset` is what Unity emits when
`CanvasSpriteAuthor.Publish()` runs.

| File | Source |
| --- | --- |
| `PremiumCat_Vampire_Popup.{tps, tpsheet, png.meta}` | Shared atlas + sidecar metadata. `.tpsheet` regenerated via TexturePacker CLI before capture. |
| `PremiumCat_Vampire_Popup.tps.fab.json` | v3 unified manifest declaring both trees. |
| `Silloutte{1,2}.prefab` | UI hierarchy under a `CanvasSpriteAuthor` root — `UIIcon` / `UISolid` children with per-RectTransform pivot and anchored position. Reference only. |
| `Silloutte{1,2}.asset(.meta)` | Sprite asset committed from meow-tower. Byte-exact target. |

## Status

Both sprites byte-exact under default `cargo test`
(`tests/golden_fab_silloutte.rs`).

A third fixture, `Silloutte3`, was dropped along with `rootAnchored`
(the field that captured Unity's `Mesh.CombineMeshes` FMA residual for
non-origin CSA roots). With `rootAnchored` gone, any CSA tree whose root
sits at non-`(0, 0)` drifts ~1 ULP per vertex from Unity's emit. Future
fixtures should pin their root at origin in the prefab.

## Manifest authoring recipe (v3)

- One `trees[]` entry per published sprite. `mode: "ui"` for CSA.
- Set `scale: 0.01` to mirror `CanvasSpriteAuthor._scaleFactor`. Default
  is `1.0` (SpriteRenderer / Box prefab path).
- Each `children[]` node carries `pos`, `sizeDelta`, `pivot`, optional
  `scale` (per-axis flip via `[-1, 1]`), and a `type` discriminator.
- UIIcon → `{ "type": "sprite", "sprite": "...", "method": "ID|MX|MY|MXY|..." }`.
  Geometric flips: negative `scale` (per-axis), not `FX`/`FY`/`FXY` methods.
- UISolid → `{ "type": "polygon", "color": "RRGGBB", "vertices": [...],
  "triangles": [0, 2, 3, 3, 1, 0] }`. Color maps to the tpsheet entry
  `Color_<UPPER>`.
- Resolve sprite GUIDs in the prefab against the atlas's per-sprite
  `.asset.meta` files to recover tpsheet entry names.
