//! Dialogue de fusion à trois volets : gauche = notre version, centre = résultat
//! éditable, droite = leur version. Blocs colorés selon leur nature, connecteurs
//! courbes entre les volets, accepter / ignorer par bloc dans les gouttières,
//! barre d'outils, compteur, confirmations à l'enregistrement et à la fermeture.
//!
//! **Aucune règle de fusion ici** : tout est piloté par `julie_diff::MergeSession`
//! (classement des blocs, application des non conflictuels, annuler / rétablir,
//! frappe libre). Cette vue relaie des gestes et dessine l'état de la session.

use std::ops::Range;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui::{
    self, pos2, vec2, Align2, Color32, FontId, Id, Key, Modifiers, Rect, Sense, Ui,
};
use julie_core::CancelToken;
use julie_diff::{ApplyScope, Block, BlockKind, HunkKind, MergeSession, Side, SideState};
use julie_text::{Document, Language};
use julie_vcs::{ConflictKind, ConflictVersions, Content, VcsError};

use crate::diffpane::{map_top, paint_connector};
use crate::editor::{
    split_panes, sync_scroll, Decorations, EditorTheme, EditorView, LineDecoration, Tone,
};
use crate::icons::{self, Icon};
use crate::kit::{self, Look};
use crate::texts as t;
use crate::theme::{self, Palette};

const GAP: f32 = 60.0;
const BUTTON: f32 = 22.0;
const TITLE_BAR: f32 = 40.0;
const TOOLBAR: f32 = 38.0;
const PANE_HEADER: f32 = 26.0;
const FOOTER: f32 = 46.0;
/// Au-delà, pas de dialogue : choix d'un côté seulement (spécification).
const MAX_LINES: usize = 200_000;

/// Pourquoi un fichier n'ouvre pas le dialogue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ineligible {
    Deletion,
    OneSide,
    Binary,
    NotUtf8,
    TooLarge,
    Unsupported,
}

/// Les trois textes à fusionner, en UTF-8 exact (fins de ligne comprises).
///
/// `ours` est le volet de **gauche** et `theirs` celui de **droite** : le travail de
/// l'utilisatrice est à gauche. Pendant un rebase (ou la remise d'une mise de côté) c'est
/// l'étage 3 de Git, les deux sont alors échangés (voir [`swapped`]).
pub struct Inputs {
    pub base: String,
    pub ours: String,
    pub theirs: String,
}

/// Les volets sont-ils échangés par rapport aux étages de Git ? Oui quand l'étage 3 porte
/// le travail de l'utilisatrice (rebase, remise d'une mise de côté).
pub fn swapped(versions: &ConflictVersions) -> bool {
    versions.labels.yours() == julie_vcs::ConflictSide::Theirs
}

/// (gauche, droite) : le contenu de chaque volet, votre travail à gauche.
fn panes(versions: &ConflictVersions) -> (&Content, &Content) {
    if swapped(versions) {
        (&versions.theirs, &versions.ours)
    } else {
        (&versions.ours, &versions.theirs)
    }
}

/// Vérifie les cas d'exclusion de la spécification et décode les trois versions.
pub fn inputs(versions: &ConflictVersions) -> Result<Inputs, Ineligible> {
    if versions.kind.involves_deletion() {
        return Err(Ineligible::Deletion);
    }
    if matches!(
        versions.kind,
        ConflictKind::AddedByUs | ConflictKind::AddedByThem
    ) {
        return Err(Ineligible::OneSide);
    }
    let text = |content: &Content, optional: bool| -> Result<String, Ineligible> {
        match content {
            Content::Missing if optional => Ok(String::new()),
            Content::Missing => Err(Ineligible::OneSide),
            Content::TooLarge { .. } => Err(Ineligible::TooLarge),
            Content::Submodule { .. } | Content::Directory => Err(Ineligible::Unsupported),
            Content::Blob { bytes, mode } => {
                if *mode == julie_vcs::FileMode::Symlink {
                    return Err(Ineligible::Unsupported);
                }
                if julie_vcs::is_binary(bytes) {
                    return Err(Ineligible::Binary);
                }
                if bytes.iter().filter(|b| **b == b'\n').count() > MAX_LINES {
                    return Err(Ineligible::TooLarge);
                }
                String::from_utf8(bytes.clone()).map_err(|_| Ineligible::NotUtf8)
            }
        }
    };
    let (left, right) = panes(versions);
    Ok(Inputs {
        base: text(&versions.base, true)?,
        ours: text(left, false)?,
        theirs: text(right, false)?,
    })
}

impl Ineligible {
    /// La raison, en français, pour la barre d'état et les infobulles.
    pub fn reason(self) -> &'static str {
        match self {
            Ineligible::Deletion => t::MERGE_REASON_DELETION,
            Ineligible::OneSide => t::MERGE_REASON_ONE_SIDE,
            Ineligible::Binary => t::MERGE_REASON_BINARY,
            Ineligible::NotUtf8 => t::MERGE_REASON_NOT_UTF8,
            Ineligible::TooLarge => t::MERGE_REASON_TOO_LARGE,
            Ineligible::Unsupported => t::MERGE_REASON_UNSUPPORTED,
        }
    }
}

/// Les mêmes vérifications que [`inputs`], sans copier ni décoder les versions :
/// sert à griser « Fusionner… » d'avance (tâche de fond, fichiers jusqu'à 20 Mio).
pub fn eligibility(versions: &ConflictVersions) -> Result<(), Ineligible> {
    let check = |content: &Content, optional: bool| -> Result<(), Ineligible> {
        match content {
            Content::Missing if optional => Ok(()),
            Content::Missing => Err(Ineligible::OneSide),
            Content::TooLarge { .. } => Err(Ineligible::TooLarge),
            Content::Submodule { .. } | Content::Directory => Err(Ineligible::Unsupported),
            Content::Blob { bytes, mode } => {
                if *mode == julie_vcs::FileMode::Symlink {
                    return Err(Ineligible::Unsupported);
                }
                if julie_vcs::is_binary(bytes) {
                    return Err(Ineligible::Binary);
                }
                if bytes.iter().filter(|b| **b == b'\n').count() > MAX_LINES {
                    return Err(Ineligible::TooLarge);
                }
                std::str::from_utf8(bytes)
                    .map(|_| ())
                    .map_err(|_| Ineligible::NotUtf8)
            }
        }
    };
    if versions.kind.involves_deletion() {
        return Err(Ineligible::Deletion);
    }
    if matches!(
        versions.kind,
        ConflictKind::AddedByUs | ConflictKind::AddedByThem
    ) {
        return Err(Ineligible::OneSide);
    }
    check(&versions.base, true)?;
    check(&versions.ours, false)?;
    check(&versions.theirs, false)
}

