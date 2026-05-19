//! Editable document. Wraps `manifest::Manifest`, tracks dirty state, owns
//! the sibling atlas (lazy). Save round-trips via `crate::serialize::serialize`.

use crate::atlas::{Atlas, AtlasError};
use crate::serialize;
use std::fs;
use std::path::{Path, PathBuf};
use unity_sprite_author::manifest::{self, Graphic, Manifest, Node};

pub struct Doc {
    pub path: PathBuf,
    pub manifest: Manifest,
    /// Lazy atlas (sibling `.tpsheet` + `.png`). `None` until first request;
    /// `Some(Err)` if a load was attempted and failed, so we don't retry on
    /// every frame.
    pub atlas: Option<Result<Atlas, AtlasError>>,
    pub dirty: bool,
}

#[derive(Debug)]
pub enum LoadError {
    Io(std::io::Error),
    Parse(manifest::ManifestError),
}

#[derive(Debug)]
pub enum SaveError {
    Io(std::io::Error),
    /// Editor state failed to re-parse via the canonical parser — indicates a
    /// bug in our serializer or invalid state the user produced.
    Validate(manifest::ManifestError),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "io: {e}"),
            Self::Parse(e) => write!(f, "parse: {e}"),
        }
    }
}

impl std::fmt::Display for SaveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "io: {e}"),
            Self::Validate(e) => write!(f, "validate: {e}"),
        }
    }
}

impl Doc {
    pub fn open(path: &Path) -> Result<Self, LoadError> {
        let bytes = fs::read_to_string(path).map_err(LoadError::Io)?;
        let mut manifest = manifest::parse(&bytes).map_err(LoadError::Parse)?;
        // Files authored manually (or by older tooling) sometimes carry a
        // `type: "sprite"` leaf whose `sprite` is `Color_*` — semantically
        // that's a polygon-fill, not an atlas sprite. Normalize on load so
        // the inspector dropdown + preview path treat it correctly. Save
        // will reflect the normalized shape; reload is idempotent.
        normalize_color_sprites(&mut manifest);
        Ok(Self {
            path: path.to_path_buf(),
            manifest,
            atlas: None,
            dirty: false,
        })
    }

    pub fn save(&mut self) -> Result<(), SaveError> {
        let text = serialize::serialize(&self.manifest);
        manifest::parse(&text).map_err(SaveError::Validate)?;
        fs::write(&self.path, text).map_err(SaveError::Io)?;
        self.dirty = false;
        Ok(())
    }

    /// Borrow the atlas, loading on first access. Returns `Err` on failure;
    /// callers usually want to surface this in the UI as a "no atlas — sprite
    /// picker unavailable" affordance.
    pub fn atlas_mut(&mut self) -> &mut Result<Atlas, AtlasError> {
        if self.atlas.is_none() {
            self.atlas = Some(Atlas::load_for_fab_json(&self.path));
        }
        self.atlas.as_mut().unwrap()
    }
}

/// In-place normalization: convert sprite leaves whose `sprite` reference
/// starts with `Color_` into rect-shape polygon leaves with that color.
/// The original sprite's `method` / `border_mult` / `flip_x` / `flip_y`
/// fields don't apply to a flat color, so they're dropped. Position,
/// rotation, scale, pivot, size, name, and children are preserved.
pub fn normalize_color_sprites(m: &mut Manifest) {
    for tree in &mut m.trees {
        normalize_node(&mut tree.root);
    }
}

