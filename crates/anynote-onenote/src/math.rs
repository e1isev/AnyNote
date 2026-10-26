//! OneNote equations → LaTeX.
//!
//! OneNote stores an equation as math-formatted text runs in which
//! U+FDD0 opens a structure, U+FDEE separates its arguments and U+FDEF closes
//! it. The structure's kind (fraction, superscript, n-ary operator, …) comes
//! from the math object attached to the run holding the U+FDD0. Letters use
//! the Unicode "Mathematical Alphanumeric Symbols" block (𝑥, 𝑎, 𝛼).
//!
//! We rebuild the tree from that and print LaTeX that KaTeX can render.

use onenote_parser::contents::{MathInlineObject, MathObjectType};

const OPEN: char = '\u{fdd0}';
const SEP: char = '\u{fdee}';
const CLOSE: char = '\u{fdef}';

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Text,
    Accent,
    Box,
    BoxedFormula,
    Brackets,
    BracketsWithSeps,
    EquationArray,
    Fraction,
    FunctionApply,
    LeftSubSup,
    LowerLimit,
    Matrix,
    Nary,
    OpChar,
    Overbar,
    Phantom,
    Radical,
    SlashedFraction,
    Stack,
    StretchStack,
    Subscript,
    SubSup,
    Superscript,
    Underbar,
    UpperLimit,
}

/// The parts of a parser math object we need.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MathObj {
    pub kind: Kind,
    pub column: Option<u8>,
    pub chars: [Option<char>; 3],
}

