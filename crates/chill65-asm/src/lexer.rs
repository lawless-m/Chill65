//! Source reading and tokenisation.
//!
//! Turns a raw source file into a token stream, preserving comments and source
//! positions — both are required downstream: comments become provenance in the
//! emitted Rust (plan section 3), positions make every later diagnostic useful.
//!
//! Traps this handles, all confirmed present in the corpus:
//!
//! - Files are VAX block-padded to a 512-byte multiple with NUL bytes.
//! - `.MAC` files use CRLF; the `.DAT` files have no line terminators at all
//!   and their record structure is recovered from their `.BYTE` directives.
//! - Numeric literals are radix-sensitive (`.RADIX`), and a trailing dot means
//!   decimal regardless — `15.` is fifteen even under `.RADIX 16`.
//! - Local labels are `N$` (`10$:`), and `$` is also common *inside* symbol
//!   names (`$COINA`, `$$CRDT`).
//! - Addressing modes are written as prefixes (`LDA I,FROM`), not suffixes.
//!
//! Radix is deliberately *not* lexer state. `.RADIX` is a directive, and macro
//! bodies are re-lexed in whatever radix is current at the expansion site, so a
//! number token carries its raw text and [`Tok::Number::value`] resolves it
//! against a radix supplied by the caller.

use std::fmt;
use std::sync::Arc;

/// Where a token came from. Carried by every token; provenance is a requirement,
/// not a debugging aid.
#[derive(Clone, PartialEq, Eq)]
pub struct Span {
    pub file: Arc<str>,
    /// 1-based.
    pub line: u32,
    /// 1-based, counted in bytes.
    pub col: u32,
}

impl fmt::Debug for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}:{}", self.file, self.line, self.col)
    }
}

/// The fourteen addressing modes, exactly as Atari's own opcode processor
/// enumerates them (`atari_tools/e2_tools/OPC65.MAC`, label `AMCHR`).
///
/// `X` and `Y` are the auto-sizing forms, resolved to zero-page or absolute by
/// AMA (`.ENABL AMA`). Resolution belongs to the encoder, not here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// `(addr,X)` indexed indirect
    Nx,
    /// zero page
    Z,
    /// immediate
    I,
    /// absolute
    A,
    /// `(addr),Y` indirect indexed
    Ny,
    /// zero page,X
    Zx,
    /// absolute,Y
    Ay,
    /// absolute,X
    Ax,
    /// branch / relative
    S,
    /// accumulator
    Ac,
    /// `(addr)` indirect
    N,
    /// zero page,Y
    Zy,
    /// auto-sized ,X
    X,
    /// auto-sized ,Y
    Y,
}

