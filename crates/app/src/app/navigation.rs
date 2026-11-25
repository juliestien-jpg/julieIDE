//! Navigation dans le code : Ctrl+clic, Ctrl+B, survol avec Ctrl, Alt+F7, liste
//! surgissante, fenêtre Usages, historique, texte non enregistré envoyé à l'index.
//!
//! Aucune règle de résolution ici : on envoie la position et le texte de l'éditeur à
//! `julie-index`, on montre sa réponse. Spécification : `docs/specs/navigation.md`.

use eframe::egui::{Event, Pos2};
use julie_core::Location;
use julie_index::{Declaration, Navigation, NavigationRequest, NavigationResult, NoTarget};
use julie_index::{Usage, UsagesQuery, UsagesSummary};

use super::*;
use crate::editor::EditorIntent;
use crate::nav::{
    ClickRequest, Inflight, Link, NavState, PopupItem, PopupOutcome, PopupPurpose, Purpose,
    TargetPopup, UnsavedSync, HOVER_DEBOUNCE, HOVER_MAX_LATENCY, HOVER_TOOLTIP_AFTER, TYPING_PAUSE,
};
use crate::usages::UsagesIntent;

/// Les langages que l'index des symboles sait lire.
fn is_code_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(str::to_lowercase)
        .is_some_and(|e| {
            matches!(
                e.as_str(),
                "php" | "phtml" | "inc" | "js" | "mjs" | "cjs" | "jsx" | "ts" | "tsx"
            )
        })
}

fn ctrl_modifiers() -> Modifiers {
    Modifiers {
        ctrl: true,
        command: true,
        ..Modifiers::NONE
    }
}

impl JulieStormApp {
    // ----- Disponibilité ------------------------------------------------------------

    /// Un fichier de code est ouvert dans l'éditeur actif et un projet est ouvert.
    pub(super) fn nav_available(&self) -> bool {
        self.project.is_some()
            && self
                .editors
                .active_editor()
                .and_then(|e| e.path())
                .is_some_and(is_code_file)
    }

    pub(super) fn nav_unavailable_reason(&self) -> Option<String> {
        if self.project.is_none() {
            Some(t::REASON_NO_PROJECT_OPEN.to_owned())
        } else if !self.nav_available() {
            Some(t::REASON_NO_CODE_FILE.to_owned())
        } else {
            None
        }
    }

    // ----- Intentions de l'éditeur --------------------------------------------------

    pub(super) fn nav_editor_intents(&mut self, intents: Vec<EditorIntent>) {
        for intent in intents {
            match intent {
                EditorIntent::GutterMarkClicked { .. } => {}
                EditorIntent::Navigate { position, anchor } => {
                    self.nav_request(Purpose::Jump, position, anchor);
                }
                EditorIntent::Hover { position } => {
                    let hover = &mut self.nav.hover;
                    hover.current = Some((position, Instant::now()));
                    hover.pending = Some((position, Instant::now()));
                    if hover
                        .link
                        .as_ref()
                        .is_some_and(|l| !l.range.contains(position))
                    {
                        hover.link = None;
                        hover.info = None;
                    }
                }
                EditorIntent::HoverEnd => {
                    let hover = &mut self.nav.hover;
                    hover.current = None;
                    hover.pending = None;
                    hover.inflight = None;
                    hover.link = None;
                    hover.info = None;
                }
            }
        }
    }

    /// Texte de l'éditeur actif tel que l'index doit le lire (copié une fois par version).
    fn nav_text(&mut self) -> Option<(PathBuf, Arc<str>)> {
        let editor = self.editors.active_editor()?;
        let path = editor.path()?.to_path_buf();
        let version = editor.document.buffer().version();
        if let Some((cached_path, cached_version, text)) = &self.nav.text_cache {
            if *cached_path == path && *cached_version == version {
                return Some((path, text.clone()));
            }
        }
        let text: Arc<str> = Arc::from(editor.document.text().as_str());
        self.nav.text_cache = Some((path.clone(), version, text.clone()));
        Some((path, text))
    }

