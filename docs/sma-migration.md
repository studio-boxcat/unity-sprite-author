# SpriteMeshAuthor migration scope

> **Related:** [[CLAUDE.md]], [[fab.md]], [[unity-probes.md]]

Phase 2a study of `Packages/com.boxcat.libs/Core/Runtime/SpriteMeshAuthoring/SpriteMeshAuthor.cs`
(meow-tower) for the second half of the
"byte-to-byte matching clean migration" effort. 36 prefabs use SMA
(`/tmp/sma-files.txt`); `Box_00_Normal.prefab` alone has dozens of
instances. Script GUID `d003afe76b0a48aa8f1caad657e5095a`.

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

## Why it doesn't fit `.tps.fab.json` v1

The current `fab.md` schema emits Sprite `.asset` files where each
combined entry is a single Sprite backed by an atlas-internal mesh. SMA
emits standalone Mesh `.asset` files (or sub-assets) that reference an
atlas texture indirectly via a SpriteRenderer's material in the consuming
prefab. The Sprite emit pipeline doesn't know how to write a Mesh `.asset`.

## What Phase 2b needs

To migrate SMA to a Rust-emitted manifest analogous to fab.json, the
crate must grow:

1. **Mesh `.asset` emit module.** New format (separate file from
   `emit::SpriteAsset`). Unity's `Mesh` YAML serialization is much
   simpler than Sprite — primarily `m_VertexData`, `m_IndexBuffer`,
   `m_SubMeshes`, `m_LocalAABB`. Two variants by `UsedInCanvas`:
   - SpriteRenderer-mode: VBO is `pos.xyz (f32) + uv.xy (Float16)`,
     interleaved per vertex, padded.
   - CanvasRenderer-mode: VBO is `pos.xyz (f32) + uv.xy (f32) +
     color.rgba (Color32)`.

2. **Tiled mesh generator.** Port `GetMesh_Tiled` (the quotient logic
   for partial tiles; ear-clip not needed since UISolid-style quad).

3. **Simple mesh generator.** Verts/uvs/tris copied from a tpsheet
   entry, transformed by a per-renderer matrix that folds in flip,
   local-to-world, and world-to-root chains. Matrix application order
   matters — `MultiplyPoint2D` semantics, no FMA fusion since these
   matrices are applied per-vert (no `Mesh.CombineMeshes` step on
   this path).

4. **Manifest schema.** Either extend `.tps.fab.json` (`type: "mesh"`
   variant on each combined entry) or introduce a sibling
   `<atlas>.tps.mesh.json`. Schema needs: output path (mesh asset
   location), `usedInCanvas` flag, ordered list of renderer entries
   (sprite name, flip x/y, draw mode, tile size for Tiled, per-renderer
   local-to-root translation + rotation).

5. **Dumper.** Same shape as `examples/csa_dumper.cs` but walks
   `SpriteRenderer` children and dumps `{sprite, flipX, flipY, drawMode,
   size, localToRoot}` per leaf, plus the SMA's `UsedInCanvas` flag.

6. **Byte-exact validation.** Existing committed Mesh `.asset` files
   in meow-tower (36 distinct + sub-assets) become the golden corpus.

## Architectural decision points

- **In-crate vs adjacent crate.** SMA Mesh emit shares triangulation
  and atlas-rect lookup with the Sprite path but the output asset
  type is different. A sibling `unity-mesh-author` crate may be cleaner
  than bloating this one. Current preference: keep in-crate as a new
  `mesh_emit` module so the pipeline can emit both Sprite and Mesh
  `.asset` from a single fab manifest.
- **Manifest unification.** A combined-mesh entry and a combined-sprite
  entry differ only in output asset type + a few flags (UsedInCanvas,
  flip handling, drawMode/tiled-size). Could be one schema with a
  `kind: "sprite"|"mesh"` discriminator. Decided post-spike.
- **f16 UV emission.** Unity's `half` is IEEE 754 binary16, round-to-nearest.
  Rust has no stable f16; need an explicit f32 → u16 conversion that
  matches Unity's `Mathf.FloatToHalf` exactly (bit-by-bit). Pin via
  fixtures.

## Open question — should this live here?

The Sprite-emit crate's scope is "Sprite `.asset` byte-exactness". Mesh
emit is a meaningful expansion. The alternative is to keep SMA in C#
indefinitely — but that contradicts the "clean removal" goal. Decision
needed before Phase 2b lands code.

## References

- `Packages/com.boxcat.libs/Core/Runtime/SpriteMeshAuthoring/SpriteMeshAuthor.cs` (meow-tower)
- `Packages/com.boxcat.libs/Core/Runtime/SpriteMeshAuthoring/SpriteMeshBuilder.cs` (meow-tower)
- `Packages/com.boxcat.libs/Core/Runtime/SpriteMeshAuthoring/MeshData.cs` (meow-tower)
- `Sprite{Combine,Mirror,Patch,Renderer}Feed.cs` — `ISpriteRendererFeed`
  implementations that build per-renderer state before mesh extraction.
