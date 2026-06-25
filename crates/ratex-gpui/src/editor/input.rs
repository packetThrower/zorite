//! The typed-input interpreter — "natural typing" that maps keystrokes to structural
//! edits. GUI-free; the view feeds it characters and repaints.
//!
//! Triggers: `/` opens a fraction (cursor into the numerator), `^`/`_` open a
//! super/subscript on the preceding atom, `(` opens a delimiter pair (cursor inside),
//! `)` hops out of it, space exits the current structure, and any other character is
//! inserted as a literal symbol. Multi-character `\commands` are a later milestone.

use crate::editor::cursor::{Cursor, Slot, Step};
use crate::editor::model::{Atom, Row};

/// Apply one typed character as a structural edit at the cursor.
pub fn type_char(top: &mut Row, cursor: &mut Cursor, ch: char) {
    match ch {
        '/' => cursor.insert(
            top,
            Atom::Frac {
                num: Row::new(),
                den: Row::new(),
            },
        ),
        '^' => insert_script(
            top,
            cursor,
            Atom::SupSub {
                sup: Some(Row::new()),
                sub: None,
            },
        ),
        '_' => insert_script(
            top,
            cursor,
            Atom::SupSub {
                sup: None,
                sub: Some(Row::new()),
            },
        ),
        '(' => cursor.insert(
            top,
            Atom::Delim {
                open: "(".into(),
                body: Row::new(),
                close: ")".into(),
            },
        ),
        ')' => close_delim(cursor),
        ' ' => exit_structure(cursor),
        _ => cursor.insert(top, Atom::Sym(ch.to_string())),
    }
}

/// A script needs a base — the atom just before the cursor. With none (a row/slot
/// start), the keystroke is dropped rather than producing baseless `^{}`.
fn insert_script(top: &mut Row, cursor: &mut Cursor, script: Atom) {
    if cursor.index == 0 {
        return;
    }
    cursor.insert(top, script);
}

/// `)` hops out of the innermost delimiter body, to just after the delimiter.
fn close_delim(cursor: &mut Cursor) {
    if let Some(&Step {
        slot: Slot::Body,
        atom,
    }) = cursor.path.last()
    {
        cursor.path.pop();
        cursor.index = atom + 1;
    }
}

/// Space exits the current structure (MathQuill convention): hop up one level, to just
/// after the structure. A no-op at the top level.
fn exit_structure(cursor: &mut Cursor) {
    if let Some(step) = cursor.path.pop() {
        cursor.index = step.atom + 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(s: &str) -> Row {
        let mut top = Row::new();
        let mut cur = Cursor::start();
        for ch in s.chars() {
            type_char(&mut top, &mut cur, ch);
        }
        top
    }

    #[test]
    fn plain_symbols() {
        assert_eq!(typed("4x").to_latex(), "4 x");
    }

    #[test]
    fn slash_makes_a_fraction_and_enters_numerator() {
        // "1/2": '1' -> sym; '/' -> fraction, cursor into the numerator; '2' -> numerator.
        let mut top = Row::new();
        let mut cur = Cursor::start();
        for ch in "1/2".chars() {
            type_char(&mut top, &mut cur, ch);
        }
        assert_eq!(top.to_latex(), r"1 \frac{2}{\square}");
    }

    #[test]
    fn caret_makes_a_superscript() {
        assert_eq!(typed("x^2").to_latex(), "x ^{2}");
    }

    #[test]
    fn underscore_makes_a_subscript() {
        assert_eq!(typed("a_i").to_latex(), "a _{i}");
    }

    #[test]
    fn script_with_no_base_is_dropped() {
        // '^' at the row start has no base, so it's ignored; '2' lands as a symbol.
        assert_eq!(typed("^2").to_latex(), "2");
    }

    #[test]
    fn parens_wrap_then_close_exits() {
        let mut top = Row::new();
        let mut cur = Cursor::start();
        for ch in "(x)".chars() {
            type_char(&mut top, &mut cur, ch);
        }
        assert_eq!(top.to_latex(), r"\left( x \right)");
        assert_eq!(cur.path, vec![]); // exited the body
        assert_eq!(cur.index, 1); // sitting just after the delimiter
    }

    #[test]
    fn space_exits_a_script() {
        let mut top = Row::new();
        let mut cur = Cursor::start();
        for ch in "x^2".chars() {
            type_char(&mut top, &mut cur, ch);
        }
        type_char(&mut top, &mut cur, ' '); // exit the superscript
        assert_eq!(cur.path, vec![]);
        type_char(&mut top, &mut cur, '3'); // continues after the script
        assert_eq!(top.to_latex(), "x ^{2} 3");
    }
}
