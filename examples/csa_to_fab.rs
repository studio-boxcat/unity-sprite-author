// Unity-free CSA Author prefab → `.tps.fab.json` (v3) converter.
//
// Replaces the old C# scratch dumper (`examples/csa_dumper.cs`) plus the
// v1 converter (`examples/csa_dump_to_fab.rs`). End-to-end is now:
//
//   1. `unity-assetdb usage <CSA_GUID>`         — list CSA-bearing prefabs
//   2. `pspec convert <prefab>`                 — emit `.prefab.pspec` JSON
//      (tree-shaped, typed components, resolved `$<alias>` sprite refs)
//   3. Parse the pspec JSON, validate structure (fail loud on unknown
//      component types, missing fields, unexpected children)
//   4. For each `$<alias>`: `unity-assetdb alias <name>` → sprite `.asset`
//      path → atlas dir + tpsheet entry name (atlas `_prefix` stripped)
//   5. Group by atlas, emit one v3 `.tps.fab.json` next to each `.tps`.
//
// All discovery + parsing is Rust + CLI tools; no Unity Editor required.
//
// Usage:
//   cargo run --release --example csa_to_fab -- $MEOW_CLIENT
//
// `--dry-run` (default if `--write` not passed): print emitted JSON to
// stdout grouped by atlas; nothing on disk changes.
// `--write`: write `<atlas>.tps.fab.json` files next to each `.tps`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

const CSA_GUID: &str = "571ad98c7c0d4a559a0cf213d8da355f";

// Allowed component __type names. Anything outside this set on any
// GameObject in the prefab triggers a fail-loud.
const CSA_TYPE: &str = "CanvasSpriteAuthor";
const UIICON_TYPE: &str = "UIIcon";
const UISLICE_TYPE: &str = "UISlice";
const UISOLID_TYPE: &str = "UISolid";
const CANVAS_RENDERER_TYPE: &str = "CanvasRenderer";

// ---------------------------------------------------------------------------
// Lightweight JSON value (avoids pulling serde_json into a tiny example —
// the pspec output is regular enough that a minimal recursive parser fits).
//
// Actually we DO use serde_json (already a runtime dep). Keeps the example
// short and the fail-loud semantics straightforward.

use serde_json::{Map, Value};

#[derive(Debug)]
struct Fail(String);
impl Fail {
    fn new(msg: impl Into<String>) -> Self {
        Self(msg.into())
    }
}
impl std::fmt::Display for Fail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

type R<T> = Result<T, Fail>;

// ---------------------------------------------------------------------------
// CLI wrappers.

fn run_cli(prog: &str, args: &[&str], cwd: Option<&Path>) -> R<String> {
    let mut cmd = Command::new(prog);
    cmd.args(args);
    if let Some(d) = cwd {
        cmd.current_dir(d);
    }
    let out = cmd
        .output()
        .map_err(|e| Fail::new(format!("spawn {prog}: {e}")))?;
    if !out.status.success() {
        return Err(Fail::new(format!(
            "{prog} {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn assetdb_usage(meow_client: &Path, guid: &str) -> R<Vec<PathBuf>> {
    let out = run_cli("unity-assetdb", &["usage", guid], Some(meow_client))?;
    let mut paths = Vec::new();
    for line in out.lines() {
        if line.trim().is_empty() {
            continue;
        }
        // Tab-separated: <path>\t<line_no>\t<context>
        let path = line.split('\t').next().unwrap_or("");
        if path.is_empty() {
            return Err(Fail::new(format!("malformed usage line: {line:?}")));
        }
        // Skip the script .cs.meta that ships its own GUID (the asset
        // we're searching for itself), and anything else that isn't a
        // .prefab.
        if !path.ends_with(".prefab") {
            continue;
        }
        paths.push(PathBuf::from(path));
    }
    Ok(paths)
}

fn assetdb_alias(meow_client: &Path, name: &str) -> R<Vec<AliasHit>> {
    let out = run_cli("unity-assetdb", &["alias", name], Some(meow_client))?;
    let mut hits = Vec::new();
    for line in out.lines() {
        if line.trim().is_empty() {
            continue;
        }
        // <guid>\t<name>\t<type>\t<path>
        let parts: Vec<&str> = line.splitn(4, '\t').collect();
        if parts.len() != 4 {
            return Err(Fail::new(format!("malformed alias line: {line:?}")));
        }
        hits.push(AliasHit {
            guid: parts[0].to_string(),
            name: parts[1].to_string(),
            kind: parts[2].to_string(),
            path: PathBuf::from(parts[3]),
        });
    }
    Ok(hits)
}

#[derive(Debug)]
struct AliasHit {
    guid: String,
    name: String,
    kind: String,
    path: PathBuf,
}

fn pspec_convert(meow_client: &Path, prefab: &Path) -> R<Value> {
    // pspec writes <prefab>.pspec next to the prefab. Read it back.
    let abs = if prefab.is_absolute() {
        prefab.to_path_buf()
    } else {
        meow_client.join(prefab)
    };
    let pspec_path = {
        let mut p = abs.clone();
        p.as_mut_os_string().push(".pspec");
        p
    };
    // Convert (overwrites the sidecar / pspec file).
    run_cli(
        "pspec",
        &["convert", abs.to_str().expect("utf8 path")],
        Some(meow_client),
    )?;
    let text = std::fs::read_to_string(&pspec_path)
        .map_err(|e| Fail::new(format!("read {pspec_path:?}: {e}")))?;
    serde_json::from_str(&text).map_err(|e| Fail::new(format!("parse {pspec_path:?}: {e}")))
}

// ---------------------------------------------------------------------------
// Domain — what we extract per prefab.

#[derive(Debug)]
struct PrefabCsa {
    prefab_path: PathBuf,
    output_sprite_alias: String, // "CM_BTN_Close_Blank" (after `$` strip)
    csa_scale: f32,              // _scaleFactor on the CSA MB. Defaults to 1.
    root_anchored: [f32; 2],     // root RectTransform.anchoredPosition. Defaults [0,0].
    children: Vec<LeafNode>,
}

#[derive(Debug)]
struct LeafNode {
    name: String,
    pos: [f32; 2],
    size_delta: [f32; 2],
    pivot: [f32; 2],
    scale: [f32; 2], // local_scale.xy. Drives geometric flip via negative values.
    rot_deg: f32,
    /// Either a leaf with a graphic, or a transform-only container with
    /// `children`. Pure containers (no graphic) act as nested anchor
    /// points whose transform composes with descendant leaves.
    graphic: Option<LeafGraphic>,
    children: Vec<LeafNode>,
}

#[derive(Debug)]
enum LeafGraphic {
    UIIcon {
        sprite_alias: String,
        ui_scale: f32,
        method: String, // "ID" | "MX" | "MY" | "MXY" | "FX" | "FY" | "FXY"
        border_mult: f32,
    },
    UISlice {
        sprite_alias: String,
        ui_scale: f32,
        method: String,
        border_mult: f32,
    },
    UISolid {
        // Color is 8-hex RRGGBBAA. fab v3 emits 6-hex when alpha is FF.
        color8: String,
    },
}

// ---------------------------------------------------------------------------
// pspec JSON → PrefabCsa with fail-loud validation.

fn obj<'a>(v: &'a Value, ctx: &str) -> R<&'a Map<String, Value>> {
    v.as_object()
        .ok_or_else(|| Fail::new(format!("{ctx}: expected object, got {v}")))
}

fn arr<'a>(v: &'a Value, ctx: &str) -> R<&'a Vec<Value>> {
    v.as_array()
        .ok_or_else(|| Fail::new(format!("{ctx}: expected array, got {v}")))
}

