# Benchmarks

> **Related:** [[CLAUDE.md]], [[TODO.md]]

Profile data, runbook, and the cache decision.

## Runbook

```sh
cargo bench                                  # all benches
cargo bench --bench pipeline emit_cake       # filter by name
cargo flamegraph --bench pipeline -- --bench # CPU flamegraph (needs cargo-flamegraph)
```

`[profile.bench]` carries `debug = true` so flamegraph symbols resolve.

## Latest numbers

Hardware: Apple Silicon (Darwin 25.3), `cargo bench` release profile (LTO=thin, panic=unwind, codegen-units=1).

| Bench | Time | Notes |
|---|---|---|
| `pipeline_generate_orgel_62_sprites` | **28.2 ms** | full end-to-end including FS writes |
| `tpsheet_parse_orgel` | 107 µs | one-shot |
| `render_data_build_cake_decoleft` | 306 ns | per-sprite mesh encoding |
| `emit_cake_decoleft` | 2.75 µs | per-sprite YAML emission |
| `meta_render_asset_meta` | 50 ns | 189-byte template |
| `yaml_guid_hex` | 31 ns | 16-byte LUT encoder |

Pure CPU portion of the full pipeline: **~0.3 ms** for 62 sprites
(parse + 62×(build + emit + meta_render)). The remaining ~28 ms is filesystem I/O.

## Optimizations applied

Single LUT-based `hex_encode` shared across `yaml::guid_hex`, `render_data::encode_typelessdata`, and `render_data::encode_index_buffer`. Inlined the per-channel `format!` allocations in `emit::write_vertex_channels` (was 14 `String` allocations × 2 RD blocks per sprite). Bumped `emit` String capacity hint from 4 KB to 8 KB to clear the typical sprite size in one allocation.

## Cache decision: not needed

The user asked: "if need use cache, you can use `./config/unity-sprite-author/` for cache path." The profile says **no** — caching wouldn't pay off here:

- On a cache hit we still need to write 62 × 2 files. Writes are the bottleneck, not recomputation.
- On a cache miss the recompute is **<1 ms** for 62 sprites. The cache lookup overhead (hash inputs, read cache file, decode entries) is the same order of magnitude.
- Skip-write-if-equal already prevents redundant writes when bytes haven't changed.

Reconsider if the workload changes:
- Atlases grow >10× current size (e.g., 600+ sprites per atlas).
- Invocation overhead (Unity calling the dylib for *every* asset refresh, not just tpsheet imports) becomes routine.
- Bulk "Reimport All" over 200+ atlases stalls developers.

If/when caching is added: per-atlas key = `blake3(tpsheet_bytes, tps_bytes, atlas_guid_bytes, prefix, ppu_bytes)`. Value = the `GenerateOutput` (paths + cached bytes). Default location `./config/unity-sprite-author/cache.bin`, env-overridable. Binary format with magic `"USAC"`, u32 version, repeated entries `(key_hash[16], sprite_dir_len_u16, sprite_dir, written_count_u32, [(path_len_u16, path, asset_len_u32, asset_bytes, meta_len_u32, meta_bytes)], deleted_count_u32, [(path_len_u16, path)])`. Hand-rolled to keep `bincode`/`serde` out of the BoxcatBridge cdylib (this crate is consumed as an rlib through that bridge).

## What dominates the 28 ms FS portion

For 62 sprites the pipeline does (in order):
1. `read_dir` over `sprite_dir` (one scan, ~62 dir entries) — orphan detection.
2. 62 × `fs::read_to_string` of existing `.asset.meta` files — GUID resolution.
3. ~108 × `fs::read` for skip-write-if-equal comparison (62 .asset + 62 .asset.meta minus the ~16 that match cleanly).
4. ~108 × `fs::write` to `.tmp` + `fs::rename` to final — the two-phase commit.
5. 0–62 × `fs::remove_file` for orphans + the consumed `.tpsheet`.

If FS becomes the bottleneck in real use, the lever is to widen the skip-write-if-equal window: hash the existing file content once, compare against an in-memory hash of the planned bytes, only commit on mismatch. Saves the `.tmp` write + rename for unchanged outputs. Probably 10–20 ms savings on a stable atlas. Defer until measured pain.
