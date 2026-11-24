//! Identité visuelle de JulieStorm : un thème sombre « nuit d'orage » (bleu nuit,
//! accent violet électrique, éclair jaune pour les correspondances) et un thème
//! clair assorti. Rien n'est repris d'un autre IDE.
//!
//! Toute couleur de l'interface passe par [`Palette`] ; les composants (éditeur,
//! terminal) gardent leur propre thème, que l'on règle ici pour qu'il s'accorde.

use eframe::egui::{self, Color32, CornerRadius, FontFamily, FontId, Stroke, TextStyle, Visuals};

use crate::editor::EditorTheme;

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

/// Les couleurs de l'interface, claires ou sombres.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub dark: bool,
    /// Fond des panneaux.
    pub bg: Color32,
    /// Barres (menus, onglets inactifs, état).
    pub bg_alt: Color32,
    /// Fenêtres flottantes, menus.
    pub bg_raised: Color32,
    pub border: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub accent: Color32,
    /// Fond d'une ligne sélectionnée.
    pub accent_soft: Color32,
    /// Fond du texte apparié (recherche) : l'éclair.
    pub spark_bg: Color32,
    pub spark_fg: Color32,
    #[allow(dead_code)] // pour les états « réussi » des vues de la passe 2
    pub ok: Color32,
    pub warn: Color32,
    pub error: Color32,
    pub git_modified: Color32,
    pub git_added: Color32,
    pub git_untracked: Color32,
    pub git_deleted: Color32,
    pub git_conflict: Color32,
    pub ignored: Color32,
}

pub fn palette(dark: bool) -> Palette {
    if dark {
        Palette {
            dark,
            bg: rgb(0x17, 0x19, 0x25),
            bg_alt: rgb(0x1e, 0x21, 0x32),
            bg_raised: rgb(0x25, 0x29, 0x40),
            border: rgb(0x34, 0x3a, 0x55),
            text: rgb(0xd5, 0xd9, 0xe8),
            text_dim: rgb(0x81, 0x89, 0xa6),
            accent: rgb(0x8e, 0x7b, 0xff),
            accent_soft: rgb(0x3a, 0x34, 0x70),
            spark_bg: rgb(0x6b, 0x55, 0x12),
            spark_fg: rgb(0xff, 0xd9, 0x6b),
            ok: rgb(0x5f, 0xd3, 0x8d),
            warn: rgb(0xf2, 0xb8, 0x4b),
            error: rgb(0xff, 0x6b, 0x7a),
            git_modified: rgb(0x6f, 0xa8, 0xff),
            git_added: rgb(0x5f, 0xd3, 0x8d),
            git_untracked: rgb(0xd9, 0xa0, 0x5b),
            git_deleted: rgb(0x8a, 0x90, 0xa8),
            git_conflict: rgb(0xff, 0x6b, 0x7a),
            ignored: rgb(0x5e, 0x65, 0x82),
        }
    } else {
        Palette {
            dark,
            bg: rgb(0xf5, 0xf4, 0xfa),
            bg_alt: rgb(0xeb, 0xea, 0xf3),
            bg_raised: rgb(0xff, 0xff, 0xff),
            border: rgb(0xcf, 0xcd, 0xe0),
            text: rgb(0x22, 0x24, 0x3a),
            text_dim: rgb(0x6b, 0x6f, 0x8c),
            accent: rgb(0x5b, 0x3f, 0xe0),
            accent_soft: rgb(0xda, 0xd3, 0xfa),
            spark_bg: rgb(0xff, 0xe3, 0x94),
            spark_fg: rgb(0x5a, 0x3d, 0x00),
            ok: rgb(0x1e, 0x9e, 0x5a),
            warn: rgb(0xb7, 0x79, 0x1f),
            error: rgb(0xd6, 0x33, 0x4a),
            git_modified: rgb(0x25, 0x63, 0xc9),
            git_added: rgb(0x1e, 0x9e, 0x5a),
            git_untracked: rgb(0xb3, 0x68, 0x0f),
            git_deleted: rgb(0x7a, 0x7f, 0x99),
            git_conflict: rgb(0xd6, 0x33, 0x4a),
            ignored: rgb(0xa5, 0xa8, 0xc0),
        }
    }
}

