//! Fenêtres flottantes de recherche : aller au fichier, recherche dans les
//! fichiers, palette d'actions. Entièrement pilotées au clavier.

pub mod find_in_files;
pub mod goto_file;
pub mod palette;

use eframe::egui::{
    self, text::LayoutJob, vec2, Align2, Color32, Context, FontId, Frame, Id, Margin, Rect, Stroke,
    TextFormat, Ui,
};

use crate::theme::Palette;

/// Fenêtre flottante centrée en haut, sans barre de titre. Rend son rectangle
/// (pour détecter un clic à l'extérieur).
pub fn floating(
    ctx: &Context,
    id: &'static str,
    width: f32,
    p: &Palette,
    content: impl FnOnce(&mut Ui),
) -> Rect {
    let response = egui::Area::new(Id::unique(id))
        .order(egui::Order::Foreground)
        .anchor(Align2::CENTER_TOP, vec2(0.0, 48.0))
        .show(ctx, |ui| {
            Frame::new()
                .fill(p.bg_raised)
                .stroke(Stroke::new(1.0, p.border))
                .corner_radius(10)
                .inner_margin(Margin::same(10))
                .shadow(egui::Shadow {
                    offset: [0, 8],
                    blur: 28,
                    spread: 2,
                    color: Color32::from_black_alpha(110),
                })
                .show(ui, |ui| {
                    ui.set_width(width);
                    content(ui);
                });
        });
    response.response.rect
}

/// Un clic a eu lieu hors de `rect` à cette image.
pub fn clicked_outside(ctx: &Context, rect: Rect) -> bool {
    ctx.input(|i| {
        i.pointer.primary_pressed()
            && i.pointer
                .interact_pos()
                .is_some_and(|pos| !rect.contains(pos))
    })
}

/// Texte d'une ligne avec des plages surlignées (colonnes en caractères).
pub fn highlighted_job(
    text: &str,
    ranges: &[(usize, usize)],
    font: FontId,
    base: Color32,
    highlight_fg: Color32,
    highlight_bg: Color32,
) -> LayoutJob {
    let mut job = LayoutJob::default();
    let mut buffer = String::new();
    let mut current = false;
    let flush = |job: &mut LayoutJob, buffer: &mut String, highlighted: bool| {
        if buffer.is_empty() {
            return;
        }
        job.append(
            buffer,
            0.0,
            TextFormat {
                font_id: font.clone(),
                color: if highlighted { highlight_fg } else { base },
                background: if highlighted {
                    highlight_bg
                } else {
                    Color32::TRANSPARENT
                },
                ..Default::default()
            },
        );
        buffer.clear();
    };
    for (i, ch) in text.chars().enumerate() {
        let highlighted = ranges.iter().any(|&(a, b)| i >= a && i < b);
        if highlighted != current {
            flush(&mut job, &mut buffer, current);
            current = highlighted;
        }
        buffer.push(ch);
    }
    flush(&mut job, &mut buffer, current);
    job
}
