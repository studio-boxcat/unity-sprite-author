# unity-sprite-author

> **Related:** [[TODO.md]], [[BENCHMARKS.md]], [[fab.md]]

Native library (`cdylib`) called from Unity's `TPSheetPostprocessor` via P/Invoke. Authors Unity Sprite `.asset` files byte-exactly from a TexturePacker `.tpsheet` + `.tps` + atlas `.png`.

## Purpose

C# (`TPSheetPostprocessor`) is the `AssetPostprocessor` entry point — for each imported `.tpsheet` it reads `_prefix` from `TPSImporter` on the sibling `.tps`, picks up PPU from the `.png`'s `TextureImporter`, and calls into the native lib. The lib does parse + bytes + filesystem; C# routes the returned written/deleted paths back through `AssetDatabase.ImportAsset`/`DeleteAsset`.

The previous C# `CreateSprites` path was slow (managed), non-deterministic across Unity versions, and re-ran on every reimport. Moving it to native code with a byte-exact contract makes it fast, deterministic, and version-stable.

The bar is **byte-exactness**: output must equal what Unity's `EditorUtility.CopySerialized` emits, byte-for-byte. Existing committed `.asset` files survive the swap with no diff churn — AssetBundle hashes, addressables, `.meta` GUID refs stay stable.

## Goal

- **Input** (path-based, via FFI): tpsheet path, tps path, atlas `.png` path, output sprite dir, prefix string, PPU.
- **Output**: byte-exact `.asset` + matching `.asset.meta` per sprite. Orphan `.asset`/`.asset.meta` pruned. `.tpsheet` + `.tpsheet.meta` deleted on success.
- **Failure**: all-or-nothing. Nothing written, nothing deleted, error returned to C# for `Debug.LogError`.

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
   P/Invoke → unity_sprite_author::generate (Rust cdylib)
       ↓
   For each written/deleted path: AssetDatabase.ImportAsset / DeleteAsset
       ↓
   (.tpsheet + .tpsheet.meta gone; .asset + .asset.meta written or pruned)
```

## C# ↔ Rust contract

```c
typedef struct {
    const char* tpsheet_path;     // utf-8, null-terminated
    const char* tps_path;
    const char* atlas_png_path;   // sibling .png.meta read by Rust for atlas GUID
    const char* sprite_dir;       // output dir (e.g. .../Orgel/)
    const char* prefix;           // "" if none
    float       ppu;              // from TextureImporter.spritePixelsPerUnit
} GenerateInputs;

// Output arena. C# reads the path lists, copies strings to managed memory,
// then calls free_output once. Inner pointers invalid after free.
typedef struct {
    const char* const* written_paths;   // length = written_len
    uintptr_t          written_len;
    const char* const* deleted_paths;   // length = deleted_len; INCLUDES the consumed .tpsheet
    uintptr_t          deleted_len;
    void*              _arena;          // opaque; owned by Rust until free_output
} GenerateOutput;

typedef struct {
    int32_t     code;             // 0 = success
    const char* message;          // null on success; freed via free_error
} ErrorOut;

