//! Palette d'actions : liste filtrable des actions du registre, avec leur
//! raccourci. Tout au clavier (Haut/Bas, Entrée, Échap).

use eframe::egui::{self, vec2, FontId, Key, Modifiers, Sense, TextEdit};

use super::{clicked_outside, floating};
use crate::actions::{ActionId, Registry};
use crate::texts as t;
use crate::theme::Palette;

pub enum Outcome {
    Pending,
    Close,
    Run(ActionId),
}

pub struct ActionPalette {
    pub query: String,
    pub selected: usize,
    focus_pending: bool,
}

impl ActionPalette {
    pub fn new() -> Self {
        Self {
            query: String::new(),
            selected: 0,
            focus_pending: true,
        }
    }

    /// Actions dont le libellé contient tous les mots de la requête.
    pub fn matches(&self, registry: &Registry) -> Vec<ActionId> {
        let query = self.query.to_lowercase();
        let words: Vec<&str> = query.split_whitespace().collect();
        registry
            .all()
            .iter()
            .filter(|a| {
                let label = a.label.to_lowercase();
                words.iter().all(|w| label.contains(w))
            })
            .map(|a| a.id)
            .collect()
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        registry: &Registry,
        enabled: &dyn Fn(ActionId) -> bool,
    ) -> Outcome {
        let (up, down, enter, escape) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::NONE, Key::ArrowUp),
                i.consume_key(Modifiers::NONE, Key::ArrowDown),
                i.consume_key(Modifiers::NONE, Key::Enter),
                i.consume_key(Modifiers::NONE, Key::Escape),
            )
        });
        if escape {
            return Outcome::Close;
        }
        let matches = self.matches(registry);
        if down && !matches.is_empty() {
            self.selected = (self.selected + 1).min(matches.len() - 1);
        }
        if up {
            self.selected = self.selected.saturating_sub(1);
        }
        self.selected = self.selected.min(matches.len().saturating_sub(1));
        let mut outcome = Outcome::Pending;
        if enter {
            if let Some(&id) = matches.get(self.selected) {
                if enabled(id) {
                    outcome = Outcome::Run(id);
                }
            }
        }
        let before = self.query.clone();
        let rect = floating(ctx, "julie-action-palette", 520.0, p, |ui| {
            ui.label(
                egui::RichText::new(t::PALETTE_TITLE)
                    .small()
                    .color(p.text_dim),
            );
            let edit = ui.add(
                TextEdit::singleline(&mut self.query)
                    .hint_text(t::PALETTE_HINT)
                    .desired_width(f32::INFINITY)
                    .margin(vec2(8.0, 6.0)),
            );
            if self.focus_pending {
                self.focus_pending = false;
                edit.request_focus();
            }
            ui.add_space(4.0);
            if matches.is_empty() {
                ui.weak(t::PALETTE_NONE);
            }
            for (i, &id) in matches.iter().enumerate() {
                let action = registry.get(id);
                let on = enabled(id);
                let (rect, response) =
                    ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::click());
                if i == self.selected {
                    ui.painter().rect_filled(rect, 5.0, p.accent_soft);
                } else if response.hovered() {
                    ui.painter().rect_filled(rect, 5.0, p.bg_alt);
                }
                let color = if on { p.text } else { p.text_dim };
                ui.painter().text(
                    egui::pos2(rect.min.x + 10.0, rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    action.label,
                    FontId::proportional(14.0),
                    color,
                );
                if let Some(text) = registry.shortcut_text(id) {
                    ui.painter().text(
                        egui::pos2(rect.max.x - 10.0, rect.center().y),
                        egui::Align2::RIGHT_CENTER,
                        text,
                        FontId::proportional(12.5),
                        p.text_dim,
                    );
                }
                if response.clicked() && on {
                    outcome = Outcome::Run(id);
                }
            }
        });
        if self.query != before {
            self.selected = 0;
        }
        if matches!(outcome, Outcome::Pending) && clicked_outside(ctx, rect) {
            return Outcome::Close;
        }
        outcome
    }
}

impl Default for ActionPalette {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_keeps_actions_containing_every_word() {
        let registry = Registry::new();
        let mut palette = ActionPalette::new();
        palette.query = "enregistrer tout".into();
        assert_eq!(palette.matches(&registry), vec![ActionId::SaveAll]);
        palette.query = String::new();
        assert_eq!(palette.matches(&registry).len(), registry.all().len());
        palette.query = "zzz".into();
        assert!(palette.matches(&registry).is_empty());
    }
}
