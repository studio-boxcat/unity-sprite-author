# Unity-side probe procedures

> **Related:** [[CLAUDE.md]], [[TODO.md]]

Four open TODO items need data from inside the Unity Editor that the Rust
test suite can't fabricate (the editor is the source of truth for
`m_RenderDataKey` ↔ `.asset.meta` round-tripping, `settingsRaw`'s bit
layout, `m_SpriteAtlas`-driven `m_AtlasRD` divergence, and per-sprite
`spriteScale` after a Texture Importer reimport). This doc captures the
procedure for each so any maintainer can run them and close the item
without re-deriving the setup — record findings in the commit message of
the follow-up fix, or under [[TODO.md#unity-side-probes-blocked-on-editor-in-the-loop]].

All four assume the meow-tower checkout at `$MEOW_CLIENT` (alias for
`$MEOW_ROOT/meow-tower`). The Rust side is consumed via the BoxcatBridge
cdylib — no separate dylib drop is needed; the `bxc_sprite_author_generate`
entry point wraps `pipeline::generate` in this rlib.

**Common prerequisite (A, B, D).** `pipeline::generate` deletes the
`.tpsheet` on success, so it's only present mid-import. Before any
procedure that needs the postprocessor to fire, regenerate the sibling
`.tpsheet` either with `TexturePackerCLI <atlas>.tps` (Unity closed) or
right-click the `.tps` in the Editor → `Run TexturePacker`.

## A. Bootstrap experiment — GUID preservation

**Question.** Does Unity preserve the GUID this crate writes into a
fresh `.asset.meta` across `AssetDatabase.ImportAsset`? If it overwrites
the GUID at import time, BoxcatBridge needs a second-pass rewrite of
`m_RenderDataKey` after the import settles.

**Rust-side coverage.** `pipeline::tests::pipeline_mint_then_preserve_is_byte_idempotent`
proves the rlib's mint → write → re-read → re-emit chain is a fixed
point. The Unity-side question below is the remaining unknown.

**Procedure.** (Regen `Orgel.tpsheet` per the common prerequisite above.)

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
4. Boot Unity with the regenerated `Orgel.tpsheet` in place. The
   `TPSheetPostprocessor` fires on it and re-emits both files; our
   rlib mints a fresh GUID. The `.tpsheet` is deleted again on success.
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

**Procedure.** (Per-atlas `.tpsheet` regen as above.) Pick three
Texture Importers with deliberately differing settings and read back
the emitted `settingsRaw`:

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
3. Reimport the PNG so the texture importer settings update, then
   `touch <atlas>.tpsheet` to re-trigger `TPSheetPostprocessor` —
   the sprite `.asset` re-emit fires from the tpsheet import, not
   the png one.
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
change — `golden_parity` skips any sprite whose committed `m_PixelsToUnits`
doesn't match the test's `ATLAS_PPU = 80` (and the e2e would too, if
the post-rlib-pivot meow-tower checkout had `.tpsheet`s to walk). We
need a consistent `(.tps, .tpsheet, .asset)` triple.

**Procedure.**

1. From the meow-tower checkout, ensure the current `Orgel.tps` and
   `Orgel.tpsheet` are what TexturePacker last emitted (regen per the
   common prerequisite above).
2. Force a reimport so this rlib re-emits every Orgel sprite under the
   current `.tps`:
   ```sh
   touch Assets/21_Collections/OrgelContents/1204/Orgel.tpsheet
   # Boot Unity, wait for the postprocessor to settle, quit. The
   # .tpsheet is consumed on success — regenerate as above if you
   # want to re-run.
   ```
3. From the rust repo, copy the regenerated triple back into the fixture:
   ```sh
   cp $MEOW_CLIENT/Assets/21_Collections/OrgelContents/1204/Orgel.tps        crates/core/tests/golden/orgel/
   cp $MEOW_CLIENT/Assets/21_Collections/OrgelContents/1204/Orgel.tpsheet    crates/core/tests/golden/orgel/
   cp $MEOW_CLIENT/Assets/21_Collections/OrgelContents/1204/Orgel/*.asset    crates/core/tests/golden/orgel/sprites/
   cp $MEOW_CLIENT/Assets/21_Collections/OrgelContents/1204/Orgel/*.asset.meta crates/core/tests/golden/orgel/sprites/
   ```
4. Run `cargo test golden_parity` and `MEOW_TOWER_PATH=$MEOW_CLIENT cargo test --test e2e_meow_tower`.
   The e2e byte-exact rate for Orgel should jump from "skipped (drift)" to
   100% byte-exact.

**Result format.** A diff in `crates/core/tests/golden/orgel/` plus an entry in the
commit message noting the byte-exact rate before/after.

## Archive

Retired investigations have moved out of this doc — `git log -- docs/unity-probes.md` recovers the Phase-1b corpus regen narrative and the Silloutte3 root-anchored y-shift writeup. Two operational notes that didn't die with them:

- **Phase-1b corpus regen**: the path forward is a one-shot regen — re-run TexturePackerCLI across every atlas, run the pipeline, accept the new bytes as canonical, commit the resulting `.asset` diffs together. Silloutte goldens align with current TexturePacker output; Orgel doesn't (documented under D above).
- **Silloutte3 retirement principle**: future fab fixtures whose CSA root sits off-origin will drift ~1 ULP per vertex from Unity's emit. Pin the root at origin, or re-introduce the FMA-fused `compute_m13_axis` path that the retired Silloutte3 fixture exercised.
