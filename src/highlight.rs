//! Host-side syntax highlighting for fenced code blocks, feeding both views
//! through their engine-free callbacks (`MarkdownView::on_highlight`,
//! `EditorState::set_code_highlighter`) — the same host-supplies-renderer
//! pattern as math and mermaid. gpui-component's tree-sitter highlighter does
//! the work; the grammars compiled in are the feature list on the
//! `gpui-component` dependency in Cargo.toml. Unknown languages fall back to
//! plain `text` (no tokens), so an unhighlightable block just stays the flat
//! code color.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use gpui::HighlightStyle;
use gpui_component::highlighter::{HighlightTheme, SyntaxHighlighter};

type Styles = Rc<Vec<(Range<usize>, HighlightStyle)>>;

/// Cache of highlighted blocks, keyed by `(language, code hash)`. Styles bake
/// the active theme's colors in, so a theme switch clears it (`set_theme`).
#[derive(Default)]
pub struct HighlightStore {
    theme: Option<Arc<HighlightTheme>>,
    cache: HashMap<(String, u64), Styles>,
}

impl HighlightStore {
    /// Adopt the active theme's highlight styles; drops the cache when they
    /// actually changed (called from `apply_theme`).
    pub fn set_theme(&mut self, theme: Arc<HighlightTheme>) {
        if self.theme.as_ref().is_some_and(|t| Arc::ptr_eq(t, &theme)) {
            return;
        }
        self.theme = Some(theme);
        self.cache.clear();
    }

    /// Token styles for `code` under `lang` (sorted, non-overlapping byte
    /// ranges). Empty until a theme is set and for languages with no grammar.
    pub fn highlight(&mut self, lang: &str, code: &str) -> Styles {
        let Some(theme) = self.theme.clone() else {
            return Rc::new(Vec::new());
        };
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        code.hash(&mut hasher);
        let key = (lang.to_string(), hasher.finish());
        if let Some(hit) = self.cache.get(&key) {
            return hit.clone();
        }
        // A fresh parse per changed block: blocks are note-sized, tree-sitter
        // is fast, and incremental reuse wouldn't survive block switching.
        let mut hl = SyntaxHighlighter::new(lang);
        let rope = ropey::Rope::from_str(code);
        hl.update(None, &rope, Some(std::time::Duration::from_millis(80)));
        let styles = Rc::new(hl.styles(&(0..code.len()), &theme));
        self.cache.insert(key, styles.clone());
        styles
    }
}
