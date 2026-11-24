//! Découpage d'un texte en lignes, sans rien perdre.
//!
//! Règles communes à tout le moteur :
//!
//! - une ligne se termine par `\n` ou `\r\n` ; un `\r` isolé n'est **pas** une fin
//!   de ligne (comme pour Git) : il reste dans le contenu de la ligne ;
//! - la dernière ligne peut ne pas avoir de fin de ligne ([`LineEnding::None`]) ;
//! - un texte vide a zéro ligne ; `"a\n"` a une ligne ; `"a\nb"` en a deux ;
//! - une marque BOM (U+FEFF) en tête est mise de côté : elle ne fait partie
//!   d'aucune ligne et [`TextLines::has_bom`] la signale.

use std::ops::Range;

/// Marque d'ordre d'octets UTF-8, vue comme caractère.
pub const BOM: char = '\u{feff}';

/// Fin d'une ligne.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LineEnding {
    /// Pas de fin de ligne : seulement possible sur la dernière ligne d'un texte.
    None,
    /// `\n`.
    Lf,
    /// `\r\n`.
    CrLf,
}

impl LineEnding {
    pub fn as_str(self) -> &'static str {
        match self {
            LineEnding::None => "",
            LineEnding::Lf => "\n",
            LineEnding::CrLf => "\r\n",
        }
    }
}

/// Style de fins de ligne d'un texte entier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LineEndingStyle {
    /// Aucune fin de ligne dans le texte (vide, ou une seule ligne non terminée).
    None,
    Lf,
    CrLf,
    /// Les deux se rencontrent.
    Mixed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LineSpan {
    /// Début du contenu, en octets dans le texte d'origine (BOM comprise dans le décompte).
    pub start: usize,
    /// Fin du contenu (fin de ligne exclue).
    pub end: usize,
    pub ending: LineEnding,
}

/// Table des lignes d'un texte, indépendante de la durée de vie du texte.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct LineTable {
    pub spans: Vec<LineSpan>,
    pub has_bom: bool,
}

impl LineTable {
    pub fn new(text: &str) -> Self {
        let has_bom = text.starts_with(BOM);
        let bytes = text.as_bytes();
        let mut start = if has_bom { BOM.len_utf8() } else { 0 };
        let mut spans = Vec::new();
        let mut i = start;
        while i < bytes.len() {
            if bytes[i] == b'\n' {
                let (end, ending) = if i > start && bytes[i - 1] == b'\r' {
                    (i - 1, LineEnding::CrLf)
                } else {
                    (i, LineEnding::Lf)
                };
                spans.push(LineSpan { start, end, ending });
                start = i + 1;
            }
            i += 1;
        }
        if start < bytes.len() {
            spans.push(LineSpan {
                start,
                end: bytes.len(),
                ending: LineEnding::None,
            });
        }
        Self { spans, has_bom }
    }

    pub fn len(&self) -> usize {
        self.spans.len()
    }

    pub fn content<'a>(&self, text: &'a str, line: usize) -> &'a str {
        let span = self.spans[line];
        &text[span.start..span.end]
    }

    pub fn ending(&self, line: usize) -> LineEnding {
        self.spans[line].ending
    }

    pub fn has_final_newline(&self) -> bool {
        self.spans
            .last()
            .is_some_and(|span| span.ending != LineEnding::None)
    }

    pub fn style(&self) -> LineEndingStyle {
        let (lf, crlf) = self.count_endings();
        match (lf > 0, crlf > 0) {
            (false, false) => LineEndingStyle::None,
            (true, false) => LineEndingStyle::Lf,
            (false, true) => LineEndingStyle::CrLf,
            (true, true) => LineEndingStyle::Mixed,
        }
    }

    fn count_endings(&self) -> (usize, usize) {
        let mut lf = 0;
        let mut crlf = 0;
        for span in &self.spans {
            match span.ending {
                LineEnding::Lf => lf += 1,
                LineEnding::CrLf => crlf += 1,
                LineEnding::None => {}
            }
        }
        (lf, crlf)
    }

    /// Fin de ligne la plus fréquente ; à égalité, celle de la première ligne
    /// terminée. `None` si le texte n'a aucune fin de ligne.
    pub fn dominant_ending(&self) -> Option<LineEnding> {
        let (lf, crlf) = self.count_endings();
        if lf == 0 && crlf == 0 {
            return None;
        }
        Some(match lf.cmp(&crlf) {
            std::cmp::Ordering::Greater => LineEnding::Lf,
            std::cmp::Ordering::Less => LineEnding::CrLf,
            std::cmp::Ordering::Equal => self
                .spans
                .iter()
                .map(|span| span.ending)
                .find(|ending| *ending != LineEnding::None)
                .unwrap_or(LineEnding::Lf),
        })
    }
}

