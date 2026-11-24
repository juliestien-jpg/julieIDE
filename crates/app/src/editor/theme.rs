//! Couleurs de l'éditeur. Le composant ne connaît aucune couleur en dur : il lit
//! tout ici. `dev-interface`, qui tient le thème de l'application, peut fournir
//! son propre [`EditorTheme`] ; à défaut, [`EditorTheme::for_visuals`] choisit la
//! palette claire ou sombre de JulieStorm d'après les réglages egui.

use eframe::egui::{Color32, Visuals};
use julie_text::HighlightKind;

use super::decorations::Tone;

/// Les couleurs d'un éditeur.
#[derive(Debug, Clone, PartialEq)]
pub struct EditorTheme {
    pub background: Color32,
    pub text: Color32,
    pub gutter_background: Color32,
    pub line_number: Color32,
    pub line_number_current: Color32,
    pub current_line: Color32,
    pub selection: Color32,
    pub selection_unfocused: Color32,
    pub cursor: Color32,
    pub bracket_match: Color32,
    pub ime_underline: Color32,
    pub scrollbar: Color32,
    pub scrollbar_hover: Color32,
    /// Couleur du texte par catégorie, dans l'ordre de [`HighlightKind::ALL`].
    pub syntax: [Color32; 15],
    /// Fond (plages, lignes) et teinte franche (gouttière) par [`Tone`], dans
    /// l'ordre de [`Tone::ALL`].
    pub tones: [(Color32, Color32); 10],
}

impl EditorTheme {
    pub fn for_visuals(visuals: &Visuals) -> Self {
        if visuals.dark_mode {
            Self::dark()
        } else {
            Self::light()
        }
    }

    /// Palette sombre JulieStorm.
    pub fn dark() -> Self {
        let rgb = Color32::from_rgb;
        let rgba = Color32::from_rgba_unmultiplied;
        Self {
            background: rgb(0x1f, 0x20, 0x24),
            text: rgb(0xc9, 0xcd, 0xd4),
            gutter_background: rgb(0x1f, 0x20, 0x24),
            line_number: rgb(0x5d, 0x62, 0x6b),
            line_number_current: rgb(0xa9, 0xae, 0xb7),
            current_line: rgb(0x26, 0x28, 0x2e),
            selection: rgb(0x21, 0x42, 0x83),
            selection_unfocused: rgb(0x34, 0x3a, 0x46),
            cursor: rgb(0xe6, 0xe8, 0xec),
            bracket_match: rgb(0x3b, 0x51, 0x4d),
            ime_underline: rgb(0xe6, 0xe8, 0xec),
            scrollbar: rgba(0xff, 0xff, 0xff, 0x22),
            scrollbar_hover: rgba(0xff, 0xff, 0xff, 0x44),
            syntax: [
                rgb(0xd0, 0x8b, 0x5b), // mot-clé
                rgb(0x8f, 0xb5, 0x73), // chaîne
                rgb(0x5f, 0xa8, 0xd3), // nombre
                rgb(0x7a, 0x80, 0x8a), // commentaire
                rgb(0xc7, 0x92, 0xea), // variable
                rgb(0xe5, 0xc0, 0x7b), // fonction
                rgb(0x4e, 0xc9, 0xb0), // type
                rgb(0xb3, 0x8c, 0xd9), // constante
                rgb(0xd7, 0xba, 0x7d), // balise
                rgb(0xa5, 0xb8, 0xe0), // attribut
                rgb(0x9c, 0xc6, 0xd8), // propriété
                rgb(0xe0, 0xa3, 0x6f), // sélecteur
                rgb(0xb0, 0xb6, 0xbf), // opérateur
                rgb(0x9a, 0xa0, 0xa8), // ponctuation
                rgb(0xd6, 0x9d, 0x85), // échappement
            ],
            tones: [
                (rgba(0x4e, 0x9a, 0x5a, 0x40), rgb(0x5a, 0xa8, 0x66)), // ajouté
                (rgba(0xb0, 0x4a, 0x4a, 0x40), rgb(0xc0, 0x5a, 0x5a)), // supprimé
                (rgba(0x4a, 0x7a, 0xb8, 0x40), rgb(0x5a, 0x8a, 0xc8)), // modifié
                (rgba(0xc0, 0x50, 0x40, 0x55), rgb(0xd0, 0x60, 0x50)), // conflit
                (rgba(0x8a, 0x7a, 0x30, 0x60), rgb(0xb0, 0x9a, 0x40)), // résultat de recherche
                (rgba(0xc0, 0x9a, 0x20, 0x90), rgb(0xe0, 0xb4, 0x30)), // résultat courant
                (rgba(0x50, 0x60, 0x70, 0x60), rgb(0x70, 0x80, 0x90)), // occurrence
                (rgba(0xc0, 0x9a, 0x20, 0x30), rgb(0xd0, 0xaa, 0x30)), // avertissement
                (rgba(0xc0, 0x40, 0x40, 0x30), rgb(0xd0, 0x50, 0x50)), // erreur
                (rgba(0x60, 0x90, 0xc0, 0x30), rgb(0x70, 0xa0, 0xd0)), // information
            ],
        }
    }

