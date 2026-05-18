//! Multi-selection state. Helpers expose Finder/VSCode-style behaviors:
//! `set_single` (plain click), `toggle` (Cmd-click), `extend` (Shift-click),
//! `replace_with` (marquee). The last-inserted path is the *primary* — what
//! the inspector / pickers operate on when the model requires a single anchor.

use crate::doc::NodePath;
use std::collections::HashSet;

#[derive(Debug, Clone, Default)]
pub struct Selection {
    items: Vec<NodePath>,
}

impl Selection {
    /// Most-recently-added selection. Inspector + pickers anchor on this.
    pub fn primary(&self) -> Option<&NodePath> {
        self.items.last()
    }

    pub fn is_selected(&self, p: &NodePath) -> bool {
        self.items.iter().any(|q| q == p)
    }

    /// Single-click semantic: clear everything else and select just `p`.
    pub fn set_single(&mut self, p: NodePath) {
        self.items.clear();
        self.items.push(p);
    }

    /// Cmd-click semantic: toggle membership without disturbing the rest.
    /// Toggling a selected item makes the previous item the new primary.
    pub fn toggle(&mut self, p: NodePath) {
        if let Some(pos) = self.items.iter().position(|q| q == &p) {
            self.items.remove(pos);
        } else {
            self.items.push(p);
        }
    }

    /// Shift-click semantic: add without removing existing. Re-adding bumps
    /// the path to primary (move to end).
    pub fn extend(&mut self, p: NodePath) {
        if let Some(pos) = self.items.iter().position(|q| q == &p) {
            let existing = self.items.remove(pos);
            self.items.push(existing);
        } else {
            self.items.push(p);
        }
    }

    /// Marquee semantic: replace selection with the supplied set.
    pub fn replace_with<I: IntoIterator<Item = NodePath>>(&mut self, paths: I) {
        let mut seen: HashSet<NodePath> = HashSet::new();
        self.items.clear();
        for p in paths {
            if seen.insert(p.clone()) {
                self.items.push(p);
            }
        }
    }

    pub fn clear(&mut self) {
        self.items.clear();
    }

    pub fn iter(&self) -> impl Iterator<Item = &NodePath> {
        self.items.iter()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    #[cfg(test)]
    pub fn as_slice(&self) -> &[NodePath] {
        &self.items
    }

    /// Filter out paths whose ancestor is also selected — used before
    /// multi-delete so we don't try to resolve a path whose ancestor was just
    /// removed (the ancestor takes the subtree with it).
    pub fn without_descendants_of_selected(&self) -> Vec<NodePath> {
        let mut out = Vec::with_capacity(self.items.len());
        for p in &self.items {
            let mut shadowed = false;
            for q in &self.items {
                if p != q && path_is_descendant_of(p, q) {
                    shadowed = true;
                    break;
                }
            }
            if !shadowed {
                out.push(p.clone());
            }
        }
        out
    }
}

/// True iff `descendant` lives strictly below `ancestor` in the same tree.
pub fn path_is_descendant_of(descendant: &NodePath, ancestor: &NodePath) -> bool {
    descendant.doc == ancestor.doc
        && descendant.tree == ancestor.tree
        && descendant.child_chain.len() > ancestor.child_chain.len()
        && descendant.child_chain.starts_with(&ancestor.child_chain)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(chain: &[usize]) -> NodePath {
        NodePath { doc: 0, tree: 0, child_chain: chain.to_vec() }
    }

    #[test]
    fn set_single_replaces() {
        let mut s = Selection::default();
        s.set_single(p(&[1]));
        s.set_single(p(&[2]));
        assert_eq!(s.as_slice(), &[p(&[2])]);
        assert_eq!(s.primary(), Some(&p(&[2])));
    }

    #[test]
    fn toggle_adds_then_removes() {
        let mut s = Selection::default();
        s.toggle(p(&[0]));
        s.toggle(p(&[1]));
        assert_eq!(s.as_slice(), &[p(&[0]), p(&[1])]);
        s.toggle(p(&[0]));
        assert_eq!(s.as_slice(), &[p(&[1])]);
    }

    #[test]
    fn extend_promotes_to_primary() {
        let mut s = Selection::default();
        s.extend(p(&[0]));
        s.extend(p(&[1]));
        s.extend(p(&[0])); // bumps [0] to primary
        assert_eq!(s.primary(), Some(&p(&[0])));
        assert_eq!(s.as_slice(), &[p(&[1]), p(&[0])]);
    }

    #[test]
    fn descendants_dropped_when_ancestor_selected() {
        let mut s = Selection::default();
        s.set_single(p(&[0, 1, 2])); // child
        s.extend(p(&[0, 1]));        // parent
        s.extend(p(&[3]));           // unrelated
        let pruned = s.without_descendants_of_selected();
        assert_eq!(pruned, vec![p(&[0, 1]), p(&[3])]);
    }

    #[test]
    fn replace_with_dedupes() {
        let mut s = Selection::default();
        s.replace_with([p(&[0]), p(&[1]), p(&[0])]);
        assert_eq!(s.as_slice(), &[p(&[0]), p(&[1])]);
    }
}