impl Mode {
    /// The name as written in source. Needed to undo prefix classification:
    /// `CMAC.MAC` declares `.MACRO CMPIN A,B`, so `A,` lexes as a prefix in a
    /// context where it is really a parameter name.
    pub fn name(self) -> &'static str {
        match self {
            Mode::Nx => "NX", Mode::Z => "Z", Mode::I => "I", Mode::A => "A",
            Mode::Ny => "NY", Mode::Zx => "ZX", Mode::Ay => "AY", Mode::Ax => "AX",
            Mode::S => "S", Mode::Ac => "AC", Mode::N => "N", Mode::Zy => "ZY",
            Mode::X => "X", Mode::Y => "Y",
        }
    }

    fn from_name(s: &str) -> Option<Mode> {
        Some(match s {
            "NX" => Mode::Nx,
            "Z" => Mode::Z,
            "I" => Mode::I,
            "A" => Mode::A,
            "NY" => Mode::Ny,
            "ZX" => Mode::Zx,
            "AY" => Mode::Ay,
            "AX" => Mode::Ax,
            "S" => Mode::S,
            "AC" => Mode::Ac,
            "N" => Mode::N,
            "ZY" => Mode::Zy,
            "X" => Mode::X,
            "Y" => Mode::Y,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Tok {
    /// `NAME:` or `NAME::` — the double colon marks a global.
    LabelDef { name: String, global: bool },
    /// `10$:`
    LocalLabelDef(u32),
    /// `10$` used as an operand.
    LocalLabel(u32),
    /// Any identifier, including directive names (which begin with `.`).
    Symbol(String),
    /// `=` or `==`; the latter defines a global.
    Assign { global: bool },
    /// A numeric literal. `text` is the raw digits; see [`Tok::number_value`].
    Number {
        text: String,
        /// Written with a trailing dot, so decimal whatever the current radix.
        forced_decimal: bool,
        /// Set by a `^H` / `^D` / `^B` / `^O` prefix.
        explicit_radix: Option<u32>,
    },
    /// A bare `.` — the current location counter.
    Dot,
    /// An addressing-mode prefix, e.g. the `I` in `LDA I,FROM`.
    Prefix(Mode),
    /// `; ...` to end of line. Retained, not discarded.
    Comment(String),
    /// A delimited string from `.ASCII` / `.ASCIZ`. The delimiter is whatever
    /// character follows the directive, so the text is captured by the lexer —
    /// tokenising and reassembling it would lose the spacing.
    Str(String),
    /// A single `'`. In MACRO-11 this is *both* the concatenation operator
    /// (`B'COND`, `IF'.B` in `HLL65F.MAC`) and a literal delimiter (`.PRINT`).
    /// Which one it is depends on macro context, so the decision is deferred to
    /// the macro engine rather than guessed at here.
    Quote,
    /// `^C` — ones-complement of the term that follows.
    ///
    /// The lexer has to own this one. `^C` is followed by an ordinary symbol,
    /// so leaving the caret as punctuation glues the operator letter onto the
    /// name and `^CMLED1` in `ALHARD.MAC:183` becomes a symbol called
    /// `CMLED1`.
    Complement,
    Punct(char),
    /// End of a source line. Significant: this is a line-oriented assembler.
    Eol,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    pub tok: Tok,
    pub span: Span,
    /// Whitespace immediately preceded this token.
    ///
    /// Needed because macro call sites separate arguments by *space*
    /// (`TRAI 2*EN.MAX EN.NUM`) while definitions use commas
    /// (`.MACRO TRAI FROM,TO`). Without this the two arguments of that call
    /// are indistinguishable from one expression, since the lexer otherwise
    /// discards whitespace.
    pub space_before: bool,
}

impl Token {
    /// Resolve a [`Tok::Number`] against the radix in force at its use site.
    ///
    /// An explicit `^H`-style prefix wins; then a trailing dot forces decimal;
    /// otherwise `radix` applies.
    pub fn number_value(&self, radix: u32) -> Option<u32> {
        match &self.tok {
            Tok::Number {
                text,
                forced_decimal,
                explicit_radix,
            } => {
                let r = explicit_radix.unwrap_or(if *forced_decimal { 10 } else { radix });
                // Digits are accumulated positionally and are *not* range
                // checked against the radix, which is not what
                // `from_str_radix` does. `ALVROM.MAC:1212` writes
                //
                //     VCTR -24.*15.,-6C.,0
                //
                // where the trailing dot forces decimal and `C` is not a
                // decimal digit at all. The original toolchain read it anyway,
                // as 6*10+12, and `TEMPST.LDA` carries the result: that vector
                // is stored with dx = -72 beside dy = -360, and -360 is
                // `-24.*15.` exactly. So the value is whatever the accumulation
                // produces, and only a character outside 0-9 A-F is rejected.
                let mut v: u32 = 0;
                for c in text.chars() {
                    let d = c.to_digit(16)?;
                    v = v.checked_mul(r)?.checked_add(d)?;
                }
                Some(v)
            }
            _ => None,
        }
    }
}

/// Strip VAX block padding and normalise line endings.
///
/// Files are padded to a 512-byte multiple with NUL. Only *trailing* NULs are
/// padding — an interior NUL would be corruption, so this trims from the end
/// rather than filtering globally, and cannot silently shorten real content.
///
/// Handles CRLF, CR-only and LF-only line endings, so every convention in the
/// corpus arrives as `\n`. Files with *no* terminator at all — every `.DAT` —
/// go through [`recover_records`].
pub fn normalise(raw: &[u8]) -> String {
    let end = raw
        .iter()
        .rposition(|&b| b != 0)
        .map(|i| i + 1)
        .unwrap_or(0);
    let text = String::from_utf8_lossy(&raw[..end]);

    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push('\n');
            }
            c => out.push(c),
        }
    }

    if out.trim().is_empty() || out.contains('\n') {
        return out;
    }
    recover_records(&out)
}

