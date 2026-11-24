//! Couleurs du terminal : fond, texte, curseur, sélection, 16 couleurs ANSI.
//! Palettes à nous (claire et sombre), remplaçables par `TerminalView::theme`.

use eframe::egui::{Color32, Visuals};
use julie_terminal::{xterm_rgb, Color};

#[derive(Debug, Clone, PartialEq)]
pub struct TerminalTheme {
    pub background: Color32,
    pub foreground: Color32,
    pub cursor: Color32,
    /// Texte sous le curseur plein.
    pub cursor_text: Color32,
    pub selection: Color32,
    pub scrollbar: Color32,
    /// Couleurs ANSI 0–15 (noir, rouge, vert, jaune, bleu, magenta, cyan, blanc,
    /// puis leurs versions vives).
    pub ansi: [Color32; 16],
    /// Le gras éclaircit les couleurs 0–7 (usage répandu des terminaux).
    pub bold_is_bright: bool,
}

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

impl TerminalTheme {
    pub fn dark() -> Self {
        Self {
            background: rgb(28, 29, 33),
            foreground: rgb(208, 211, 217),
            cursor: rgb(220, 222, 226),
            cursor_text: rgb(28, 29, 33),
            selection: rgb(52, 78, 122),
            scrollbar: Color32::from_rgba_unmultiplied(200, 200, 210, 60),
            ansi: [
                rgb(46, 48, 54),
                rgb(226, 104, 104),
                rgb(132, 196, 112),
                rgb(222, 186, 100),
                rgb(100, 152, 232),
                rgb(196, 120, 216),
                rgb(84, 188, 200),
                rgb(200, 203, 210),
                rgb(110, 114, 124),
                rgb(250, 132, 132),
                rgb(160, 222, 138),
                rgb(244, 212, 128),
                rgb(134, 182, 250),
                rgb(222, 150, 240),
                rgb(120, 214, 226),
                rgb(244, 246, 250),
            ],
            bold_is_bright: true,
        }
    }

    pub fn light() -> Self {
        Self {
            background: rgb(250, 250, 248),
            foreground: rgb(36, 38, 44),
            cursor: rgb(36, 38, 44),
            cursor_text: rgb(250, 250, 248),
            selection: rgb(178, 206, 246),
            scrollbar: Color32::from_rgba_unmultiplied(40, 40, 60, 60),
            ansi: [
                rgb(36, 38, 44),
                rgb(196, 40, 40),
                rgb(40, 140, 50),
                rgb(160, 120, 0),
                rgb(30, 90, 200),
                rgb(150, 50, 170),
                rgb(0, 130, 150),
                rgb(170, 172, 178),
                rgb(100, 104, 112),
                rgb(226, 70, 70),
                rgb(60, 170, 70),
                rgb(190, 150, 20),
                rgb(60, 120, 230),
                rgb(180, 80, 200),
                rgb(20, 160, 180),
                rgb(120, 122, 128),
            ],
            bold_is_bright: true,
        }
    }

    pub fn for_visuals(visuals: &Visuals) -> Self {
        if visuals.dark_mode {
            Self::dark()
        } else {
            Self::light()
        }
    }

    /// Couleur d'avant-plan ou de fond d'une cellule.
    pub fn color(&self, color: Color, bold: bool) -> Color32 {
        match color {
            Color::Foreground => self.foreground,
            Color::Background => self.background,
            Color::Indexed(i) if i < 8 && bold && self.bold_is_bright => self.ansi[i as usize + 8],
            Color::Indexed(i) if i < 16 => self.ansi[i as usize],
            Color::Indexed(i) => {
                let (r, g, b) = xterm_rgb(i);
                rgb(r, g, b)
            }
            Color::Rgb(r, g, b) => rgb(r, g, b),
        }
    }
}
