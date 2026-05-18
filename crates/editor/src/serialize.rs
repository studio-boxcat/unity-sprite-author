//! Serialize `manifest::Manifest` back into the raw JSON schema. The core
//! `manifest::raw` types are deserialize-only and private; we mirror the
//! schema here for round-trip writes. Tested via load → save → load against
//! the committed fixtures in `crates/core/tests/golden/fab/`.

use serde_json::{json, Map, Value};
use unity_sprite_author::manifest::{
    DrawMode, Graphic, Manifest, Node, Output, SpriteMethod, Tree,
};

pub fn serialize(m: &Manifest) -> String {
    let trees: Vec<Value> = m.trees.iter().map(tree_to_json).collect();
    let root = json!({
        "version": 1,
        "combined": trees,
    });
    // Pretty-printed with serde defaults (2-space). Matches authored-by-hand
    // shape closely enough; one-time reformat diff per atlas.
    serde_json::to_string_pretty(&root).unwrap() + "\n"
}

fn tree_to_json(t: &Tree) -> Value {
    let mut o = Map::new();
    o.insert("name".into(), Value::String(t.name.clone()));
    o.insert("mode".into(), Value::String(mode_str(&t.output).into()));
    if let Output::Sma {
        file_id,
        output_path,
        keep_vertices,
        keep_indices,
        ..
    } = &t.output
    {
        o.insert("fileId".into(), json!(*file_id));
        o.insert("outputPath".into(), Value::String(output_path.clone()));
        // keepVertices/keepIndices default to true on load; round-trip honest.
        o.insert("keepVertices".into(), Value::Bool(*keep_vertices));
        o.insert("keepIndices".into(), Value::Bool(*keep_indices));
    }
    // The synthesized root is a pure container — emit its `children` as the
    // tree's `children`, matching the on-disk shape.
    let children: Vec<Value> = t.root.children.iter().map(node_to_json).collect();
    if !children.is_empty() {
        o.insert("children".into(), Value::Array(children));
    }
    Value::Object(o)
}

fn node_to_json(n: &Node) -> Value {
    let mut o = Map::new();
    if !n.name.is_empty() {
        o.insert("name".into(), Value::String(n.name.clone()));
    }
    if n.pos != [0.0, 0.0] {
        o.insert("pos".into(), json!(n.pos));
    }
    if let Some(s) = n.size {
        o.insert("size".into(), json!(s));
    }
    if let Some(p) = n.pivot {
        o.insert("pivot".into(), json!(p));
    }
    if n.scale != [1.0, 1.0] {
        if n.scale[0] == n.scale[1] {
            o.insert("scale".into(), json!(n.scale[0]));
        } else {
            o.insert("scale".into(), json!(n.scale));
        }
    }
    if n.rot_deg_ccw != 0.0 {
        o.insert("rotDegCCW".into(), json!(n.rot_deg_ccw));
    }
    if let Some(g) = &n.graphic {
        graphic_into(&mut o, g);
    }
    let children: Vec<Value> = n.children.iter().map(node_to_json).collect();
    if !children.is_empty() {
        o.insert("children".into(), Value::Array(children));
    }
    Value::Object(o)
}

fn graphic_into(o: &mut Map<String, Value>, g: &Graphic) {
    match g {
        Graphic::Sprite {
            sprite,
            method,
            border_mult,
            flip_x,
            flip_y,
        } => {
            o.insert("type".into(), Value::String("sprite".into()));
            o.insert("sprite".into(), Value::String(sprite.clone()));
            if *method != SpriteMethod::Id {
                o.insert("method".into(), Value::String(method_str(*method).into()));
            }
            if *border_mult != 1.0 {
                o.insert("borderMult".into(), json!(*border_mult));
            }
            if *flip_x {
                o.insert("flipX".into(), Value::Bool(true));
            }
            if *flip_y {
                o.insert("flipY".into(), Value::Bool(true));
            }
        }
        Graphic::Polygon {
            polygon_sprite,
            vertices,
            triangles,
        } => {
            o.insert("type".into(), Value::String("polygon".into()));
            // polygon_sprite is `Color_RRGGBB[AA]` — strip prefix for `color`.
            let color = polygon_sprite
                .strip_prefix("Color_")
                .unwrap_or(polygon_sprite);
            o.insert("color".into(), Value::String(color.to_string()));
            o.insert("vertices".into(), json!(vertices));
            if let Some(t) = triangles {
                o.insert("triangles".into(), json!(t));
            }
        }
        Graphic::SpriteRenderer { sprite, draw_mode } => {
            o.insert("type".into(), Value::String("spriteRenderer".into()));
            o.insert("sprite".into(), Value::String(sprite.clone()));
            if *draw_mode != DrawMode::Simple {
                o.insert("drawMode".into(), Value::String(draw_mode_str(*draw_mode).into()));
            }
        }
    }
}