    /// Demande à l'index ce qu'il y a sous `position` (clic, Ctrl+B, Alt+F7).
    pub(super) fn nav_request(&mut self, purpose: Purpose, position: TextPosition, anchor: Pos2) {
        if !self.nav_available() {
            return;
        }
        let Some((path, text)) = self.nav_text() else {
            return;
        };
        let Some(project) = &self.project else {
            return;
        };
        let handle = project.index.navigate(NavigationRequest {
            path: path.clone(),
            text,
            position,
        });
        self.nav.hover.inflight = None;
        self.nav.popup = None;
        self.nav.click = Some(ClickRequest {
            task: handle.id(),
            purpose,
            anchor,
        });
    }

    /// Le menu contextuel et Alt+F7 agissent au curseur.
    pub(super) fn nav_at_cursor(&mut self, purpose: Purpose) {
        let Some(editor) = self.editors.active_editor() else {
            return;
        };
        let position = editor.document.cursor_position();
        let anchor = self
            .editors
            .active_view()
            .and_then(|view| view.point_at(position))
            .unwrap_or_else(|| self.ctx.content_rect().center());
        self.nav_request(purpose, position, anchor);
    }

    fn send_hover(&mut self, position: TextPosition) {
        if !self.nav_available() {
            return;
        }
        let Some((path, text)) = self.nav_text() else {
            return;
        };
        let Some(project) = &self.project else {
            return;
        };
        let handle = project.index.navigate(NavigationRequest {
            path: path.clone(),
            text,
            position,
        });
        self.nav.hover.inflight = Some(Inflight {
            task: handle.id(),
            path,
            position,
            sent: Instant::now(),
        });
    }

    /// À chaque image : anti-rebond du survol, texte non enregistré, infobulle.
    pub(super) fn nav_tick(&mut self, ctx: &egui::Context) {
        if let Some((position, since)) = self.nav.hover.pending {
            let elapsed = since.elapsed();
            if elapsed >= HOVER_DEBOUNCE {
                self.nav.hover.pending = None;
                // Une demande de clic en vol ne doit pas être annulée par un survol.
                if self.nav.click.is_none() {
                    self.send_hover(position);
                }
            } else {
                ctx.request_repaint_after(HOVER_DEBOUNCE - elapsed);
            }
        }
        if let (Some(_), Some((_, since))) = (&self.nav.hover.link, self.nav.hover.current) {
            if let Some(left) = HOVER_TOOLTIP_AFTER.checked_sub(since.elapsed()) {
                ctx.request_repaint_after(left);
            }
        }
        self.sync_unsaved(ctx);
    }

    // ----- Texte non enregistré -----------------------------------------------------

    /// `set_unsaved(chemin, Some(texte))` après une pause de frappe ; `None` dès que
    /// l'onglet n'est plus modifié (enregistré, annulé) ou fermé.
    fn sync_unsaved(&mut self, ctx: &egui::Context) {
        let Some(project) = &self.project else {
            return;
        };
        let now = Instant::now();
        let mut open: HashSet<PathBuf> = HashSet::new();
        for editor in &self.editors.items {
            let Some(path) = editor.path() else {
                continue;
            };
            open.insert(path.to_path_buf());
            let modified = editor.document.is_modified();
            let version = editor.document.buffer().version();
            let known = self.nav.unsaved.contains_key(path);
            if !modified && !known {
                continue;
            }
            let entry = self
                .nav
                .unsaved
                .entry(path.to_path_buf())
                .or_insert(UnsavedSync {
                    seen_version: version,
                    seen_at: now,
                    sent_version: None,
                });
            if modified {
                if entry.seen_version != version {
                    entry.seen_version = version;
                    entry.seen_at = now;
                }
                if entry.sent_version != Some(version) {
                    let waited = now.duration_since(entry.seen_at);
                    if waited >= TYPING_PAUSE {
                        let text: Arc<str> = Arc::from(editor.document.text().as_str());
                        project.index.set_unsaved(path.to_path_buf(), Some(text));
                        entry.sent_version = Some(version);
                    } else {
                        ctx.request_repaint_after(TYPING_PAUSE - waited);
                    }
                }
            } else if entry.sent_version.take().is_some() {
                project.index.set_unsaved(path.to_path_buf(), None);
            }
        }
        // Onglets fermés : l'index relit le disque.
        let closed: Vec<PathBuf> = self
            .nav
            .unsaved
            .keys()
            .filter(|p| !open.contains(*p))
            .cloned()
            .collect();
        for path in closed {
            if let Some(entry) = self.nav.unsaved.remove(&path) {
                if entry.sent_version.is_some() {
                    project.index.set_unsaved(path, None);
                }
            }
        }
        self.nav
            .unsaved
            .retain(|_, e| e.sent_version.is_some() || e.seen_at.elapsed().as_secs() < 3600);
    }

