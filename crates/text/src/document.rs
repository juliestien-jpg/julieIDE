//! Document : un tampon, ses sélections, son fichier et sa coloration. C'est ce
//! que l'éditeur affiche et modifie.

use std::fs;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime};

use julie_core::file::{self, Encoding, LineEnding, TextFile, TextFormat};
use julie_core::TextPosition;

use crate::buffer::{Bias, Buffer, EditKind};
use crate::error::{TextError, TextResult};
use crate::movement::{
    self, column_for_visual, next_grapheme, prev_grapheme, visual_column, word_left, word_right,
    TAB_WIDTH,
};
use crate::selection::{Selection, Selections};
use crate::session::{SyntaxSession, Waker};
use crate::syntax::{HighlightSpan, Language};

/// Au-delà, un fichier n'est pas ouvert ([`TextError::TooLarge`]).
pub const MAX_OPEN_SIZE: u64 = 50 * 1024 * 1024;
/// Au-delà, un fichier est ouvert sans coloration.
pub const MAX_HIGHLIGHT_SIZE: u64 = 5 * 1024 * 1024;
/// Octets examinés pour reconnaître un fichier binaire (octet NUL).
const BINARY_SNIFF: usize = 8 * 1024;

/// Ce qui identifie l'état d'un fichier sur le disque à un moment donné.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiskStamp {
    pub modified: Option<SystemTime>,
    pub len: u64,
}

impl DiskStamp {
    /// Accès disque.
    pub fn read(path: &Path) -> std::io::Result<DiskStamp> {
        fs::metadata(path).map(|meta| DiskStamp::from_metadata(&meta))
    }

    fn from_metadata(meta: &fs::Metadata) -> DiskStamp {
        DiskStamp {
            modified: meta.modified().ok(),
            len: meta.len(),
        }
    }
}

/// Le fichier a-t-il bougé sur le disque depuis qu'on l'a lu ou écrit ?
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskState {
    Unchanged,
    Changed,
    Deleted,
}

/// Compare l'état du disque à `known` (accès disque).
pub fn check_disk(path: &Path, known: Option<DiskStamp>) -> DiskState {
    match (DiskStamp::read(path), known) {
        (Err(_), _) => DiskState::Deleted,
        (Ok(now), Some(known)) if now == known => DiskState::Unchanged,
        (Ok(_), _) => DiskState::Changed,
    }
}

/// Un fichier lu et décodé, prêt à devenir un [`Document`].
#[derive(Debug, Clone)]
pub struct LoadedFile {
    pub path: PathBuf,
    pub file: TextFile,
    pub stamp: DiskStamp,
}

/// Lit un fichier texte (accès disque, bloquant) : refuse les fichiers binaires
/// et ceux de plus de [`MAX_OPEN_SIZE`].
pub fn load_file(path: &Path) -> TextResult<LoadedFile> {
    let io = |source| julie_core::Error::io(path, source);
    let meta = fs::metadata(path).map_err(io)?;
    if meta.len() > MAX_OPEN_SIZE {
        return Err(TextError::TooLarge {
            path: path.to_path_buf(),
            size: meta.len(),
        });
    }
    let bytes = fs::read(path).map_err(io)?;
    if bytes[..bytes.len().min(BINARY_SNIFF)].contains(&0) {
        return Err(TextError::Binary {
            path: path.to_path_buf(),
        });
    }
    let stamp = DiskStamp::read(path).unwrap_or(DiskStamp::from_metadata(&meta));
    Ok(LoadedFile {
        path: path.to_path_buf(),
        file: file::decode(bytes),
        stamp,
    })
}

/// Ce qu'il faut pour écrire un document, détaché de lui (pour une tâche de fond).
#[derive(Debug, Clone)]
pub struct SaveRequest {
    pub path: PathBuf,
    pub text: String,
    pub format: TextFormat,
    /// État de l'historique enregistré (voir [`Buffer::state_id`]).
    pub state_id: u64,
}

impl SaveRequest {
    /// Encode et écrit atomiquement (accès disque, bloquant).
    pub fn execute(self) -> TextResult<SaveOutcome> {
        file::write_text(&self.path, &self.text, self.format)?;
        let stamp = DiskStamp::read(&self.path).ok();
        Ok(SaveOutcome {
            path: self.path,
            state_id: self.state_id,
            stamp,
        })
    }
}

/// Résultat d'un enregistrement réussi, à rendre au document par
/// [`Document::mark_saved`].
#[derive(Debug, Clone)]
pub struct SaveOutcome {
    pub path: PathBuf,
    pub state_id: u64,
    pub stamp: Option<DiskStamp>,
}

/// Indentation d'un fichier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndentStyle {
    Tabs,
    Spaces(u8),
}

impl Default for IndentStyle {
    fn default() -> Self {
        IndentStyle::Spaces(4)
    }
}

impl IndentStyle {
    /// Devine l'indentation d'après les premières lignes ; 4 espaces à défaut.
    pub fn detect(buffer: &Buffer) -> IndentStyle {
        let (mut tabs, mut spaces) = (0usize, 0usize);
        let mut widths = [0usize; 9];
        let mut previous = 0usize;
        for line in 0..buffer.len_lines().min(2000) {
            let mut chars = buffer.line(line).chars();
            match chars.next() {
                Some('\t') => tabs += 1,
                Some(' ') => {
                    spaces += 1;
                    let count = 1 + chars.take_while(|&c| c == ' ').count();
                    let diff = count.abs_diff(previous);
                    if (2..=8).contains(&diff) {
                        widths[diff] += 1;
                    }
                    previous = count;
                }
                Some(_) => previous = 0,
                None => {}
            }
        }
        if tabs > spaces {
            return IndentStyle::Tabs;
        }
        match (2..=8).max_by_key(|&w| (widths[w], w == 4)) {
            Some(width) if widths[width] > 0 => IndentStyle::Spaces(width as u8),
            _ => IndentStyle::default(),
        }
    }

    /// Une unité d'indentation.
    pub fn unit(self) -> String {
        match self {
            IndentStyle::Tabs => "\t".to_owned(),
            IndentStyle::Spaces(n) => " ".repeat(n as usize),
        }
    }

