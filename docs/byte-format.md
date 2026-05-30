# Byte format — tpsheet → Sprite `.asset` / `.asset.meta`

> **Related:** [[CLAUDE.md]], [[fab.md]], [[TODO.md]]

The bar is byte-exactness with Unity's `EditorUtility.CopySerialized` emit. This is the reference for how each output byte is formed: the GUID policy, the two `.asset.meta` shape variants, the tpsheet→`.asset` field map, and the byte-exactness traps the corpus audit surfaced.

Reference fixture for parity testing: `tests/golden/orgel/` — a self-contained snapshot of `Orgel.{tpsheet, tps, png.meta}` + the per-sprite `.asset` / `.asset.meta` corpus (`Cake__DecoLeft.asset` is the canonical example). The matching meow-tower-side files live under `meow-tower/Assets/21_Collections/OrgelContents/1204/Orgel/`; the `.tpsheet` there is retained on disk after import (TexturePacker needs it for `smartUpdateKey`), so it's only absent before the first pack.

## GUID policy

- For sprite `<name>` in output dir `<dir>`:
  - If `<dir>/<prefix><name>.asset.meta` exists → read existing `guid`, preserve. Detect the file's shape (Legacy189 vs Modern186, `mainObjectFileID` value) and rewrite in the same shape so on-disk bytes don't churn just because we touched the file. The `guid` and detected shape are the only preserved values. Hand-edits to other fields are not supported.
  - Else → mint random 128-bit GUID, write a fresh `.asset.meta` in the Modern186 shape with `mainObjectFileID: 21300000`.
- `m_RenderDataKey` in the `.asset` body uses the SAME GUID as the sibling `.asset.meta` (verified against `Cake__DecoLeft.asset.meta` corpus, 3645 files: `m_RenderDataKey` always equals own meta GUID).
- Renames must be done in tpsheet AND Unity at the same time by the developer. This design does not detect renames automatically.

## `.asset.meta` shape

Two trailing-space variants exist in the corpus and one varying field:

- **Legacy189** (older Unity emit): trailing space after `userData:`, `assetBundleName:`, `assetBundleVariant:`. 189 bytes.
- **Modern186** (current Unity emit): no trailing spaces. 186 bytes.
- **`mainObjectFileID`**: usually `21300000` (Sprite class fileID). Some incompletely-imported sprites carry `0` instead.

To avoid byte churn on existing metas the pipeline preserves both axes when present (`meta::detect_shape`). Fresh mints use Modern186 + `21300000`. Golden tests stage the committed `.asset.meta` before invoking the pipeline so the preserve branch is exercised; the mint branch is unit-tested via `mint_guid_from(lo, hi)` with fixed entropy (no `rand` crate).

## Field map (tpsheet → `.asset`)

tpsheet line format (semicolon-separated):

```
<name>;<x>;<y>;<w>;<h>; <pivotX>;<pivotY>; <bL>;<bR>;<bT>;<bB>;
  <vCount>;<v0x>;<v0y>;...;
  <triCount>;<t0a>;<t0b>;<t0c>;...
```

| Sprite `.asset` field          | Source                                                              |
| ------------------------------ | ------------------------------------------------------------------- |
| `m_Rect`                       | tpsheet rect                                                        |
| `textureRect`                  | always tpsheet rect. If an existing `.asset` carries a divergent `textureRect.{w,h}` (only seen on legacy Tight + `spriteMode: Multiple` outputs), `generate()` emits a non-fatal warning via `GenerateOutput.warnings` (also echoed to stderr) and overwrites with the current tpsheet's rect. Current tpsheet is authoritative. |
| `m_Pivot`                      | tpsheet pivot                                                       |
| `m_Border`                     | tpsheet borders (LRTB)                                              |
| `m_PixelsToUnits`              | `ppu / spriteScale` (PPU from importer; spriteScale from `.tps`)    |
| `_typelessdata` pos (stream 0) | `(px − w·pivotX)/ppu`, `(py − h·pivotY)/ppu`, `0` — vec3 f32 LE      |
| `_typelessdata` uv (stream 1)  | `(rect.x + px)/atlasW`, `(rect.y + py)/atlasH` — vec2 f32 LE         |
| `_typelessdata` layout         | stream 0 packed, padded up to 16-byte boundary, then stream 1       |
| `m_DataSize`                   | `align16(vCount·12) + vCount·8`                                     |
| `m_IndexBuffer`                | tpsheet triangles, u16 LE                                           |
| `uvTransform`                  | `(ppu, rect.x + w·pivotX, ppu, rect.y + h·pivotY)`                  |
| `settingsRaw`                  | constant `192` (0xC0). No emit-side guard — divergence surfaces via the e2e byte-mismatch.|
| `texture` GUID                 | from atlas `.png.meta`                                              |
| `m_RenderDataKey` GUID         | own `.asset.meta` GUID (preserve or mint per policy above)          |

## Known byte-exactness traps (from corpus audit)

- `m_PackingTag: ` and `m_SpriteID: ` end with a literal trailing space before LF.
- File ends `m_SpriteID: \n` with single LF, no trailing blank line.
- `_typelessdata` is one unbroken hex line, never folded.
- `m_RenderDataKey` is the only non-flow nested mapping; everything else is flow `{x: ..., y: ...}`.
- `atlasRectOffset: {x: -1, y: -1}` — Unity's sentinel for non-SpriteAtlas sprites. Constant; applies uniformly to TexturePacker-imported sprites AND to `SpriteFactory.CreateFromMesh` outputs (verified against the Silloutte1 golden). The earlier "fabricated ships (0, 0)" claim was wrong — emit doesn't branch on `SpriteSource` here.
- `m_Border` field order is `{x: L, y: B, z: R, w: T}` per Unity `Sprite.cs`. Verified empirically: 50/51 non-zero-border sprites in the meow-tower corpus emit byte-exactly under the current formula (the lone outlier is .tps drift — golden has all-zero borders, current tpsheet has non-zero). The hard-fail guard was retired once this was proven.
- Float formatting must match C# `ToString("R")`. `yaml::float` uses Rust's default `Display`, which matches across the entire golden corpus (93 distinct fractional literals); the round-trip guard lives in `yaml::tests::float_corpus_full_roundtrip` so a future Display divergence surfaces as a unit failure instead of a golden-byte mismatch.
- `m_AtlasRD == m_RD` for non-SpriteAtlas sprites (verified across the corpus); emit always writes `m_SpriteAtlas: {fileID: 0}` as a constant. The "diverge under SpriteAtlas" hypothesis is a deferred probe (see [[TODO.md]] Unity-side probes) — guard wiring waits on a fixture.
- LF line endings; pin via `.gitattributes` (`*.asset binary`, `*.asset.meta binary`).
- `mainObjectFileID: 21300000` in every sprite `.asset.meta` (Unity class ID 213). Constant, not parameterized.
