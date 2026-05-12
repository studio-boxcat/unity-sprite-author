# Unity-side probe procedures

> **Related:** [[CLAUDE.md]], [[TODO.md]]

Four open TODO items need data from inside the Unity Editor that the Rust
test suite can't fabricate (the editor is the source of truth for
`m_RenderDataKey` ↔ `.asset.meta` round-tripping, `settingsRaw`'s bit
layout, `m_SpriteAtlas`-driven `m_AtlasRD` divergence, and per-sprite
`spriteScale` after a Texture Importer reimport). This doc captures the
procedure for each so any maintainer can run them, paste the output back
into TODO.md, and close the item without re-deriving the setup.

All four assume the meow-tower checkout at `$MEOW_CLIENT` (alias for
`$MEOW_ROOT/meow-tower`). The Rust side is consumed via the BoxcatBridge
cdylib — no separate dylib drop is needed; the `bxc_sprite_author_generate`
entry point wraps `pipeline::generate` in this rlib.

## A. Bootstrap experiment — GUID preservation

**Question.** Does Unity preserve the GUID this crate writes into a
fresh `.asset.meta` across `AssetDatabase.ImportAsset`? If it overwrites
the GUID at import time, BoxcatBridge needs a second-pass rewrite of
`m_RenderDataKey` after the import settles.

**Rust-side coverage.** `pipeline::tests::pipeline_mint_then_preserve_is_byte_idempotent`
proves the rlib's mint → write → re-read → re-emit chain is a fixed
point. The Unity-side question below is the remaining unknown.

**Procedure.**

1. Close Unity.
2. From the meow-tower repo root, snapshot a known sprite:
   ```sh
   cp Assets/21_Collections/OrgelContents/1204/Orgel/Cake__DecoLeft.asset{,.bak}
   cp Assets/21_Collections/OrgelContents/1204/Orgel/Cake__DecoLeft.asset.meta{,.bak}
   ```
   Extract the current GUID for comparison: `grep ^guid: ...Cake__DecoLeft.asset.meta.bak`.
3. Delete the originals (force the mint branch on next import):
   ```sh
   rm Assets/21_Collections/OrgelContents/1204/Orgel/Cake__DecoLeft.asset
   rm Assets/21_Collections/OrgelContents/1204/Orgel/Cake__DecoLeft.asset.meta
   ```
4. Boot Unity. The `TPSheetPostprocessor` fires on the sibling `Orgel.tpsheet`
   and re-emits both files; our rlib mints a fresh GUID.
5. Read the new pair:
   ```sh
   grep ^guid: Assets/21_Collections/OrgelContents/1204/Orgel/Cake__DecoLeft.asset.meta
   grep -A1 m_RenderDataKey Assets/21_Collections/OrgelContents/1204/Orgel/Cake__DecoLeft.asset
   ```
   The `guid:` value in the meta MUST equal the `first:` value under
   `m_RenderDataKey` in the asset. If they diverge, Unity rewrote one
   of them mid-import.
6. Clear `Library/` and repeat from step 4 — Unity rebuilds its internal
   asset DB from scratch on this path, which is where any post-mint
   rewrite would surface.
7. Restore the originals:
   ```sh
   mv .../Cake__DecoLeft.asset.bak .../Cake__DecoLeft.asset
   mv .../Cake__DecoLeft.asset.meta.bak .../Cake__DecoLeft.asset.meta
   ```

**Result format.** Paste into TODO.md under the Bootstrap item:
- `guid (meta) = <32-hex>`, `m_RenderDataKey.first = <32-hex>`, **match / divergent**.
- If divergent: the rlib mint contract is unchanged; BoxcatBridge gets a
  second-pass `m_RenderDataKey` rewriter that triggers after Unity's
  postprocessor settles. File a follow-up issue with the exact divergence.

## B. `settingsRaw` bit layout

**Question.** Every sprite in the meow-tower corpus emits `settingsRaw: 192`
(0xC0). Is this constant across filter mode / wrap mode / color space, or
does it actually pack those bits? The current emit hard-codes 192 with no
guard (see CLAUDE.md trap note); a varying corpus would surface as e2e
mismatch but never as a localized failure.

**Procedure.** Pick three Texture Importers with deliberately differing
settings and read back the emitted `settingsRaw`:

1. From the meow-tower repo root, pick three atlas PNGs with stable goldens:
   ```sh
   ls Assets/21_Collections/OrgelContents/1204/Orgel.png
   ls Assets/10_UIElements/CommonAtlas.png
   ls Assets/31_Fx/Sprites/FX_Raindrop_Seq_01.png
   ```
2. For each PNG, open `<atlas>.png.meta` and vary one knob per pass:
   - filter mode: `0` (Point), `1` (Bilinear), `2` (Trilinear).
   - wrap mode: `0` (Repeat), `1` (Clamp), `2` (Mirror).
   - sRGBTexture: `0` (linear), `1` (sRGB).
3. Trigger Reimport on the atlas (`AssetDatabase.ImportAsset(<png-path>, ForceUpdate)`
   via `just scratch '...'` if llm-bridge is available, or right-click → Reimport in the Editor).
