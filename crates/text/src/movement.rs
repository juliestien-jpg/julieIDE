//! Calculs de positions : graphèmes, mots, colonnes visuelles.

use unicode_segmentation::GraphemeCursor;

use crate::buffer::Buffer;

/// Largeur d'une tabulation, en colonnes.
pub const TAB_WIDTH: u32 = 4;

/// Fenêtre de caractères examinée autour d'une position pour trouver une limite
/// de graphème (un graphème plus long est coupé, sans dommage pour le texte).
const GRAPHEME_WINDOW: usize = 32;

/// Classe d'un caractère pour les déplacements par mot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CharClass {
    Space,
    Word,
    Punctuation,
}

pub fn char_class(c: char) -> CharClass {
    if c.is_whitespace() {
        CharClass::Space
    } else if c.is_alphanumeric() || c == '_' {
        CharClass::Word
    } else {
        CharClass::Punctuation
    }
}

/// Position suivante pour la flèche droite : graphème suivant, ou début de la
/// ligne suivante (une fin de ligne `\r\n` se franchit d'un coup).
pub fn next_grapheme(buffer: &Buffer, offset: usize) -> usize {
    let line = buffer.line_of(offset);
    let line_start = buffer.line_start(line);
    let line_len = buffer.line_len(line);
    let column = offset - line_start;
    if column >= line_len {
        return if line + 1 < buffer.len_lines() {
            buffer.line_start(line + 1)
        } else {
            buffer.len_chars()
        };
    }
    let window_end = (column + GRAPHEME_WINDOW).min(line_len);
    let window_start = column.saturating_sub(GRAPHEME_WINDOW);
    let text = buffer
        .slice(line_start + window_start..line_start + window_end)
        .to_string();
    let byte = char_to_byte_in(&text, column - window_start);
    let mut cursor = GraphemeCursor::new(byte, text.len(), true);
    match cursor.next_boundary(&text, 0) {
        Ok(Some(next)) => line_start + window_start + text[..next].chars().count(),
        _ => offset + 1,
    }
}

/// Position précédente pour la flèche gauche.
pub fn prev_grapheme(buffer: &Buffer, offset: usize) -> usize {
    if offset == 0 {
        return 0;
    }
    let line = buffer.line_of(offset);
    let line_start = buffer.line_start(line);
    let column = (offset - line_start).min(buffer.line_len(line));
    if column == 0 {
        return if line == 0 {
            0
        } else {
            buffer.line_end(line - 1)
        };
    }
    let window_start = column.saturating_sub(GRAPHEME_WINDOW);
    let window_end = (column + GRAPHEME_WINDOW).min(buffer.line_len(line));
    let text = buffer
        .slice(line_start + window_start..line_start + window_end)
        .to_string();
    let byte = char_to_byte_in(&text, column - window_start);
    let mut cursor = GraphemeCursor::new(byte, text.len(), true);
    match cursor.prev_boundary(&text, 0) {
        Ok(Some(prev)) => line_start + window_start + text[..prev].chars().count(),
        _ => line_start + column - 1,
    }
}

/// Ctrl+Droite : fin du mot suivant (ou de la ponctuation), sans quitter la
/// ligne ; en fin de ligne, début de la suivante.
pub fn word_right(buffer: &Buffer, offset: usize) -> usize {
    let line = buffer.line_of(offset);
    let end = buffer.line_end(line);
    if offset >= end {
        return next_grapheme(buffer, offset);
    }
    let mut position = offset;
    while position < end && char_class(buffer.rope().char(position)) == CharClass::Space {
        position += 1;
    }
    if position < end {
        let class = char_class(buffer.rope().char(position));
        while position < end && char_class(buffer.rope().char(position)) == class {
            position += 1;
        }
    }
    position
}

/// Ctrl+Gauche : début du mot précédent ; en début de ligne, fin de la précédente.
pub fn word_left(buffer: &Buffer, offset: usize) -> usize {
    let line = buffer.line_of(offset);
    let start = buffer.line_start(line);
    let offset = offset.min(buffer.line_end(line));
    if offset <= start {
        return prev_grapheme(buffer, offset);
    }
    let mut position = offset;
    while position > start && char_class(buffer.rope().char(position - 1)) == CharClass::Space {
        position -= 1;
    }
    if position > start {
        let class = char_class(buffer.rope().char(position - 1));
        while position > start && char_class(buffer.rope().char(position - 1)) == class {
            position -= 1;
        }
    }
    position
}

