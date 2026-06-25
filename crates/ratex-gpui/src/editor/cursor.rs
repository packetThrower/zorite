//! Cursor + structural edits over the model. GUI-free.
//!
//! The cursor is a path of [`Step`]s descending from the top row into nested slots,
//! plus an `index` between atoms in the target row. Editing inserts/removes atoms and
//! navigates left/right — descending into structures (fraction / root / delimiter /
//! script slots), walking a structure's slots in order, and ascending out at
//! boundaries, MathQuill-style.
//!
//! A super/subscript ([`Atom::SupSub`]) attaches to the preceding row atom — its base —
//! so the base stays an editable symbol; navigating a script visits its present
//! sub/sup slots.

use crate::editor::model::{Atom, Row};

/// Which child row of a structural atom a [`Step`] descends into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    Num,
    Den,
    Radicand,
    Index,
    Body,
    Sup,
    Sub,
}

/// One descent: into `slot` of the structural atom at `atom` in the parent row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Step {
    pub atom: usize,
    pub slot: Slot,
}

/// A position in the model: a descent path + an index between atoms in the target row.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Cursor {
    pub path: Vec<Step>,
    pub index: usize,
}

/// A structural atom's child rows, in left-to-right navigation order. Leaves have none.
fn nav_slots(atom: &Atom) -> Vec<Slot> {
    match atom {
        Atom::Frac { .. } => vec![Slot::Num, Slot::Den],
        Atom::Sqrt { index, .. } => {
            if index.is_some() {
                vec![Slot::Index, Slot::Radicand]
            } else {
                vec![Slot::Radicand]
            }
        }
        Atom::Delim { .. } => vec![Slot::Body],
        Atom::SupSub { sup, sub } => {
            // Present scripts only; subscript first (matching the serialized order).
            let mut s = Vec::new();
            if sub.is_some() {
                s.push(Slot::Sub);
            }
            if sup.is_some() {
                s.push(Slot::Sup);
            }
            s
        }
        Atom::Sym(_) => vec![],
    }
}

fn slot_row(atom: &Atom, slot: Slot) -> &Row {
    match (atom, slot) {
        (Atom::Frac { num, .. }, Slot::Num) => num,
        (Atom::Frac { den, .. }, Slot::Den) => den,
        (Atom::Sqrt { radicand, .. }, Slot::Radicand) => radicand,
        (Atom::Sqrt { index: Some(i), .. }, Slot::Index) => i,
        (Atom::Delim { body, .. }, Slot::Body) => body,
        (Atom::SupSub { sub: Some(r), .. }, Slot::Sub) => r,
        (Atom::SupSub { sup: Some(r), .. }, Slot::Sup) => r,
        _ => unreachable!("cursor invariant: slot {slot:?} does not exist on this atom"),
    }
}

fn slot_row_mut(atom: &mut Atom, slot: Slot) -> &mut Row {
    match (atom, slot) {
        (Atom::Frac { num, .. }, Slot::Num) => num,
        (Atom::Frac { den, .. }, Slot::Den) => den,
        (Atom::Sqrt { radicand, .. }, Slot::Radicand) => radicand,
        (Atom::Sqrt { index: Some(i), .. }, Slot::Index) => i,
        (Atom::Delim { body, .. }, Slot::Body) => body,
        (Atom::SupSub { sub: Some(r), .. }, Slot::Sub) => r,
        (Atom::SupSub { sup: Some(r), .. }, Slot::Sup) => r,
        _ => unreachable!("cursor invariant: slot {slot:?} does not exist on this atom"),
    }
}

/// Resolve a path to the target row (immutable).
fn resolve<'a>(row: &'a Row, path: &[Step]) -> &'a Row {
    match path.split_first() {
        None => row,
        Some((step, rest)) => resolve(slot_row(&row.atoms[step.atom], step.slot), rest),
    }
}

/// Resolve a path to the target row (mutable).
fn resolve_mut<'a>(row: &'a mut Row, path: &[Step]) -> &'a mut Row {
    match path.split_first() {
        None => row,
        Some((step, rest)) => resolve_mut(slot_row_mut(&mut row.atoms[step.atom], step.slot), rest),
    }
}

impl Cursor {
    /// Cursor at the start of the top-level row.
    pub fn start() -> Self {
        Self::default()
    }