fn s<'a>(v: &'a Value, ctx: &str) -> R<&'a str> {
    v.as_str()
        .ok_or_else(|| Fail::new(format!("{ctx}: expected string, got {v}")))
}

fn f(v: &Value, ctx: &str) -> R<f32> {
    v.as_f64()
        .map(|x| x as f32)
        .ok_or_else(|| Fail::new(format!("{ctx}: expected number, got {v}")))
}

fn xy(node: &Map<String, Value>, key: &str, default: [f32; 2], ctx: &str) -> R<[f32; 2]> {
    let Some(v) = node.get(key) else {
        return Ok(default);
    };
    let m = obj(v, &format!("{ctx}.{key}"))?;
    let x = m
        .get("x")
        .map(|v| f(v, &format!("{ctx}.{key}.x")))
        .transpose()?
        .unwrap_or(default[0]);
    let y = m
        .get("y")
        .map(|v| f(v, &format!("{ctx}.{key}.y")))
        .transpose()?
        .unwrap_or(default[1]);
    Ok([x, y])
}

fn xyz(node: &Map<String, Value>, key: &str, default: [f32; 3], ctx: &str) -> R<[f32; 3]> {
    let Some(v) = node.get(key) else {
        return Ok(default);
    };
    let m = obj(v, &format!("{ctx}.{key}"))?;
    let x = m
        .get("x")
        .map(|v| f(v, &format!("{ctx}.{key}.x")))
        .transpose()?
        .unwrap_or(default[0]);
    let y = m
        .get("y")
        .map(|v| f(v, &format!("{ctx}.{key}.y")))
        .transpose()?
        .unwrap_or(default[1]);
    let z = m
        .get("z")
        .map(|v| f(v, &format!("{ctx}.{key}.z")))
        .transpose()?
        .unwrap_or(default[2]);
    Ok([x, y, z])
}

