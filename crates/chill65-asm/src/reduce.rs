//! Is the hand-written control flow *reducible*?
//!
//! `codegen-readiness.md` measured 266 branches — 30.4% of the game's control
//! flow — as written by hand rather than by an HLL65F construct, and closed by
//! saying that whether those are mostly reducible "is a separate and more
//! important question for the WASM target… It should be [answered], before
//! Phase 5 is planned." This answers it.
//!
//! It matters because of the shape of the two lowerings, not their correctness.
//! `emit.rs` lowers a routine with any bare branch as a block-dispatch state
//! machine — a `loop` around a `match` on a block address — which is exactly
//! what plan §4 warns LLVM cannot optimise across. A **reducible** graph can, in
//! principle, be rebuilt into nested `loop`/`if` with no dispatch at all; an
//! irreducible one cannot, without node splitting or a dispatch for the part
//! that resists. So the fraction that is reducible is the fraction a structure
//! recovery pass could reach.
//!
//! # Measurement only
//!
//! Nothing here changes what is emitted. It reports; the decision about whether
//! to write that pass is taken with the number in hand rather than before it.
//!
//! # The graph
//!
//! Blocks are cut with **the same leader rule the state machine uses** — the
//! routine entry, every branch target inside the routine, and every instruction
//! following a control transfer — because measuring a different graph from the
//! one the emitter builds would answer a different question.
//!
//! Edges:
//!
//! - a conditional branch goes to its target and to the following instruction;
//! - a `JMP` to its target;
//! - a `JSR` to the **following** instruction: it is a call, and control comes
//!   back. (The state machine yields there, and the dispatch re-enters at the
//!   return address, which is why that address is already a leader.)
//! - `RTS`, `RTI`, `BRK`, an indirect `JMP`, and any transfer leaving the
//!   routine leave the graph.
//!
//! Dominators are computed over the blocks reachable from the entry, since that
//! is where they are defined; unreachable blocks are counted and skipped.

use std::collections::{BTreeMap, BTreeSet};

use crate::ir::{Instruction, Ir};
use crate::lexer::Mode;

/// Whether a routine's graph could be rebuilt as structured control flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Every retreating edge's target dominates its source: every loop has a
    /// single entry, and the graph is a nest of natural loops.
    Reducible,
    /// At least one loop is entered at two different blocks.
    Irreducible,
}

impl std::fmt::Display for Verdict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Verdict::Reducible => "reducible",
            Verdict::Irreducible => "IRREDUCIBLE",
        })
    }
}

/// One routine's verdict.
#[derive(Debug, Clone)]
pub struct Routine {
    pub name: String,
    pub entry: u16,
    /// Blocks reachable from the entry.
    pub blocks: usize,
    /// Blocks with a leader but no path from the entry, excluded from the
    /// dominator computation.
    pub unreachable: usize,
    /// Branch instructions written by hand — an empty macro-expansion chain.
    pub hand_branches: usize,
    /// Retreating edges found: the loops in the graph.
    pub retreating: usize,
    pub verdict: Verdict,
    /// Retreating edges whose target does not dominate their source — the ones
    /// that make the verdict irreducible.
    pub offenders: Vec<(u16, u16)>,
}

/// The measurement over a whole program.
#[derive(Debug, Clone, Default)]
pub struct Report {
    /// One entry per routine carrying at least one hand-written branch.
    pub routines: Vec<Routine>,
    /// Hand-written branches that fell outside any routine, so have no graph.
    pub hand_branches_outside: usize,
}

impl Report {
    pub fn routines_in(&self, v: Verdict) -> usize {
        self.routines.iter().filter(|r| r.verdict == v).count()
    }

    pub fn branches_in(&self, v: Verdict) -> usize {
        self.routines
            .iter()
            .filter(|r| r.verdict == v)
            .map(|r| r.hand_branches)
            .sum()
    }

    /// Hand-written branches that sit in a routine, either way.
    pub fn hand_branches(&self) -> usize {
        self.routines.iter().map(|r| r.hand_branches).sum()
    }