/// Ce que le dialogue demande à l'application.
#[derive(Debug)]
pub enum MergeIntent {
    /// Écrire le résultat et marquer le fichier résolu. `expected` : les octets
    /// du fichier lus à l'ouverture (`None` : écraser quoi qu'il y ait).
    Save {
        bytes: Vec<u8>,
        expected: Option<Vec<u8>>,
    },
    /// Fermer sans rien écrire.
    Close,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ask {
    /// Enregistrer alors qu'il reste des blocs en attente.
    SavePending,
    /// Fermer après des gestes.
    Discard,
    /// Le fichier a changé sur le disque depuis l'ouverture.
    Overwrite,
}

/// Les cartes de correspondance des lignes entre volets (défilement synchronisé).
#[derive(Default)]
struct Maps {
    l_to_c: Vec<(Range<usize>, Range<usize>)>,
    c_to_l: Vec<(Range<usize>, Range<usize>)>,
    c_to_r: Vec<(Range<usize>, Range<usize>)>,
    r_to_c: Vec<(Range<usize>, Range<usize>)>,
}

pub struct MergeDialog {
    pub path: PathBuf,
    name: String,
    expected: Option<Vec<u8>>,
    session: MergeSession,
    left: Document,
    center: Document,
    right: Document,
    left_view: EditorView,
    center_view: EditorView,
    right_view: EditorView,
    blocks: Vec<Block>,
    maps: Maps,
    left_dec: Decorations,
    center_dec: Decorations,
    right_dec: Decorations,
    dirty: bool,
    pub current: Option<usize>,
    titles: [String; 3],
    help: Option<&'static str>,
    eol_note: Option<String>,
    /// Bandeau « les versions diffèrent sur le BOM » (écart M7 de la recette).
    bom_note: Option<String>,
    ask: Option<Ask>,
    pub error: Option<String>,
    pub saving: bool,
    typing_at: Option<Instant>,
    center_goto: Option<usize>,
    focus_center: bool,
}

fn new_view(salt: &'static str, dark: bool, read_only: bool) -> EditorView {
    let mut view = EditorView::new(("fusion", salt)).with_read_only(read_only);
    view.options.font_size = 13.5;
    view.options.highlight_current_line = !read_only;
    view.options.match_brackets = !read_only;
    view.theme = Some(theme::editor_theme(dark));
    view
}

fn new_document(
    text: &str,
    language: Language,
    waker: &julie_text::Waker,
    read_only: bool,
) -> Document {
    let mut document = Document::new(text);
    if language != Language::Plain {
        document.set_language(language);
        document.enable_syntax(Some(waker.clone()));
    }
    document.set_read_only(read_only);
    document
}

fn display_text(bytes: &Content) -> String {
    bytes
        .bytes()
        .map(|b| julie_core::file::decode(b.to_vec()).text)
        .unwrap_or_default()
}

/// Remplace le texte du document en ne touchant qu'à la zone qui diffère, pour
/// garder le curseur et un historique léger.
fn set_document_text(document: &mut Document, new: &str) {
    let old = document.text();
    if old == new {
        return;
    }
    let prefix = old
        .chars()
        .zip(new.chars())
        .take_while(|(a, b)| a == b)
        .count();
    let old_rest = old.chars().count() - prefix;
    let new_rest = new.chars().count() - prefix;
    let suffix = old
        .chars()
        .rev()
        .zip(new.chars().rev())
        .take(old_rest.min(new_rest))
        .take_while(|(a, b)| a == b)
        .count();
    let old_len = prefix + old_rest;
    let cursor = document
        .buffer()
        .position_to_offset(document.cursor_position());
    let middle: String = new.chars().skip(prefix).take(new_rest - suffix).collect();
    document.set_cursor(prefix, false);
    document.set_cursor(old_len - suffix, true);
    document.paste(&middle);
    let delta = middle.chars().count() as isize - (old_len - suffix - prefix) as isize;
    let mapped = if cursor <= prefix {
        cursor
    } else if cursor >= old_len - suffix {
        (cursor as isize + delta).max(0) as usize
    } else {
        prefix + middle.chars().count()
    };
    document.set_cursor(mapped.min(document.buffer().len_chars()), false);
}

fn result_text_of(session: &MergeSession) -> String {
    let mut text = String::new();
    for line in session.result_lines() {
        text.push_str(line);
        text.push('\n');
    }
    text
}

fn side_shape(block: &Block) -> Side {
    if block.kind == BlockKind::Right {
        Side::Right
    } else {
        Side::Left
    }
}

fn tone_of(block: &Block, side: Side) -> Tone {
    if block.kind == BlockKind::Conflict {
        return Tone::Conflict;
    }
    match block.shape(side) {
        HunkKind::Added => Tone::Added,
        HunkKind::Removed => Tone::Removed,
        HunkKind::Modified => Tone::Modified,
    }
}

impl MergeDialog {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        versions: &ConflictVersions,
        session: MergeSession,
        project_root: &std::path::Path,
        waker: &julie_text::Waker,
        dark: bool,
    ) -> Self {
        let language = Language::from_path(&versions.path);
        let (left_content, right_content) = panes(versions);
        let ours_text = display_text(left_content);
        let theirs_text = display_text(right_content);
        let left = new_document(&ours_text, language, waker, true);
        let right = new_document(&theirs_text, language, waker, true);
        let center = new_document(&result_text_of(&session), language, waker, false);
        let name = versions
            .path
            .strip_prefix(project_root)
            .unwrap_or(&versions.path)
            .display()
            .to_string()
            .replace('\\', "/");
        let report = session.line_endings();
        let eol_note = report.differ.then(|| {
            t::merge_eol_banner(match report.result {
                julie_diff::LineEnding::CrLf => "CRLF",
                _ => "LF",
            })
        });
        let bom = session.bom();
        let bom_note = bom
            .differ
            .then(|| t::merge_bom_banner(bom.result, bom.their_change_is_dropped()));
        let labels = &versions.labels;
        let help = t::merge_side_help(labels.side(labels.yours()).role);
        let (left_label, right_label) =
            (labels.side(labels.yours()), labels.side(labels.incoming()));
        let expected = match &versions.work_tree {
            Content::Blob { bytes, .. } => Some(bytes.clone()),
            _ => None,
        };
        let mut dialog = Self {
            path: versions.path.clone(),
            name,
            expected,
            session,
            left,
            center,
            right,
            left_view: new_view("gauche", dark, true),
            center_view: new_view("centre", dark, false),
            right_view: new_view("droite", dark, true),
            blocks: Vec::new(),
            maps: Maps::default(),
            left_dec: Decorations::default(),
            center_dec: Decorations::default(),
            right_dec: Decorations::default(),
            dirty: true,
            current: None,
            titles: [
                t::merge_side_title(left_label),
                t::MERGE_TITLE_RESULT.to_owned(),
                t::merge_side_title(right_label),
            ],
            help,
            eol_note,
            bom_note,
            ask: None,
            error: None,
            saving: false,
            typing_at: None,
            center_goto: None,
            focus_center: false,
        };
        dialog.refresh();
        dialog.current = dialog.session.next_pending(None);
        dialog.center_goto = dialog.current;
        dialog
    }

