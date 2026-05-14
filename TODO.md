# TODO

> **Related:** [[CLAUDE.md]], [[BENCHMARKS.md]]

Deferred items surfaced during planning.

## CSA migration — Phase 1b ROOT CAUSED: tpsheet drift, not a pipeline bug

The 5-ULP `m_Rect.height` divergence on `PA_InfinitePencil_Clock` is
**upstream data drift**, not a Rust pipeline bug. Investigation steps
that nailed it:

1. Unity-side bit-level probe (`/tmp/pa-probe.cs`) dumped per-leaf
   `cis.MultiplyPoint(srcVert)` Y values + final combined AABB
   (`y0=0xC027B97E`, `y1=0x40324CCD`, `rect.h=0x44072A76`).
2. Rust-side trace in `combine::build_combined` + `apply_transform`
   dumped same. Clock1's per-vert Y bits matched Unity bit-exact; Clock2
   diverged 16 ULPs in `pre_y` (= `v_local × ui_scale`) at vertex 4:
   Unity `0x438308EB` vs Rust `0x438308DB`. Same code path, same
   inputs *from the loaded sprite* — so the divergence is in the input
   data, not in the chain.
3. Confirmed by running the pipeline *without* the fab manifest (so
   Clock2 emits as per-tpsheet): `cmp` against committed `Clock2.asset`
   diverges at char 349. Same for `Clock1.asset` (char 516) and
   `Color_FFEBBE.asset` (char 490). Multiple committed constituent
   sprites diverge from current-tpsheet emit.
4. The .tpsheet I regenerated via `texturepacker` produces vertex/rect
   data that doesn't match what TexturePacker historically produced
   when the committed `_sprite.asset` files were emitted. TexturePacker
   is deterministic w.r.t. source PNGs, so the source PNGs must have
   drifted since the goldens were last regenerated — or TexturePacker
   itself has version drift.

**Implication for Phase 1b validation:** byte-exact diff against the
*committed* `.asset` corpus cannot honestly succeed today, because the
committed bytes were emitted from a now-lost historical tpsheet. The
*correct* validation is a single-pass run-the-migration-end-to-end:
regenerate every atlas's `.tpsheet` via `TexturePackerCLI`, run the
pipeline, accept the new emit bytes as the canonical truth, commit the
resulting `_sprite.asset` diffs as part of the migration commit. The
Silloutte 1/2/3 goldens happen to align with the current TexturePacker
output and remain byte-exact, which is why those tests pass.

**Migration tool itself is correct.** Same Rust pipeline + same
tpsheet = bit-stable output regardless of whether parts are per-tpsheet
or fab-combined. `examples/fab_verify.rs` reproduces the
diagnostic; `examples/csa_dumper.cs` + `examples/csa_dump_to_fab.rs`
produce 20/20 structurally valid manifests covering all 58 CSA
prefabs.

## Byte-exactness gaps to validate

- ~~**`textureRect` sub-pixel shrink for some polygon-trimmed sprites.**~~ **SOLVED** (commit `285f264`), then **SUPERSEDED** (commit `17659b1`): preserve replaced with a hard `Error::TextureRectDivergence`. Later **DEMOTED to warning + overwrite** when a 7.0.3 corpus repack surfaced legacy `Tight + spriteMode:Multiple` outputs on dozens of atlases beyond the original 3 FriendInvite emoji — fail-loud became disproportionate to risk. Behavior now: `GenerateOutput.warnings` carries one entry per divergent sprite (and stderr echoes), pipeline overwrites with current-tpsheet rect.

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
  
  Hypothesis to test next: Unity internally stores pivot as something other than the .tpsheet-parsed f32 (maybe higher-precision via a different ingestion path), or computes m_Offset via `Sprite.CreateSprite`'s native C++ which may use Vector2 components with hidden f64 precision. Or m_Offset uses the sprite's actual mesh bbox computed in Unity's local-space transform, not the rect. The ad-hoc probe scripts used during the iteration-3 investigation have been deleted (commit history has them); reconstruct from the corpus values listed above if revisiting. After migration runs once, regenerated goldens will reflect our formula and become byte-stable — but the legacy goldens diverge.
  
  **Magnitude of the gap is sub-millipixel.** Across the 6 fixtures probed, the max |target − ours| is ~1e-5 pixels (OA_Lock) and the median is ~6e-6 px. That's 4-5 orders of magnitude below the screen-pixel grid and far below any rendering precision Unity uses at runtime. **Practical implication: shipping with the current X-exact, Y-imprecise formula causes invisible regenerations** (the .asset bytes change because m_Offset.y values differ by 1-32 ULPs, but the rendered sprites are pixel-identical). Treat the gap as git-diff churn on first regeneration, not as a visual regression. Re-prioritize closing it only when Unity engine source becomes available (e.g. via UnityCsReference repo or decompilation).



