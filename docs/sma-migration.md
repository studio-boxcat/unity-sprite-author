# SpriteMeshAuthor migration

> **Related:** [[CLAUDE.md]], [[fab.md]], [[unity-probes.md]]

Phase 2 of the "byte-to-byte matching clean migration": SMA Mesh `.asset`
emit moved from C# to this crate. Source studied:
`Packages/com.boxcat.libs/Core/Runtime/SpriteMeshAuthoring/SpriteMeshAuthor.cs`
(meow-tower). Script GUID `d003afe76b0a48aa8f1caad657e5095a`.

## What SMA emits

A **Mesh asset** (`.asset` file containing a `Mesh`), **not** a Sprite. The
`Mesh` is either:
- A primary `.asset` next to the prefab, or
- A sub-asset attached to a main asset.

`SpriteMeshAuthor.Publish()` clears the Mesh in place, runs
`SpriteMeshBuilder.ExtractMeshDataByHierarchicalOrder`, then writes back
via `BuildMeshForCanvasRenderer` (`UsedInCanvas == true`) or
`BuildMeshForSpriteRenderer` (`false`). The two paths differ in vertex
buffer layout — see below.

## Mesh build algorithm

`ExtractMeshDataByHierarchicalOrder`:
1. DFS-collect all active `SpriteRenderer`s under root.
2. Pick `tex/material` from the first white-colored renderer (fallback:
   first renderer). All renderers must share texture + material; color
   must be white.
3. Per renderer, build a per-vertex `Matrix4x4` chain:
   `wtl × renderer.localToWorldMatrix × flipMatrix` — where
   `flipMatrix.m00 = renderer.flipX ? -1 : 1`, `m11 = flipY ? -1 : 1`.
4. Dispatch on `renderer.drawMode`:
   - `Simple` → copy `sprite.vertices`, `sprite.uv`, `sprite.triangles`
     verbatim; apply matrix per vertex.
   - `Tiled` (Continuous only) → axis-aligned quad tiling. Repeats
     `sprite` across `renderer.size`; final row/column gets a partial
     UV slice when the divisor isn't integer. Hard-asserted: sprite is
     a quad, `tileRepeatX * tileRepeatY < 200`.

Position uses `Vector3` (z = 0); UV uses `Vector2`. Triangles are u16.

`BuildMeshFor{SpriteRenderer,CanvasRenderer}` differ:
- **SpriteRenderer** path: positions f32, UVs are **Float16** (half2)
  via `VertexAttributeFormat.Float16`; `UploadMeshData(markNoLongerReadable: true)`.
- **CanvasRenderer** path: positions + UVs both f32; colors written as
  opaque white per vertex; `UploadMeshData(false)` keeps the mesh readable.

## Implementation map

| Concern | Module |
|---|---|
| Mesh `.asset` emit (both VBO layouts) | `src/mesh_emit.rs` |
| `Mathf.FloatToHalf` byte-port (round-to-nearest-even) | `mesh_emit::float_to_half` |
| Tiled mesh (`GetMesh_Tiled` port) | `mesh_emit::tiled_mesh` |
| Per-renderer matrix application | `mesh_emit::build_mesh` |
| Manifest (v3 unified tree, CSA + SMA) | `src/manifest.rs` |
| Manifest (legacy flat SMA-only) | `src/mesh_manifest.rs` |
| Pipeline dispatch | `pipeline::load_mesh_manifest` |
| Golden fixture | `tests/golden_sma_mesh.rs` (Box_29_Ghost, 32 meshes) |
| SMA dumper | `examples/sma_dumper.cs` → `examples/sma_dump_to_mesh_manifest.rs` |

## Resolved design choices

- **In-crate, not a sibling.** `mesh_emit` lives next to `emit` and
  reuses `tpsheet` / `meta` / `yaml`.
- **Unified v3 manifest.** A single `.tps.fab.json` v3 carries CSA and
  SMA trees, discriminated per-tree by `Output::Csa | Output::Sma`.
  Legacy `.tps.mesh.json` still parses as a fallback for old dumps.
- **f16 UVs via explicit byte-port.** `float_to_half` mirrors Unity's
  `Mathf.FloatToHalf` bit-for-bit; pinned by the Box_29_Ghost golden.

## References

- `Packages/com.boxcat.libs/Core/Runtime/SpriteMeshAuthoring/SpriteMeshAuthor.cs` (meow-tower)
- `Packages/com.boxcat.libs/Core/Runtime/SpriteMeshAuthoring/SpriteMeshBuilder.cs` (meow-tower)
- `Packages/com.boxcat.libs/Core/Runtime/SpriteMeshAuthoring/MeshData.cs` (meow-tower)
- `Sprite{Combine,Mirror,Patch,Renderer}Feed.cs` — `ISpriteRendererFeed`
  implementations that build per-renderer state before mesh extraction.