fn parse_prefab(pspec: &Value, prefab_path: &Path) -> R<PrefabCsa> {
    let ctx = prefab_path.display().to_string();
    let root = obj(pspec, &ctx)?;

    // Allowed root-level keys. Anything outside this set indicates a
    // prefab shape we don't yet handle — fail loud.
    const ROOT_KEYS: &[&str] = &[
        "__id",
        "name",
        "pos",
        "anchor",
        "sizeDelta",
        "pivot",
        "rot",
        "scale",
        "layer",
        "tag",
        "active",
        "components",
        "children",
    ];
    for k in root.keys() {
        if !ROOT_KEYS.contains(&k.as_str()) {
            return Err(Fail::new(format!(
                "{ctx}: unexpected root field {k:?} (allowed: {ROOT_KEYS:?})"
            )));
        }
    }

    let root_anchored = xy(root, "pos", [0.0, 0.0], &ctx)?;

    // Root components: exactly one CanvasSpriteAuthor; CanvasRenderer ok; nothing else.
    let comps = root
        .get("components")
        .map(|v| arr(v, &format!("{ctx}.components")))
        .transpose()?
        .map(|v| v.as_slice())
        .unwrap_or(&[]);
    let mut csa_comp: Option<&Map<String, Value>> = None;
    let mut root_graphic: Option<LeafGraphic> = None;
    for c in comps {
        let cm = obj(c, &format!("{ctx}.components[?]"))?;
        let ty = s(
            cm.get("__type")
                .ok_or_else(|| Fail::new(format!("{ctx}.components[?]: missing __type")))?,
            &format!("{ctx}.components[?].__type"),
        )?;
        match ty {
            CSA_TYPE => {
                if csa_comp.is_some() {
                    return Err(Fail::new(format!("{ctx}: multiple CanvasSpriteAuthor on root")));
                }
                csa_comp = Some(cm);
            }
            CANVAS_RENDERER_TYPE => {}
            UIICON_TYPE | UISLICE_TYPE | UISOLID_TYPE => {
                // CSA may share its GameObject with the leaf's graphic
                // (e.g. GS_Ad). Treat the root itself as the only leaf.
                if root_graphic.is_some() {
                    return Err(Fail::new(format!(
                        "{ctx}: root has multiple graphic components"
                    )));
                }
                let (g, _fx, _fy) = parse_graphic(ty, cm, &ctx)?;
                root_graphic = Some(g);
            }
            _ => {
                return Err(Fail::new(format!(
                    "{ctx}: root has unexpected component {ty:?} (allowed: CanvasSpriteAuthor, CanvasRenderer, UIIcon, UISlice, UISolid)"
                )));
            }
        }
    }
    let csa = csa_comp
        .ok_or_else(|| Fail::new(format!("{ctx}: root missing CanvasSpriteAuthor")))?;
    let csa_scale = csa
        .get("_scaleFactor")
        .map(|v| f(v, &format!("{ctx}.CSA._scaleFactor")))
        .transpose()?
        .unwrap_or(1.0);
    let raw_output_alias = csa
        .get("_sprite")
        .map(|v| s(v, &format!("{ctx}.CSA._sprite")))
        .transpose()?
        .map(strip_dollar)
        .ok_or_else(|| Fail::new(format!("{ctx}: CSA._sprite missing")))?;
    // Strip pspec disambiguator suffixes (`^<dir>` or `@<atlas>`) to get
    // the bare output filename stem. The disambiguator is only meaningful
    // for the alias lookup, not for the on-disk asset name.
    let output_sprite_alias: String = raw_output_alias
        .split(['^', '@'])
        .next()
        .unwrap_or(&raw_output_alias)
        .to_string();

    let children_v = root
        .get("children")
        .map(|v| arr(v, &format!("{ctx}.children")))
        .transpose()?
        .map(|v| v.as_slice())
        .unwrap_or(&[]);
    let mut leaves = Vec::new();
    for (i, ch) in children_v.iter().enumerate() {
        let leaf = parse_leaf(ch, &format!("{ctx}.children[{i}]"))?;
        leaves.push(leaf);
    }
    // If the root carries a graphic itself (GS_Ad-style), synthesize a
    // single-leaf where the leaf IS the root's transform + graphic. Root
    // sizeDelta becomes the leaf's sizeDelta.
    if let Some(g) = root_graphic {
        let root_size_delta = xy(root, "sizeDelta", [0.0, 0.0], &ctx)?;
        let root_pivot = xy(root, "pivot", [0.5, 0.5], &ctx)?;
        leaves.insert(
            0,
            LeafNode {
                name: String::new(),
                pos: [0.0, 0.0],
                size_delta: root_size_delta,
                pivot: root_pivot,
                scale: [1.0, 1.0],
                rot_deg: 0.0,
                graphic: Some(g),
                children: Vec::new(),
            },
        );
    }
    if leaves.is_empty() {
        return Err(Fail::new(format!("{ctx}: zero graphic children")));
    }

    Ok(PrefabCsa {
        prefab_path: prefab_path.to_path_buf(),
        output_sprite_alias,
        csa_scale,
        root_anchored,
        children: leaves,
    })
}