    /// Palette claire JulieStorm.
    pub fn light() -> Self {
        let rgb = Color32::from_rgb;
        let rgba = Color32::from_rgba_unmultiplied;
        Self {
            background: rgb(0xff, 0xff, 0xff),
            text: rgb(0x1e, 0x1f, 0x22),
            gutter_background: rgb(0xfa, 0xfa, 0xfb),
            line_number: rgb(0xa0, 0xa4, 0xab),
            line_number_current: rgb(0x40, 0x44, 0x4b),
            current_line: rgb(0xf3, 0xf5, 0xfa),
            selection: rgb(0xb8, 0xd2, 0xf5),
            selection_unfocused: rgb(0xd8, 0xdd, 0xe5),
            cursor: rgb(0x10, 0x10, 0x10),
            bracket_match: rgb(0xc8, 0xe8, 0xdc),
            ime_underline: rgb(0x10, 0x10, 0x10),
            scrollbar: rgba(0, 0, 0, 0x22),
            scrollbar_hover: rgba(0, 0, 0, 0x44),
            syntax: [
                rgb(0x00, 0x33, 0xb3), // mot-clé
                rgb(0x06, 0x7d, 0x17), // chaîne
                rgb(0x17, 0x50, 0xeb), // nombre
                rgb(0x8c, 0x8c, 0x8c), // commentaire
                rgb(0x87, 0x10, 0x94), // variable
                rgb(0x00, 0x62, 0x7a), // fonction
                rgb(0x00, 0x80, 0x80), // type
                rgb(0x87, 0x10, 0x94), // constante
                rgb(0x00, 0x33, 0xb3), // balise
                rgb(0x17, 0x4a, 0xd4), // attribut
                rgb(0x87, 0x10, 0x94), // propriété
                rgb(0x9e, 0x52, 0x00), // sélecteur
                rgb(0x3a, 0x3a, 0x3a), // opérateur
                rgb(0x50, 0x50, 0x50), // ponctuation
                rgb(0x00, 0x37, 0xa6), // échappement
            ],
            tones: [
                (rgba(0x6a, 0xc0, 0x70, 0x40), rgb(0x5a, 0xb0, 0x60)),
                (rgba(0xe0, 0x70, 0x70, 0x40), rgb(0xd0, 0x60, 0x60)),
                (rgba(0x70, 0xa0, 0xe0, 0x40), rgb(0x60, 0x90, 0xd0)),
                (rgba(0xf0, 0x80, 0x70, 0x60), rgb(0xe0, 0x60, 0x50)),
                (rgba(0xf0, 0xd0, 0x60, 0x80), rgb(0xd0, 0xb0, 0x40)),
                (rgba(0xf0, 0xb0, 0x30, 0xb0), rgb(0xe0, 0x90, 0x20)),
                (rgba(0xa0, 0xb0, 0xc0, 0x60), rgb(0x80, 0x90, 0xa0)),
                (rgba(0xf0, 0xd0, 0x60, 0x40), rgb(0xd0, 0xa0, 0x30)),
                (rgba(0xf0, 0x80, 0x80, 0x40), rgb(0xd0, 0x40, 0x40)),
                (rgba(0x80, 0xb0, 0xf0, 0x40), rgb(0x50, 0x80, 0xd0)),
            ],
        }
    }

    pub fn syntax_color(&self, kind: HighlightKind) -> Color32 {
        let index = HighlightKind::ALL
            .iter()
            .position(|k| *k == kind)
            .unwrap_or_default();
        self.syntax[index]
    }

    /// Fond d'une plage ou d'une ligne décorée.
    pub fn tone_background(&self, tone: Tone) -> Color32 {
        self.tones[tone.index()].0
    }

    /// Teinte franche (marque de gouttière, séparateur).
    pub fn tone_strong(&self, tone: Tone) -> Color32 {
        self.tones[tone.index()].1
    }
}
