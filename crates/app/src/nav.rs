//! Navigation dans le code, côté interface : liste surgissante des cibles,
//! historique avant / arrière, état du survol avec Ctrl.
//!
//! Aucune règle de résolution ici : `julie-index` dit ce qu'il y a sous une
//! position (`NavigationResult`), ce module n'en montre que le résultat et renvoie
//! l'intention (aller à, chercher les usages). Spécification : `docs/specs/navigation.md`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{
    self, vec2, Color32, Event, FontId, Frame, Id, Key, Margin, Modifiers, Pos2, Rect, Sense,
    Stroke,
};
use julie_core::{Location, TaskId, TextPosition, TextRange};
use julie_index::Declaration;

use crate::texts as t;
use crate::theme::Palette;

/// Positions gardées pour Ctrl+Alt+Gauche / Droite.
const HISTORY_LIMIT: usize = 50;
/// Le survol attend que le pointeur se soit arrêté avant d'interroger l'index.
pub const HOVER_DEBOUNCE: Duration = Duration::from_millis(60);
/// Passé ce délai, la réponse d'un survol n'est plus montrée (pas d'indicateur d'attente).
pub const HOVER_MAX_LATENCY: Duration = Duration::from_millis(100);
/// Immobile depuis ce délai, l'infobulle de déclaration apparaît.
pub const HOVER_TOOLTIP_AFTER: Duration = Duration::from_millis(500);
/// Pause de frappe avant d'envoyer le texte non enregistré à l'index.
pub const TYPING_PAUSE: Duration = Duration::from_millis(300);

const ROW_HEIGHT: f32 = 40.0;
const MAX_ROWS: usize = 15;
const POPUP_WIDTH: f32 = 600.0;

// ----- Historique ------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct History {
    back: Vec<Location>,
    forward: Vec<Location>,
}

impl History {
    /// Un saut va avoir lieu depuis `from` : on s'en souvient, l'« avance » est perdue.
    pub fn push(&mut self, from: Location) {
        if self.back.last() != Some(&from) {
            self.back.push(from);
            if self.back.len() > HISTORY_LIMIT {
                self.back.remove(0);
            }
        }
        self.forward.clear();
    }

    /// Ctrl+Alt+Gauche : la position d'avant, `current` passe dans l'« avance ».
    pub fn go_back(&mut self, current: Option<Location>) -> Option<Location> {
        let target = self.back.pop()?;
        if let Some(current) = current {
            self.forward.push(current);
        }
        Some(target)
    }

    pub fn go_forward(&mut self, current: Option<Location>) -> Option<Location> {
        let target = self.forward.pop()?;
        if let Some(current) = current {
            self.back.push(current);
        }
        Some(target)
    }

    pub fn can_back(&self) -> bool {
        !self.back.is_empty()
    }

    pub fn can_forward(&self) -> bool {
        !self.forward.is_empty()
    }

    pub fn counts(&self) -> (usize, usize) {
        (self.back.len(), self.forward.len())
    }
}

// ----- État de la navigation -----------------------------------------------------------

/// Ce que la personne veut faire d'une réponse de l'index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    /// Ctrl+clic, Ctrl+B : aller à la déclaration.
    Jump,
    /// Alt+F7 : trouver les usages du symbole sous le curseur.
    Usages,
}

/// Une demande de navigation en attente de réponse (clic, Ctrl+B, Alt+F7).
#[derive(Debug, Clone)]
pub struct ClickRequest {
    pub task: TaskId,
    pub purpose: Purpose,
    /// Sous le mot : où ancrer la liste surgissante.
    pub anchor: Pos2,
}

#[derive(Debug, Clone)]
pub struct Inflight {
    pub task: TaskId,
    pub path: PathBuf,
    pub position: TextPosition,
    pub sent: Instant,
}

/// Mot navigable souligné.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub path: PathBuf,
    pub range: TextRange,
}

#[derive(Default)]
pub struct Hover {
    /// Position survolée avec Ctrl et depuis quand (anti-rebond).
    pub pending: Option<(TextPosition, Instant)>,
    /// Dernière position interrogée ou à interroger, et l'instant où le pointeur y est arrivé.
    pub current: Option<(TextPosition, Instant)>,
    pub inflight: Option<Inflight>,
    pub link: Option<Link>,
    /// Ligne de déclaration de la cible (infobulle après 500 ms).
    pub info: Option<String>,
}

#[derive(Debug, Clone)]
pub struct UnsavedSync {
    pub seen_version: u64,
    pub seen_at: Instant,
    /// Version dont le texte est connu de l'index (`None` : l'index lit le disque).
    pub sent_version: Option<u64>,
}

