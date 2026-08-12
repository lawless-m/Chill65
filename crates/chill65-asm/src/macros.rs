//! Macro definition and expansion.
//!
//! The load-bearing component. The 6502 instruction set itself arrives as a
//! macro package (`M6502.MAC`), and `HLL65F.MAC` supplies the structured
//! control flow the whole recompiler project rests on.
//!
//! # Argument separators
//!
//! Definitions use commas (`.MACRO TRAI FROM,TO`) but call sites use spaces
//! (`TRAI 2*EN.MAX EN.NUM`), and some definitions use spaces too
//! (`.MACRO ADAI VAL MEM`). Since the lexer discards whitespace, splitting a
//! call site's arguments relies on [`Token::space_before`] — without it,
//! `2*EN.MAX EN.NUM` is indistinguishable from one expression.
//!
//! # Concatenation
//!
//! `'` joins a parameter to adjacent text: `HLL65F.MAC` builds mnemonics with
//! `B'COND` (→ `BNE`) and `IF'.B`. The lexer deliberately leaves `'` as an
//! uninterpreted [`Tok::Quote`] because it is also a literal delimiter in
//! `.PRINT`; the decision is made here, where macro context is known.
//!
//! # Generated labels
//!
//! A parameter written `?B` mints a fresh unique symbol per expansion, so
//! `INC16`'s internal branch target cannot collide with itself.

use crate::lexer::{Span, Tok, Token};

#[derive(Clone, Debug)]
pub struct Param {
    pub name: String,
    /// Declared `?NAME`: each expansion substitutes a fresh unique label.
    pub generated: bool,
}

#[derive(Clone, Debug)]
pub struct MacroDef {
    pub name: String,
    pub params: Vec<Param>,
    /// Body lines, each without its terminating `Eol`.
    pub body: Vec<Vec<Token>>,
}

#[derive(Debug)]
pub struct ExpandError {
    pub message: String,
}

fn synthetic(tok: Tok, span: &Span, space_before: bool) -> Token {
    Token {
        tok,
        span: span.clone(),
        space_before,
    }
}

/// Parse the parameter list of a `.MACRO` line.
///
/// Accepts both comma and space separation, and `?NAME` for generated labels.
pub fn parse_params(toks: &[Token]) -> Vec<Param> {
    let mut out = Vec::new();
    let mut generated = false;
    for t in toks {
        match &t.tok {
            Tok::Punct('?') => generated = true,
            Tok::Punct(',') => {}
            Tok::Symbol(s) => {
                out.push(Param {
                    name: s.clone(),
                    generated,
                });
                generated = false;
            }
            // `.MACRO CMPIN A,B` — `A,` was classified as an addressing-mode
            // prefix by the lexer, which cannot know it is in a parameter list.
            // Recover the name; the comma it swallowed was the separator.
            Tok::Prefix(m) => {
                out.push(Param {
                    name: m.name().to_string(),
                    generated,
                });
                generated = false;
            }
            Tok::Eol | Tok::Comment(_) => break,
            _ => {}
        }
    }
    out
}

