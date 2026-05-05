# TODO

Deferred items surfaced during planning. Address before shipping, not before M1.

## Byte-exactness gaps to validate

- ~~**`m_Offset` formula — X solved, Y unsolved.**~~ **SOLVED** in iteration 3: `m_Offset = (rect.pos + pivot * rect.size) - (rect.pos + rect.size * 0.5)`. The `rect.x`/`rect.y` mathematically cancel but introduce f32 rounding noise that exactly matches Unity. Verified across all 6 stuck fixtures; e2e byte-exact rate jumped 64% → 81% across the meow-tower corpus.

- ~~**Non-zero `m_Border` hard-fail guard.**~~ **REMOVED** in iteration 3: 50/51 non-zero-border sprites in meow-tower emit byte-exactly under the current formula. The lone outlier is .tps drift (golden has zero borders, current tpsheet has non-zero) — not a formula bug.

Old m_Offset analysis (kept for history; ignore the "unsolved" framing):

- **`m_Offset` formula — X solved, Y unsolved.** `pivot.x * w − w * 0.5` reproduces f32 bits byte-exactly across all 9 fixtures probed (AC_IC_Orgel, AC_Platform_Apple, OE_Calendar, OE_Icon_Sun, OA_DC_Autumn2, OA_Lock, OA_ArrowBrown, OA_ArrowWhite). The Y axis fails on non-(0.5,0.5)-pivot sprites by varying ULP gaps that don't follow a single pattern:
  - AC_PT_Icon_Gift (h=81, py=0.45726): target -3.4619446 (0xc05d9080); my A: 0xc05d9070; matching pivot bits exist at delta -2 or -3 ULPs from canonical f32 parse of "0.45726" (0x3eea1dfc → 0x3eea1df9/dfa).
  - OE_Calendar (h=75, py=0.653333): target 0x4137ffe0; matching pivot delta -1.
  - OE_Icon_Sun (h=102, py=0.470588): target 0xc0400080; matching pivot delta -2.
  - OA_DC_Autumn2 (h=78, py=0.381443): target 0xc113f590; matching pivot delta -2.
  - OA_Lock (h=115, py=0.817391): target 0x4211fff8; matching pivot delta +1.
  - **AC_Platform_Apple** (h=76, py=0.5125): target 0x3f733300; **NO matching pivot bits within ±32 ULP** with formula A. Matching `h` exists in range 75.99994..76 (delta -3..-8 ULPs); rect.h is integer 76 in tpsheet. Suggests Unity does not multiply by stored `rect.h` directly.
  
  Tried evaluation orders: `p*s − s*.5`, `(p-.5)*s`, `(.5-o)*s`, `s*.5 − o*s`, `s*(.5-o)`, `-(o-.5)*s`, FMA variants, f64-internal-then-cast, ppu round-trip via local units (with ppu=80/100), `1 − orig` precision paths. None reproduces target Y consistently. AC_Platform_Apple breaks every f32 formula attempted.
  
  Hypothesis to test next: Unity internally stores pivot as something other than the .tpsheet-parsed f32 (maybe higher-precision via a different ingestion path), or computes m_Offset via `Sprite.CreateSprite`'s native C++ which may use Vector2 components with hidden f64 precision. Or m_Offset uses the sprite's actual mesh bbox computed in Unity's local-space transform, not the rect. Probe scripts at `examples/probe_offset.rs` and `examples/probe_formulas.rs`. After migration runs once, regenerated goldens will reflect our formula and become byte-stable — but the legacy goldens diverge.
  
  **Magnitude of the gap is sub-millipixel.** Across the 6 fixtures probed, the max |target − ours| is ~1e-5 pixels (OA_Lock) and the median is ~6e-6 px. That's 4-5 orders of magnitude below the screen-pixel grid and far below any rendering precision Unity uses at runtime. **Practical implication: shipping with the current X-exact, Y-imprecise formula causes invisible regenerations** (the .asset bytes change because m_Offset.y values differ by 1-32 ULPs, but the rendered sprites are pixel-identical). Treat the gap as git-diff churn on first regeneration, not as a visual regression. Re-prioritize closing it only when Unity engine source becomes available (e.g. via UnityCsReference repo or decompilation).



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