    pub fn set_dark(&mut self, dark: bool) {
        for view in [
            &mut self.left_view,
            &mut self.center_view,
            &mut self.right_view,
        ] {
            view.theme = Some(theme::editor_theme(dark));
        }
    }

    // ----- Gestes (clavier, souris, scénario) ------------------------------------

    pub fn accept(&mut self, id: usize, side: Side) -> Result<(), String> {
        self.session.accept(id, side).map_err(|e| e.to_string())?;
        self.after_gesture(Some(id));
        Ok(())
    }

    pub fn accept_both(&mut self, id: usize, first: Side) -> Result<(), String> {
        self.session
            .accept_both(id, first)
            .map_err(|e| e.to_string())?;
        self.after_gesture(Some(id));
        Ok(())
    }

    pub fn ignore(&mut self, id: usize, side: Side) -> Result<(), String> {
        self.session.ignore(id, side).map_err(|e| e.to_string())?;
        self.after_gesture(Some(id));
        Ok(())
    }

    pub fn apply(&mut self, scope: ApplyScope) -> usize {
        let applied = self.session.apply_non_conflicting(scope);
        self.after_gesture(None);
        applied
    }

    pub fn undo(&mut self) {
        self.session.undo();
        self.after_gesture(None);
    }

    pub fn redo(&mut self) {
        self.session.redo();
        self.after_gesture(None);
    }

    /// Simule une frappe : remplace la ligne `line` (base zéro) du résultat.
    pub fn edit_line(&mut self, line: usize, text: &str) -> Result<(), String> {
        self.session
            .replace_lines(line..line + 1, &[text])
            .map_err(|e| e.to_string())?;
        self.after_gesture(None);
        Ok(())
    }

    pub fn step(&mut self, forward: bool) {
        let found = if forward {
            self.session
                .next_pending(self.current)
                .or_else(|| self.session.next_pending(None))
        } else {
            self.session
                .previous_pending(self.current)
                .or_else(|| self.session.previous_pending(None))
        };
        if let Some(id) = found {
            self.current = Some(id);
            self.center_goto = Some(id);
        }
    }

    pub fn focus_center(&mut self) {
        self.focus_center = true;
    }

    pub fn goto_line(&mut self, line: u32) {
        let offset = self
            .center
            .buffer()
            .position_to_offset(julie_core::TextPosition::new(line, 0));
        self.center.set_cursor(offset, false);
        self.center_view.center_line(line);
        self.center_view.request_focus();
    }

    fn after_gesture(&mut self, block: Option<usize>) {
        self.typing_at = None;
        self.error = None;
        if let Some(id) = block {
            self.current = Some(id);
        }
        set_document_text(&mut self.center, &result_text_of(&self.session));
        self.dirty = true;
    }

    fn pending_counts(&self) -> julie_diff::MergeCounts {
        self.session.counts()
    }

    /// Demande d'enregistrement (bouton, Ctrl+Entrée, scénario).
    pub fn request_save(&mut self, out: &mut Vec<MergeIntent>) {
        if self.saving {
            return;
        }
        if self.pending_counts().is_done() {
            self.do_save(true, out);
        } else {
            self.ask = Some(Ask::SavePending);
        }
    }

    fn do_save(&mut self, check_disk: bool, out: &mut Vec<MergeIntent>) {
        self.ask = None;
        self.saving = true;
        self.error = None;
        out.push(MergeIntent::Save {
            bytes: self.session.result_text().into_bytes(),
            expected: if check_disk {
                self.expected.clone()
            } else {
                None
            },
        });
    }

    pub fn request_close(&mut self, out: &mut Vec<MergeIntent>) {
        if self.session.is_touched() {
            self.ask = Some(Ask::Discard);
        } else {
            out.push(MergeIntent::Close);
        }
    }

    /// Répond à la question posée (scénario : `merge.confirm` / `merge.cancel`).
    pub fn answer(&mut self, yes: bool, out: &mut Vec<MergeIntent>) {
        let Some(ask) = self.ask.take() else {
            return;
        };
        if !yes {
            return;
        }
        match ask {
            Ask::SavePending => self.do_save(true, out),
            Ask::Overwrite => self.do_save(false, out),
            Ask::Discard => out.push(MergeIntent::Close),
        }
    }

    pub fn has_question(&self) -> bool {
        self.ask.is_some()
    }

    /// L'écriture a échoué : montrer la raison (ou proposer d'écraser).
    pub fn on_save_failed(&mut self, error: &VcsError) {
        self.saving = false;
        if matches!(error, VcsError::ChangedOnDisk(_)) {
            self.ask = Some(Ask::Overwrite);
        } else {
            self.error = Some(crate::gitview::failure_text(error));
        }
    }

    // ----- Recalcul des blocs, des cartes et des décorations ---------------------

