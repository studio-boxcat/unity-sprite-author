# unity-sprite-author

> **Related:** [[TODO.md]], [[BENCHMARKS.md]], [[fab.md]], [[unity-probes.md]]

Rust `rlib` consumed by meow-tower via the shared **BoxcatBridge** cdylib (see `Packages/com.boxcat.libs/Native~/bridge/` in `meow-tower`). Authors Unity Sprite `.asset` files byte-exactly from a TexturePacker `.tpsheet` + `.tps` + atlas `.png`.

## Purpose

C# (`TPSheetPostprocessor`) is the `AssetPostprocessor` entry point — for each imported `.tpsheet` it reads `_prefix` from `TPSImporter` on the sibling `.tps`, picks up PPU from the `.png`'s `TextureImporter`, and calls `BoxcatBridge.SpriteAuthorGenerate(...)`, which dispatches into this rlib's `pipeline::generate`. The pipeline does parse + bytes + filesystem; C# routes the returned written/deleted paths back through `AssetDatabase.ImportAsset`/`DeleteAsset`.

The previous C# `CreateSprites` path was slow (managed), non-deterministic across Unity versions, and re-ran on every reimport. Moving it to native code with a byte-exact contract makes it fast, deterministic, and version-stable.

The bar is **byte-exactness**: output must equal what Unity's `EditorUtility.CopySerialized` emits, byte-for-byte. Existing committed `.asset` files survive the swap with no diff churn — AssetBundle hashes, addressables, `.meta` GUID refs stay stable.

## Goal

- **Input** (path-based): tpsheet path, tps path, atlas `.png` path, output sprite dir, prefix string, PPU.
- **Output**: byte-exact `.asset` + matching `.asset.meta` per sprite. Orphan `.asset`/`.asset.meta` pruned. `.tpsheet` + `.tpsheet.meta` deleted on success.
- **Failure**: all-or-nothing. Nothing written, nothing deleted, error returned to the caller.

## Non-Goals

- Other Unity asset types (`.controller`, `.spriteatlasv2`, etc.). Sprite-only by design.
- Reimplementing Unity's tight-mesh tracer / alpha outline algorithms. Tpsheet always carries verts + tris.
- Standalone CLI, watcher, or scratch-path workflow.
- Cross-Unity-version compat. Target one version at a time; bump explicitly.

## Pipeline

```
TexturePacker → Foo.tps + Foo.tpsheet (in Assets/, alongside Foo.png)
       ↓
Unity import → TPSheetPostprocessor.OnPostprocessAllAssets
       ↓
   For each *.tpsheet: prefix from TPSImporter (.tps), PPU from TextureImporter (.png)
       ↓
   BoxcatBridge.SpriteAuthorGenerate → bxc_sprite_author_generate (cdylib) →
   unity_sprite_author::pipeline::generate (this rlib)
       ↓
   For each written/deleted path: AssetDatabase.ImportAsset / DeleteAsset
       ↓
   (.tpsheet + .tpsheet.meta gone; .asset + .asset.meta written or pruned)
```

## Public Rust API

The only entry point callers use is `pipeline::generate`. The crate has no FFI of its own — the BoxcatBridge cdylib in meow-tower wraps this fn behind `bxc_sprite_author_generate` and handles C# marshalling.

```rust
use std::path::Path;
use unity_sprite_author::pipeline;

let result = pipeline::generate(&pipeline::GenerateInputs {
    tpsheet_path:   Path::new("Assets/Atlas.tpsheet"),
    tps_path:       Path::new("Assets/Atlas.tps"),
    atlas_png_path: Path::new("Assets/Atlas.png"),  // sibling .png.meta read for atlas GUID
    sprite_dir:     Path::new("Assets/Sprites"),    // output dir
    prefix:         "Atlas",                        // "" if none
    ppu:            100.0,                          // from TextureImporter.spritePixelsPerUnit
})?;
// result.written_paths — new/updated sprite .asset paths (caller invokes
//                       AssetDatabase.ImportAsset on each in Unity)
// result.deleted_paths — pruned .asset paths + the consumed .tpsheet(.meta)
```

### Invariants

- **No panics in normal control flow.** `pipeline::generate` returns `Result<GenerateOutput, Error>`. `unwrap`/`expect` reserved for genuine bugs. The bridge wraps the call in `catch_unwind` and surfaces panics as `rc=2` with the panic message.
- **Two-phase commit** (mandatory): Phase 1 collects all `(path, bytes)` pairs in memory — any error here = nothing written. Phase 2 writes each `.asset` + `.asset.meta` to a `.tmp` sibling, then atomic-renames after all temps succeed. Only after every rename succeeds: prune orphans, delete `.tpsheet` + `.tpsheet.meta`. On phase-2 failure: clean up `.tmp` files, leave originals, return error. Pre-existing `.tmp` files from prior crashes are deleted at function entry.
- **Skip-write-if-equal**: before writing, read existing bytes; if identical, skip. Avoids mtime churn that would re-import dependents in Unity.
- **No global state**: no `OnceCell`, `lazy_static`, thread-locals, or caches outliving a `generate` call. Mono does not unload native plugins on domain reload — global state would leak across script recompiles.
- `.tpsheet` + `.tpsheet.meta` deletion happens inside this crate on success only. Both paths are reported in `deleted_paths` so the caller can route them through `AssetDatabase.DeleteAsset`.