/// Le mot sous `offset` (double-clic) : plage de caractères de même classe,
/// sans passer la ligne. Sur un blanc entre deux mots, le mot qui précède.
pub fn word_at(buffer: &Buffer, offset: usize) -> std::ops::Range<usize> {
    let line = buffer.line_of(offset);
    let start = buffer.line_start(line);
    let end = buffer.line_end(line);
    let offset = offset.min(end);
    let class_at = |i: usize| char_class(buffer.rope().char(i));
    let anchor = if offset < end && class_at(offset) != CharClass::Space {
        offset
    } else if offset > start && class_at(offset - 1) != CharClass::Space {
        offset - 1
    } else if offset < end {
        offset
    } else {
        return offset..offset;
    };
    let class = class_at(anchor);
    let mut a = anchor;
    while a > start && class_at(a - 1) == class {
        a -= 1;
    }
    let mut b = anchor + 1;
    while b < end && class_at(b) == class {
        b += 1;
    }
    a..b
}

/// Colonne visuelle (tabulations développées) de la colonne `column` de la ligne.
pub fn visual_column(buffer: &Buffer, line: usize, column: usize) -> u32 {
    let mut visual = 0u32;
    for c in buffer.line(line).chars().take(column) {
        visual = advance(visual, c);
    }
    visual
}

/// Colonne (en caractères) la plus proche de la colonne visuelle `target`.
pub fn column_for_visual(buffer: &Buffer, line: usize, target: u32) -> usize {
    let mut visual = 0u32;
    for (column, c) in buffer.line(line).chars().enumerate() {
        let next = advance(visual, c);
        if next > target {
            // Au plus près : avant le caractère si on n'en a pas dépassé la moitié.
            return if target - visual < (next - visual).div_ceil(2) {
                column
            } else {
                column + 1
            };
        }
        visual = next;
    }
    buffer.line_len(line)
}

/// Colonne visuelle après le caractère `c` placé en `visual`.
pub fn advance(visual: u32, c: char) -> u32 {
    if c == '\t' {
        (visual / TAB_WIDTH + 1) * TAB_WIDTH
    } else {
        visual + 1
    }
}

fn char_to_byte_in(text: &str, chars: usize) -> usize {
    text.char_indices()
        .nth(chars)
        .map_or(text.len(), |(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graphemes_and_line_endings_are_crossed_whole() {
        // « é » décomposé (e + accent combinant), un drapeau, puis \r\n.
        let buffer = Buffer::new("e\u{301}🇫🇷x\r\ny");
        assert_eq!(next_grapheme(&buffer, 0), 2);
        assert_eq!(next_grapheme(&buffer, 2), 4);
        assert_eq!(next_grapheme(&buffer, 5), 7);
        assert_eq!(prev_grapheme(&buffer, 7), 5);
        assert_eq!(prev_grapheme(&buffer, 4), 2);
        assert_eq!(prev_grapheme(&buffer, 2), 0);
    }

    #[test]
    fn words() {
        let buffer = Buffer::new("  $nom->get_x();\nsuite");
        assert_eq!(word_right(&buffer, 0), 3);
        assert_eq!(word_right(&buffer, 3), 6);
        assert_eq!(word_right(&buffer, 6), 8);
        assert_eq!(word_left(&buffer, 8), 6);
        assert_eq!(word_left(&buffer, 3), 2);
        assert_eq!(word_at(&buffer, 4), 3..6);
        assert_eq!(word_at(&buffer, 9), 8..13);
        assert_eq!(word_right(&buffer, 16), 17);
        assert_eq!(word_left(&buffer, 17), 16);
    }

    #[test]
    fn visual_columns_expand_tabs() {
        let buffer = Buffer::new("\tab\tc");
        assert_eq!(visual_column(&buffer, 0, 1), 4);
        assert_eq!(visual_column(&buffer, 0, 4), 8);
        assert_eq!(column_for_visual(&buffer, 0, 1), 0);
        assert_eq!(column_for_visual(&buffer, 0, 3), 1);
        assert_eq!(column_for_visual(&buffer, 0, 8), 4);
        assert_eq!(column_for_visual(&buffer, 0, 99), 5);
    }
}