    fn refresh(&mut self) {
        self.dirty = false;
        self.blocks = self.session.blocks();
        let alignment = self.session.alignment();
        self.maps = Maps {
            l_to_c: alignment
                .iter()
                .map(|a| (a.left.clone(), a.result.clone()))
                .collect(),
            c_to_l: alignment
                .iter()
                .map(|a| (a.result.clone(), a.left.clone()))
                .collect(),
            c_to_r: alignment
                .iter()
                .map(|a| (a.result.clone(), a.right.clone()))
                .collect(),
            r_to_c: alignment
                .iter()
                .map(|a| (a.right.clone(), a.result.clone()))
                .collect(),
        };
        let (mut dl, mut dc, mut dr) = (
            Decorations::default(),
            Decorations::default(),
            Decorations::default(),
        );
        for block in &self.blocks {
            let faint = block.resolved;
            let mark = |dec: &mut Decorations, lines: &Range<usize>, tone: Tone| {
                let tone = if faint { Tone::Occurrence } else { tone };
                if lines.is_empty() {
                    dec.separators.push((lines.start as u32, tone));
                } else {
                    dec.lines.push(LineDecoration {
                        lines: lines.start as u32..lines.end as u32,
                        tone,
                    });
                }
            };
            if block.left != SideState::NotApplicable {
                mark(&mut dl, &block.ours, tone_of(block, Side::Left));
            }
            if block.right != SideState::NotApplicable {
                mark(&mut dr, &block.theirs, tone_of(block, Side::Right));
            }
            mark(&mut dc, &block.result, tone_of(block, side_shape(block)));
        }
        self.left_dec = dl;
        self.center_dec = dc;
        self.right_dec = dr;
    }

    /// La frappe dans le centre : on la relaie à la session par lignes entières.
    fn on_center_edited(&mut self) {
        let text = self.center.text();
        let mut lines: Vec<&str> = text.split('\n').collect();
        if lines.last() == Some(&"") {
            lines.pop();
        }
        let same = {
            let mut current = self.session.result_lines();
            lines.iter().all(|l| current.next() == Some(*l)) && current.next().is_none()
        };
        if same {
            return;
        }
        let count = self.session.result_line_count();
        if self.session.replace_lines(0..count, &lines).is_ok() {
            // Une rafale de frappe forme une seule étape annulable.
            if self
                .typing_at
                .is_some_and(|at| at.elapsed() < Duration::from_millis(1000))
            {
                self.session.merge_last_two_steps();
            }
            self.typing_at = Some(Instant::now());
            self.dirty = true;
        }
    }

    // ----- Affichage ---------------------------------------------------------------

    pub fn show(&mut self, ctx: &egui::Context, p: &Palette) -> Vec<MergeIntent> {
        let mut out = Vec::new();
        let screen = ctx.content_rect();
        let margin = if screen.width() > 1500.0 { 24.0 } else { 8.0 };
        let size = vec2(
            (screen.width() - 2.0 * margin).max(640.0),
            (screen.height() - 2.0 * margin).max(380.0),
        );
        let frame = egui::Frame::new()
            .fill(p.bg)
            .stroke(egui::Stroke::new(1.0, p.border))
            .corner_radius(10)
            .shadow(egui::Shadow {
                offset: [0, 10],
                blur: 36,
                spread: 2,
                color: Color32::from_black_alpha(130),
            });
        kit::modal("fusion-dialogue").frame(frame).show(ctx, |ui| {
            let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
            self.keys(ui, &mut out);
            self.draw(ui, rect, p, &mut out);
        });
        self.show_questions(ctx, p, &mut out);
        out
    }

    fn keys(&mut self, ui: &mut Ui, out: &mut Vec<MergeIntent>) {
        if self.ask.is_some() || self.saving {
            return;
        }
        let consume = |ui: &mut Ui, modifiers: Modifiers, key: Key| {
            ui.input_mut(|i| i.consume_key(modifiers, key))
        };
        let alt_shift = Modifiers::ALT | Modifiers::SHIFT;
        let current = self.current;
        let act = |dialog: &mut MergeDialog, f: &dyn Fn(&mut MergeDialog, usize)| {
            if let Some(id) = current {
                f(dialog, id);
            }
        };
        if consume(ui, alt_shift, Key::ArrowLeft) {
            act(self, &|d, id| {
                let _ = d.ignore(id, Side::Left);
            });
        } else if consume(ui, alt_shift, Key::ArrowRight) {
            act(self, &|d, id| {
                let _ = d.ignore(id, Side::Right);
            });
        } else if consume(ui, Modifiers::ALT, Key::ArrowLeft) {
            act(self, &|d, id| {
                let _ = d.accept(id, Side::Left);
            });
        } else if consume(ui, Modifiers::ALT, Key::ArrowRight) {
            act(self, &|d, id| {
                let _ = d.accept(id, Side::Right);
            });
        }
        if consume(ui, Modifiers::SHIFT, Key::F7) {
            self.step(false);
        } else if consume(ui, Modifiers::NONE, Key::F7) {
            self.step(true);
        }
        if consume(ui, Modifiers::CTRL, Key::Enter) {
            self.request_save(out);
        }
        if consume(ui, Modifiers::CTRL | Modifiers::SHIFT, Key::Z) {
            self.redo();
        } else if consume(ui, Modifiers::CTRL, Key::Z) {
            self.undo();
        }
        if consume(ui, Modifiers::NONE, Key::Escape) {
            self.request_close(out);
        }
    }

