//! Small-state persistence — the settings-backed loaders/savers for recent
//! pages, favorites, sidebar collapse state, and the open-tabs sidecar —
//! split from `app.rs`.

use super::*;

/// How many recently-viewed pages the sidebar's page tree is capped to.
const RECENT_PAGES_LIMIT: usize = 10;

impl AppView {
    /// Load the persisted recent-pages list, falling back to the most-recently
    /// edited pages so the sidebar isn't empty before anything's been viewed.
    pub(super) fn load_recent_pages(&self) -> Vec<i64> {
        let stored: Vec<i64> = self
            .db
            .get_setting("recent_pages")
            .map(|s| s.split(',').filter_map(|x| x.trim().parse().ok()).collect())
            .unwrap_or_default();
        if stored.is_empty() {
            self.db
                .recent_page_ids(RECENT_PAGES_LIMIT)
                .unwrap_or_default()
        } else {
            stored
        }
    }

    /// Mark a named page as most-recently-viewed (front of the list, capped)
    /// and persist it. The sidebar page tree is filtered to this list.
    pub(super) fn record_recent(&mut self, page_id: i64) {
        self.recent_pages.retain(|&id| id != page_id);
        self.recent_pages.insert(0, page_id);
        self.recent_pages.truncate(RECENT_PAGES_LIMIT);
        let csv = self
            .recent_pages
            .iter()
            .map(i64::to_string)
            .collect::<Vec<_>>()
            .join(",");
        if let Err(e) = self.db.set_setting("recent_pages", &csv) {
            log::error!("save recent pages: {e}");
        }
    }

    /// Load the persisted favorites (a comma-separated id list; empty if none).
    pub(super) fn load_favorites(&self) -> Vec<i64> {
        self.db
            .get_setting("favorites")
            .map(|s| s.split(',').filter_map(|x| x.trim().parse().ok()).collect())
            .unwrap_or_default()
    }

    /// Whether `id` is pinned to the sidebar's Favorites group.
    pub fn is_favorite(&self, id: i64) -> bool {
        self.favorites.contains(&id)
    }

    /// Pin / unpin a page (sidebar right-click → Favorite) and persist. The
    /// sidebar reads `favorites` at render, so a notify is all that's needed.
    pub(super) fn toggle_favorite(&mut self, id: i64, cx: &mut Context<Self>) {
        match self.favorites.iter().position(|&x| x == id) {
            Some(pos) => {
                self.favorites.remove(pos);
            }
            None => self.favorites.push(id),
        }
        self.persist_favorites();
        cx.notify();
    }

    /// Persist the favorites as a comma-separated id list (mirrors recent pages).
    pub(super) fn persist_favorites(&self) {
        let csv = self
            .favorites
            .iter()
            .map(i64::to_string)
            .collect::<Vec<_>>()
            .join(",");
        if let Err(e) = self.db.set_setting("favorites", &csv) {
            log::error!("save favorites: {e}");
        }
    }

    /// Load the persisted collapsed sidebar nodes (newline-separated paths —
    /// titles are single-line, so a newline can't appear inside one).
    pub(super) fn load_collapsed(&self) -> HashSet<String> {
        self.db
            .get_setting("collapsed_nodes")
            .map(|s| {
                s.split('\n')
                    .filter(|x| !x.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Whether the sidebar tree node at `path` is collapsed (descendants hidden).
    pub fn is_collapsed(&self, path: &str) -> bool {
        self.collapsed_nodes.contains(path)
    }

    /// Collapse / expand a sidebar namespace node (its disclosure chevron) and
    /// persist. The sidebar reads `collapsed_nodes` at render, so just notify.
    pub fn toggle_collapsed(&mut self, path: &str, cx: &mut Context<Self>) {
        if !self.collapsed_nodes.remove(path) {
            self.collapsed_nodes.insert(path.to_string());
        }
        let data = self
            .collapsed_nodes
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        if let Err(e) = self.db.set_setting("collapsed_nodes", &data) {
            log::error!("save collapsed nodes: {e}");
        }
        cx.notify();
    }

    /// Load the persisted collapsed sidebar sections (newline-separated keys).
    pub(super) fn load_collapsed_sections(&self) -> HashSet<String> {
        self.db
            .get_setting("collapsed_sections")
            .map(|s| {
                s.split('\n')
                    .filter(|x| !x.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Whether the sidebar section `key` is collapsed (its rows hidden).
    pub fn is_section_collapsed(&self, key: &str) -> bool {
        self.collapsed_sections.contains(key)
    }

    /// Collapse / expand a sidebar section (its header chevron) and persist.
    pub fn toggle_section(&mut self, key: &str, cx: &mut Context<Self>) {
        if !self.collapsed_sections.remove(key) {
            self.collapsed_sections.insert(key.to_string());
        }
        let data = self
            .collapsed_sections
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        if let Err(e) = self.db.set_setting("collapsed_sections", &data) {
            log::error!("save collapsed sections: {e}");
        }
        cx.notify();
    }

    /// Serialize the tab set to the open-tabs sidecar (Settings → General →
    /// Remember window → Open tabs). Called from `render`, so every way a tab
    /// can open/close/reorder funnels through one save point; writes only on
    /// change, and only from the main window.
    pub(super) fn persist_open_tabs(&mut self) {
        if !self.is_main_window || !crate::paths::open_tabs_enabled() {
            return;
        }
        let mut out = format!("active {}\n", self.active);
        for tab in &self.tabs {
            match &tab.kind {
                TabKind::Journal | TabKind::Game => {} // pinned / never restored
                TabKind::Page(id) => out.push_str(&format!("page {id}\n")),
                TabKind::Pdf(path) => out.push_str(&format!("pdf {}\n", self.pdf_ref(path))),
                TabKind::Whiteboard(id) => out.push_str(&format!("whiteboard {id}\n")),
                TabKind::AllPages => out.push_str("allpages\n"),
                TabKind::Graph => out.push_str("graph\n"),
                TabKind::Properties => out.push_str("properties\n"),
            }
        }
        if out != self.last_tabs_saved {
            crate::paths::save_open_tabs(&out);
            self.last_tabs_saved = out;
        }
    }

    /// Re-save the tab set even if unchanged — the Settings switch just turned
    /// the feature on, so the (empty) sidecar needs the current state.
    pub(crate) fn force_persist_open_tabs(&mut self) {
        self.last_tabs_saved.clear();
        self.persist_open_tabs();
    }
}