#[derive(Default)]
pub struct NavState {
    pub history: History,
    pub click: Option<ClickRequest>,
    pub hover: Hover,
    pub popup: Option<TargetPopup>,
    pub unsaved: HashMap<PathBuf, UnsavedSync>,
    /// Texte de l'éditeur déjà copié pour l'index : (fichier, version du tampon, texte).
    pub text_cache: Option<(PathBuf, u64, Arc<str>)>,
}

impl NavState {
    pub fn busy(&self) -> bool {
        self.click.is_some() || self.hover.inflight.is_some() || self.hover.pending.is_some()
    }
}

// ----- Liste surgissante ----------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopupPurpose {
    /// Choisir la déclaration où aller.
    Jump,
    /// Choisir la déclaration dont on veut les usages.
    Usages,
}

#[derive(Debug, Clone)]
pub struct PopupItem {
    pub decl: Declaration,
    /// Ligne du constructeur proposée en plus de la classe (`new X`).
    pub constructor: bool,
}

#[derive(Debug)]
pub enum PopupOutcome {
    Pending,
    Close,
    Choose(PopupItem),
}

pub struct TargetPopup {
    pub title: String,
    pub items: Vec<PopupItem>,
    pub filter: String,
    /// Indice dans les lignes visibles (après filtre).
    pub selected: usize,
    pub anchor: Pos2,
    pub purpose: PopupPurpose,
    /// Cible devinée (type inconnu) : bandeau d'avertissement.
    pub guessed: bool,
    scroll_to_selected: bool,
    pub rect: Option<Rect>,
}

impl TargetPopup {
    pub fn new(
        title: String,
        items: Vec<PopupItem>,
        anchor: Pos2,
        purpose: PopupPurpose,
        guessed: bool,
    ) -> Self {
        Self {
            title,
            items,
            filter: String::new(),
            selected: 0,
            anchor,
            purpose,
            guessed,
            scroll_to_selected: true,
            rect: None,
        }
    }