    fn draw(&mut self, ui: &mut Ui, rect: Rect, p: &Palette, out: &mut Vec<MergeIntent>) {
        if self.dirty {
            self.refresh();
        }
        let painter = ui.painter().clone();
        let counts = self.pending_counts();

        // Barre de titre.
        let title_bar = Rect::from_min_size(rect.min, vec2(rect.width(), TITLE_BAR));
        painter.rect_filled(
            title_bar,
            egui::CornerRadius {
                nw: 10,
                ne: 10,
                sw: 0,
                se: 0,
            },
            p.bg_alt,
        );
        icons::paint(
            &painter,
            Rect::from_center_size(
                pos2(rect.left() + 24.0, title_bar.center().y),
                vec2(18.0, 18.0),
            ),
            Icon::Bolt,
            p.accent,
        );
        let title = painter.text(
            pos2(rect.left() + 42.0, title_bar.center().y),
            Align2::LEFT_CENTER,
            t::merge_title(&self.name),
            FontId::proportional(15.0),
            p.text,
        );
        painter.text(
            pos2(title.right() + 12.0, title_bar.center().y),
            Align2::LEFT_CENTER,
            t::MERGE_SUBTITLE,
            FontId::proportional(12.5),
            p.text_dim,
        );
        let close = Rect::from_center_size(
            pos2(title_bar.right() - 22.0, title_bar.center().y),
            vec2(26.0, 26.0),
        );
        if icons::tool_button(
            ui,
            close,
            Id::unique("fusion-fermer"),
            Icon::Close,
            p,
            t::MERGE_CLOSE,
            true,
        ) {
            self.request_close(out);
        }
        // Compteur.
        let (counter, color) = if counts.is_done() {
            (t::MERGE_ALL_DONE.to_owned(), p.ok)
        } else {
            (t::merge_counts(counts.changes, counts.conflicts), p.warn)
        };
        let galley = painter.layout_no_wrap(counter, FontId::proportional(13.5), color);
        let chip = Rect::from_min_size(
            pos2(
                close.left() - 16.0 - galley.size().x - 18.0,
                title_bar.center().y - 12.0,
            ),
            vec2(galley.size().x + 18.0, 24.0),
        );
        painter.rect_filled(chip, 12.0, color.gamma_multiply(0.18));
        painter.galley(
            pos2(chip.left() + 9.0, chip.center().y - galley.size().y / 2.0),
            galley,
            color,
        );

        // Barre d'outils.
        let toolbar = Rect::from_min_size(
            pos2(rect.left(), title_bar.bottom()),
            vec2(rect.width(), TOOLBAR),
        );
        self.draw_toolbar(ui, toolbar, p, &counts);

        // Bandeaux.
        let mut top = toolbar.bottom();
        let mut banners: Vec<(Color32, String, Icon)> = Vec::new();
        if let Some(note) = &self.eol_note {
            banners.push((p.warn, note.clone(), Icon::Warning));
        }
        if let Some(note) = &self.bom_note {
            banners.push((p.warn, note.clone(), Icon::Warning));
        }
        if let Some(help) = self.help {
            banners.push((p.accent, help.to_owned(), Icon::Bolt));
        }
        if let Some(error) = &self.error {
            banners.push((p.error, error.trim().replace('\n', "  "), Icon::Warning));
        }
        for (color, text, icon) in banners {
            let banner = Rect::from_min_size(pos2(rect.left(), top), vec2(rect.width(), 26.0));
            painter.rect_filled(banner, 0.0, color.gamma_multiply(0.2));
            icons::paint(
                &painter,
                Rect::from_center_size(
                    pos2(banner.left() + 18.0, banner.center().y),
                    vec2(14.0, 14.0),
                ),
                icon,
                color,
            );
            painter.with_clip_rect(banner).text(
                pos2(banner.left() + 34.0, banner.center().y),
                Align2::LEFT_CENTER,
                text,
                FontId::proportional(13.0),
                p.text,
            );
            top += 26.0;
        }

        // Volets.
        let footer = Rect::from_min_max(pos2(rect.left(), rect.bottom() - FOOTER), rect.max);
        let panes_area =
            Rect::from_min_max(pos2(rect.left(), top), pos2(rect.right(), footer.top()));
        let header_rect =
            Rect::from_min_size(panes_area.min, vec2(panes_area.width(), PANE_HEADER));
        let body = Rect::from_min_max(
            pos2(panes_area.left(), header_rect.bottom()),
            panes_area.max,
        );
        let (headers, _) = split_panes(header_rect, 3, GAP);
        painter.rect_filled(header_rect, 0.0, p.bg_alt);
        for (rect, title) in headers.iter().zip(&self.titles) {
            painter.text(
                pos2(rect.left() + 10.0, rect.center().y),
                Align2::LEFT_CENTER,
                title,
                FontId::proportional(13.0),
                p.text,
            );
        }
        let (panes, gaps) = split_panes(body, 3, GAP);

        if std::mem::take(&mut self.focus_center) {
            self.center_view.request_focus();
        }
        if let Some(id) = self.center_goto.take() {
            if let Some(block) = self.blocks.get(id) {
                self.center_view.center_line(block.result.start as u32);
                let offset =
                    self.center
                        .buffer()
                        .position_to_offset(julie_core::TextPosition::new(
                            block.result.start as u32,
                            0,
                        ));
                self.center.set_cursor(offset, false);
            }
        }
        let out_left = self
            .left_view
            .show_in_rect(ui, panes[0], &mut self.left, &self.left_dec);
        let out_center =
            self.center_view
                .show_in_rect(ui, panes[1], &mut self.center, &self.center_dec);
        let out_right =
            self.right_view
                .show_in_rect(ui, panes[2], &mut self.right, &self.right_dec);
        if out_center.changed {
            self.on_center_edited();
        }
        if out_center.selection_changed && self.center_view.has_focus(ui.ctx()) {
            let line = self.center.cursor_position().line as usize;
            if let Some(id) = self.session.block_at_result_line(line) {
                self.current = Some(id);
            }
        }
        // Un clic dans un volet de côté choisit le bloc courant.
        for (response, layout, is_left) in [
            (&out_left.response, &out_left.layout, true),
            (&out_right.response, &out_right.layout, false),
        ] {
            if response.clicked() {
                if let Some(pos) = response.interact_pointer_pos() {
                    if let Some(line) = layout.line_at(pos.y) {
                        let line = line as usize;
                        if let Some(block) = self.blocks.iter().find(|b| {
                            if is_left {
                                b.ours.contains(&line)
                            } else {
                                b.theirs.contains(&line)
                            }
                        }) {
                            self.current = Some(block.id);
                        }
                    }
                }
            }
        }

        // Connecteurs et commandes de gouttière.
        let editor_theme = theme::editor_theme(p.dark);
        let mut gestures: Vec<(usize, Side, bool)> = Vec::new();
        for block in &self.blocks {
            let current = self.current == Some(block.id);
            let center_y = out_center
                .layout
                .lines_y_range(block.result.start as u32..block.result.end as u32);
            for (side, gap, side_layout, lines) in [
                (Side::Left, gaps[0], &out_left.layout, &block.ours),
                (Side::Right, gaps[1], &out_right.layout, &block.theirs),
            ] {
                if block.state(side) == SideState::NotApplicable {
                    continue;
                }
                let tone = tone_of(block, side);
                let side_y = side_layout.lines_y_range(lines.start as u32..lines.end as u32);
                let (a, b) = if side == Side::Left {
                    (side_y, center_y)
                } else {
                    (center_y, side_y)
                };
                let (fill, edge) = if block.resolved {
                    (
                        Color32::TRANSPARENT,
                        editor_theme.tone_strong(tone).gamma_multiply(0.35),
                    )
                } else {
                    (
                        editor_theme.tone_background(tone),
                        editor_theme.tone_strong(tone),
                    )
                };
                paint_connector(
                    ui.painter(),
                    gap,
                    a,
                    b,
                    fill,
                    edge,
                    if current { 2.5 } else { 1.2 },
                );
                self.gutter_buttons(
                    ui,
                    p,
                    &editor_theme,
                    block,
                    side,
                    gap,
                    side_layout,
                    lines,
                    &mut gestures,
                );
            }
        }
        let mut errors = Vec::new();
        for (id, side, ignore) in gestures {
            let result = if ignore {
                self.ignore(id, side)
            } else {
                self.accept(id, side)
            };
            if let Err(error) = result {
                errors.push(error);
            }
        }
        if let Some(error) = errors.pop() {
            self.error = Some(error);
        }

        let outputs = [out_left, out_center, out_right];
        let maps = &self.maps;
        sync_scroll(
            &mut [
                &mut self.left_view,
                &mut self.center_view,
                &mut self.right_view,
            ],
            &outputs,
            |from, to, top| match (from, to) {
                (0, 1) => map_top(&maps.l_to_c, top),
                (1, 0) => map_top(&maps.c_to_l, top),
                (1, 2) => map_top(&maps.c_to_r, top),
                (2, 1) => map_top(&maps.r_to_c, top),
                (0, _) => map_top(&maps.c_to_r, map_top(&maps.l_to_c, top)),
                _ => map_top(&maps.c_to_l, map_top(&maps.r_to_c, top)),
            },
        );

        self.draw_footer(ui, footer, p, &counts, out);
    }