fn parse_leaf(v: &Value, ctx: &str) -> R<LeafNode> {
    let node = obj(v, ctx)?;
    const LEAF_KEYS: &[&str] = &[
        "__id",
        "name",
        "pos",
        "anchor",
        "sizeDelta",
        "pivot",
        "rot",
        "scale",
        "layer",
        "tag",
        "active",
        "components",
        "children",
    ];
    for k in node.keys() {
        if !LEAF_KEYS.contains(&k.as_str()) {
            return Err(Fail::new(format!(
                "{ctx}: unexpected field {k:?} (allowed: {LEAF_KEYS:?})"
            )));
        }
    }
    let name = node
        .get("name")
        .map(|v| s(v, &format!("{ctx}.name")))
        .transpose()?
        .unwrap_or("")
        .to_string();
    let pos = xy(node, "pos", [0.0, 0.0], ctx)?;
    let size_delta = xy(node, "sizeDelta", [0.0, 0.0], ctx)?;
    let pivot = xy(node, "pivot", [0.5, 0.5], ctx)?;
    let scale_xyz = xyz(node, "scale", [1.0, 1.0, 1.0], ctx)?;
    let scale = [scale_xyz[0], scale_xyz[1]];

    // Rotation: pspec emits `rot` as Euler `{x,y,z}` or omitted (= identity).
    let rot = xyz(node, "rot", [0.0, 0.0, 0.0], ctx)?;
    if rot[0] != 0.0 || rot[1] != 0.0 {
        return Err(Fail::new(format!(
            "{ctx}.rot: only z-axis rotation supported (got x={} y={})",
            rot[0], rot[1]
        )));
    }
    let rot_deg = rot[2];

    let comps = node
        .get("components")
        .map(|v| arr(v, &format!("{ctx}.components")))
        .transpose()?
        .map(|v| v.as_slice())
        .unwrap_or(&[]);

    let mut graphic: Option<LeafGraphic> = None;
    let mut leaf_fx = false;
    let mut leaf_fy = false;
    for c in comps {
        let cm = obj(c, &format!("{ctx}.components[?]"))?;
        let ty = s(
            cm.get("__type")
                .ok_or_else(|| Fail::new(format!("{ctx}.components[?]: missing __type")))?,
            &format!("{ctx}.components[?].__type"),
        )?;
        match ty {
            CANVAS_RENDERER_TYPE => {}
            UIICON_TYPE | UISLICE_TYPE | UISOLID_TYPE => {
                if graphic.is_some() {
                    return Err(Fail::new(format!(
                        "{ctx}: multiple graphic components (got a second {ty:?})"
                    )));
                }
                let (g, fx, fy) = parse_graphic(ty, cm, ctx)?;
                graphic = Some(g);
                leaf_fx = fx;
                leaf_fy = fy;
            }
            _ => {
                return Err(Fail::new(format!(
                    "{ctx}: unexpected component {ty:?} (allowed: UIIcon, UISlice, UISolid, CanvasRenderer)"
                )));
            }
        }
    }
    // Fold FX/FY/FXY into leaf.scale negation.
    let scale = [
        if leaf_fx { -scale[0] } else { scale[0] },
        if leaf_fy { -scale[1] } else { scale[1] },
    ];

    // Recurse into nested children. v3 schema supports tree-shaped
    // hierarchies; the pipeline's `manifest::walk` composes parent
    // transforms recursively.
    let grandchildren_v = node
        .get("children")
        .map(|v| arr(v, &format!("{ctx}.children")))
        .transpose()?
        .map(|v| v.as_slice())
        .unwrap_or(&[]);
    let mut grandchildren = Vec::new();
    for (i, gc) in grandchildren_v.iter().enumerate() {
        grandchildren.push(parse_leaf(gc, &format!("{ctx}.children[{i}]"))?);
    }

    // Pure containers (no graphic) are allowed if they have children.
    if graphic.is_none() && grandchildren.is_empty() {
        return Err(Fail::new(format!(
            "{ctx}: node has no graphic and no children"
        )));
    }

    Ok(LeafNode {
        name,
        pos,
        size_delta,
        pivot,
        scale,
        rot_deg,
        graphic,
        children: grandchildren,
    })
}

fn parse_graphic(ty: &str, m: &Map<String, Value>, ctx: &str) -> R<(LeafGraphic, bool, bool)> {
    // Returns (graphic, flip_x, flip_y). FX/FY/FXY methods desugar to
    // ID + negative leaf scale; the caller applies the flip to leaf.scale.
    let ui_scale = m
        .get("_scaleFactor")
        .map(|v| f(v, &format!("{ctx}.{ty}._scaleFactor")))
        .transpose()?
        .unwrap_or(1.0);
    let border_mult = m
        .get("_borderMultiplier")
        .map(|v| f(v, &format!("{ctx}.{ty}._borderMultiplier")))
        .transpose()?
        .unwrap_or(1.0);

    match ty {
        UIICON_TYPE | UISLICE_TYPE => {
            let sprite_alias = m
                .get("_sprite")
                .map(|v| s(v, &format!("{ctx}.{ty}._sprite")))
                .transpose()?
                .map(strip_dollar)
                .ok_or_else(|| Fail::new(format!("{ctx}.{ty}._sprite missing")))?;
            // Default method by type — UIIcon defaults ID, UISlice has no
            // default ("must be set"). pspec normally serializes them.
            // pspec / Unity emit method values like "Identity"/"FX"/"FY"/"FXY"
            // that fab v3 doesn't accept directly. Normalize:
            //   Identity → ID
            //   FX/FY/FXY → ID (the geometric flip is applied via leaf.scale
            //                   negative components at the converter call site)
            let raw_method = m
                .get("_method")
                .map(|v| s(v, &format!("{ctx}.{ty}._method")))
                .transpose()?
                .unwrap_or("ID");
            let (method, fx, fy) = match raw_method {
                "Identity" | "ID" => ("ID".to_string(), false, false),
                "FX" => ("ID".to_string(), true, false),
                "FY" => ("ID".to_string(), false, true),
                "FXY" => ("ID".to_string(), true, true),
                other => (other.to_string(), false, false),
            };
            if ty == UIICON_TYPE {
                Ok((LeafGraphic::UIIcon {
                    sprite_alias,
                    ui_scale,
                    method,
                    border_mult,
                }, fx, fy))
            } else {
                Ok((LeafGraphic::UISlice {
                    sprite_alias,
                    ui_scale,
                    method,
                    border_mult,
                }, fx, fy))
            }
        }
        UISOLID_TYPE => {
            // UISolid in this corpus references a synthesized `Color_<hex>`
            // atlas sprite via `_sprite`. The color is encoded in the alias
            // name (after stripping the atlas `_prefix`), e.g.
            // `$PE_33_Color_32264DBD` → "32264DBD".
            let sprite_alias = m
                .get("_sprite")
                .map(|v| s(v, &format!("{ctx}.UISolid._sprite")))
                .transpose()?
                .map(strip_dollar)
                .ok_or_else(|| Fail::new(format!("{ctx}.UISolid._sprite missing")))?;
            let color_idx = sprite_alias.find("Color_").ok_or_else(|| {
                Fail::new(format!(
                    "{ctx}.UISolid._sprite {sprite_alias:?}: expected to contain 'Color_<hex>'"
                ))
            })?;
            // Strip pspec disambiguator suffixes: `^<dir>` (e.g. `Color_FFFFFF^TreasureTrove`)
            // or `@<atlas>`. Only the hex characters between `Color_` and the
            // first suffix delimiter are meaningful.
            let tail = &sprite_alias[color_idx + "Color_".len()..];
            let color_hex = tail
                .split(['^', '@'])
                .next()
                .unwrap_or(tail);
            if !(color_hex.len() == 6 || color_hex.len() == 8)
                || !color_hex.chars().all(|c| c.is_ascii_hexdigit())
            {
                return Err(Fail::new(format!(
                    "{ctx}.UISolid: color hex {color_hex:?} (from {sprite_alias:?}) \
                     must be 6 or 8 ascii hex"
                )));
            }
            let color8 = if color_hex.len() == 6 {
                format!("{}FF", color_hex.to_ascii_uppercase())
            } else {
                color_hex.to_ascii_uppercase()
            };
            Ok((LeafGraphic::UISolid { color8 }, false, false))
        }
        _ => unreachable!("ty validated by caller"),
    }
}

