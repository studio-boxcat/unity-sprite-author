//! Editable document. Wraps `manifest::Manifest`, tracks dirty state, owns
//! the sibling atlas (lazy). Save round-trips via `crate::serialize::serialize`.

use crate::atlas::{Atlas, AtlasError};
use crate::serialize;
use std::fs;
use std::path::{Path, PathBuf};
use unity_sprite_author::manifest::{self, Manifest};

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
        let manifest = manifest::parse(&bytes).map_err(LoadError::Parse)?;
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
