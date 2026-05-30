# TODO

> **Related:** [[CLAUDE.md]], [[BENCHMARKS.md]], [[fab.md]], [[byte-format.md]]

Deferred items surfaced during planning.

## Unity-side probes (blocked on Editor in the loop)

Four byte-exactness questions need data from inside the Unity Editor, none
runtime-visible: GUID-preservation bootstrap, `settingsRaw` bit layout,
`m_AtlasRD` vs `m_RD` divergence under SpriteAtlas, and a non-1.0
`spriteScale` fixture refresh (the Orgel `Cake__DecoLeft` golden carries a
drifted `spriteScale`, so `golden_parity` skips its `m_PixelsToUnits`). Each
needs a one-off Editor capture to resolve; until then emit stays on the
corpus-verified formula.

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

## Deferred (waiting on a real case)

- **TexturePacker spawn overhead** — `texturepacker` boots in ~33 ms
  (`--version`); a small-atlas pack is ~50 ms total, so spawn/init is ~66%
  of it (real packing ~17 ms). Worth eliminating *if* it ever mattered for a
  frequently-repacking watch — but TexturePacker is a closed CLI with no
  public lib/SDK to FFI into, and no server/warm-process mode. 50 ms/pack is
  imperceptible for artist-facing repacks; revisit only if CodeAndWeb ships a
  library or daemon interface.
- **`keepStandalone` allowlist** — only matters if a part ever needs
  both standalone *and* combined emission. Otherwise just rename in
  TexturePacker. No corpus pressure today.
- **Bilinear UV sampling for polygon parts** — all polygon verts sample
  the polygon sprite's atlas-rect center pixel
  (`combine::polygon_uv_center`). Mirrors `SolidUVCache.Get` byte-exact.
  Switch to bilinear-over-rect only when a non-solid polygon part
  appears in authoring.
- **SMA polygon end-to-end golden** — the polygon branch of `mesh_emit`
  (`Graphic::Polygon` → `MeshRendererContent::Polygon`) is pinned only by
  `mesh_emit::tests::build_mesh_polygon_*` unit math; the `Box_29_Ghost`
  golden was captured pre-port and doesn't exercise it. Capture a real
  polygon-bearing SMA fixture when one is available.
- **Off-origin CSA root drifts ~1 ULP/vertex** — fab fixtures whose CSA
  root sits off-origin diverge ~1 ULP per vertex from Unity's emit (the
  retired Silloutte3 fixture). Pin the root at origin when authoring, or
  re-introduce the FMA-fused `compute_m13_axis` path it exercised.
