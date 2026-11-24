//! Diff côte à côte en lecture seule : deux éditeurs à défilement synchronisé,
//! blocs colorés, surlignage au mot, connecteurs courbes entre les volets,
//! navigation F7 / Maj+F7. Sert aux changements locaux, aux fichiers d'un commit
//! et à la comparaison attendu / obtenu des tests.
//!
//! Aucune règle de diff ici : `julie_diff::diff_lines` fait le calcul (dans une
//! tâche de fond, voir [`prepare_sides`]), cette vue ne fait que dessiner.

use std::ops::Range;
use std::path::PathBuf;

use eframe::egui::{
    self, pos2, vec2, Color32, FontId, Id, Key, Mesh, Modifiers, Painter, Rangef, Rect, Shape,
    Stroke, Ui,
};
use julie_core::{CancelToken, TextPosition, TextRange};
use julie_diff::{diff_lines, Diff, DiffOptions, FinalNewlineChange, HunkKind, LineEndingStyle};
use julie_text::{Document, Language};
use julie_vcs::{Content, DiffRequest, DiffSides, Revision};

use crate::editor::{
    split_panes, sync_scroll, Decorations, EditorView, LineDecoration, RangeDecoration, Tone,
};
use crate::icons::{self, Icon};
use crate::texts as t;
use crate::theme::{self, Palette};

/// Largeur de la gouttière où l'on dessine les connecteurs.
pub const GAP: f32 = 44.0;
const HEADER: f32 = 26.0;

/// Pourquoi on ne montre pas deux volets de texte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    Identical,
    /// « Seules les fins de ligne diffèrent » (noms des styles).
    OnlyLineEndings {
        from: &'static str,
        to: &'static str,
    },
    Binary {
        left: Option<u64>,
        right: Option<u64>,
    },
    TooLarge,
    Unsupported,
    Failed(String),
}

impl Notice {
    pub fn text(&self) -> String {
        match self {
            Notice::Identical => t::DIFF_IDENTICAL.to_owned(),
            Notice::OnlyLineEndings { from, to } => t::diff_only_line_endings(from, to),
            Notice::Binary { left, right } => t::diff_binary(*left, *right),
            Notice::TooLarge => t::DIFF_TOO_LARGE.to_owned(),
            Notice::Unsupported => t::DIFF_UNSUPPORTED.to_owned(),
            Notice::Failed(error) => error.clone(),
        }
    }

    pub fn key(&self) -> &'static str {
        match self {
            Notice::Identical => "identical",
            Notice::OnlyLineEndings { .. } => "only_line_endings",
            Notice::Binary { .. } => "binary",
            Notice::TooLarge => "too_large",
            Notice::Unsupported => "unsupported",
            Notice::Failed(_) => "failed",
        }
    }
}

/// Le résultat du calcul de fond : tout ce qu'il faut pour monter la vue.
pub struct Prepared {
    pub title: String,
    pub path: PathBuf,
    pub left_title: String,
    pub right_title: String,
    pub body: Body,
}

pub enum Body {
    Texts {
        left: String,
        right: String,
        diff: Box<Diff>,
    },
    Notice(Notice),
}

fn style_name(style: LineEndingStyle) -> &'static str {
    match style {
        LineEndingStyle::Lf => "LF",
        LineEndingStyle::CrLf => "CRLF",
        LineEndingStyle::Mixed => t::EOL_MIXED,
        LineEndingStyle::None => "—",
    }
}

fn revision_title(revision: &Revision) -> String {
    match revision {
        Revision::Absent => t::DIFF_REV_ABSENT.to_owned(),
        Revision::Head => t::DIFF_REV_HEAD.to_owned(),
        Revision::Index => t::DIFF_REV_INDEX.to_owned(),
        Revision::Stage(_) => t::DIFF_REV_STAGE.to_owned(),
        Revision::Commit(id) => t::diff_rev_commit(&id.chars().take(8).collect::<String>()),
        Revision::WorkTree => t::DIFF_REV_WORKTREE.to_owned(),
    }
}

