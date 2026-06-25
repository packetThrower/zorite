//! The structural math editor.
//!
//! - [`model`] — the edit tree (`Row` / `Atom`) and LaTeX serialization. No gpui.
//! - [`geometry`] — caret + slot rects from RaTeX's layout (top-row caret done;
//!   nested slots next). No gpui.
//! - [`cursor`] — cursor + structural edits (insert / backspace / navigate). No gpui.
//! - [`input`] — typed-char → edit interpreter ("natural typing"). No gpui.
//! - [`view`] — the gpui view: renders the formula + caret and turns keystrokes into
//!   edits. The symbol palette is a later milestone.

pub mod cursor;
pub mod geometry;
pub mod input;
pub mod model;
pub mod view;
