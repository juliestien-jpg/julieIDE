//! Affinage d'un bloc modifié : quels mots, quels caractères ont changé.
//!
//! Le bloc est découpé en jetons (mots, suites d'espaces, signes un par un, sauts
//! de ligne) et comparé jeton par jeton. Quand un mot est remplacé par un mot qui
//! lui ressemble (au moins la moitié des caractères en commun), on descend au
//! caractère ; sinon le mot entier est marqué.

use std::borrow::Cow;
use std::ops::Range;

use julie_core::CancelToken;

use crate::diff::{Interner, Whitespace};
use crate::myers;
use crate::text::TextLines;

/// Au-delà, on n'affine pas (le bloc est montré modifié en entier).
const MAX_LINES: usize = 400;
const MAX_BYTES: usize = 100_000;

/// Portion modifiée d'une ligne.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineSpan {
    /// Numéro de ligne dans le texte de ce côté (pas relatif au bloc).
    pub line: usize,
    /// Plage d'**octets** dans le contenu de la ligne (toujours sur des
    /// frontières de caractères).
    pub range: Range<usize>,
}

/// Affinage d'un bloc modifié : portions supprimées à gauche, ajoutées à droite,
/// dans l'ordre du texte. Une ligne sans portion est inchangée à l'intérieur
/// du bloc.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InlineDiff {
    pub left: Vec<InlineSpan>,
    pub right: Vec<InlineSpan>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TokenKind {
    Word,
    Blank,
    Other,
    Newline,
}

#[derive(Debug, Clone)]
struct Token {
    line: usize,
    range: Range<usize>,
    kind: TokenKind,
}

fn classify(c: char) -> TokenKind {
    if c.is_alphanumeric() || c == '_' {
        TokenKind::Word
    } else if c == ' ' || c == '\t' {
        TokenKind::Blank
    } else {
        TokenKind::Other
    }
}

fn tokenize(lines: &TextLines<'_>, range: Range<usize>, skip_blanks: bool) -> Vec<Token> {
    let mut tokens = Vec::new();
    for line in range.clone() {
        let content = lines.content(line);
        let mut chars = content.char_indices().peekable();
        while let Some((start, c)) = chars.next() {
            let kind = classify(c);
            let mut end = start + c.len_utf8();
            if kind != TokenKind::Other {
                while let Some(&(next, c2)) = chars.peek() {
                    if classify(c2) != kind {
                        break;
                    }
                    end = next + c2.len_utf8();
                    chars.next();
                }
            }
            if kind == TokenKind::Blank && skip_blanks {
                continue;
            }
            tokens.push(Token {
                line,
                range: start..end,
                kind,
            });
        }
        if line + 1 < range.end {
            tokens.push(Token {
                line,
                range: content.len()..content.len(),
                kind: TokenKind::Newline,
            });
        }
    }
    tokens
}

fn token_ids<'a>(
    lines: &TextLines<'a>,
    tokens: &[Token],
    blanks_equal: bool,
    interner: &mut Interner<'a>,
) -> Vec<u32> {
    tokens
        .iter()
        .map(|token| match token.kind {
            TokenKind::Newline => interner.intern(Cow::Borrowed(""), 3),
            TokenKind::Blank if blanks_equal => interner.intern(Cow::Borrowed(""), 2),
            _ => {
                let text = &lines.content(token.line)[token.range.clone()];
                interner.intern(Cow::Borrowed(text), 1)
            }
        })
        .collect()
}

fn push_span(spans: &mut Vec<InlineSpan>, line: usize, range: Range<usize>) {
    if range.is_empty() {
        return;
    }
    if let Some(last) = spans.last_mut() {
        if last.line == line && last.range.end == range.start {
            last.range.end = range.end;
            return;
        }
    }
    spans.push(InlineSpan { line, range });
}

fn push_tokens(spans: &mut Vec<InlineSpan>, tokens: &[Token]) {
    for token in tokens {
        push_span(spans, token.line, token.range.clone());
    }
}

/// Positions d'octets de chaque caractère, plus la longueur totale.
fn char_offsets(text: &str) -> Vec<usize> {
    let mut offsets: Vec<usize> = text.char_indices().map(|(i, _)| i).collect();
    offsets.push(text.len());
    offsets
}

pub(crate) fn refine(
    left: &TextLines<'_>,
    left_range: Range<usize>,
    right: &TextLines<'_>,
    right_range: Range<usize>,
    whitespace: Whitespace,
    cancel: &CancelToken,
) -> julie_core::Result<Option<InlineDiff>> {
    let bytes = |lines: &TextLines<'_>, range: &Range<usize>| -> usize {
        range.clone().map(|line| lines.content(line).len()).sum()
    };
    if left_range.len() > MAX_LINES
        || right_range.len() > MAX_LINES
        || bytes(left, &left_range) > MAX_BYTES
        || bytes(right, &right_range) > MAX_BYTES
    {
        return Ok(None);
    }

    let skip_blanks = whitespace == Whitespace::IgnoreAll;
    let blanks_equal = whitespace == Whitespace::IgnoreAmount;
    let left_tokens = tokenize(left, left_range, skip_blanks);
    let right_tokens = tokenize(right, right_range, skip_blanks);
    let mut interner = Interner::new();
    let left_ids = token_ids(left, &left_tokens, blanks_equal, &mut interner);
    let right_ids = token_ids(right, &right_tokens, blanks_equal, &mut interner);
    let changes = myers::diff_ids(&left_ids, &right_ids, cancel)?;

    let mut out = InlineDiff::default();
    for change in changes {
        let removed = &left_tokens[change.a.clone()];
        let added = &right_tokens[change.b.clone()];
        if let ([old], [new]) = (removed, added) {
            if old.kind == TokenKind::Word && new.kind == TokenKind::Word {
                let old_text = &left.content(old.line)[old.range.clone()];
                let new_text = &right.content(new.line)[new.range.clone()];
                if refine_word(old, old_text, new, new_text, &mut out, cancel)? {
                    continue;
                }
            }
        }
        push_tokens(&mut out.left, removed);
        push_tokens(&mut out.right, added);
    }
    Ok(Some(out))
}