/// Split a call site's argument list.
///
/// Arguments are separated by commas *or* whitespace. Angle brackets group, so
/// `.IF IDN,<.A>,<.OR.>` survives, and `TRAM EN.MA2(X) EZ.MA2` splits into two
/// arguments rather than four tokens.
pub fn split_args(toks: &[Token]) -> Vec<Vec<Token>> {
    let mut out: Vec<Vec<Token>> = Vec::new();
    let mut cur: Vec<Token> = Vec::new();
    let mut depth = 0i32;

    for (i, t) in toks.iter().enumerate() {
        match &t.tok {
            Tok::Eol | Tok::Comment(_) => break,
            Tok::Punct('<') => {
                depth += 1;
                cur.push(t.clone());
            }
            Tok::Punct('>') => {
                depth -= 1;
                cur.push(t.clone());
            }
            Tok::Punct(',') if depth == 0 => {
                out.push(std::mem::take(&mut cur));
            }
            // Same recovery at the call site: `CMPIN A,B` passes `A` as an
            // argument, and the prefix token already consumed the comma.
            Tok::Prefix(m) if depth == 0 => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
                out.push(vec![Token {
                    tok: Tok::Symbol(m.name().to_string()),
                    span: t.span.clone(),
                    space_before: t.space_before,
                }]);
            }
            _ => {
                // A gap at depth zero starts a new argument, but only after we
                // already have something — leading whitespace is not a
                // separator.
                if depth == 0 && t.space_before && !cur.is_empty() && i > 0 {
                    out.push(std::mem::take(&mut cur));
                }
                cur.push(t.clone());
            }
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    // Angle brackets wrapping a *whole* argument delimit it; they are not part
    // of it. MACRO-11 spells expression grouping the same way, so leaving them
    // in place meant the body evaluated them as an expression:
    // `DNEGATE <X,YINCL-ZSHIP>,<X,YINC>` (`ASTRD2.MAC:3684`) substituted into
    // `SBC AA` gave `SBC <X,YINCL-ZSHIP>`, and the evaluator met the index
    // prefix where it wanted an operand. Unwrapped, it is the indexed operand
    // the original assembled.
    //
    // Only a group that spans the entire argument is stripped — `<A>+<B>` is
    // one expression with two groups in it, and keeps both.
    for arg in &mut out {
        if arg.len() < 2
            || !matches!(arg[0].tok, Tok::Punct('<'))
            || !matches!(arg[arg.len() - 1].tok, Tok::Punct('>'))
        {
            continue;
        }
        let mut depth = 0i32;
        let spans_all = arg.iter().enumerate().all(|(i, t)| {
            match &t.tok {
                Tok::Punct('<') => depth += 1,
                Tok::Punct('>') => depth -= 1,
                _ => {}
            }
            // The opening bracket may only close at the very last token.
            depth > 0 || i == arg.len() - 1
        });
        if spans_all {
            arg.remove(arg.len() - 1);
            arg.remove(0);
        }
    }
    out
}

impl MacroDef {
    /// Expand this macro with `args`, returning body lines with parameters
    /// substituted.
    ///
    /// `gensym` supplies the counter for `?` parameters.
    pub fn expand(
        &self,
        args: &[Vec<Token>],
        gensym: &mut u32,
    ) -> Result<Vec<Vec<Token>>, ExpandError> {
        // Bind parameters. Missing arguments bind to empty, which is what the
        // `.IF B,<COND>` / `.IF NB,<COND>` tests in HLL65F rely on.
        let mut bind: Vec<(String, Vec<Token>)> = Vec::new();
        for (i, p) in self.params.iter().enumerate() {
            // A `?` formal generates a label only when the actual is omitted
            // or empty. `CMAC.MAC` declares `.MACRO BINT INTVAL,?B,?C` and is
            // called as `BINT 2,40$` — the caller names the branch target `B`
            // while `C` stays internal and is minted. Always generating would
            // point `JMP B` at a label nobody defines.
            let supplied = args.get(i).is_some_and(|a| !a.is_empty());
            let value = if p.generated && !supplied {
                *gensym += 1;
                let name = format!("~G{:05}", *gensym);
                vec![synthetic(
                    Tok::Symbol(name),
                    &Span {
                        file: "<macro>".into(),
                        line: 0,
                        col: 0,
                    },
                    false,
                )]
            } else {
                args.get(i).cloned().unwrap_or_default()
            };
            bind.push((p.name.clone(), value));
        }

        let mut out = Vec::new();
        for line in &self.body {
            out.push(substitute(line, &bind));
        }
        Ok(out)
    }
}

/// Substitute parameters through one body line, resolving `'` concatenation.
fn substitute(line: &[Token], bind: &[(String, Vec<Token>)]) -> Vec<Token> {
    fn lookup<'b>(bind: &'b [(String, Vec<Token>)], name: &str) -> Option<&'b Vec<Token>> {
        bind.iter().find(|(p, _)| p == name).map(|(_, v)| v)
    }

    // Pass one: expand each token, tracking whether it came from a parameter.
    let mut out: Vec<Token> = Vec::new();
    let mut i = 0usize;
    while i < line.len() {
        let t = &line[i];

        // Concatenation marks, and how they are told apart from the
        // character-value operator.
        //
        // MACRO-11 spells both with `'`, and the lexer deliberately leaves it
        // uninterpreted (see the module comment above) because only this layer
        // knows the macro context. The distinction is **adjacency**: a quote
        // that touches the token before it is a concatenation mark, and one
        // with a space in front of it is the operator.
        //
        // Three forms appear, two of them in Space Duel alone:
        //
        // ```text
        // B'COND          mark — Crystal Castles, HLL65F
        // LABEL''X''Y     marks — fuse LABEL, X and Y into one symbol
        // ...4=''...5     operator then mark — the code of ...5's character
        // ```
        //
        // A quote is a **mark for this expansion** when one of the tokens it
        // touches is a parameter *this* expansion binds. Anything else is the
        // operator, or a mark belonging to an expansion further in.
        //
        // The adjacency alone will not do, and this is the case that proves it.
        // `VGMC.MAC`'s ALPHA macro wraps an `.IRPC` around its parameter:
        //
        // ```text
        // .MACRO ALPHA STRING
        // .IRPC ...X,<STRING>
        // JSRL CHAR.'...X
        // ```
        //
        // When ALPHA expands, `...X` is not bound — `.IRPC` has not run yet.
        // Treating the quote as a mark because `CHAR.` sits against it fused
        // the pair into the literal `CHAR....X` and consumed the quote, so the
        // `.IRPC` inside had nothing left to substitute into. Requiring a bound
        // neighbour leaves the quote alone for the inner expansion, which does
        // bind `...X` and joins it properly.
        //
        // Against the three forms that occur:
        //
        // ```text
        // B'COND          mark — COND is bound on the right
        // LABEL''X''Y     marks — X and Y are bound
        // ...4=''...5     operator then mark — the first quote touches `=` and
        //                 another quote, neither bound; the second touches
        //                 `...5`, which `.IRPC` binds
        // ```
        //
        // Getting this wrong in the other direction fused `.BYTE` with its
        // quote and produced a mnemonic named `.BYTE'`; `.BYTE` is never a
        // bound parameter, so that cannot recur.
        let bound = |k: usize| {
            matches!(&line[k].tok, Tok::Symbol(s) if lookup(bind, s).is_some())
        };
        let is_mark = |k: usize| {
            if !matches!(line[k].tok, Tok::Quote) {
                return false;
            }
            let left = k > 0 && !line[k].space_before && bound(k - 1);
            let right = line
                .get(k + 1)
                .is_some_and(|n| !n.space_before && bound(k + 1));
            left || right
        };

        if is_mark(i) {
            // Doubled marks are one join, not two: `LABEL''X` closes LABEL and
            // opens X. Skip the run of marks and take the token beyond it.
            let mut k = i;
            while k < line.len() && is_mark(k) {
                k += 1;
            }
            // ...but only when both marks are *ours*. A doubled mark can
            // straddle two expansions, and then the second one is not this
            // expansion's to consume.
            //
            // `AS2POK.MAC`'s `OFFSET` macro writes `LABEL''X''Y` inside
            // `.IRPC X` inside `.IRPC Y`. Expanding the macro binds only
            // `LABEL`, so the first mark is its own and the second belongs to
            // the `X` loop. Fusing them both turned the mark character itself
            // into part of the name — `SF` and `'` became the symbol `SF'`,
            // and the sound-pointer tables came out all zero because
            // `.IF DF,LABEL''X''Y` could never be true.
            if matches!(line.get(k).map(|t| &t.tok), Some(Tok::Quote)) {
                i = k;
                continue;
            }
            if let Some(next) = line.get(k) {
                let text = render(next, bind);
                match out.last_mut() {
                    // Fuse with what is already there, unless that is the
                    // character-value operator waiting for its character.
                    Some(last) if !matches!(last.tok, Tok::Quote) => {
                        // A blank cannot extend a symbol name — it ends one. So
                        // the join takes the text only as far as the first one.
                        //
                        // `VGMC.MAC`'s `ALPHA` runs `.IRPC ...X,<STRING>` over
                        // `JSRL CHAR.'...X`, and `AS2ROM.MAC:1097` passes
                        // `<MCMLXXX ATARI IN>`. On the two blank iterations the
                        // join is `CHAR.` and a space, which named a symbol
                        // `"CHAR. "` that nothing defines; the blank glyph is
                        // `CHAR.:` at `VGAN.MAC:129`.
                        let joined = format!("{}{}", tok_text(last), text);
                        let fused = joined
                            .trim_start()
                            .split_whitespace()
                            .next()
                            .unwrap_or("")
                            .to_string();
                        let (span, sp) = (last.span.clone(), last.space_before);
                        *last = synthetic(Tok::Symbol(fused), &span, sp);
                    }
                    _ => out.push(synthetic(Tok::Symbol(text), &next.span, false)),
                }
                i = k + 1;
                continue;
            }
            i = k;
            continue;
        }

        match &t.tok {
            Tok::Symbol(name) => match lookup(bind, name) {
                Some(v) => {
                    for (k, vt) in v.iter().enumerate() {
                        let mut nt = vt.clone();
                        if k == 0 {
                            nt.space_before = t.space_before;
                        }
                        out.push(nt);
                    }
                }
                None => out.push(t.clone()),
            },
            // A parameter used as a label *definition* must substitute too.
            // `INC16 ADDR,?B` ends its body with `B:`, and if only the
            // reference `BNE B` were substituted the generated label would be
            // referenced and never defined.
            Tok::LabelDef { name, global } => match lookup(bind, name) {
                Some(v) => {
                    let text: String = v.iter().map(tok_text).collect();
                    out.push(synthetic(
                        Tok::LabelDef {
                            name: text,
                            global: *global,
                        },
                        &t.span,
                        t.space_before,
                    ));
                }
                None => out.push(t.clone()),
            },
            _ => out.push(t.clone()),
        }
        i += 1;
    }
    out
}