fn mode_str(o: &Output) -> &'static str {
    match o {
        Output::Csa => "ui",
        Output::Sma { used_in_canvas: true, .. } => "sma-canvas",
        Output::Sma { used_in_canvas: false, .. } => "sma-renderer",
    }
}

pub fn method_str(m: SpriteMethod) -> &'static str {
    match m {
        SpriteMethod::Id => "ID",
        SpriteMethod::Mx => "MX",
        SpriteMethod::My => "MY",
        SpriteMethod::Mxy => "MXY",
        SpriteMethod::Tx => "TX",
        SpriteMethod::Ty => "TY",
        SpriteMethod::TxMc3 => "TX_MC3",
        SpriteMethod::R1c3 => "R1C3",
        SpriteMethod::R3c3 => "R3C3",
        SpriteMethod::R3c3Nf => "R3C3_NF",
        SpriteMethod::MxR1c3 => "MX_R1C3",
        SpriteMethod::MxR1c4 => "MX_R1C4",
        SpriteMethod::MxR3c2 => "MX_R3C2",
        SpriteMethod::MxR3c3 => "MX_R3C3",
        SpriteMethod::MxR3c4 => "MX_R3C4",
        SpriteMethod::MxR3c6 => "MX_R3C6",
        SpriteMethod::MyR2c2 => "MY_R2C2",
        SpriteMethod::MyR2c3 => "MY_R2C3",
        SpriteMethod::MyR3c1 => "MY_R3C1",
        SpriteMethod::MyR3c2 => "MY_R3C2",
        SpriteMethod::MyR3c3 => "MY_R3C3",
        SpriteMethod::MxyR3c3 => "MXY_R3C3",
        SpriteMethod::MxyR3c3Nf => "MXY_R3C3_NF",
    }
}

pub fn draw_mode_str(d: DrawMode) -> &'static str {
    match d {
        DrawMode::Simple => "simple",
        DrawMode::Tiled => "tiled",
    }
}

pub const ALL_METHODS: &[SpriteMethod] = &[
    SpriteMethod::Id,
    SpriteMethod::Mx, SpriteMethod::My, SpriteMethod::Mxy,
    SpriteMethod::Tx, SpriteMethod::Ty, SpriteMethod::TxMc3,
    SpriteMethod::R1c3, SpriteMethod::R3c3, SpriteMethod::R3c3Nf,
    SpriteMethod::MxR1c3, SpriteMethod::MxR1c4,
    SpriteMethod::MxR3c2, SpriteMethod::MxR3c3, SpriteMethod::MxR3c4, SpriteMethod::MxR3c6,
    SpriteMethod::MyR2c2, SpriteMethod::MyR2c3,
    SpriteMethod::MyR3c1, SpriteMethod::MyR3c2, SpriteMethod::MyR3c3,
    SpriteMethod::MxyR3c3, SpriteMethod::MxyR3c3Nf,
];

#[cfg(test)]
mod tests {
    use super::*;
    use unity_sprite_author::manifest::parse;

    #[test]
    fn round_trip_minimal_ui() {
        let src = r#"{"version":1,"combined":[{"name":"X","mode":"ui","children":[{"type":"sprite","sprite":"foo"}]}]}"#;
        let m = parse(src).unwrap();
        let out = serialize(&m);
        let m2 = parse(&out).unwrap();
        assert_eq!(m, m2);
    }

    #[test]
    fn round_trip_polygon_and_sma() {
        let src = r#"{"version":1,"combined":[
          {"name":"X","mode":"ui","children":[
            {"pos":[10,20],"type":"polygon","color":"32264D","vertices":[[0,0],[1,0],[1,1]],"triangles":[0,1,2]}
          ]},
          {"name":"Y","mode":"sma-canvas","fileId":-1,"outputPath":"o.asset","keepVertices":false,"keepIndices":true,
           "children":[{"type":"spriteRenderer","sprite":"a","drawMode":"tiled","size":[4,2]}]}
        ]}"#;
        let m = parse(src).unwrap();
        let out = serialize(&m);
        let m2 = parse(&out).unwrap();
        assert_eq!(m, m2);
    }

    #[test]
    fn round_trip_scale_collapses_to_uniform() {
        let src = r#"{"version":1,"combined":[{"name":"X","mode":"ui","children":[
          {"scale":2.5,"type":"sprite","sprite":"a"},
          {"scale":[-1,1],"type":"sprite","sprite":"b"}
        ]}]}"#;
        let m = parse(src).unwrap();
        let out = serialize(&m);
        let m2 = parse(&out).unwrap();
        assert_eq!(m, m2);
        // Uniform scale should be emitted as a single number, not [2.5, 2.5].
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        let first_child = &v["combined"][0]["children"][0];
        assert_eq!(first_child["scale"], serde_json::json!(2.5));
        let second_child = &v["combined"][0]["children"][1];
        assert_eq!(second_child["scale"], serde_json::json!([-1.0, 1.0]));
    }
}
