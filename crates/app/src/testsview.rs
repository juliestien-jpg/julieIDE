//! Vue des tests (onglet « Tests » de la fenêtre du bas) : arbre de résultats en
//! direct à gauche, console du nœud choisi à droite, barre de progression verte
//! ou rouge, compteurs, liens `fichier:ligne` cliquables, comparaison
//! attendu / obtenu.
//!
//! Aucune règle PHPUnit ici : l'arbre (`ResultTree`) et les états viennent de
//! `julie-phpunit`, qui analyse la sortie TeamCity ; cette vue les dessine.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui::{
    self, pos2, vec2, Align2, Color32, EventFilter, FontId, Id, Key, Modifiers, Rect, RichText,
    Sense, Ui,
};
use julie_core::process::Stream;
use julie_core::{Location, TaskId};
use julie_phpunit::{
    find_links, Detection, NodeId, NodeKind, PhpUnit, PhpUnitEvent, Problem, ResultTree,
    RunOutcome, RunStart, RunSummary, TestNode, TestState,
};

use crate::icons::{self, Icon};
use crate::kit::{self, Look};
use crate::texts as t;
use crate::theme::Palette;

const ROW: f32 = 22.0;
const TOOLBAR: f32 = 34.0;
const PROGRESS: f32 = 40.0;
const MAX_CONSOLE_LINES: usize = 5000;

/// Ce que la vue demande à l'application.
#[derive(Debug)]
pub enum TestsIntent {
    Run,
    /// Arrêter l'exécution en cours puis relancer (après confirmation).
    Restart,
    Stop,
    Open(Location),
    Compare {
        title: String,
        expected: String,
        actual: String,
    },
}

struct RunView {
    tree: ResultTree,
    #[allow(dead_code)]
    start: RunStart,
    summary: Option<Box<RunSummary>>,
    started: Instant,
}

#[derive(Clone)]
struct Seg {
    text: String,
    color: Color32,
    link: Option<Location>,
    strong: bool,
}

#[derive(Clone, Default)]
struct CLine {
    segs: Vec<Seg>,
}

pub struct TestsView {
    pub service: Option<PhpUnit>,
    root: PathBuf,
    detection: Option<Box<Detection>>,
    detect_requested: bool,
    problem: Option<Problem>,
    run: Option<RunView>,
    pub run_task: Option<TaskId>,
    pub selected: NodeId,
    toggled: HashMap<NodeId, bool>,
    pub show_passed: bool,
    exists: HashMap<PathBuf, bool>,
    console_key: Option<(NodeId, u64, bool)>,
    console: Vec<CLine>,
    list_focus: Id,
    pub confirm_restart: bool,
    /// Liens de la console affichée (pour le mode scénario).
    links: Vec<Location>,
}

impl TestsView {
    pub fn new(root: PathBuf, service: Option<PhpUnit>) -> Self {
        Self {
            service,
            root,
            detection: None,
            detect_requested: false,
            problem: None,
            run: None,
            run_task: None,
            selected: NodeId::ROOT,
            toggled: HashMap::new(),
            show_passed: true,
            exists: HashMap::new(),
            console_key: None,
            console: Vec::new(),
            list_focus: Id::unique("tests-liste"),
            confirm_restart: false,
            links: Vec::new(),
        }
    }

    pub fn is_running(&self) -> bool {
        self.run.as_ref().is_some_and(|r| r.summary.is_none()) && self.run_task.is_some()
    }

    pub fn has_run(&self) -> bool {
        self.run.is_some()
    }

    /// Lance la détection (PHP, PHPUnit, configuration) la première fois que la
    /// vue est montrée.
    pub fn ensure_detected(&mut self) {
        if !self.detect_requested {
            self.detect_requested = true;
            if let Some(service) = &self.service {
                service.detect();
            }
        }
    }

    // ----- Événements ------------------------------------------------------------

    pub fn on_event(&mut self, task: Option<TaskId>, event: &PhpUnitEvent) {
        match event {
            PhpUnitEvent::Detected(detection) => self.detection = Some(detection.clone()),
            other => {
                // Un événement d'une exécution périmée est écarté.
                if task.is_none() || task != self.run_task {
                    return;
                }
                match other {
                    PhpUnitEvent::LaunchFailed(problem) => {
                        self.problem = Some(problem.clone());
                        self.run = None;
                        self.run_task = None;
                    }
                    PhpUnitEvent::RunStarted(start) => {
                        self.problem = None;
                        self.run = Some(RunView {
                            tree: ResultTree::new(self.root.clone()),
                            start: (**start).clone(),
                            summary: None,
                            started: Instant::now(),
                        });
                        self.selected = NodeId::ROOT;
                        self.toggled.clear();
                        self.console_key = None;
                    }
                    PhpUnitEvent::Events(events) => {
                        if let Some(run) = &mut self.run {
                            run.tree.apply_all(events);
                        }
                    }
                    PhpUnitEvent::RunFinished(summary) => {
                        if let Some(run) = &mut self.run {
                            run.summary = Some(summary.clone());
                        }
                    }
                    PhpUnitEvent::Detected(_) => {}
                }
            }
        }
    }