/// Thème de l'éditeur accordé à la palette.
pub fn editor_theme(dark: bool) -> EditorTheme {
    let p = palette(dark);
    let mut theme = if dark {
        EditorTheme::dark()
    } else {
        EditorTheme::light()
    };
    if dark {
        theme.background = rgb(0x1a, 0x1c, 0x2b);
        theme.gutter_background = rgb(0x1a, 0x1c, 0x2b);
        theme.current_line = rgb(0x21, 0x24, 0x37);
        theme.selection = rgb(0x3f, 0x38, 0x7c);
        theme.selection_unfocused = rgb(0x2e, 0x31, 0x4a);
        theme.cursor = rgb(0xf0, 0xe9, 0xff);
    } else {
        theme.gutter_background = rgb(0xfa, 0xf9, 0xfd);
        theme.current_line = rgb(0xf3, 0xf1, 0xfb);
        theme.selection = p.accent_soft;
    }
    theme
}

/// Applique le thème à tout l'interface.
pub fn apply(ctx: &egui::Context, dark: bool) {
    let p = palette(dark);
    let mut visuals = if dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };
    visuals.panel_fill = p.bg;
    visuals.window_fill = p.bg_raised;
    visuals.window_stroke = Stroke::new(1.0, p.border);
    visuals.extreme_bg_color = if dark {
        rgb(0x11, 0x13, 0x1d)
    } else {
        rgb(0xff, 0xff, 0xff)
    };
    visuals.faint_bg_color = p.bg_alt;
    visuals.code_bg_color = p.bg_alt;
    visuals.hyperlink_color = p.accent;
    visuals.warn_fg_color = p.warn;
    visuals.error_fg_color = p.error;
    visuals.selection.bg_fill = p.accent_soft;
    visuals.selection.stroke = Stroke::new(1.0, p.accent);
    visuals.window_corner_radius = CornerRadius::same(8);
    visuals.menu_corner_radius = CornerRadius::same(6);

    visuals.widgets.noninteractive.bg_fill = p.bg;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    visuals.widgets.inactive.bg_fill = p.bg_alt;
    visuals.widgets.inactive.weak_bg_fill = p.bg_alt;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, p.border);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, p.text);
    visuals.widgets.hovered.bg_fill = p.accent_soft;
    visuals.widgets.hovered.weak_bg_fill = p.accent_soft;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, p.accent);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.5, p.text);
    visuals.widgets.active.bg_fill = p.accent;
    visuals.widgets.active.weak_bg_fill = p.accent;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, p.accent);
    visuals.widgets.active.fg_stroke = Stroke::new(1.5, if dark { p.text } else { Color32::WHITE });
    visuals.widgets.open.bg_fill = p.accent_soft;
    visuals.widgets.open.weak_bg_fill = p.accent_soft;
    let which = if dark {
        egui::Theme::Dark
    } else {
        egui::Theme::Light
    };
    ctx.set_theme(which);
    ctx.set_visuals_of(which, visuals);

    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = egui::vec2(8.0, 4.0);
        style.spacing.button_padding = egui::vec2(8.0, 3.0);
        style
            .text_styles
            .insert(TextStyle::Body, FontId::new(14.0, FontFamily::Proportional));
        style.text_styles.insert(
            TextStyle::Button,
            FontId::new(14.0, FontFamily::Proportional),
        );
        style.text_styles.insert(
            TextStyle::Small,
            FontId::new(12.0, FontFamily::Proportional),
        );
        style.text_styles.insert(
            TextStyle::Monospace,
            FontId::new(13.5, FontFamily::Monospace),
        );
    });
}
