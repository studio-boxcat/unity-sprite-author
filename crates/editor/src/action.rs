//! Central action dispatch. Every user-facing command flows through one
//! `Action` enum — keyboard shortcuts, menu items, command-palette entries,
//! tree right-click menus, canvas interactions all push an `Action` and
//! `App::dispatch` routes it.
//!
//! Why this exists: adding a new feature used to mean wiring the same
//! behavior into 3-4 places (keyboard, native menu, in-window menu,
//! palette). With this layer, "add a feature" = add an `Action` variant +
//! one `match` arm + (optionally) a `CommandEntry` row.

use crate::ops::NewGraphic;

/// One unit of user intent. Parameterless variants surface in the command
/// palette; parameterized ones (path-aware, key-aware) get emitted by the
/// UI surface that has the necessary context.
#[derive(Debug, Clone)]
pub enum Action {
    // ---- File ----
    OpenDialog,
    SaveActive,
    SaveAll,
    CloseActiveTab,

    // ---- Edit (history) ----
    Undo,
    Redo,

    // ---- Tree (selection-relative; palette-callable) ----
    AddUnderSelection(NewGraphic),
    DuplicateSelection,
    DeleteSelection,

    // ---- View toggles + canvas controls ----
    Fit,
    ToggleShowPolygon,
    ToggleShowPivot,
    ToggleShowOutlines,
    ToggleShowAABB,

    // ---- Tabs ----
    NextTab,
    PrevTab,

    // ---- Modal / palette ----
    OpenPalette,
}

/// A palette-listable command. Parameterless `Action`s only — anything that
/// requires runtime context (a path, a key, a value) doesn't belong here.
pub struct CommandEntry {
    pub label: &'static str,
    /// Lowercase keywords for fuzzy filter. Include synonyms.
    pub keywords: &'static [&'static str],
    /// Display-only shortcut hint (e.g. "⌘O"). Doesn't actually bind the
    /// shortcut — bindings live in `App::handle_shortcuts`.
    pub accelerator: Option<&'static str>,
    pub action_factory: fn() -> Action,
}

/// Static registry of palette-callable commands. `action_factory` returns
/// the action fresh each invocation so `Action::Clone` isn't required and
/// non-`Copy` payloads (paths) can be added later.
pub fn commands() -> &'static [CommandEntry] {
    &[
        CommandEntry { label: "Open File…",                keywords: &["open", "file"],            accelerator: Some("⌘O"),   action_factory: || Action::OpenDialog },
        CommandEntry { label: "Save",                      keywords: &["save"],                     accelerator: Some("⌘S"),   action_factory: || Action::SaveActive },
        CommandEntry { label: "Save All",                  keywords: &["save", "all"],              accelerator: Some("⌘⇧S"), action_factory: || Action::SaveAll },
        CommandEntry { label: "Close Tab",                 keywords: &["close", "tab"],             accelerator: Some("⌘W"),   action_factory: || Action::CloseActiveTab },

        CommandEntry { label: "Undo",                      keywords: &["undo"],                     accelerator: Some("⌘Z"),   action_factory: || Action::Undo },
        CommandEntry { label: "Redo",                      keywords: &["redo"],                     accelerator: Some("⌘⇧Z"), action_factory: || Action::Redo },

        CommandEntry { label: "New Sprite",                keywords: &["new", "sprite"],            accelerator: Some("⌘N"),   action_factory: || Action::AddUnderSelection(NewGraphic::Sprite) },
        CommandEntry { label: "New Container",             keywords: &["new", "container", "group"], accelerator: Some("⌘⇧N"), action_factory: || Action::AddUnderSelection(NewGraphic::Container) },
        CommandEntry { label: "New Rect",                  keywords: &["new", "rect", "rectangle"], accelerator: None,         action_factory: || Action::AddUnderSelection(NewGraphic::Rect) },
        CommandEntry { label: "New Polygon",               keywords: &["new", "polygon"],           accelerator: None,         action_factory: || Action::AddUnderSelection(NewGraphic::Polygon) },
        CommandEntry { label: "New SpriteRenderer (SMA)",  keywords: &["new", "sprite", "renderer", "sma"], accelerator: None, action_factory: || Action::AddUnderSelection(NewGraphic::SpriteRenderer) },
        CommandEntry { label: "Duplicate Selection",       keywords: &["duplicate"],                accelerator: Some("⌘D"),   action_factory: || Action::DuplicateSelection },
        CommandEntry { label: "Delete Selection",          keywords: &["delete", "remove"],         accelerator: Some("⌫"),   action_factory: || Action::DeleteSelection },

        CommandEntry { label: "Fit View",                  keywords: &["fit", "zoom", "view"],      accelerator: None,         action_factory: || Action::Fit },
        CommandEntry { label: "Toggle Show Polygons",      keywords: &["toggle", "show", "polygon"], accelerator: None,         action_factory: || Action::ToggleShowPolygon },
        CommandEntry { label: "Toggle Show Pivot Markers", keywords: &["toggle", "show", "pivot"],  accelerator: None,         action_factory: || Action::ToggleShowPivot },
        CommandEntry { label: "Toggle Show Part Outlines", keywords: &["toggle", "show", "outline"], accelerator: None,         action_factory: || Action::ToggleShowOutlines },
        CommandEntry { label: "Toggle Show Atlas AABB",    keywords: &["toggle", "show", "aabb"],   accelerator: None,         action_factory: || Action::ToggleShowAABB },

        CommandEntry { label: "Next Tab",                  keywords: &["next", "tab"],              accelerator: Some("⌘⇧]"), action_factory: || Action::NextTab },
        CommandEntry { label: "Previous Tab",              keywords: &["previous", "prev", "tab"],  accelerator: Some("⌘⇧["), action_factory: || Action::PrevTab },
    ]
}