/// Un texte vu comme une suite de lignes. Ne copie rien.
///
/// C'est le découpage de référence : les numéros de ligne de toutes les
/// structures de cette crate (blocs de diff, blocs de fusion, correspondances)
/// sont des indices dans ce découpage, à partir de zéro.
#[derive(Debug, Clone)]
pub struct TextLines<'a> {
    text: &'a str,
    table: LineTable,
}

impl<'a> TextLines<'a> {
    pub fn new(text: &'a str) -> Self {
        Self {
            text,
            table: LineTable::new(text),
        }
    }

    pub(crate) fn from_parts(text: &'a str, table: LineTable) -> Self {
        Self { text, table }
    }

    pub(crate) fn text(&self) -> &'a str {
        self.text
    }

    pub(crate) fn table(&self) -> &LineTable {
        &self.table
    }

    /// Nombre de lignes.
    pub fn len(&self) -> usize {
        self.table.len()
    }

    pub fn is_empty(&self) -> bool {
        self.table.len() == 0
    }

    /// Contenu de la ligne, sans sa fin de ligne.
    pub fn content(&self, line: usize) -> &'a str {
        self.table.content(self.text, line)
    }

    /// Fin de la ligne.
    pub fn ending(&self, line: usize) -> LineEnding {
        self.table.ending(line)
    }

    /// Plage d'octets du contenu de la ligne dans le texte d'origine.
    pub fn byte_range(&self, line: usize) -> Range<usize> {
        let span = self.table.spans[line];
        span.start..span.end
    }

    /// Itère sur les contenus des lignes.
    pub fn iter(&self) -> impl Iterator<Item = &'a str> + '_ {
        (0..self.len()).map(|line| self.content(line))
    }

    /// Le texte commençait par une marque BOM.
    pub fn has_bom(&self) -> bool {
        self.table.has_bom
    }

    /// La dernière ligne se termine par une fin de ligne.
    pub fn has_final_newline(&self) -> bool {
        self.table.has_final_newline()
    }

    /// Style de fins de ligne du texte.
    pub fn style(&self) -> LineEndingStyle {
        self.table.style()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dump(text: &str) -> Vec<(&str, LineEnding)> {
        let lines = TextLines::new(text);
        (0..lines.len())
            .map(|i| (lines.content(i), lines.ending(i)))
            .collect()
    }

    #[test]
    fn empty_text_has_no_line() {
        assert!(dump("").is_empty());
        assert!(TextLines::new("").is_empty());
        assert!(!TextLines::new("").has_final_newline());
    }

    #[test]
    fn splits_lf_crlf_and_unterminated() {
        assert_eq!(
            dump("a\nb\r\nc"),
            vec![
                ("a", LineEnding::Lf),
                ("b", LineEnding::CrLf),
                ("c", LineEnding::None)
            ]
        );
        assert_eq!(dump("\n"), vec![("", LineEnding::Lf)]);
        assert_eq!(
            dump("\r\n\n"),
            vec![("", LineEnding::CrLf), ("", LineEnding::Lf)]
        );
    }

    #[test]
    fn lone_cr_stays_in_the_line() {
        assert_eq!(dump("a\rb\n"), vec![("a\rb", LineEnding::Lf)]);
        assert_eq!(dump("a\r"), vec![("a\r", LineEnding::None)]);
        assert_eq!(dump("a\r\r\n"), vec![("a\r", LineEnding::CrLf)]);
    }

    #[test]
    fn bom_is_set_aside() {
        let lines = TextLines::new("\u{feff}a\nb\n");
        assert!(lines.has_bom());
        assert_eq!(lines.content(0), "a");
        assert_eq!(lines.byte_range(0), 3..4);
        let only_bom = TextLines::new("\u{feff}");
        assert!(only_bom.has_bom());
        assert!(only_bom.is_empty());
    }

    #[test]
    fn styles_and_dominant_ending() {
        assert_eq!(TextLines::new("a").style(), LineEndingStyle::None);
        assert_eq!(TextLines::new("a\n").style(), LineEndingStyle::Lf);
        assert_eq!(TextLines::new("a\r\nb\r\n").style(), LineEndingStyle::CrLf);
        assert_eq!(TextLines::new("a\r\nb\n").style(), LineEndingStyle::Mixed);
        assert_eq!(LineTable::new("a").dominant_ending(), None);
        assert_eq!(
            LineTable::new("a\r\nb\nc\r\n").dominant_ending(),
            Some(LineEnding::CrLf)
        );
        // Égalité : la première ligne terminée décide.
        assert_eq!(
            LineTable::new("a\nb\r\n").dominant_ending(),
            Some(LineEnding::Lf)
        );
        assert_eq!(
            LineTable::new("a\r\nb\n").dominant_ending(),
            Some(LineEnding::CrLf)
        );
    }

    #[test]
    fn very_long_line_is_one_line() {
        let long = "x".repeat(1_000_000);
        let text = format!("{long}\nfin");
        let lines = TextLines::new(&text);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines.content(0).len(), 1_000_000);
    }
}
