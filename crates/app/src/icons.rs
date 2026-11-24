//! Les icônes de JulieStorm : dessinées au tracé (formes simples, une couleur),
//! pas de police d'icônes ni d'image reprise ailleurs.

use eframe::egui::{self, pos2, vec2, Color32, Painter, Rect, Response, Sense, Shape, Stroke, Ui};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Folder,
    FolderOpen,
    File,
    ChevronRight,
    ChevronDown,
    Branch,
    Terminal,
    Flask,
    Search,
    Close,
    Dot,
    Bolt,
}

/// Dessine `icon` dans `rect` (carré de préférence).
pub fn paint(p: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let at = |x: f32, y: f32| {
        pos2(
            rect.min.x + rect.width() * x,
            rect.min.y + rect.height() * y,
        )
    };
    let w = (rect.width() / 12.0).max(1.2);
    let stroke = Stroke::new(w, color);
    let line = |a: (f32, f32), b: (f32, f32)| p.line_segment([at(a.0, a.1), at(b.0, b.1)], stroke);
    let poly = |pts: &[(f32, f32)]| {
        p.add(Shape::line(
            pts.iter().map(|&(x, y)| at(x, y)).collect(),
            stroke,
        ))
    };
    match icon {
        Icon::Folder => {
            p.rect_filled(
                Rect::from_min_max(at(0.08, 0.16), at(0.42, 0.36)),
                1.5,
                color,
            );
            p.rect_filled(
                Rect::from_min_max(at(0.08, 0.30), at(0.92, 0.84)),
                2.0,
                color,
            );
        }
        Icon::FolderOpen => {
            poly(&[
                (0.08, 0.84),
                (0.08, 0.18),
                (0.42, 0.18),
                (0.5, 0.32),
                (0.86, 0.32),
                (0.86, 0.46),
            ]);
            p.add(Shape::convex_polygon(
                vec![
                    at(0.08, 0.84),
                    at(0.22, 0.46),
                    at(0.97, 0.46),
                    at(0.84, 0.84),
                ],
                color,
                Stroke::NONE,
            ));
        }
        Icon::File => {
            p.add(Shape::closed_line(
                vec![
                    at(0.2, 0.08),
                    at(0.62, 0.08),
                    at(0.82, 0.28),
                    at(0.82, 0.92),
                    at(0.2, 0.92),
                ],
                stroke,
            ));
            poly(&[(0.62, 0.08), (0.62, 0.28), (0.82, 0.28)]);
            line((0.32, 0.52), (0.7, 0.52));
            line((0.32, 0.7), (0.7, 0.7));
        }
        Icon::ChevronRight => {
            poly(&[(0.38, 0.25), (0.64, 0.5), (0.38, 0.75)]);
        }
        Icon::ChevronDown => {
            poly(&[(0.25, 0.38), (0.5, 0.64), (0.75, 0.38)]);
        }
        Icon::Branch => {
            let r = rect.width() * 0.11;
            line((0.3, 0.26), (0.3, 0.74));
            poly(&[(0.72, 0.4), (0.72, 0.5), (0.3, 0.68)]);
            for (x, y) in [(0.3, 0.2), (0.3, 0.8), (0.72, 0.34)] {
                p.circle_filled(at(x, y), r * 1.2, color);
            }
        }
        Icon::Terminal => {
            p.rect_stroke(
                Rect::from_min_max(at(0.06, 0.16), at(0.94, 0.84)),
                2.0,
                stroke,
                egui::StrokeKind::Inside,
            );
            poly(&[(0.22, 0.36), (0.4, 0.5), (0.22, 0.64)]);
            line((0.5, 0.65), (0.74, 0.65));
        }
        Icon::Flask => {
            p.add(Shape::closed_line(
                vec![
                    at(0.4, 0.1),
                    at(0.6, 0.1),
                    at(0.6, 0.4),
                    at(0.9, 0.86),
                    at(0.1, 0.86),
                    at(0.4, 0.4),
                ],
                stroke,
            ));
            line((0.26, 0.66), (0.74, 0.66));
        }
        Icon::Search => {
            p.circle_stroke(at(0.42, 0.42), rect.width() * 0.27, stroke);
            line((0.62, 0.62), (0.9, 0.9));
        }
        Icon::Close => {
            line((0.28, 0.28), (0.72, 0.72));
            line((0.72, 0.28), (0.28, 0.72));
        }
        Icon::Dot => {
            p.circle_filled(at(0.5, 0.5), rect.width() * 0.2, color);
        }
        Icon::Bolt => {
            p.add(Shape::convex_polygon(
                vec![
                    at(0.6, 0.04),
                    at(0.2, 0.58),
                    at(0.46, 0.58),
                    at(0.36, 0.96),
                    at(0.82, 0.38),
                    at(0.55, 0.38),
                ],
                color,
                Stroke::NONE,
            ));
        }
    }
}

/// Bouton carré portant une icône, avec fond quand il est sélectionné ou survolé.
pub fn button(
    ui: &mut Ui,
    icon: Icon,
    size: f32,
    selected: bool,
    colors: &crate::theme::Palette,
    tooltip: &str,
) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    if selected || response.hovered() {
        let fill = if selected {
            colors.accent_soft
        } else {
            colors.bg_alt
        };
        ui.painter().rect_filled(rect, 6.0, fill);
    }
    if selected {
        let bar = Rect::from_min_size(rect.min, vec2(3.0, rect.height()));
        ui.painter().rect_filled(bar, 1.5, colors.accent);
    }
    let color = if selected {
        colors.text
    } else {
        colors.text_dim
    };
    paint(ui.painter(), rect.shrink(size * 0.22), icon, color);
    response.on_hover_text(tooltip)
}