4. Read the emitted `settingsRaw` from any sprite in that atlas:
   ```sh
   grep settingsRaw Assets/21_Collections/OrgelContents/1204/Orgel/Cake__DecoLeft.asset
   ```
5. Tabulate which knob (if any) shifts the value off 192.
6. Restore the originals via git checkout once the matrix is complete.

**Result format.** A 3-by-3 table (or "all 192") in TODO.md, plus the
mapping from knob → bit position. If a single knob varies it, add a
`SpriteAsset.settings_raw` field threaded from the importer and emit
that value verbatim, with the existing 192 default for the unmapped
flags.

## C. `m_AtlasRD` vs `m_RD` divergence under SpriteAtlas

**Question.** Every non-SpriteAtlas sprite in the corpus has
`m_AtlasRD == m_RD` (this rlib emits them identically). Once a sprite is
managed by a SpriteAtlas asset (the `m_SpriteAtlas` field becomes
non-zero), `m_AtlasRD` should point at the *packed* atlas rect while
`m_RD` keeps the original. Confirm the divergence exists, and decide
whether to fail loud (panic on `m_SpriteAtlas != {fileID:0}`) or thread
the SpriteAtlas data through.

**Procedure.**

1. Create a fresh `SpriteAtlas` asset in the meow-tower project that
   includes one of our atlases (e.g. Orgel). Right-click → Create →
   2D → Sprite Atlas; drag the atlas folder into the `Objects for Packing`
   list. Save.
2. Trigger atlas packing: `Window → 2D → Sprite Atlas → Pack Preview` (or
   `SpriteAtlasUtility.PackAllAtlases()` via `just scratch` if available).
3. Open one of the now-packed sprites' `.asset` and inspect:
   ```sh
   grep -A20 m_RD Assets/.../Cake__DecoLeft.asset
   grep -A20 m_AtlasRD Assets/.../Cake__DecoLeft.asset
   grep m_SpriteAtlas Assets/.../Cake__DecoLeft.asset
   ```
4. Compare every sub-field between `m_RD` and `m_AtlasRD`. Note which
   fields diverge (`texture.guid`, `textureRect`, `uvTransform`, etc.).
5. If divergent: the rlib needs an `AtlasPacked { texture_guid, texture_rect, uv_transform }`
   variant on `SpriteSource` so emit branches on it. Until then, keep
   the planned guard.
6. Delete the test SpriteAtlas and revert.

**Result format.** Either "confirmed identical even under SpriteAtlas
(no-op)" or a list of divergent fields with example bytes for one
sprite — that list becomes the spec for the new emit branch.

## D. Non-1.0 `spriteScale` fixture refresh

**Question.** 54 of 62 Orgel sprites have `spriteScale != 1.0` in the
current `Orgel.tps`, but the committed `.asset` goldens predate that
change — the e2e currently skips them rather than fail. We need a
consistent `(.tps, .tpsheet, .asset)` triple.

**Procedure.**

1. From the meow-tower checkout, ensure the current `Orgel.tps` and
   `Orgel.tpsheet` are what TexturePacker last emitted (commit `tps drift`
   in the rust repo's e2e report if uncertain).
2. Force a reimport so this rlib re-emits every Orgel sprite under the
   current `.tps`:
   ```sh
   touch Assets/21_Collections/OrgelContents/1204/Orgel.tpsheet
   # Boot Unity, wait for the postprocessor to settle, quit.
   ```
3. From the rust repo, copy the regenerated triple back into the fixture:
   ```sh
   cp $MEOW_CLIENT/Assets/21_Collections/OrgelContents/1204/Orgel.tps        tests/golden/orgel/
   cp $MEOW_CLIENT/Assets/21_Collections/OrgelContents/1204/Orgel.tpsheet    tests/golden/orgel/
   cp $MEOW_CLIENT/Assets/21_Collections/OrgelContents/1204/Orgel/*.asset    tests/golden/orgel/sprites/
   cp $MEOW_CLIENT/Assets/21_Collections/OrgelContents/1204/Orgel/*.asset.meta tests/golden/orgel/sprites/
   ```
4. Run `cargo test golden_parity` and `MEOW_TOWER_PATH=$MEOW_CLIENT cargo test --test e2e_meow_tower`.
   The e2e byte-exact rate for Orgel should jump from "skipped (drift)" to
   100% byte-exact.

**Result format.** A diff in `tests/golden/orgel/` plus an entry in the
commit message noting the byte-exact rate before/after.

## Archive

- **Silloutte3 root-anchored y-shift** — SOLVED in `combine::compute_m13_axis`; `Combined.rootAnchored` schema field. Resolved by walking `CanvasSpriteAuthor.GenerateMesh` → `Matrix4x4.operator *` → `Mesh.CombineMeshes` in the UnityCsReference + meow-tower decompiler stack: the per-`CombineInstance` matrix's `m13` translation row is FMA-fused at matrix-multiply time, while the per-vertex transform itself uses two-step f32. For Silloutte3 (root anchored `(141.8, 370.875)`) the FMA residual is `~-9.24e-8`, shifting every y by 1 ULP. The same chain collapses to `canvas_scale × offset` exactly when `rootAnchored = (0, 0)`, so Silloutte1/2 + every Box/SpriteRenderer caller are unchanged.