    /// Une nouvelle exécution commence : on repart de zéro.
    pub fn begin(&mut self, task: TaskId) {
        self.run_task = Some(task);
        self.problem = None;
        self.confirm_restart = false;
    }

    // ----- Arbre ----------------------------------------------------------------

    fn expanded(&self, node: &TestNode) -> bool {
        if let Some(value) = self.toggled.get(&node.id) {
            return *value;
        }
        node.kind == NodeKind::Root
            || node.worst.is_failure()
            || node.state.is_failure()
            || node.state == TestState::Running
    }

    fn visible(&self, node: &TestNode) -> bool {
        self.show_passed
            || node.kind == NodeKind::Root
            || node.state != TestState::Passed
            || !node.finished && !node.is_test
    }

    /// Les lignes affichées : (nœud, profondeur).
    fn rows(&self, tree: &ResultTree) -> Vec<(NodeId, u16)> {
        let mut rows = Vec::new();
        let mut stack = vec![(tree.root(), 0u16)];
        while let Some((id, depth)) = stack.pop() {
            let node = tree.node(id);
            if node.kind != NodeKind::Root && !self.visible(node) {
                continue;
            }
            rows.push((id, depth));
            if !node.children.is_empty() && self.expanded(node) {
                for child in tree.children(id).iter().rev() {
                    stack.push((*child, depth + 1));
                }
            }
        }
        rows
    }

    fn expand_to(&mut self, tree_path: Vec<NodeId>) {
        for id in tree_path {
            self.toggled.insert(id, true);
        }
    }

    /// Sélectionne le premier nœud dont le libellé contient `text` (ou `racine`).
    pub fn select_label(&mut self, text: &str) -> bool {
        let Some(run) = &self.run else { return false };
        if text == "racine" || text == "root" {
            self.selected = NodeId::ROOT;
            return true;
        }
        let wanted = text.to_lowercase();
        let found = run
            .tree
            .iter()
            .find(|n| n.kind != NodeKind::Root && n.label.to_lowercase().contains(&wanted))
            .map(|n| n.id);
        match found {
            Some(id) => {
                let path = run.tree.path_of(id);
                self.expand_to(path);
                self.selected = id;
                true
            }
            None => false,
        }
    }

    pub fn toggle_selected(&mut self) {
        if let Some(run) = &self.run {
            let node = run.tree.node(self.selected);
            let now = self.expanded(node);
            self.toggled.insert(self.selected, !now);
        }
    }

    /// Le lien numéro `n` (base 1) de la console affichée.
    pub fn link(&self, n: usize) -> Option<Location> {
        self.links.get(n.checked_sub(1)?).cloned()
    }

    /// Ce qu'il faut comparer pour le nœud choisi.
    pub fn compare_intent(&self) -> Option<TestsIntent> {
        let run = self.run.as_ref()?;
        let node = run.tree.node(self.selected);
        let comparison = node.diagnostic.as_ref()?.comparison.as_ref()?;
        Some(TestsIntent::Compare {
            title: node.label.clone(),
            expected: comparison.expected_text(),
            actual: comparison.actual_text(),
        })
    }

    // ----- Affichage ----------------------------------------------------------------