    /// Un onglet modifié n'a pas encore été donné à l'index dans sa version actuelle.
    pub(super) fn unsaved_pending(&self) -> bool {
        self.editors.items.iter().any(|editor| {
            let Some(path) = editor.path() else {
                return false;
            };
            let version = editor.document.buffer().version();
            let sent = self.nav.unsaved.get(path).and_then(|e| e.sent_version);
            if editor.document.is_modified() {
                sent != Some(version)
            } else {
                sent.is_some()
            }
        })
    }

    // ----- Réponses de l'index --------------------------------------------------------

    pub(super) fn on_navigation(&mut self, task: Option<TaskId>, result: NavigationResult) {
        let Some(task) = task else {
            return;
        };
        if self.nav.click.as_ref().is_some_and(|c| c.task == task) {
            if let Some(request) = self.nav.click.take() {
                self.on_click_result(request, result);
            }
        } else if self
            .nav
            .hover
            .inflight
            .as_ref()
            .is_some_and(|h| h.task == task)
        {
            if let Some(inflight) = self.nav.hover.inflight.take() {
                self.on_hover_result(inflight, result);
            }
        }
        // Toute autre réponse est périmée : une demande plus récente l'a remplacée.
    }

    fn on_hover_result(&mut self, inflight: Inflight, result: NavigationResult) {
        let still_there = self.nav.hover.current.map(|(p, _)| p) == Some(inflight.position);
        let fast = inflight.sent.elapsed() <= HOVER_MAX_LATENCY;
        let navigable = matches!(
            result.navigation,
            Navigation::Targets(_) | Navigation::Declaration(_) | Navigation::OpenFile(_)
        );
        if !(still_there && fast && navigable) {
            self.nav.hover.link = None;
            self.nav.hover.info = None;
            return;
        }
        self.nav.hover.link = result.word.map(|range| Link {
            path: inflight.path.clone(),
            range,
        });
        self.nav.hover.info = match &result.navigation {
            Navigation::Targets(targets) => targets.targets.first().map(|d| {
                if d.container.is_empty() {
                    d.line_text.clone()
                } else {
                    format!("{} — {}", d.line_text, d.container)
                }
            }),
            _ => None,
        };
    }

    fn on_click_result(&mut self, request: ClickRequest, result: NavigationResult) {
        let indexing = result.indexing;
        match result.navigation {
            Navigation::Targets(targets) => {
                let single = targets.targets.len() == 1 && targets.guess.is_none();
                match request.purpose {
                    Purpose::Jump if single && targets.constructor.is_none() => {
                        self.nav_jump(targets.targets[0].clone());
                    }
                    Purpose::Usages if single => {
                        self.start_usages(targets.targets[0].clone(), false, false);
                    }
                    purpose => {
                        let name = targets.targets[0].name.clone();
                        let guessed = targets.guess.is_some();
                        let title = if purpose == Purpose::Usages && !guessed {
                            t::nav_usages_pick_title(&name)
                        } else {
                            t::nav_list_title(targets.guess.as_ref(), &name)
                        };
                        let mut items: Vec<PopupItem> = targets
                            .targets
                            .into_iter()
                            .map(|decl| PopupItem {
                                decl,
                                constructor: false,
                            })
                            .collect();
                        if purpose == Purpose::Jump {
                            if let Some(decl) = targets.constructor {
                                items.push(PopupItem {
                                    decl,
                                    constructor: true,
                                });
                            }
                        }
                        self.nav.popup = Some(TargetPopup::new(
                            title,
                            items,
                            request.anchor,
                            if purpose == Purpose::Jump {
                                PopupPurpose::Jump
                            } else {
                                PopupPurpose::Usages
                            },
                            guessed,
                        ));
                    }
                }
            }
            Navigation::Declaration(declaration) => {
                self.start_usages(declaration, request.purpose == Purpose::Jump, false);
            }
            Navigation::OpenFile(path) => {
                self.nav_record_origin();
                self.open_file(path, None);
            }
            Navigation::Nothing(why) => self.nav_nothing(why, indexing),
        }
    }

