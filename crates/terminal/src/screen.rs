//! Instantané de l'écran d'une session : ce que l'interface dessine.
//!
//! Les couleurs restent symboliques (`Color::Indexed`, `Foreground`…) : la
//! palette est celle du thème de l'interface. Une couleur redéfinie par le
//! programme (OSC 4/10/11) arrive déjà résolue en `Color::Rgb`.

use std::sync::Arc;

/// Une couleur de cellule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Color {
    /// Couleur du texte par défaut du thème.
    Foreground,
    /// Fond par défaut du thème.
    Background,
    /// Palette à 256 entrées : 0–15 couleurs ANSI (thème), 16–231 cube,
    /// 232–255 gris (voir [`xterm_rgb`]).
    Indexed(u8),
    /// Vraie couleur (24 bits).
    Rgb(u8, u8, u8),
}

/// Attributs d'une cellule (combinables).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CellFlags(u16);

impl CellFlags {
    pub const BOLD: CellFlags = CellFlags(1);
    pub const ITALIC: CellFlags = CellFlags(1 << 1);
    pub const UNDERLINE: CellFlags = CellFlags(1 << 2);
    pub const INVERSE: CellFlags = CellFlags(1 << 3);
    pub const DIM: CellFlags = CellFlags(1 << 4);
    pub const STRIKEOUT: CellFlags = CellFlags(1 << 5);
    pub const HIDDEN: CellFlags = CellFlags(1 << 6);
    /// Caractère large (CJK, emoji) : occupe aussi la cellule suivante.
    pub const WIDE: CellFlags = CellFlags(1 << 7);
    /// Seconde moitié d'un caractère large : ne rien y dessiner.
    pub const WIDE_SPACER: CellFlags = CellFlags(1 << 8);
    /// Dans la sélection courante.
    pub const SELECTED: CellFlags = CellFlags(1 << 9);

    pub const fn empty() -> Self {
        CellFlags(0)
    }

    pub const fn contains(self, other: CellFlags) -> bool {
        self.0 & other.0 == other.0
    }

    pub fn insert(&mut self, other: CellFlags) {
        self.0 |= other.0;
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }
}

impl std::ops::BitOr for CellFlags {
    type Output = CellFlags;
    fn bitor(self, rhs: CellFlags) -> CellFlags {
        CellFlags(self.0 | rhs.0)
    }
}

/// Une cellule de l'écran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenCell {
    pub ch: char,
    /// Caractères combinants (accents décomposés…) à dessiner sur `ch`.
    pub zerowidth: Option<Arc<[char]>>,
    pub fg: Color,
    pub bg: Color,
    pub flags: CellFlags,
}

impl Default for ScreenCell {
    fn default() -> Self {
        Self {
            ch: ' ',
            zerowidth: None,
            fg: Color::Foreground,
            bg: Color::Background,
            flags: CellFlags::empty(),
        }
    }
}

/// Forme du curseur demandée par le programme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CursorShape {
    Block,
    Underline,
    Beam,
    HollowBlock,
}

/// Position du curseur dans la partie affichée (ligne 0 = haut de l'écran visible).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenCursor {
    pub line: usize,
    pub column: usize,
    pub shape: CursorShape,
}

/// Modes demandés par le programme, utiles à l'interface et au codage des touches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Modes {
    /// Flèches en mode application (`ESC O A` au lieu de `ESC [ A`).
    pub app_cursor: bool,
    pub app_keypad: bool,
    /// Collage encadré demandé (`ESC [ ? 2004 h`).
    pub bracketed_paste: bool,
    /// Écran alternatif (vim, less) : pas d'historique.
    pub alt_screen: bool,
    /// Dans l'écran alternatif, la molette envoie des flèches.
    pub alternate_scroll: bool,
    /// Le programme suit la souris (non géré en v0 : la souris reste à la sélection).
    pub mouse: bool,
    pub show_cursor: bool,
}

/// Ce qui est visible d'une session, copié sous verrou puis dessiné sans lui.
#[derive(Debug, Clone, Default)]
pub struct Screen {
    pub columns: usize,
    pub lines: usize,
    /// `lines × columns` cellules, ligne par ligne.
    pub cells: Vec<ScreenCell>,
    /// `None` : curseur caché, ou hors de la partie affichée (historique).
    pub cursor: Option<ScreenCursor>,
    /// Lignes d'historique au-dessus de l'écran affiché (0 = tout en bas).
    pub display_offset: usize,
    /// Lignes d'historique disponibles.
    pub history_size: usize,
    pub modes: Modes,
    /// Une sélection existe (même hors de la partie affichée).
    pub has_selection: bool,
}

impl Screen {
    /// Cellules de la ligne affichée `line` (vide hors bornes).
    pub fn row(&self, line: usize) -> &[ScreenCell] {
        if line >= self.lines {
            return &[];
        }
        &self.cells[line * self.columns..(line + 1) * self.columns]
    }

    /// Texte de la ligne affichée, sans les espaces de fin.
    pub fn line_text(&self, line: usize) -> String {
        let mut text = String::new();
        for cell in self.row(line) {
            if cell.flags.contains(CellFlags::WIDE_SPACER) {
                continue;
            }
            text.push(cell.ch);
            if let Some(extra) = &cell.zerowidth {
                text.extend(extra.iter());
            }
        }
        text.truncate(text.trim_end().len());
        text
    }

    /// Tout le texte affiché, une ligne par ligne d'écran.
    pub fn text(&self) -> String {
        (0..self.lines)
            .map(|l| self.line_text(l))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Couleur RGB d'une entrée de la palette xterm 256 couleurs (valeurs
/// standard ; pour 0–15, la palette par défaut d'xterm — le thème a la sienne).
pub fn xterm_rgb(index: u8) -> (u8, u8, u8) {
    const BASE: [(u8, u8, u8); 16] = [
        (0, 0, 0),
        (205, 0, 0),
        (0, 205, 0),
        (205, 205, 0),
        (0, 0, 238),
        (205, 0, 205),
        (0, 205, 205),
        (229, 229, 229),
        (127, 127, 127),
        (255, 0, 0),
        (0, 255, 0),
        (255, 255, 0),
        (92, 92, 255),
        (255, 0, 255),
        (0, 255, 255),
        (255, 255, 255),
    ];
    match index {
        0..=15 => BASE[index as usize],
        16..=231 => {
            let i = index - 16;
            let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            (level(i / 36), level((i / 6) % 6), level(i % 6))
        }
        _ => {
            let gray = 8 + (index - 232) * 10;
            (gray, gray, gray)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xterm_palette_values() {
        assert_eq!(xterm_rgb(16), (0, 0, 0));
        assert_eq!(xterm_rgb(196), (255, 0, 0));
        assert_eq!(xterm_rgb(231), (255, 255, 255));
        assert_eq!(xterm_rgb(232), (8, 8, 8));
        assert_eq!(xterm_rgb(255), (238, 238, 238));
    }
}
