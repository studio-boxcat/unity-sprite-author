# unity-sprite-author

> **Related:** [[TODO.md]], [[BENCHMARKS.md]], [[fab.md]], [[sma-migration.md]], [[unity-probes.md]]

Rust `rlib` consumed by meow-tower via the shared **BoxcatBridge** cdylib (see `Packages/com.boxcat.libs/Native~/bridge/` in `meow-tower`). Authors Unity Sprite `.asset` files byte-exactly from a TexturePacker `.tpsheet` + `.tps` + atlas `.png`.

## Purpose

A `ScriptedImporter` (`TPSheetImporter`) imports `.tpsheet` files and dispatches into `pipeline::generate` via the BoxcatBridge FFI. The pipeline does parse + bytes + filesystem; Unity reimports the written `.asset` files automatically. Prefix comes from `TPSheetImporter._prefix` on the sibling `.tpsheet.meta`; PPU from the `.png.meta` `TextureImporter`.

The bar is **byte-exactness**: output must equal what Unity's `EditorUtility.CopySerialized` emits, byte-for-byte. Existing committed `.asset` files survive the swap with no diff churn — AssetBundle hashes, addressables, `.meta` GUID refs stay stable. The previous managed C# `CreateSprites` path was slow, version-unstable, and re-ran on every reimport; the native port fixes all three.

## Goal

- **Input** (path-based): tpsheet path, tps path, atlas `.png` path, output sprite dir, prefix string, PPU.
- **Output**: byte-exact `.asset` + matching `.asset.meta` per sprite. Orphan `.asset`/`.asset.meta` pruned. `.tpsheet` retained on disk (TexturePacker uses it for `smartUpdateKey` hash checks).
- **Failure**: all-or-nothing. Nothing written, nothing deleted, error returned to the caller.

## Non-Goals

- Other Unity asset types (`.controller`, `.spriteatlasv2`, etc.). Sprite-only by design.
- Reimplementing Unity's tight-mesh tracer / alpha outline algorithms. Tpsheet always carries verts + tris.
- Daemon / long-running service. The `TPSheetImporter` is event-driven (Unity import), not a poller.
- Cross-Unity-version compat. Target one version at a time; bump explicitly.

## Pipeline

```
TexturePacker → Foo.tps + Foo.tpsheet + Foo.png (in Assets/)
       ↓
Unity import → TPSheetImporter.OnImportAsset
       ↓
BoxcatBridge.SpriteAuthorGenerate → bxc_sprite_author_generate (cdylib)
       ↓
pipeline::generate (this rlib) — .tpsheet retained, .asset written/pruned
       ↓
(SmartUpdate hash check skips redundant runs)
```

## Public Rust API

The primary entry point is `pipeline::generate`. The crate has no FFI of its own — the BoxcatBridge cdylib in meow-tower wraps this fn behind `bxc_sprite_author_generate` and handles C# marshalling. `pipeline::StandardLayout::from_tpsheet` / `from_tps` is a small helper for callers (the bridge and the CLI) that need to derive `tpsheet` / `tps` / `png` / `sprite_dir` from a single stem path under the standard `<parent>/<stem>.ext` convention. `meta::read_tps_prefix` is the parallel helper for sourcing the `_prefix` default from a `.tps.meta` ScriptedImporter block — shared so the bridge and CLI agree on the on-disk shape.

```rust
use std::path::Path;
use unity_sprite_author::pipeline;

let result = pipeline::generate(&pipeline::GenerateInputs {
    tpsheet_path:   Path::new("Assets/Atlas.tpsheet"),
    tps_path:       Path::new("Assets/Atlas.tps"),
    atlas_png_path: Path::new("Assets/Atlas.png"),  // sibling .png.meta read for atlas GUID + `alphaIsTransparency` sync
    sprite_dir:     Path::new("Assets/Sprites"),    // output dir
    prefix:         "Atlas",                        // "" if none
    ppu:            100.0,                          // from TextureImporter.spritePixelsPerUnit
})?;
// result.written_paths — new/updated sprite .asset paths, plus the atlas
//                       .png when alphaIsTransparency was rewritten
//                       (caller invokes AssetDatabase.ImportAsset on each)
// result.deleted_paths — pruned .asset paths
```

For non-pipeline consumers (the GUI editor today), `combine::build_combined_with_ranges` returns the same `CombinedMesh` plus per-part `[start, end)` index ranges into the merged vertex arrays — useful for picking, outlining, and vertex-color overrides without re-running the build per part. `build_combined` is a one-line wrapper that discards the ranges.