fn content_bytes(content: &Content) -> Result<Vec<u8>, Notice> {
    match content {
        Content::Missing => Ok(Vec::new()),
        Content::Blob { bytes, .. } => Ok(bytes.clone()),
        Content::TooLarge { .. } => Err(Notice::TooLarge),
        Content::Submodule { .. } | Content::Directory => Err(Notice::Unsupported),
    }
}

/// Calcule le diff de deux volets lus par `julie-vcs`. **Bloquant** : à lancer
/// dans une tâche.
pub fn prepare_sides(sides: &DiffSides, title: String, cancel: &CancelToken) -> Prepared {
    let mut prepared = Prepared {
        title,
        path: sides.request.path.clone(),
        left_title: revision_title(&sides.request.left),
        right_title: revision_title(&sides.request.right),
        body: Body::Notice(Notice::Unsupported),
    };
    if sides.left.is_binary() || sides.right.is_binary() {
        prepared.body = Body::Notice(Notice::Binary {
            left: sides.left.size(),
            right: sides.right.size(),
        });
        return prepared;
    }
    let (left, right) = match (content_bytes(&sides.left), content_bytes(&sides.right)) {
        (Ok(left), Ok(right)) => (left, right),
        (Err(notice), _) | (_, Err(notice)) => {
            prepared.body = Body::Notice(notice);
            return prepared;
        }
    };
    prepared.body = prepare_bodies(left, right, cancel);
    prepared
}

/// Diff de deux textes (attendu / obtenu d'un test). **Bloquant.**
pub fn prepare_texts(
    title: String,
    left_title: String,
    right_title: String,
    left: String,
    right: String,
    cancel: &CancelToken,
) -> Prepared {
    Prepared {
        title,
        path: PathBuf::from("attendu-obtenu.txt"),
        left_title,
        right_title,
        body: prepare_bodies(left.into_bytes(), right.into_bytes(), cancel),
    }
}

fn prepare_bodies(left: Vec<u8>, right: Vec<u8>, cancel: &CancelToken) -> Body {
    // Le diff se calcule sur les octets exacts (fins de ligne comprises) ; les
    // volets montrent le texte décodé (fins de ligne normalisées).
    let left_raw = String::from_utf8_lossy(&left).into_owned();
    let right_raw = String::from_utf8_lossy(&right).into_owned();
    let diff = match diff_lines(&left_raw, &right_raw, &DiffOptions::default(), cancel) {
        Ok(diff) => diff,
        Err(error) => return Body::Notice(Notice::Failed(error.to_string())),
    };
    if diff.identical {
        return Body::Notice(Notice::Identical);
    }
    if diff.only_line_endings_differ() {
        return Body::Notice(Notice::OnlyLineEndings {
            from: style_name(diff.left_style),
            to: style_name(diff.right_style),
        });
    }
    Body::Texts {
        left: julie_core::file::decode(left).text,
        right: julie_core::file::decode(right).text,
        diff: Box::new(diff),
    }
}

/// Défilement synchronisé : où tombe la ligne du haut (fractionnaire) d'un volet
/// dans l'autre. `pairs` couvre les deux textes sans trou.
pub fn map_top(pairs: &[(Range<usize>, Range<usize>)], top: f32) -> f32 {
    let top = top.max(0.0);
    let line = top.floor() as usize;
    let fraction = top - top.floor();
    for (from, to) in pairs {
        if from.is_empty() {
            continue;
        }
        if line >= from.start && line < from.end {
            let position = (line - from.start) as f32 + fraction;
            let ratio = to.len() as f32 / from.len() as f32;
            return to.start as f32 + position * ratio;
        }
    }
    match pairs.last() {
        Some((from, to)) => to.end as f32 + (top - from.end as f32),
        None => top,
    }
}