    /// Indices des lignes qui passent le filtre (nom, conteneur ou fichier).
    pub fn visible(&self) -> Vec<usize> {
        let needle = self.filter.to_lowercase();
        self.items
            .iter()
            .enumerate()
            .filter(|(_, item)| {
                needle.is_empty()
                    || item.decl.qualified_name.to_lowercase().contains(&needle)
                    || item.decl.container.to_lowercase().contains(&needle)
                    || item.decl.relative.to_lowercase().contains(&needle)
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// Sélectionne la ligne visible numéro `n` (base zéro).
    pub fn select(&mut self, n: usize) {
        let count = self.visible().len();
        self.selected = n.min(count.saturating_sub(1));
        self.scroll_to_selected = true;
    }

    /// Clavier : à appeler **avant** de dessiner l'éditeur, qui sinon prendrait les
    /// touches. Les frappes de texte servent au filtre.
    pub fn handle_keys(&mut self, ctx: &egui::Context) -> PopupOutcome {
        let (escape, up, down, page_up, page_down, enter, backspace, typed) = ctx.input_mut(|i| {
            let keys = (
                i.consume_key(Modifiers::NONE, Key::Escape),
                i.consume_key(Modifiers::NONE, Key::ArrowUp),
                i.consume_key(Modifiers::NONE, Key::ArrowDown),
                i.consume_key(Modifiers::NONE, Key::PageUp),
                i.consume_key(Modifiers::NONE, Key::PageDown),
                i.consume_key(Modifiers::NONE, Key::Enter),
                i.consume_key(Modifiers::NONE, Key::Backspace),
            );
            let mut typed = String::new();
            i.events.retain(|event| match event {
                Event::Text(text) => {
                    typed.push_str(text);
                    false
                }
                _ => true,
            });
            (
                keys.0, keys.1, keys.2, keys.3, keys.4, keys.5, keys.6, typed,
            )
        });
        if escape {
            return PopupOutcome::Close;
        }
        let before = (self.filter.clone(), self.selected);
        if backspace {
            self.filter.pop();
        }
        self.filter
            .extend(typed.chars().filter(|c| !c.is_control()));
        let visible = self.visible();
        if self.filter != before.0 {
            self.selected = 0;
            self.scroll_to_selected = true;
        }
        let last = visible.len().saturating_sub(1);
        if down {
            self.selected = (self.selected + 1).min(last);
        }
        if up {
            self.selected = self.selected.saturating_sub(1);
        }
        if page_down {
            self.selected = (self.selected + 8).min(last);
        }
        if page_up {
            self.selected = self.selected.saturating_sub(8);
        }
        self.selected = self.selected.min(last);
        if self.selected != before.1 {
            self.scroll_to_selected = true;
        }
        if enter {
            if let Some(&i) = visible.get(self.selected) {
                return PopupOutcome::Choose(self.items[i].clone());
            }
        }
        PopupOutcome::Pending
    }

    /// Dessine la liste sous le mot ; un clic sur une ligne la choisit, un clic
    /// dehors ferme.
    pub fn show(&mut self, ctx: &egui::Context, p: &Palette) -> PopupOutcome {
        let visible = self.visible();
        let screen = ctx.content_rect();
        let rows = visible.len().clamp(1, MAX_ROWS);
        let height = 24.0 + 20.0 + rows as f32 * ROW_HEIGHT + 30.0;
        let x = self
            .anchor
            .x
            .min(screen.right() - POPUP_WIDTH - 12.0)
            .max(screen.left() + 4.0);
        let below = self.anchor.y + 2.0;
        // Pas la place dessous : au-dessus de la ligne du mot.
        let y = if below + height > screen.bottom() - 4.0 {
            (self.anchor.y - 22.0 - height).max(screen.top() + 4.0)
        } else {
            below
        };
        let mut outcome = PopupOutcome::Pending;
        let scroll = std::mem::take(&mut self.scroll_to_selected);
        let area = egui::Area::new(Id::unique("nav-targets-popup"))
            .order(egui::Order::Foreground)
            .fixed_pos(egui::pos2(x, y))
            .show(ctx, |ui| {
                Frame::new()
                    .fill(p.bg_raised)
                    .stroke(Stroke::new(1.0, p.border))
                    .corner_radius(8)
                    .inner_margin(Margin::same(8))
                    .shadow(egui::Shadow {
                        offset: [0, 6],
                        blur: 22,
                        spread: 1,
                        color: Color32::from_black_alpha(110),
                    })
                    .show(ui, |ui| {
                        ui.set_width(POPUP_WIDTH);
                        let title_color = if self.guessed { p.warn } else { p.text_dim };
                        ui.label(
                            egui::RichText::new(&self.title)
                                .size(12.5)
                                .strong()
                                .color(title_color),
                        );
                        let hint = if self.filter.is_empty() {
                            egui::RichText::new(t::NAV_LIST_FILTER_HINT)
                                .size(12.0)
                                .color(p.text_dim)
                        } else {
                            egui::RichText::new(format!("{} ▏", self.filter))
                                .size(12.5)
                                .color(p.text)
                        };
                        ui.label(hint);
                        ui.add_space(2.0);
                        if visible.is_empty() {
                            ui.weak(t::NAV_LIST_NO_MATCH);
                            return;
                        }
                        egui::ScrollArea::vertical()
                            .id_salt("nav-targets-scroll")
                            .max_height(ROW_HEIGHT * MAX_ROWS as f32)
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                for (n, &i) in visible.iter().enumerate() {
                                    let selected = n == self.selected;
                                    let item = &self.items[i];
                                    let (rect, response) = ui.allocate_exact_size(
                                        vec2(ui.available_width(), ROW_HEIGHT),
                                        Sense::click(),
                                    );
                                    if selected {
                                        ui.painter().rect_filled(rect, 5.0, p.accent_soft);
                                        if scroll {
                                            ui.scroll_to_rect(rect, None);
                                        }
                                    } else if response.hovered() {
                                        ui.painter().rect_filled(rect, 5.0, p.bg_alt);
                                    }
                                    paint_item(ui, p, rect, item);
                                    if response.clicked() {
                                        self.selected = n;
                                        outcome = PopupOutcome::Choose(item.clone());
                                    }
                                }
                            });
                        ui.add_space(2.0);
                        ui.label(
                            egui::RichText::new(t::NAV_LIST_KEYS)
                                .size(11.5)
                                .color(p.text_dim),
                        );
                    });
            });
        let rect = area.response.rect;
        self.rect = Some(rect);
        if matches!(outcome, PopupOutcome::Pending) && crate::search::clicked_outside(ctx, rect) {
            return PopupOutcome::Close;
        }
        outcome
    }

    pub fn state(&self) -> serde_json::Value {
        let visible = self.visible();
        serde_json::json!({
            "title": self.title,
            "guessed": self.guessed,
            "purpose": format!("{:?}", self.purpose),
            "filter": self.filter,
            "selected": self.selected,
            "rows": visible.iter().map(|&i| {
                let item = &self.items[i];
                format!(
                    "{}{} | {} | {}:{} | {}",
                    t::symbol_kind_label(item.decl.kind),
                    if item.constructor { " (constructeur)" } else { "" },
                    item.decl.qualified_name,
                    item.decl.relative,
                    item.decl.range.start.line + 1,
                    item.decl.line_text,
                )
            }).collect::<Vec<_>>(),
            "libraries": visible.iter().filter(|&&i| self.items[i].decl.library).count(),
        })
    }
}

fn paint_item(ui: &egui::Ui, p: &Palette, rect: Rect, item: &PopupItem) {
    let decl = &item.decl;
    let painter = ui.painter().with_clip_rect(rect);
    let kind = if item.constructor {
        t::NAV_CONSTRUCTOR
    } else {
        t::symbol_kind_label(decl.kind)
    };
    let top = rect.min.y + 12.0;
    let kind_rect = painter.text(
        egui::pos2(rect.min.x + 8.0, top),
        egui::Align2::LEFT_CENTER,
        kind,
        FontId::proportional(11.5),
        p.accent,
    );
    let name_color = if decl.library { p.text_dim } else { p.text };
    let name_rect = painter.text(
        egui::pos2(kind_rect.max.x + 8.0, top),
        egui::Align2::LEFT_CENTER,
        &decl.qualified_name,
        FontId::proportional(13.5),
        name_color,
    );
    if decl.library {
        painter.text(
            egui::pos2(name_rect.max.x + 8.0, top),
            egui::Align2::LEFT_CENTER,
            t::NAV_LIBRARY,
            FontId::proportional(11.5),
            p.warn,
        );
    }
    painter.text(
        egui::pos2(rect.max.x - 8.0, top),
        egui::Align2::RIGHT_CENTER,
        format!("{}:{}", decl.relative, decl.range.start.line + 1),
        FontId::proportional(12.0),
        p.text_dim,
    );
    painter.text(
        egui::pos2(rect.min.x + 8.0, rect.max.y - 11.0),
        egui::Align2::LEFT_CENTER,
        &decl.line_text,
        FontId::monospace(12.0),
        p.text_dim,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use julie_index::{SymbolKind, SymbolLanguage};

    fn at(line: u32) -> Location {
        Location {
            path: PathBuf::from("a.php"),
            position: TextPosition::new(line, 0),
        }
    }

    pub(crate) fn declaration(name: &str, container: &str, relative: &str) -> Declaration {
        Declaration {
            kind: SymbolKind::Method,
            language: SymbolLanguage::Php,
            name: name.to_owned(),
            qualified_name: format!("{container}::{name}"),
            container: container.to_owned(),
            path: PathBuf::from(relative),
            relative: relative.to_owned(),
            range: TextRange::new(
                TextPosition::new(3, 4),
                TextPosition::new(3, 4 + name.len() as u32),
            ),
            line_text: format!("public function {name}()"),
            library: false,
        }
    }

    #[test]
    fn history_goes_back_and_forward_and_forgets_the_future_on_a_new_jump() {
        let mut history = History::default();
        history.push(at(1));
        history.push(at(2));
        assert_eq!(history.go_back(Some(at(9))), Some(at(2)));
        assert_eq!(history.go_back(Some(at(2))), Some(at(1)));
        assert!(!history.can_back());
        assert_eq!(history.go_forward(Some(at(1))), Some(at(2)));
        assert_eq!(history.counts(), (1, 1));
        // Un nouveau saut efface l'avance.
        history.push(at(5));
        assert!(!history.can_forward());
    }

    #[test]
    fn history_keeps_fifty_positions_and_ignores_a_repeat() {
        let mut history = History::default();
        for line in 0..80 {
            history.push(at(line));
        }
        history.push(at(79));
        assert_eq!(history.counts().0, HISTORY_LIMIT);
        assert_eq!(history.go_back(None), Some(at(79)));
        assert_eq!(history.go_back(None), Some(at(78)));
    }

    #[test]
    fn popup_filters_by_name_container_and_file() {
        let items = vec![
            PopupItem {
                decl: declaration("getName", "App\\Legacy\\Order", "src/Legacy/Order.php"),
                constructor: false,
            },
            PopupItem {
                decl: declaration("getName", "App\\Model\\Admin", "src/Model/Admin.php"),
                constructor: false,
            },
        ];
        let mut popup =
            TargetPopup::new("titre".into(), items, Pos2::ZERO, PopupPurpose::Jump, true);
        assert_eq!(popup.visible(), vec![0, 1]);
        popup.filter = "admin".into();
        assert_eq!(popup.visible(), vec![1]);
        popup.filter = "Order.php".into();
        assert_eq!(popup.visible(), vec![0]);
        popup.filter = "zzz".into();
        assert!(popup.visible().is_empty());
        popup.filter.clear();
        popup.select(99);
        assert_eq!(popup.selected, 1);
    }
}