impl MathObj {
    pub fn from_parser(m: &MathInlineObject) -> Self {
        use MathObjectType as T;
        let kind = match m.object_type() {
            T::SimpleText | T::PlainText => Kind::Text,
            T::Accent => Kind::Accent,
            T::Box => Kind::Box,
            T::BoxedFormula => Kind::BoxedFormula,
            T::Brackets => Kind::Brackets,
            T::BracketsWithSeps => Kind::BracketsWithSeps,
            T::EquationArray => Kind::EquationArray,
            T::Fraction => Kind::Fraction,
            T::FunctionApply => Kind::FunctionApply,
            T::LeftSubSup => Kind::LeftSubSup,
            T::LowerLimit => Kind::LowerLimit,
            T::Matrix => Kind::Matrix,
            T::Nary => Kind::Nary,
            T::OpChar => Kind::OpChar,
            T::Overbar => Kind::Overbar,
            T::Phantom => Kind::Phantom,
            T::Radical => Kind::Radical,
            T::SlashedFraction => Kind::SlashedFraction,
            T::Stack => Kind::Stack,
            T::StretchStack => Kind::StretchStack,
            T::Subscript => Kind::Subscript,
            T::SubSup => Kind::SubSup,
            T::Superscript => Kind::Superscript,
            T::Underbar => Kind::Underbar,
            T::UpperLimit => Kind::UpperLimit,
        };
        MathObj { kind, column: m.column(), chars: [m.char(), m.char1(), m.char2()] }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum M {
    Text(String),
    Obj(Option<MathObj>, Vec<Vec<M>>),
}

/// Convert math runs (text plus the object attached to each run) to LaTeX.
pub fn to_latex(runs: &[(String, Option<MathObj>)]) -> String {
    // Build the tree with a stack of open structures.
    let mut stack: Vec<(Option<MathObj>, Vec<Vec<M>>)> = Vec::new();
    let mut root: Vec<M> = Vec::new();
    fn current<'a>(root: &'a mut Vec<M>, stack: &'a mut [(Option<MathObj>, Vec<Vec<M>>)]) -> &'a mut Vec<M> {
        match stack.last_mut() {
            Some((_, args)) => args.last_mut().expect("structure has an argument"),
            None => root,
        }
    }
    for (text, obj) in runs {
        for c in text.chars() {
            match c {
                OPEN => stack.push((*obj, vec![Vec::new()])),
                SEP => {
                    if let Some((_, args)) = stack.last_mut() {
                        args.push(Vec::new());
                    }
                }
                CLOSE => {
                    if let Some((o, args)) = stack.pop() {
                        current(&mut root, &mut stack).push(M::Obj(o, args));
                    }
                }
                c => {
                    let target = current(&mut root, &mut stack);
                    match target.last_mut() {
                        Some(M::Text(s)) => s.push(c),
                        _ => target.push(M::Text(c.to_string())),
                    }
                }
            }
        }
    }
    // Close anything left open by a truncated equation.
    while let Some((o, args)) = stack.pop() {
        current(&mut root, &mut stack).push(M::Obj(o, args));
    }
    let out = render(&root);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn render(nodes: &[M]) -> String {
    let mut out = String::new();
    for n in nodes {
        let piece = match n {
            M::Text(t) => text(t),
            M::Obj(obj, args) => object(obj.as_ref(), args),
        };
        out.push_str(&piece);
    }
    out
}

fn arg(args: &[Vec<M>], i: usize) -> String {
    args.get(i).map(|a| render(a)).unwrap_or_default()
}

fn group(s: &str) -> String {
    format!("{{{s}}}")
}

fn object(obj: Option<&MathObj>, args: &[Vec<M>]) -> String {
    let Some(obj) = obj else {
        return args.iter().map(|a| render(a)).collect::<Vec<_>>().join(" ");
    };
    let ch = |i: usize| obj.chars[i];
    let a = |i: usize| arg(args, i);
    match obj.kind {
        Kind::Superscript => format!("{}^{}", group(&a(0)), group(&a(1))),
        Kind::Subscript => format!("{}_{}", group(&a(0)), group(&a(1))),
        Kind::SubSup => format!("{}_{}^{}", group(&a(0)), group(&a(1)), group(&a(2))),
        Kind::LeftSubSup => format!("{{}}_{}^{}{}", group(&a(0)), group(&a(1)), group(&a(2))),
        Kind::Fraction => match ch(0) {
            Some('⁄') | Some('∕') => format!("{}/{}", group(&a(0)), group(&a(1))),
            _ => format!("\\frac{}{}", group(&a(0)), group(&a(1))),
        },
        Kind::SlashedFraction => format!("{}/{}", group(&a(0)), group(&a(1))),
        Kind::Stack => format!("\\genfrac{{}}{{}}{{0pt}}{{}}{}{}", group(&a(0)), group(&a(1))),
        Kind::Nary => {
            let op = match ch(0) {
                Some('∑') => "\\sum",
                Some('∏') => "\\prod",
                Some('∐') => "\\coprod",
                Some('∫') => "\\int",
                Some('∬') => "\\iint",
                Some('∭') => "\\iiint",
                Some('∮') => "\\oint",
                Some('⋃') => "\\bigcup",
                Some('⋂') => "\\bigcap",
                _ => "\\int",
            };
            let (lower, upper, body) = (a(0), a(1), a(2));
            let mut s = op.to_string();
            if !lower.trim().is_empty() {
                s.push('_');
                s.push_str(&group(&lower));
            }
            if !upper.trim().is_empty() {
                s.push('^');
                s.push_str(&group(&upper));
            }
            s.push(' ');
            s.push_str(&body);
            s
        }
        Kind::Brackets | Kind::BracketsWithSeps => {
            let open = delimiter(ch(0).unwrap_or('('), true);
            let close = delimiter(ch(1).unwrap_or(')'), false);
            let sep = match ch(2) {
                Some(c) => format!(" {} ", delimiter_middle(c)),
                None => " , ".into(),
            };
            let inner = args.iter().map(|x| render(x)).collect::<Vec<_>>().join(&sep);
            format!("\\left{open} {inner} \\right{close}")
        }
        Kind::Matrix => {
            let cells: Vec<String> = args.iter().map(|x| render(x)).collect();
            let n = cells.len().max(1);
            let cols = obj.column.map(usize::from).filter(|c| n.checked_rem(*c) == Some(0)).unwrap_or_else(|| {
                let r = (n as f64).sqrt() as usize;
                if r * r == n {
                    r
                } else {
                    n
                }
            });
            let rows: Vec<String> = cells.chunks(cols).map(|r| r.join(" & ")).collect();
            format!("\\begin{{matrix}} {} \\end{{matrix}}", rows.join(" \\\\ "))
        }
        Kind::EquationArray => {
            let rows: Vec<String> = args.iter().map(|x| render(x)).collect();
            format!("\\begin{{aligned}} {} \\end{{aligned}}", rows.join(" \\\\ "))
        }
        Kind::FunctionApply => format!("{} {}", function_name(&args[0]), a(1)),
        Kind::LowerLimit => format!("{}_{}", function_name(&args[0]), group(&a(1))),
        Kind::UpperLimit => format!("{}^{}", function_name(&args[0]), group(&a(1))),
        Kind::Radical => {
            let (degree, radicand) = if args.len() >= 2 { (a(0), a(1)) } else { (String::new(), a(0)) };
            if degree.trim().is_empty() {
                format!("\\sqrt{}", group(&radicand))
            } else {
                format!("\\sqrt[{degree}]{}", group(&radicand))
            }
        }
        Kind::Accent => {
            let cmd = match ch(0) {
                Some('\u{302}') | Some('^') => "\\hat",
                Some('\u{303}') | Some('~') => "\\tilde",
                Some('\u{307}') => "\\dot",
                Some('\u{308}') => "\\ddot",
                Some('\u{20d7}') | Some('\u{20d1}') => "\\vec",
                Some('\u{304}') | Some('\u{305}') | Some('\u{af}') => "\\bar",
                Some('\u{30c}') => "\\check",
                Some('\u{306}') => "\\breve",
                _ => "",
            };
            if cmd.is_empty() {
                a(0)
            } else {
                format!("{cmd}{}", group(&a(0)))
            }
        }
        Kind::Overbar => format!("\\overline{}", group(&a(0))),
        Kind::Underbar => format!("\\underline{}", group(&a(0))),
        Kind::BoxedFormula => format!("\\boxed{}", group(&a(0))),
        Kind::Box | Kind::Phantom | Kind::OpChar | Kind::Text | Kind::StretchStack => {
            args.iter().map(|x| render(x)).collect::<Vec<_>>().join(" ")
        }
    }
}

/// A function name slot: `sin` → `\sin`, `lim` → `\lim`.
fn function_name(nodes: &[M]) -> String {
    if let [M::Text(t)] = nodes {
        let name = text(t);
        let known = [
            "sin", "cos", "tan", "cot", "sec", "csc", "arcsin", "arccos", "arctan", "sinh", "cosh", "tanh", "coth",
            "log", "ln", "lg", "exp", "lim", "liminf", "limsup", "max", "min", "sup", "inf", "det", "dim", "ker",
            "deg", "gcd", "arg", "Pr",
        ];
        if known.contains(&name.trim()) {
            return format!("\\{} ", name.trim());
        }
        if name.trim().chars().count() > 1 && name.trim().chars().all(|c| c.is_ascii_alphabetic()) {
            return format!("\\operatorname{{{}}}", name.trim());
        }
        return name;
    }
    render(nodes)
}

fn delimiter(c: char, open: bool) -> String {
    match c {
        '(' | ')' | '[' | ']' | '|' | '/' | '.' => c.to_string(),
        '{' => "\\{".into(),
        '}' => "\\}".into(),
        '⟨' => "\\langle".into(),
        '⟩' => "\\rangle".into(),
        '‖' => "\\|".into(),
        '⌊' => "\\lfloor".into(),
        '⌋' => "\\rfloor".into(),
        '⌈' => "\\lceil".into(),
        '⌉' => "\\rceil".into(),
        _ => {
            if open {
                "(".into()
            } else {
                ")".into()
            }
        }
    }
}

fn delimiter_middle(c: char) -> String {
    match c {
        '|' => "\\mid".into(),
        '‖' => "\\|".into(),
        c => c.to_string(),
    }
}

/// Plain math text to LaTeX: normalise mathematical alphanumerics and map
/// symbols to commands.
fn text(t: &str) -> String {
    let mut out = String::new();
    for c in t.chars() {
        let c = normalize_letter(c);
        let rep: Option<&str> = match c {
            '\u{2061}'..='\u{2064}' | '\u{200b}' | '⬚' => Some(""),
            'α' => Some("\\alpha "),
            'β' => Some("\\beta "),
            'γ' => Some("\\gamma "),
            'δ' => Some("\\delta "),
            'ε' => Some("\\varepsilon "),
            'ϵ' => Some("\\epsilon "),
            'ζ' => Some("\\zeta "),
            'η' => Some("\\eta "),
            'θ' => Some("\\theta "),
            'ϑ' => Some("\\vartheta "),
            'ι' => Some("\\iota "),
            'κ' => Some("\\kappa "),
            'λ' => Some("\\lambda "),
            'μ' => Some("\\mu "),
            'ν' => Some("\\nu "),
            'ξ' => Some("\\xi "),
            'π' => Some("\\pi "),
            'ρ' => Some("\\rho "),
            'σ' => Some("\\sigma "),
            'ς' => Some("\\varsigma "),
            'τ' => Some("\\tau "),
            'υ' => Some("\\upsilon "),
            'φ' => Some("\\varphi "),
            'ϕ' => Some("\\phi "),
            'χ' => Some("\\chi "),
            'ψ' => Some("\\psi "),
            'ω' => Some("\\omega "),
            'Γ' => Some("\\Gamma "),
            'Δ' => Some("\\Delta "),
            'Θ' => Some("\\Theta "),
            'Λ' => Some("\\Lambda "),
            'Ξ' => Some("\\Xi "),
            'Π' => Some("\\Pi "),
            'Σ' => Some("\\Sigma "),
            'Υ' => Some("\\Upsilon "),
            'Φ' => Some("\\Phi "),
            'Ψ' => Some("\\Psi "),
            'Ω' => Some("\\Omega "),
            '∑' => Some("\\sum "),
            '∏' => Some("\\prod "),
            '∫' => Some("\\int "),
            '∬' => Some("\\iint "),
            '∮' => Some("\\oint "),
            '√' => Some("\\surd "),
            '∞' => Some("\\infty "),
            '±' => Some("\\pm "),
            '∓' => Some("\\mp "),
            '×' => Some("\\times "),
            '÷' => Some("\\div "),
            '·' | '⋅' => Some("\\cdot "),
            '∘' => Some("\\circ "),
            '≤' => Some("\\le "),
            '≥' => Some("\\ge "),
            '≠' => Some("\\ne "),
            '≈' => Some("\\approx "),
            '≡' => Some("\\equiv "),
            '∼' => Some("\\sim "),
            '≅' => Some("\\cong "),
            '∈' => Some("\\in "),
            '∉' => Some("\\notin "),
            '∋' => Some("\\ni "),
            '⊂' => Some("\\subset "),
            '⊃' => Some("\\supset "),
            '⊆' => Some("\\subseteq "),
            '⊇' => Some("\\supseteq "),
            '∪' => Some("\\cup "),
            '∩' => Some("\\cap "),
            '∅' => Some("\\emptyset "),
            '∀' => Some("\\forall "),
            '∃' => Some("\\exists "),
            '¬' => Some("\\neg "),
            '∧' => Some("\\wedge "),
            '∨' => Some("\\vee "),
            '∂' => Some("\\partial "),
            '∇' => Some("\\nabla "),
            '→' => Some("\\to "),
            '←' => Some("\\leftarrow "),
            '↔' => Some("\\leftrightarrow "),
            '⇒' => Some("\\Rightarrow "),
            '⇐' => Some("\\Leftarrow "),
            '⇔' => Some("\\Leftrightarrow "),
            '↦' => Some("\\mapsto "),
            '∝' => Some("\\propto "),
            '∠' => Some("\\angle "),
            '°' => Some("^\\circ "),
            '…' => Some("\\ldots "),
            '⋯' => Some("\\cdots "),
            '⋮' => Some("\\vdots "),
            '⋱' => Some("\\ddots "),
            'ℏ' => Some("\\hbar "),
            'ℓ' => Some("\\ell "),
            '′' => Some("'"),
            '″' => Some("''"),
            'ℝ' => Some("\\mathbb{R}"),
            'ℕ' => Some("\\mathbb{N}"),
            'ℤ' => Some("\\mathbb{Z}"),
            'ℚ' => Some("\\mathbb{Q}"),
            'ℂ' => Some("\\mathbb{C}"),
            '−' => Some("-"),
            '∗' => Some("*"),
            '{' => Some("\\{"),
            '}' => Some("\\}"),
            '%' => Some("\\%"),
            '#' => Some("\\#"),
            '&' => Some("\\&"),
            '$' => Some("\\$"),
            '_' => Some("\\_"),
            '\\' => Some("\\backslash "),
            c if c.is_control() => Some(""),
            _ => None,
        };
        match rep {
            Some(r) => out.push_str(r),
            None => out.push(c),
        }
    }
    out
}

/// Map Mathematical Alphanumeric Symbols (𝑥, 𝐀, 𝛼, 𝟏) to plain letters.
fn normalize_letter(c: char) -> char {
    let u = c as u32;
    match u {
        0x210E => 'h', // ℎ planck constant = italic h
        0x2145 => 'D', // ⅅ
        0x2146 => 'd', // ⅆ
        0x2147 => 'e', // ⅇ
        0x2148 => 'i', // ⅈ
        0x1D400..=0x1D6A3 => {
            let i = (u - 0x1D400) % 52;
            char::from_u32(if i < 26 { 'A' as u32 + i } else { 'a' as u32 + i - 26 }).unwrap_or(c)
        }
        0x1D6A8..=0x1D7C9 => {
            let i = (u - 0x1D6A8) % 58;
            match i {
                0..=24 => char::from_u32(0x391 + i + u32::from(i >= 17)).unwrap_or(c), // skip U+03A2
                25 => '∇',
                26..=50 => char::from_u32(0x3B1 + i - 26).unwrap_or(c),
                51 => '∂',
                52 => 'ϵ',
                53 => 'ϑ',
                54 => 'ϰ',
                55 => 'ϕ',
                56 => 'ϱ',
                _ => 'ϖ',
            }
        }
        0x1D7CE..=0x1D7FF => char::from_u32('0' as u32 + (u - 0x1D7CE) % 10).unwrap_or(c),
        _ => c,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(kind: Kind, c: Option<char>) -> Option<MathObj> {
        Some(MathObj { kind, column: None, chars: [c, None, None] })
    }

    fn run(t: &str, o: Option<MathObj>) -> (String, Option<MathObj>) {
        (t.to_string(), o)
    }

    #[test]
    fn superscripts_and_letters() {
        // ﷐𝑎﷮2﷯+﷐𝑏﷮2﷯ as OneNote splits it into runs.
        let sup = obj(Kind::Superscript, Some('^'));
        let runs = [
            run("\u{fdd0}", sup),
            run("𝑎\u{fdee}", obj(Kind::Superscript, None)),
            run("2\u{fdef}", obj(Kind::Superscript, None)),
            run("+", obj(Kind::Text, None)),
            run("\u{fdd0}", sup),
            run("𝑏\u{fdee}", None),
            run("2\u{fdef}", None),
        ];
        assert_eq!(to_latex(&runs), "{a}^{2}+{b}^{2}");
    }

    #[test]
    fn fractions_integrals_functions() {
        let frac = obj(Kind::Fraction, Some('/'));
        let nary = obj(Kind::Nary, Some('∫'));
        let runs = [
            run("\u{fdd0}", nary),
            run("0\u{fdee}1\u{fdee}𝑥\u{fdef}ⅆ𝑥=", None),
            run("\u{fdd0}", frac),
            run("1\u{fdee}2\u{fdef}", None),
        ];
        assert_eq!(to_latex(&runs), "\\int_{0}^{1} xdx=\\frac{1}{2}");

        let apply = obj(Kind::FunctionApply, Some('\u{2061}'));
        let runs = [run("\u{fdd0}", apply), run("cos\u{fdee}𝜃\u{fdef}", None)];
        assert_eq!(to_latex(&runs), "\\cos \\theta");
    }

    #[test]
    fn matrices_and_brackets() {
        let brackets = Some(MathObj { kind: Kind::Brackets, column: None, chars: [Some('|'), Some('|'), None] });
        let matrix = obj(Kind::Matrix, Some('■'));
        let runs = [
            run("\u{fdd0}", brackets),
            run("\u{fdd0}", matrix),
            run("𝑎\u{fdee}𝑏\u{fdee}𝑐\u{fdee}𝑑\u{fdef}\u{fdef}", None),
        ];
        assert_eq!(to_latex(&runs), "\\left| \\begin{matrix} a & b \\\\ c & d \\end{matrix} \\right|");
    }

    #[test]
    fn unbalanced_input_does_not_panic() {
        assert_eq!(to_latex(&[run("\u{fdef}\u{fdee}𝑥\u{fdd0}", None)]), "x");
    }
}