/// Restore record boundaries in a file that has none.
///
/// All 34 `.DAT` files in the corpus contain no line terminator whatsoever —
/// only NUL padding and one unbroken run of text. They came off a VAX, where
/// record structure is filesystem metadata rather than in-band bytes, and
/// archiving them to a flat stream discarded it.
///
/// Their only directive is `.BYTE` (4,484 occurrences across the corpus, with
/// nothing else), so each `.BYTE` starts a record. This is applied *only* to
/// files that have no terminators at all, so ordinary source is untouched.
fn recover_records(text: &str) -> String {
    const MARK: &str = ".BYTE";
    let upper = text.to_ascii_uppercase();
    let mut out = String::with_capacity(text.len() + 512);
    let mut last = 0usize;
    let mut from = 0usize;
    while let Some(rel) = upper[from..].find(MARK) {
        let at = from + rel;
        // Only break where a record actually ends: whitespace before the very
        // first `.BYTE` is not a record of its own.
        if at > 0 && !text[last..at].trim().is_empty() {
            out.push_str(&text[last..at]);
            out.push('\n');
            last = at;
        }
        from = at + MARK.len();
    }
    out.push_str(&text[last..]);
    out
}

fn is_sym_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '$' || c == '_' || c == '.'
}

fn is_sym_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '$' || c == '_' || c == '.'
}

pub struct Lexer {
    file: Arc<str>,
}

impl Lexer {
    pub fn new(file: impl Into<Arc<str>>) -> Self {
        Lexer { file: file.into() }
    }

    /// Tokenise already-normalised source.
    pub fn tokenise(&self, src: &str) -> Vec<Token> {
        let mut out = Vec::new();
        for (line_idx, line) in src.split('\n').enumerate() {
            self.tokenise_line(line, line_idx as u32 + 1, &mut out);
        }
        out
    }

    /// Read, normalise and tokenise a file in one step.
    pub fn tokenise_bytes(&self, raw: &[u8]) -> Vec<Token> {
        self.tokenise(&normalise(raw))
    }

