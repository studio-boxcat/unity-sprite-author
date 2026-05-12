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

All crate infrastructure required for byte-exact reproduction has landed
(Phases 1–7 in `docs/fab.md`). What remains is *authoring* the
`<atlas>.tps.fab.json` manifests:

- For each part (`SR` / `T` / `B` / `Image` / `SL`), translate the prefab's
  `RectTransform.{anchoredPosition, sizeDelta, pivot}` and the
  `CanvasSpriteAuthor._scaleFactor` (`0.01`) into per-part `tx` / `ty` /
  `partPivot` / `sx` / `sy`.
- UIIcon parts (`SR`, `T`, `B`, `SL`) map to method `Id` (native-scale).
- UISolid parts (`Image`) map to `Part::Polygon` with the four corner verts
  of `sizeDelta * scaleFactor`.
- `combine::calc_rect_and_pivot` should yield `m_Rect (282.5, 770)` and
  `m_Pivot (0.5, 0.40551946)` matching `Silloutte1.asset`.

The pipeline integration test
`pipeline::tests::pipeline_emits_combined_sprite_and_excludes_parts`
demonstrates the end-to-end wiring on a synthetic single-polygon fab.

Tracked at `TODO.md` "byte-exact `CanvasSpriteAuthor.Publish()` reproduction".