    #[allow(clippy::too_many_arguments)]
    fn gutter_buttons(
        &self,
        ui: &mut Ui,
        p: &Palette,
        editor_theme: &EditorTheme,
        block: &Block,
        side: Side,
        gap: Rect,
        layout: &crate::editor::EditorLayout,
        lines: &Range<usize>,
        gestures: &mut Vec<(usize, Side, bool)>,
    ) {
        // Un changement identique n'a qu'une paire de commandes : côté gauche.
        if block.kind == BlockKind::Identical && side == Side::Right {
            return;
        }
        if self.saving || self.ask.is_some() {
            return;
        }
        let top = layout.line_top(lines.start as u32);
        let bottom = layout.line_top(lines.end as u32).max(top + BUTTON);
        if bottom < gap.top() || top > gap.bottom() {
            return;
        }
        let y = top
            .max(gap.top() + 2.0)
            .min((bottom - BUTTON).max(gap.top() + 2.0))
            .min(gap.bottom() - BUTTON - 2.0);
        let start = gap.left() + (gap.width() - (2.0 * BUTTON + 4.0)) / 2.0;
        let (accept_x, ignore_x) = match side {
            Side::Left => (start, start + BUTTON + 4.0),
            Side::Right => (start + BUTTON + 4.0, start),
        };
        let tone = tone_of(block, side);
        let id_of = |what: &str| Id::unique(("fusion-commande", block.id, side as u8, what));
        let appends = block.accept_appends(side);
        if block.can_accept(side) {
            let rect = Rect::from_min_size(pos2(accept_x, y), vec2(BUTTON, BUTTON));
            let response = ui.interact(rect, id_of("accepter"), Sense::click());
            let fill = editor_theme.tone_strong(tone);
            ui.painter().rect_filled(
                rect,
                5.0,
                if response.hovered() {
                    fill
                } else {
                    fill.gamma_multiply(0.82)
                },
            );
            let icon = if appends {
                Icon::Plus
            } else if side == Side::Left {
                Icon::ArrowRight
            } else {
                Icon::ArrowLeft
            };
            icons::paint(ui.painter(), rect.shrink(4.0), icon, Color32::WHITE);
            let shortcut = if side == Side::Left {
                "Alt+Gauche"
            } else {
                "Alt+Droite"
            };
            let tip = if appends {
                t::merge_tip_append(shortcut)
            } else {
                t::merge_tip_accept(side == Side::Left, shortcut)
            };
            response.clone().on_hover_text(tip);
            if response.clicked() {
                gestures.push((block.id, side, false));
            }
        }
        if block.can_ignore(side) {
            let rect = Rect::from_min_size(pos2(ignore_x, y), vec2(BUTTON, BUTTON));
            let response = ui.interact(rect, id_of("ignorer"), Sense::click());
            ui.painter().rect_filled(
                rect,
                5.0,
                if response.hovered() {
                    p.accent_soft
                } else {
                    p.bg_raised
                },
            );
            ui.painter().rect_stroke(
                rect,
                5.0,
                egui::Stroke::new(1.0, p.border),
                egui::StrokeKind::Inside,
            );
            icons::paint(ui.painter(), rect.shrink(5.0), Icon::Close, p.text_dim);
            let shortcut = if side == Side::Left {
                "Alt+Maj+Gauche"
            } else {
                "Alt+Maj+Droite"
            };
            response
                .clone()
                .on_hover_text(t::merge_tip_ignore(side == Side::Left, shortcut));
            if response.clicked() {
                gestures.push((block.id, side, true));
            }
        }
    }

