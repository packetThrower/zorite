//! `ratex-gpui` — a structural, MathQuill-style math editor for GPUI, built on the
//! [RaTeX](https://github.com/erweixin/RaTeX) typesetting engine.
//!
//! RaTeX is the engine (parse → layout → render); this crate is the **editor**.
//!
//! - [`editor`] — the structural editor. Its logic is GUI-free ([`editor::model`],
//!   later `editor::geometry`); the gpui glue (`editor::view` + palette) is layered on
//!   top so the editor could move to another GUI with a thin adapter swap.
//! - `render` (later milestone) — display a formula as a gpui element, the `ratex-gtk4`
//!   analog.
//!
//! See `spikes/ratex-probe/DESIGN.md` for the full design and milestones.

pub mod editor;