fn strip_dollar(s: &str) -> String {
    s.strip_prefix('$').unwrap_or(s).to_string()
}

// ---------------------------------------------------------------------------
// Atlas resolution.
//
// Each leaf's sprite_alias is the bare asset name; `unity-assetdb alias`
// resolves it to the `.asset` path. The atlas dir is the parent dir of the
// .asset, and the .tps lives one level up at `<atlas_dir>.tps`.
// The atlas `_prefix` (TPSImporter.Prefix) is read from `<atlas>.tps.meta`
// and stripped from the sprite name to get the tpsheet entry.

#[derive(Debug, Clone)]
struct AtlasInfo {
    /// `.tps` path, project-relative.
    tps_path: PathBuf,
    /// `_prefix` (e.g. "OG_0404_"), empty if none.
    prefix: String,
}

fn resolve_alias_to_atlas(
    meow_client: &Path,
    alias: &str,
    expected_kind: &str, // "Sprite" for UIIcon/UISlice, anything for output
) -> R<(AtlasInfo, String, PathBuf)> {
    // Returns (atlas info, tpsheet entry name (= alias minus _prefix), .asset path).
    //
    // pspec emits two disambiguator suffixes on aliases:
    //   ^<bare_atlas_dir>  — same bare name in multiple atlases. unity-assetdb
    //                        knows this form natively.
    //   @<atlas_name>      — texture is its own atlas, or sprite is in a named
    //                        atlas not directly named after its dir. unity-assetdb
    //                        does NOT know this form; we strip and look up bare.
    let (lookup_alias, atlas_hint_at): (String, Option<String>) = if let Some(idx) = alias.find('@') {
        (alias[..idx].to_string(), Some(alias[idx + 1..].to_string()))
    } else {
        (alias.to_string(), None)
    };
    let hits = assetdb_alias(meow_client, &lookup_alias)?;
    // Prefer a hit of the expected kind (Sprite > Texture2D fallback).
    let hit = hits
        .iter()
        .find(|h| h.kind == expected_kind)
        .or_else(|| hits.iter().find(|h| h.kind == "Texture2D"))
        .or_else(|| hits.first())
        .ok_or_else(|| {
            Fail::new(format!(
                "alias {alias:?} not in asset DB (looked up as {lookup_alias:?})"
            ))
        })?;
    let asset_path = &hit.path;

    // Atlas resolution v2: walk up from the .asset's dir until we find a
    // sibling `.tps` file (the asset DB entry's dir or any ancestor). The
    // first `.tps` whose stem we encounter in a walked dir is the atlas.
    // Handles CommonAtlas (sibling-of-dir layout) and PiggyBank (same-dir
    // layout) uniformly.
    let tps_path = find_atlas_tps(meow_client, asset_path, atlas_hint_at.as_deref())?;

    let mut abs_tps_meta = meow_client.join(&tps_path);
    abs_tps_meta.as_mut_os_string().push(".meta");
    let prefix = read_tps_meta_prefix(&abs_tps_meta).unwrap_or_default();

    // tpsheet entry name = .asset filename stem minus atlas prefix. The
    // asset filename is the canonical bare name (no `^<suffix>` /
    // `@<atlas>` disambiguators that the alias-form may have carried).
    let bare = asset_path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| Fail::new(format!("alias {alias:?} asset path has no stem")))?;
    let entry_name = bare.strip_prefix(prefix.as_str()).unwrap_or(bare).to_string();

    Ok((AtlasInfo { tps_path, prefix }, entry_name, asset_path.clone()))
}