    fn draw_toolbar(
        &mut self,
        ui: &mut Ui,
        rect: Rect,
        p: &Palette,
        counts: &julie_diff::MergeCounts,
    ) {
        ui.painter().hline(
            rect.x_range(),
            rect.bottom(),
            egui::Stroke::new(1.0, p.border),
        );
        let cy = rect.center().y;
        let mut x = rect.left() + 12.0;
        let icon_button =
            |ui: &mut Ui, x: &mut f32, id: &str, icon: Icon, tip: &str, enabled: bool| {
                let r = Rect::from_center_size(pos2(*x + 13.0, cy), vec2(26.0, 26.0));
                *x += 30.0;
                icons::tool_button(
                    ui,
                    r,
                    Id::unique(("fusion-outil", id)),
                    icon,
                    p,
                    tip,
                    enabled,
                )
            };
        let has_blocks = !self.blocks.is_empty();
        if icon_button(
            ui,
            &mut x,
            "prec",
            Icon::ArrowUp,
            t::MERGE_PREVIOUS,
            has_blocks,
        ) {
            self.step(false);
        }
        if icon_button(
            ui,
            &mut x,
            "suiv",
            Icon::ArrowDown,
            t::MERGE_NEXT,
            has_blocks,
        ) {
            self.step(true);
        }
        x += 8.0;
        ui.painter().vline(
            x,
            (cy - 10.0)..=(cy + 10.0),
            egui::Stroke::new(1.0, p.border),
        );
        x += 14.0;
        let label = ui.painter().text(
            pos2(x, cy),
            Align2::LEFT_CENTER,
            t::MERGE_APPLY_LABEL,
            FontId::proportional(13.0),
            p.text_dim,
        );
        x = label.right() + 10.0;
        let can_apply = counts.changes > 0;
        for (scope, text, tip) in [
            (ApplyScope::All, t::MERGE_APPLY_ALL, t::MERGE_APPLY_ALL_TIP),
            (
                ApplyScope::Left,
                t::MERGE_APPLY_LEFT,
                t::MERGE_APPLY_LEFT_TIP,
            ),
            (
                ApplyScope::Right,
                t::MERGE_APPLY_RIGHT,
                t::MERGE_APPLY_RIGHT_TIP,
            ),
        ] {
            let w = kit::text_width(ui, text);
            let r = Rect::from_min_size(pos2(x, cy - 14.0), vec2(w, 28.0));
            x += w + 6.0;
            let look = if scope == ApplyScope::All && can_apply {
                Look::Primary(p.accent)
            } else {
                Look::Normal
            };
            if kit::text_button_at(
                ui,
                p,
                r,
                Id::unique(("fusion-appliquer", scope as u8)),
                text,
                look,
                can_apply && !self.saving && self.ask.is_none(),
                tip,
            ) {
                self.apply(scope);
            }
        }
        x += 4.0;
        ui.painter().vline(
            x,
            (cy - 10.0)..=(cy + 10.0),
            egui::Stroke::new(1.0, p.border),
        );
        x += 6.0;
        if icon_button(
            ui,
            &mut x,
            "annuler",
            Icon::Undo,
            t::MERGE_UNDO,
            self.session.can_undo(),
        ) {
            self.undo();
        }
        if icon_button(
            ui,
            &mut x,
            "retablir",
            Icon::Redo,
            t::MERGE_REDO,
            self.session.can_redo(),
        ) {
            self.redo();
        }
    }

    fn draw_footer(
        &mut self,
        ui: &mut Ui,
        rect: Rect,
        p: &Palette,
        counts: &julie_diff::MergeCounts,
        out: &mut Vec<MergeIntent>,
    ) {
        ui.painter().rect_filled(
            rect,
            egui::CornerRadius {
                nw: 0,
                ne: 0,
                sw: 10,
                se: 10,
            },
            p.bg_alt,
        );
        ui.painter().text(
            pos2(rect.left() + 14.0, rect.center().y),
            Align2::LEFT_CENTER,
            if self.saving {
                t::MERGE_SAVING
            } else {
                t::MERGE_FOOTER_HINT
            },
            FontId::proportional(12.5),
            p.text_dim,
        );
        let save_label = t::MERGE_SAVE;
        let cancel_label = t::MERGE_CANCEL;
        let w_save = kit::text_width(ui, save_label) + 20.0;
        let w_cancel = kit::text_width(ui, cancel_label) + 10.0;
        let save = Rect::from_min_size(
            pos2(rect.right() - 16.0 - w_save, rect.center().y - 15.0),
            vec2(w_save, 30.0),
        );
        let cancel = Rect::from_min_size(
            pos2(save.left() - 10.0 - w_cancel, rect.center().y - 15.0),
            vec2(w_cancel, 30.0),
        );
        let idle = !self.saving && self.ask.is_none();
        if kit::text_button_at(
            ui,
            p,
            cancel,
            Id::unique("fusion-annuler"),
            cancel_label,
            Look::Normal,
            idle,
            t::MERGE_CANCEL_TIP,
        ) {
            self.request_close(out);
        }
        let color = if counts.is_done() { p.ok } else { p.accent };
        if kit::text_button_at(
            ui,
            p,
            save,
            Id::unique("fusion-enregistrer"),
            save_label,
            Look::Primary(color),
            idle,
            t::MERGE_SAVE_TIP,
        ) {
            self.request_save(out);
        }
    }

    fn show_questions(&mut self, ctx: &egui::Context, p: &Palette, out: &mut Vec<MergeIntent>) {
        let Some(ask) = self.ask else { return };
        let counts = self.pending_counts();
        let (title, detail, yes, no) = match ask {
            Ask::SavePending => (
                t::MERGE_ASK_PENDING_TITLE.to_owned(),
                t::merge_ask_pending_detail(counts.changes, counts.conflicts),
                t::MERGE_SAVE_ANYWAY,
                t::MERGE_BACK,
            ),
            Ask::Discard => (
                t::MERGE_ASK_DISCARD_TITLE.to_owned(),
                t::MERGE_ASK_DISCARD_DETAIL.to_owned(),
                t::MERGE_DISCARD,
                t::MERGE_KEEP_WORKING,
            ),
            Ask::Overwrite => (
                t::MERGE_ASK_OVERWRITE_TITLE.to_owned(),
                t::MERGE_ASK_OVERWRITE_DETAIL.to_owned(),
                t::MERGE_OVERWRITE,
                t::MERGE_BACK,
            ),
        };
        let mut answer = None;
        kit::modal("fusion-question").show(ctx, |ui| {
            ui.set_width(440.0);
            ui.label(egui::RichText::new(title).strong().size(15.0).color(p.text));
            ui.add_space(4.0);
            ui.label(egui::RichText::new(detail).color(p.text_dim));
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if kit::button(ui, p, yes, true, true).clicked() {
                    answer = Some(true);
                }
                if ui.button(no).clicked() {
                    answer = Some(false);
                }
            });
        });
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            answer = Some(false);
        } else if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) {
            answer = Some(true);
        }
        if let Some(yes) = answer {
            self.answer(yes, out);
        }
    }

    // ----- État pour le mode scénario --------------------------------------------

    pub fn state(&self) -> serde_json::Value {
        let counts = self.session.counts();
        let result = self.session.result_text();
        serde_json::json!({
            "file": self.name,
            "counts": { "changes": counts.changes, "conflicts": counts.conflicts },
            "all_resolved": counts.is_done(),
            "touched": self.session.is_touched(),
            "current": self.current,
            "question": self.ask.map(|a| format!("{a:?}")),
            "saving": self.saving,
            "error": self.error,
            "eol_banner": self.eol_note,
            "bom_banner": self.bom_note,
            "help": self.help,
            "titles": self.titles,
            "result_lines": self.session.result_line_count(),
            "result_text": result.chars().take(4000).collect::<String>(),
            "blocks": self.blocks.iter().map(|b| serde_json::json!({
                "id": b.id,
                "kind": format!("{:?}", b.kind),
                "left": format!("{:?}", b.left),
                "right": format!("{:?}", b.right),
                "resolved": b.resolved,
                "ours": [b.ours.start, b.ours.end],
                "theirs": [b.theirs.start, b.theirs.end],
                "result": [b.result.start, b.result.end],
            })).collect::<Vec<_>>(),
        })
    }
}

