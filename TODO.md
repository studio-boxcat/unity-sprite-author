# TODO

> **Related:** [[CLAUDE.md]], [[BENCHMARKS.md]], [[fab.md]]

Deferred items surfaced during planning.

## Unity-side probes (blocked on Editor in the loop)

- **GUID preservation bootstrap** — verify Unity preserves a Rust-supplied
  GUID across `AssetDatabase.ImportAsset`. The Rust half is covered by
  `pipeline::tests::pipeline_mint_then_preserve_is_byte_idempotent`; the
  Unity half needs a one-off probe per
  [[unity-probes.md#a-bootstrap-experiment--guid-preservation]].
- **`settingsRaw` bit layout** — every sampled sprite emits `192`; until a
  varied fixture proves otherwise we hardcode that constant. Probe:
  [[unity-probes.md#b-settingsraw-bit-layout]].
- **`m_AtlasRD` vs `m_RD`** — identical for non-SpriteAtlas sprites. The
  planned guard waits on a SpriteAtlas-managed fixture; probe:
  [[unity-probes.md#c-m_atlasrd-vs-m_rd-divergence-under-spriteatlas]].
- **Non-1.0 `spriteScale` fixture refresh** — 54/62 Orgel sprites have
  non-1 spriteScale today but the committed goldens predate that, so the
  byte-exact integration test currently skips them. Procedure:
  [[unity-probes.md#d-non-10-spritescale-fixture-refresh]].

## Sub-millipixel `m_Offset.y` drift

`m_Offset.x` is byte-exact across the corpus; `m_Offset.y` diverges by
1-32 ULPs (~1e-5 px max, ~6e-6 px median) on non-`(0.5, 0.5)`-pivot
sprites. Every f32 op order tried — `p*s − s*.5`, `(p-.5)*s`, `(.5-o)*s`,
`s*.5 − o*s`, FMA variants, f64-internal-then-cast, etc. — reproduces the
X axis bit-exactly but breaks `AC_Platform_Apple` (h=76, py=0.5125) on Y
with no candidate within ±32 ULP. Hypothesis: Unity stores pivot or
computes the offset via a `Sprite.CreateSprite` native-code path with
hidden f64 precision, or derives the offset from the mesh AABB not the
rect. Magnitude is invisible at runtime; revisit only when Unity engine
source becomes available (UnityCsReference or decompilation).

## SMA polygon-color synthesis (Phase 2b blocker)

9 Box atlases (`Boxes/{03_Unicorn,05_Dino,09_Wizard,28_Boardgame,29_Ghost,30_Sleepy,31_Raincoat,32_Pilot,33_Vampire}`)
have child SpriteRenderers named `Polygon` whose `.sprite` is a
runtime-created Color texture (no GUID, hashed name like `mBzYY2st`).
The Unity-side SMA pipeline accepts these because it walks geometry; the
Rust port requires every renderer's sprite to resolve in the sibling
tpsheet. To unblock the polygon path on SMA:

- **Migration tool** (`examples/csa_to_fab.rs` SMA branch): detect
  `sprite_guid == ""` + hashed name, recover `(color RGBA, mesh.vertices,
  mesh.triangles)` from the SpriteRenderer's MeshFilter or
  `sprite.vertices`.
- **Manifest schema** (`src/manifest.rs` v3 Node): add polygon-leaf
  fields to `spriteRenderer` mode (mirrors the existing UISolid path on
  CSA trees).
- **Emit extension** (`src/mesh_emit::build_mesh`): for polygon
  renderers, synthesize verts directly + sample UVs from a
  `Color_RRGGBB` entry in the same tpsheet (mirrors
  `combine::polygon_mesh_with_tris` on the CSA side).
- **Golden coverage**: extend `tests/golden/sma/box_29_ghost/` to
  exercise the polygon branch.

`Tower.tps` is the other Phase 2b miss — its atlas outputs to
`Tower_SpriteAtlas.tpsheet` (non-matching stem) and the migrator assumes
`<stem>.tpsheet`. Either special-case in the runner or rename the .tps.

## Pack-step features (CLI follow-ups)

- **1×1 color PNG synthesis.** When a tree references a polygon `color`
  whose `Color_RRGGBB` entry is missing from the tpsheet, synthesize the
  pixel PNG into the source `Sprites_Export~/` dir and add it to the
  `.tps`, so the next TexturePackerCLI pack picks it up. Mirrors
  meow-tower's `CanvasSpriteAuthor.ReplaceColorTextures` /
  `ColorTextureUtils.CreateTexture`. Naturally fits the
  `unity-sprite-author` CLI pack step (one place that already touches
  TexturePacker), not `pipeline::generate` (which runs *after* the pack).
- **`keepStandalone` allowlist** if a part ever needs both standalone
  and combined emission. Otherwise rename in TexturePacker.
- **Bilinear UV sampling for polygon parts.** All polygon verts sample
  the polygon sprite's atlas-rect center today. Defer until a non-solid
  polygon part appears.

## Phase-1b corpus regen (historical: root-caused, not actioned)

The `PA_InfinitePencil_Clock` byte-exactness gap was traced to upstream
tpsheet drift, not a pipeline bug. Investigation summary:

- Unity-side bit-level probe + Rust-side trace showed Clock1's per-vert
  Y bits matched Unity bit-exact; Clock2 diverged 16 ULPs in
  `v_local × ui_scale` at vertex 4. Same code path, same inputs from
  the loaded sprite — so the divergence is in the input data.
- Running the pipeline *without* the fab manifest also diverged on
  multiple constituent sprites. The .tpsheet I regenerated via
  `texturepacker` produces data that doesn't match what TexturePacker
  historically produced when the goldens were emitted. Source PNGs (or
  TexturePacker itself) drifted since the last regen.

Correct path forward is the one-shot corpus regen: re-run TexturePackerCLI
across every atlas, run the pipeline, accept the new bytes as canonical,
commit the resulting `_sprite.asset` diffs together. The Silloutte
goldens align with current TexturePacker output, which is why those tests
stay green.

The pa_clock diagnostic test was dropped along with the v1 fab schema
retirement (commit immediately preceding this entry) — reproducing the
diagnostic now requires running `pipeline::generate` against a real
meow-tower atlas under `--skip-pack`.
