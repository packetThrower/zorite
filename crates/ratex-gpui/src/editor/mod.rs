//! The structural math editor.
//!
//! - [`model`] — the edit tree (`Row` / `Atom`) and LaTeX serialization. No gpui.
//! - `geometry` (M1, next) — exact caret + slot rects from RaTeX's layout. No gpui.
//! - `view` (M3) — the gpui `Element`, input handling, and the symbol palette.

pub mod model;
