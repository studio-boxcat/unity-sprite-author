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
| Mesh `.asset` emit (both VBO layouts) | `crates/core/src/mesh_emit.rs` |
| `Mathf.FloatToHalf` byte-port (round-to-nearest-even) | `mesh_emit::float_to_half` |
| Tiled mesh (`GetMesh_Tiled` port) | `mesh_emit::tiled_mesh` |
| Per-renderer matrix application | `mesh_emit::build_mesh` |
| Manifest (v3 unified tree, CSA + SMA) | `crates/core/src/manifest.rs` |
| Manifest (legacy flat SMA-only) | `crates/core/src/mesh_manifest.rs` |
| Pipeline dispatch | `pipeline::load_mesh_manifest` |
| Golden fixture | `crates/core/tests/golden_sma_mesh.rs` (Box_29_Ghost, 32 meshes) |
| SMA dumper | `crates/core/examples/sma_dumper.cs` → `crates/core/examples/sma_dump_to_mesh_manifest.rs` |

## Resolved design choices

- **In-crate, not a sibling.** `mesh_emit` lives next to `emit` and
  reuses `tpsheet` / `meta` / `yaml`.
- **Unified v3 manifest.** A single `.tps.fab.json` v3 carries CSA and
  SMA trees, discriminated per-tree by `Output::Csa | Output::Sma`.
  Legacy `.tps.mesh.json` still parses as a fallback for old dumps.
- **f16 UVs via explicit byte-port.** `float_to_half` mirrors Unity's
  `Mathf.FloatToHalf` bit-for-bit; pinned by the Box_29_Ghost golden.

## Outstanding (Phase 2b): polygon-color synthesis

9 Box atlases (`Boxes/{03_Unicorn,05_Dino,09_Wizard,28_Boardgame,29_Ghost,30_Sleepy,31_Raincoat,32_Pilot,33_Vampire}`)
have child SpriteRenderers named `Polygon` whose `.sprite` is a
runtime-created Color texture (no GUID, hashed name like `mBzYY2st`).
The Unity-side SMA pipeline accepts these because it walks geometry;
the Rust port requires every renderer's sprite to resolve in the
sibling tpsheet, and `manifest::to_mesh_combined` currently rejects
polygon graphics under SMA trees (`bridge_to_mesh_rejects_polygon_in_sma`).

To unblock:

- **Authoring path**: hand- or LLM-edited fab.json declares polygon
  leaves under SMA trees the same way the CSA side does (see [[fab.md]]
  schema for `type: "polygon"`).
- **Manifest schema** (`manifest.rs` Node): add polygon-leaf fields to
  `spriteRenderer` mode (mirrors the existing UISolid path on CSA trees).
- **Emit extension** (`mesh_emit::build_mesh`): for polygon renderers,
  synthesize verts directly + sample UVs from a `Color_RRGGBB` entry in
  the same tpsheet (mirrors `combine::polygon_mesh_with_tris` on CSA).
- **Golden coverage**: extend `tests/golden/sma/box_29_ghost/` to
  exercise the polygon branch.

## References

- `Packages/com.boxcat.libs/Core/Runtime/SpriteMeshAuthoring/SpriteMeshAuthor.cs` (meow-tower)
- `Packages/com.boxcat.libs/Core/Runtime/SpriteMeshAuthoring/SpriteMeshBuilder.cs` (meow-tower)
- `Packages/com.boxcat.libs/Core/Runtime/SpriteMeshAuthoring/MeshData.cs` (meow-tower)
- `Sprite{Combine,Mirror,Patch,Renderer}Feed.cs` — `ISpriteRendererFeed`
  implementations that build per-renderer state before mesh extraction.
