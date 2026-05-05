# Unity-side migration plan

> **Related:** [[CLAUDE.md]], [[TODO.md]], [[BENCHMARKS.md]]

How to swap the in-Unity `CreateSprites` C# path for the Rust `cdylib` without breaking the meow-tower repo. Frozen-legacy atlases (`textureType=Sprite + spriteImportMode=Multiple`) keep their existing C# code path; only the new-format atlases route through native.

## Order of operations (rollout sequence)

The change is split into PRs that each leave the project in a working state.

### PR 1 — `cdylib` FFI surface, no C# changes

Goal: ship a working native binary that's not yet wired up.

**Rust:**
- `src/lib.rs`: define the C ABI per [[CLAUDE.md#c--rust-contract]]:
  - `extern "C" fn abi_version() -> u32` returning a constant.
  - `extern "C" fn generate(in: *const GenerateInputs, out: *mut GenerateOutput, err: *mut ErrorOut) -> i32`.
  - `extern "C" fn free_output(out: *mut GenerateOutput)`.
  - `extern "C" fn free_error(err: *mut ErrorOut)`.
- `src/ffi.rs`: `#[repr(C)]` structs, `CStr`/`CString` conversions, opaque `OutputArena` for memory ownership. The outermost `extern "C"` body is exactly `catch_unwind(AssertUnwindSafe(|| inner()))` and the inner returns `Result<_, Error>`.
- Cargo: confirm `crate-type = ["cdylib", "rlib"]`, `panic = "unwind"`, no `lto` regression that strips exports.
- A symbol-export sanity test: `nm -gU target/release/libunity_sprite_author.dylib | grep -E '^_(generate|free_output|free_error|abi_version)$'`.
- **Cross-build recipe** (a `Justfile` or shell script):
  - macOS arm64+x86_64 via `cargo zigbuild`, joined with `lipo`, ad-hoc `codesign`.
  - Windows x86_64-msvc via `cargo xwin build --target x86_64-pc-windows-msvc --release` (NOT `-gnu`; mixing CRTs corrupts the heap when ownership crosses).
- Drop binaries into `meow-tower/Assets/Plugins/Editor/`:
  - `libunity_sprite_author.dylib` (universal)
  - `unity_sprite_author.dll`
- Hand-author `.dylib.meta` and `.dll.meta` with `Editor: 1`, all platforms `0`. Unity's auto-detection defaults to "Any Platform" and bundles into player builds — guard against this explicitly.

**C# (none yet)**: nothing imports the lib. Unity loads it but it sits idle.

**Verify**: Unity Editor opens cleanly with the binaries committed; no errors in console.

### PR 2 — `TPSImporter` ScriptedImporter on `.tps`

Goal: a permanent home for `_prefix` (and any future per-atlas config) that survives the eventual `.tpsheet` deletion.

**C#:**
- New `Assets/50_Modules/Tools/TexturePacker/TPSImporter.cs`:
  - `[ScriptedImporter(version: 1, ext: "tps")]` with one serialized field `[SerializeField] private string _prefix = "";`.
  - `OnImportAsset(AssetImportContext)` is a no-op — does not call `ctx.AddObjectToAsset`. The importer exists purely to give Unity a place to store inspector-editable per-`.tps` settings. (Verified Unity is OK with a no-op `OnImportAsset` — `.tps` produces no main asset.)
  - Inspector editor (`[CustomEditor(typeof(TPSImporter))]`) so users see/edit `_prefix`.
- Update `TexturePackerUtils.cs` prefix readers (lines 103, 167) to read from `TPSImporter` on the `.tps` instead of from `.tpsheet.meta`:
  - `GetPrefixFromTpsheet(tpsPath)` → `GetPrefixFromTps(tpsPath)`.
  - Update both call sites.
- Leave `TPSheetImporter.cs` in place. It still owns `.tpsheet` and is referenced by the postprocessor for now.

**Migration script** (`scripts/migrate-tpsheet-meta.sh`):
- Walk every `.tpsheet.meta` with non-empty `_prefix`.
- For each, rewrite the corresponding `.tps.meta` (currently `DefaultImporter`) to a `ScriptedImporter` block referencing `TPSImporter`'s script GUID and copying the `_prefix` value over.
- Preserve the `.tps.meta` `guid:` field (don't regenerate — would break any references, even though typically nothing references `.tps`).
- `--dry-run` prints planned changes without writing.
- Idempotent: if `.tps.meta` already has `ScriptedImporter:` with `TPSImporter`, skip.
- Reverse via `git checkout` — script does not delete `.tpsheet.meta` in PR 2; cleanup deferred.

**Verify**:
- Run script `--dry-run`; eyeball output against a few `.tpsheet.meta` files.
- Run script for real; commit migrated `.tps.meta` files.
- Open Unity; confirm prefix is visible/editable on a `.tps` inspector.
- Confirm the existing `CreateSprites` C# path still works using the new prefix source (legacy and new alike).

### PR 3 — wire `TPSheetPostprocessor` to the native lib

Goal: replace the body of `CreateSprites` with a `[DllImport]` call.

**C#:**
- New `Assets/50_Modules/Tools/TexturePacker/NativeSpriteAuthor.cs`:
  - Constants: `LIB_NAME = "unity_sprite_author"`, `EXPECTED_ABI = 1`.
  - `[DllImport(LIB_NAME)] internal static extern uint abi_version();`
  - `[DllImport(LIB_NAME)] internal static extern int generate(ref GenerateInputs in, out IntPtr out, out IntPtr err);`
  - `[DllImport(LIB_NAME)] internal static extern void free_output(IntPtr out);`
  - `[DllImport(LIB_NAME)] internal static extern void free_error(IntPtr err);`
  - `[StructLayout(LayoutKind.Sequential)] struct GenerateInputs { ... }` matching the Rust `#[repr(C)]`.
  - Public wrapper `GenerateOutput Generate(GenerateInputs inputs)` that:
    - Asserts `abi_version() == EXPECTED_ABI` on first call (cached static bool); throws `DllNotFoundException` on mismatch.
    - Calls native `generate`, marshals output paths to managed `string[]`, calls `free_output` in a `finally` block.
    - On non-zero return, marshals error message, calls `free_error`, throws `Exception`.
- Modify `TPSheetPostprocessor.cs` `CreateSprites(...)`:
  - Replace the entire method body with a call to `NativeSpriteAuthor.Generate`.
  - Loop over `output.WrittenPaths` calling `AssetDatabase.ImportAsset(p, ImportAssetOptions.ForceUpdate)`.
  - Loop over `output.DeletedPaths` calling `AssetDatabase.DeleteAsset(p)`. (Includes the consumed `.tpsheet` and `.tpsheet.meta`.)
  - Drop `StartAssetEditing` wrapper from this branch (no effect on raw FS writes anyway).

**Verify**:
- Build on macOS, smoke-test on a single small atlas (Cake / Orgel).
- Inspect `git diff` over `Assets/.../*.asset` after reimport — expect zero changes for sprite_scale=1, centered-pivot sprites; expect `.asset` deltas for the others (per [[TODO.md]] m_Offset gap).
- Compare a prefab referencing the atlas before/after — visual position must be unchanged for the byte-exact subset; track shifts elsewhere.
- Build on Windows porting station; smoke-test there too.

### PR 4 — clean up

Goal: delete the now-unused `CreateSprites` body, prune comments, possibly remove `TPSheetImporter` if no remaining references.

**C#:**
- Audit usage of `TPSheetImporter` after PR 3. If only the postprocessor's now-dead `prefix` read at line 57-58 references it, clean up.
- Delete the `.tpsheet.meta` files that the migration script left behind (now that prefix has been moved).
- Or: leave them and let Unity GC them on next refresh.

**Rust:**
- Audit `TODO.md` for unaddressed gaps (m_Offset, settingsRaw, non-zero borders).

**Verify**: full meow-tower reimport, inspect `git diff`, scan for unintended deltas.

## Risks

| Risk | Mitigation |
|---|---|
| **`m_Offset` gap** ([[TODO.md]]). Our formula matches Unity for centered pivots only. ~36% of sprites would shift on first migration reimport. | Run e2e in CI to track parity rate. Rollout staged: legacy atlases unaffected; new-format atlases visually shift only where m_Offset is non-zero (mostly UI). Visually inspect representative scenes before merging PR 3. Defer wide rollout until m_Offset closed. |
| **Unity overwrites Rust-supplied `.asset.meta` GUID on import.** Spec says preserve; Unity normally does, but new metas during import may regenerate. | Bootstrap test: delete a `.asset` + `.asset.meta` pair, run pipeline, verify Rust-supplied GUID survives `AssetDatabase.ImportAsset`. Added to TODO.md. |
| **`x86_64-pc-windows-gnu` heap corruption.** Mixing CRTs across FFI. | Use `-msvc` target via `cargo xwin`. Documented in CLAUDE.md. |
| **`StartAssetEditing` reentry hazard with `SaveAndReimport`.** | Already deferred per TODO.md; legacy path is frozen, new path doesn't re-enter. |
| **Migration script wipes prefix on a `.tps.meta` that was hand-edited.** | `--dry-run` first; idempotency check (skip if already ScriptedImporter); commit `.tps.meta` changes in a single reviewable commit. |
| **Plugin loaded but old C# code path still wins** (developers without latest C#). | `abi_version` handshake: C# asserts at first call; mismatch → clear error + Editor-modal log. |
| **Bulk reimport stalls developers**. | Already profiled — 28 ms per 62 sprites. 200 atlases ≈ 6s on full reimport. Acceptable. |
| **macOS Gatekeeper / quarantine on first checkout**. | Build recipe runs `codesign --sign -` ad-hoc. Document `xattr -d com.apple.quarantine` in CLAUDE.md if it bites. |

## What does **not** change

- The legacy code path (lines 35-39, 178+, plus `OnPreprocessTexture` / `OnPostprocessSprites` in `TPSheetPostprocessor.cs`). Frozen until a future PR strips it.
- `SheetLoader.cs`, `SheetPostprocessor.cs` legacy helpers.
- The `.tpsheet` file itself remains in Assets/ (new-format atlases delete it post-import; legacy keeps it as importer trigger).
- Unity AssetBundle / addressables references — sprite GUIDs are preserved, so prefab references stay valid.

## What gets deleted

- After PR 3 lands successfully: the `CreateSprites` body in C#.
- After PR 4: `.tpsheet.meta` files (Unity auto-GCs once `.tpsheet` files are deleted by the new pipeline).
- After legacy is stripped (future PR, out of scope here): `TPSheetImporter.cs`, `OnPreprocessTexture` and `OnPostprocessSprites` overrides, the `IsLegacyTexture` branch.

## Decision points needing user sign-off before PR 3

1. **m_Offset gap**: ship PR 3 knowing ~36% of sprites will get new m_Offset bytes? Or block on closing the formula first?
2. **Reimport-everything cadence**: PR 3 doesn't auto-reimport — it changes the import logic, but committed `.asset` files stay byte-exact unless a developer triggers a reimport. Stage rollout: re-import only the byte-exact subset first; defer the rest until m_Offset closes.
3. **Windows porting team build cadence**: do they pull pre-built `.dll` from git, or build locally? Pre-built is simpler if the maintainer commits both binaries on each Rust change.