### Caller-side notes (Unity / C#)

`StartAssetEditing` is **not** wrapped around the call — Rust writes raw bytes that the editing batch wouldn't observe anyway. `AssetDatabase.Refresh()` is **not** called — too coarse, retriggers postprocessors. The canonical C# integration is `meow-tower`'s `TPSheetPostprocessor.cs` calling `BoxcatBridge.SpriteAuthorGenerate(...)`.

## GUID policy

- For sprite `<name>` in output dir `<dir>`:
  - If `<dir>/<prefix><name>.asset.meta` exists → read existing `guid`, preserve. Always rewrite the file from the canonical 189-byte template; the `guid` field is the only preserved value. Hand-edits to `.asset.meta` are not supported.
  - Else → mint random 128-bit GUID, write a fresh `.asset.meta` from the same template.
- `m_RenderDataKey` in the `.asset` body uses the SAME GUID as the sibling `.asset.meta` (verified against `Cake__DecoLeft.asset.meta` corpus, 3645 files: `m_RenderDataKey` always equals own meta GUID).
- Renames must be done in tpsheet AND Unity at the same time by the developer. This design does not detect renames automatically.

### `.asset.meta` shape

Two trailing-space variants exist in the corpus and one varying field:

- **Legacy189** (older Unity emit): trailing space after `userData:`, `assetBundleName:`, `assetBundleVariant:`. 189 bytes.
- **Modern186** (current Unity emit): no trailing spaces. 186 bytes.
- **`mainObjectFileID`**: usually `21300000` (Sprite class fileID). Some incompletely-imported sprites carry `0` instead.

To avoid byte churn on existing metas the pipeline preserves both axes when present (`meta::detect_shape`). Fresh mints use Modern186 + `21300000`. Schema (Legacy189 form shown):

```
fileFormatVersion: 2
guid: <32-lower-hex>
NativeFormatImporter:
  externalObjects: {}
  mainObjectFileID: 21300000
  userData: 
  assetBundleName: 
  assetBundleVariant: 
```

### Test strategy for GUID determinism

Random-mint conflicts with byte-equal goldens. Strategy: **tests stage the committed `.asset.meta` into the temp input dir before running the pipeline**, so only the preserve branch is exercised by golden tests. The mint branch is covered by a focused unit test using a seeded RNG.

## Reference: tpsheet → Sprite `.asset` field map

tpsheet line format (semicolon-separated):

```
<name>;<x>;<y>;<w>;<h>; <pivotX>;<pivotY>; <bL>;<bR>;<bT>;<bB>;
  <vCount>;<v0x>;<v0y>;...;
  <triCount>;<t0a>;<t0b>;<t0c>;...
```

| Sprite `.asset` field          | Source                                                              |
| ------------------------------ | ------------------------------------------------------------------- |
| `m_Rect`                       | tpsheet rect                                                        |
| `textureRect`                  | always tpsheet rect. If an existing `.asset` carries a divergent `textureRect.{w,h}` (only seen on legacy Tight + `spriteMode: Multiple` outputs), `generate()` returns `Error::TextureRectDivergence` rather than overwriting — delete the stale `.asset` and let Unity re-emit. |
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

Reference asset for parity testing: `meow-tower/Assets/21_Collections/OrgelContents/1204/Orgel/Cake__DecoLeft.asset` (paired with `Orgel.tpsheet`, `Orgel.png`).

## Reference Implementations

C# integration (matched-pair to this crate, in `meow-tower`):
- `Packages/com.boxcat.libs/TexturePacker/TPSheetPostprocessor.cs` — `OnPostprocessAllAssets` entry point; calls `BoxcatBridge.SpriteAuthorGenerate(...)`.
- `Packages/com.boxcat.libs/Native/BoxcatBridge.cs` — `SpriteAuthorGenerate` wrapper + `SpriteAuthorResult` shape; marshals UTF-8 paths and frees the native arena.
- `Packages/com.boxcat.libs/Native~/bridge/src/lib.rs` — `bxc_sprite_author_generate` / `bxc_sprite_author_free` exports wrapping `unity_sprite_author::pipeline::generate`.
- `Packages/com.boxcat.libs/TexturePacker/TPSImporter.cs` — `ScriptedImporter` holding `_prefix` on `.tps`.
- `Packages/com.boxcat.libs/TexturePacker/TexturePackerUtils.cs` — `.tps` parsing for `spriteScale` (and `_prefix` reader for menu items).
- `Packages/com.boxcat.libs/TexturePacker/SheetLoader.cs` — historical tpsheet parser. No longer on the import path; useful as a cross-reference for our parser.

