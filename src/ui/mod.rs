//! Render helpers for the workspace chrome and the outliner. These are
//! free functions (not methods) taking the `AppView` to read and a
//! `Context<AppView>` for event listeners — the same idiom etch341 uses.

pub mod block_row;
pub mod links;
pub mod page_view;
pub mod sidebar;