uint32_t abi_version(void);                 // C# asserts on first call; bump on any struct change
int32_t  generate(const GenerateInputs* in, GenerateOutput* out, ErrorOut* err);
void     free_output(GenerateOutput* out);
void     free_error(ErrorOut* err);
```

- Rust allocates output buffers via the opaque `_arena`; C# calls `free_output` exactly once after copying all strings to managed memory. C# wraps the call in an `IDisposable` `SafeHandle`-style struct so `free_output` runs even on exceptions.
- Outermost `extern "C"` body is exactly `catch_unwind(AssertUnwindSafe(|| inner_generate(...)))`. `inner_generate` returns `Result<_, Error>`; never panics in normal control flow. `unwrap`/`expect` are bugs.
- **Two-phase commit** (mandatory): Phase 1 collects all `(path, bytes)` pairs in memory — any error here = nothing written. Phase 2 writes each `.asset` + `.asset.meta` to a `.tmp` sibling, then atomic-renames after all temps succeed. Only after every rename succeeds: prune orphans, delete `.tpsheet` + `.tpsheet.meta`. On phase-2 failure: clean up `.tmp` files, leave originals, return error. Pre-existing `.tmp` files from prior crashes are deleted at function entry.
- **Skip-write-if-equal**: before writing, read existing bytes; if identical, skip. Avoids mtime churn that would re-import dependents in Unity.
- **No global state**: no `OnceCell`, `lazy_static`, thread-locals, or caches outliving a `generate` call. Mono does not unload native plugins on domain reload — global state would leak across script recompiles.
- `.tpsheet` + `.tpsheet.meta` deletion happens inside Rust on success only. Both paths are reported in `deleted_paths` so C# can call `AssetDatabase.DeleteAsset` on them.
- Native binary lives at `meow-tower/Assets/50_Modules/Tools/TexturePacker/Editor/{libunity_sprite_author.dylib,unity_sprite_author.dll}`. The `Editor/` subfolder makes Unity treat it as editor-only automatically, no manual plugin-import flags required.

C# call pattern (canonical):

```csharp
using var output = NativeSpriteAuthor.Generate(inputs);  // throws on error
foreach (var path in output.WrittenPaths)
    AssetDatabase.ImportAsset(path, ImportAssetOptions.ForceUpdate);
foreach (var path in output.DeletedPaths)
    AssetDatabase.DeleteAsset(path);
```

`StartAssetEditing` is **not** wrapped around this — Rust writes raw bytes that the editing batch wouldn't observe anyway. `AssetDatabase.Refresh()` is **not** called — too coarse, retriggers postprocessors.

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
| `settingsRaw`                  | constant `192` (0xC0). Panic if a future fixture diverges.          |
| `texture` GUID                 | from atlas `.png.meta`                                              |
| `m_RenderDataKey` GUID         | own `.asset.meta` GUID (preserve or mint per policy above)          |

Reference asset for parity testing: `meow-tower/Assets/21_Collections/OrgelContents/1204/Orgel/Cake__DecoLeft.asset` (paired with `Orgel.tpsheet`, `Orgel.png`).

## Reference Implementations

C# integration (matched-pair to this crate):
- `meow-tower/Assets/50_Modules/Tools/TexturePacker/TPSheetPostprocessor.cs` — `OnPostprocessAllAssets` entry point.
- `meow-tower/Assets/50_Modules/Tools/TexturePacker/NativeSpriteAuthor.cs` — P/Invoke wrapper; mirror struct-for-struct with `src/ffi.rs`.
- `meow-tower/Assets/50_Modules/Tools/TexturePacker/TPSImporter.cs` — `ScriptedImporter` holding `_prefix` on `.tps`.
- `meow-tower/Assets/50_Modules/Tools/TexturePacker/TexturePackerUtils.cs` — `.tps` parsing for `spriteScale` (and `_prefix` reader for menu items).
- `meow-tower/Assets/50_Modules/Tools/TexturePacker/SheetLoader.cs` — historical tpsheet parser. No longer on the import path; useful as a cross-reference for our parser.

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
- `atlasRectOffset: {x: -1, y: -1}` — that `-1` is a Unity default, not zero.
- `m_Border` field order is `{x: L, y: B, z: R, w: T}` per Unity `Sprite.cs`. Verified empirically: 50/51 non-zero-border sprites in the meow-tower corpus emit byte-exactly under the current formula (the lone outlier is .tps drift — golden has all-zero borders, current tpsheet has non-zero). The hard-fail guard was retired once this was proven.
- Float formatting must match C# `ToString("R")`. Build a `unity_float_format` with a unit-test table seeded from every distinct float in the golden corpus before milestone-3.
- `m_AtlasRD == m_RD` only valid for non-SpriteAtlas sprites — guard with hard panic on `m_SpriteAtlas != {fileID:0}`.
- LF line endings; pin via `.gitattributes` (`*.asset binary`, `*.asset.meta binary`).
- `mainObjectFileID: 21300000` in every sprite `.asset.meta` (Unity class ID 213). Constant, not parameterized.
- `atlasRectOffset: {x: -1, y: -1}` — Unity default; not zero.

## Tech

- Rust stable. `cdylib` only, no binary.
- Cross-compile targets:
  - `aarch64-apple-darwin` + `x86_64-apple-darwin` via `cargo zigbuild`, combined into a universal dylib via `lipo`.
  - `x86_64-pc-windows-gnu` via `cargo zigbuild`. Despite the `-gnu` triple, the resulting DLL imports the `api-ms-win-crt-*` API sets — i.e. UCRT, the same CRT MSVC has shipped against since VS 2015 — so it's ABI-compatible with Unity Editor on Windows 10+. Win7/8.1 are out of scope (not supported by current Unity LTS). Verify CRT linkage post-build with `strings unity_sprite_author.dll | grep -i 'api-ms-win-crt'` (no `msvcrt.dll`, no `vcruntime`).
- `[profile.release] panic = "unwind"` (required by `catch_unwind`). The outermost extern fn is the only `catch_unwind` site; inner code returns `Result`. `unwrap`/`expect` reserved for genuine bugs.
- Custom Unity-flavor YAML emitter; no `serde_yaml`. `unity_float_format` matches C# `ToString("R")` — table-driven tests seeded from every distinct float in the golden corpus.
- Golden-file `assert_eq!` over committed Unity-emitted samples. `.gitattributes` pins `*.asset binary` and `*.asset.meta binary` to prevent CRLF conversion.
- Symbol-export sanity check (CI/local): `nm -gU target/release/libunity_sprite_author.dylib | grep -E '^_(generate|free_output|free_error|abi_version)$'`.
- macOS post-build: `codesign --sign - --force --timestamp=none <dylib>` (ad-hoc).
- Single-programmer team. Native binary built locally and committed to `Assets/Plugins/Editor/`. Windows porting team pulls the committed `.dll`; only the maintainer needs Rust toolchain.

### Universal macOS build recipe

```sh
cargo zigbuild --release --target aarch64-apple-darwin
cargo zigbuild --release --target x86_64-apple-darwin
lipo -create \
  target/aarch64-apple-darwin/release/libunity_sprite_author.dylib \
  target/x86_64-apple-darwin/release/libunity_sprite_author.dylib \
  -output Assets/Plugins/Editor/libunity_sprite_author.dylib
