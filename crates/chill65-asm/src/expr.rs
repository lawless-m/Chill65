//! Expression evaluation against a symbol table and location counter.
//!
//! # Evaluation is strictly left to right, with no operator precedence
//!
//! This is MACRO-11 behaviour and it is not a detail — the corpus depends on it
//! in both directions:
//!
//! - `M6502.MAC`'s `TR16AI` extracts a high byte with `FROM&^H0FF00/^H100`.
//!   Left to right that is `(FROM & 0xFF00) / 0x100`. Under conventional
//!   precedence `/` would bind first, giving `FROM & 0xFF` — the *low* byte, and
//!   every 16-bit immediate transfer in the game would be silently wrong.
//! - `HLL65F.MAC` computes listing indentation as `..NST$+1*3+..SRC$` in
//!   `$INDEN` and `..NST$*3+..SRC$` in `$UNDEN`. Those are the same formula at
//!   two nesting levels only if the first reads `((..NST$+1)*3)+..SRC$`.
//! - The same file writes `<9.*3+..SRC$>` in angle brackets, which is only
//!   necessary *because* left-to-right would otherwise subtract first.
//!
//! Angle brackets are therefore the sole means of grouping.
//!
//! Arithmetic is 16-bit wrapping, matching the target.
//!
//! Forward references are not errors: assembly is two-pass, so an undefined
//! symbol yields [`Eval::Unresolved`] and pass one carries on.

use crate::lexer::{Tok, Token};

/// Resolves symbol names to values. Pass one supplies a partial table.
pub trait Symbols {
    fn lookup(&self, name: &str) -> Option<u16>;
}

impl Symbols for std::collections::HashMap<String, u16> {
    /// Case-insensitive, because the dialect is.
    ///
    /// `HLL65F.MAC`'s `FND` writes `...S1` when it pushes and `...s1` when it
    /// tests (`.if eq,...s1&2`) — the same symbol in two cases. A
    /// case-sensitive table reports it undefined and the whole
    /// structured-control-flow package collapses.
    fn lookup(&self, name: &str) -> Option<u16> {
        let up = name.to_ascii_uppercase();
        // Six-character RAD50 truncation, matching the assembler's interning.
        // Synthetic names (containing `~`) are exempt and already unique.
        let key: String = if up.contains('~') {
            up
        } else {
            up.chars().take(6).collect()
        };
        self.get(name).or_else(|| self.get(&key)).copied()
    }
}

impl<F: Fn(&str) -> Option<u16>> Symbols for F {
    fn lookup(&self, name: &str) -> Option<u16> {
        self(name)
    }
}

/// Everything an expression needs from its surroundings.
pub struct Context<'a> {
    /// Radix currently in force (`.RADIX`). Trailing-dot and `^H`-style
    /// literals override it per-token.
    pub radix: u32,
    /// Value of `.` — the location counter. `None` where `.` is meaningless.
    pub location: Option<u16>,
    pub symbols: &'a dyn Symbols,
}

