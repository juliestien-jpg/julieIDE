//! Petits outils d'interface partagés par les vues Git, fusion et tests :
//! disposition par rectangles, boutons, pastilles, dates et durées lisibles.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use eframe::egui::{
    self, pos2, vec2, Align2, Color32, FontId, Rect, Response, RichText, Sense, Ui, UiBuilder,
};

use crate::texts as t;
use crate::theme::Palette;

/// Un sous-espace de `ui` limité à `rect`.
pub fn child<R>(ui: &mut Ui, rect: Rect, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.scope_builder(UiBuilder::new().max_rect(rect), add).inner
}

/// Bouton de texte ; `primary` le met en avant (fond d'accent).
pub fn button(ui: &mut Ui, p: &Palette, label: &str, primary: bool, enabled: bool) -> Response {
    let text = if primary && enabled {
        RichText::new(label).color(Color32::WHITE).strong()
    } else {
        RichText::new(label)
    };
    let mut button = egui::Button::new(text);
    if primary && enabled {
        button = button.fill(p.accent);
    }
    ui.add_enabled(enabled, button)
}

/// Pastille de texte (étiquette de branche, compteur…) peinte à `pos` (coin
/// haut-gauche, hauteur 16). Rend sa largeur.
pub fn pill(ui: &Ui, pos: egui::Pos2, text: &str, fill: Color32, fg: Color32, clip: Rect) -> f32 {
    let painter = ui.painter().with_clip_rect(clip);
    let galley = painter.layout_no_wrap(text.to_owned(), FontId::proportional(11.5), fg);
    let width = galley.size().x + 10.0;
    let rect = Rect::from_min_size(pos, vec2(width, 16.0));
    painter.rect_filled(rect, 8.0, fill);
    painter.galley(
        pos2(pos.x + 5.0, pos.y + 8.0 - galley.size().y / 2.0),
        galley,
        fg,
    );
    width
}

/// Case à cocher peinte dans `rect` (14 × 14 au centre).
pub fn paint_checkbox(ui: &Ui, rect: Rect, checked: bool, partial: bool, p: &Palette) {
    let boxed = Rect::from_center_size(rect.center(), vec2(14.0, 14.0));
    let painter = ui.painter();
    if checked || partial {
        painter.rect_filled(boxed, 3.0, p.accent);
        if checked {
            crate::icons::paint(
                painter,
                boxed.shrink(2.0),
                crate::icons::Icon::Check,
                Color32::WHITE,
            );
        } else {
            painter.rect_filled(
                Rect::from_center_size(boxed.center(), vec2(8.0, 2.0)),
                1.0,
                Color32::WHITE,
            );
        }
    } else {
        painter.rect_stroke(
            boxed,
            3.0,
            egui::Stroke::new(1.3, p.text_dim),
            egui::StrokeKind::Inside,
        );
    }
}

/// Texte centré dans `rect` (états vides).
pub fn centered_note(ui: &Ui, rect: Rect, text: &str, p: &Palette) {
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        FontId::proportional(14.0),
        p.text_dim,
    );
}

/// Aspect d'un bouton de texte dessiné dans un rectangle imposé.
#[derive(Debug, Clone, Copy)]
pub enum Look {
    Normal,
    Primary(Color32),
}

/// Bouton de texte posé dans `rect` (barres d'outils, pieds de dialogue).
#[allow(clippy::too_many_arguments)]
pub fn text_button_at(
    ui: &mut Ui,
    p: &Palette,
    rect: Rect,
    id: egui::Id,
    label: &str,
    look: Look,
    enabled: bool,
    tooltip: &str,
) -> bool {
    let response = ui.interact(rect, id, Sense::click());
    let hovered = enabled && response.hovered();
    let (fill, fg) = match look {
        Look::Primary(color) if enabled => (
            if hovered {
                color.gamma_multiply(1.15)
            } else {
                color
            },
            Color32::WHITE,
        ),
        _ => (
            if hovered { p.accent_soft } else { p.bg_alt },
            if enabled { p.text } else { p.ignored },
        ),
    };
    ui.painter().rect_filled(rect, 5.0, fill);
    if matches!(look, Look::Normal) {
        ui.painter().rect_stroke(
            rect,
            5.0,
            egui::Stroke::new(1.0, p.border),
            egui::StrokeKind::Inside,
        );
    }
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        FontId::proportional(13.0),
        fg,
    );
    if !tooltip.is_empty() {
        response.clone().on_hover_text(tooltip);
    }
    enabled && response.clicked()
}