fn find_atlas_tps(meow_client: &Path, asset_path: &Path, hint: Option<&str>) -> R<PathBuf> {
    // Walk up from the .asset's parent. At each level try, in order:
    //   1. `<cur.parent>/<cur.basename>.tps`  — the sibling-of-dir layout
    //      used by CommonAtlas-style atlases (sprite dir + sibling .tps).
    //   2. `<cur>/<hint>.tps` if a hint was supplied, then any `<cur>/*.tps`
    //      if exactly one exists. Handles PiggyBank-style (.tps + sprite_dir
    //      both inside the same parent).
    // Walks up until Assets root.
    let mut cur = asset_path.parent().map(Path::to_path_buf);
    while let Some(dir) = cur {
        // (1) sibling-stem.tps at parent level
        let dir_name = dir.file_name().and_then(|s| s.to_str()).map(str::to_string);
        let parent = dir.parent().map(Path::to_path_buf);
        if let (Some(name), Some(p)) = (dir_name.clone(), parent.clone()) {
            let candidate = p.join(format!("{name}.tps"));
            if meow_client.join(&candidate).exists() {
                return Ok(candidate);
            }
        }
        // (2) .tps inside the current dir
        let abs_dir = meow_client.join(&dir);
        let mut tps_hits: Vec<PathBuf> = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&abs_dir) {
            for e in entries.flatten() {
                if e.path().extension().and_then(|s| s.to_str()) == Some("tps") {
                    tps_hits.push(dir.join(e.file_name()));
                }
            }
        }
        if let Some(h) = hint {
            if let Some(matched) = tps_hits
                .iter()
                .find(|p| p.file_stem().and_then(|s| s.to_str()) == Some(h))
            {
                return Ok(matched.clone());
            }
        }
        if tps_hits.len() == 1 {
            return Ok(tps_hits.into_iter().next().unwrap());
        }
        if tps_hits.len() > 1 {
            return Err(Fail::new(format!(
                "ambiguous atlas: dir {dir:?} contains multiple .tps files: {tps_hits:?} (hint={hint:?})"
            )));
        }
        // No hits this level — walk up.
        let next = parent;
        if next.as_deref() == Some(Path::new("")) || next.is_none() {
            break;
        }
        cur = next;
    }
    Err(Fail::new(format!(
        "no .tps found walking up from {asset_path:?}"
    )))
}

