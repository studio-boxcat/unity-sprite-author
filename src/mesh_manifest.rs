// Typed IR for SMA Mesh `.asset` emission.
//
// Sibling of `fab::Manifest` for the mesh-emit side. Sprite-emit and
// mesh-emit are kept structurally separate (the asset types are different
// — `!u!213` Sprite vs `!u!43` Mesh).
//
// The legacy flat `.tps.mesh.json` v1 parser was retired in favor of the
// v3 unified manifest (`crate::manifest`), which bridges directly to this
// IR via `to_mesh_combined`. Downstream consumers (`mesh_emit`, `pipeline`)
// read these types.

#[derive(Debug, PartialEq)]
pub struct MeshManifest {
    pub meshes: Vec<MeshCombined>,
}

#[derive(Debug, PartialEq)]
pub struct MeshCombined {
    pub file_id: i64,
    pub name: String,
    /// Output `.asset` path, relative to the `.tps`'s directory. Multiple
    /// combined entries pointing at the same path are grouped into one
    /// multi-mesh asset (e.g. all Box_29_Ghost meshes land in
    /// `!Output/Box_29_Ghost.asset`).
    pub output_path: String,
    pub used_in_canvas: bool,
    pub keep_vertices: bool,
    pub keep_indices: bool,
    pub renderers: Vec<MeshRenderer>,
}

#[derive(Debug, PartialEq)]
pub struct MeshRenderer {
    pub sprite: String,
    pub flip_x: bool,
    pub flip_y: bool,
    pub draw_mode: DrawMode,
    pub size: Option<[f32; 2]>,
    /// 8 floats, row-major: `[m00, m01, m02, m03, m10, m11, m12, m13]`.
    /// Combined flip × renderer.localToWorld × root.worldToLocal as
    /// captured by the v3 → mesh bridge.
    pub local_to_root: [f32; 8],
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum DrawMode {
    Simple,
    Tiled,
}
