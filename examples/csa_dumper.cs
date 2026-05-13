var sb = new System.Text.StringBuilder();
string[] paths = {
    "Assets/21_Collections/PremiumCatEvents/33_Vampire/Elements/Ornaments/Silloutte1.prefab",
    "Assets/21_Collections/PremiumCatEvents/33_Vampire/Elements/Ornaments/Silloutte2.prefab",
    "Assets/21_Collections/PremiumCatEvents/33_Vampire/Elements/Ornaments/Silloutte3.prefab",
};

string BitsHex(float v) => System.BitConverter.SingleToInt32Bits(v).ToString("X8");
string AssetGuid(UnityEngine.Object o) {
    if (o == null) return "";
    var p = AssetDatabase.GetAssetPath(o);
    return AssetDatabase.AssetPathToGUID(p);
}

foreach (var pp in paths) {
    var prefab = AssetDatabase.LoadAssetAtPath<GameObject>(pp);
    if (prefab == null) { sb.AppendLine("PREFAB " + pp + " ERR=load"); continue; }

    var canvasGO = new GameObject("__probe", typeof(RectTransform), typeof(Canvas));
    var inst = (GameObject)PrefabUtility.InstantiatePrefab(prefab, canvasGO.transform);
    Canvas.ForceUpdateCanvases();

    var rootT = inst.transform;
    var rootRT = rootT as RectTransform;
    var rootW2L = rootT.worldToLocalMatrix;

    var csa = inst.GetComponent<Boxcat.Core.CanvasSpriteAuthor>();
    var f_csa_sprite = typeof(Boxcat.Core.CanvasSpriteAuthor).GetField("_sprite",
        System.Reflection.BindingFlags.NonPublic | System.Reflection.BindingFlags.Instance);
    var f_csa_scale = typeof(Boxcat.Core.CanvasSpriteAuthor).GetField("_scaleFactor",
        System.Reflection.BindingFlags.NonPublic | System.Reflection.BindingFlags.Instance);
    var csaSprite = (Sprite)f_csa_sprite.GetValue(csa);
    float csaScale = (float)f_csa_scale.GetValue(csa);

    sb.AppendLine("PREFAB " + pp);
    sb.AppendLine("  output_sprite_guid=" + AssetGuid(csaSprite));
    sb.AppendLine("  output_sprite_path=" + (csaSprite ? AssetDatabase.GetAssetPath(csaSprite) : ""));
    sb.AppendLine("  atlas_png_guid=" + (csaSprite && csaSprite.texture ? AssetGuid(csaSprite.texture) : ""));
    sb.AppendLine("  scale_factor=" + csaScale.ToString("R"));
    sb.AppendLine("  root_anchored=" + rootRT.anchoredPosition.x.ToString("R") + "," + rootRT.anchoredPosition.y.ToString("R"));

    var graphics = inst.GetComponentsInChildren<UnityEngine.UI.Graphic>(true);
    foreach (var g in graphics) {
        var t = g.transform;
        var rt = t as RectTransform;
        var gl = t.localToWorldMatrix;
        var rel = rootW2L * gl;

        var sf = g.GetType().GetField("_sprite",
            System.Reflection.BindingFlags.NonPublic | System.Reflection.BindingFlags.Instance);
        var sprite = sf != null ? (Sprite)sf.GetValue(g) : null;
        var scF = g.GetType().GetField("_scaleFactor",
            System.Reflection.BindingFlags.NonPublic | System.Reflection.BindingFlags.Instance);
        float scale = scF != null ? (float)scF.GetValue(g) : 0f;
        var mF = g.GetType().GetField("_method",
            System.Reflection.BindingFlags.NonPublic | System.Reflection.BindingFlags.Instance);
        string methodName = mF != null ? mF.GetValue(g).ToString() : "";
        int methodVal = mF != null ? System.Convert.ToInt32(mF.GetValue(g)) : -1;
        var bF = g.GetType().GetField("_borderMultiplier",
            System.Reflection.BindingFlags.NonPublic | System.Reflection.BindingFlags.Instance);
        float bm = bF != null ? (float)bF.GetValue(g) : 1f;

        var ms = MonoScript.FromMonoBehaviour(g);
        var scriptGuid = ms ? AssetGuid(ms) : "";
        var c = g.color;
        var ap = rt ? rt.anchoredPosition : (Vector2)t.localPosition;
        var sd = rt ? rt.sizeDelta : Vector2.zero;
        var pv = rt ? rt.pivot : new Vector2(0.5f, 0.5f);
        var ls = t.localScale;

        sb.AppendLine("  LEAF " + g.name);
        sb.AppendLine("    script_guid=" + scriptGuid);
        sb.AppendLine("    sprite_guid=" + AssetGuid(sprite));
        sb.AppendLine("    sprite_name=" + (sprite ? sprite.name : ""));
        sb.AppendLine("    scale_factor=" + scale.ToString("R"));
        sb.AppendLine("    method_name=" + methodName + " method_val=" + methodVal);
        sb.AppendLine("    border_mult=" + bm.ToString("R"));
        sb.AppendLine("    color=" + c.r.ToString("R") + "," + c.g.ToString("R") + "," + c.b.ToString("R") + "," + c.a.ToString("R"));
        sb.AppendLine("    anchored=" + ap.x.ToString("R") + "," + ap.y.ToString("R"));
        sb.AppendLine("    size_delta=" + sd.x.ToString("R") + "," + sd.y.ToString("R"));
        sb.AppendLine("    pivot=" + pv.x.ToString("R") + "," + pv.y.ToString("R"));
        sb.AppendLine("    local_scale=" + ls.x.ToString("R") + "," + ls.y.ToString("R") + "," + ls.z.ToString("R"));
        sb.AppendLine("    rel_m03=" + rel.m03.ToString("R") + " bits=0x" + BitsHex(rel.m03));
        sb.AppendLine("    rel_m13=" + rel.m13.ToString("R") + " bits=0x" + BitsHex(rel.m13));
    }

    UnityEngine.Object.DestroyImmediate(canvasGO);
}

System.IO.File.WriteAllText("/tmp/csa-dump.txt", sb.ToString());
return "wrote " + sb.Length + " bytes to /tmp/csa-dump.txt";
