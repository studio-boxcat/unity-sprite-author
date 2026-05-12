# Silloutte fab fixtures

Three `CanvasSpriteAuthor`-published sprites from
`meow-tower/Assets/21_Collections/PremiumCatEvents/33_Vampire/Elements/Ornaments/`.
The .prefab is the source-of-truth; the .asset is what Unity emits when
`CanvasSpriteAuthor.Publish()` runs.

| File | Source |
| --- | --- |
| `PremiumCat_Vampire_Popup.{tps, tpsheet, png.meta}` | The shared atlas + sidecar metadata. `.tpsheet` regenerated locally via the TexturePacker CLI before capture. |
| `Silloutte{1,2,3}.prefab` | UI hierarchy under a `CanvasSpriteAuthor` root: `UIIcon` / `UISolid` children with per-RectTransform pivot and anchored position. |
| `Silloutte{1,2,3}.asset(.meta)` | Sprite asset committed in meow-tower. Byte-exact target. |

## What's pending

These fixtures don't have a `.tps.fab.json` yet because the v1 schema in
`docs/fab.md` doesn't capture per-part `RectTransform.pivot` — Silloutte parts
use mixed pivots like `(0, 0.5)` and `(0.5, 0)`, which shift the slice/method
output relative to the anchored position. The schema needs a `partPivot: [x, y]`
field on the atlas-sprite part variant (defaulting to `(0.5, 0.5)` for the
SpriteRenderer-tree use case).

The byte-exact test also requires the fabricated-sprite emit path:

- `m_Rect.{x, y} = (0, 0)`, `m_Rect.{w, h}` = mesh AABB size.
- `atlasRectOffset = (0, 0)` (not `(-1, -1)`).
- `m_Pivot` derived from mesh AABB so that pivot lands at the mesh origin.

Tracked in `TODO.md` under ".tps.fab.json follow-ups".