    fn nav_nothing(&mut self, why: NoTarget, indexing: Option<u8>) {
        let text = match why {
            NoTarget::NotASymbol => return,
            NoTarget::NotInProject { .. } | NoTarget::NotFound { .. } if indexing.is_some() => {
                t::nav_indexing(indexing.unwrap_or(0))
            }
            NoTarget::NotInProject { name } => t::nav_not_in_project(&name),
            NoTarget::NotFound { name } => t::nav_not_found(&name),
            NoTarget::DynamicName => t::NAV_DYNAMIC_NAME.to_owned(),
            NoTarget::ExternalModule { specifier } => t::nav_external_module(&specifier),
        };
        self.say(text, false);
    }

    // ----- Sauts et historique ------------------------------------------------------------

    fn nav_current_location(&self) -> Option<Location> {
        let editor = self.editors.active_editor()?;
        Some(Location {
            path: editor.path()?.to_path_buf(),
            position: editor.document.cursor_position(),
        })
    }

    fn nav_record_origin(&mut self) {
        if let Some(from) = self.nav_current_location() {
            self.nav.history.push(from);
        }
    }

    pub(super) fn nav_jump(&mut self, declaration: Declaration) {
        self.nav.popup = None;
        self.nav_record_origin();
        self.open_file(
            declaration.path,
            Some(Goto {
                position: declaration.range.start,
                end: None,
            }),
        );
    }

    fn nav_open_location(&mut self, location: Location) {
        self.open_file(
            location.path,
            Some(Goto {
                position: location.position,
                end: None,
            }),
        );
    }

    pub(super) fn nav_back(&mut self) {
        let current = self.nav_current_location();
        if let Some(target) = self.nav.history.go_back(current) {
            self.nav_open_location(target);
        }
    }

    pub(super) fn nav_forward(&mut self) {
        let current = self.nav_current_location();
        if let Some(target) = self.nav.history.go_forward(current) {
            self.nav_open_location(target);
        }
    }

    /// Boutons précédent / suivant de la souris.
    pub(super) fn nav_mouse_buttons(&mut self, ctx: &egui::Context) {
        let (back, forward) = ctx.input(|i| {
            (
                i.pointer.button_pressed(egui::PointerButton::Extra1),
                i.pointer.button_pressed(egui::PointerButton::Extra2),
            )
        });
        if back && self.nav.history.can_back() {
            self.nav_back();
        }
        if forward && self.nav.history.can_forward() {
            self.nav_forward();
        }
    }

    // ----- Liste surgissante -----------------------------------------------------------------

    /// Avant de dessiner l'éditeur : la liste prend le clavier.
    pub(super) fn nav_popup_keys(&mut self, ctx: &egui::Context) {
        let Some(popup) = &mut self.nav.popup else {
            return;
        };
        let outcome = popup.handle_keys(ctx);
        self.on_popup_outcome(outcome);
    }

    pub(super) fn draw_nav_popup(&mut self, ctx: &egui::Context, p: &Palette) {
        let Some(popup) = &mut self.nav.popup else {
            return;
        };
        let outcome = popup.show(ctx, p);
        self.on_popup_outcome(outcome);
    }

    fn on_popup_outcome(&mut self, outcome: PopupOutcome) {
        match outcome {
            PopupOutcome::Pending => {}
            PopupOutcome::Close => {
                self.nav.popup = None;
                self.focus_editor();
            }
            PopupOutcome::Choose(item) => {
                let purpose = self.nav.popup.as_ref().map(|p| p.purpose);
                self.nav.popup = None;
                match purpose {
                    Some(PopupPurpose::Usages) => self.start_usages(item.decl, false, false),
                    _ => self.nav_jump(item.decl),
                }
            }
        }
    }

    /// Infobulle de déclaration sous le pointeur (Ctrl enfoncé, immobile depuis 500 ms).
    pub(super) fn draw_hover_tooltip(&mut self, ctx: &egui::Context, p: &Palette) {
        let (Some(info), Some(_), Some((_, since))) = (
            &self.nav.hover.info,
            &self.nav.hover.link,
            self.nav.hover.current,
        ) else {
            return;
        };
        if since.elapsed() < HOVER_TOOLTIP_AFTER {
            return;
        }
        let Some(pointer) = ctx.input(|i| i.pointer.hover_pos()) else {
            return;
        };
        egui::Area::new(Id::unique("nav-hover-tooltip"))
            .order(egui::Order::Tooltip)
            .fixed_pos(pointer + vec2(12.0, 18.0))
            .interactable(false)
            .show(ctx, |ui| {
                Frame::new()
                    .fill(p.bg_raised)
                    .stroke(egui::Stroke::new(1.0, p.border))
                    .corner_radius(6)
                    .inner_margin(Margin::symmetric(8, 5))
                    .show(ui, |ui| {
                        ui.label(
                            egui::RichText::new(info.as_str())
                                .monospace()
                                .size(12.5)
                                .color(p.text),
                        );
                    });
            });
    }

