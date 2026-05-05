# TODO

Deferred items surfaced during planning. Address before shipping, not before M1.

## Byte-exactness gaps to validate

- **`m_Offset` formula incomplete.** Probed against AC_IC_Orgel (`pivot=0.485437, w=103, m_Offset.x=-1.4999886`): the eval order `pivot * size − size * 0.5` reproduces the f32 bits exactly (0xbfbfffa0). But against AC_PT_Icon_Gift (`pivot=0.45726, h=81, m_Offset.y=-3.4619446`): every evaluation order tried (`pivot*h − h*.5`, `(pivot-.5)*h`, `(2p-1)*h*.5`, FMA paths, f64-internal-then-cast) lands on -3.4619408 (0xc05d9070); target is -3.4619446 (0xc05d9080), 16 ULPs apart. Unity is not just doing pivot×size arithmetic — likely uses original (untrimmed) sprite size from the `.tps`, or per-vertex bbox of the geometry. Need to read Unity engine source (`Sprite.CreateSprite` C++) to derive. After migration runs once, Unity-emitted goldens will be byte-stable for our formula — but currently the e2e shows ~36% sprites diverging at `m_Offset` for non-(0.5,0.5) pivots.



- **Bootstrap experiment**: verify Unity preserves a Rust-supplied GUID across `AssetDatabase.ImportAsset`. Procedure: delete `Cake__DecoLeft.asset` + `.asset.meta` from a clean Unity-closed checkout; boot Unity, trigger postprocessor; assert the new `.asset.meta` GUID equals `m_RenderDataKey` GUID in the new `.asset`. Repeat with `Library/` cleared. If GUIDs diverge, the FFI contract needs a second-pass write of `m_RenderDataKey` after Unity stabilizes the meta. — gating risk
- **Non-zero-border golden test**: meow-tower carries 51 sprites with non-zero borders (positive AND negative; e.g. `NonogramSkins/30_Orgel/Frame` `{0,108,88,94}`, `OrgelContents/0203/DomeDecor__H` `{0,-3,0,0}`). My order/eval matches Unity for both samples, but `emit::emit` hard-fails with `EmitError::NonZeroBorderUnsupported` until a dedicated fixture set + golden-byte tests exist. Action: copy 3-5 non-zero-border sprites into `tests/golden/borders/`, add a per-sprite assert_eq, then delete the guard in `emit.rs`.
- **Non-1.0 spriteScale fixture**: 54 of 62 Orgel sprites have non-1 `spriteScale` in the current `Orgel.tps`, but the committed `.asset` goldens were emitted with the old `.tps` state. The byte-exact integration test currently skips these. Either re-import in Unity (regenerate `.asset` goldens) or capture a fresh consistent fixture pair `(Foo.tps, Foo.tpsheet, Foo/*.asset)` from a different atlas.
- **`settingsRaw` bit layout**: every sampled `.asset` has `settingsRaw: 192`. Diff this across atlases with different filter mode, wrap mode, color space settings — find a varied fixture or rule out variation. Until then, hardcode 192 with a panic-guard.
- **`m_AtlasRD` vs `m_RD` divergence**: identical for non-SpriteAtlas sprites (verified). Confirm the constraint with a SpriteAtlas-managed fixture; panic on `m_SpriteAtlas != {fileID:0}` until that's spec'd.
- **Float format unit-test table**: build before M3. Seed by `grep -oE '[0-9]+\.[0-9]+' tests/golden/**/*.asset | sort -u`. Each value verified against C# `((float)x).ToString("R", CultureInfo.InvariantCulture)`.

## C# integration items (defer to M8)

- `TexturePackerUtils.cs:103, 167` — `GetPrefixFromTpsheet` and `GetSourceImagePath` currently read prefix from `.tpsheet.meta`. Update to read from `TPSImporter` on `.tps` BEFORE running the migration script.
- `TPSheetImporter.cs` — keep around during dual-path lifetime; delete after legacy is stripped.
- First-time atlas import PPU: fresh PNG has `spritePixelsPerUnit = 100` default. Document the gotcha; developer must set PPU and trigger reimport for first import to pick up custom PPU. Alternative: move PPU onto `TPSImporter`.
- `ti.SaveAndReimport()` re-entry hazard at `TPSheetPostprocessor.cs:67`: confirm the new path doesn't recurse infinitely. The legacy branch uses `continue`; new branch falls through. Verify the importer-reconfigure is idempotent (already mostly the case via `dirty=false` short-circuit).

## Build & deployment

- macOS dylib `codesign --sign -` step in build recipe.
- Hand-author `.dylib.meta` / `.dll.meta` plugin-import flags (Editor only, all platforms off). Commit alongside binaries.
- `cargo xwin` setup for Windows builds from macOS.
- `abi_version()` handshake: bump on every FFI struct change. C# asserts on first call.
- `git_sha()` build stamp via `vergen` for diagnostic logging (optional).

## Test infrastructure

- `tests/golden/` directory layout: per-atlas folder containing `.tpsheet`, `.tps`, `.png.meta`, `.tpsheet.meta`, and the full set of expected `.asset` + `.asset.meta` files. Tests stage these into `target/test-tmp/<test>/` before running the pipeline (so the preserve-existing-meta branch is exercised).
- Diff harness: on byte-equality mismatch, write `target/diff/<name>.{actual,expected}` and print first divergent offset + 32-byte hex window.
- Mint-branch unit test using a seeded `StdRng`.

## Unity-side ergonomics (post-MVP)

- Plugin reload requires Editor restart on dylib commit. Document; consider a build-stamp the wrapper logs at startup.
- macOS Gatekeeper / quarantine xattr handling on first checkout.