    fn width(self) -> u32 {
        match self {
            IndentStyle::Tabs => TAB_WIDTH,
            IndentStyle::Spaces(n) => u32::from(n.max(1)),
        }
    }
}

/// Pourquoi un document est en lecture seule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadOnlyReason {
    /// Demandé par l'appelant (volet de diff, version historique…).
    Requested,
    /// Fichier Windows-1252 contenant un octet non attribué (0x81, 0x8D, 0x8F,
    /// 0x90, 0x9D) : on ne le réécrit pas.
    UndefinedBytes,
}

/// Déplacement du curseur.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Motion {
    Left,
    Right,
    WordLeft,
    WordRight,
    Up,
    Down,
    /// Début de ligne « intelligent » : premier caractère non blanc, puis colonne 0.
    LineStart,
    LineEnd,
    PageUp(usize),
    PageDown(usize),
    DocumentStart,
    DocumentEnd,
}

/// Un remplacement dans une transaction, avec la place du curseur dans le texte
/// inséré (en caractères ; `None` : à la fin).
struct Edit {
    range: Range<usize>,
    text: String,
    cursor: Option<usize>,
}

/// Le document édité.
pub struct Document {
    buffer: Buffer,
    selections: Selections,
    path: Option<PathBuf>,
    format: TextFormat,
    new_line: &'static str,
    indent: IndentStyle,
    language: Language,
    read_only: Option<ReadOnlyReason>,
    disk: Option<DiskStamp>,
    syntax: Option<SyntaxSession>,
    clock: Box<dyn Fn() -> Instant + Send>,
}

impl Default for Document {
    fn default() -> Self {
        Self::new("")
    }
}

impl std::fmt::Debug for Document {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Document")
            .field("path", &self.path)
            .field("format", &self.format)
            .field("language", &self.language)
            .field("version", &self.buffer.version())
            .field("modified", &self.is_modified())
            .finish_non_exhaustive()
    }
}

impl Document {
    /// Un document sans fichier (LF, UTF-8, texte brut).
    pub fn new(text: &str) -> Self {
        let buffer = Buffer::new(text);
        let indent = IndentStyle::detect(&buffer);
        Self {
            buffer,
            selections: Selections::default(),
            path: None,
            format: TextFormat::default(),
            new_line: "\n",
            indent,
            language: Language::Plain,
            read_only: None,
            disk: None,
            syntax: None,
            clock: Box::new(Instant::now),
        }
    }

    /// Un document tiré d'un fichier lu par [`load_file`].
    pub fn from_loaded(loaded: LoadedFile) -> Self {
        let mut document = Self::from_text_file(loaded.file, Some(loaded.path));
        document.disk = Some(loaded.stamp);
        document
    }

    /// Un document tiré d'un texte décodé (par `julie_core::file::decode`, ou une
    /// version tirée de Git). `path` sert au langage et à l'enregistrement.
    pub fn from_text_file(file: TextFile, path: Option<PathBuf>) -> Self {
        let mut document = Self::new(&file.text);
        document.format = file.format;
        document.new_line = dominant_line_ending(&file.text, file.format.line_ending);
        if file.format.encoding == Encoding::Windows1252
            && file
                .text
                .contains(['\u{81}', '\u{8D}', '\u{8F}', '\u{90}', '\u{9D}'])
        {
            document.read_only = Some(ReadOnlyReason::UndefinedBytes);
        }
        if let Some(path) = &path {
            document.language = if file.text.len() as u64 > MAX_HIGHLIGHT_SIZE {
                Language::Plain
            } else {
                Language::from_path(path)
            };
        }
        document.path = path;
        document
    }

    /// Ouvre un fichier (accès disque, bloquant : dans une tâche).
    pub fn open(path: &Path) -> TextResult<Self> {
        load_file(path).map(Self::from_loaded)
    }

    // ----- Accès -----------------------------------------------------------

    pub fn buffer(&self) -> &Buffer {
        &self.buffer
    }

    pub fn text(&self) -> String {
        self.buffer.text()
    }