/// Construit la session dans une tâche de fond. **Bloquant.**
pub fn build_session(inputs: &Inputs, cancel: &CancelToken) -> Option<MergeSession> {
    MergeSession::new(&inputs.base, &inputs.ours, &inputs.theirs, cancel).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use julie_vcs::FileMode;

    fn versions(kind: ConflictKind, ours: Content, theirs: Content) -> ConflictVersions {
        ConflictVersions {
            path: PathBuf::from("C:/p/a.txt"),
            repo_path: "a.txt".into(),
            kind,
            operation: None,
            base: Content::Blob {
                bytes: b"a\n".to_vec(),
                mode: FileMode::Regular,
            },
            ours,
            theirs,
            work_tree: Content::Missing,
            labels: julie_vcs::ConflictLabels::for_operation(Some(julie_vcs::OperationKind::Merge)),
        }
    }

    fn blob(bytes: &[u8]) -> Content {
        Content::Blob {
            bytes: bytes.to_vec(),
            mode: FileMode::Regular,
        }
    }

    #[test]
    fn excluded_files_only_offer_a_side() {
        let ok = versions(ConflictKind::BothModified, blob(b"b\n"), blob(b"c\n"));
        assert!(inputs(&ok).is_ok());
        let deleted = versions(ConflictKind::DeletedByThem, blob(b"b\n"), Content::Missing);
        assert_eq!(inputs(&deleted).err(), Some(Ineligible::Deletion));
        let binary = versions(ConflictKind::BothModified, blob(b"b\0\n"), blob(b"c\n"));
        assert_eq!(inputs(&binary).err(), Some(Ineligible::Binary));
        let latin = versions(
            ConflictKind::BothModified,
            blob(&[0xe9, b'\n']),
            blob(b"c\n"),
        );
        assert_eq!(inputs(&latin).err(), Some(Ineligible::NotUtf8));
        let big = versions(
            ConflictKind::BothModified,
            Content::TooLarge { size: 30_000_000 },
            blob(b"c\n"),
        );
        assert_eq!(inputs(&big).err(), Some(Ineligible::TooLarge));
        let added = versions(ConflictKind::BothAdded, blob(b"x\n"), blob(b"y\n"));
        assert!(inputs(&added).is_ok());
    }

    #[test]
    fn eligibility_agrees_with_inputs() {
        let many_lines = "x\n".repeat(MAX_LINES + 1).into_bytes();
        let cases = [
            versions(ConflictKind::BothModified, blob(b"b\n"), blob(b"c\n")),
            versions(ConflictKind::BothAdded, blob(b"x\n"), blob(b"y\n")),
            versions(ConflictKind::DeletedByThem, blob(b"b\n"), Content::Missing),
            versions(ConflictKind::AddedByUs, Content::Missing, blob(b"c\n")),
            versions(ConflictKind::BothModified, blob(b"b\0\n"), blob(b"c\n")),
            versions(
                ConflictKind::BothModified,
                blob(&[0xe9, b'\n']),
                blob(b"c\n"),
            ),
            versions(
                ConflictKind::BothModified,
                Content::TooLarge { size: 30_000_000 },
                blob(b"c\n"),
            ),
            versions(ConflictKind::BothModified, blob(&many_lines), blob(b"c\n")),
        ];
        for case in &cases {
            assert_eq!(
                eligibility(case).err(),
                inputs(case).err(),
                "{:?}",
                case.kind
            );
        }
    }

    #[test]
    fn during_a_rebase_your_commit_is_on_the_left() {
        use julie_vcs::{ConflictLabels, OperationKind as Op};
        let mut v = versions(
            ConflictKind::BothModified,
            blob(b"amont\n"),
            blob(b"vous\n"),
        );
        v.operation = Some(Op::Rebase);
        v.labels = ConflictLabels::for_operation(Some(Op::Rebase));
        assert!(swapped(&v));
        let inputs = inputs(&v).unwrap();
        assert_eq!(inputs.ours, "vous\n", "gauche = votre commit");
        assert_eq!(inputs.theirs, "amont\n", "droite = l'amont");
        let merge = versions(ConflictKind::BothModified, blob(b"a\n"), blob(b"b\n"));
        assert!(!swapped(&merge));
        assert_eq!(super::inputs(&merge).unwrap().ours, "a\n");
    }

    #[test]
    fn document_text_is_patched_in_place() {
        let mut doc = Document::new("a\nb\nc\n");
        set_document_text(&mut doc, "a\nB\nB2\nc\n");
        assert_eq!(doc.text(), "a\nB\nB2\nc\n");
        set_document_text(&mut doc, "a\nc\n");
        assert_eq!(doc.text(), "a\nc\n");
        set_document_text(&mut doc, "");
        assert_eq!(doc.text(), "");
        set_document_text(&mut doc, "x\n");
        assert_eq!(doc.text(), "x\n");
    }

    #[test]
    fn typing_in_the_center_reaches_the_session() {
        let cancel = CancelToken::new();
        let session = MergeSession::new(
            "a\nb\nc\nd\ne\n",
            "a\nB\nc\nd\ne\n",
            "a\nb\nc\nD\ne\n",
            &cancel,
        )
        .unwrap();
        let waker: julie_text::Waker = std::sync::Arc::new(|| {});
        let v = versions(
            ConflictKind::BothModified,
            blob(b"a\nB\nc\nd\ne\n"),
            blob(b"a\nb\nc\nD\ne\n"),
        );
        let mut dialog = MergeDialog::new(&v, session, std::path::Path::new("C:/p"), &waker, true);
        assert_eq!(dialog.session.counts().changes, 2);
        dialog.apply(ApplyScope::All);
        assert_eq!(dialog.center.text(), "a\nB\nc\nD\ne\n");
        dialog.edit_line(0, "z").unwrap();
        assert_eq!(dialog.center.text(), "z\nB\nc\nD\ne\n");
        dialog.undo();
        assert_eq!(dialog.center.text(), "a\nB\nc\nD\ne\n");
        // Frappe dans le document : relayée à la session.
        dialog.center.set_cursor(0, false);
        dialog.center.insert_text("q");
        dialog.on_center_edited();
        assert_eq!(dialog.session.result_text(), "qa\nB\nc\nD\ne\n");
    }
}