    pub fn show(
        &mut self,
        ui: &mut Ui,
        p: &Palette,
        run_shortcut: &str,
        stop_shortcut: &str,
    ) -> Vec<TestsIntent> {
        let mut out = Vec::new();
        let rect = ui.available_rect_before_wrap();
        ui.allocate_rect(rect, Sense::hover());
        self.ensure_detected();
        let toolbar = Rect::from_min_size(rect.min, vec2(rect.width(), TOOLBAR));
        self.show_toolbar(ui, toolbar, p, run_shortcut, stop_shortcut, &mut out);
        let top = toolbar.bottom();
        let Some(run) = self.run.take() else {
            let body = Rect::from_min_max(pos2(rect.left(), top), rect.max);
            self.show_idle(ui, body, p);
            self.show_modals(ui.ctx(), p, &mut out);
            return out;
        };
        let progress = Rect::from_min_size(pos2(rect.left(), top), vec2(rect.width(), PROGRESS));
        self.show_progress(ui, progress, p, &run);
        let body = Rect::from_min_max(pos2(rect.left(), progress.bottom()), rect.max);
        let left_width = (body.width() * 0.46)
            .clamp(260.0, 640.0)
            .min(body.width() - 200.0);
        let left = Rect::from_min_size(body.min, vec2(left_width, body.height()));
        let right = Rect::from_min_max(pos2(left.right() + 1.0, body.top()), body.max);
        ui.painter().vline(
            left.right(),
            body.y_range(),
            egui::Stroke::new(1.0, p.border),
        );
        self.show_tree(ui, left, p, &run, &mut out);
        self.show_console(ui, right, p, &run, &mut out);
        if self.is_running() {
            ui.ctx().request_repaint_after(Duration::from_millis(500));
        }
        self.run = Some(run);
        self.show_modals(ui.ctx(), p, &mut out);
        out
    }

    fn show_toolbar(
        &mut self,
        ui: &mut Ui,
        rect: Rect,
        p: &Palette,
        run_shortcut: &str,
        stop_shortcut: &str,
        out: &mut Vec<TestsIntent>,
    ) {
        ui.painter().rect_filled(rect, 0.0, p.bg_alt);
        let cy = rect.center().y;
        let running = self.is_running();
        let button = |ui: &mut Ui,
                      x: f32,
                      id: &str,
                      icon: Icon,
                      color: Color32,
                      tip: String,
                      enabled: bool| {
            let r = Rect::from_center_size(pos2(x, cy), vec2(26.0, 26.0));
            let response = ui.interact(r, Id::unique(("tests-outil", id)), Sense::click());
            if enabled && response.hovered() {
                ui.painter().rect_filled(r, 5.0, p.accent_soft);
            }
            icons::paint(
                ui.painter(),
                r.shrink(5.0),
                icon,
                if enabled { color } else { p.ignored },
            );
            response.clone().on_hover_text(tip);
            enabled && response.clicked()
        };
        if button(
            ui,
            rect.left() + 20.0,
            "lancer",
            Icon::Play,
            p.ok,
            t::tests_run_tip(run_shortcut),
            self.service.is_some(),
        ) {
            out.push(TestsIntent::Run);
        }
        if button(
            ui,
            rect.left() + 50.0,
            "arreter",
            Icon::Stop,
            p.error,
            t::tests_stop_tip(stop_shortcut),
            running,
        ) {
            out.push(TestsIntent::Stop);
        }
        if button(
            ui,
            rect.left() + 80.0,
            "relancer",
            Icon::Refresh,
            p.accent,
            t::TESTS_RERUN.to_owned(),
            self.service.is_some() && self.run.is_some(),
        ) {
            out.push(TestsIntent::Run);
        }
        ui.painter().vline(
            rect.left() + 102.0,
            (cy - 10.0)..=(cy + 10.0),
            egui::Stroke::new(1.0, p.border),
        );
        // Filtre : afficher les réussis.
        let label = t::TESTS_SHOW_PASSED;
        let w = kit::text_width(ui, label) + 20.0;
        let r = Rect::from_min_size(pos2(rect.left() + 112.0, cy - 13.0), vec2(w, 26.0));
        let response = ui.interact(r, Id::unique("tests-filtre-reussis"), Sense::click());
        if response.hovered() {
            ui.painter().rect_filled(r, 5.0, p.accent_soft);
        }
        kit::paint_checkbox(
            ui,
            Rect::from_min_size(r.min, vec2(24.0, r.height())),
            self.show_passed,
            false,
            p,
        );
        ui.painter().text(
            pos2(r.left() + 26.0, cy),
            Align2::LEFT_CENTER,
            label,
            FontId::proportional(13.0),
            p.text,
        );
        if response.clicked() {
            self.show_passed = !self.show_passed;
        }
        // À droite : versions détectées.
        if let Some(text) = self.versions_text() {
            ui.painter().text(
                pos2(rect.right() - 12.0, cy),
                Align2::RIGHT_CENTER,
                text,
                FontId::proportional(12.5),
                p.text_dim,
            );
        }
    }

    fn versions_text(&self) -> Option<String> {
        let detection = self.detection.as_ref()?;
        let php = detection.php.as_ref().ok()?;
        let phpunit = detection.phpunit.as_ref().ok()?;
        Some(t::tests_versions(
            php.version.as_deref(),
            phpunit.version.as_ref().map(|v| v.text.as_str()),
        ))
    }