fn smooth(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

/// Dessine, dans la gouttière `gap`, la forme courbe qui relie un bloc de
/// gauche (`left`, ordonnées écran) à son vis-à-vis de droite (`right`).
pub fn paint_connector(
    painter: &Painter,
    gap: Rect,
    left: Rangef,
    right: Rangef,
    fill: Color32,
    edge: Color32,
    edge_width: f32,
) {
    let min_y = left.min.min(right.min);
    let max_y = left.max.max(right.max);
    if max_y < gap.top() - 1.0 || min_y > gap.bottom() + 1.0 {
        return;
    }
    let painter = painter.with_clip_rect(gap);
    const STEPS: usize = 20;
    let thin = |range: Rangef| {
        if range.max - range.min < 2.0 {
            Rangef::new(range.min - 1.0, range.min + 1.0)
        } else {
            range
        }
    };
    let (left, right) = (thin(left), thin(right));
    let mut mesh = Mesh::default();
    let mut top_edge = Vec::with_capacity(STEPS + 1);
    let mut bottom_edge = Vec::with_capacity(STEPS + 1);
    for i in 0..=STEPS {
        let u = i as f32 / STEPS as f32;
        let x = gap.left() + u * gap.width();
        let s = smooth(u);
        let top = left.min + (right.min - left.min) * s;
        let bottom = left.max + (right.max - left.max) * s;
        mesh.colored_vertex(pos2(x, top), fill);
        mesh.colored_vertex(pos2(x, bottom), fill);
        top_edge.push(pos2(x, top));
        bottom_edge.push(pos2(x, bottom));
        if i > 0 {
            let k = (i * 2) as u32;
            mesh.add_triangle(k - 2, k - 1, k);
            mesh.add_triangle(k - 1, k + 1, k);
        }
    }
    painter.add(Shape::mesh(mesh));
    let stroke = Stroke::new(edge_width, edge);
    painter.add(Shape::line(top_edge, stroke));
    painter.add(Shape::line(bottom_edge, stroke));
}

fn tone_of(kind: HunkKind) -> Tone {
    match kind {
        HunkKind::Added => Tone::Added,
        HunkKind::Removed => Tone::Removed,
        HunkKind::Modified => Tone::Modified,
    }
}

fn char_column(doc: &Document, line: usize, byte: usize) -> Option<u32> {
    if line >= doc.buffer().len_lines() {
        return None;
    }
    let text = doc.buffer().line(line).to_string();
    if byte > text.len() || !text.is_char_boundary(byte) {
        return None;
    }
    Some(text[..byte].chars().count() as u32)
}

fn spans_to_ranges(
    spans: &[julie_diff::InlineSpan],
    doc: &Document,
    tone: Tone,
    out: &mut Vec<RangeDecoration>,
) {
    for span in spans {
        let (Some(start), Some(end)) = (
            char_column(doc, span.line, span.range.start),
            char_column(doc, span.line, span.range.end),
        ) else {
            continue;
        };
        if end > start {
            out.push(RangeDecoration {
                range: TextRange::new(
                    TextPosition::new(span.line as u32, start),
                    TextPosition::new(span.line as u32, end),
                ),
                tone,
            });
        }
    }
}

fn decorations(diff: &Diff, left: &Document, right: &Document) -> (Decorations, Decorations) {
    let (mut dl, mut dr) = (Decorations::default(), Decorations::default());
    for hunk in &diff.hunks {
        let tone = tone_of(hunk.kind);
        if hunk.left.is_empty() {
            dl.separators.push((hunk.left.start as u32, Tone::Added));
        } else {
            dl.lines.push(LineDecoration {
                lines: hunk.left.start as u32..hunk.left.end as u32,
                tone,
            });
        }
        if hunk.right.is_empty() {
            dr.separators.push((hunk.right.start as u32, Tone::Removed));
        } else {
            dr.lines.push(LineDecoration {
                lines: hunk.right.start as u32..hunk.right.end as u32,
                tone,
            });
        }
        if let Some(inline) = &hunk.inline {
            spans_to_ranges(&inline.left, left, Tone::Removed, &mut dl.ranges);
            spans_to_ranges(&inline.right, right, Tone::Added, &mut dr.ranges);
        }
    }
    dl.ranges.sort_by_key(|r| r.range.start);
    dr.ranges.sort_by_key(|r| r.range.start);
    (dl, dr)
}

fn new_document(text: &str, path: &std::path::Path, waker: &julie_text::Waker) -> Document {
    let mut document = Document::new(text);
    document.set_read_only(true);
    let language = Language::from_path(path);
    if language != Language::Plain {
        document.set_language(language);
        document.enable_syntax(Some(waker.clone()));
    }
    document
}

fn new_view(salt: (&'static str, &'static str), dark: bool) -> EditorView {
    let mut view = EditorView::new(salt).with_read_only(true);
    view.options.highlight_current_line = false;
    view.options.match_brackets = false;
    view.theme = Some(theme::editor_theme(dark));
    view
}

/// La vue elle-même.
pub struct DiffPane {
    salt: &'static str,
    pub title: String,
    left_title: String,
    right_title: String,
    left: Document,
    right: Document,
    left_view: EditorView,
    right_view: EditorView,
    diff: Option<Diff>,
    notice: Option<Notice>,
    left_dec: Decorations,
    right_dec: Decorations,
    to_right: Vec<(Range<usize>, Range<usize>)>,
    to_left: Vec<(Range<usize>, Range<usize>)>,
    /// Bloc courant (F7 / Maj+F7).
    pub current: Option<usize>,
    center: Option<usize>,
    loaded: bool,
}

impl DiffPane {
    pub fn new(salt: &'static str, dark: bool) -> Self {
        Self {
            salt,
            title: String::new(),
            left_title: String::new(),
            right_title: String::new(),
            left: Document::new(""),
            right: Document::new(""),
            left_view: new_view((salt, "gauche"), dark),
            right_view: new_view((salt, "droite"), dark),
            diff: None,
            notice: None,
            left_dec: Decorations::default(),
            right_dec: Decorations::default(),
            to_right: Vec::new(),
            to_left: Vec::new(),
            current: None,
            center: None,
            loaded: false,
        }
    }

    pub fn is_loaded(&self) -> bool {
        self.loaded
    }

    pub fn clear(&mut self) {
        self.loaded = false;
        self.diff = None;
        self.notice = None;
        self.title.clear();
    }

    pub fn set_dark(&mut self, dark: bool) {
        self.left_view.theme = Some(theme::editor_theme(dark));
        self.right_view.theme = Some(theme::editor_theme(dark));
    }

    pub fn hunk_count(&self) -> usize {
        self.diff.as_ref().map_or(0, |d| d.hunks.len())
    }

    /// Monte la vue avec un calcul terminé. Garde le défilement si c'est le même
    /// fichier (rafraîchissement) : le bloc courant reste choisi.
    pub fn set(&mut self, prepared: Prepared, waker: &julie_text::Waker) {
        let same = self.loaded && self.title == prepared.title;
        self.title = prepared.title;
        self.left_title = prepared.left_title;
        self.right_title = prepared.right_title;
        self.loaded = true;
        match prepared.body {
            Body::Notice(notice) => {
                self.notice = Some(notice);
                self.diff = None;
                self.current = None;
            }
            Body::Texts { left, right, diff } => {
                self.notice = None;
                self.left = new_document(&left, &prepared.path, waker);
                self.right = new_document(&right, &prepared.path, waker);
                let (dl, dr) = decorations(&diff, &self.left, &self.right);
                self.left_dec = dl;
                self.right_dec = dr;
                let alignment = diff.alignment();
                self.to_right = alignment
                    .iter()
                    .map(|a| (a.left.clone(), a.right.clone()))
                    .collect();
                self.to_left = alignment
                    .iter()
                    .map(|a| (a.right.clone(), a.left.clone()))
                    .collect();
                let count = diff.hunks.len();
                self.diff = Some(*diff);
                if !same || self.current.is_none_or(|c| c >= count) {
                    self.current = (count > 0).then_some(0);
                    self.center = self.current;
                }
            }
        }
    }

    /// Différence suivante (`delta` = 1) ou précédente (-1).
    pub fn step(&mut self, delta: isize) {
        let count = self.hunk_count();
        if count == 0 {
            return;
        }
        let next = match self.current {
            None => 0,
            Some(c) => (c as isize + delta).clamp(0, count as isize - 1) as usize,
        };
        self.current = Some(next);
        self.center = Some(next);
    }

    /// Montre les deux volets dans `rect` (en-têtes compris).
    pub fn show(&mut self, ui: &mut Ui, rect: Rect, p: &Palette, keys: bool) {
        if !self.loaded {
            return;
        }
        if let Some(notice) = &self.notice {
            let text = notice.text();
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                text,
                FontId::proportional(14.0),
                p.text_dim,
            );
            return;
        }
        if keys {
            let shifted = ui.input_mut(|i| i.consume_key(Modifiers::SHIFT, Key::F7));
            if shifted {
                self.step(-1);
            } else if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::F7)) {
                self.step(1);
            }
        }
        let header = Rect::from_min_size(rect.min, vec2(rect.width(), HEADER));
        let body = Rect::from_min_max(pos2(rect.min.x, rect.min.y + HEADER), rect.max);
        let (panes, gaps) = split_panes(body, 2, GAP);
        let (header_panes, _) = split_panes(header, 2, GAP);

        // En-têtes : titres des versions, compteur, navigation.
        let painter = ui.painter().clone();
        painter.rect_filled(header, 0.0, p.bg_alt);
        for (rect, title) in header_panes
            .iter()
            .zip([&self.left_title, &self.right_title])
        {
            painter.text(
                pos2(rect.left() + 8.0, rect.center().y),
                egui::Align2::LEFT_CENTER,
                title,
                FontId::proportional(12.5),
                p.text_dim,
            );
        }
        let count = self.hunk_count();
        let salt = self.salt;
        // À droite de l'en-tête : compteur et navigation.
        let right = header.right() - 6.0;
        let button = |x: f32| Rect::from_center_size(pos2(x, header.center().y), vec2(20.0, 20.0));
        if icons::tool_button(
            ui,
            button(right - 10.0),
            Id::unique((salt, "next")),
            Icon::ArrowDown,
            p,
            t::DIFF_NEXT,
            count > 0,
        ) {
            self.step(1);
        }
        if icons::tool_button(
            ui,
            button(right - 32.0),
            Id::unique((salt, "prev")),
            Icon::ArrowUp,
            p,
            t::DIFF_PREVIOUS,
            count > 0,
        ) {
            self.step(-1);
        }
        // Trop étroit pour les titres, le compteur et la note : on garde les boutons.
        if header_panes[1].width() > 360.0 {
            let counter = painter.text(
                pos2(right - 46.0, header.center().y),
                egui::Align2::RIGHT_CENTER,
                t::diff_count(count),
                FontId::proportional(12.5),
                p.text,
            );
            if let Some(diff) = &self.diff {
                let note = match diff.final_newline {
                    FinalNewlineChange::Same => None,
                    FinalNewlineChange::Added => Some(t::DIFF_FINAL_NEWLINE_ADDED),
                    FinalNewlineChange::Removed => Some(t::DIFF_FINAL_NEWLINE_REMOVED),
                };
                if let Some(note) = note {
                    painter.text(
                        pos2(counter.left() - 14.0, header.center().y),
                        egui::Align2::RIGHT_CENTER,
                        note,
                        FontId::proportional(12.0),
                        p.warn,
                    );
                }
            }
        }

        if let Some(hunk) = self.center.take() {
            if let Some(h) = self.diff.as_ref().and_then(|d| d.hunks.get(hunk)) {
                self.left_view.center_line(h.left.start as u32);
            }
        }
        let out_left = self
            .left_view
            .show_in_rect(ui, panes[0], &mut self.left, &self.left_dec);
        let out_right =
            self.right_view
                .show_in_rect(ui, panes[1], &mut self.right, &self.right_dec);

        // Connecteurs.
        let editor_theme = theme::editor_theme(p.dark);
        if let Some(diff) = &self.diff {
            for (index, hunk) in diff.hunks.iter().enumerate() {
                let tone = tone_of(hunk.kind);
                let current = self.current == Some(index);
                paint_connector(
                    ui.painter(),
                    gaps[0],
                    out_left
                        .layout
                        .lines_y_range(hunk.left.start as u32..hunk.left.end as u32),
                    out_right
                        .layout
                        .lines_y_range(hunk.right.start as u32..hunk.right.end as u32),
                    editor_theme.tone_background(tone),
                    editor_theme.tone_strong(tone),
                    if current { 2.0 } else { 1.0 },
                );
            }
        }
        let outputs = [out_left, out_right];
        let to_right = &self.to_right;
        let to_left = &self.to_left;
        sync_scroll(
            &mut [&mut self.left_view, &mut self.right_view],
            &outputs,
            |from, _to, top| {
                if from == 0 {
                    map_top(to_right, top)
                } else {
                    map_top(to_left, top)
                }
            },
        );
    }

    pub fn state(&self) -> serde_json::Value {
        serde_json::json!({
            "title": self.title,
            "loaded": self.loaded,
            "notice": self.notice.as_ref().map(|n| format!("{}: {}", n.key(), n.text())),
            "hunks": self.hunk_count(),
            "current": self.current,
            "left_title": self.left_title,
            "right_title": self.right_title,
            "left_lines": self.left.buffer().len_lines(),
            "right_lines": self.right.buffer().len_lines(),
            "left_first_line": self.left.buffer().line(0).to_string(),
            "right_first_line": self.right.buffer().line(0).to_string(),
            "kinds": self.diff.as_ref().map(|d| d.hunks.iter().map(|h| format!("{:?}", h.kind)).collect::<Vec<_>>()),
        })
    }
}