    /// The report as printed, hottest first.
    pub fn render(&self) -> String {
        use std::fmt::Write;
        let mut out = String::new();
        let mut rows = self.routines.clone();
        rows.sort_by_key(|r| (std::cmp::Reverse(r.hand_branches), r.entry));

        writeln!(
            out,
            "\nReducibility of the hand-written control flow ({} routines with a bare branch)",
            rows.len()
        )
        .unwrap();
        writeln!(
            out,
            "    {:<12} {:>5} {:>7} {:>6} {:>6}  {}",
            "routine", "entry", "blocks", "loops", "bare", "verdict"
        )
        .unwrap();
        for r in &rows {
            let unreach = if r.unreachable > 0 {
                format!(" (+{} unreachable)", r.unreachable)
            } else {
                String::new()
            };
            writeln!(
                out,
                "    {:<12} {:>5X} {:>7} {:>6} {:>6}  {}{}",
                r.name, r.entry, r.blocks, r.retreating, r.hand_branches, r.verdict, unreach
            )
            .unwrap();
        }

        for r in rows.iter().filter(|r| r.verdict == Verdict::Irreducible) {
            writeln!(
                out,
                "    {} — loops entered at more than one block:",
                r.name
            )
            .unwrap();
            for (from, to) in &r.offenders {
                writeln!(out, "        {from:04X} -> {to:04X}").unwrap();
            }
        }

        let total = self.hand_branches();
        let pct = |n: usize| {
            if total == 0 {
                0.0
            } else {
                100.0 * n as f64 / total as f64
            }
        };
        writeln!(
            out,
            "    aggregate: {total} hand-written branches inside routines",
        )
        .unwrap();
        for v in [Verdict::Reducible, Verdict::Irreducible] {
            writeln!(
                out,
                "      {:<12}: {:>3} routines, {:>4} branches ({:.1}%)",
                v.to_string(),
                self.routines_in(v),
                self.branches_in(v),
                pct(self.branches_in(v))
            )
            .unwrap();
        }
        if self.hand_branches_outside > 0 {
            writeln!(
                out,
                "      outside any routine: {} branches, no graph to classify",
                self.hand_branches_outside
            )
            .unwrap();
        }
        out
    }
}

/// Classify every routine carrying at least one hand-written branch.
pub fn measure(ir: &Ir) -> Report {
    let mut report = Report::default();
    for i in ir.instructions() {
        if i.is_branch() && i.chain.is_empty() && i.routine.is_none() {
            report.hand_branches_outside += 1;
        }
    }
    // `Ir::routines()` reports in emission order and dedupes only adjacent
    // names, so a routine whose instructions are emitted in two runs is named
    // twice. Classifying it twice would count its branches twice.
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for name in ir.routines() {
        if !seen.insert(name.clone()) {
            continue;
        }
        let code = routine_code(ir, &name);
        let hand = code
            .values()
            .filter(|i| i.is_branch() && i.chain.is_empty())
            .count();
        if hand == 0 {
            continue;
        }
        if let Some(r) = classify(&name, &code, hand) {
            report.routines.push(r);
        }
    }
    report
}

/// A routine's instructions, keyed by address.
fn routine_code<'a>(ir: &'a Ir, name: &str) -> BTreeMap<u16, &'a Instruction> {
    ir.instructions()
        .filter(|i| i.routine.as_deref() == Some(name))
        .map(|i| (i.addr, i))
        .collect()
}

fn classify(name: &str, code: &BTreeMap<u16, &Instruction>, hand: usize) -> Option<Routine> {
    let entry = *code.keys().next()?;
    let end = code.values().map(|i| i.addr.wrapping_add(i.size)).max()?;
    let inside = |a: u16| a >= entry && a < end;

    // The state machine's leader rule, unchanged.
    let mut leaders: BTreeSet<u16> = BTreeSet::new();
    leaders.insert(entry);
    for i in code.values() {
        let transfers =
            i.is_branch() || matches!(i.mnemonic.as_str(), "JMP" | "JSR" | "RTS" | "RTI" | "BRK");
        if !transfers {
            continue;
        }
        let next = i.addr.wrapping_add(i.size);
        if inside(next) {
            leaders.insert(next);
        }
        if let Some(t) = i.value {
            if (i.is_branch() || i.mnemonic == "JMP") && inside(t) {
                leaders.insert(t);
            }
        }
    }

    let mut succ: BTreeMap<u16, Vec<u16>> = BTreeMap::new();
    for leader in &leaders {
        let stop = leaders.iter().find(|l| *l > leader).copied().unwrap_or(end);
        let mut edges = Vec::new();
        let mut fell_through = true;
        for i in code.range(*leader..stop).map(|(_, i)| *i) {
            let next = i.addr.wrapping_add(i.size);
            if i.is_branch() {
                if let Some(t) = i.value.filter(|t| inside(*t)) {
                    edges.push(t);
                }
                if inside(next) {
                    edges.push(next);
                }
                fell_through = false;
                break;
            }
            match i.mnemonic.as_str() {
                "JMP" if i.mode != Mode::N => {
                    if let Some(t) = i.value.filter(|t| inside(*t)) {
                        edges.push(t);
                    }
                    fell_through = false;
                    break;
                }
                // A call returns to the instruction after it.
                "JSR" => {
                    if inside(next) {
                        edges.push(next);
                    }
                    fell_through = false;
                    break;
                }
                "JMP" | "RTS" | "RTI" | "BRK" => {
                    fell_through = false;
                    break;
                }
                _ => {}
            }
        }
        if fell_through && inside(stop) {
            edges.push(stop);
        }
        edges.dedup();
        succ.insert(*leader, edges);
    }

    let (order, retreating) = depth_first(entry, &succ);
    let reachable: BTreeSet<u16> = order.iter().copied().collect();
    let dom = dominators(entry, &succ, &order, &reachable);

    // Reducible iff every retreating edge is a back edge: its target dominates
    // its source, so the loop it closes has one entry.
    let offenders: Vec<(u16, u16)> = retreating
        .iter()
        .filter(|(from, to)| !dom.get(from).is_some_and(|d| d.contains(to)))
        .copied()
        .collect();

    Some(Routine {
        name: name.to_string(),
        entry,
        blocks: reachable.len(),
        unreachable: leaders.len() - reachable.len(),
        hand_branches: hand,
        retreating: retreating.len(),
        verdict: if offenders.is_empty() {
            Verdict::Reducible
        } else {
            Verdict::Irreducible
        },
        offenders,
    })
}

