# unity-sprite-author

> **Related:** [[TODO.md]], [[BENCHMARKS.md]], [[fab.md]], [[byte-format.md]]

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
Foo.tpsheet changes → Unity import → TPSheetImporter.OnImportAsset
       ↓
BoxcatBridge.SpriteAuthorGenerate → bxc_sprite_author_generate (cdylib)
       ↓
pipeline::generate (this rlib) — .tpsheet retained, .asset written/pruned
```

TexturePacker emits `Foo.tps` + `Foo.tpsheet` + `Foo.png` into `Assets/`. Unity reacts only to `.tpsheet` writes (via `TPSheetImporter`); the upstream `.tps` / source-folder → repack step is driven separately (the `Texture Packer / Pack` Editor menu, or the offline `unity-sprite-author <atlas.tps>` one-shot), which produces a fresh `.tpsheet` the importer then picks up.

## Public Rust API

The primary entry point is `pipeline::generate`. The crate has no FFI of its own — the BoxcatBridge cdylib in meow-tower wraps this fn behind `bxc_sprite_author_generate` and handles C# marshalling. `pipeline::StandardLayout::from_tpsheet` / `from_tps` is a small helper for callers (the bridge and the CLI) that need to derive `tpsheet` / `tps` / `png` / `sprite_dir` from a single stem path under the standard `<parent>/<stem>.ext` convention. `meta::read_tps_prefix` is the parallel helper for sourcing the `_prefix` default from the `.tpsheet.meta` (`TPSheetImporter`) block — shared so the bridge and CLI agree on the on-disk shape.

```rust
use std::path::Path;
use unity_sprite_author::pipeline;