TS port for mesh internals (proven byte-exact for `m_IndexBuffer`, float-tolerant elsewhere):
- `prefab-saloon/src/lib/sprite/tpsheet-parser.ts`
- `prefab-saloon/src/lib/sprite/generator.ts` — port `encodeTypelessData`, `encodeIndexBuffer`, `pixelToLocal`, `pixelToUV`, `alignTo16` verbatim.
- `prefab-saloon/src/lib/sprite/generator.test.ts` — 4 verified mesh fixtures; lift into Rust tests.

Do **not** port: `prefab-saloon/src/lib/prefab/{parser,serializer,templates}.ts`. We don't read `.asset` files, and the YAML emitter must be Unity-flavor specific from day one.

## Known byte-exactness traps (from corpus audit)

- `m_PackingTag: ` and `m_SpriteID: ` end with a literal trailing space before LF.
- File ends `m_SpriteID: \n` with single LF, no trailing blank line.
- `_typelessdata` is one unbroken hex line, never folded.
- `m_RenderDataKey` is the only non-flow nested mapping; everything else is flow `{x: ..., y: ...}`.
- `atlasRectOffset: {x: -1, y: -1}` — Unity's sentinel for non-SpriteAtlas sprites. Constant; applies uniformly to TexturePacker-imported sprites AND to `SpriteFactory.CreateFromMesh` outputs (verified against the Silloutte1 golden). The earlier "fabricated ships (0, 0)" claim was wrong — emit doesn't branch on `SpriteSource` here.
- `m_Border` field order is `{x: L, y: B, z: R, w: T}` per Unity `Sprite.cs`. Verified empirically: 50/51 non-zero-border sprites in the meow-tower corpus emit byte-exactly under the current formula (the lone outlier is .tps drift — golden has all-zero borders, current tpsheet has non-zero). The hard-fail guard was retired once this was proven.
- Float formatting must match C# `ToString("R")`. `yaml::float` uses Rust's default `Display`, which matches across the entire golden corpus (93 distinct fractional literals); the round-trip guard lives in `yaml::tests::float_corpus_full_roundtrip` so a future Display divergence surfaces as a unit failure instead of a golden-byte mismatch.
- `m_AtlasRD == m_RD` for non-SpriteAtlas sprites (verified across the corpus); emit always writes `m_SpriteAtlas: {fileID: 0}` as a constant. The "diverge under SpriteAtlas" hypothesis lives as a deferred probe in [[unity-probes.md#c-m_atlasrd-vs-m_rd-divergence-under-spriteatlas]] — guard wiring waits on a fixture.
- LF line endings; pin via `.gitattributes` (`*.asset binary`, `*.asset.meta binary`).
- `mainObjectFileID: 21300000` in every sprite `.asset.meta` (Unity class ID 213). Constant, not parameterized.

## Tech

- Rust stable. `rlib` only (no binary, no cdylib). The cdylib path was retired when sprite-author was folded into the BoxcatBridge cdylib at `meow-tower/Packages/com.boxcat.libs/Native~/bridge/`.
- `[profile.release] panic = "unwind"` — required by the bridge's outer `catch_unwind`. Inner code returns `Result`; `unwrap`/`expect` reserved for genuine bugs.
- Custom Unity-flavor YAML emitter; no `serde_yaml`. `yaml::float` matches C# `ToString("R")` — table-driven tests in `yaml::tests::float_corpus_full_roundtrip` seeded from every distinct float in the golden corpus.
- Golden-file `assert_eq!` over committed Unity-emitted samples. `.gitattributes` pins `*.asset binary` and `*.asset.meta binary` to prevent CRLF conversion.
- Cross-platform concerns (universal macOS dylib, Windows UCRT linkage) now live in the bridge crate, not here.

## Migration

One-shot rollout shipped (meow-tower commits `0d9143ec…668fd2eb`). The `.tpsheet.meta` `_prefix` was relocated to `.tps.meta` (the new `TPSImporter` `ScriptedImporter`'s home) by `scripts/migrate-tpsheet-meta.sh` — idempotent, `--dry-run` flag, reversible via git. Re-run is safe; skips already-migrated `.tps.meta`.

## Layout

```
unity-sprite-author/
├── src/
│   ├── lib.rs              # module declarations
│   ├── pipeline.rs         # orchestrate: parse → build → write → prune → delete
│   ├── tpsheet.rs          # parser (mirrors SheetLoader.cs)
│   ├── tps.rs              # minimal parser (spriteScale lookup)
│   ├── meta.rs             # .png.meta GUID read; .asset.meta read/write
│   ├── render_data.rs      # _typelessdata, m_IndexBuffer, uvTransform
│   ├── emit.rs             # SpriteAsset → bytes
│   ├── yaml.rs             # Unity-flavor YAML + unity_float_format
│   ├── triangulator.rs     # tight-mesh outline tracer
│   ├── combine.rs          # fab combined-sprite mesh stitching
│   └── fab.rs              # .tps.fab.json sidecar parser
├── tests/
│   ├── golden_parity.rs    # full byte-equality across multiple atlases
│   └── golden/             # committed .tpsheet + .tps + .png.meta + expected .asset
├── scripts/
│   └── migrate-tpsheet-meta.sh  # --dry-run; .tpsheet.meta → .tps.meta
└── CLAUDE.md
```