fn read_tps_meta_prefix(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    for line in text.lines() {
        let t = line.trim_start();
        if let Some(rest) = t.strip_prefix("_prefix:") {
            return Some(rest.trim().to_string());
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Color helpers.

fn fmt_color(color8: &str) -> String {
    // 8-hex when alpha != FF, 6-hex when FF. The fab v3 schema accepts both.
    if color8.len() == 8 && color8[6..].eq_ignore_ascii_case("FF") {
        color8[..6].to_ascii_uppercase()
    } else {
        color8.to_ascii_uppercase()
    }
}

// ---------------------------------------------------------------------------
// v3 fab.json emitter.

fn fmt_f(v: f32) -> String {
    // Match the rest of the crate's emit: Rust default Display, which is
    // round-trip-stable for f32 (matches C#'s ToString("R")).
    let s = format!("{v}");
    if !s.contains('.') && !s.contains('e') && !s.contains('E') {
        return s;
    }
    s
}

fn fmt_xy(a: [f32; 2]) -> String {
    format!("[{}, {}]", fmt_f(a[0]), fmt_f(a[1]))
}

fn write_tree(
    p: &PrefabCsa,
    sprite_resolutions: &BTreeMap<String, String>, // alias → atlas-relative entry name
    out: &mut String,
) -> R<()> {
    out.push_str("    {\n");
    out.push_str(&format!(
        "      \"name\": \"{}\",\n",
        p.output_sprite_alias
    ));
    out.push_str("      \"mode\": \"ui\",\n");
    // Default scale = 1.0; emit explicitly when it differs (CSA canvas
    // prefabs declare `scale: 0.01`).
    if p.csa_scale != 1.0 {
        out.push_str(&format!("      \"scale\": {},\n", fmt_f(p.csa_scale)));
    }
    // Non-origin CSA roots are intentionally rejected by the new manifest
    // schema (the FMA residual path that captured Unity's `Mesh.CombineMeshes`
    // 1-ULP shift was retired). Surface as an error so the author repins the
    // root in the prefab.
    if p.root_anchored != [0.0, 0.0] {
        return Err(Fail(format!(
            "{}: CSA root at non-origin {:?} — pin the prefab's root \
             RectTransform.anchoredPosition to (0, 0) before re-running",
            p.output_sprite_alias, p.root_anchored
        )));
    }
    out.push_str("      \"children\": [\n");
    for (i, leaf) in p.children.iter().enumerate() {
        write_leaf(leaf, sprite_resolutions, out)?;
        if i + 1 < p.children.len() {
            out.push_str(",\n");
        } else {
            out.push('\n');
        }
    }
    out.push_str("      ]\n    }");
    Ok(())
}

fn write_leaf(
    leaf: &LeafNode,
    sprite_resolutions: &BTreeMap<String, String>,
    out: &mut String,
) -> R<()> {
    out.push_str("        {");
    let mut sep = " ";
    if leaf.pos != [0.0, 0.0] {
        out.push_str(&format!("{sep}\"pos\": {}", fmt_xy(leaf.pos)));
        sep = ", ";
    }
    // sizeDelta is only meaningful for size-fitted (stretchable) methods
    // — mirror/tile/slice. Native-scale methods (ID + UISolid polygon,
    // which absorbs sizeDelta into its vertices) get it dropped.
    let emit_size_delta = match &leaf.graphic {
        Some(LeafGraphic::UIIcon { method, .. }) | Some(LeafGraphic::UISlice { method, .. }) => {
            method != "ID"
        }
        Some(LeafGraphic::UISolid { .. }) => false,
        None => false,
    };
    if emit_size_delta && leaf.size_delta != [0.0, 0.0] {
        out.push_str(&format!("{sep}\"sizeDelta\": {}", fmt_xy(leaf.size_delta)));
        sep = ", ";
    }
    if leaf.pivot != [0.5, 0.5] {
        out.push_str(&format!("{sep}\"pivot\": {}", fmt_xy(leaf.pivot)));
        sep = ", ";
    }
    if leaf.scale != [1.0, 1.0] {
        if leaf.scale[0] == leaf.scale[1] {
            out.push_str(&format!("{sep}\"scale\": {}", fmt_f(leaf.scale[0])));
        } else {
            out.push_str(&format!("{sep}\"scale\": {}", fmt_xy(leaf.scale)));
        }
        sep = ", ";
    }
    if leaf.rot_deg != 0.0 {
        out.push_str(&format!("{sep}\"rotDeg\": {}", fmt_f(leaf.rot_deg)));
        sep = ", ";
    }
    if let Some(g) = &leaf.graphic {
        write_graphic_flat(g, leaf.size_delta, leaf.pivot, sep, sprite_resolutions, out)?;
        sep = ", ";
    }
    if !leaf.children.is_empty() {
        out.push_str(&format!("{sep}\"children\": [\n"));
        for (i, c) in leaf.children.iter().enumerate() {
            write_leaf(c, sprite_resolutions, out)?;
            if i + 1 < leaf.children.len() {
                out.push_str(",\n");
            } else {
                out.push('\n');
            }
        }
        out.push_str("        ]");
    }
    out.push_str(" }");
    Ok(())
}

fn write_graphic_flat(
    g: &LeafGraphic,
    leaf_size: [f32; 2],
    leaf_pivot: [f32; 2],
    initial_sep: &str,
    sprite_resolutions: &BTreeMap<String, String>,
    out: &mut String,
) -> R<()> {
    match g {
        LeafGraphic::UIIcon {
            sprite_alias,
            ui_scale,
            method,
            border_mult,
        }
        | LeafGraphic::UISlice {
            sprite_alias,
            ui_scale,
            method,
            border_mult,
        } => {
            let entry = sprite_resolutions.get(sprite_alias).ok_or_else(|| {
                Fail::new(format!("unresolved sprite alias {sprite_alias:?}"))
            })?;
            out.push_str(&format!(
                "{initial_sep}\"type\": \"sprite\", \"sprite\": \"{}\"",
                entry
            ));
            if method != "ID" {
                out.push_str(&format!(", \"method\": \"{}\"", method));
            }
            if *ui_scale != 1.0 {
                out.push_str(&format!(", \"uiScale\": {}", fmt_f(*ui_scale)));
            }
            if *border_mult != 1.0 {
                out.push_str(&format!(", \"borderMult\": {}", fmt_f(*border_mult)));
            }
        }
        LeafGraphic::UISolid { color8 } => {
            // Pivot-relative BL/BR/TL/TR quad of the leaf's sizeDelta.
            // Matches the Silloutte v3 fixture's order; the [0,2,3,3,1,0]
            // triangle list override is required because the ear-clipper
            // sees this order as self-crossing.
            let (sx, sy) = (leaf_size[0], leaf_size[1]);
            let (px, py) = (leaf_pivot[0], leaf_pivot[1]);
            let bl = [-px * sx, -py * sy];
            let br = [(1.0 - px) * sx, -py * sy];
            let tl = [-px * sx, (1.0 - py) * sy];
            let tr = [(1.0 - px) * sx, (1.0 - py) * sy];
            out.push_str(&format!(
                "{initial_sep}\"type\": \"polygon\", \"color\": \"{}\", \"vertices\": [{}, {}, {}, {}], \"triangles\": [0, 2, 3, 3, 1, 0]",
                fmt_color(color8),
                fmt_xy(bl),
                fmt_xy(br),
                fmt_xy(tl),
                fmt_xy(tr),
            ));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Main.

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let meow_client = args
        .get(1)
        .map(PathBuf::from)
        .or_else(|| std::env::var("MEOW_CLIENT").ok().map(PathBuf::from))
        .expect("usage: csa_to_fab <MEOW_CLIENT_PATH> [--write]");
    let write_mode = args.iter().any(|a| a == "--write");

    if let Err(e) = run(&meow_client, write_mode) {
        eprintln!("csa_to_fab: {e}");
        std::process::exit(1);
    }
}

fn run(meow_client: &Path, write_mode: bool) -> R<()> {
    eprintln!("discovering CSA prefabs via unity-assetdb…");
    let prefabs = assetdb_usage(meow_client, CSA_GUID)?;
    eprintln!("  found {} prefabs", prefabs.len());

    // Per-prefab parse + per-leaf alias resolution. Atlas groups built as
    // we go: atlas_tps_path → Vec<PrefabCsa>.
    let mut by_atlas: BTreeMap<PathBuf, Vec<(PrefabCsa, BTreeMap<String, String>)>> =
        BTreeMap::new();
    let mut alias_cache: BTreeMap<String, (AtlasInfo, String)> = BTreeMap::new();

    let mut ok = 0;
    let mut failed = Vec::new();

    for prefab in &prefabs {
        let result = (|| -> R<()> {
            let pspec = pspec_convert(meow_client, prefab)?;
            let p = parse_prefab(&pspec, prefab)?;
            // Atlas is decided by the leaves' sprite atlas, not the CSA
            // output: CSA outputs live in a holding `Authoring/` dir; the
            // migration places them into the leaves' atlas sprite_dir.
            let mut resolutions: BTreeMap<String, String> = BTreeMap::new();
            let mut atlas: Option<AtlasInfo> = None;
            resolve_subtree(
                meow_client,
                prefab,
                &p.children,
                &mut alias_cache,
                &mut resolutions,
                &mut atlas,
            )?;
            let atlas = atlas.ok_or_else(|| {
                Fail::new(format!(
                    "{:?}: no UIIcon/UISlice leaves — cannot determine atlas",
                    prefab
                ))
            })?;
            // The pipeline applies the atlas's `_prefix` to every combined
            // sprite's name when writing. The pspec output sprite alias
            // ALREADY carries the prefix, so strip it before emit to avoid
            // double-prefixing (`AC_` + `AC_IC_Cat` → `AC_AC_IC_Cat`).
            let mut p = p;
            if !atlas.prefix.is_empty() {
                if let Some(bare) = p.output_sprite_alias.strip_prefix(&atlas.prefix) {
                    p.output_sprite_alias = bare.to_string();
                }
            }
            by_atlas
                .entry(atlas.tps_path)
                .or_default()
                .push((p, resolutions));
            Ok(())
        })();
        match result {
            Ok(()) => ok += 1,
            Err(e) => failed.push((prefab.clone(), e)),
        }
    }

    eprintln!("\n--- summary ---");
    eprintln!("parsed:  {ok}");
    eprintln!("failed:  {}", failed.len());
    for (p, e) in &failed {
        eprintln!("  {}: {e}", p.display());
    }

    // Don't abort on failures — write the converters that succeeded; print
    // the failure list to stderr so the caller can decide whether to retry
    // or accept the partial migration. The eventual exit code reflects the
    // failure count (caller can check $? after `--write`).
    let had_failures = !failed.is_empty();

    // Emit per atlas.
    for (tps_path, prefabs) in &by_atlas {
        let mut out = String::new();
        out.push_str("{\n  \"version\": 1,\n  \"trees\": [\n");
        for (i, (p, res)) in prefabs.iter().enumerate() {
            write_tree(p, res, &mut out)?;
            if i + 1 < prefabs.len() {
                out.push_str(",\n");
            } else {
                out.push('\n');
            }
        }
        out.push_str("  ]\n}\n");

        let fab_path = {
            let mut p = meow_client.join(tps_path);
            p.as_mut_os_string().push(".fab.json");
            p
        };
        if write_mode {
            std::fs::write(&fab_path, &out)
                .map_err(|e| Fail::new(format!("write {fab_path:?}: {e}")))?;
            eprintln!("wrote {} ({} prefabs)", fab_path.display(), prefabs.len());
        } else {
            eprintln!("=== {} ({} prefabs) ===", tps_path.display(), prefabs.len());
            println!("{out}");
        }
    }

    if had_failures {
        eprintln!(
            "\nNOTE: {} prefab(s) failed conversion — see list above. The {} \
             successful prefabs were emitted.",
            failed.len(),
            ok
        );
    }

    Ok(())
}

fn resolve_subtree(
    meow_client: &Path,
    prefab: &Path,
    leaves: &[LeafNode],
    cache: &mut BTreeMap<String, (AtlasInfo, String)>,
    resolutions: &mut BTreeMap<String, String>,
    atlas: &mut Option<AtlasInfo>,
) -> R<()> {
    for leaf in leaves {
        if let Some(g) = &leaf.graphic {
            match g {
                LeafGraphic::UIIcon { sprite_alias, .. }
                | LeafGraphic::UISlice { sprite_alias, .. } => {
                    let (leaf_atlas, entry, _) =
                        resolve_with_cache(meow_client, sprite_alias, "Sprite", cache)?;
                    match atlas {
                        None => *atlas = Some(leaf_atlas.clone()),
                        Some(a) if a.tps_path != leaf_atlas.tps_path => {
                            return Err(Fail::new(format!(
                                "{:?}: leaf {sprite_alias:?} → atlas {:?}, but prior \
                                 leaves use atlas {:?} — multi-atlas prefabs unsupported",
                                prefab, leaf_atlas.tps_path, a.tps_path
                            )));
                        }
                        _ => {}
                    }
                    resolutions.insert(sprite_alias.clone(), entry);
                }
                LeafGraphic::UISolid { .. } => {}
            }
        }
        resolve_subtree(meow_client, prefab, &leaf.children, cache, resolutions, atlas)?;
    }
    Ok(())
}

fn resolve_with_cache(
    meow_client: &Path,
    alias: &str,
    expected_kind: &str,
    cache: &mut BTreeMap<String, (AtlasInfo, String)>,
) -> R<(AtlasInfo, String, PathBuf)> {
    if let Some((atlas, entry)) = cache.get(alias) {
        return Ok((atlas.clone(), entry.clone(), PathBuf::new()));
    }
    let (atlas, entry, asset_path) = resolve_alias_to_atlas(meow_client, alias, expected_kind)?;
    cache.insert(alias.to_string(), (atlas.clone(), entry.clone()));
    Ok((atlas, entry, asset_path))
}
