//! The structural math editor.
//!
//! - [`model`] — the edit tree (`Row` / `Atom`) and LaTeX serialization. No gpui.
//! - [`geometry`] — caret + slot rects from RaTeX's layout (top-row caret done;
//!   nested slots next). No gpui.
//! - [`cursor`] — cursor + structural edits (insert / backspace / navigate). No gpui.
//! - [`input`] — typed-char → edit interpreter ("natural typing"). No gpui.
//! - `view` (M3) — the gpui `Element`, input handling, and the symbol palette.

pub mod cursor;
pub mod geometry;
pub mod input;
pub mod model;
