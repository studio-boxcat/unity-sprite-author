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

## TPSheetImporter prefix migration (remaining)

`TPSheetImporter` (`ScriptedImporter` for `.tpsheet`) replaces the old
`TPSImporter` + `TPSheetPostprocessor` pair. Pipeline retains the
`.tpsheet` on disk; deduplication handled by Unity's import system
(importer only fires when file changes). `_prefix` migrated from
`.tps.meta` to `.tpsheet.meta`.

**Remaining:**

- Run `scripts/migrate-prefix-to-tpsheet.sh` on meow-tower after
  `.tpsheet` files exist on disk (requires one full TP corpus pack).

## `watch` (CLI) follow-ups

- No unit coverage for `crates/cli/src/watch.rs` — the hint→`.tps` matching
  (`TpsEntry::matches`), `.tps` discovery, and arg parsing are exercised only
  manually + via the shared `unity-watch` tests. Add fixtures if the matching
  logic grows (e.g. absolute `fileLists` entries, symlinked source dirs).
- Watchman `Fresh` on startup intentionally does **not** repack everything
  (only re-scans the `.tps` set). If a source changed while `watch` was down,
  that atlas stays stale until its next edit — run the one-shot CLI to catch up.

## `e2e_meow_tower` drift

`GiftShop` and `PiggyBank` atlases show 4 `.asset` mismatches in the opt-in
`e2e_meow_tower` test — the live meow-tower `.tps` drifted from the committed
`.asset` goldens (same class as the `m_Border` outlier noted in CLAUDE.md, not
an emit regression: the test goes through `emit::emit`, untouched by the watch
work). Repack those atlases in meow-tower or refresh the goldens to re-green.

## Deferred (waiting on a real case)

- **`keepStandalone` allowlist** — only matters if a part ever needs
  both standalone *and* combined emission. Otherwise just rename in
  TexturePacker. No corpus pressure today.
- **Bilinear UV sampling for polygon parts** — all polygon verts sample
  the polygon sprite's atlas-rect center pixel
  (`combine::polygon_uv_center`). Mirrors `SolidUVCache.Get` byte-exact.
  Switch to bilinear-over-rect only when a non-solid polygon part
  appears in authoring.
