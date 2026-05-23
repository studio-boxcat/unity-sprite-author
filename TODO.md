# TODO

> **Related:** [[CLAUDE.md]], [[BENCHMARKS.md]], [[fab.md]], [[sma-migration.md]], [[unity-probes.md]]

Deferred items surfaced during planning.

## Unity-side probes (blocked on Editor in the loop)

Four byte-exactness questions need data from inside the Unity Editor:
GUID preservation bootstrap, `settingsRaw` bit layout, `m_AtlasRD` vs
`m_RD` divergence under SpriteAtlas, and a non-1.0 `spriteScale` fixture
refresh. Procedures + result formats live in [[unity-probes.md]].

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

9 Box atlases ship `Polygon` SpriteRenderers backed by runtime Color
textures that the Rust port currently rejects. Full work breakdown in
[[sma-migration.md#outstanding-phase-2b-polygon-color-synthesis]].

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