    fn show_idle(&self, ui: &mut Ui, rect: Rect, p: &Palette) {
        let mut lines: Vec<(Color32, String)> = Vec::new();
        if let Some(problem) = &self.problem {
            lines.push((p.error, t::problem_text(problem)));
        }
        match &self.detection {
            None => lines.push((p.text_dim, t::TESTS_DETECTING.to_owned())),
            Some(detection) => {
                lines.push(match &detection.php {
                    Ok(php) => (p.ok, t::tests_php_ok(&php.path, php.version.as_deref())),
                    Err(problem) => (p.error, t::problem_text(problem)),
                });
                lines.push(match &detection.phpunit {
                    Ok(info) => (
                        p.ok,
                        t::tests_phpunit_ok(
                            &info.path,
                            info.version.as_ref().map(|v| v.text.as_str()),
                        ),
                    ),
                    Err(problem) => (p.error, t::problem_text(problem)),
                });
                lines.push(match &detection.configuration {
                    Ok(path) => (p.ok, t::tests_config_ok(path)),
                    Err(problem) => (p.error, t::problem_text(problem)),
                });
            }
        }
        // Le même problème n'est dit qu'une fois (détection et lancement raté).
        let mut seen = std::collections::HashSet::new();
        lines.retain(|(_, text)| seen.insert(text.clone()));
        let mut y = rect.top() + 22.0;
        ui.painter().text(
            pos2(rect.left() + 18.0, y),
            Align2::LEFT_CENTER,
            t::TESTS_IDLE,
            FontId::proportional(14.0),
            p.text,
        );
        y += 28.0;
        for (color, text) in lines {
            icons::paint(
                ui.painter(),
                Rect::from_center_size(pos2(rect.left() + 24.0, y), vec2(13.0, 13.0)),
                if color == p.ok {
                    Icon::Check
                } else if color == p.error {
                    Icon::Warning
                } else {
                    Icon::Dot
                },
                color,
            );
            let clip = ui.painter().with_clip_rect(rect);
            clip.text(
                pos2(rect.left() + 40.0, y),
                Align2::LEFT_CENTER,
                text,
                FontId::proportional(13.0),
                p.text,
            );
            y += 24.0;
        }
    }

    fn show_progress(&self, ui: &mut Ui, rect: Rect, p: &Palette, run: &RunView) {
        let counts = run.tree.counts();
        let failing = counts.failures() > 0
            || matches!(
                run.summary.as_ref().map(|s| s.outcome),
                Some(RunOutcome::Aborted { .. })
            );
        let stopped = matches!(
            run.summary.as_ref().map(|s| s.outcome),
            Some(RunOutcome::Stopped | RunOutcome::TimedOut)
        );
        let color = if failing {
            p.error
        } else if stopped {
            p.warn
        } else {
            p.ok
        };
        let elapsed = match &run.summary {
            Some(summary) => summary.duration,
            None => run.started.elapsed(),
        };
        let text = match &run.summary {
            None => t::tests_counts(counts, elapsed),
            Some(summary) => t::tests_summary(summary, elapsed),
        };
        ui.painter().text(
            pos2(rect.left() + 14.0, rect.top() + 12.0),
            Align2::LEFT_CENTER,
            text,
            FontId::proportional(13.0),
            if failing { p.error } else { p.text },
        );
        let bar = Rect::from_min_size(
            pos2(rect.left() + 14.0, rect.top() + 25.0),
            vec2(rect.width() - 28.0, 8.0),
        );
        ui.painter().rect_filled(bar, 4.0, p.bg_alt);
        let total = counts.total.unwrap_or(0).max(counts.finished());
        let fraction = if run.summary.is_some() && counts.started > 0 {
            1.0
        } else if total == 0 {
            0.0
        } else {
            (counts.finished() as f32 / total as f32).clamp(0.0, 1.0)
        };
        if fraction > 0.0 {
            let filled = Rect::from_min_size(
                bar.min,
                vec2((bar.width() * fraction).max(8.0), bar.height()),
            );
            ui.painter().rect_filled(filled, 4.0, color);
        }
    }

