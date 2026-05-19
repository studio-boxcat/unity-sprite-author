//! Persisted UI preferences. Round-tripped through `eframe::Storage` (RON
//! file under the OS app-data dir) — loaded once at startup, saved on
//! shutdown + periodic autosave by eframe itself.
//!
//! Every field uses `#[serde(default)]` (via the struct-level attribute) so
//! adding a new toggle in code Just Works against an older on-disk file —
//! missing fields fall back to `Default`. That's the contract you rely on
//! when introducing new settings: don't remove or rename fields lightly;
//! rotting field names mean older configs silently lose values.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    // ---- Preview canvas toggles ----
    pub show_polygon: bool,
    pub show_pivot_markers: bool,
    pub show_part_outlines: bool,
    pub show_atlas_aabb: bool,

    // ---- File dialogs ----
    /// Last directory used in File > Open — File dialog defaults here.
    pub last_open_dir: Option<PathBuf>,
    /// Most-recently-opened fab.json paths (newest first, capped at 8).
    pub recent_files: Vec<PathBuf>,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            show_polygon: true,
            show_pivot_markers: true,
            show_part_outlines: true,
            show_atlas_aabb: true,
            last_open_dir: None,
            recent_files: Vec::new(),
        }
    }
}

impl Preferences {
    /// Key used to round-trip through `eframe::Storage`. Bumping this string
    /// invalidates every existing user's settings on next launch — only do
    /// that for a deliberate schema break.
    pub const STORAGE_KEY: &'static str = "preferences";

    /// Load from storage, falling back to defaults. eframe's `get_value`
    /// returns `None` both for "key missing" and "deserialize failed", so an
    /// incompatible struct change after a release silently resets the user.
    /// Acceptable for non-critical UI prefs; never put unrecoverable state
    /// here.
    pub fn load(storage: Option<&dyn eframe::Storage>) -> Self {
        storage
            .and_then(|s| eframe::get_value::<Preferences>(s, Self::STORAGE_KEY))
            .unwrap_or_default()
    }

    pub fn save_to(&self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, Self::STORAGE_KEY, self);
    }

    /// Push a path to the front of `recent_files`, dedup + cap to 8.
    pub fn note_open(&mut self, path: PathBuf) {
        self.recent_files.retain(|p| p != &path);
        self.recent_files.insert(0, path.clone());
        self.recent_files.truncate(8);
        if let Some(parent) = path.parent() {
            self.last_open_dir = Some(parent.to_path_buf());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_on_for_show_toggles() {
        let p = Preferences::default();
        assert!(p.show_polygon);
        assert!(p.show_pivot_markers);
        assert!(p.show_part_outlines);
        assert!(p.show_atlas_aabb);
        assert!(p.recent_files.is_empty());
    }

    #[test]
    fn note_open_dedupes_and_caps() {
        let mut p = Preferences::default();
        for i in 0..10 {
            p.note_open(PathBuf::from(format!("/tmp/{i}.json")));
        }
        assert_eq!(p.recent_files.len(), 8);
        // Most-recent first.
        assert_eq!(p.recent_files[0], PathBuf::from("/tmp/9.json"));
        // Re-opening an existing path bubbles it to the front without growing.
        p.note_open(PathBuf::from("/tmp/3.json"));
        assert_eq!(p.recent_files[0], PathBuf::from("/tmp/3.json"));
        assert_eq!(p.recent_files.len(), 8);
    }

    #[test]
    fn note_open_updates_last_open_dir() {
        let mut p = Preferences::default();
        p.note_open(PathBuf::from("/foo/bar/baz.json"));
        assert_eq!(p.last_open_dir, Some(PathBuf::from("/foo/bar")));
    }

    #[test]
    fn forward_compat_missing_field_uses_default() {
        // Empty object → every field falls back via `#[serde(default)]`.
        let parsed: Preferences = serde_json::from_str("{}").expect("parse");
        assert!(parsed.show_polygon);
        assert!(parsed.recent_files.is_empty());

        // A subset of fields → only the rest defaults; supplied fields win.
        let parsed: Preferences = serde_json::from_str(
            r#"{"show_polygon": false, "show_pivot_markers": false}"#,
        ).expect("parse");
        assert!(!parsed.show_polygon);
        assert!(!parsed.show_pivot_markers);
        assert!(parsed.show_part_outlines, "defaulted");
    }
}
