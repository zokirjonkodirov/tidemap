//! A small LaTeX-to-Unicode converter for `$math$` and `$$math$$`.
//!
//! There is no typesetting engine here: Greek letters, operators, arrows, fractions, roots,
//! digit super- and subscripts become plain Unicode text, which is enough to read most formulas
//! in READMEs and notes. Anything unknown is kept as typed.

fn command(name: &str) -> Option<&'static str> {
    Some(match name {
        "alpha" => "α",
        "beta" => "β",
        "gamma" => "γ",
        "delta" => "δ",
        "epsilon" | "varepsilon" => "ε",
        "zeta" => "ζ",
        "eta" => "η",
        "theta" | "vartheta" => "θ",
        "iota" => "ι",
        "kappa" => "κ",
        "lambda" => "λ",
        "mu" => "μ",
        "nu" => "ν",
        "xi" => "ξ",
        "pi" | "varpi" => "π",
        "rho" | "varrho" => "ρ",
        "sigma" | "varsigma" => "σ",
        "tau" => "τ",
        "upsilon" => "υ",
        "phi" | "varphi" => "φ",
        "chi" => "χ",
        "psi" => "ψ",
        "omega" => "ω",
        "Gamma" => "Γ",
        "Delta" => "Δ",
        "Theta" => "Θ",
        "Lambda" => "Λ",
        "Xi" => "Ξ",
        "Pi" => "Π",
        "Sigma" => "Σ",
        "Upsilon" => "Υ",
        "Phi" => "Φ",
        "Psi" => "Ψ",
        "Omega" => "Ω",
        "times" => "×",
        "cdot" => "·",
        "div" => "÷",
        "pm" => "±",
        "mp" => "∓",
        "leq" | "le" => "≤",
        "geq" | "ge" => "≥",
        "neq" | "ne" => "≠",
        "approx" => "≈",
        "equiv" => "≡",
        "sim" => "∼",
        "simeq" | "cong" => "≅",
        "propto" => "∝",
        "ll" => "≪",
        "gg" => "≫",
        "to" | "rightarrow" => "→",
        "leftarrow" | "gets" => "←",
        "Rightarrow" | "implies" => "⇒",
        "Leftarrow" => "⇐",
        "leftrightarrow" => "↔",
        "Leftrightarrow" | "iff" => "⇔",
        "mapsto" => "↦",
        "uparrow" => "↑",
        "downarrow" => "↓",
        "infty" => "∞",
        "partial" => "∂",
        "nabla" => "∇",
        "sum" => "∑",
        "prod" => "∏",
        "int" => "∫",
        "iint" => "∬",
        "oint" => "∮",
        "forall" => "∀",
        "exists" => "∃",
        "nexists" => "∄",
        "in" => "∈",
        "notin" => "∉",
        "ni" => "∋",
        "subset" => "⊂",
        "subseteq" => "⊆",
        "supset" => "⊃",
        "supseteq" => "⊇",
        "cup" => "∪",
        "cap" => "∩",
        "setminus" => "∖",
        "emptyset" | "varnothing" => "∅",
        "land" | "wedge" => "∧",
        "lor" | "vee" => "∨",
        "neg" | "lnot" => "¬",
        "ldots" | "dots" | "cdots" | "dotsc" => "…",
        "vdots" => "⋮",
        "ddots" => "⋱",
        "degree" => "°",
        "angle" => "∠",
        "perp" => "⊥",
        "parallel" => "∥",
        "hbar" => "ℏ",
        "ell" => "ℓ",
        "Re" => "ℜ",
        "Im" => "ℑ",
        "aleph" => "ℵ",
        "prime" => "′",
        "circ" => "∘",
        "bullet" => "•",
        "star" => "⋆",
        "oplus" => "⊕",
        "otimes" => "⊗",
        "langle" => "⟨",
        "rangle" => "⟩",
        "lfloor" => "⌊",
        "rfloor" => "⌋",
        "lceil" => "⌈",
        "rceil" => "⌉",
        "therefore" => "∴",
        "because" => "∵",
        "mid" => "∣",
        "quad" | "qquad" => "  ",
        "," | ";" | ":" | " " => " ",
        "!" => "",
        "{" => "{",
        "}" => "}",
        "%" => "%",
        "&" => "&",
        "$" => "$",
        "#" => "#",
        "_" => "_",
        "sin" | "cos" | "tan" | "cot" | "sec" | "csc" | "log" | "ln" | "exp" | "lim" | "max" | "min" | "det" | "sup" | "inf"
        | "arcsin" | "arccos" | "arctan" | "sinh" | "cosh" | "tanh" | "gcd" | "deg" | "dim" | "ker" | "arg" | "mod" => return None,
        _ => return None,
    })
}