/// Render a token as text for concatenation, substituting parameters.
fn render(t: &Token, bind: &[(String, Vec<Token>)]) -> String {
    match &t.tok {
        Tok::Symbol(name) => match bind.iter().find(|(p, _)| p == name).map(|(_, v)| v) {
            Some(v) => v.iter().map(tok_text).collect(),
            None => name.clone(),
        },
        other => tok_text_of(other),
    }
}

pub fn token_text(t: &Token) -> String {
    tok_text(t)
}

fn tok_text(t: &Token) -> String {
    tok_text_of(&t.tok)
}

fn tok_text_of(t: &Tok) -> String {
    match t {
        Tok::Symbol(s) => s.clone(),
        Tok::Number { text, .. } => text.clone(),
        Tok::Dot => ".".into(),
        Tok::Punct(c) => c.to_string(),
        Tok::LocalLabel(n) => format!("{n}$"),
        // Prefixes render with their comma, as written: `LDA I,FROM`.
        Tok::Prefix(m) => format!("{m:?},").to_uppercase(),
        Tok::Quote => "'".into(),
        // A delimited string is its contents, spaces and all. `.NCHR` counts
        // those characters and `.IRPC` iterates them, so rendering it empty
        // silently dropped every blank-padded macro argument.
        Tok::Str(s) => s.clone(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;

    fn toks(s: &str) -> Vec<Token> {
        let mut v = Lexer::new("t").tokenise(s);
        v.retain(|t| !matches!(t.tok, Tok::Eol));
        v
    }

    fn text(line: &[Token]) -> String {
        line.iter()
            .map(|t| {
                let s = tok_text(t);
                if t.space_before && !s.is_empty() {
                    format!(" {s}")
                } else {
                    s
                }
            })
            .collect::<String>()
            .trim()
            .to_string()
    }

    #[test]
    fn splits_space_separated_call_arguments() {
        // TRAI 2*EN.MAX EN.NUM — two arguments, not one expression.
        let a = split_args(&toks("2*EN.MAX EN.NUM"));
        assert_eq!(a.len(), 2, "got {:?}", a.iter().map(|x| text(x)).collect::<Vec<_>>());
        assert_eq!(text(&a[0]), "2*EN.MAX");
        assert_eq!(text(&a[1]), "EN.NUM");
    }

    #[test]
    fn splits_comma_separated_call_arguments() {
        let a = split_args(&toks("PKVN,TNUM"));
        assert_eq!(a.len(), 2);
        assert_eq!(text(&a[0]), "PKVN");
        assert_eq!(text(&a[1]), "TNUM");
    }

    #[test]
    fn keeps_parenthesised_operand_whole() {
        // TR16AM EN.MA2(X) EZ.MA2
        let a = split_args(&toks("EN.MA2(X) EZ.MA2"));
        assert_eq!(a.len(), 2, "got {:?}", a.iter().map(|x| text(x)).collect::<Vec<_>>());
        assert_eq!(text(&a[0]), "EN.MA2(X)");
    }

    #[test]
    fn angle_brackets_group_arguments() {
        // The brackets group; they are not part of what they group. This test
        // previously asserted `<.A>` came back with its brackets, which was
        // incidental to what it names and turned out to be wrong: MACRO-11
        // spells expression grouping the same way, so a retained pair was
        // evaluated as an expression once the argument reached a macro body.
        //
        // Nothing depended on the old text. `.IF IDN` and `.IF NB` call
        // `strip_angles` on their own operands (`assemble.rs`), so they see the
        // same thing either way.
        let a = split_args(&toks("IDN,<.A>,<.OR.>"));
        assert_eq!(a.len(), 3, "the brackets still separate three arguments");
        assert_eq!(text(&a[1]), ".A");
        assert_eq!(text(&a[2]), ".OR.");

        // A group that does not span the whole argument is an expression and
        // keeps its brackets.
        let b = split_args(&toks("<ZZ*2>+<DX&3>"));
        assert_eq!(b.len(), 1);
        assert_eq!(text(&b[0]), "<ZZ*2>+<DX&3>");
    }

    #[test]
    fn parses_parameter_lists_both_ways() {
        let p = parse_params(&toks("FROM,TO"));
        assert_eq!(p.len(), 2);
        assert!(!p[0].generated);

        // .MACRO ADAI VAL MEM — space separated in the definition too.
        let p = parse_params(&toks("VAL MEM"));
        assert_eq!(p.len(), 2);

        // .MACRO INC16 ADDR,?B — the ? marks a generated label.
        let p = parse_params(&toks("ADDR,?B"));
        assert_eq!(p.len(), 2);
        assert!(!p[0].generated);
        assert!(p[1].generated, "?B must be a generated label");
    }

    fn def(name: &str, params: &str, body: &[&str]) -> MacroDef {
        MacroDef {
            name: name.into(),
            params: parse_params(&toks(params)),
            body: body.iter().map(|l| toks(l)).collect(),
        }
    }

    #[test]
    fn trai_expands_to_lda_immediate_and_sta() {
        // .MACRO TRAI FROM,TO / LDA I,FROM / STA TO / .ENDM
        let m = def("TRAI", "FROM,TO", &["	LDA I,FROM", "	STA TO"]);
        let mut g = 0;
        let out = m.expand(&split_args(&toks("0FF IN.BSL")), &mut g).unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(text(&out[0]), "LDA I,0FF");
        assert!(matches!(out[0][1].tok, Tok::Prefix(crate::lexer::Mode::I)));
        assert_eq!(text(&out[1]), "STA IN.BSL");
    }

    #[test]
    fn tr16ai_masking_expands() {
        // LDA I,FROM&^H0FF / STA TO / LDA I,FROM&^H0FF00/^H100 / STA 1+TO
        let m = def(
            "TR16AI",
            "FROM,TO",
            &[
                "	LDA I,FROM&^H0FF",
                "	STA TO",
                "	LDA I,FROM&^H0FF00/^H100",
                "	STA 1+TO",
            ],
        );
        let mut g = 0;
        let out = m.expand(&split_args(&toks("CTROM CT.ROM")), &mut g).unwrap();
        assert_eq!(text(&out[0]), "LDA I,CTROM&0FF");
        assert_eq!(text(&out[1]), "STA CT.ROM");
        assert_eq!(text(&out[2]), "LDA I,CTROM&0FF00/100");
        assert_eq!(text(&out[3]), "STA 1+CT.ROM");
    }

    #[test]
    fn generated_labels_are_unique_per_expansion() {
        // .MACRO INC16 ADDR,?B / INC ADDR / BNE B / INC 1+ADDR / B: / .ENDM
        let m = def("INC16", "ADDR,?B", &["	INC ADDR", "	BNE B", "	INC 1+ADDR", "B:"]);
        let mut g = 0;
        let a = m.expand(&split_args(&toks("CT.ROM")), &mut g).unwrap();
        let b = m.expand(&split_args(&toks("CT.RAM")), &mut g).unwrap();

        let label_a = text(&a[1]);
        let label_b = text(&b[1]);
        assert!(label_a.starts_with("BNE ~"), "got {label_a}");
        assert_ne!(
            label_a, label_b,
            "two expansions minted the same label — they would collide"
        );
    }

    #[test]
    fn generated_parameter_yields_to_a_supplied_argument() {
        // CMAC.MAC: .MACRO BINT INTVAL,?B,?C  called as  BINT 2,40$
        let m = def(
            "BINT",
            "INTVAL,?B,?C",
            &["	CMP I,INTVAL+1", "	BPL C", "	CMP I,0-INTVAL", "	BMI C", "	JMP B", "C:"],
        );
        let mut g = 0;
        let out = m.expand(&split_args(&toks("2,40$")), &mut g).unwrap();
        assert_eq!(text(&out[4]), "JMP 40$", "B must take the supplied argument");
        assert!(
            text(&out[1]).starts_with("BPL ~"),
            "C was not supplied, so it must still be minted: {}",
            text(&out[1])
        );
    }

    #[test]
    fn generated_label_is_defined_as_well_as_referenced() {
        // INC16's body ends with `B:` — the definition site. Substituting only
        // the reference would leave the minted label undefined.
        let m = def("INC16", "ADDR,?B", &["	INC ADDR", "	BNE B", "	INC 1+ADDR", "B:"]);
        let mut g = 0;
        let out = m.expand(&split_args(&toks("CT.ROM")), &mut g).unwrap();
        let reference = text(&out[1]).replace("BNE ", "");
        match &out[3][0].tok {
            Tok::LabelDef { name, .. } => assert_eq!(
                *name, reference,
                "definition and reference must be the same minted label"
            ),
            other => panic!("expected a label definition, got {other:?}"),
        }
    }

    #[test]
    fn concatenation_builds_a_mnemonic_from_a_parameter() {
        // HLL65F: .MACRO IF COND,... / B'COND .+4  — with COND=NE gives BNE.
        let m = def("IF", "COND", &["	B'COND .+4"]);
        let mut g = 0;
        let out = m.expand(&split_args(&toks("NE")), &mut g).unwrap();
        assert_eq!(text(&out[0]), "BNE .+4");
    }

    #[test]
    fn concatenation_with_leading_parameter() {
        // HLL65F: IF'COND  and  COND'END
        let m = def("X", "COND", &["	IF'COND", "	COND'END"]);
        let mut g = 0;
        let out = m.expand(&split_args(&toks("EQ")), &mut g).unwrap();
        assert_eq!(text(&out[0]), "IFEQ");
        assert_eq!(text(&out[1]), "EQEND");
    }

    #[test]
    fn missing_arguments_bind_to_empty() {
        // .IF NB,<COND> relies on an absent argument being blank, not an error.
        let m = def("END", "ON,COND", &["	.IF NB,<COND>"]);
        let mut g = 0;
        let out = m.expand(&split_args(&toks("")), &mut g).unwrap();
        assert_eq!(text(&out[0]), ".IF NB,<>");
    }
}