    fn tokenise_line(&self, line: &str, lineno: u32, out: &mut Vec<Token>) {
        let b: Vec<char> = line.chars().collect();
        let mut i = 0usize;
        // Whitespace is discarded, but *whether it was there* is retained on the
        // next token: macro call sites separate arguments by space.
        let mut ws_pending = true;
        let span = |col: usize| Span {
            file: self.file.clone(),
            line: lineno,
            col: col as u32 + 1,
        };

        while i < b.len() {
            let c = b[i];

            // Form feed is a page break in these listings (`CCN.MAC:310` has one
            // on a line of its own); vertical tab likewise. Both are whitespace.
            if c == ' ' || c == '\t' || c == '\u{c}' || c == '\u{b}' {
                ws_pending = true;
                i += 1;
                continue;
            }

            let gap = std::mem::replace(&mut ws_pending, false);

            // `.ASCII /text/` — the character after the directive is the
            // delimiter, and everything up to its next occurrence is literal.
            if matches!(out.last().map(|t| &t.tok), Some(Tok::Symbol(s))
                if s.eq_ignore_ascii_case(".ASCII") || s.eq_ignore_ascii_case(".ASCIZ"))
            {
                let delim = c;
                let start = i + 1;
                let mut j = start;
                while j < b.len() && b[j] != delim {
                    j += 1;
                }
                let text: String = b[start..j].iter().collect();
                out.push(Token {
                    tok: Tok::Str(text),
                    span: span(i),
                    space_before: gap,
                });
                i = if j < b.len() { j + 1 } else { j };
                continue;
            }

            // `^/text/` — MACRO-11's delimited argument. The character after
            // the caret is the delimiter and everything up to its next
            // occurrence is literal text, **spaces included**. Space Duel
            // passes blank-padded strings this way, `ASCIN ^/  RECORDS  /`,
            // and lexing the interior as ordinary tokens threw the padding
            // away: whitespace survives a token stream only as a
            // `space_before` flag, so `  R  ` came back as ` R` and `.NCHR`
            // counted two characters instead of five.
            //
            // The delimiter must be non-alphanumeric, which is what separates
            // this from the `^H`/`^D`/`^B`/`^O` radix overrides handled below.
            // Crystal Castles contains exactly one non-radix caret and its
            // delimiter would be alphanumeric, so that corpus cannot reach
            // this branch.
            //
            // Nor may the delimiter be a blank. `ALLANG.MAC:213` writes
            // `ASCVH <^ MCMLXXX ATARI>`, where the caret is the copyright
            // glyph in Tempest's own character set and the blanks are text.
            // Read as a delimiter, the first blank swallows the caret and the
            // second closes the string, so the message loses three characters
            // and arrives as `MCMLXXXATARI`.
            if c == '^'
                && i + 1 < b.len()
                && !b[i + 1].is_ascii_alphanumeric()
                && !b[i + 1].is_ascii_whitespace()
            {
                let delim = b[i + 1];
                let start = i + 2;
                let mut j = start;
                while j < b.len() && b[j] != delim {
                    j += 1;
                }
                out.push(Token {
                    tok: Tok::Str(b[start..j].iter().collect()),
                    span: span(i),
                    space_before: gap,
                });
                i = if j < b.len() { j + 1 } else { j };
                continue;
            }

            // Comment runs to end of line and is kept whole.
            if c == ';' {
                let text: String = b[i + 1..].iter().collect();
                out.push(Token {
                    tok: Tok::Comment(text),
                    span: span(i),
                    space_before: gap,
                });
                i = b.len();
                continue;
            }

            // Local label: digits followed by '$'. Must be tried before Number,
            // or `10$` lexes as 10 then a stray '$'.
            if c.is_ascii_digit() {
                let mut j = i;
                while j < b.len() && b[j].is_ascii_digit() {
                    j += 1;
                }
                if j < b.len() && b[j] == '$' {
                    let n: u32 = b[i..j].iter().collect::<String>().parse().unwrap_or(0);
                    let start = i;
                    i = j + 1;
                    if i < b.len() && b[i] == ':' {
                        i += 1;
                        out.push(Token {
                            tok: Tok::LocalLabelDef(n),
                            span: span(start),
                            space_before: gap,
                        });
                    } else {
                        out.push(Token {
                            tok: Tok::LocalLabel(n),
                            span: span(start),
                            space_before: gap,
                        });
                    }
                    continue;
                }

                // Plain number: alphanumeric run (hex digits are letters), with
                // an optional trailing dot forcing decimal.
                let mut j = i;
                while j < b.len() && b[j].is_ascii_alphanumeric() {
                    j += 1;
                }
                let text: String = b[i..j].iter().collect();
                let mut forced = false;
                if j < b.len() && b[j] == '.' && !(j + 1 < b.len() && is_sym_char(b[j + 1])) {
                    forced = true;
                    j += 1;
                }
                out.push(Token {
                    tok: Tok::Number {
                        text,
                        forced_decimal: forced,
                        explicit_radix: None,
                    },
                    span: span(i),
                    space_before: gap,
                });
                i = j;
                continue;
            }

            // Radix override: ^H / ^D / ^B / ^O followed by digits.
            if c == '^' && i + 1 < b.len() {
                let r = match b[i + 1].to_ascii_uppercase() {
                    'H' => Some(16),
                    'D' => Some(10),
                    'B' => Some(2),
                    'O' => Some(8),
                    _ => None,
                };
                // `^C` is the ones-complement operator, not a radix, and it is
                // taken here for the reason `Tok::Complement` records.
                if b[i + 1].to_ascii_uppercase() == 'C' {
                    out.push(Token {
                        tok: Tok::Complement,
                        span: span(i),
                        space_before: gap,
                    });
                    i += 2;
                    continue;
                }
                if let Some(r) = r {
                    let mut j = i + 2;
                    while j < b.len() && b[j].is_ascii_alphanumeric() {
                        j += 1;
                    }
                    if j > i + 2 {
                        out.push(Token {
                            tok: Tok::Number {
                                text: b[i + 2..j].iter().collect(),
                                forced_decimal: false,
                                explicit_radix: Some(r),
                            },
                            span: span(i),
                            space_before: gap,
                        });
                        i = j;
                        continue;
                    }
                }
                out.push(Token {
                    tok: Tok::Punct('^'),
                    span: span(i),
                    space_before: gap,
                });
                i += 1;
                continue;
            }

            if is_sym_start(c) {
                // A lone '.' not starting an identifier is the location counter.
                if c == '.' && !(i + 1 < b.len() && is_sym_char(b[i + 1])) {
                    out.push(Token {
                        tok: Tok::Dot,
                        span: span(i),
                        space_before: gap,
                    });
                    i += 1;
                    continue;
                }

                let mut j = i;
                while j < b.len() && is_sym_char(b[j]) {
                    j += 1;
                }
                let name: String = b[i..j].iter().collect();
                let start = i;
                i = j;

                // Label definition: NAME: or NAME::
                if i < b.len() && b[i] == ':' {
                    let mut global = i + 1 < b.len() && b[i + 1] == ':';
                    i += if global { 2 } else { 1 };
                    // The second colon may be separated from the first.
                    // `ASTRD2.MAC:5011` reads `PARAMS:` tab `:LDA I,0` — the
                    // only line of its kind in either corpus, and plainly a
                    // mistyped `PARAMS::`. Left alone, the stray colon opened
                    // the operator field and the `LDA I,0` was never assembled.
                    //
                    // The byte evidence cannot say whether the original also
                    // made `PARAMS` global: it is referenced only from within
                    // its own unit, so both readings emit the same image. This
                    // one accounts for the character rather than discarding it,
                    // and is the reading that would be right if the symbol ever
                    // were referenced from elsewhere.
                    if !global {
                        let mut k = i;
                        while k < b.len() && (b[k] == ' ' || b[k] == '\t') {
                            k += 1;
                        }
                        if k < b.len() && b[k] == ':' {
                            global = true;
                            i = k + 1;
                        }
                    }
                    out.push(Token {
                        tok: Tok::LabelDef { name, global },
                        span: span(start),
                        space_before: gap,
                    });
                    continue;
                }

                // Addressing-mode prefix: one of the fourteen names in OPC65.MAC,
                // immediately followed by a comma. No symbol in the corpus is
                // named after a mode, so this is unambiguous here — verified
                // across all three source trees before relying on it.
                if i < b.len() && b[i] == ',' {
                    if let Some(m) = Mode::from_name(&name.to_ascii_uppercase()) {
                        out.push(Token {
                            tok: Tok::Prefix(m),
                            span: span(start),
                            space_before: gap,
                        });
                        i += 1; // consume the comma; it is part of the prefix
                        continue;
                    }
                }

                out.push(Token {
                    tok: Tok::Symbol(name),
                    span: span(start),
                    space_before: gap,
                });
                continue;
            }

            if c == '\'' {
                out.push(Token {
                    tok: Tok::Quote,
                    span: span(i),
                    space_before: gap,
                });
                i += 1;
                continue;
            }

            if c == '=' {
                let global = i + 1 < b.len() && b[i + 1] == '=';
                let start = i;
                i += if global { 2 } else { 1 };
                out.push(Token {
                    tok: Tok::Assign { global },
                    span: span(start),
                    space_before: gap,
                });
                continue;
            }

            out.push(Token {
                tok: Tok::Punct(c),
                span: span(i),
                space_before: gap,
            });
            i += 1;
        }

        out.push(Token {
            tok: Tok::Eol,
            span: span(b.len()),
            space_before: ws_pending,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lex(s: &str) -> Vec<Tok> {
        Lexer::new("test")
            .tokenise(s)
            .into_iter()
            .map(|t| t.tok)
            .collect()
    }

    fn lex_bytes(b: &[u8]) -> Vec<Tok> {
        Lexer::new("test")
            .tokenise_bytes(b)
            .into_iter()
            .map(|t| t.tok)
            .collect()
    }

    const NUL_PADDED: &[u8] = include_bytes!("../tests/fixtures/nul_padded.mac");
    const CRLF: &[u8] = include_bytes!("../tests/fixtures/crlf.mac");
    const CR_ONLY: &[u8] = include_bytes!("../tests/fixtures/cr_only.dat");

    #[test]
    fn strips_nul_padding_without_truncating_content() {
        // Fixture is a 512-byte block; real content is shorter.
        assert_eq!(NUL_PADDED.len(), 512);
        let text = normalise(NUL_PADDED);
        assert!(!text.contains('\0'), "NUL padding survived");
        assert!(
            text.trim_end().ends_with("RTS"),
            "content truncated: tail was {:?}",
            text.trim_end().chars().rev().take(20).collect::<String>()
        );
        // The last real token must still be there.
        let toks = lex_bytes(NUL_PADDED);
        assert!(toks.contains(&Tok::Symbol("RTS".into())));
    }

    #[test]
    fn crlf_and_cr_only_tokenise_identically() {
        assert_ne!(CRLF, CR_ONLY, "fixtures must differ in their bytes");
        assert_eq!(lex_bytes(CRLF), lex_bytes(CR_ONLY));
    }

    #[test]
    fn trailing_dot_forces_decimal_regardless_of_radix() {
        let toks = Lexer::new("t").tokenise("15. 15");
        let nums: Vec<_> = toks
            .iter()
            .filter(|t| matches!(t.tok, Tok::Number { .. }))
            .collect();
        assert_eq!(nums.len(), 2);
        // Under .RADIX 16: `15.` is fifteen, bare `15` is twenty-one.
        assert_eq!(nums[0].number_value(16), Some(15));
        assert_eq!(nums[1].number_value(16), Some(21));
        // Under .RADIX 10 both are fifteen.
        assert_eq!(nums[0].number_value(10), Some(15));
        assert_eq!(nums[1].number_value(10), Some(15));
    }

    #[test]
    fn local_label_is_not_number_then_dollar() {
        assert_eq!(
            lex("10$:"),
            vec![Tok::LocalLabelDef(10), Tok::Eol],
            "10$: must be one token, not 10 followed by $"
        );
        assert_eq!(lex("BNE 10$"), vec![
            Tok::Symbol("BNE".into()),
            Tok::LocalLabel(10),
            Tok::Eol
        ]);
    }

    #[test]
    fn addressing_prefix_is_distinguished() {
        // Three tokens before Eol, with the prefix its own kind.
        assert_eq!(
            lex("LDA I,FROM"),
            vec![
                Tok::Symbol("LDA".into()),
                Tok::Prefix(Mode::I),
                Tok::Symbol("FROM".into()),
                Tok::Eol
            ]
        );
        // The modes actually used across the corpus.
        assert_eq!(lex("LDA X,VITAB")[1], Tok::Prefix(Mode::X));
        assert_eq!(lex("STA NY,PKPTR")[1], Tok::Prefix(Mode::Ny));
        assert_eq!(lex("INC AY,$BCCNT")[1], Tok::Prefix(Mode::Ay));
        assert_eq!(lex("LDA Z,$CMODE")[1], Tok::Prefix(Mode::Z));
    }

    #[test]
    fn comment_text_survives() {
        let toks = lex("	STX EN.EWM	;  end of wave mode init");
        let c = toks
            .iter()
            .find_map(|t| match t {
                Tok::Comment(s) => Some(s.clone()),
                _ => None,
            })
            .expect("comment token missing");
        assert_eq!(c, "  end of wave mode init");
    }

    #[test]
    fn dollar_inside_symbol_names() {
        assert_eq!(lex("$COINA")[0], Tok::Symbol("$COINA".into()));
        assert_eq!(lex("$$CRDT:")[0], Tok::LabelDef {
            name: "$$CRDT".into(),
            global: false
        });
    }

    #[test]
    fn bare_dot_is_location_counter_but_directives_are_symbols() {
        assert_eq!(lex(".BYTE 0")[0], Tok::Symbol(".BYTE".into()));
        assert_eq!(lex("	.= 0CCE0"), vec![
            Tok::Dot,
            Tok::Assign { global: false },
            Tok::Number {
                text: "0CCE0".into(),
                forced_decimal: false,
                explicit_radix: None
            },
            Tok::Eol
        ]);
        // .=.-1 — the byte-punning idiom from CRP.MAC's LDAL/LDAH macros.
        assert_eq!(lex("	.=.-1"), vec![
            Tok::Dot,
            Tok::Assign { global: false },
            Tok::Dot,
            Tok::Punct('-'),
            Tok::Number {
                text: "1".into(),
                forced_decimal: false,
                explicit_radix: None
            },
            Tok::Eol
        ]);
    }

    #[test]
    fn the_second_colon_of_a_global_label_may_be_separated() {
        // ASTRD2.MAC:5011 reads `PARAMS:` tab `:LDA I,0`. Treating the second
        // colon as the start of the operator field lost the instruction.
        let toks = lex("PARAMS:	:LDA I,0");
        assert_eq!(
            toks[0],
            Tok::LabelDef {
                name: "PARAMS".into(),
                global: true
            }
        );
        // The control: the rest of the line must still be there, and must not
        // begin with a stray colon.
        assert_eq!(toks[1], Tok::Symbol("LDA".into()));
        // And a lone colon after a label is only the label's, not any colon
        // later on the line.
        let plain = lex("PARAMS:	LDA I,0");
        assert_eq!(
            plain[0],
            Tok::LabelDef {
                name: "PARAMS".into(),
                global: false
            }
        );
    }

    #[test]
    fn global_label_and_global_equate() {
        assert_eq!(lex("TEMP1::")[0], Tok::LabelDef {
            name: "TEMP1".into(),
            global: true
        });
        // PKI1 == POKEY+15. from CRP.MAC
        let toks = lex("PKI1	==	POKEY+15.");
        assert_eq!(toks[1], Tok::Assign { global: true });
        assert!(matches!(
            toks[4],
            Tok::Number {
                forced_decimal: true,
                ..
            }
        ));
    }

    #[test]
    fn radix_override_prefix() {
        // FROM&^H0FF from M6502.MAC's TR16AI
        let toks = Lexer::new("t").tokenise("FROM&^H0FF");
        assert_eq!(toks[0].tok, Tok::Symbol("FROM".into()));
        assert_eq!(toks[1].tok, Tok::Punct('&'));
        assert_eq!(toks[2].number_value(10), Some(0xFF));
    }

    /// Smoke-lex every source file in a real corpus. Ignored by default because
    /// the corpus is third-party and not in this repository; run with
    ///
    /// ```text
    /// CHILL65_CORPUS=/path/to/crystal-castles cargo test -p chill65-asm -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "needs CHILL65_CORPUS pointing at a local source tree"]
    fn smoke_lex_entire_corpus() {
        let Ok(dir) = std::env::var("CHILL65_CORPUS") else {
            eprintln!("CHILL65_CORPUS unset — skipping");
            return;
        };
        let mut files = 0usize;
        let mut tokens = 0usize;
        let mut walk = vec![std::path::PathBuf::from(dir)];
        while let Some(p) = walk.pop() {
            let Ok(rd) = std::fs::read_dir(&p) else {
                continue;
            };
            for e in rd.flatten() {
                let path = e.path();
                if path.is_dir() {
                    if path.file_name().is_some_and(|n| n == ".git") {
                        continue;
                    }
                    walk.push(path);
                    continue;
                }
                let ext = path
                    .extension()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_ascii_uppercase();
                if ext != "MAC" && ext != "DAT" {
                    continue;
                }
                let raw = std::fs::read(&path).expect("read");
                let toks = Lexer::new(path.to_string_lossy().to_string()).tokenise_bytes(&raw);
                assert!(
                    !toks.is_empty(),
                    "{} produced no tokens",
                    path.display()
                );
                files += 1;
                tokens += toks.len();
            }
        }
        eprintln!("lexed {files} files, {tokens} tokens");
        assert!(files > 50, "expected a real corpus, saw {files} files");
    }

    #[test]
    fn recovers_records_in_a_file_with_no_line_terminators() {
        // A .DAT file: no CR, no LF, many .BYTE statements run together.
        let raw = b" .BYTE 0,  4,  4 .BYTE   80, 80 .BYTE 0";
        let text = normalise(raw);
        assert_eq!(text.lines().count(), 3, "got {text:?}");
        let toks = lex_bytes(raw);
        assert_eq!(
            toks.iter().filter(|t| **t == Tok::Symbol(".BYTE".into())).count(),
            3
        );
        // Ordinary source, which has terminators, must be left alone.
        let normal = normalise(b"\t.BYTE 1\n\t.BYTE 2\n");
        assert_eq!(normal.lines().count(), 2);
    }

    #[test]
    fn ascii_string_keeps_its_spacing() {
        // CGR.MAC:4 — the copyright string emitted at the very start of ROM.
        let toks = lex("	.ASCII	/  (C) 1983 ATARI ALL RIGHTS RESERVED/");
        assert_eq!(toks[0], Tok::Symbol(".ASCII".into()));
        assert_eq!(
            toks[1],
            Tok::Str("  (C) 1983 ATARI ALL RIGHTS RESERVED".into()),
            "leading spaces and internal spacing must survive verbatim"
        );
        // version-2 and version-3 add a second one.
        let toks = lex("	.ASCII  / PIRATES BEWARE/");
        assert_eq!(toks[1], Tok::Str(" PIRATES BEWARE".into()));
    }

    #[test]
    fn quote_is_tokenised_not_interpreted() {
        // B'COND from HLL65F.MAC — concatenation, resolved by the macro engine.
        assert_eq!(lex("	B'COND .+4"), vec![
            Tok::Symbol("B".into()),
            Tok::Quote,
            Tok::Symbol("COND".into()),
            Tok::Dot,
            Tok::Punct('+'),
            Tok::Number {
                text: "4".into(),
                forced_decimal: false,
                explicit_radix: None
            },
            Tok::Eol
        ]);
    }
}