const FUNCTIONS: &[&str] = &[
    "sin", "cos", "tan", "cot", "sec", "csc", "log", "ln", "exp", "lim", "max", "min", "det", "sup", "inf", "arcsin", "arccos", "arctan",
    "sinh", "cosh", "tanh", "gcd", "deg", "dim", "ker", "arg", "mod",
];

fn superscript(ch: char) -> Option<char> {
    Some(match ch {
        '0' => '⁰',
        '1' => '¹',
        '2' => '²',
        '3' => '³',
        '4' => '⁴',
        '5' => '⁵',
        '6' => '⁶',
        '7' => '⁷',
        '8' => '⁸',
        '9' => '⁹',
        '+' => '⁺',
        '-' | '−' => '⁻',
        '=' => '⁼',
        '(' => '⁽',
        ')' => '⁾',
        'n' => 'ⁿ',
        'i' => 'ⁱ',
        _ => return None,
    })
}

fn subscript(ch: char) -> Option<char> {
    Some(match ch {
        '0' => '₀',
        '1' => '₁',
        '2' => '₂',
        '3' => '₃',
        '4' => '₄',
        '5' => '₅',
        '6' => '₆',
        '7' => '₇',
        '8' => '₈',
        '9' => '₉',
        '+' => '₊',
        '-' | '−' => '₋',
        '=' => '₌',
        '(' => '₍',
        ')' => '₎',
        'a' => 'ₐ',
        'e' => 'ₑ',
        'h' => 'ₕ',
        'i' => 'ᵢ',
        'j' => 'ⱼ',
        'k' => 'ₖ',
        'l' => 'ₗ',
        'm' => 'ₘ',
        'n' => 'ₙ',
        'o' => 'ₒ',
        'p' => 'ₚ',
        'r' => 'ᵣ',
        's' => 'ₛ',
        't' => 'ₜ',
        'u' => 'ᵤ',
        'v' => 'ᵥ',
        'x' => 'ₓ',
        _ => return None,
    })
}

struct Parser<'a> {
    chars: Vec<char>,
    at: usize,
    _text: &'a str,
}

impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    /// One argument: a `{group}` or a single token.
    fn argument(&mut self) -> String {
        while self.peek() == Some(' ') {
            self.at += 1;
        }
        match self.peek() {
            Some('{') => {
                self.at += 1;
                let inner = self.until_close();
                inner
            }
            Some('\\') => self.command_token(),
            Some(ch) => {
                self.at += 1;
                ch.to_string()
            }
            None => String::new(),
        }
    }

    fn until_close(&mut self) -> String {
        let mut out = String::new();
        while let Some(ch) = self.peek() {
            if ch == '}' {
                self.at += 1;
                break;
            }
            out.push_str(&self.token());
        }
        out
    }

    fn command_token(&mut self) -> String {
        self.at += 1; // the backslash
        let Some(first) = self.peek() else { return "\\".into() };
        let mut name = String::new();
        if first.is_ascii_alphabetic() {
            while let Some(ch) = self.peek() {
                if ch.is_ascii_alphabetic() {
                    name.push(ch);
                    self.at += 1;
                } else {
                    break;
                }
            }
        } else {
            name.push(first);
            self.at += 1;
        }
        match name.as_str() {
            "frac" | "dfrac" | "tfrac" => {
                let (a, b) = (self.argument(), self.argument());
                format!("{}/{}", wrap(&a), wrap(&b))
            }
            "sqrt" => {
                let root = if self.peek() == Some('[') {
                    self.at += 1;
                    let mut n = String::new();
                    while let Some(ch) = self.peek() {
                        self.at += 1;
                        if ch == ']' {
                            break;
                        }
                        n.push(ch);
                    }
                    n
                } else {
                    String::new()
                };
                let body = self.argument();
                let prefix = match root.as_str() {
                    "" | "2" => "√".to_owned(),
                    "3" => "∛".to_owned(),
                    "4" => "∜".to_owned(),
                    other => format!("{other}√"),
                };
                if body.chars().count() <= 1 { format!("{prefix}{body}") } else { format!("{prefix}({body})") }
            }
            "text" | "mathrm" | "mathbf" | "mathit" | "mathcal" | "mathbb" | "mathsf" | "mathtt" | "boldsymbol" | "operatorname" | "textbf"
            | "textit" | "mbox" | "bm" | "vec" | "hat" | "bar" | "tilde" | "overline" | "underline" => {
                let inner = self.argument();
                match name.as_str() {
                    "vec" => format!("{inner}\u{20d7}"),
                    "hat" => format!("{inner}\u{302}"),
                    "bar" | "overline" => format!("{inner}\u{304}"),
                    "tilde" => format!("{inner}\u{303}"),
                    "mathbb" => blackboard(&inner),
                    _ => inner,
                }
            }
            "left" | "right" | "big" | "Big" | "bigg" | "Bigg" | "displaystyle" | "textstyle" | "limits" | "nolimits" => {
                // `\left(` keeps its bracket; `\left.` drops it.
                if self.peek() == Some('.') && matches!(name.as_str(), "left" | "right") {
                    self.at += 1;
                }
                String::new()
            }
            "\\" => " ".into(),
            "newline" => " ".into(),
            other if FUNCTIONS.contains(&other) => other.to_owned(),
            other => match command(other) {
                Some(text) => text.to_owned(),
                None => format!("\\{other}"),
            },
        }
    }

    fn token(&mut self) -> String {
        let Some(ch) = self.peek() else { return String::new() };
        match ch {
            '\\' => self.command_token(),
            '{' => {
                self.at += 1;
                self.until_close()
            }
            '^' | '_' => {
                self.at += 1;
                let arg = self.argument();
                let map = if ch == '^' { superscript } else { subscript };
                let converted: Option<String> = arg.chars().map(map).collect();
                match converted {
                    Some(text) if !arg.is_empty() => text,
                    _ if arg.chars().count() <= 1 => format!("{ch}{arg}"),
                    _ => format!("{ch}({arg})"),
                }
            }
            '&' => {
                self.at += 1;
                "  ".into()
            }
            '~' => {
                self.at += 1;
                " ".into()
            }
            '\n' => {
                self.at += 1;
                " ".into()
            }
            '-' => {
                self.at += 1;
                "−".into()
            }
            '*' => {
                self.at += 1;
                "∗".into()
            }
            _ => {
                self.at += 1;
                ch.to_string()
            }
        }
    }
}

fn wrap(text: &str) -> String {
    if text.chars().count() <= 1 || text.chars().all(|c| c.is_alphanumeric() || matches!(c, '.' | '²' | '³' | '¹')) {
        text.to_owned()
    } else {
        format!("({text})")
    }
}

fn blackboard(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            'R' => 'ℝ',
            'N' => 'ℕ',
            'Z' => 'ℤ',
            'Q' => 'ℚ',
            'C' => 'ℂ',
            'P' => 'ℙ',
            other => other,
        })
        .collect()
}

/// Plain Unicode text for the LaTeX source `tex`.
pub fn to_unicode(tex: &str) -> String {
    let mut parser = Parser { chars: tex.trim().chars().collect(), at: 0, _text: tex };
    let mut out = String::with_capacity(tex.len());
    while parser.peek().is_some() {
        out.push_str(&parser.token());
    }
    // Collapse the runs of spaces that commands like `\,` leave behind.
    let mut collapsed = String::with_capacity(out.len());
    let mut space = false;
    for ch in out.chars() {
        if ch == ' ' {
            if !space {
                collapsed.push(ch);
            }
            space = true;
        } else {
            space = false;
            collapsed.push(ch);
        }
    }
    collapsed.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::to_unicode;

    #[test]
    fn converts_common_formulas() {
        assert_eq!(to_unicode(r"E = mc^2"), "E = mc²");
        assert_eq!(to_unicode(r"\alpha + \beta \leq \gamma"), "α + β ≤ γ");
        assert_eq!(to_unicode(r"\frac{a+b}{2}"), "(a+b)/2");
        assert_eq!(to_unicode(r"\sqrt{x^2+1}"), "√(x²+1)");
        assert_eq!(to_unicode(r"\sum_{i=1}^{n} x_i"), "∑ᵢ₌₁ⁿ xᵢ");
        assert_eq!(to_unicode(r"x_1 \to \infty"), "x₁ → ∞");
        assert_eq!(to_unicode(r"\mathbb{R}^3 \times \text{label}"), "ℝ³ × label");
        assert_eq!(to_unicode(r"\unknown{x}"), "\\unknownx");
    }
}
