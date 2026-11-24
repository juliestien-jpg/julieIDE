//! Géométrie d'un éditeur affiché, et outils pour poser plusieurs éditeurs côte à
//! côte (diff à deux volets, fusion à trois) avec un défilement synchronisé.

use std::ops::Range;

use eframe::egui::{Rangef, Rect};

use super::view::{EditorOutput, EditorView};

/// Où se trouvent les lignes d'un éditeur à l'écran, à la dernière image. C'est
/// ce dont l'interface a besoin pour dessiner entre deux volets (connecteurs d'un
/// diff ou d'une fusion).
#[derive(Debug, Clone, PartialEq)]
pub struct EditorLayout {
    /// Tout l'éditeur.
    pub rect: Rect,
    /// Numéros de ligne et marques.
    pub gutter_rect: Rect,
    /// Zone du texte (sans les barres de défilement).
    pub text_rect: Rect,
    pub line_height: f32,
    pub char_width: f32,
    /// Défilement vertical, en points depuis le haut du texte.
    pub scroll_y: f32,
    /// Lignes au moins en partie visibles (fin exclue).
    pub visible_lines: Range<u32>,
    pub line_count: u32,
}

impl EditorLayout {
    /// Ordonnée écran du haut de la ligne `line` (peut être hors de la zone).
    /// `line == line_count` donne le bas de la dernière ligne : pratique pour un
    /// bloc vide en fin de fichier.
    pub fn line_top(&self, line: u32) -> f32 {
        self.text_rect.top() + line as f32 * self.line_height - self.scroll_y
    }

    /// Hauteur écran de la ligne `line`.
    pub fn line_y_range(&self, line: u32) -> Rangef {
        let top = self.line_top(line);
        Rangef::new(top, top + self.line_height)
    }

    /// Hauteur écran d'un bloc de lignes `lines` (vide : un trait entre deux lignes).
    pub fn lines_y_range(&self, lines: Range<u32>) -> Rangef {
        Rangef::new(
            self.line_top(lines.start),
            self.line_top(lines.end.max(lines.start)),
        )
    }

    /// La ligne sous l'ordonnée écran `y`, s'il y en a une.
    pub fn line_at(&self, y: f32) -> Option<u32> {
        let line = ((y - self.text_rect.top() + self.scroll_y) / self.line_height).floor();
        (line >= 0.0 && (line as u32) < self.line_count).then_some(line as u32)
    }

    /// Ligne du haut, fractionnaire (2.5 : la moitié de la ligne 2 est cachée).
    pub fn top_line(&self) -> f32 {
        self.scroll_y / self.line_height
    }
}

/// Découpe `rect` en `panes` volets séparés par des gouttières de largeur `gap`
/// (la zone où dessiner les connecteurs) : `(volets, gouttières)`.
pub fn split_panes(rect: Rect, panes: usize, gap: f32) -> (Vec<Rect>, Vec<Rect>) {
    let panes = panes.max(1);
    let gaps = panes - 1;
    let width = ((rect.width() - gap * gaps as f32) / panes as f32).max(0.0);
    let mut pane_rects = Vec::with_capacity(panes);
    let mut gap_rects = Vec::with_capacity(gaps);
    let mut x = rect.left();
    for i in 0..panes {
        pane_rects.push(Rect::from_x_y_ranges(x..=x + width, rect.y_range()));
        x += width;
        if i < gaps {
            gap_rects.push(Rect::from_x_y_ranges(x..=x + gap, rect.y_range()));
            x += gap;
        }
    }
    (pane_rects, gap_rects)
}

/// Défilement vertical synchronisé : à appeler après avoir affiché les volets.
/// Le premier volet que l'utilisatrice a fait défiler à cette image
/// (`EditorOutput::scrolled`) entraîne les autres ; `map(de, vers, ligne)` traduit
/// la ligne du haut (fractionnaire) d'un volet dans un autre — l'identité pour
/// deux versions alignées, la correspondance des blocs pour un diff.
///
/// Rend l'indice du volet meneur, s'il y en a eu un.
pub fn sync_scroll(
    views: &mut [&mut EditorView],
    outputs: &[EditorOutput],
    map: impl Fn(usize, usize, f32) -> f32,
) -> Option<usize> {
    let leader = outputs.iter().position(|o| o.scrolled)?;
    let top = outputs[leader].layout.top_line();
    for (index, view) in views.iter_mut().enumerate() {
        if index != leader {
            view.set_top_line(map(leader, index, top));
        }
    }
    Some(leader)
}