- **Bootstrap experiment**: verify *Unity* preserves a Rust-supplied GUID across `AssetDatabase.ImportAsset`. Full procedure in [[unity-probes.md#a-bootstrap-experiment--guid-preservation]]. The Rust-side half (mint → write → re-read → re-emit byte-identical) is covered by `pipeline::tests::pipeline_mint_then_preserve_is_byte_idempotent`. The Unity-side half is blocked on having the Editor in the loop. — gating risk
- ~~**Non-zero-border golden test**~~ — landed; the `EmitError::NonZeroBorderUnsupported` guard is gone (commit `38b3dae`) and a one-time corpus pass verified 50/51 non-zero-border sprites byte-exactly under the current formula. The lone outlier is .tps drift on `OG_0303_Window__Layer186`, not a formula gap. (The meow-tower e2e re-checks the corpus when tpsheets are present; the rust-side regression guard is `golden_parity` + the inline `cake_decoleft_*` tests.)
- **Non-1.0 spriteScale fixture**: 54 of 62 Orgel sprites have non-1 `spriteScale` in the current `Orgel.tps`, but the committed `.asset` goldens were emitted with the old `.tps` state. The byte-exact integration test currently skips these. Procedure to refresh the fixture in [[unity-probes.md#d-non-10-spritescale-fixture-refresh]].
- **`settingsRaw` bit layout**: every sampled `.asset` has `settingsRaw: 192`. Until a varied fixture proves otherwise, hardcoded 192 (no surface guard — see CLAUDE.md emit note). Probe procedure in [[unity-probes.md#b-settingsraw-bit-layout]].
- **`m_AtlasRD` vs `m_RD` divergence**: identical for non-SpriteAtlas sprites (verified). The planned `m_SpriteAtlas != {fileID:0}` guard waits on a SpriteAtlas-managed fixture; probe procedure in [[unity-probes.md#c-m_atlasrd-vs-m_rd-divergence-under-spriteatlas]].
- ~~**Float format unit-test table**~~ — landed in `yaml::tests::float_corpus_full_roundtrip` (commit `3b0b8ec`). Scans every `.asset` under `tests/golden/`, extracts every distinct fractional float literal (93 currently), and asserts `yaml::float()` round-trips each one bit-exactly. Future Rust Display divergence from C# `ToString("R")` will surface here as a unit-test failure rather than a golden-byte mismatch.

## SMA Phase 2b blockers (post first-run)

After Phase 2b's first-pass corpus run (`examples/migrate_corpus`, `.tps.mesh.json` siblings in place), the migrator lands **234/244 atlases**. The remaining 10:

- **9 Box atlases** — `Boxes/{03_Unicorn,05_Dino,09_Wizard,28_Boardgame,29_Ghost,30_Sleepy,31_Raincoat,32_Pilot,33_Vampire}` fail with `mesh 'Back'/'Wall' references unknown sprite 'mBzYY2st'`. Root cause: these prefabs have child SpriteRenderers named `Polygon` whose `.sprite` is a **runtime-created Color texture** (no asset GUID, hashed name like `mBzYY2st`). The SMA C# pipeline accepts these because it walks geometry, not atlas entries. Our Rust port (`mesh_emit::build_mesh`) requires every renderer's sprite to resolve in the sibling tpsheet — there's no polygon/color-fill emit path on the mesh side. To unblock:
  - **Dumper extension** (`examples/sma_dumper.cs`): detect `sprite_guid == ""` + hashed name, recover `(color RGBA, mesh.vertices, mesh.triangles)` from the SpriteRenderer's MeshFilter or directly from `sprite.vertices`.
  - **Manifest schema** (`src/mesh_manifest.rs`): add `MeshRendererKind::Polygon { color, vertices, triangles }` alongside the current sprite-backed renderer.
  - **Emit extension** (`src/mesh_emit::build_mesh`): for `Polygon` renderers, synthesize verts directly from the captured polygon and sample UVs from a `Color_RRGGBB` entry in the same tpsheet (mirrors the CSA polygon path in `combine::polygon_mesh_with_tris`). Color_RRGGBB sprites already exist in every Box atlas.
  - **Golden coverage**: add a Box_29_Ghost-style fixture exercising the polygon branch.

- **1 Tower.tps** — atlas outputs to `Tower_SpriteAtlas.tpsheet` (non-matching stem). `examples/migrate_corpus` assumes `<stem>.tpsheet`. Either special-case in the runner or rename the .tps to match the .tpsheet stem.

Phase 3 (atomic C# + prefab deletion) cannot proceed until the polygon-path work above lands — deleting `SpriteMeshAuthoring/*.cs` without a Rust replacement leaves the 9 Box prefabs without a Mesh emit path.

## C# integration & Unity-side ergonomics

Concerns that live in meow-tower / BoxcatBridge now, not in this rlib:

- First-time atlas import PPU gotcha — owned by `TPSheetPostprocessor.cs` / `TPSImporter.cs` docs in meow-tower.
- Plugin reload requires Editor restart on dylib commit — owned by BoxcatBridge.
- macOS Gatekeeper / quarantine xattr on first checkout — owned by BoxcatBridge.

## Build & deployment

Build/deploy concerns (universal macOS dylib, `cargo xwin` Windows cross, `.dylib.meta` plugin-import flags, codesign) moved to the BoxcatBridge crate in meow-tower when the cdylib path was retired. Nothing left for this crate to deploy directly.

## Test infrastructure

All three items landed (per-atlas golden layout under `tests/golden/<atlas>/`, diff harness in `src/emit.rs` + `tests/golden_fab_silloutte.rs` writing `target/diff/<name>.{actual,expected}`, deterministic mint-branch seam at `meta::tests::mint_guid_from_seeds_is_deterministic`). Nothing outstanding.

## `.tps.fab.json` follow-ups

> See [[fab.md]] for the v1 contract.

Deferred from v1:

- ~~**Byte-exact `CanvasSpriteAuthor.Publish()` reproduction.**~~ **SOLVED** for Silloutte1/2/3 (`tests/golden_fab_silloutte.rs` runs in default CI). Fix involved four changes:
    - **Schema additions**: `Combined.canvasScale`, `Part::AtlasSprite.uiScale`, `Part::AtlasSprite.offset`, `Part::Polygon.triangles` (UISolid quad index override).
    - **`local_src_verts`**: multiply-by-precomputed-reciprocal `(px - pivot_px) * (1/ppu)` (matches Unity's stored `Sprite.vertices`).
    - **Per-part transform**: `apply_transform` does `v_canvas × canvasScale + m13 + translation` in that exact f32 op order, where `m13` is the FMA-fused per-`CombineInstance` translation row (see the Silloutte3 bullet below). For `rootAnchored = (0, 0)` this collapses to `offset × canvasScale`, which mirrors `Matrix4x4.MultiplyPoint`'s precomputed translation row. The algebraically-equivalent `(v + offset) × canvasScale` rounds 1 ULP differently.
    - **Polygon UV**: `polygon_mesh_with_tris` multiplies by `1/atlas.{width,height}` (mirrors `SolidUVCache.Get` averaging Unity's already-multiplied `DataUtility.GetInnerUV` bounds).
    - **`atlasRectOffset`**: Fabricated branch now emits `(-1, -1)` (Unity's sentinel for non-SpriteAtlas sprites), not `(0, 0)`. Both the CLAUDE.md trap note and the emit code had encoded a false belief that `SpriteFactory.CreateFromMesh` ships `(0, 0)`; the Silloutte1 golden disproves it.
    - **Discovery**: each UIIcon part has a uniform stretch factor of `sizeDelta / native_size` (not just `_scaleFactor`-based scaling) that has to be encoded in the manifest as `sx`/`sy`. This was non-obvious and only verified empirically against the golden mesh. Future authoring tools / exporters need to compute this stretch from the prefab data.
    - **Silloutte2 byte-exact**, **Silloutte3 byte-exact** (the latter solved by tracing through the UnityCsReference + meow-tower decompiler stack — `CanvasSpriteAuthor.GenerateMesh` builds each `CombineInstance.transform` as `baseMatrix * g.transform.localToWorldMatrix`, where the per-instance matrix's `m13` translation row is computed FMA-fused at the matrix-multiplication step but the per-vertex transform itself uses regular two-step f32 chains. For Silloutte3 (root anchored `(141.8, 370.875)`) that FMA residual is `~-9.24e-8` and shifts every y-coordinate by ~1 ULP. Schema gains `rootAnchored: [f32; 2]` on `Combined`; `combine::compute_m13_axis` does the f64-promotion FMA fusion (`canvas_scale × (root + offset) + (-canvas_scale × root)`) once per part; `apply_transform`'s per-vert chain stays as two f32 ops. For `rootAnchored = (0, 0)` the formula collapses to `canvas_scale × offset` exactly, so Silloutte1 + Silloutte2 + every Box/SpriteRenderer use case is unchanged.)
    - **Caveat — `apply_transform` op order**: verified against Silloutte1 where the only non-1 affine component is `sx=-1` (no rounding noise). A future fab manifest using non-trivial `sx`/`sy`/`rotDeg` *together with* non-1 `uiScale` should be cross-checked against a Unity probe — the f32 order between per-part affine and the canvas chain may need to swap. The current op order is now pinned by five regression tests in `combine::tests::apply_transform_*` (identity collapse, ui-scale post-multiply, matrix-vs-naive 1-ULP divergence at `(0.1, -300, 0.01)`, world-unit translate, pre-canvas affine scale) — any drift will surface at the unit-test level instead of waiting on the byte-exact Silloutte1 test.
- **Color-PNG synthesis**: when a polygon part's `polygonSprite` is missing from the atlas and the name matches `Color_RRGGBBFF`, synthesize the pixel PNG into the source `Sprites_Export~/` so the next TexturePacker pack picks it up. Mirrors `CanvasSpriteAuthor.ReplaceColorTextures` / `ColorTextureUtils.CreateTexture`. Probably a separate CLI step rather than inside `generate()`, since `generate` runs *after* TP and shouldn't trigger a re-pack from the C# postprocessor path.
- **Standalone exporter** from meow-tower `BoxAuthor` / `SpriteMeshAuthor` tree to bootstrap manifests for the 39 existing box prefabs.
- **`keepStandalone` allowlist** if a part ever needs both standalone and combined emission. Otherwise rename in TexturePacker.
- **Bilinear UV sampling for polygon parts** (UV maps 0..1 onto `polygonSprite`'s full atlas rect). Defer until a non-solid polygon part appears.
- **`FX`/`FY`/`FXY` method aliases** that desugar to negative `sx`/`sy`. Skipped in v1 to keep one obvious way.

Decided:

- **Manifest discovery is implicit** (`<tps_path>.fab.json`). No new `pipeline::generate` parameter. Matches how `.png.meta` / `.tps.meta` siblings are already discovered. Revisit only if a real need to override appears.
- **Skip-write-if-equal kept**, including for fabricated sprites. Combined `.asset`s are only ~25% larger than per-tpsheet sprites (~7-8 KB vs ~6 KB avg); per-file read stays sub-20 µs, ~5-20× cheaper than the write it avoids, and dwarfed by the avoided Unity reimport of dependents. One code path, no special case.
- **Drop the on-disk `textureRect` preserve branch; warn + overwrite instead.** Corpus survey originally found **3 divergent** sprites (all FriendInvite emoji); the 2026-05-05 fail-loud version covered that. A later 7.0.3 corpus repack surfaced the same drift on legacy Orgel/PiggyBank atlases (Tight + spriteMode:Multiple), making fail-loud disproportionate. Current behavior: warn via `GenerateOutput.warnings` + stderr, overwrite with the current rect. Sub-pixel — visually invisible.