/// Demande de diff d'un fichier de changement local, pour titrer la vue.
pub fn request_title(request: &DiffRequest, root: Option<&std::path::Path>) -> String {
    let path = &request.path;
    root.and_then(|root| path.strip_prefix(root).ok())
        .unwrap_or(path)
        .display()
        .to_string()
        .replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_top_is_identity_on_equal_ranges_and_proportional_in_hunks() {
        let pairs = vec![(0..10, 0..10), (10..14, 10..12), (14..30, 12..28)];
        assert_eq!(map_top(&pairs, 3.5), 3.5);
        // Dans un bloc de 4 lignes à gauche pour 2 à droite : moitié.
        assert_eq!(map_top(&pairs, 12.0), 11.0);
        assert_eq!(map_top(&pairs, 20.25), 18.25);
        // Au-delà de la fin : on prolonge.
        assert_eq!(map_top(&pairs, 32.0), 30.0);
    }

    #[test]
    fn insertion_points_are_skipped() {
        let pairs = vec![(0..5, 0..5), (5..5, 5..8), (5..10, 8..13)];
        assert_eq!(map_top(&pairs, 5.0), 8.0);
    }

    #[test]
    fn identical_and_binary_give_a_notice() {
        let cancel = CancelToken::new();
        let same = prepare_texts(
            "a".into(),
            "g".into(),
            "d".into(),
            "x\n".into(),
            "x\n".into(),
            &cancel,
        );
        assert!(matches!(same.body, Body::Notice(Notice::Identical)));
        let crlf = prepare_texts(
            "a".into(),
            "g".into(),
            "d".into(),
            "x\ny\n".into(),
            "x\r\ny\r\n".into(),
            &cancel,
        );
        assert!(matches!(
            crlf.body,
            Body::Notice(Notice::OnlyLineEndings {
                from: "LF",
                to: "CRLF"
            })
        ));
        let different = prepare_texts(
            "a".into(),
            "g".into(),
            "d".into(),
            "x\ny\n".into(),
            "x\nz\n".into(),
            &cancel,
        );
        match different.body {
            Body::Texts { diff, .. } => assert_eq!(diff.hunks.len(), 1),
            _ => panic!("des blocs attendus"),
        }
    }
}
