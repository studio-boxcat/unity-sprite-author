# unity-sprite-author

Native library (`cdylib`) called from Unity's `TPSheetPostprocessor` via P/Invoke. Replaces the C# sprite-asset generation path with byte-exact output. Legacy subasset path stays untouched.

## Purpose

Replace the body of `CreateSprites(...)` in `TPSheetPostprocessor.cs` with a Rust `cdylib` that authors `.asset` files byte-exactly. C# remains the `AssetPostprocessor` entry point — it gates the legacy/new branch, reconfigures the texture importer, reads `_prefix` from `TPSheetImporter`, calls the native lib, and refreshes AssetDatabase. The lib does parse + bytes + filesystem.

Today's `CreateSprites` is slow (C# managed), non-deterministic across Unity versions, and re-runs on every reimport. Moving it to native code with a byte-exact contract makes it fast, deterministic, and version-stable.

The bar is **byte-exactness**: output must equal what Unity's `EditorUtility.CopySerialized` emits today, byte-for-byte. Existing committed `.asset` files survive the swap with no diff churn — AssetBundle hashes, addressables, `.meta` GUID refs stay stable.

## Goal

Drop-in replacement for `CreateSprites(...)`:

- **Input** (path-based, via FFI): tpsheet path, tps path, atlas `.png` path, output sprite dir, prefix string, PPU.
- **Output**: byte-exact `.asset` + matching `.asset.meta` per sprite. Orphan `.asset`/`.asset.meta` pruned. `.tpsheet` + `.tpsheet.meta` deleted on success.
- **Failure**: all-or-nothing. Nothing written, nothing deleted, error returned to C# for `Debug.LogError`.

## Non-Goals

- Legacy path (`textureType == Sprite && spriteImportMode == Multiple`). Frozen C#, future-stripped.
- Other Unity asset types (`.controller`, `.spriteatlasv2`, etc.). Sprite-only by design.
- Reimplementing Unity's tight-mesh tracer / alpha outline algorithms. Tpsheet always carries verts + tris.
- Standalone CLI, watcher, or scratch-path workflow.
- Cross-Unity-version compat. Target one version at a time; bump explicitly.

## Pipeline

```
TexturePacker → Foo.tps + Foo.tpsheet (in Assets/, alongside Foo.png)
       ↓
Unity import → TPSheetPostprocessor (C#)
       ↓
   IsLegacyTexture? ── yes ──► frozen C# legacy path (subassets in .png.meta)
       ↓ no
   ConfigureTextureImporter (textureType=Default, alphaIsTransparency, mip=off)
       ↓
   P/Invoke → unity_sprite_author::generate (Rust cdylib)
       ↓
   For each written/deleted path: AssetDatabase.ImportAsset / DeleteAsset
       ↓
   (.tpsheet + .tpsheet.meta gone; .asset + .asset.meta written or pruned)
```

Branch signal (verified, `TPSheetPostprocessor.cs:14-15`):

```csharp
private static bool IsLegacyTexture(TextureImporter ti) =>
    ti.textureType is TextureImporterType.Sprite && ti.spriteImportMode is SpriteImportMode.Multiple;
```

Default texture type for fresh PNGs falls into the new path.

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
- Native binary lives at `Assets/Plugins/Editor/libunity_sprite_author.{dylib,dll}`. Editor-only, never shipped in builds. Hand-author the `.meta` plugin-import flags (`Editor: 1`, all platforms `0`) — Unity's auto-detection defaults to "Any Platform" and bundles the dylib into player builds.

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

### `.asset.meta` canonical template

Verified across 3645 sprite `.asset.meta` files in `meow-tower/Assets/21_Collections/`: every file is exactly **189 bytes**, schema-identical, only `guid` varies. Template (LF endings, trailing space after each empty value, single trailing LF):

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

`mainObjectFileID: 21300000` is constant for sprites (Unity class ID 213). Pin as compile-time constant; do not parameterize.

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
| `m_Rect`, `textureRect`        | tpsheet rect                                                        |
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

C# spec (authoritative):
- `meow-tower/Assets/50_Modules/Tools/TexturePacker/TPSheetPostprocessor.cs` — replaceable boundary is `CreateSprites` (lines 108-172).
- `meow-tower/Assets/50_Modules/Tools/TexturePacker/TPSheetImporter.cs` — holds `_prefix`.
- `meow-tower/Assets/50_Modules/Tools/TexturePacker/SheetLoader.cs` — tpsheet parsing, including no-polygon rect fallback.
- `meow-tower/Assets/50_Modules/Tools/TexturePacker/TexturePackerUtils.cs` — `.tps` parsing for `spriteScale`.

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
- `m_Border` field order is `{x: L, y: B, z: R, w: T}` per Unity `Sprite.cs`. No non-zero-border fixture exists in the corpus; need to author one through Unity to validate.
- Float formatting must match C# `ToString("R")`. Build a `unity_float_format` with a unit-test table seeded from every distinct float in the golden corpus before milestone-3.
- `m_AtlasRD == m_RD` only valid for non-SpriteAtlas sprites — guard with hard panic on `m_SpriteAtlas != {fileID:0}`.
- LF line endings; pin via `.gitattributes` (`*.asset binary`, `*.asset.meta binary`).
- `mainObjectFileID: 21300000` in every sprite `.asset.meta` (Unity class ID 213). Constant, not parameterized.
- `atlasRectOffset: {x: -1, y: -1}` — Unity default; not zero.

## Tech

- Rust stable. `cdylib` only, no binary.
- Cross-compile targets:
  - `aarch64-apple-darwin` + `x86_64-apple-darwin` via `cargo zigbuild`, combined into a universal dylib via `lipo`.
  - `x86_64-pc-windows-msvc` via `cargo xwin` (NOT `-gnu`; Unity Editor on Windows is MSVC-CRT, mixing CRTs across the FFI boundary corrupts the heap when ownership crosses).
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

Shell script with `--dry-run`. Walks each `.tpsheet.meta` with non-empty `_prefix`, copies the value to the corresponding `.tps.meta` (consumed by the new `ScriptedImporter` on `.tps`). Idempotent; reversible via git.

## Layout (planned)

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
