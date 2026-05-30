// SMA dumper — for each SpriteMeshAuthor prefab listed in /tmp/sma-files.txt,
// instantiate (open scene, root only — SMA doesn't need a Canvas), walk
// SpriteRenderer children DFS, capture per-renderer state needed to emit
// a `.tps.mesh.json` entry that `pipeline::generate` consumes.
//
// Per leaf: { script_guid, sprite_guid, sprite_name, flipX, flipY, drawMode,
// size?, localToRoot (8 floats row-major) }. Per prefab: { prefab_path,
// output_asset_path, used_in_canvas, mesh_file_id, atlas_png_guid,
// atlas_prefix }. Output: /tmp/sma-dump.txt
//
// Run via: just scratch "$(cat path/to/sma_dumper.cs)"
//
// The companion converter (Rust example: `sma_dump_to_mesh_manifest`,
// to be written) reads this dump and emits the `<atlas>.tps.mesh.json`
// files keyed off `atlas_png_guid`, grouping by output_asset_path so
// every Mesh in a Box prefab lands in the same multi-mesh `.asset`.

var sb = new System.Text.StringBuilder();
var paths = System.IO.File.ReadAllLines("/tmp/sma-files.txt")
    .Where(s => s.Trim().Length > 0 && s.EndsWith(".prefab")).ToArray();

string Bx(float v) { return System.BitConverter.SingleToInt32Bits(v).ToString("X8"); }
string AssetGuid(UnityEngine.Object o) {
    if (o == null) return "";
    var p = AssetDatabase.GetAssetPath(o);
    return AssetDatabase.AssetPathToGUID(p);
}

foreach (var pp in paths) {
    var prefab = AssetDatabase.LoadAssetAtPath<GameObject>(pp);
    if (prefab == null) { sb.AppendLine("PREFAB " + pp + " ERR=load"); continue; }

    var holder = new GameObject("__probe");
    var inst = (GameObject) PrefabUtility.InstantiatePrefab(prefab, holder.transform);

    // Find every SMA on the instance + its descendants.
    var smas = inst.GetComponentsInChildren<Boxcat.Core.SpriteMeshAuthor>(true);
    foreach (var sma in smas) {
        var rootT = sma.transform;
        var rootW2L = rootT.worldToLocalMatrix;

        // Mesh field (output asset).
        var meshField = typeof(Boxcat.Core.SpriteMeshAuthor).GetField("Mesh");
        var meshObj = meshField != null ? meshField.GetValue(sma) as Mesh : null;
        long meshFileId = 0;
        string outputAssetPath = "";
        if (meshObj != null) {
            outputAssetPath = AssetDatabase.GetAssetPath(meshObj);
            // Mesh file_id is part of the embedded sub-asset header.
            // AssetDatabase.TryGetGUIDAndLocalFileIdentifier gives both.
            UnityEditor.AssetDatabase.TryGetGUIDAndLocalFileIdentifier(meshObj,
                out var _g, out meshFileId);
        }
        // SMA.UsedInCanvas is a public bool.
        bool usedInCanvas = sma.UsedInCanvas;

        // Collect SpriteRenderers under this SMA root, active only,
        // matching SpriteMeshBuilder.ExtractMeshDataByHierarchicalOrder.
        var srs = rootT.GetComponentsInChildren<SpriteRenderer>(false);
        if (srs.Length == 0) continue;

        // Pick the first white-colored renderer as the texture reference,
        // mirroring SMA's "first non-runtime SR" logic.
        SpriteRenderer sr0 = srs.FirstOrDefault(x => x.color == Color.white) ?? srs[0];
        var tex = sr0.sprite ? sr0.sprite.texture : null;
        string atlasPngGuid = tex ? AssetGuid(tex) : "";
        string atlasPngPath = tex ? AssetDatabase.GetAssetPath(tex) : "";

        // Atlas _prefix from TPSheetImporter on the sibling .tpsheet.
        string atlasPrefix = "";
        if (atlasPngPath.Length > 0) {
            var tpsheetPath = atlasPngPath.Substring(0, atlasPngPath.Length - 4) + ".tpsheet";
            var imp = AssetImporter.GetAtPath(tpsheetPath) as TexturePacker.TPSheetImporter;
            if (imp != null) atlasPrefix = imp.Prefix;
        }

        sb.AppendLine("SMA " + AssetDatabase.GetAssetPath(prefab) + " :: " + sma.gameObject.name);
        sb.AppendLine("  output_asset_path=" + outputAssetPath);
        sb.AppendLine("  mesh_file_id=" + meshFileId);
        sb.AppendLine("  used_in_canvas=" + (usedInCanvas ? "1" : "0"));
        sb.AppendLine("  atlas_png_guid=" + atlasPngGuid);
        sb.AppendLine("  atlas_prefix=" + atlasPrefix);
        sb.AppendLine("  sma_name=" + sma.gameObject.name);

        foreach (var sr in srs) {
            if (sr.sprite == null) continue;
            // CalculateRendererToRootMatrix: flip × renderer.localToWorld
            // × root.worldToLocal (composed L→R).
            var flip = Matrix4x4.identity;
            flip.m00 = sr.flipX ? -1f : 1f;
            flip.m11 = sr.flipY ? -1f : 1f;
            var renderL2W = sr.transform.localToWorldMatrix;
            var combined = rootW2L * renderL2W * flip;

            sb.AppendLine("  LEAF " + sr.gameObject.name);
            sb.AppendLine("    sprite_guid=" + AssetGuid(sr.sprite));
            sb.AppendLine("    sprite_name=" + sr.sprite.name);
            sb.AppendLine("    flip_x=" + (sr.flipX ? "1" : "0"));
            sb.AppendLine("    flip_y=" + (sr.flipY ? "1" : "0"));
            string drawMode = sr.drawMode == SpriteDrawMode.Tiled ? "tiled" : "simple";
            sb.AppendLine("    draw_mode=" + drawMode);
            if (sr.drawMode == SpriteDrawMode.Tiled) {
                sb.AppendLine("    size=" + sr.size.x.ToString("R") + "," + sr.size.y.ToString("R"));
            }
            // localToRoot as 8 floats row-major: [m00 m01 m02 m03 m10 m11 m12 m13]
            sb.AppendLine("    l2r="
                + combined.m00.ToString("R") + "," + combined.m01.ToString("R") + ","
                + combined.m02.ToString("R") + "," + combined.m03.ToString("R") + ","
                + combined.m10.ToString("R") + "," + combined.m11.ToString("R") + ","
                + combined.m12.ToString("R") + "," + combined.m13.ToString("R"));
            sb.AppendLine("    l2r_bits=0x"
                + Bx(combined.m00) + ",0x" + Bx(combined.m01) + ",0x"
                + Bx(combined.m02) + ",0x" + Bx(combined.m03) + ",0x"
                + Bx(combined.m10) + ",0x" + Bx(combined.m11) + ",0x"
                + Bx(combined.m12) + ",0x" + Bx(combined.m13));
        }
    }

    UnityEngine.Object.DestroyImmediate(holder);
}

System.IO.File.WriteAllText("/tmp/sma-dump.txt", sb.ToString());
return "wrote " + sb.Length + " bytes to /tmp/sma-dump.txt";
