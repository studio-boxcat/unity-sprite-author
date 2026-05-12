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

## Status

**All three sprites byte-exact** under default `cargo test`
(`tests/golden_fab_silloutte.rs`). Manifest at
`PremiumCat_Vampire_Popup.tps.fab.json`.

Manifest authoring recipe (for future fab fixtures):

- For each part, translate the prefab's
  `RectTransform.{anchoredPosition, sizeDelta, m_Pivot}` and the
  `CanvasSpriteAuthor._scaleFactor` (`0.01`) into the part's
  `offset` / `partPivot` / `uiScale` (`100` for UIIcon, `1` for UISolid).
- UIIcon `_method` 0/1/2/3 → `"ID"` / `"MX"` / `"MY"` / `"MXY"`.
  `_method` 4/5/6 (FX / FY / FXY) → `"ID"` plus negative `sx` / `sy`.
- UISolid parts map to `Part::Polygon` with the four corner verts in
  canvas pixels (`±sizeDelta/2`) and an explicit `triangles: [0, 2, 3, 3, 1, 0]`
  for the BL/BR/TL/TR vertex layout.
- Resolve sprite GUIDs in the prefab against the atlas's per-sprite
  `.asset.meta` files to recover tpsheet entry names.
- Set the combined `canvasScale: 0.01` and `rootAnchored: [<root_ap_x>, <root_ap_y>]`
  from the prefab root's `RectTransform.anchoredPosition`. The root anchored
  matters for byte-exactness — see `combine::compute_m13_axis` for the
  FMA-fused chain that captures Unity's `Mesh.CombineMeshes` residual.