    pub fn version(&self) -> u64 {
        self.buffer.version()
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Change le chemin (« Enregistrer sous »). Le langage suit l'extension.
    pub fn set_path(&mut self, path: PathBuf) {
        self.set_language(Language::from_path(&path));
        self.path = Some(path);
        self.disk = None;
    }

    pub fn format(&self) -> TextFormat {
        self.format
    }

    /// Change l'encodage (« convertir en UTF-8 » après un refus d'encodage) :
    /// le document devient modifié.
    pub fn set_encoding(&mut self, encoding: Encoding) {
        if self.format.encoding != encoding {
            self.format.encoding = encoding;
            self.buffer.mark_unsaved();
        }
    }

    pub fn indent_style(&self) -> IndentStyle {
        self.indent
    }

    pub fn set_indent_style(&mut self, indent: IndentStyle) {
        self.indent = indent;
    }

    pub fn language(&self) -> Language {
        self.language
    }

    pub fn is_modified(&self) -> bool {
        self.buffer.is_modified()
    }

    pub fn read_only(&self) -> Option<ReadOnlyReason> {
        self.read_only
    }

    pub fn is_read_only(&self) -> bool {
        self.read_only.is_some()
    }

    /// Lecture seule demandée (volets de diff) ; `false` la lève sauf si elle
    /// vient du contenu du fichier.
    pub fn set_read_only(&mut self, read_only: bool) {
        match (read_only, self.read_only) {
            (true, None) => self.read_only = Some(ReadOnlyReason::Requested),
            (false, Some(ReadOnlyReason::Requested)) => self.read_only = None,
            _ => {}
        }
    }

    pub fn disk_stamp(&self) -> Option<DiskStamp> {
        self.disk
    }

    pub fn selections(&self) -> &Selections {
        &self.selections
    }

    /// Position (ligne, colonne) du curseur principal.
    pub fn cursor_position(&self) -> TextPosition {
        self.buffer
            .offset_to_position(self.selections.primary().head)
    }

    /// Horloge des étapes d'annulation (tests).
    pub fn set_clock(&mut self, clock: impl Fn() -> Instant + Send + 'static) {
        self.clock = Box::new(clock);
    }

    // ----- Coloration --------------------------------------------------------

    /// Lance la coloration en fil de fond ; `waker` est appelé quand un résultat
    /// est prêt (depuis le fil de fond). Sans effet en texte brut.
    pub fn enable_syntax(&mut self, waker: Option<Waker>) {
        self.buffer.take_byte_edits();
        self.syntax = (self.language != Language::Plain).then(|| {
            SyntaxSession::new(
                self.language,
                self.buffer.rope(),
                self.buffer.version(),
                waker,
            )
        });
    }

    /// Change le langage ; la coloration est relancée si elle était active.
    pub fn set_language(&mut self, language: Language) {
        if language == self.language {
            return;
        }
        self.language = language;
        if self.syntax.is_some() {
            self.syntax = None;
            self.enable_syntax(None);
        }
    }

    /// Prend le dernier résultat de coloration ; `true` s'il a changé.
    pub fn poll_syntax(&mut self) -> bool {
        self.syntax.as_mut().is_some_and(SyntaxSession::poll)
    }

    pub fn syntax(&self) -> Option<&SyntaxSession> {
        self.syntax.as_ref()
    }

    pub fn syntax_mut(&mut self) -> Option<&mut SyntaxSession> {
        self.syntax.as_mut()
    }

    /// Plages colorées de la ligne (vide sans coloration).
    pub fn highlights(&self, line: usize) -> &[HighlightSpan] {
        self.syntax.as_ref().map_or(&[], |s| s.line(line))
    }

    fn after_change(&mut self) {
        let edits = self.buffer.take_byte_edits();
        if let Some(syntax) = &mut self.syntax {
            syntax.update(self.buffer.rope(), self.buffer.version(), edits);
        }
    }

    // ----- Fichier -----------------------------------------------------------

    /// Prépare l'enregistrement : copie du texte et de son format. Clôt l'étape
    /// d'annulation en cours, pour qu'une frappe suivante rende le document
    /// modifié.
    pub fn save_request(&mut self) -> TextResult<SaveRequest> {
        let path = self.path.clone().ok_or(TextError::NoPath)?;
        self.buffer.seal();
        Ok(SaveRequest {
            path,
            text: self.buffer.text(),
            format: self.format,
            state_id: self.buffer.state_id(),
        })
    }

    /// Note un enregistrement réussi.
    pub fn mark_saved(&mut self, outcome: &SaveOutcome) {
        self.buffer.mark_saved(outcome.state_id);
        if self.path.as_deref() == Some(outcome.path.as_path()) {
            self.disk = outcome.stamp;
        }
    }

    /// Enregistre tout de suite (accès disque, bloquant : tests et outils).
    pub fn save(&mut self) -> TextResult<()> {
        let outcome = self.save_request()?.execute()?;
        self.mark_saved(&outcome);
        Ok(())
    }

    /// Remplace le contenu par celui relu sur le disque : historique vidé,
    /// curseur gardé sur la même ligne (bornée), état non modifié.
    pub fn reload(&mut self, loaded: LoadedFile) {
        let position = self.cursor_position();
        self.buffer.reset(&loaded.file.text);
        self.format = loaded.file.format;
        self.new_line = dominant_line_ending(&loaded.file.text, loaded.file.format.line_ending);
        self.disk = Some(loaded.stamp);
        let line = (position.line as usize).min(self.buffer.len_lines() - 1);
        let offset = self
            .buffer
            .position_to_offset(TextPosition::new(line as u32, position.column));
        self.selections = Selections::single(Selection::cursor(offset));
        if let Some(syntax) = &mut self.syntax {
            // Tout le texte a changé : on repart d'une analyse complète.
            let edits = self.buffer.take_byte_edits();
            syntax.update(self.buffer.rope(), self.buffer.version(), edits);
        } else {
            self.buffer.take_byte_edits();
        }
    }

    /// Le fichier a été supprimé du disque : le document devient modifié (le
    /// prochain enregistrement le recrée).
    pub fn mark_deleted_on_disk(&mut self) {
        self.disk = None;
        self.buffer.mark_unsaved();
    }

    // ----- Sélections et déplacements -----------------------------------------

    pub fn set_selections(&mut self, selections: Selections) {
        self.buffer.seal();
        self.selections = selections.clamp(self.buffer.len_chars());
    }

    /// Place le curseur (clic) ; étend la sélection si `extend` (Maj+clic).
    pub fn set_cursor(&mut self, offset: usize, extend: bool) {
        let offset = offset.min(self.buffer.len_chars());
        let primary = self.selections.primary().moved_to(offset, extend);
        self.set_selections(Selections::single(primary));
    }

    pub fn select_all(&mut self) {
        let len = self.buffer.len_chars();
        self.set_selections(Selections::single(Selection::new(0, len)));
    }

    /// Sélectionne le mot sous `offset` (double-clic).
    pub fn select_word_at(&mut self, offset: usize) {
        let range = movement::word_at(&self.buffer, offset);
        self.set_selections(Selections::single(Selection::new(range.start, range.end)));
    }

    /// Plage de la ligne `line`, fin de ligne comprise (triple-clic).
    pub fn line_range(&self, line: usize) -> Range<usize> {
        let line = line.min(self.buffer.len_lines() - 1);
        let start = self.buffer.line_start(line);
        let end = if line + 1 < self.buffer.len_lines() {
            self.buffer.line_start(line + 1)
        } else {
            self.buffer.len_chars()
        };
        start..end
    }

    pub fn select_line(&mut self, line: usize) {
        let range = self.line_range(line);
        self.set_selections(Selections::single(Selection::new(range.start, range.end)));
    }

    pub fn move_cursor(&mut self, motion: Motion, extend: bool) {
        let buffer = &self.buffer;
        let selections = self.selections.map(|selection| {
            let head = selection.head;
            // Flèche gauche/droite sans Maj sur une sélection : elle se réduit
            // à son bord.
            if !extend && !selection.is_empty() {
                match motion {
                    Motion::Left => return Selection::cursor(selection.start()),
                    Motion::Right => return Selection::cursor(selection.end()),
                    _ => {}
                }
            }
            let vertical = |lines: isize| -> Selection {
                let line = buffer.line_of(head);
                let column = head - buffer.line_start(line);
                let goal = selection
                    .goal_column
                    .unwrap_or_else(|| visual_column(buffer, line, column));
                let target = line as isize + lines;
                let offset = if target < 0 {
                    0
                } else if target as usize >= buffer.len_lines() {
                    buffer.len_chars()
                } else {
                    let target = target as usize;
                    buffer.line_start(target) + column_for_visual(buffer, target, goal)
                };
                let mut moved = selection.moved_to(offset, extend);
                moved.goal_column = Some(goal);
                moved
            };
            let offset = match motion {
                Motion::Left => prev_grapheme(buffer, head),
                Motion::Right => next_grapheme(buffer, head),
                Motion::WordLeft => word_left(buffer, head),
                Motion::WordRight => word_right(buffer, head),
                Motion::Up => return vertical(-1),
                Motion::Down => return vertical(1),
                Motion::PageUp(lines) => return vertical(-(lines.max(1) as isize)),
                Motion::PageDown(lines) => return vertical(lines.max(1) as isize),
                Motion::LineStart => {
                    let line = buffer.line_of(head);
                    let start = buffer.line_start(line);
                    let indent = buffer
                        .line(line)
                        .chars()
                        .take_while(|c| *c == ' ' || *c == '\t')
                        .count();
                    if head == start + indent {
                        start
                    } else {
                        start + indent
                    }
                }
                Motion::LineEnd => buffer.line_end(buffer.line_of(head)),
                Motion::DocumentStart => 0,
                Motion::DocumentEnd => buffer.len_chars(),
            };
            selection.moved_to(offset, extend)
        });
        self.set_selections(selections);
    }

    // ----- Modifications -----------------------------------------------------

    /// Applique des remplacements triés (texte d'avant) ; rend `false` si rien
    /// n'a changé. Les sélections sans remplacement suivent le texte.
    fn transact(
        &mut self,
        edits: Vec<Edit>,
        kind: EditKind,
        selections: Option<Selections>,
    ) -> bool {
        if self.read_only.is_some() || edits.is_empty() {
            return false;
        }
        let before = self.selections.clone();
        let mapped: Vec<(Range<usize>, String, usize)> = edits
            .iter()
            .map(|e| (e.range.clone(), e.text.clone(), e.text.chars().count()))
            .collect();
        let after = selections.unwrap_or_else(|| {
            before.map(|selection| {
                let owner = edits.iter().position(|e| {
                    e.range.start <= selection.start() && selection.end() <= e.range.end
                });
                match owner {
                    Some(i) => {
                        let edit = &edits[i];
                        let start = map_through(&mapped, edit.range.start, Bias::Before);
                        let cursor = edit.cursor.unwrap_or(mapped[i].2);
                        Selection::cursor(start + cursor)
                    }
                    // Un texte inséré contre la sélection reste dehors.
                    None if selection.is_empty() => {
                        Selection::cursor(map_through(&mapped, selection.head, Bias::After))
                    }
                    None => {
                        let start = map_through(&mapped, selection.start(), Bias::After);
                        let end = map_through(&mapped, selection.end(), Bias::Before).max(start);
                        if selection.head >= selection.anchor {
                            Selection::new(start, end)
                        } else {
                            Selection::new(end, start)
                        }
                    }
                }
            })
        });
        let edits: Vec<(Range<usize>, String)> =
            edits.into_iter().map(|e| (e.range, e.text)).collect();
        let now = (self.clock)();
        let version = self.buffer.version();
        self.buffer.apply(&edits, kind, &before, after.clone(), now);
        if self.buffer.version() == version {
            return false;
        }
        self.selections = after.clamp(self.buffer.len_chars());
        self.after_change();
        true
    }

    /// Un remplacement par sélection ; ceux qui chevauchent le précédent sont
    /// ignorés.
    fn edit_each(
        &mut self,
        kind: EditKind,
        mut f: impl FnMut(&Self, Selection) -> Option<Edit>,
    ) -> bool {
        let mut edits: Vec<Edit> = Vec::new();
        for selection in self.selections.iter() {
            if let Some(edit) = f(self, *selection) {
                if edits
                    .last()
                    .is_none_or(|last| last.range.end <= edit.range.start)
                {
                    edits.push(edit);
                }
            }
        }
        self.transact(edits, kind, None)
    }

    /// Frappe : remplace chaque sélection par `text`. Les frappes successives
    /// forment une seule étape d'annulation. Un `}` tapé seul sur une ligne
    /// blanche retire un cran d'indentation.
    pub fn insert_text(&mut self, text: &str) -> bool {
        if text.is_empty() {
            return false;
        }
        let text = self.normalize_line_endings(text);
        if text.contains(['\n', '\r']) {
            return self.paste(&text);
        }
        let closer = matches!(text.as_str(), "}" | "]" | ")");
        let unit_width = self.indent.width();
        self.edit_each(EditKind::Typing, |doc, selection| {
            if closer && selection.is_empty() {
                let line = doc.buffer.line_of(selection.head);
                let start = doc.buffer.line_start(line);
                let before: String = doc.buffer.slice(start..selection.head).to_string();
                if !before.is_empty() && before.chars().all(|c| c == ' ' || c == '\t') {
                    let width = visual_width(&before);
                    let target = width.saturating_sub(unit_width) / unit_width * unit_width;
                    let indent = doc.indent_to(target);
                    return Some(Edit {
                        range: start..selection.head,
                        text: format!("{indent}{text}"),
                        cursor: None,
                    });
                }
            }
            Some(Edit {
                range: selection.range(),
                text: text.clone(),
                cursor: None,
            })
        })
    }

    /// Collage : une étape à lui seul ; les fins de ligne sont ramenées à
    /// celles du document.
    pub fn paste(&mut self, text: &str) -> bool {
        let text = self.normalize_line_endings(text);
        self.buffer.seal();
        let changed = self.edit_each(EditKind::Other, |_, selection| {
            Some(Edit {
                range: selection.range(),
                text: text.clone(),
                cursor: None,
            })
        });
        self.buffer.seal();
        changed
    }

    /// Entrée : nouvelle ligne avec l'indentation de la ligne courante, un cran
    /// de plus après `{`, `[` ou `(` ; le fermant qui suit passe sur sa ligne.
    pub fn insert_newline(&mut self) -> bool {
        let new_line = self.new_line;
        let unit = self.indent.unit();
        self.edit_each(EditKind::Other, |doc, selection| {
            let buffer = &doc.buffer;
            let head = selection.start();
            let line = buffer.line_of(head);
            let start = buffer.line_start(line);
            let indent: String = buffer
                .line(line)
                .chars()
                .take(head - start)
                .take_while(|c| *c == ' ' || *c == '\t')
                .collect();
            let before = (head > start).then(|| buffer.rope().char(head - 1));
            let after = buffer.char_at(selection.end());
            let opens = matches!(before, Some('{' | '[' | '('));
            let closes = matches!(
                (before, after),
                (Some('{'), Some('}')) | (Some('['), Some(']')) | (Some('('), Some(')'))
            );
            let (text, cursor) = if closes {
                let first = format!("{new_line}{indent}{unit}");
                let cursor = first.chars().count();
                (format!("{first}{new_line}{indent}"), Some(cursor))
            } else if opens {
                (format!("{new_line}{indent}{unit}"), None)
            } else {
                (format!("{new_line}{indent}"), None)
            };
            Some(Edit {
                range: selection.range(),
                text,
                cursor,
            })
        })
    }

    /// Retour arrière.
    pub fn backspace(&mut self) -> bool {
        self.edit_each(EditKind::Deleting, |doc, selection| {
            let range = if selection.is_empty() {
                prev_grapheme(&doc.buffer, selection.head)..selection.head
            } else {
                selection.range()
            };
            (!range.is_empty()).then(|| Edit {
                range,
                text: String::new(),
                cursor: None,
            })
        })
    }

    /// Suppr.
    pub fn delete_forward(&mut self) -> bool {
        self.edit_each(EditKind::Deleting, |doc, selection| {
            let range = if selection.is_empty() {
                selection.head..next_grapheme(&doc.buffer, selection.head)
            } else {
                selection.range()
            };
            (!range.is_empty()).then(|| Edit {
                range,
                text: String::new(),
                cursor: None,
            })
        })
    }

    /// Ctrl+Retour arrière.
    pub fn delete_word_back(&mut self) -> bool {
        self.edit_each(EditKind::Deleting, |doc, selection| {
            let range = if selection.is_empty() {
                word_left(&doc.buffer, selection.head)..selection.head
            } else {
                selection.range()
            };
            (!range.is_empty()).then(|| Edit {
                range,
                text: String::new(),
                cursor: None,
            })
        })
    }

    /// Ctrl+Suppr.
    pub fn delete_word_forward(&mut self) -> bool {
        self.edit_each(EditKind::Deleting, |doc, selection| {
            let range = if selection.is_empty() {
                selection.head..word_right(&doc.buffer, selection.head)
            } else {
                selection.range()
            };
            (!range.is_empty()).then(|| Edit {
                range,
                text: String::new(),
                cursor: None,
            })
        })
    }

    pub fn undo(&mut self) -> bool {
        if self.read_only.is_some() {
            return false;
        }
        self.buffer.seal();
        match self.buffer.undo() {
            Some(selections) => {
                self.selections = selections.clamp(self.buffer.len_chars());
                self.after_change();
                true
            }
            None => false,
        }
    }

    pub fn redo(&mut self) -> bool {
        if self.read_only.is_some() {
            return false;
        }
        match self.buffer.redo() {
            Some(selections) => {
                self.selections = selections.clamp(self.buffer.len_chars());
                self.after_change();
                true
            }
            None => false,
        }
    }

    // ----- Presse-papiers ----------------------------------------------------

    /// Texte à copier : les sélections (jointes par des fins de ligne), ou à
    /// défaut les lignes des curseurs entières, fin de ligne comprise.
    pub fn copy_text(&self) -> Option<String> {
        if self.selections.iter().any(|s| !s.is_empty()) {
            let parts: Vec<String> = self
                .selections
                .iter()
                .filter(|s| !s.is_empty())
                .map(|s| self.buffer.slice(s.range()).to_string())
                .collect();
            return Some(parts.join(self.new_line));
        }
        let mut text = String::new();
        for (first, last) in self.line_blocks() {
            let start = self.buffer.line_start(first);
            let end = self.line_range(last).end;
            text.push_str(&self.buffer.slice(start..end).to_string());
            if !text.ends_with(['\n', '\r']) {
                text.push_str(self.new_line);
            }
        }
        (!text.is_empty()).then_some(text)
    }

    /// Couper : comme [`Self::copy_text`], puis supprime ce qui a été copié.
    pub fn cut(&mut self) -> Option<String> {
        if self.read_only.is_some() {
            return None;
        }
        let text = self.copy_text()?;
        if self.selections.iter().any(|s| !s.is_empty()) {
            self.buffer.seal();
            self.edit_each(EditKind::Other, |_, selection| {
                (!selection.is_empty()).then(|| Edit {
                    range: selection.range(),
                    text: String::new(),
                    cursor: None,
                })
            });
            self.buffer.seal();
        } else {
            self.delete_lines();
        }
        Some(text)
    }

    // ----- Opérations sur les lignes -------------------------------------------

    /// Blocs de lignes touchés par les sélections (première, dernière), fusionnés
    /// quand ils se touchent. Une sélection qui finit en colonne 0 n'inclut pas
    /// cette ligne.
    fn line_blocks(&self) -> Vec<(usize, usize)> {
        let mut blocks: Vec<(usize, usize)> = Vec::new();
        for selection in self.selections.iter() {
            let first = self.buffer.line_of(selection.start());
            let mut last = self.buffer.line_of(selection.end());
            if !selection.is_empty()
                && last > first
                && self.buffer.line_start(last) == selection.end()
            {
                last -= 1;
            }
            match blocks.last_mut() {
                Some(block) if first <= block.1 + 1 => block.1 = block.1.max(last),
                _ => blocks.push((first, last)),
            }
        }
        blocks
    }

    /// Positions (ligne, colonne) des sélections, pour les recalculer après une
    /// opération sur les lignes.
    fn selection_positions(&self) -> Vec<(TextPosition, TextPosition)> {
        self.selections
            .iter()
            .map(|s| {
                (
                    self.buffer.offset_to_position(s.anchor),
                    self.buffer.offset_to_position(s.head),
                )
            })
            .collect()
    }

    /// Applique `edits` puis replace les sélections aux positions `positions`.
    fn transact_lines(
        &mut self,
        edits: Vec<Edit>,
        positions: Vec<(TextPosition, TextPosition)>,
    ) -> bool {
        if self.read_only.is_some() || edits.is_empty() {
            return false;
        }
        // Les sélections finales se calculent sur le texte d'après : on applique
        // d'abord avec des sélections provisoires, puis on corrige l'étape.
        let primary = self.selections.primary_index();
        self.buffer.seal();
        let changed = self.transact(edits, EditKind::Other, Some(self.selections.clone()));
        if changed {
            let selections = Selections::from_vec(
                positions
                    .into_iter()
                    .map(|(a, h)| {
                        Selection::new(
                            self.buffer.position_to_offset(a),
                            self.buffer.position_to_offset(h),
                        )
                    })
                    .collect(),
                primary,
            );
            self.selections = selections.clone();
            self.buffer.set_last_after(selections);
        }
        changed
    }

    /// Ctrl+Y : supprime les lignes des sélections.
    pub fn delete_lines(&mut self) -> bool {
        let blocks = self.line_blocks();
        let len_lines = self.buffer.len_lines();
        let mut edits = Vec::new();
        let mut positions = Vec::new();
        let mut removed_before = 0usize;
        for &(first, last) in &blocks {
            let range = if last + 1 < len_lines {
                self.buffer.line_start(first)..self.buffer.line_start(last + 1)
            } else if first > 0 {
                self.buffer.line_end(first - 1)..self.buffer.len_chars()
            } else {
                0..self.buffer.len_chars()
            };
            let target_line = if last + 1 < len_lines || first == 0 {
                first - removed_before
            } else {
                first - 1 - removed_before
            };
            for (_, head) in self.selection_positions() {
                let line = head.line as usize;
                if line >= first && line <= last {
                    let p = TextPosition::new(target_line as u32, head.column);
                    positions.push((p, p));
                }
            }
            removed_before += last - first + 1;
            edits.push(Edit {
                range,
                text: String::new(),
                cursor: None,
            });
        }
        self.transact_lines(edits, positions)
    }

    /// Ctrl+D : duplique la sélection, ou à défaut les lignes des curseurs.
    pub fn duplicate(&mut self) -> bool {
        if self.selections.iter().any(|s| !s.is_empty()) {
            self.buffer.seal();
            let changed = self.edit_each(EditKind::Other, |doc, selection| {
                let text = doc.buffer.slice(selection.range()).to_string();
                Some(Edit {
                    range: selection.end()..selection.end(),
                    text,
                    cursor: Some(0),
                })
            });
            self.buffer.seal();
            return changed;
        }
        let blocks = self.line_blocks();
        let positions_before = self.selection_positions();
        let mut edits = Vec::new();
        let mut positions = Vec::new();
        let mut added_before = 0usize;
        for &(first, last) in &blocks {
            let start = self.buffer.line_start(first);
            let end = self.buffer.line_end(last);
            let copy = self.buffer.slice(start..end).to_string();
            let count = last - first + 1;
            for (anchor, head) in &positions_before {
                if (head.line as usize) >= first && (head.line as usize) <= last {
                    let shift = (added_before + count) as u32;
                    positions.push((
                        TextPosition::new(anchor.line + shift, anchor.column),
                        TextPosition::new(head.line + shift, head.column),
                    ));
                }
            }
            added_before += count;
            edits.push(Edit {
                range: end..end,
                text: format!("{}{copy}", self.new_line),
                cursor: None,
            });
        }
        self.transact_lines(edits, positions)
    }

    /// Alt+Maj+Haut / Bas : déplace les lignes des sélections d'un cran.
    pub fn move_lines(&mut self, up: bool) -> bool {
        let blocks = self.line_blocks();
        let len_lines = self.buffer.len_lines();
        if blocks.is_empty()
            || (up && blocks[0].0 == 0)
            || (!up && blocks.last().is_some_and(|b| b.1 + 1 >= len_lines))
        {
            return false;
        }
        let positions_before = self.selection_positions();
        let mut edits = Vec::new();
        for &(first, last) in &blocks {
            let (top, bottom) = if up {
                (first - 1, last)
            } else {
                (first, last + 1)
            };
            let contents: Vec<String> = (top..=bottom)
                .map(|line| self.buffer.line(line).to_string())
                .collect();
            let endings: Vec<&str> = (top..bottom)
                .map(|line| self.buffer.line_ending(line))
                .collect();
            let mut order: Vec<usize> = (0..contents.len()).collect();
            if up {
                order.rotate_left(1);
            } else {
                order.rotate_right(1);
            }
            let mut text = String::new();
            for (i, &index) in order.iter().enumerate() {
                text.push_str(&contents[index]);
                if let Some(ending) = endings.get(i) {
                    // Une ligne sans fin dans un fichier mélangé : la fin du document.
                    text.push_str(if ending.is_empty() {
                        self.new_line
                    } else {
                        ending
                    });
                }
            }
            edits.push(Edit {
                range: self.buffer.line_start(top)..self.buffer.line_end(bottom),
                text,
                cursor: None,
            });
        }
        let positions = positions_before
            .into_iter()
            .map(|(anchor, head)| {
                let shift = |p: TextPosition| {
                    let in_block = blocks
                        .iter()
                        .any(|&(f, l)| (p.line as usize) >= f && (p.line as usize) <= l);
                    match (in_block, up) {
                        (true, true) => TextPosition::new(p.line - 1, p.column),
                        (true, false) => TextPosition::new(p.line + 1, p.column),
                        _ => p,
                    }
                };
                (shift(anchor), shift(head))
            })
            .collect();
        self.transact_lines(edits, positions)
    }

    /// Ctrl+/ : commente ou décommente les lignes des sélections. Sans
    /// sélection, le curseur passe ensuite à la ligne suivante.
    pub fn toggle_line_comment(&mut self) -> bool {
        let line_prefix = self.language.line_comment();
        let block = self.language.block_comment();
        if line_prefix.is_none() && block.is_none() {
            return false;
        }
        let single_cursor = self.selections.len() == 1 && self.selections.primary().is_empty();
        let blocks = self.line_blocks();
        let mut edits = Vec::new();
        // Décalage de colonne par ligne : (ligne, colonne où ça commence, delta).
        let mut shifts: Vec<(usize, u32, i64)> = Vec::new();
        for &(first, last) in &blocks {
            let lines: Vec<usize> = (first..=last)
                .filter(|&l| first == last || !self.buffer.line(l).to_string().trim().is_empty())
                .collect();
            if let Some(prefix) = line_prefix {
                let commented = lines.iter().all(|&l| {
                    self.buffer
                        .line(l)
                        .to_string()
                        .trim_start()
                        .starts_with(prefix)
                });
                for &line in &lines {
                    let start = self.buffer.line_start(line);
                    let content = self.buffer.line(line).to_string();
                    if commented {
                        let indent = content.chars().take_while(|c| c.is_whitespace()).count();
                        let at = start + indent;
                        let len = prefix.chars().count();
                        edits.push(Edit {
                            range: at..at + len,
                            text: String::new(),
                            cursor: None,
                        });
                        shifts.push((line, indent as u32, -(len as i64)));
                    } else {
                        edits.push(Edit {
                            range: start..start,
                            text: prefix.to_owned(),
                            cursor: None,
                        });
                        shifts.push((line, 0, prefix.chars().count() as i64));
                    }
                }
            } else if let Some((open, close)) = block {
                let commented = lines.iter().all(|&l| {
                    let content = self.buffer.line(l).to_string();
                    let trimmed = content.trim();
                    trimmed.starts_with(open)
                        && trimmed.ends_with(close)
                        && trimmed.len() >= open.len() + close.len()
                });
                for &line in &lines {
                    let start = self.buffer.line_start(line);
                    let content = self.buffer.line(line).to_string();
                    let chars: Vec<char> = content.chars().collect();
                    let indent = chars.iter().take_while(|c| c.is_whitespace()).count();
                    let trailing = chars.iter().rev().take_while(|c| c.is_whitespace()).count();
                    let content_end = chars.len() - trailing;
                    if commented {
                        let open_len = open.chars().count();
                        let close_len = close.chars().count();
                        edits.push(Edit {
                            range: start + indent..start + indent + open_len,
                            text: String::new(),
                            cursor: None,
                        });
                        edits.push(Edit {
                            range: start + content_end - close_len..start + content_end,
                            text: String::new(),
                            cursor: None,
                        });
                        shifts.push((line, indent as u32, -(open_len as i64)));
                    } else {
                        edits.push(Edit {
                            range: start + indent..start + indent,
                            text: open.to_owned(),
                            cursor: None,
                        });
                        edits.push(Edit {
                            range: start + content_end..start + content_end,
                            text: close.to_owned(),
                            cursor: None,
                        });
                        shifts.push((line, indent as u32, open.chars().count() as i64));
                    }
                }
            }
        }
        edits.sort_by_key(|e| e.range.start);
        let len_lines = self.buffer.len_lines();
        let positions = self
            .selection_positions()
            .into_iter()
            .map(|(anchor, head)| {
                let shift = |p: TextPosition| {
                    if single_cursor {
                        let line = (p.line as usize + 1).min(len_lines - 1);
                        return TextPosition::new(line as u32, p.column);
                    }
                    let delta = shifts
                        .iter()
                        .find(|(l, from, _)| *l == p.line as usize && p.column >= *from)
                        .map_or(0, |s| s.2);
                    TextPosition::new(p.line, (i64::from(p.column) + delta).max(0) as u32)
                };
                (shift(anchor), shift(head))
            })
            .collect();
        self.transact_lines(edits, positions)
    }

    /// Tab : insère une indentation au curseur, ou indente les lignes d'une
    /// sélection.
    pub fn indent(&mut self) -> bool {
        if self.selections.iter().all(Selection::is_empty) {
            let width = self.indent.width();
            let tabs = self.indent == IndentStyle::Tabs;
            return self.edit_each(EditKind::Typing, |doc, selection| {
                let line = doc.buffer.line_of(selection.head);
                let column = selection.head - doc.buffer.line_start(line);
                let text = if tabs {
                    "\t".to_owned()
                } else {
                    let visual = visual_column(&doc.buffer, line, column);
                    " ".repeat((width - visual % width) as usize)
                };
                Some(Edit {
                    range: selection.range(),
                    text,
                    cursor: None,
                })
            });
        }
        let unit = self.indent.unit();
        let unit_len = unit.chars().count() as i64;
        let mut edits = Vec::new();
        let mut indented = Vec::new();
        for (first, last) in self.line_blocks() {
            for line in first..=last {
                if self.buffer.line_len(line) == 0 {
                    continue;
                }
                let start = self.buffer.line_start(line);
                edits.push(Edit {
                    range: start..start,
                    text: unit.clone(),
                    cursor: None,
                });
                indented.push(line);
            }
        }
        let positions = self.shift_positions(&indented, unit_len);
        self.transact_lines(edits, positions)
    }

    /// Maj+Tab : retire un cran d'indentation aux lignes des sélections.
    pub fn outdent(&mut self) -> bool {
        let width = self.indent.width();
        let mut edits = Vec::new();
        let mut shifts: Vec<(usize, i64)> = Vec::new();
        for (first, last) in self.line_blocks() {
            for line in first..=last {
                let content = self.buffer.line(line).to_string();
                let mut visual = 0u32;
                let mut count = 0usize;
                for c in content.chars() {
                    if visual >= width || !(c == ' ' || c == '\t') {
                        break;
                    }
                    visual = movement::advance(visual, c);
                    count += 1;
                }
                if count > 0 {
                    let start = self.buffer.line_start(line);
                    edits.push(Edit {
                        range: start..start + count,
                        text: String::new(),
                        cursor: None,
                    });
                    shifts.push((line, -(count as i64)));
                }
            }
        }
        let positions = self
            .selection_positions()
            .into_iter()
            .map(|(a, h)| {
                let shift = |p: TextPosition| {
                    let delta = shifts
                        .iter()
                        .find(|(l, _)| *l == p.line as usize)
                        .map_or(0, |s| s.1);
                    TextPosition::new(p.line, (i64::from(p.column) + delta).max(0) as u32)
                };
                (shift(a), shift(h))
            })
            .collect();
        self.transact_lines(edits, positions)
    }

    /// Positions des sélections après un ajout de `delta` caractères en tête des
    /// lignes `lines` (une position en colonne 0 y reste).
    fn shift_positions(&self, lines: &[usize], delta: i64) -> Vec<(TextPosition, TextPosition)> {
        self.selection_positions()
            .into_iter()
            .map(|(a, h)| {
                let shift = |p: TextPosition| {
                    if p.column > 0 && lines.contains(&(p.line as usize)) {
                        TextPosition::new(p.line, (i64::from(p.column) + delta).max(0) as u32)
                    } else {
                        p
                    }
                };
                (shift(a), shift(h))
            })
            .collect()
    }

    // ----- Divers -------------------------------------------------------------

    /// Parenthèse, crochet ou accolade appariée à celle qui touche `offset`
    /// (juste après ou juste avant) : `(position de celle-ci, position de l'autre)`.
    /// Recherche bornée à 100 000 caractères ; chaînes et commentaires ne sont pas
    /// distingués.
    pub fn matching_bracket(&self, offset: usize) -> Option<(usize, usize)> {
        const LIMIT: usize = 100_000;
        let candidates = [offset.checked_sub(1), Some(offset)];
        for at in candidates.into_iter().flatten() {
            let Some(c) = self.buffer.char_at(at) else {
                continue;
            };
            let (open, close, forward) = match c {
                '(' => ('(', ')', true),
                '[' => ('[', ']', true),
                '{' => ('{', '}', true),
                ')' => ('(', ')', false),
                ']' => ('[', ']', false),
                '}' => ('{', '}', false),
                _ => continue,
            };
            let rope = self.buffer.rope();
            let mut depth = 0usize;
            if forward {
                let end = (at + LIMIT).min(rope.len_chars());
                for (i, ch) in rope.slice(at..end).chars().enumerate() {
                    if ch == open {
                        depth += 1;
                    } else if ch == close {
                        depth -= 1;
                        if depth == 0 {
                            return Some((at, at + i));
                        }
                    }
                }
            } else {
                let start = at.saturating_sub(LIMIT);
                let slice = rope.slice(start..=at);
                let mut chars = slice.chars_at(slice.len_chars());
                let mut i = at + 1;
                while let Some(ch) = chars.prev() {
                    i -= 1;
                    if ch == close {
                        depth += 1;
                    } else if ch == open {
                        depth -= 1;
                        if depth == 0 {
                            return Some((at, i));
                        }
                    }
                }
            }
        }
        None
    }

    fn normalize_line_endings(&self, text: &str) -> String {
        if !text.contains('\r') && self.new_line == "\n" {
            return text.to_owned();
        }
        let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
        if self.new_line == "\n" {
            normalized
        } else {
            normalized.replace('\n', self.new_line)
        }
    }

    /// Indentation de largeur visuelle `width`, dans le style du fichier.
    fn indent_to(&self, width: u32) -> String {
        match self.indent {
            IndentStyle::Tabs => {
                let mut s = "\t".repeat((width / TAB_WIDTH) as usize);
                s.push_str(&" ".repeat((width % TAB_WIDTH) as usize));
                s
            }
            IndentStyle::Spaces(_) => " ".repeat(width as usize),
        }
    }
}

fn visual_width(text: &str) -> u32 {
    text.chars().fold(0, movement::advance)
}

/// Où va `offset` (texte d'avant) une fois appliqués des remplacements triés.
fn map_through(edits: &[(Range<usize>, String, usize)], offset: usize, bias: Bias) -> usize {
    let mut delta: isize = 0;
    for (range, _, inserted) in edits {
        if offset < range.start || (offset == range.start && bias == Bias::Before) {
            break;
        }
        if offset >= range.end && offset > range.start {
            delta += *inserted as isize - range.len() as isize;
            continue;
        }
        let base = (range.start as isize + delta) as usize;
        return if bias == Bias::After {
            base + inserted
        } else {
            base
        };
    }
    (offset as isize + delta).max(0) as usize
}

/// Fin de ligne des nouvelles lignes : `\n` pour un fichier homogène (le texte en
/// mémoire n'a que des `\n`), la plus fréquente pour un fichier mélangé.
fn dominant_line_ending(text: &str, line_ending: LineEnding) -> &'static str {
    if line_ending != LineEnding::Mixed {
        return "\n";
    }
    let bytes = text.as_bytes();
    let (mut lf, mut crlf, mut cr) = (0usize, 0usize, 0usize);
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\r' if bytes.get(i + 1) == Some(&b'\n') => {
                crlf += 1;
                i += 1;
            }
            b'\r' => cr += 1,
            b'\n' => lf += 1,
            _ => {}
        }
        i += 1;
    }
    if crlf >= lf && crlf >= cr {
        "\r\n"
    } else if lf >= cr {
        "\n"
    } else {
        "\r"
    }
}