    // ----- Usages ---------------------------------------------------------------------------

    pub(super) fn reveal_usages(&mut self) {
        self.tools.bottom = BottomTool::Usages;
        self.tools.bottom_open = true;
    }

    pub(super) fn start_usages(
        &mut self,
        symbol: Declaration,
        jump_if_single: bool,
        include_libraries: bool,
    ) {
        let Some(project) = &self.project else {
            return;
        };
        let handle = project.index.find_usages(UsagesQuery {
            symbol: symbol.clone(),
            include_libraries,
        });
        self.nav.popup = None;
        self.usages
            .start(symbol, include_libraries, Some(handle.id()), jump_if_single);
        if !jump_if_single {
            self.reveal_usages();
        }
    }

    pub(super) fn on_usages(&mut self, task: Option<TaskId>, batch: Vec<Usage>) {
        let Some(index) = self
            .usages
            .tabs
            .iter()
            .position(|t| t.task == task && task.is_some())
        else {
            return;
        };
        let tab = &mut self.usages.tabs[index];
        tab.push(batch);
        if !tab.revealed && tab.usages.len() >= 2 {
            tab.revealed = true;
            self.usages.active = index;
            self.reveal_usages();
        }
    }

    pub(super) fn on_usages_finished(&mut self, task: Option<TaskId>, summary: UsagesSummary) {
        let Some(index) = self
            .usages
            .tabs
            .iter()
            .position(|t| t.task == task && task.is_some())
        else {
            return;
        };
        let tab = &mut self.usages.tabs[index];
        tab.finish(summary);
        if tab.revealed {
            return;
        }
        // Ctrl+clic sur une déclaration : un seul usage → on y va ; aucun → un message.
        match tab.usages.len() {
            0 => {
                self.usages.close(index);
                self.say(t::NAV_NO_USAGES, false);
            }
            1 => {
                let usage = tab.usages[0].clone();
                self.usages.close(index);
                self.open_usage_at(&usage);
            }
            _ => {
                tab.revealed = true;
                self.usages.active = index;
                self.reveal_usages();
            }
        }
    }

    fn open_usage_at(&mut self, usage: &Usage) {
        self.nav_record_origin();
        self.open_file(
            usage.path.clone(),
            Some(Goto {
                position: usage.range.start,
                end: Some(usage.range.end),
            }),
        );
    }

    pub(super) fn usages_intents(&mut self, intents: Vec<UsagesIntent>) {
        for intent in intents {
            match intent {
                UsagesIntent::Open(index) => {
                    let usage = self
                        .usages
                        .active_tab()
                        .and_then(|tab| tab.usages.get(index).cloned());
                    if let Some(usage) = usage {
                        self.open_usage_at(&usage);
                    }
                }
                UsagesIntent::Relaunch { include_libraries } => {
                    let symbol = self.usages.active_tab().map(|tab| tab.symbol.clone());
                    if let (Some(symbol), Some(project)) = (symbol, &self.project) {
                        let handle = project.index.find_usages(UsagesQuery {
                            symbol,
                            include_libraries,
                        });
                        self.usages.restart_active(include_libraries, handle.id());
                    }
                }
                UsagesIntent::Stop => {
                    if let Some(project) = &self.project {
                        project.index.cancel_usages();
                    }
                    if let Some(tab) = self.usages.active_tab_mut() {
                        tab.running = false;
                        tab.interrupted = true;
                        tab.task = None;
                    }
                }
                UsagesIntent::CloseTab(index) => {
                    let running = self.usages.tabs.get(index).is_some_and(|t| t.running);
                    if running {
                        if let Some(project) = &self.project {
                            project.index.cancel_usages();
                        }
                    }
                    self.usages.close(index);
                    if self.usages.is_empty() {
                        self.tools.bottom_open = false;
                        self.focus_editor();
                    }
                }
            }
        }
    }