let result = pipeline::generate(&pipeline::GenerateInputs {
    tpsheet_path:   Path::new("Assets/Atlas.tpsheet"),
    tps_path:       Path::new("Assets/Atlas.tps"),
    atlas_png_path: Path::new("Assets/Atlas.png"),  // sibling .png.meta read for atlas GUID + `alphaIsTransparency` sync
    sprite_dir:     Path::new("Assets/Sprites"),    // output dir
    prefix:         "Atlas",                        // "" if none
})?;  // PPU is read from <atlas>.png.meta `spriteScale`/importer inside generate()
// result.written_paths — new/updated sprite .asset paths, plus the atlas
//                       .png when alphaIsTransparency was rewritten
//                       (caller invokes AssetDatabase.ImportAsset on each)
// result.deleted_paths — pruned .asset paths
```

`pipeline::build` is `generate`'s phase 1 without the commit: it returns a `BuildPlan` (the in-memory `(path, bytes)` writes — final combined sprites/meshes included — plus the would-be `deleted_paths` and `warnings`) and touches nothing on disk. Used to diff the would-be output against committed goldens (the `e2e_meow_tower` corpus test drives it) or to dry-run; `generate` runs the same compute then commits.

For non-pipeline consumers (the GUI editor today), `combine::build_combined_with_ranges` returns the same `CombinedMesh` plus per-part `[start, end)` index ranges into the merged vertex arrays — useful for picking, outlining, and vertex-color overrides without re-running the build per part. `build_combined` is a one-line wrapper that discards the ranges.

### Invariants

- **No panics in normal control flow.** `pipeline::generate` returns `Result<GenerateOutput, Error>`. `unwrap`/`expect` reserved for genuine bugs. The bridge wraps the call in `catch_unwind` and surfaces panics as `rc=2` with the panic message.
- **Two-phase commit** (mandatory): Phase 1 collects all `(path, bytes)` pairs in memory — any error here = nothing written. Phase 2 writes each `.asset` + `.asset.meta` to a `.tmp` sibling, then atomic-renames after all temps succeed. Only after every rename succeeds: prune orphans. On phase-2 failure: clean up `.tmp` files, leave originals, return error. Pre-existing `.tmp` files from prior crashes are deleted at function entry.
- **Skip-write-if-equal**: before writing, read existing bytes; if identical, skip. Avoids mtime churn that would re-import dependents in Unity.
- **No global state**: no `OnceCell`, `lazy_static`, thread-locals, or caches outliving a `generate` call. Mono does not unload native plugins on domain reload — global state would leak across script recompiles.
- `.tpsheet` is retained on disk after a successful run — TexturePacker uses it for its `smartUpdateKey` hash check (skips redundant `.png` rewrites on next publish).
- **SmartUpdate `.hash` skip**: TexturePacker embeds a `$TexturePacker:SmartUpdate:<key>$` line in the `.tpsheet` header. `generate` caches the last-processed key in `<sprite_dir>/.hash` and short-circuits to an empty `GenerateOutput` when it still matches. So a redundant `generate` on an unchanged `.tpsheet` is a cheap no-op — relevant whenever more than one author can touch the same `.tpsheet` (e.g. an external repack-and-author plus Unity's `TPSheetImporter`). Best-effort write; a missing / unwritable `.hash` just costs the next run a full pass. (`.hash` is not a `.asset`, so orphan pruning never touches it.)
- Atlas `.png.meta` `alphaIsTransparency` is kept in lockstep with the tpsheet's `alphahandling` header: `PremultiplyAlpha` / `KeepTransparentPixels` → `0` (premultiplied); anything else → `1` (Unity straight-alpha default). Surgical line rewrite — only the value flips. When a rewrite happens, the atlas `.png` path is appended to `written_paths` so the caller's `AssetDatabase.ImportAsset` retriggers `TextureImporter` (a `.meta`-only touch isn't always enough). Missing field → `Error::Meta(NoAlphaIsTransparencyField)`.

### CLI

`unity-sprite-author <atlas.tps>` packs the `.tps` with TexturePackerCLI (at the platform install path — `crates/pack`'s `TEXTUREPACKER_CMD`, the single source shared by CLI / editor / bridge; no env/flag override), mints any missing `.tps.meta` / `.png.meta` via the `unity-assetdb` `register` API, then runs `pipeline::generate`. Intended for one-shot regens outside the Unity Editor — Unity must NOT be running, same caveat as `scripts/regen-corpus.sh`.

```sh
just install                                  # builds release, symlinks to ~/.local/bin/
unity-sprite-author Atlas.tps                 # pack + author; prefix/ppu read from existing metas
unity-sprite-author Atlas.tps --prefix AC_ --sprite-dir Atlas
unity-sprite-author Atlas.tps --skip-pack     # reuse existing .tpsheet/.png
```

`--prefix` overrides the meta-derived default (`_prefix` from the `.tpsheet.meta`, else `""`). PPU is read from `<atlas>.png.meta` inside `generate` — there is no `--ppu` flag. Default `--sprite-dir` follows `pipeline::StandardLayout` (`<tps-parent>/<tps-stem>/`), shared with the meow-tower bridge. `.tps.fab.json` / `.tps.mesh.json` sidecars are picked up automatically since the bin just forwards `tps_path` to `pipeline::generate` — no extra flags.

### Caller-side notes (Unity / C#)

`StartAssetEditing` is **not** wrapped around the call — Rust writes raw bytes that the editing batch wouldn't observe anyway. The canonical C# integration is `TPSheetImporter` (`ScriptedImporter`) calling `BoxcatBridge.SpriteAuthorGenerate` on `.tpsheet` import.

## Byte format

The byte-exactness reference — GUID policy, the two `.asset.meta` shape variants, the tpsheet→`.asset` field map, and the corpus-audit traps — lives in [[byte-format.md]]. Parity fixture: `tests/golden/orgel/` (`Cake__DecoLeft.asset` is the canonical example).

## Reference Implementations

C# integration in `meow-tower` lives under `Packages/com.boxcat.libs/{TexturePacker,Native,Native~/bridge}/`. Entry chain: `TPSheetImporter.OnImportAsset` → `BoxcatBridge.SpriteAuthorGenerate` → `bxc_sprite_author_generate` (cdylib) → `pipeline::generate`. `TPSheetImporter` (`ScriptedImporter`) holds the `_prefix` on `.tpsheet`.

TS prior art at `prefab-saloon/src/lib/sprite/` (`tpsheet-parser.ts`, `generator.ts`) was the byte-exact reference for `m_IndexBuffer` + mesh encoding during the initial port; goldens have since taken over that role. The sibling `prefab-saloon/src/lib/prefab/{parser,serializer,templates}.ts` is intentionally *not* ported — we don't read `.asset` files and the YAML emit must be Unity-flavor specific from day one.

## Tech

- Rust stable. Primary artifact is the `rlib` consumed by BoxcatBridge (`meow-tower/Packages/com.boxcat.libs/Native~/bridge/`); a thin `unity-sprite-author` bin (see [[#cli]]) shares the same rlib for offline packs. No cdylib lives here — that path was retired when sprite-author was folded into the BoxcatBridge cdylib.
- `[profile.release] panic = "unwind"` — required by the bridge's outer `catch_unwind`. Inner code returns `Result`; `unwrap`/`expect` reserved for genuine bugs.
- Custom Unity-flavor YAML emitter; no `serde_yaml`. `yaml::float` matches C# `ToString("R")` — table-driven tests in `yaml::tests::float_corpus_full_roundtrip` seeded from every distinct float in the golden corpus.
- Golden-file `assert_eq!` over committed Unity-emitted samples. `.gitattributes` pins `*.asset binary` and `*.asset.meta binary` to prevent CRLF conversion.
- Cross-platform concerns (universal macOS dylib, Windows UCRT linkage) now live in the bridge crate, not here.

## Layout

Cargo workspace, three crates (plus vendored submodules under `vendor/`, see below):

- **`crates/core/`** — the rlib (`unity-sprite-author` package). Consumed by the BoxcatBridge cdylib in meow-tower (`Native~/bridge/` points its `path = "..."` here). Module-level orientation lives in `src/lib.rs`; key entry points are `pipeline::generate`, `manifest::parse` (unified CSA+SMA tree), `combine::build_combined`, `mesh_emit::build_mesh`, and `emit::SpriteAsset`. Golden tests under `tests/golden/{orgel,fab,sma}/`; opt-in `e2e_meow_tower.rs` walks a meow-tower checkout.
- **`crates/cli/`** — offline `unity-sprite-author` bin. Mints missing `.tps.meta`/`.png.meta` (`unity-assetdb`), drives the `crates/pack` flow, then `pipeline::generate`. See [[#cli]].
- **`crates/pack/`** — the `.tps → .tpsheet` step shared by the CLI and the boxcat bridge's Editor watch: pre-pack 1×1 `Color_*.png` synthesis (`color_synth` + `color_png`; .tps DOM via `tps-core`) and the TexturePackerCLI shell-out. `tps-core` is a **git** dep so each consuming workspace's `[patch]` collapses it to that workspace's `vendor/tps` (a path dep would duplicate-package).
- **`crates/editor/`** — GUI tool (`eframe` + `egui`, native macOS menubar via `muda`) for authoring `.tps.fab.json`. Every user-facing command flows through one `Action` enum in `action.rs` — extend `Action`, add a `match` arm in `App::dispatch`, optionally register a `CommandEntry` for palette discoverability. Editor and CLI never leak into the rlib.

Vendored as git submodules (path deps from `crates/cli`; each is its own Cargo workspace/crate, so the root `Cargo.toml` `exclude = ["vendor"]`s them):

- **`vendor/unity-assetdb/`** ([studio-boxcat/unity-assetdb](https://github.com/studio-boxcat/unity-assetdb)) — `.tps.meta`/`.png.meta` minting (`register`); its own `refresh` rides `unity-watch`.
- **`vendor/tps/`** ([studio-boxcat/tps](https://github.com/studio-boxcat/tps)) — the `tps-core` TexturePacker `.tps` DOM (`list_file_lists`, sprite settings). `crates/pack` git-deps it; the root `[patch]` collapses it to this submodule. Cross-repo deploy + propagation (push → submodule bump → rebuild): `` `tps-deployment.md` `` (tps).
- **`vendor/unity-watch/`** ([studio-boxcat/unity-watch](https://github.com/studio-boxcat/unity-watch)) — standalone shared Watchman wire layer (`since` / `enumerate` / `subscribe` + `init_socket_env`). Not consumed directly here, but the vendored unity-assetdb (and unity-solution-generator) git-dep it — the root `Cargo.toml` `[patch]` redirects that git URL to this submodule so the whole graph resolves to one copy.

Supporting: `docs/` (fab schema, SMA migration map, Unity-Editor probe runbooks), `scripts/` (corpus regen, authoring teardown), `justfile` (`just install` → `~/.local/bin/`).