### Invariants

- **No panics in normal control flow.** `pipeline::generate` returns `Result<GenerateOutput, Error>`. `unwrap`/`expect` reserved for genuine bugs. The bridge wraps the call in `catch_unwind` and surfaces panics as `rc=2` with the panic message.
- **Two-phase commit** (mandatory): Phase 1 collects all `(path, bytes)` pairs in memory — any error here = nothing written. Phase 2 writes each `.asset` + `.asset.meta` to a `.tmp` sibling, then atomic-renames after all temps succeed. Only after every rename succeeds: prune orphans. On phase-2 failure: clean up `.tmp` files, leave originals, return error. Pre-existing `.tmp` files from prior crashes are deleted at function entry.
- **Skip-write-if-equal**: before writing, read existing bytes; if identical, skip. Avoids mtime churn that would re-import dependents in Unity.
- **No global state**: no `OnceCell`, `lazy_static`, thread-locals, or caches outliving a `generate` call. Mono does not unload native plugins on domain reload — global state would leak across script recompiles.
- `.tpsheet` is retained on disk after a successful run — TexturePacker uses it for its `smartUpdateKey` hash check (skips redundant `.png` rewrites on next publish).
- Atlas `.png.meta` `alphaIsTransparency` is kept in lockstep with the tpsheet's `alphahandling` header: `PremultiplyAlpha` / `KeepTransparentPixels` → `0` (premultiplied); anything else → `1` (Unity straight-alpha default). Surgical line rewrite — only the value flips. When a rewrite happens, the atlas `.png` path is appended to `written_paths` so the caller's `AssetDatabase.ImportAsset` retriggers `TextureImporter` (a `.meta`-only touch isn't always enough). Missing field → `Error::Meta(NoAlphaIsTransparencyField)`.

### CLI

`unity-sprite-author <atlas.tps>` packs the `.tps` with TexturePackerCLI (shells out to `texturepacker`), mints any missing `.tps.meta` / `.png.meta` via the `unity-assetdb` `register` API, then runs `pipeline::generate`. Intended for one-shot regens outside the Unity Editor — Unity must NOT be running, same caveat as `scripts/regen-corpus.sh`.

```sh
just install                                  # builds release, symlinks to ~/.local/bin/
unity-sprite-author Atlas.tps                 # pack + author; prefix/ppu read from existing metas
unity-sprite-author Atlas.tps --prefix AC_ --ppu 100 --sprite-dir Atlas
unity-sprite-author Atlas.tps --skip-pack     # reuse existing .tpsheet/.png
```

`--prefix` / `--ppu` override the meta-derived defaults. PPU is required (CLI flag or `spritePixelsToUnits` in `.png.meta`); prefix defaults to `""` when neither source supplies one. Default `--sprite-dir` follows `pipeline::StandardLayout` (`<tps-parent>/<tps-stem>/`), shared with the meow-tower bridge. `.tps.fab.json` / `.tps.mesh.json` sidecars are picked up automatically since the bin just forwards `tps_path` to `pipeline::generate` — no extra flags.

### Caller-side notes (Unity / C#)

`StartAssetEditing` is **not** wrapped around the call — Rust writes raw bytes that the editing batch wouldn't observe anyway. The canonical C# integration is `TPSheetImporter` (`ScriptedImporter`) calling `BoxcatBridge.SpriteAuthorGenerate` on `.tpsheet` import.

## GUID policy

- For sprite `<name>` in output dir `<dir>`:
  - If `<dir>/<prefix><name>.asset.meta` exists → read existing `guid`, preserve. Detect the file's shape (Legacy189 vs Modern186, `mainObjectFileID` value) and rewrite in the same shape so on-disk bytes don't churn just because we touched the file. The `guid` and detected shape are the only preserved values. Hand-edits to other fields are not supported.
  - Else → mint random 128-bit GUID, write a fresh `.asset.meta` in the Modern186 shape with `mainObjectFileID: 21300000`.
- `m_RenderDataKey` in the `.asset` body uses the SAME GUID as the sibling `.asset.meta` (verified against `Cake__DecoLeft.asset.meta` corpus, 3645 files: `m_RenderDataKey` always equals own meta GUID).
- Renames must be done in tpsheet AND Unity at the same time by the developer. This design does not detect renames automatically.

### `.asset.meta` shape

Two trailing-space variants exist in the corpus and one varying field:

- **Legacy189** (older Unity emit): trailing space after `userData:`, `assetBundleName:`, `assetBundleVariant:`. 189 bytes.
- **Modern186** (current Unity emit): no trailing spaces. 186 bytes.
- **`mainObjectFileID`**: usually `21300000` (Sprite class fileID). Some incompletely-imported sprites carry `0` instead.

To avoid byte churn on existing metas the pipeline preserves both axes when present (`meta::detect_shape`). Fresh mints use Modern186 + `21300000`. Golden tests stage the committed `.asset.meta` before invoking the pipeline so the preserve branch is exercised; the mint branch is unit-tested via `mint_guid_from(lo, hi)` with fixed entropy (no `rand` crate).

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

Reference fixture for parity testing: `tests/golden/orgel/` — a self-contained snapshot of `Orgel.{tpsheet, tps, png.meta}` + the per-sprite `.asset` / `.asset.meta` corpus (`Cake__DecoLeft.asset` is the canonical example). The matching meow-tower-side files live under `meow-tower/Assets/21_Collections/OrgelContents/1204/Orgel/`, but the `.tpsheet` there is ephemeral — `pipeline::generate` deletes it on success, so it's only present mid-import.

## Reference Implementations

C# integration in `meow-tower` lives under `Packages/com.boxcat.libs/{TexturePacker,Native,Native~/bridge}/`. Entry chain: `TPSheetImporter.OnImportAsset` → `BoxcatBridge.SpriteAuthorGenerate` → `bxc_sprite_author_generate` (cdylib) → `pipeline::generate`. `TPSheetImporter` (`ScriptedImporter`) holds the `_prefix` on `.tpsheet`.

TS prior art at `prefab-saloon/src/lib/sprite/` (`tpsheet-parser.ts`, `generator.ts`) was the byte-exact reference for `m_IndexBuffer` + mesh encoding during the initial port; goldens have since taken over that role. The sibling `prefab-saloon/src/lib/prefab/{parser,serializer,templates}.ts` is intentionally *not* ported — we don't read `.asset` files and the YAML emit must be Unity-flavor specific from day one.

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

- Rust stable. Primary artifact is the `rlib` consumed by BoxcatBridge (`meow-tower/Packages/com.boxcat.libs/Native~/bridge/`); a thin `unity-sprite-author` bin (see [[#cli]]) shares the same rlib for offline packs. No cdylib lives here — that path was retired when sprite-author was folded into the BoxcatBridge cdylib.
- `[profile.release] panic = "unwind"` — required by the bridge's outer `catch_unwind`. Inner code returns `Result`; `unwrap`/`expect` reserved for genuine bugs.
- Custom Unity-flavor YAML emitter; no `serde_yaml`. `yaml::float` matches C# `ToString("R")` — table-driven tests in `yaml::tests::float_corpus_full_roundtrip` seeded from every distinct float in the golden corpus.
- Golden-file `assert_eq!` over committed Unity-emitted samples. `.gitattributes` pins `*.asset binary` and `*.asset.meta binary` to prevent CRLF conversion.
- Cross-platform concerns (universal macOS dylib, Windows UCRT linkage) now live in the bridge crate, not here.

## Layout

Cargo workspace, three crates:

- **`crates/core/`** — the rlib (`unity-sprite-author` package). Consumed by the BoxcatBridge cdylib in meow-tower (`Native~/bridge/` points its `path = "..."` here). Module-level orientation lives in `src/lib.rs`; key entry points are `pipeline::generate`, `manifest::parse` (unified CSA+SMA tree), `combine::build_combined`, `mesh_emit::build_mesh`, and `emit::SpriteAsset`. Golden tests under `tests/golden/{orgel,fab,sma}/`; opt-in `e2e_meow_tower.rs` walks a meow-tower checkout.
- **`crates/cli/`** — offline `unity-sprite-author` bin. Shells out to TexturePackerCLI, threads the result into `core`. Pre-pack step synthesizes missing 1×1 `Color_*.png` swatches into the .tps source dir (`color_png` + `color_synth` modules; .tps DOM via the `tps-core` crate from meow-toolbox). See [[#cli]].
- **`crates/editor/`** — GUI tool (`eframe` + `egui`, native macOS menubar via `muda`) for authoring `.tps.fab.json`. Every user-facing command flows through one `Action` enum in `action.rs` — extend `Action`, add a `match` arm in `App::dispatch`, optionally register a `CommandEntry` for palette discoverability. Editor and CLI never leak into the rlib.

Supporting: `docs/` (fab schema, SMA migration map, Unity-Editor probe runbooks), `scripts/` (corpus regen, one-shot `.tpsheet.meta` → `.tps.meta` migration), `justfile` (`just install` → `~/.local/bin/`).
