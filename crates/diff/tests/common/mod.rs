//! Outils partagés par les tests d'intégration.
#![allow(dead_code)]

use julie_core::CancelToken;
use julie_diff::{BlockKind, MergeSession};

/// Générateur pseudo-aléatoire déterministe (xorshift) : les tests de propriété
/// rejouent toujours les mêmes cas.
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    pub fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound as u64) as usize
    }

    pub fn chance(&mut self, one_in: usize) -> bool {
        self.below(one_in) == 0
    }
}

const WORDS: [&str; 12] = [
    "alpha",
    "beta",
    "gamma",
    "delta",
    "",
    "}",
    "    return;",
    "x",
    "y",
    "z",
    "  x",
    "x ",
];

/// Des lignes tirées d'un petit alphabet : beaucoup de répétitions, donc beaucoup
/// d'alignements ambigus.
pub fn random_lines(rng: &mut Rng, max_len: usize, alphabet: usize) -> Vec<String> {
    let len = rng.below(max_len + 1);
    (0..len)
        .map(|_| WORDS[rng.below(alphabet.min(WORDS.len()))].to_string())
        .collect()
}

/// Quelques modifications au hasard : suppressions, insertions, remplacements.
pub fn mutate(rng: &mut Rng, lines: &[String], alphabet: usize, max_edits: usize) -> Vec<String> {
    let mut out = lines.to_vec();
    for _ in 0..rng.below(max_edits + 1) {
        let word = WORDS[rng.below(alphabet.min(WORDS.len()))].to_string();
        match rng.below(3) {
            0 if !out.is_empty() => {
                let at = rng.below(out.len());
                out.remove(at);
            }
            1 if !out.is_empty() => {
                let at = rng.below(out.len());
                out[at] = word;
            }
            _ => {
                let at = rng.below(out.len() + 1);
                out.insert(at, word);
            }
        }
    }
    out
}

pub fn join_lf(lines: &[String]) -> String {
    lines.iter().map(|line| format!("{line}\n")).collect()
}

/// Assemble des lignes avec des fins de ligne au hasard (LF, CRLF) et, une fois
/// sur trois, sans saut de ligne final.
pub fn join_random(rng: &mut Rng, lines: &[String]) -> String {
    let style = rng.below(3);
    let mut out = String::new();
    for (index, line) in lines.iter().enumerate() {
        out.push_str(line);
        let last = index + 1 == lines.len();
        if last && rng.chance(3) {
            break;
        }
        let crlf = match style {
            0 => false,
            1 => true,
            _ => rng.chance(2),
        };
        out.push_str(if crlf { "\r\n" } else { "\n" });
    }
    out
}

pub fn open(base: &str, ours: &str, theirs: &str) -> MergeSession {
    MergeSession::new(base, ours, theirs, &CancelToken::new()).expect("pas d'annulation")
}

/// Nombre de blocs (gauche, droite, identique, conflit).
pub fn tally(session: &MergeSession) -> (usize, usize, usize, usize) {
    let mut out = (0, 0, 0, 0);
    for block in session.blocks() {
        match block.kind {
            BlockKind::Left => out.0 += 1,
            BlockKind::Right => out.1 += 1,
            BlockKind::Identical => out.2 += 1,
            BlockKind::Conflict => out.3 += 1,
        }
    }
    out
}

/// Vérifie les invariants de structure d'une session.
pub fn check_invariants(session: &MergeSession) {
    let blocks = session.blocks();
    let mut position = 0;
    for block in &blocks {
        assert!(block.result.start >= position, "blocs dans le désordre");
        assert!(block.result.start <= block.result.end);
        position = block.result.end;
        assert_eq!(session.block(block.id).as_ref(), Some(block));
    }
    assert!(position <= session.result_line_count());

    let alignment = session.alignment();
    let (mut left, mut result, mut right) = (0, 0, 0);
    for segment in &alignment {
        assert_eq!(segment.left.start, left);
        assert_eq!(segment.result.start, result);
        assert_eq!(segment.right.start, right);
        left = segment.left.end;
        result = segment.result.end;
        right = segment.right.end;
        if let Some(id) = segment.block {
            assert_eq!(blocks[id].result, segment.result);
        }
    }
    assert_eq!(left, session.ours().len());
    assert_eq!(right, session.theirs().len());
    assert_eq!(result, session.result_line_count());

    let pending = blocks.iter().filter(|b| !b.resolved).count();
    let counts = session.counts();
    assert_eq!(counts.changes + counts.conflicts, pending);
    assert_eq!(session.is_all_resolved(), pending == 0);
}