/// Depth-first search from `entry`. Returns the reverse postorder and every
/// retreating edge — one whose target was still on the search stack, which is
/// what a loop looks like before dominators say whether it is a natural one.
fn depth_first(entry: u16, succ: &BTreeMap<u16, Vec<u16>>) -> (Vec<u16>, Vec<(u16, u16)>) {
    #[derive(Clone, Copy, PartialEq)]
    enum Colour {
        Grey,
        Black,
    }
    let mut colour: BTreeMap<u16, Colour> = BTreeMap::new();
    let mut retreating = Vec::new();
    let mut postorder = Vec::new();
    let mut stack = vec![(entry, 0usize)];
    colour.insert(entry, Colour::Grey);

    // Iterative rather than recursive: a routine's block count is unbounded.
    while let Some((n, i)) = stack.pop() {
        let edges = succ.get(&n).map(|e| e.as_slice()).unwrap_or(&[]);
        if i < edges.len() {
            let m = edges[i];
            stack.push((n, i + 1));
            match colour.get(&m) {
                Some(Colour::Grey) => retreating.push((n, m)),
                Some(Colour::Black) => {}
                None => {
                    colour.insert(m, Colour::Grey);
                    stack.push((m, 0));
                }
            }
        } else {
            colour.insert(n, Colour::Black);
            postorder.push(n);
        }
    }

    postorder.reverse();
    (postorder, retreating)
}

/// Dominator sets over the reachable blocks, by iteration to a fixed point.
fn dominators(
    entry: u16,
    succ: &BTreeMap<u16, Vec<u16>>,
    rpo: &[u16],
    reachable: &BTreeSet<u16>,
) -> BTreeMap<u16, BTreeSet<u16>> {
    let mut preds: BTreeMap<u16, Vec<u16>> = BTreeMap::new();
    for (n, edges) in succ {
        if !reachable.contains(n) {
            continue;
        }
        for m in edges {
            preds.entry(*m).or_default().push(*n);
        }
    }

    let all: BTreeSet<u16> = reachable.clone();
    let mut dom: BTreeMap<u16, BTreeSet<u16>> = reachable
        .iter()
        .map(|n| {
            (
                *n,
                if *n == entry {
                    BTreeSet::from([entry])
                } else {
                    all.clone()
                },
            )
        })
        .collect();

    let mut changed = true;
    while changed {
        changed = false;
        for n in rpo.iter().filter(|n| **n != entry) {
            let mut new: Option<BTreeSet<u16>> = None;
            for p in preds.get(n).map(|p| p.as_slice()).unwrap_or(&[]) {
                let Some(d) = dom.get(p) else { continue };
                new = Some(match new {
                    None => d.clone(),
                    Some(acc) => acc.intersection(d).copied().collect(),
                });
            }
            let mut new = new.unwrap_or_default();
            new.insert(*n);
            if dom.get(n) != Some(&new) {
                dom.insert(*n, new);
                changed = true;
            }
        }
    }
    dom
}