/// Fuzzy substring filter over command labels + keywords. Returns matches
/// sorted by relevance (label-prefix > label-contains > keyword-only).
/// An empty query returns the full list in registry order.
pub fn filter_commands<'a>(query: &str) -> Vec<&'a CommandEntry> {
    let cmds = commands();
    if query.is_empty() {
        return cmds.iter().collect();
    }
    let q = query.to_lowercase();
    let terms: Vec<&str> = q.split_whitespace().collect();
    let mut scored: Vec<(i32, &CommandEntry)> = cmds
        .iter()
        .filter_map(|c| {
            let label_lc = c.label.to_lowercase();
            // Every term must appear in label OR keywords for the command
            // to qualify — narrow first, then score.
            let qualified = terms.iter().all(|t| {
                label_lc.contains(t) || c.keywords.iter().any(|k| k.contains(t))
            });
            if !qualified {
                return None;
            }
            let mut score = 0;
            if let Some(pos) = label_lc.find(&q) {
                // Earlier-in-label > later. Prefix gets the highest bonus.
                score += 1_000 - pos as i32;
            } else if c.keywords.iter().any(|k| k.contains(&q)) {
                score += 100;
            }
            Some((score, c))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored.into_iter().map(|(_, c)| c).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_query_returns_full_list_in_registry_order() {
        let filtered = filter_commands("");
        assert_eq!(filtered.len(), commands().len());
        assert_eq!(filtered[0].label, "Open File…");
    }

    #[test]
    fn label_prefix_wins_over_keyword_only_match() {
        // "save" matches "Save", "Save All" (label-prefix) and also other
        // commands whose keywords include "save". Label matches rank first.
        let filtered = filter_commands("save");
        assert!(filtered[0].label.starts_with("Save"));
    }

    #[test]
    fn multi_term_filter_requires_all_terms() {
        let filtered = filter_commands("save all");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].label, "Save All");
    }

    #[test]
    fn keyword_synonyms_match() {
        // "group" is a keyword for the container command.
        let filtered = filter_commands("group");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].label, "New Container");
    }

    #[test]
    fn case_insensitive_matching() {
        let lower = filter_commands("undo");
        let upper = filter_commands("UNDO");
        assert_eq!(lower.len(), upper.len());
        assert_eq!(lower[0].label, upper[0].label);
    }

    #[test]
    fn no_matches_returns_empty_vec() {
        let filtered = filter_commands("absolutelynothingmatchesthis");
        assert!(filtered.is_empty());
    }
}