fn normalize_node(node: &mut Node) {
    if let Some(Graphic::Sprite { sprite, .. }) = &node.graphic {
        if sprite.starts_with("Color_") {
            let polygon_sprite = sprite.clone();
            node.graphic = Some(Graphic::Polygon {
                polygon_sprite,
                // Default rect-shape: 2×2 centered. The user can resize via
                // the 9-way handles or the inspector.
                vertices: vec![[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]],
                triangles: Some(vec![0, 2, 3, 3, 1, 0]),
            });
        }
    }
    for c in &mut node.children {
        normalize_node(c);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_mini(s: &str) -> Manifest {
        manifest::parse(s).unwrap()
    }

    #[test]
    fn normalize_sprite_with_color_ref_to_rect() {
        // Sprite leaf with sprite="Color_B73F3E" → should become a Polygon
        // (rect-shape) leaf with polygon_sprite preserved.
        let mut m = parse_mini(r#"{"version":1,"combined":[{
            "name":"X","mode":"ui","children":[
                {"type":"sprite","sprite":"Color_B73F3E"}
            ]}]}"#);
        normalize_color_sprites(&mut m);
        match &m.trees[0].root.children[0].graphic {
            Some(Graphic::Polygon { polygon_sprite, vertices, triangles }) => {
                assert_eq!(polygon_sprite, "Color_B73F3E");
                assert_eq!(vertices.len(), 4);
                assert!(triangles.is_some());
            }
            other => panic!("expected Polygon, got {other:?}"),
        }
    }

    #[test]
    fn normalize_preserves_non_color_sprite_leaves() {
        let mut m = parse_mini(r#"{"version":1,"combined":[{
            "name":"X","mode":"ui","children":[
                {"type":"sprite","sprite":"Foo"}
            ]}]}"#);
        normalize_color_sprites(&mut m);
        match &m.trees[0].root.children[0].graphic {
            Some(Graphic::Sprite { sprite, .. }) => assert_eq!(sprite, "Foo"),
            _ => panic!("non-color sprite should stay as-is"),
        }
    }

    #[test]
    fn normalize_recurses_into_descendants() {
        let mut m = parse_mini(r#"{"version":1,"combined":[{
            "name":"X","mode":"ui","children":[
                {"name":"Body","children":[
                    {"type":"sprite","sprite":"Color_FF0000"}
                ]}
            ]}]}"#);
        normalize_color_sprites(&mut m);
        let body = &m.trees[0].root.children[0];
        match &body.children[0].graphic {
            Some(Graphic::Polygon { polygon_sprite, .. }) => assert_eq!(polygon_sprite, "Color_FF0000"),
            _ => panic!("descendant sprite-color should be converted"),
        }
    }

    #[test]
    fn normalize_preserves_transform_fields() {
        let mut m = parse_mini(r#"{"version":1,"combined":[{
            "name":"X","mode":"ui","children":[
                {"type":"sprite","sprite":"Color_FF0000","pos":[12, 34],"rotDegCCW":15}
            ]}]}"#);
        normalize_color_sprites(&mut m);
        let n = &m.trees[0].root.children[0];
        assert_eq!(n.pos, [12.0, 34.0]);
        assert_eq!(n.rot_deg_ccw, 15.0);
    }

    #[test]
    fn normalize_is_idempotent() {
        let mut m = parse_mini(r#"{"version":1,"combined":[{
            "name":"X","mode":"ui","children":[
                {"type":"sprite","sprite":"Color_FF0000"}
            ]}]}"#);
        normalize_color_sprites(&mut m);
        let snapshot = m.clone();
        normalize_color_sprites(&mut m);
        assert_eq!(m, snapshot);
    }
}

/// Stable cursor into the tree under edit: doc index in the App's vec +
/// tree index + chain of child indices into `Node.children`. The empty
/// chain selects the synthesized root container.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct NodePath {
    pub doc: usize,
    pub tree: usize,
    pub child_chain: Vec<usize>,
}

impl NodePath {
    pub fn tree_root(doc: usize, tree: usize) -> Self {
        Self { doc, tree, child_chain: Vec::new() }
    }

    pub fn child(&self, idx: usize) -> Self {
        let mut chain = self.child_chain.clone();
        chain.push(idx);
        Self { doc: self.doc, tree: self.tree, child_chain: chain }
    }

    pub fn parent(&self) -> Option<Self> {
        if self.child_chain.is_empty() {
            None
        } else {
            let mut chain = self.child_chain.clone();
            chain.pop();
            Some(Self { doc: self.doc, tree: self.tree, child_chain: chain })
        }
    }

    pub fn resolve<'a>(&self, m: &'a Manifest) -> Option<&'a manifest::Node> {
        let tree = m.trees.get(self.tree)?;
        let mut node = &tree.root;
        for &idx in &self.child_chain {
            node = node.children.get(idx)?;
        }
        Some(node)
    }

    pub fn resolve_mut<'a>(&self, m: &'a mut Manifest) -> Option<&'a mut manifest::Node> {
        let tree = m.trees.get_mut(self.tree)?;
        let mut node = &mut tree.root;
        for &idx in &self.child_chain {
            node = node.children.get_mut(idx)?;
        }
        Some(node)
    }
}
