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

## Watchman watcher (remaining)

The watchman-based watcher that replaces `TPSheetPostprocessor` is
implemented: `crates/watch/` (rlib), bridge FFI
(`bxc_sprite_author_watch_{init,poll}`), C# polling via
`BoxcatBridgeInit` + `EditorApplication.update`.
`TPSheetPostprocessor` is deleted. `.tpsheet` is retained on disk
(pipeline no longer deletes it) so TexturePacker's `smartUpdateKey`
hash check works on next GUI publish.

**Remaining:**

- Migrate `_prefix` from `.tps.meta` `TPSImporter` into the fab.json
  manifest. Once done, delete `TPSImporter` + `SpriteAuthorGenerate`
  FFI (the old bridge entry point).

## Deferred (waiting on a real case)

- **`keepStandalone` allowlist** — only matters if a part ever needs
  both standalone *and* combined emission. Otherwise just rename in
  TexturePacker. No corpus pressure today.
- **Bilinear UV sampling for polygon parts** — all polygon verts sample
  the polygon sprite's atlas-rect center pixel
  (`combine::polygon_uv_center`). Mirrors `SolidUVCache.Get` byte-exact.
  Switch to bilinear-over-rect only when a non-solid polygon part
  appears in authoring.