    /// Ctrl+Alt+Bas / Haut : usage suivant / précédent de l'onglet affiché.
    pub(super) fn usage_step(&mut self, delta: isize) {
        let selected = self.usages.active_tab_mut().and_then(|tab| tab.step(delta));
        if let Some(index) = selected {
            let usage = self
                .usages
                .active_tab()
                .and_then(|tab| tab.usages.get(index).cloned());
            if let Some(usage) = usage {
                // Parcourir les usages n'empile pas l'historique à chaque pas.
                self.open_file(
                    usage.path.clone(),
                    Some(Goto {
                        position: usage.range.start,
                        end: Some(usage.range.end),
                    }),
                );
            }
        }
    }

    // ----- État pour le mode scénario ----------------------------------------------------------

    pub(super) fn nav_state(&self) -> serde_json::Value {
        let hover = &self.nav.hover;
        let (back, forward) = self.nav.history.counts();
        serde_json::json!({
            "link": hover.link.as_ref().map(|l| serde_json::json!({
                "path": l.path.display().to_string(),
                "range": format!(
                    "{}:{}-{}:{}",
                    l.range.start.line + 1, l.range.start.column + 1,
                    l.range.end.line + 1, l.range.end.column + 1
                ),
            })),
            "hover_info": hover.info,
            "popup": self.nav.popup.as_ref().map(TargetPopup::state),
            "history": { "back": back, "forward": forward },
            "indexing": self.project.as_ref().and_then(|p| p.index.symbols_progress()),
            "unsaved_sent": self.nav.unsaved.iter()
                .filter(|(_, e)| e.sent_version.is_some())
                .map(|(p, _)| p.display().to_string()).collect::<Vec<_>>(),
        })
    }

    pub(super) fn usages_state(&self) -> serde_json::Value {
        serde_json::json!({
            "active": self.usages.active,
            "tabs": self.usages.tabs.iter().map(|t| t.state()).collect::<Vec<_>>(),
        })
    }