    /// The row the cursor is currently in.
    pub fn row<'a>(&self, top: &'a Row) -> &'a Row {
        resolve(top, &self.path)
    }

    /// Insert `atom` at the cursor. A structure descends into its first slot (type `/`
    /// → cursor in the numerator); a leaf steps past.
    pub fn insert(&mut self, top: &mut Row, atom: Atom) {
        let descend = nav_slots(&atom).first().copied();
        resolve_mut(top, &self.path).atoms.insert(self.index, atom);
        match descend {
            Some(slot) => {
                self.path.push(Step {
                    atom: self.index,
                    slot,
                });
                self.index = 0;
            }
            None => self.index += 1,
        }
    }

    /// Delete the atom before the cursor. At a slot start, ascend out to before the
    /// structure (deleting an empty structure outright is a later refinement).
    pub fn backspace(&mut self, top: &mut Row) {
        if self.index > 0 {
            resolve_mut(top, &self.path).atoms.remove(self.index - 1);
            self.index -= 1;
        } else if let Some(step) = self.path.pop() {
            self.index = step.atom;
        }
    }

    /// Move right: past a leaf, into a structure's first slot, on to the next slot, or
    /// out after the structure.
    pub fn move_right(&mut self, top: &Row) {
        let len = self.row(top).atoms.len();
        if self.index < len {
            match nav_slots(&self.row(top).atoms[self.index]).first().copied() {
                Some(slot) => {
                    self.path.push(Step {
                        atom: self.index,
                        slot,
                    });
                    self.index = 0;
                }
                None => self.index += 1,
            }
        } else if let Some(step) = self.path.last().copied() {
            let slots =
                nav_slots(&resolve(top, &self.path[..self.path.len() - 1]).atoms[step.atom]);
            let pos = slots.iter().position(|s| *s == step.slot).unwrap();
            if pos + 1 < slots.len() {
                self.path.last_mut().unwrap().slot = slots[pos + 1];
                self.index = 0;
            } else {
                self.path.pop();
                self.index = step.atom + 1;
            }
        }
    }

    /// Move left: mirror of [`Cursor::move_right`].
    pub fn move_left(&mut self, top: &Row) {
        if self.index > 0 {
            match nav_slots(&self.row(top).atoms[self.index - 1])
                .last()
                .copied()
            {
                Some(slot) => {
                    self.path.push(Step {
                        atom: self.index - 1,
                        slot,
                    });
                    self.index = self.row(top).atoms.len();
                }
                None => self.index -= 1,
            }
        } else if let Some(step) = self.path.last().copied() {
            let slots =
                nav_slots(&resolve(top, &self.path[..self.path.len() - 1]).atoms[step.atom]);
            let pos = slots.iter().position(|s| *s == step.slot).unwrap();
            if pos > 0 {
                self.path.last_mut().unwrap().slot = slots[pos - 1];
                self.index = self.row(top).atoms.len();
            } else {
                self.path.pop();
                self.index = step.atom;
            }
        }
    }

    /// Move up to the vertically-stacked sibling slot (denominator → numerator,
    /// subscript → superscript). No-op outside such a slot.
    pub fn move_up(&mut self, top: &Row) {
        self.move_vert(top, true);
    }

    /// Move down to the vertically-stacked sibling slot (numerator → denominator,
    /// superscript → subscript). No-op outside such a slot.
    pub fn move_down(&mut self, top: &Row) {
        self.move_vert(top, false);
    }

    fn move_vert(&mut self, top: &Row, up: bool) {
        let Some(step) = self.path.last().copied() else {
            return;
        };
        let target = match (step.slot, up) {
            (Slot::Den, true) => Slot::Num,
            (Slot::Num, false) => Slot::Den,
            (Slot::Sub, true) => Slot::Sup,
            (Slot::Sup, false) => Slot::Sub,
            _ => return,
        };
        let parent = resolve(top, &self.path[..self.path.len() - 1]);
        if !nav_slots(&parent.atoms[step.atom]).contains(&target) {
            return; // the sibling slot isn't present (e.g. a sup-only script)
        }
        self.path.last_mut().unwrap().slot = target;
        let len = self.row(top).atoms.len();
        self.index = self.index.min(len);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::model::{Atom, Row};

    fn sym(c: &str) -> Atom {
        Atom::Sym(c.into())
    }
    fn empty_frac() -> Atom {
        Atom::Frac {
            num: Row::new(),
            den: Row::new(),
        }
    }

    #[test]
    fn flat_insert_and_backspace() {
        let mut top = Row::new();
        let mut cur = Cursor::start();
        cur.insert(&mut top, sym("a"));
        cur.insert(&mut top, sym("b"));
        assert_eq!(top.to_latex(), "a b");
        assert_eq!(cur.index, 2);
        cur.backspace(&mut top);
        assert_eq!(top.to_latex(), "a");
        assert_eq!(cur.index, 1);
    }

    #[test]
    fn vertical_nav_between_fraction_slots() {
        let mut top = Row::new();
        let mut cur = Cursor::start();
        cur.insert(&mut top, empty_frac()); // into the numerator
        cur.insert(&mut top, sym("a"));
        cur.move_down(&top); // -> denominator
        assert_eq!(
            cur.path,
            vec![Step {
                atom: 0,
                slot: Slot::Den
            }]
        );
        cur.insert(&mut top, sym("b"));
        assert_eq!(top.to_latex(), r"\frac{a}{b}");
        cur.move_up(&top); // -> numerator
        assert_eq!(
            cur.path,
            vec![Step {
                atom: 0,
                slot: Slot::Num
            }]
        );
    }

    #[test]
    fn flat_left_right() {
        let mut top = Row::new();
        let mut cur = Cursor::start();
        for c in ["a", "b", "c"] {
            cur.insert(&mut top, sym(c));
        }
        assert_eq!(cur.index, 3);
        cur.move_left(&top);
        cur.move_left(&top);
        cur.move_left(&top);
        assert_eq!(cur.index, 0);
        cur.move_left(&top); // at start — no-op
        assert_eq!(cur.index, 0);
        cur.move_right(&top);
        assert_eq!(cur.index, 1);
    }

    #[test]
    fn insert_fraction_descends_into_numerator() {
        let mut top = Row::new();
        let mut cur = Cursor::start();
        cur.insert(&mut top, empty_frac());
        assert_eq!(
            cur.path,
            vec![Step {
                atom: 0,
                slot: Slot::Num
            }]
        );
        assert_eq!(cur.index, 0);
        cur.insert(&mut top, sym("a"));
        assert_eq!(top.to_latex(), r"\frac{a}{\square}");
    }

    #[test]
    fn right_walks_num_den_then_out() {
        let mut top = Row::new();
        let mut cur = Cursor::start();
        cur.insert(&mut top, empty_frac()); // in numerator
        cur.insert(&mut top, sym("a")); // num = a
        cur.move_right(&top); // num end -> denominator
        assert_eq!(
            cur.path,
            vec![Step {
                atom: 0,
                slot: Slot::Den
            }]
        );
        assert_eq!(cur.index, 0);
        cur.insert(&mut top, sym("b")); // den = b
        assert_eq!(top.to_latex(), r"\frac{a}{b}");
        cur.move_right(&top); // den end -> out, after the fraction
        assert_eq!(cur.path, vec![]);
        assert_eq!(cur.index, 1);
    }

    #[test]
    fn backspace_at_slot_start_ascends() {
        let mut top = Row::new();
        let mut cur = Cursor::start();
        cur.insert(&mut top, empty_frac()); // empty numerator, index 0
        cur.backspace(&mut top); // ascend out
        assert_eq!(cur.path, vec![]);
        assert_eq!(cur.index, 0);
    }

    #[test]
    fn right_enters_existing_fraction() {
        let mut top = Row::new();
        let mut cur = Cursor::start();
        cur.insert(&mut top, empty_frac()); // descends into numerator
        cur.backspace(&mut top); // ascend to top, before the (still-present) fraction
        assert_eq!(cur.index, 0);
        cur.move_right(&top); // enter the fraction's numerator
        assert_eq!(
            cur.path,
            vec![Step {
                atom: 0,
                slot: Slot::Num
            }]
        );
        assert_eq!(cur.index, 0);
    }

    #[test]
    fn script_descend_and_traverse() {
        let mut top = Row::new();
        let mut cur = Cursor::start();
        cur.insert(&mut top, sym("x")); // the base
        cur.insert(
            &mut top,
            Atom::SupSub {
                sup: Some(Row::new()),
                sub: Some(Row::new()),
            },
        );
        // insert descends into the first nav slot — the subscript
        assert_eq!(
            cur.path,
            vec![Step {
                atom: 1,
                slot: Slot::Sub
            }]
        );
        cur.insert(&mut top, sym("0")); // sub = 0
        cur.move_right(&top); // sub end -> superscript
        assert_eq!(
            cur.path,
            vec![Step {
                atom: 1,
                slot: Slot::Sup
            }]
        );
        cur.insert(&mut top, sym("2")); // sup = 2
        assert_eq!(top.to_latex(), "x _{0}^{2}");
        cur.move_right(&top); // sup end -> out after the script
        assert_eq!(cur.path, vec![]);
        assert_eq!(cur.index, 2);
    }
}