/// Descend au caractère entre deux mots. Rend `false` (sans rien écrire) si les
/// mots se ressemblent trop peu.
fn refine_word(
    old: &Token,
    old_text: &str,
    new: &Token,
    new_text: &str,
    out: &mut InlineDiff,
    cancel: &CancelToken,
) -> julie_core::Result<bool> {
    let old_chars: Vec<u32> = old_text.chars().map(|c| c as u32).collect();
    let new_chars: Vec<u32> = new_text.chars().map(|c| c as u32).collect();
    let changes = myers::diff_ids(&old_chars, &new_chars, cancel)?;
    let removed: usize = changes.iter().map(|c| c.a.len()).sum();
    let common = old_chars.len() - removed;
    if common * 2 < old_chars.len().max(new_chars.len()) {
        return Ok(false);
    }
    let old_offsets = char_offsets(old_text);
    let new_offsets = char_offsets(new_text);
    for change in changes {
        push_span(
            &mut out.left,
            old.line,
            old.range.start + old_offsets[change.a.start]
                ..old.range.start + old_offsets[change.a.end],
        );
        push_span(
            &mut out.right,
            new.line,
            new.range.start + new_offsets[change.b.start]
                ..new.range.start + new_offsets[change.b.end],
        );
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spans(left: &str, right: &str) -> (Vec<String>, Vec<String>) {
        spans_with(left, right, Whitespace::Exact)
    }

    fn spans_with(left: &str, right: &str, whitespace: Whitespace) -> (Vec<String>, Vec<String>) {
        let l = TextLines::new(left);
        let r = TextLines::new(right);
        let inline = refine(
            &l,
            0..l.len(),
            &r,
            0..r.len(),
            whitespace,
            &CancelToken::new(),
        )
        .unwrap()
        .unwrap();
        let show = |lines: &TextLines<'_>, spans: &[InlineSpan]| {
            spans
                .iter()
                .map(|s| format!("{}:{}", s.line, &lines.content(s.line)[s.range.clone()]))
                .collect::<Vec<_>>()
        };
        (show(&l, &inline.left), show(&r, &inline.right))
    }

    #[test]
    fn a_replaced_word_is_marked_whole() {
        let (left, right) = spans("return foo(a, b);\n", "return bar(a, b);\n");
        assert_eq!(left, vec!["0:foo"]);
        assert_eq!(right, vec!["0:bar"]);
    }

    #[test]
    fn similar_words_go_down_to_characters() {
        let (left, right) = spans("$userName = 1;\n", "$userNames = 1;\n");
        assert!(left.is_empty());
        assert_eq!(right, vec!["0:s"]);
        let (left, right) = spans("getUserById\n", "getUsersById\n");
        assert!(left.is_empty());
        assert_eq!(right, vec!["0:s"]);
    }

    #[test]
    fn insertion_of_words_and_lines() {
        let (left, right) = spans("a b\n", "a new b\nc\n");
        assert!(left.is_empty());
        assert_eq!(right, vec!["0:new ", "1:c"]);
    }

    #[test]
    fn multibyte_characters_keep_valid_boundaries() {
        let (left, right) = spans("détecté à midi\n", "détectée à minuit\n");
        assert!(left.is_empty() || left.iter().all(|s| !s.is_empty()));
        assert!(right.contains(&"0:e".to_string()));
        let (left, right) = spans("prix : 10 €\n", "prix : 12 ¥\n");
        assert_eq!(left, vec!["0:0", "0:€"]);
        assert_eq!(right, vec!["0:2", "0:¥"]);
    }

    #[test]
    fn blanks_follow_the_whitespace_option() {
        let (left, right) = spans("a  b c\n", "a b d\n");
        assert_eq!(left, vec!["0:  ", "0:c"]);
        assert_eq!(right, vec!["0: ", "0:d"]);
        let (left, right) = spans_with("a  b c\n", "a b d\n", Whitespace::IgnoreAmount);
        assert_eq!(left, vec!["0:c"]);
        assert_eq!(right, vec!["0:d"]);
        let (left, right) = spans_with("a  b c\n", "a\tb d\n", Whitespace::IgnoreAll);
        assert_eq!(left, vec!["0:c"]);
        assert_eq!(right, vec!["0:d"]);
    }

    #[test]
    fn huge_blocks_are_not_refined() {
        let left = "x y z\n".repeat(MAX_LINES + 1);
        let right = "x y w\n".repeat(MAX_LINES + 1);
        let l = TextLines::new(&left);
        let r = TextLines::new(&right);
        let inline = refine(
            &l,
            0..l.len(),
            &r,
            0..r.len(),
            Whitespace::Exact,
            &CancelToken::new(),
        )
        .unwrap();
        assert!(inline.is_none());
    }
}