    fn show_tree(
        &mut self,
        ui: &mut Ui,
        rect: Rect,
        p: &Palette,
        run: &RunView,
        out: &mut Vec<TestsIntent>,
    ) {
        let tree = &run.tree;
        let rows = self.rows(tree);
        // Clavier.
        let focused = ui.memory(|m| m.has_focus(self.list_focus));
        ui.interact(rect, self.list_focus, Sense::click());
        let mut select: Option<NodeId> = None;
        let mut toggle: Option<(NodeId, bool)> = None;
        if focused {
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(
                    self.list_focus,
                    EventFilter {
                        vertical_arrows: true,
                        horizontal_arrows: true,
                        ..Default::default()
                    },
                )
            });
            let position = rows.iter().position(|(id, _)| *id == self.selected);
            let down = ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowDown));
            let up = ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowUp));
            let right = ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowRight));
            let left = ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowLeft));
            let enter = ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
            if let Some(i) = position {
                if down && i + 1 < rows.len() {
                    select = Some(rows[i + 1].0);
                } else if up && i > 0 {
                    select = Some(rows[i - 1].0);
                }
            }
            let node = tree.node(self.selected);
            if right && !node.children.is_empty() {
                toggle = Some((node.id, true));
            }
            if left && !node.children.is_empty() && self.expanded(node) {
                toggle = Some((node.id, false));
            } else if left {
                if let Some(parent) = node.parent {
                    select = Some(parent);
                }
            }
            if enter {
                if let Some(location) = node.jump_location() {
                    out.push(TestsIntent::Open(location));
                }
            }
        }
        let mut clicked: Option<NodeId> = None;
        let mut double: Option<NodeId> = None;
        kit::child(ui, rect, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            egui::ScrollArea::vertical()
                .id_salt("tests-arbre")
                .auto_shrink([false, false])
                .show_rows(ui, ROW, rows.len(), |ui, range| {
                    for i in range {
                        let (id, depth) = rows[i];
                        let node = tree.node(id);
                        let expanded = self.expanded(node);
                        let row = node_row(ui, p, node, depth, expanded, id == self.selected);
                        if row.chevron {
                            toggle = Some((id, !expanded));
                            clicked = Some(id);
                        } else if row.double {
                            double = Some(id);
                        } else if row.clicked {
                            clicked = Some(id);
                        }
                    }
                });
        });
        if let Some(id) = clicked.or(double) {
            ui.memory_mut(|m| m.request_focus(self.list_focus));
            select = Some(id);
        }
        if let Some((id, value)) = toggle {
            self.toggled.insert(id, value);
        }
        if let Some(id) = select {
            self.selected = id;
        }
        if let Some(id) = double {
            let node = tree.node(id);
            if node.children.is_empty() {
                if let Some(location) = node.jump_location() {
                    out.push(TestsIntent::Open(location));
                }
            } else {
                let now = self.expanded(node);
                self.toggled.insert(id, !now);
            }
        }
    }

    fn rebuild_console(&mut self, run: &RunView, p: &Palette) {
        let key = (self.selected, run.tree.version(), run.summary.is_some());
        if self.console_key == Some(key) {
            return;
        }
        self.console_key = Some(key);
        let tree = &run.tree;
        let node = tree.node(self.selected);
        let root = self.root.clone();
        let mut lines: Vec<CLine> = Vec::new();
        let cache = std::cell::RefCell::new(std::mem::take(&mut self.exists));
        let exists = |path: &Path| -> bool {
            *cache
                .borrow_mut()
                .entry(path.to_path_buf())
                .or_insert_with(|| path.is_file())
        };
        let plain = |text: &str, color: Color32| CLine {
            segs: vec![Seg {
                text: text.to_owned(),
                color,
                link: None,
                strong: false,
            }],
        };
        let linked = |text: &str, color: Color32| {
            let links = find_links(text, &root, exists);
            let mut segs = Vec::new();
            let mut at = 0;
            for link in links {
                if link.range.start > at {
                    segs.push(Seg {
                        text: text[at..link.range.start].to_owned(),
                        color,
                        link: None,
                        strong: false,
                    });
                }
                segs.push(Seg {
                    text: text[link.range.clone()].to_owned(),
                    color: p.accent,
                    link: Some(link.location),
                    strong: false,
                });
                at = link.range.end;
            }
            if at < text.len() || segs.is_empty() {
                segs.push(Seg {
                    text: text[at..].to_owned(),
                    color,
                    link: None,
                    strong: false,
                });
            }
            CLine { segs }
        };
        if node.kind == NodeKind::Root {
            if let Some(command) = tree.command() {
                lines.push(plain(&format!("$ {command}"), p.text_dim));
                lines.push(CLine::default());
            }
        } else {
            lines.push(CLine {
                segs: vec![Seg {
                    text: format!("{} — {}", node.label, t::test_state(node.state)),
                    color: state_color(node, p),
                    link: None,
                    strong: true,
                }],
            });
            if let Some(duration) = node.duration {
                lines.push(plain(&t::tests_duration(duration), p.text_dim));
            }
            lines.push(CLine::default());
        }
        if let Some(diagnostic) = &node.diagnostic {
            for text in diagnostic.message.lines() {
                lines.push(linked(text, p.error));
            }
            if let julie_phpunit::DiagnosticCause::ProcessExited { exit_code } = diagnostic.cause {
                lines.push(plain(&t::tests_process_exited(exit_code), p.error));
            }
            if let Some(comparison) = &diagnostic.comparison {
                lines.push(CLine::default());
                lines.push(plain(t::TESTS_EXPECTED, p.ok));
                for text in comparison.expected_text().lines() {
                    lines.push(plain(&format!("  {text}"), p.text));
                }
                lines.push(plain(t::TESTS_ACTUAL, p.error));
                for text in comparison.actual_text().lines() {
                    lines.push(plain(&format!("  {text}"), p.text));
                }
            }
            if !diagnostic.trace.is_empty() {
                lines.push(CLine::default());
                lines.push(plain(t::TESTS_TRACE, p.text_dim));
                for entry in &diagnostic.trace {
                    match &entry.location {
                        Some(location) => lines.push(CLine {
                            segs: vec![Seg {
                                text: entry.text.clone(),
                                color: p.accent,
                                link: Some(location.clone()),
                                strong: false,
                            }],
                        }),
                        None => lines.push(plain(&entry.text, p.text)),
                    }
                }
            }
        }
        if !node.console.is_empty() {
            if node.kind != NodeKind::Root {
                lines.push(CLine::default());
                lines.push(plain(t::TESTS_OUTPUT, p.text_dim));
            }
            for line in node.console.iter().take(MAX_CONSOLE_LINES) {
                let color = match line.stream {
                    Stream::Stdout => p.text,
                    Stream::Stderr => p.warn,
                };
                lines.push(linked(&line.text, color));
            }
        }
        if lines.is_empty() {
            lines.push(plain(t::TESTS_NO_OUTPUT, p.text_dim));
        }
        self.exists = cache.into_inner();
        self.links = lines
            .iter()
            .flat_map(|l| l.segs.iter().filter_map(|s| s.link.clone()))
            .collect();
        self.console = lines;
    }

    fn show_console(
        &mut self,
        ui: &mut Ui,
        rect: Rect,
        p: &Palette,
        run: &RunView,
        out: &mut Vec<TestsIntent>,
    ) {
        self.rebuild_console(run, p);
        let inner = rect.shrink2(vec2(10.0, 6.0));
        let node = run.tree.node(self.selected);
        let comparable = node
            .diagnostic
            .as_ref()
            .is_some_and(|d| d.comparison.is_some());
        let mut body = inner;
        if comparable {
            let button = Rect::from_min_size(
                pos2(inner.left(), inner.top()),
                vec2(kit::text_width(ui, t::TESTS_COMPARE) + 20.0, 26.0),
            );
            if kit::text_button_at(
                ui,
                p,
                button,
                Id::unique("tests-comparer"),
                t::TESTS_COMPARE,
                Look::Primary(p.accent),
                true,
                t::TESTS_COMPARE_TIP,
            ) {
                if let Some(intent) = self.compare_intent() {
                    out.push(intent);
                }
            }
            body.min.y += 32.0;
        }
        let mut open: Option<Location> = None;
        let lines = &self.console;
        kit::child(ui, body, |ui| {
            egui::ScrollArea::both()
                .id_salt("tests-console")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    for line in lines {
                        if line.segs.is_empty() {
                            ui.add_space(8.0);
                            continue;
                        }
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 0.0;
                            for seg in &line.segs {
                                let mut text = RichText::new(&seg.text)
                                    .monospace()
                                    .size(12.5)
                                    .color(seg.color);
                                if seg.strong {
                                    text = text.strong().size(14.0);
                                }
                                if seg.link.is_some() {
                                    let response = ui.add(
                                        egui::Label::new(text.underline())
                                            .sense(Sense::click())
                                            .extend(),
                                    );
                                    let response =
                                        response.on_hover_cursor(egui::CursorIcon::PointingHand);
                                    if response.clicked() {
                                        open = seg.link.clone();
                                    }
                                } else {
                                    ui.add(egui::Label::new(text).extend());
                                }
                            }
                        });
                    }
                });
        });
        if let Some(location) = open {
            out.push(TestsIntent::Open(location));
        }
    }

    fn show_modals(&mut self, ctx: &egui::Context, p: &Palette, out: &mut Vec<TestsIntent>) {
        if !self.confirm_restart {
            return;
        }
        let mut answer = None;
        kit::modal("tests-relance").show(ctx, |ui| {
            ui.set_width(420.0);
            ui.label(
                RichText::new(t::TESTS_RESTART_TITLE)
                    .strong()
                    .size(15.0)
                    .color(p.text),
            );
            ui.add_space(4.0);
            ui.label(RichText::new(t::TESTS_RESTART_DETAIL).color(p.text_dim));
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if kit::button(ui, p, t::TESTS_RESTART_YES, true, true).clicked() {
                    answer = Some(true);
                }
                if ui.button(t::DIALOG_CANCEL).clicked() {
                    answer = Some(false);
                }
            });
        });
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            answer = Some(false);
        }
        match answer {
            Some(true) => {
                self.confirm_restart = false;
                out.push(TestsIntent::Restart);
            }
            Some(false) => self.confirm_restart = false,
            None => {}
        }
    }

    // ----- État pour le mode scénario --------------------------------------------

    pub fn state(&self) -> serde_json::Value {
        let detection = self.detection.as_ref().map(|d| {
            serde_json::json!({
                "ready": d.is_ready(),
                "php": d.php.as_ref().map(|i| i.path.display().to_string()).map_err(|e| e.to_string()),
                "php_version": d.php.as_ref().ok().and_then(|i| i.version.clone()),
                "phpunit": d.phpunit.as_ref().map(|i| i.path.display().to_string()).map_err(|e| e.to_string()),
                "phpunit_version": d.phpunit.as_ref().ok().and_then(|i| i.version.as_ref().map(|v| v.text.clone())),
                "configuration": d.configuration.as_ref().map(|c| c.display().to_string()).map_err(|e| e.to_string()),
            })
        });
        let run = self.run.as_ref().map(|run| {
            let counts = run.tree.counts();
            let rows = self.rows(&run.tree);
            let node = run.tree.node(self.selected);
            serde_json::json!({
                "running": self.is_running(),
                "finished": run.summary.is_some(),
                "outcome": run.summary.as_ref().map(|s| format!("{:?}", s.outcome)),
                "green": run.summary.as_ref().map(|s| s.is_green()),
                "command": run.tree.command(),
                "counts": {
                    "total": counts.total, "started": counts.started, "passed": counts.passed,
                    "failed": counts.failed, "errors": counts.errors, "skipped": counts.skipped,
                    "incomplete": counts.incomplete, "risky": counts.risky,
                    "interrupted": counts.interrupted,
                },
                "summary_text": run.summary.as_ref().map(|s| t::tests_summary(s, s.duration)),
                "progress_color": if counts.failures() > 0 { "red" } else { "green" },
                "nodes": rows.iter().take(200).map(|(id, depth)| {
                    let n = run.tree.node(*id);
                    serde_json::json!({
                        "depth": depth,
                        "kind": format!("{:?}", n.kind),
                        "label": n.label,
                        "state": format!("{:?}", n.state),
                        "duration_ms": n.duration.map(|d| d.as_millis() as u64),
                        "expanded": !n.children.is_empty() && self.expanded(n),
                        "selected": *id == self.selected,
                    })
                }).collect::<Vec<_>>(),
                "selected": {
                    "label": node.label,
                    "state": format!("{:?}", node.state),
                    "message": node.diagnostic.as_ref().map(|d| d.message.clone()),
                    "has_comparison": node.diagnostic.as_ref().is_some_and(|d| d.comparison.is_some()),
                    "console_lines": self.console.len(),
                    "console_head": self.console.iter().take(40).map(|l| l.segs.iter().map(|s| s.text.clone()).collect::<String>()).collect::<Vec<_>>(),
                    "links": self.links.iter().map(|l| format!("{}:{}", l.path.display(), l.position.line + 1)).collect::<Vec<_>>(),
                },
            })
        });
        serde_json::json!({
            "service": self.service.is_some(),
            "detection": detection,
            "problem": self.problem.as_ref().map(t::problem_text),
            "show_passed": self.show_passed,
            "confirm_restart": self.confirm_restart,
            "run": run,
        })
    }
}

