//! The structural math model — a MathQuill-style edit tree. GUI-free.
//!
//! A [`Row`] is a horizontal list of [`Atom`]s; structural atoms (fractions, scripts,
//! roots, delimiters) hold child `Row`s, which are the editable **slots**. The model
//! serializes to LaTeX, which RaTeX then typesets — so editing manipulates the 2-D
//! structure, never raw LaTeX text. An empty slot serializes to `\square` (a visible
//! placeholder box).

/// A horizontal list of atoms — the editable unit ("slot"). Empty = a placeholder box.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Row {
    pub atoms: Vec<Atom>,
}

/// One element of a [`Row`]: a leaf symbol, or a structure with child rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Atom {
    /// A single symbol, stored as its LaTeX: `"x"`, `"4"`, `"+"`, `"\\alpha"`, `"\\int"`.
    Sym(String),
    /// `\frac{num}{den}` — a bar with a box above and below.
    Frac { num: Row, den: Row },
    /// A base carrying an optional superscript and/or subscript (`x^2`, `\int_0^1`).
    Script {
        base: Box<Atom>,
        sup: Option<Row>,
        sub: Option<Row>,
    },
    /// `\sqrt{radicand}` or `\sqrt[index]{radicand}`.
    Sqrt { radicand: Row, index: Option<Row> },
    /// Auto-growing delimiters: `\left<open> body \right<close>`.
    Delim {
        open: String,
        body: Row,
        close: String,
    },
}

impl Row {
    pub fn new() -> Self {
        Self { atoms: Vec::new() }
    }

    pub fn is_empty(&self) -> bool {
        self.atoms.is_empty()
    }

    /// Build a row from a string of single-character symbols (test/convenience helper).
    pub fn syms(s: &str) -> Self {
        Self {
            atoms: s.chars().map(|c| Atom::Sym(c.to_string())).collect(),
        }
    }

    /// Serialize to LaTeX. An empty row emits `\square` so RaTeX renders a visible,
    /// layout-occupying placeholder. Atoms are space-joined so a command symbol
    /// (`\alpha`) can't fuse with the next token — LaTeX math collapses the spaces.
    pub fn to_latex(&self) -> String {
        if self.atoms.is_empty() {
            return r"\square".to_string();
        }
        self.atoms
            .iter()
            .map(Atom::to_latex)
            .collect::<Vec<_>>()
            .join(" ")
    }
}

impl Atom {
    pub fn to_latex(&self) -> String {
        match self {
            Atom::Sym(s) => s.clone(),
            Atom::Frac { num, den } => {
                format!(r"\frac{{{}}}{{{}}}", num.to_latex(), den.to_latex())
            }
            Atom::Script { base, sup, sub } => {
                // Brace a *structural* base so the script attaches to the whole thing;
                // leave a bare symbol unbraced so operators (\int, \sum) keep their
                // limit placement (`{\int}_0^1` would demote \int to an ordinary atom).
                let mut out = if matches!(base.as_ref(), Atom::Sym(_)) {
                    base.to_latex()
                } else {
                    format!("{{{}}}", base.to_latex())
                };
                if let Some(sub) = sub {
                    out.push_str(&format!("_{{{}}}", sub.to_latex()));
                }
                if let Some(sup) = sup {
                    out.push_str(&format!("^{{{}}}", sup.to_latex()));
                }
                out
            }
            Atom::Sqrt { radicand, index } => match index {
                Some(idx) => format!(r"\sqrt[{}]{{{}}}", idx.to_latex(), radicand.to_latex()),
                None => format!(r"\sqrt{{{}}}", radicand.to_latex()),
            },
            Atom::Delim { open, body, close } => {
                format!(r"\left{} {} \right{}", open, body.to_latex(), close)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_row_is_placeholder() {
        assert_eq!(Row::new().to_latex(), r"\square");
    }

    #[test]
    fn symbols_space_join() {
        assert_eq!(Row::syms("4x").to_latex(), "4 x");
    }

    #[test]
    fn fraction() {
        let f = Atom::Frac {
            num: Row::syms("a"),
            den: Row::syms("b"),
        };
        assert_eq!(f.to_latex(), r"\frac{a}{b}");
    }

    #[test]
    fn empty_fraction_slots_render_boxes() {
        let f = Atom::Frac {
            num: Row::new(),
            den: Row::new(),
        };
        assert_eq!(f.to_latex(), r"\frac{\square}{\square}");
    }

    #[test]
    fn script_sub_then_sup() {
        let s = Atom::Script {
            base: Box::new(Atom::Sym("x".into())),
            sup: Some(Row::syms("2")),
            sub: Some(Row::syms("i")),
        };
        assert_eq!(s.to_latex(), r"x_{i}^{2}");
    }

    #[test]
    fn operator_base_unbraced_keeps_limits() {
        // ∫ with limits — base stays bare so RaTeX treats \int as an operator.
        let s = Atom::Script {
            base: Box::new(Atom::Sym(r"\int".into())),
            sup: Some(Row::syms("1")),
            sub: Some(Row::syms("0")),
        };
        assert_eq!(s.to_latex(), r"\int_{0}^{1}");
    }

    #[test]
    fn structural_base_is_braced() {
        // (a/b)^2 — the fraction base must be braced so ^2 binds to the whole fraction.
        let s = Atom::Script {
            base: Box::new(Atom::Frac {
                num: Row::syms("a"),
                den: Row::syms("b"),
            }),
            sup: Some(Row::syms("2")),
            sub: None,
        };
        assert_eq!(s.to_latex(), r"{\frac{a}{b}}^{2}");
    }

    #[test]
    fn sqrt_with_index() {
        let s = Atom::Sqrt {
            radicand: Row::syms("x"),
            index: Some(Row::syms("3")),
        };
        assert_eq!(s.to_latex(), r"\sqrt[3]{x}");
    }

    #[test]
    fn delimiters() {
        let d = Atom::Delim {
            open: "(".into(),
            body: Row::syms("x"),
            close: ")".into(),
        };
        assert_eq!(d.to_latex(), r"\left( x \right)");
    }

    /// The serializer's output must actually parse + lay out in RaTeX (the round-trip
    /// that keeps `model → LaTeX → RaTeX` honest).
    #[test]
    fn serialized_latex_parses_and_lays_out() {
        // ∫_0^1 \frac{a}{b} — operator limits + a fraction.
        let integral = Atom::Script {
            base: Box::new(Atom::Sym(r"\int".into())),
            sup: Some(Row::syms("1")),
            sub: Some(Row::syms("0")),
        };
        let integrand = Atom::Frac {
            num: Row::syms("a"),
            den: Row::syms("b"),
        };
        let row = Row {
            atoms: vec![integral, integrand],
        };
        let latex = row.to_latex();
        let nodes = ratex_parser::parse(&latex).expect("RaTeX should parse our LaTeX");
        let lbox = ratex_layout::layout(&nodes, &ratex_layout::LayoutOptions::default());
        assert!(lbox.width > 0.0, "lays out to a non-empty box");
    }
}