/// Largeur d'un bouton de texte pour `label`.
pub fn text_width(ui: &Ui, label: &str) -> f32 {
    ui.painter()
        .layout_no_wrap(label.to_owned(), FontId::proportional(13.0), Color32::WHITE)
        .size()
        .x
        + 22.0
}

/// Zone cliquable sans rien dessiner.
pub fn hit(ui: &mut Ui, rect: Rect, id: egui::Id) -> Response {
    ui.interact(rect, id, Sense::click())
}

/// `1,02 s`, `340 ms`, `2 min 05 s`.
pub fn format_duration(duration: Duration) -> String {
    let ms = duration.as_millis();
    if ms < 1000 {
        format!("{ms} ms")
    } else if ms < 60_000 {
        format!("{:.2} s", duration.as_secs_f64()).replace('.', ",")
    } else {
        let s = duration.as_secs();
        format!("{} min {:02} s", s / 60, s % 60)
    }
}

/// Jours depuis 1970-01-01 → (année, mois, jour) du calendrier grégorien.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// `JJ/MM/AAAA HH:MM`, dans le fuseau de la personne (décalage en minutes).
pub fn format_date(unix: i64, offset_minutes: i32, seconds: bool) -> String {
    let local = unix + i64::from(offset_minutes) * 60;
    let days = local.div_euclid(86_400);
    let rest = local.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    let (hh, mm, ss) = (rest / 3600, rest % 3600 / 60, rest % 60);
    if seconds {
        format!("{d:02}/{m:02}/{y:04} {hh:02}:{mm:02}:{ss:02}")
    } else {
        format!("{d:02}/{m:02}/{y:04} {hh:02}:{mm:02}")
    }
}

/// Relatif sous 7 jours (« il y a 3 h »), sinon la date.
pub fn format_when(unix: i64, offset_minutes: i32) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let age = now - unix;
    if (0..7 * 86_400).contains(&age) {
        return t::relative_age(age as u64);
    }
    format_date(unix, offset_minutes, false)
}

/// Taille lisible : `812 o`, `12 Ko`, `3,4 Mo`.
pub fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} o")
    } else if bytes < 1024 * 1024 {
        format!("{} Ko", bytes / 1024)
    } else {
        format!("{:.1} Mo", bytes as f64 / 1_048_576.0).replace('.', ",")
    }
}

/// Fenêtre modale sans fondu d'entrée (les captures du mode scénario et les
/// utilisatrices pressées la voient tout de suite).
pub fn modal(salt: &'static str) -> egui::Modal {
    let id = egui::Id::unique(salt);
    egui::Modal::new(id).area(egui::Modal::default_area(id).fade_in(false))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_are_written_day_first() {
        assert_eq!(format_date(0, 0, false), "01/01/1970 00:00");
        // 2026-11-24 09:30:05 UTC, puis le même instant à +01:00.
        assert_eq!(format_date(1_795_512_605, 0, true), "24/11/2026 09:30:05");
        assert_eq!(format_date(1_795_512_605, 60, false), "24/11/2026 10:30");
        assert_eq!(format_date(951_782_400, 0, false), "29/02/2000 00:00");
    }

    #[test]
    fn durations_and_sizes_read_naturally() {
        assert_eq!(format_duration(Duration::from_millis(340)), "340 ms");
        assert_eq!(format_duration(Duration::from_millis(1020)), "1,02 s");
        assert_eq!(format_duration(Duration::from_secs(125)), "2 min 05 s");
        assert_eq!(format_size(812), "812 o");
        assert_eq!(format_size(12 * 1024), "12 Ko");
    }
}