impl<'a> Context<'a> {
    pub fn new(radix: u32, location: Option<u16>, symbols: &'a dyn Symbols) -> Self {
        Context {
            radix,
            location,
            symbols,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Eval {
    Value(u16),
    /// At least one symbol was undefined; names the first one seen. Pass one
    /// uses this to proceed rather than fail.
    Unresolved(String),
}

impl Eval {
    pub fn value(&self) -> Option<u16> {
        match self {
            Eval::Value(v) => Some(*v),
            Eval::Unresolved(_) => None,
        }
    }

    pub fn is_unresolved(&self) -> bool {
        matches!(self, Eval::Unresolved(_))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExprError {
    pub message: String,
    pub at: Option<Token>,
}

impl std::fmt::Display for ExprError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.at {
            Some(t) => write!(f, "{:?}: {}", t.span, self.message),
            None => write!(f, "{}", self.message),
        }
    }
}

fn err(message: impl Into<String>, at: Option<&Token>) -> ExprError {
    ExprError {
        message: message.into(),
        at: at.cloned(),
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Op {
    Add,
    Sub,
    Mul,
    Div,
    And,
    Or,
    Xor,
}

/// Recognise a binary operator at `t`, returning it and how many tokens it takes.
///
/// # Why there are no `.AND.` / `.OR.` / `.NOT.` word forms
///
/// Other MACRO-11 dialects spell the logical operators as words. This corpus
/// does not use them as operators at all — searched across all three source
/// trees, `.OR.` occurs exactly once, in `HLL65F.MAC:51`:
///
/// ```text
/// .IF IDN,<.A>,<.OR.>
/// ```
///
/// which is an *identity test of a macro argument against the literal token*
/// `.OR.`, not arithmetic. `.NOT.` occurs once inside an `.ERROR` message
/// (`;ARG .NOT. CND`), and `.AND.` never occurs.
///
/// They could not be lexed as operators in any case: `.` is a symbol character,
/// so `0F0.OR.0F` tokenises as a number followed by the single symbol
/// `.OR.0F`. Supporting a form that works only when surrounded by spaces would
/// silently misparse the unspaced form, so it is deliberately not supported.
/// Should a second title need them, the *lexer* must special-case them first.
///
/// The operators the corpus actually uses are `&` (34 occurrences) and `!`
/// (16, all in `CCN.MAC`'s conditionals, e.g. `.IF EQ,COIN67!COIN01`).
fn binary_op(t: &Tok) -> Option<(Op, usize)> {
    match t {
        Tok::Punct('+') => Some((Op::Add, 1)),
        Tok::Punct('-') => Some((Op::Sub, 1)),
        Tok::Punct('*') => Some((Op::Mul, 1)),
        Tok::Punct('/') => Some((Op::Div, 1)),
        Tok::Punct('&') => Some((Op::And, 1)),
        Tok::Punct('!') => Some((Op::Or, 1)),
        // `?` is exclusive-or. Used only by CG.MAC's ROM checksum-adjustment
        // equates, and confirmed against all three shipped ROM images:
        //
        //   root  CHK01 = 082?7F?01  -> 0x82^0x7F^0x01 = 0xFC = byte at A024
        //   v2    CHK01 = 73?1       -> 0x73^0x01      = 0x72 = byte at A034
        //   v3    CHK01 = 0E7        -> 0xE7           = 0xE7 = byte at A034
        //
        // (v2 and v3 sit 16 bytes further on because their CGR.MAC emits a
        // second `.ASCII` string — " PIRATES BEWARE".)
        Tok::Punct('?') => Some((Op::Xor, 1)),
        _ => None,
    }
}

fn apply(op: Op, a: u16, b: u16, at: Option<&Token>) -> Result<u16, ExprError> {
    Ok(match op {
        Op::Add => a.wrapping_add(b),
        Op::Sub => a.wrapping_sub(b),
        Op::Mul => a.wrapping_mul(b),
        Op::Div => {
            if b == 0 {
                // MACRO-11's documented behaviour for divide-by-zero is not
                // established here, so this errors rather than inventing a
                // result. If the corpus ever trips it, that is worth knowing.
                return Err(err("division by zero", at));
            }
            // Signed, truncating toward zero — not the unsigned division the
            // u16 operands invite.
            //
            // Measured from `AS2ROM.MAC`'s `VCTRSC` macro, which rounds a
            // signed displacement with `..2=DY-<..SCAL/2>/..SCAL`. MACRO-11
            // evaluates left to right with no precedence, so for
            // `VCTRSC 8,-32,0` at `..SCAL=2` that is `(-32-1)/2`. The original
            // makes it `-16`; unsigned division of `0FFDF` makes it `07FEF`,
            // which masks to `-17`.
            //
            // The consequence is not one wrong word. `-17` is odd, so the
            // macro's short-vector test `..5&0FFE1` fails and it emits the
            // two-word long form where the original emits one word. Across the
            // vector tables at `3500-3FFF` that added 86 bytes, moved `CNTSCL`
            // from `3EC2` to `3F18`, and so corrupted every `JSRL` referring to
            // it as far back as `3000`.
            (a as i16).wrapping_div(b as i16) as u16
        }
        Op::And => a & b,
        Op::Or => a | b,
        Op::Xor => a ^ b,
    })
}

/// Evaluate the longest expression at the start of `toks`.
///
/// Returns the result and how many tokens were consumed. Stops cleanly at
/// anything that cannot continue an expression — a comma, `Eol`, a comment —
/// so callers can hand over a whole operand field.
pub fn eval_prefix(toks: &[Token], ctx: &Context) -> Result<(Eval, usize), ExprError> {
    let mut i = 0usize;
    let (mut acc, used) = operand(toks, i, ctx)?;
    i += used;

    loop {
        let Some(t) = toks.get(i) else { break };
        let Some((op, oplen)) = binary_op(&t.tok) else {
            break;
        };
        let op_tok = t.clone();
        i += oplen;
        let (rhs, used) = operand(toks, i, ctx)?;
        i += used;

        acc = match (acc, rhs) {
            (Eval::Value(a), Eval::Value(b)) => Eval::Value(apply(op, a, b, Some(&op_tok))?),
            (Eval::Unresolved(n), _) => Eval::Unresolved(n),
            (_, Eval::Unresolved(n)) => Eval::Unresolved(n),
        };
    }

    Ok((acc, i))
}

/// Evaluate `toks` as a complete expression, rejecting anything left over.
pub fn eval(toks: &[Token], ctx: &Context) -> Result<Eval, ExprError> {
    let (v, used) = eval_prefix(toks, ctx)?;
    let rest = &toks[used..];
    if let Some(t) = rest
        .iter()
        .find(|t| !matches!(t.tok, Tok::Eol | Tok::Comment(_)))
    {
        return Err(err(
            format!("unexpected {:?} after expression", t.tok),
            Some(t),
        ));
    }
    Ok(v)
}

/// One operand, including any unary prefixes and bracketed subexpressions.
fn operand(toks: &[Token], mut i: usize, ctx: &Context) -> Result<(Eval, usize), ExprError> {
    let start = i;

    // Unary prefixes stack: -X, +5, and combinations thereof.
    let mut negate = false;
    loop {
        match toks.get(i).map(|t| &t.tok) {
            Some(Tok::Punct('-')) => {
                negate = !negate;
                i += 1;
            }
            Some(Tok::Punct('+')) => {
                i += 1;
            }
            _ => break,
        }
    }

    let (mut val, used) = atom(toks, i, ctx)?;
    i += used;

    if let Eval::Value(v) = val {
        val = Eval::Value(if negate { v.wrapping_neg() } else { v });
    }

    let _ = start;
    Ok((val, i - start))
}

fn atom(toks: &[Token], i: usize, ctx: &Context) -> Result<(Eval, usize), ExprError> {
    let Some(t) = toks.get(i) else {
        return Err(err("expression ended unexpectedly", toks.last()));
    };

    match &t.tok {
        Tok::Number { .. } => {
            let v = t.number_value(ctx.radix).ok_or_else(|| {
                err(
                    format!("malformed numeric literal for radix {}", ctx.radix),
                    Some(t),
                )
            })?;
            Ok((Eval::Value(v as u16), 1))
        }

        Tok::Dot => match ctx.location {
            Some(loc) => Ok((Eval::Value(loc), 1)),
            None => Err(err("'.' used where there is no location counter", Some(t))),
        },

        Tok::Symbol(name) => match ctx.symbols.lookup(name) {
            Some(v) => Ok((Eval::Value(v), 1)),
            None => Ok((Eval::Unresolved(name.clone()), 1)),
        },

        // `'X` — MACRO-11's character-value operator: the ASCII code of the
        // character that follows.
        //
        // The lexer leaves `'` as an uninterpreted `Tok::Quote` because it is
        // also the macro concatenation mark, and only the consumer knows which
        // is meant (see the module comment in `macros.rs`). Here we are in an
        // expression, so it is the operator.
        //
        // Space Duel's `ASCIN` macro walks a string with `.IRPC` and takes each
        // character's code with `...4=''...5` — the first quote is this
        // operator, the second is the concatenation mark introducing the
        // parameter, which macro expansion has already consumed by the time the
        // expression is evaluated. Crystal Castles never uses either form.
        Tok::Quote => {
            let Some(next) = toks.get(i + 1) else {
                return Err(err("'\'' with no character after it", Some(t)));
            };
            let text = crate::macros::token_text(next);
            match text.chars().next() {
                Some(c) => Ok((Eval::Value(c as u16), 2)),
                None => Err(err("'\'' applied to an empty token", Some(next))),
            }
        }

        // <expr> — the only grouping construct, and the only way to defeat
        // left-to-right evaluation.
        Tok::Punct('<') => {
            let mut depth = 1usize;
            let mut j = i + 1;
            while j < toks.len() {
                match toks[j].tok {
                    Tok::Punct('<') => depth += 1,
                    Tok::Punct('>') => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                j += 1;
            }
            if depth != 0 {
                return Err(err("unclosed '<' in expression", Some(t)));
            }
            let inner = eval(&toks[i + 1..j], ctx)?;
            Ok((inner, j - i + 1))
        }

        other => Err(err(format!("expected an operand, found {other:?}"), Some(t))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;
    use std::collections::HashMap;

    fn syms(pairs: &[(&str, u16)]) -> HashMap<String, u16> {
        pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    fn ev(src: &str, radix: u32, loc: Option<u16>, table: &HashMap<String, u16>) -> Eval {
        let toks = Lexer::new("t").tokenise(src);
        let ctx = Context::new(radix, loc, table);
        eval(&toks, &ctx).unwrap_or_else(|e| panic!("{src:?}: {e}"))
    }

    #[test]
    fn symbol_times_literal() {
        // TRAI 2*EN.MAX EN.NUM  (CEN.MAC)
        let s = syms(&[("EN.MAX", 8)]);
        assert_eq!(ev("2*EN.MAX", 16, None, &s), Eval::Value(16));
    }

    #[test]
    fn tr16ai_byte_extraction_needs_left_to_right() {
        // M6502.MAC TR16AI:  LDA I,FROM&^H0FF  /  LDA I,FROM&^H0FF00/^H100
        let s = syms(&[("FROM", 0xBEEF)]);
        assert_eq!(ev("FROM&^H0FF", 16, None, &s), Eval::Value(0xEF));

        // This once asserted 0x00BE. That was wrong about the intermediate,
        // though right about the byte that matters: `/` is signed (see `apply`),
        // so `0BE00/0100` is -16896/256 = -66 = 0FFBE, and it is the *low byte*
        // of that which reaches the immediate. Every use of the idiom across
        // both corpora — `M6502.MAC:57,166,200`, `CCT.MAC:731`,
        // `AS2TST.MAC:833` — feeds an 8-bit operand, so none of them can tell
        // 0FFBE from 0BE, and the Crystal Castles images stay exact either way.
        assert_eq!(ev("FROM&^H0FF00/^H100", 16, None, &s), Eval::Value(0xFFBE));
        assert_eq!(
            match ev("FROM&^H0FF00/^H100", 16, None, &s) {
                Eval::Value(v) => v as u8,
                other => panic!("expected a value, got {other:?}"),
            },
            0xBE,
            "the high byte is what the immediate takes"
        );
        // Guard the reasoning: with conventional precedence this would be
        // FROM & (0xFF00/0x100) == FROM & 0xFF == 0xEF, the low byte. That
        // control is the point of the test and can still fail.
        assert_ne!(ev("FROM&^H0FF00/^H100", 16, None, &s), Eval::Value(0xEF));
    }

    #[test]
    fn division_is_signed_and_truncates_toward_zero() {
        // `AS2ROM.MAC`'s `VCTRSC` rounds a signed displacement:
        // `..2=DY-<..SCAL/2>/..SCAL`. Left to right at `..SCAL=2` and `DY=-32`
        // that is `(-32-1)/2`, which the original makes -16 — truncating toward
        // zero, not flooring, and not the 07FEF unsigned division would give.
        let s = syms(&[]);
        assert_eq!(ev("0-33/2", 10, None, &s), Eval::Value((-16i16) as u16));
        // Flooring would be -17. The distinction is what decides whether the
        // macro's `..5&0FFE1` short-vector test passes, so it changes how many
        // words the vector occupies, not just their value.
        assert_ne!(ev("0-33/2", 10, None, &s), Eval::Value((-17i16) as u16));
        // Positive operands are unaffected, which is the bulk of the corpus.
        assert_eq!(ev("33/2", 10, None, &s), Eval::Value(16));
    }

    #[test]
    fn one_plus_symbol_and_symbol_plus_one_agree() {
        // M6502.MAC's header explains 1+FROM exists for macro-argument reasons.
        let s = syms(&[("EN.DEL", 0x0300)]);
        assert_eq!(ev("1+EN.DEL", 16, None, &s), Eval::Value(0x0301));
        assert_eq!(ev("EN.DEL+1", 16, None, &s), Eval::Value(0x0301));
    }

    #[test]
    fn location_counter_difference() {
        // CG.MAC:  PL.AZ = .-PL.1A   (size of the player area)
        let s = syms(&[("PL.1A", 0x8000)]);
        assert_eq!(ev(".-PL.1A", 16, Some(0x8093), &s), Eval::Value(0x93));
    }

    #[test]
    fn decimal_dot_literals_mix_with_symbols() {
        // HLL65F.MAC:  9.*3+..SRC$   with ..SRC$ = 41.
        let s = syms(&[("..SRC$", 41)]);
        // Left to right: (9*3)+41 == 68. Note 3 is radix-16 here but still 3.
        assert_eq!(ev("9.*3+..SRC$", 16, None, &s), Eval::Value(68));
    }

    #[test]
    fn hll65f_indent_formula_is_left_to_right() {
        // $INDEN:  ...S1 = ..NST$+1*3+..SRC$
        // $UNDEN:  ...S1 = ..NST$*3+..SRC$
        // The two are the same formula one nesting level apart, which only
        // holds under left-to-right evaluation.
        let s = syms(&[("..NST$", 2), ("..SRC$", 41)]);
        assert_eq!(ev("..NST$+1*3+..SRC$", 16, None, &s), Eval::Value(50)); // (2+1)*3+41
        assert_eq!(ev("..NST$*3+..SRC$", 16, None, &s), Eval::Value(47)); // 2*3+41
    }

    #[test]
    fn angle_brackets_group() {
        // HLL65F.MAC:  .IIF GT,...S1-<9.*3+..SRC$>
        let s = syms(&[("...S1", 100), ("..SRC$", 41)]);
        assert_eq!(ev("...S1-<9.*3+..SRC$>", 16, None, &s), Eval::Value(32)); // 100-68
        // Without the brackets, left-to-right gives something quite different.
        assert_eq!(ev("...S1-9.*3+..SRC$", 16, None, &s), Eval::Value(314)); // (100-9)*3+41
    }

    #[test]
    fn undefined_symbol_is_unresolved_not_an_error() {
        let s = syms(&[]);
        assert_eq!(
            ev("NOT.YET.DEFINED", 16, None, &s),
            Eval::Unresolved("NOT.YET.DEFINED".into())
        );
        // and it propagates through arithmetic rather than blowing up
        let s2 = syms(&[("KNOWN", 4)]);
        assert!(ev("KNOWN+LATER", 16, None, &s2).is_unresolved());
        assert!(ev("LATER*2", 16, None, &s2).is_unresolved());
    }

    #[test]
    fn conditional_guard_operands_evaluate() {
        // CG.MAC:  .IF GE,.-0F0  /  .ERROR ; RAM overlap with sounds
        //          .IF GE,.-0C00 /  .ERROR ; out of lower RAM
        let s = syms(&[]);
        assert_eq!(ev(".-0F0", 16, Some(0x00E0), &s), Eval::Value(0xFFF0)); // negative, wraps
        assert_eq!(ev(".-0F0", 16, Some(0x0100), &s), Eval::Value(0x0010));
        assert_eq!(ev(".-0C00", 16, Some(0x0B00), &s), Eval::Value(0xFF00));
    }

    #[test]
    fn radix_governs_bare_literals() {
        let s = syms(&[]);
        assert_eq!(ev("10", 16, None, &s), Eval::Value(16));
        assert_eq!(ev("10", 10, None, &s), Eval::Value(10));
        assert_eq!(ev("10", 2, None, &s), Eval::Value(2));
        // C99.MAC switches to .RADIX 10 around the castle data includes.
        assert_eq!(ev("196", 10, None, &s), Eval::Value(196));
    }

    #[test]
    fn unary_minus_and_wrapping() {
        let s = syms(&[("X", 1)]);
        assert_eq!(ev("-X", 16, None, &s), Eval::Value(0xFFFF));
        assert_eq!(ev("0-1", 16, None, &s), Eval::Value(0xFFFF));
        // 0-INTVAL from CMAC.MAC's BINT macro
        let s2 = syms(&[("INTVAL", 2)]);
        assert_eq!(ev("0-INTVAL", 16, None, &s2), Eval::Value(0xFFFE));
    }

    #[test]
    fn logical_operators() {
        let s = syms(&[]);
        assert_eq!(ev("0F0&0FF", 16, None, &s), Eval::Value(0xF0));
        assert_eq!(ev("0F0!0F", 16, None, &s), Eval::Value(0xFF));
        // CCN.MAC:  .IF EQ,COIN67!COIN01   and   .IF EQ,<MULTS-1>!<CCTRS-1>
        let s2 = syms(&[("COIN67", 0), ("COIN01", 1), ("MULTS", 1), ("CCTRS", 1)]);
        assert_eq!(ev("COIN67!COIN01", 16, None, &s2), Eval::Value(1));
        assert_eq!(ev("<MULTS-1>!<CCTRS-1>", 16, None, &s2), Eval::Value(0));
    }

    #[test]
    fn question_mark_is_exclusive_or() {
        // CG.MAC's ROM checksum-adjustment equates, verified against the bytes
        // actually burned into all three revisions.
        let s = syms(&[]);
        assert_eq!(ev("082?7F?01", 16, None, &s), Eval::Value(0xFC)); // root
        assert_eq!(ev("73?1", 16, None, &s), Eval::Value(0x72)); // version-2
        assert_eq!(ev("0E7", 16, None, &s), Eval::Value(0xE7)); // version-3
    }

    #[test]
    fn word_form_operators_are_not_operators_in_this_dialect() {
        // `.OR.` appears once in the whole corpus, at HLL65F.MAC:51, as a macro
        // argument compared by identity: `.IF IDN,<.A>,<.OR.>`. It is not
        // arithmetic, and since `.` is a symbol character it cannot lex as an
        // operator anyway. Pin that so the behaviour is deliberate, not a gap.
        let toks = Lexer::new("t").tokenise("0F0.OR.0F");
        assert_eq!(toks.len(), 3, "number, symbol, Eol");
        assert_eq!(toks[1].tok, Tok::Symbol(".OR.0F".into()));

        let s = syms(&[]);
        let ctx = Context::new(16, None, &s);
        assert!(
            eval(&toks, &ctx).is_err(),
            "must fail loudly rather than silently mis-evaluate"
        );
    }

    #[test]
    fn division_by_zero_is_an_error_not_a_silent_zero() {
        let s = syms(&[]);
        let toks = Lexer::new("t").tokenise("10/0");
        let ctx = Context::new(16, None, &s);
        assert!(eval(&toks, &ctx).is_err());
    }

    #[test]
    fn eval_prefix_stops_at_a_comma() {
        // .BYTE 0, 4, 80 — the caller splits operands; we must not swallow them.
        let s = syms(&[]);
        let toks = Lexer::new("t").tokenise("2+3, 4");
        let ctx = Context::new(16, None, &s);
        let (v, used) = eval_prefix(&toks, &ctx).unwrap();
        assert_eq!(v, Eval::Value(5));
        assert_eq!(toks[used].tok, Tok::Punct(','));
    }
}