codesign --sign - --force --timestamp=none Assets/Plugins/Editor/libunity_sprite_author.dylib
```

## Migration

One-shot rollout shipped (meow-tower commits `0d9143ec…668fd2eb`). The `.tpsheet.meta` `_prefix` was relocated to `.tps.meta` (the new `TPSImporter` `ScriptedImporter`'s home) by `scripts/migrate-tpsheet-meta.sh` — idempotent, `--dry-run` flag, reversible via git. Re-run is safe; skips already-migrated `.tps.meta`.

## Layout

```
unity-sprite-author/
├── src/
│   ├── lib.rs              # FFI exports, panic catch, memory ownership
│   ├── ffi.rs              # extern "C" types, conversions, free_*
│   ├── pipeline.rs         # orchestrate: parse → build → write → prune → delete
│   ├── tpsheet.rs          # parser (mirrors SheetLoader.cs)
│   ├── tps.rs              # minimal parser (spriteScale lookup)
│   ├── meta.rs             # .png.meta GUID read; .asset.meta read/write
│   ├── geometry.rs         # SpriteGeometry from tpsheet (+ rect fallback)
│   ├── render_data.rs      # _typelessdata, m_IndexBuffer, uvTransform
│   ├── sprite.rs           # SpriteAsset value type
│   ├── emit.rs             # SpriteAsset → bytes
│   ├── yaml.rs             # Unity-flavor YAML + unity_float_format
│   └── guid.rs             # Unity GUID generate/format/parse
├── tests/
│   ├── golden_parity.rs    # full byte-equality across multiple atlases
│   └── golden/             # committed .tpsheet + .tps + .png.meta + expected .asset
├── scripts/
│   └── migrate-tpsheet-meta.sh  # --dry-run; .tpsheet.meta → .tps.meta
└── CLAUDE.md
```