// ----- Dessin d'une ligne de l'arbre ---------------------------------------------

struct NodeRowEvent {
    clicked: bool,
    double: bool,
    chevron: bool,
}

fn state_color(node: &TestNode, p: &Palette) -> Color32 {
    let state = if node.worst.severity() > node.state.severity() && !node.finished {
        node.worst
    } else {
        node.state
    };
    match state {
        TestState::Pending => p.text_dim,
        TestState::Running => p.accent,
        TestState::Passed => p.ok,
        TestState::Skipped | TestState::Incomplete => p.text_dim,
        TestState::Risky | TestState::Interrupted => p.warn,
        TestState::Failed | TestState::Error => p.error,
    }
}

fn node_row(
    ui: &mut Ui,
    p: &Palette,
    node: &TestNode,
    depth: u16,
    expanded: bool,
    selected: bool,
) -> NodeRowEvent {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click());
    if selected {
        ui.painter().rect_filled(rect, 0.0, p.accent_soft);
    } else if response.hovered() {
        ui.painter().rect_filled(rect, 0.0, p.bg_alt);
    }
    let mut x = rect.left() + 8.0 + f32::from(depth) * 16.0;
    let mut chevron = false;
    if !node.children.is_empty() {
        let r = Rect::from_center_size(pos2(x + 7.0, rect.center().y), vec2(16.0, 16.0));
        let hit = ui.interact(
            r.expand(2.0),
            Id::unique(("tests-chevron", node.id.index())),
            Sense::click(),
        );
        icons::paint(
            ui.painter(),
            r,
            if expanded {
                Icon::ChevronDown
            } else {
                Icon::ChevronRight
            },
            p.text_dim,
        );
        chevron = hit.clicked();
    }
    x += 18.0;
    let color = state_color(node, p);
    let icon_rect = Rect::from_center_size(pos2(x + 7.0, rect.center().y), vec2(13.0, 13.0));
    let effective = if node.worst.severity() > node.state.severity() && !node.finished {
        node.worst
    } else {
        node.state
    };
    let icon = match effective {
        TestState::Passed => Icon::Check,
        TestState::Failed => Icon::Close,
        TestState::Error => Icon::Warning,
        TestState::Risky | TestState::Interrupted => Icon::Warning,
        TestState::Running => Icon::Play,
        TestState::Skipped | TestState::Incomplete | TestState::Pending => Icon::Dot,
    };
    icons::paint(ui.painter(), icon_rect, icon, color);
    x += 20.0;
    let clip = ui.painter().with_clip_rect(Rect::from_min_max(
        rect.min,
        pos2(rect.right() - 80.0, rect.bottom()),
    ));
    let label = if node.kind == NodeKind::Root {
        t::TESTS_ROOT.to_owned()
    } else {
        node.label.clone()
    };
    clip.text(
        pos2(x, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(13.5),
        if node.kind == NodeKind::DataSet || node.is_test {
            ui.visuals().text_color()
        } else {
            p.text
        },
    );
    if let Some(duration) = node.duration {
        ui.painter().text(
            pos2(rect.right() - 10.0, rect.center().y),
            Align2::RIGHT_CENTER,
            kit::format_duration(duration),
            FontId::proportional(12.0),
            p.text_dim,
        );
    }
    NodeRowEvent {
        clicked: response.clicked() && !chevron,
        double: response.double_clicked(),
        chevron,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use julie_phpunit::TestEvent;

    fn tree() -> ResultTree {
        let mut tree = ResultTree::new("C:/p");
        let events = vec![
            TestEvent::TestCount(2),
            TestEvent::SuiteStarted {
                name: "main".into(),
                location_hint: None,
            },
            TestEvent::TestStarted {
                name: "a".into(),
                location_hint: None,
            },
            TestEvent::TestFinished {
                name: "a".into(),
                duration: Some(Duration::from_millis(3)),
            },
            TestEvent::TestStarted {
                name: "b".into(),
                location_hint: None,
            },
            TestEvent::TestFailed {
                name: "b".into(),
                state: TestState::Failed,
                message: "non".into(),
                details: String::new(),
                duration: None,
                comparison: None,
            },
            TestEvent::TestFinished {
                name: "b".into(),
                duration: None,
            },
            TestEvent::SuiteFinished {
                name: "main".into(),
            },
        ];
        tree.apply_all(&events);
        tree
    }

    #[test]
    fn failing_containers_are_open_and_passed_nodes_can_be_hidden() {
        let mut view = TestsView::new(PathBuf::from("C:/p"), None);
        let tree = tree();
        let rows = view.rows(&tree);
        // racine, suite, deux tests : la suite a un échec, donc elle est dépliée.
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[1].1, 1);
        view.show_passed = false;
        let rows = view.rows(&tree);
        assert_eq!(rows.len(), 3);
        let labels: Vec<&str> = rows
            .iter()
            .map(|(id, _)| tree.node(*id).name.as_str())
            .collect();
        assert!(!labels.contains(&"a"));
        // Replier la suite cache ses enfants.
        view.toggled.insert(tree.children(tree.root())[0], false);
        assert_eq!(view.rows(&tree).len(), 2);
    }
}