    /// Les étapes `ui nav.*` et `ui usages.*` : un vrai clic ou une vraie touche, aux
    /// coordonnées d'un caractère du fichier actif (lignes et colonnes à partir de 1).
    pub(super) fn nav_ui_command(
        &mut self,
        command: &str,
        arg: &str,
    ) -> Option<Result<(), String>> {
        Some(match command {
            "nav.ctrl-click" | "nav.click" | "nav.rclick" => (|| {
                let pos = self.char_point(arg)?;
                let (button, modifiers) = match command {
                    "nav.ctrl-click" => (egui::PointerButton::Primary, ctrl_modifiers()),
                    "nav.rclick" => (egui::PointerButton::Secondary, Modifiers::NONE),
                    _ => (egui::PointerButton::Primary, Modifiers::NONE),
                };
                self.injected
                    .extend(Injection::click_with(pos, button, modifiers));
                Ok(())
            })(),
            "nav.hover" => (|| {
                let pos = self.char_point(arg)?;
                self.injected.push_back(Injection {
                    events: vec![
                        Event::ModifiersChanged(ctrl_modifiers()),
                        Event::PointerMoved(pos),
                    ],
                });
                self.injected.push_back(Injection {
                    events: vec![Event::PointerMoved(pos)],
                });
                Ok(())
            })(),
            "nav.hover-off" => {
                self.injected.push_back(Injection {
                    events: vec![Event::ModifiersChanged(Modifiers::NONE), Event::PointerGone],
                });
                Ok(())
            }
            "nav.mouse" => {
                let button = match arg {
                    "back" | "precedent" => egui::PointerButton::Extra1,
                    "forward" | "suivant" => egui::PointerButton::Extra2,
                    _ => return Some(Err("`nav.mouse back|forward`".to_owned())),
                };
                let pos = self.ctx.content_rect().center();
                let event = |pressed| Event::PointerButton {
                    pos,
                    button,
                    pressed,
                    modifiers: Modifiers::NONE,
                };
                self.injected.push_back(Injection {
                    events: vec![Event::PointerMoved(pos)],
                });
                self.injected.push_back(Injection {
                    events: vec![event(true)],
                });
                self.injected.push_back(Injection {
                    events: vec![event(false)],
                });
                Ok(())
            }
            "nav.choose" => (|| {
                let popup = self
                    .nav
                    .popup
                    .as_mut()
                    .ok_or("aucune liste surgissante ouverte")?;
                let visible = popup.visible();
                let wanted = match arg.parse::<usize>() {
                    Ok(n) if n >= 1 => Some(n - 1),
                    _ => {
                        let needle = arg.to_lowercase();
                        visible.iter().position(|&i| {
                            popup.items[i]
                                .decl
                                .qualified_name
                                .to_lowercase()
                                .contains(&needle)
                        })
                    }
                };
                let wanted = wanted
                    .filter(|&n| n < visible.len())
                    .ok_or_else(|| format!("aucune ligne ne correspond à « {arg} »"))?;
                popup.select(wanted);
                // La validation passe par la vraie touche Entrée.
                self.injected.push_back(Injection {
                    events: crate::scenario::parse_combo("Enter")
                        .map(|c| c.events())
                        .unwrap_or_default(),
                });
                Ok(())
            })(),
            "usages.select" => (|| {
                let n: usize = arg.parse().map_err(|_| "numéro attendu".to_owned())?;
                let tab = self.usages.active_tab_mut().ok_or("aucun onglet Usages")?;
                let order = tab.ordered();
                let usage = order
                    .get(n.checked_sub(1).ok_or("numéro à partir de 1")?)
                    .copied()
                    .ok_or_else(|| format!("seulement {} usages", order.len()))?;
                tab.select(Some(usage));
                Ok(())
            })(),
            "usages.open" => (|| {
                let selected = self
                    .usages
                    .active_tab()
                    .and_then(|t| t.selected)
                    .ok_or("aucun usage sélectionné")?;
                self.usages_intents(vec![UsagesIntent::Open(selected)]);
                Ok(())
            })(),
            "usages.include" => {
                let on = matches!(arg, "on" | "oui" | "true");
                self.usages_intents(vec![UsagesIntent::Relaunch {
                    include_libraries: on,
                }]);
                Ok(())
            }
            "usages.stop" => {
                self.usages_intents(vec![UsagesIntent::Stop]);
                Ok(())
            }
            "usages.close" => {
                let index = self.usages.active;
                self.usages_intents(vec![UsagesIntent::CloseTab(index)]);
                Ok(())
            }
            "usages.tab" => (|| {
                let n: usize = arg.parse().map_err(|_| "numéro attendu".to_owned())?;
                if n == 0 || n > self.usages.tabs.len() {
                    return Err("onglet Usages absent".to_owned());
                }
                self.usages.active = n - 1;
                Ok(())
            })(),
            _ => return None,
        })
    }

    /// Point de l'écran au milieu du caractère `ligne:colonne` (base 1) du fichier actif.
    fn char_point(&self, arg: &str) -> Result<Pos2, String> {
        let mut parts = arg.split(':').map(|n| n.trim().parse::<u32>());
        let (Some(Ok(line)), Some(Ok(column))) = (parts.next(), parts.next()) else {
            return Err("`ligne:colonne` attendu (base 1), par exemple 12:7".to_owned());
        };
        let view = self.editors.active_view().ok_or("aucun éditeur ouvert")?;
        let position = TextPosition::new(line.saturating_sub(1), column.saturating_sub(1));
        let bottom_left = view
            .point_at(position)
            .ok_or("éditeur pas encore affiché")?;
        let layout = view.layout().ok_or("éditeur pas encore affiché")?;
        let pos = Pos2::new(
            bottom_left.x + layout.char_width / 2.0,
            bottom_left.y - layout.line_height / 2.0,
        );
        if layout.text_rect.contains(pos) {
            Ok(pos)
        } else {
            Err(format!(
                "{line}:{column} n'est pas visible : ouvrir le fichier avec `file chemin:{line}:{column}` d'abord"
            ))
        }
    }

    /// Un autre projet s'ouvre : l'ancien garde son cache de symboles, les états liés
    /// à l'ancien index sont jetés.
    pub(super) fn nav_reset_for_project(&mut self) {
        if let Some(old) = &self.project {
            old.index.cancel_usages();
            old.index.save_symbol_cache();
        }
        self.nav = NavState::default();
        self.usages.clear();
        if self.tools.bottom == BottomTool::Usages {
            self.tools.bottom = BottomTool::Git;
        }
    }
}
