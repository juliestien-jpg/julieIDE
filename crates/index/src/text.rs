//! Recherche dans les fichiers : texte ou expression régulière, casse, mot
//! entier, masque de fichiers, portée ; aperçu avec contexte.

use std::collections::HashMap;
use std::io::Read;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use julie_core::{CancelToken, TextPosition, TextRange};
use regex::{Regex, RegexBuilder};

use crate::error::{IndexError, Result};
use crate::files::{absolute_path, relative_string, FileIndex};
use crate::settings::IndexSettings;

/// Nombre d'occurrences au-delà duquel la recherche s'arrête.
pub const DEFAULT_MAX_MATCHES: usize = 10_000;
/// Octets examinés pour reconnaître un fichier binaire (comme Git).
const BINARY_PROBE: usize = 8_000;
/// Une ligne plus longue (en caractères) est rendue en extrait autour de la
/// première occurrence.
const SNIPPET_MAX_CHARS: usize = 400;
const SNIPPET_BEFORE_CHARS: usize = 80;

/// Contenus en mémoire des fichiers modifiés et non enregistrés, par chemin
/// absolu. Le texte est celui de l'éditeur (fins de ligne `\n`). La recherche et
/// l'aperçu les lisent à la place du disque.
pub type UnsavedFiles = HashMap<PathBuf, Arc<str>>;

/// Où chercher.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SearchScope {
    #[default]
    Project,
    /// Un dossier du projet (chemin absolu).
    Directory(PathBuf),
}

/// Une demande de recherche dans les fichiers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextQuery {
    pub pattern: String,
    /// `pattern` est une expression régulière (syntaxe du crate `regex`) ; sinon
    /// un texte littéral.
    pub regex: bool,
    pub case_sensitive: bool,
    /// Mot entier : l'occurrence n'est ni précédée ni suivie d'une lettre, d'un
    /// chiffre ou de `_` (même règle que `git grep -w`).
    pub whole_word: bool,
    /// Masques séparés par des virgules (`*.php, *.js`), vide = tous. Un masque
    /// sans `/` porte sur le nom, avec `/` sur le chemin relatif ; `!` en tête
    /// exclut (`!*.min.js`). Sans casse.
    pub file_mask: String,
    pub scope: SearchScope,
    /// Arrêt après ce nombre d'occurrences (`DEFAULT_MAX_MATCHES`).
    pub max_matches: usize,
}

impl TextQuery {
    pub fn new(pattern: impl Into<String>) -> Self {
        Self {
            pattern: pattern.into(),
            regex: false,
            case_sensitive: false,
            whole_word: false,
            file_mask: String::new(),
            scope: SearchScope::Project,
            max_matches: DEFAULT_MAX_MATCHES,
        }
    }
}

/// Le motif compilé. Se construit dans le fil de l'interface (rapide) pour
/// signaler tout de suite une expression invalide.
#[derive(Debug, Clone)]
pub struct Matcher {
    regex: Regex,
    whole_word: bool,
}

impl Matcher {
    pub fn new(query: &TextQuery) -> Result<Self> {
        let source = if query.regex {
            query.pattern.clone()
        } else {
            regex::escape(&query.pattern)
        };
        let regex = RegexBuilder::new(&source)
            .case_insensitive(!query.case_sensitive)
            .multi_line(true)
            .crlf(true)
            .build()
            .map_err(|error| IndexError::InvalidPattern {
                message: error.to_string(),
            })?;
        Ok(Self {
            regex,
            whole_word: query.whole_word,
        })
    }

    /// Occurrences dans une ligne (sans fin de ligne), en plages d'**octets**.
    /// Les correspondances vides sont écartées.
    pub fn find_in_line(&self, line: &str) -> Vec<Range<usize>> {
        let mut out = Vec::new();
        let mut at = 0;
        while at <= line.len() {
            let Some(found) = self.regex.find_at(line, at) else {
                break;
            };
            if found.is_empty() {
                at = next_boundary(line, found.end());
                continue;
            }
            if self.whole_word && !is_whole_word(line, found.start(), found.end()) {
                at = next_boundary(line, found.start());
                continue;
            }
            out.push(found.range());
            at = found.end();
        }
        out
    }

    /// Toutes les lignes d'un texte qui contiennent une occurrence, au plus
    /// `max_matches` occurrences en tout. Lignes séparées par `\n` ; un `\r` en
    /// fin de ligne n'en fait pas partie (une regex `$` s'arrête avant).
    pub fn search_text(&self, text: &str, max_matches: usize) -> Vec<LineMatch> {
        let bytes = text.as_bytes();
        let mut out = Vec::new();
        let mut total = 0;
        let mut pos = 0;
        let mut line_number = 0usize;
        let mut counted_to = 0;
        while pos <= text.len() && total < max_matches {
            // Recherche sur tout le texte pour sauter vite les lignes sans
            // occurrence, puis vérification sur la ligne seule.
            let Some(found) = self.regex.find_at(text, pos) else {
                break;
            };
            let start = found.start();
            let line_start = memchr::memrchr(b'\n', &bytes[..start]).map_or(0, |i| i + 1);
            let line_end = memchr::memchr(b'\n', &bytes[start..]).map_or(text.len(), |i| start + i);
            let line_start = line_start.max(pos);
            line_number += memchr::memchr_iter(b'\n', &bytes[counted_to..line_start]).count();
            counted_to = line_start;
            let line = &text[line_start..line_end];
            let line = line.strip_suffix('\r').unwrap_or(line);
            let mut ranges = self.find_in_line(line);
            if !ranges.is_empty() {
                ranges.truncate(max_matches - total);
                total += ranges.len();
                out.push(LineMatch::new(line_number as u32, line, &ranges));
            }
            if line_end >= text.len() {
                break;
            }
            pos = line_end + 1;
        }
        out
    }
}

fn next_boundary(text: &str, at: usize) -> usize {
    match text[at..].chars().next() {
        Some(c) => at + c.len_utf8(),
        None => text.len() + 1,
    }
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn is_whole_word(line: &str, start: usize, end: usize) -> bool {
    let before = line[..start].chars().next_back();
    let after = line[end..].chars().next();
    !before.is_some_and(is_word_char) && !after.is_some_and(is_word_char)
}

/// Une ligne qui contient au moins une occurrence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineMatch {
    /// Ligne, base zéro.
    pub line: u32,
    /// Les occurrences de la ligne, colonnes en **caractères** dans la ligne
    /// complète (prêtes pour sélectionner dans l'éditeur). Chacune compte pour
    /// une occurrence.
    pub matches: Vec<TextRange>,
    /// Le texte à afficher : la ligne entière, ou un extrait si elle dépasse 400
    /// caractères.
    pub snippet: String,
    /// Colonne (caractères) du début de l'extrait dans la ligne : 0 si la ligne
    /// est entière. Colonne d'une occurrence dans l'extrait = colonne − ceci.
    pub snippet_start: u32,
    /// Longueur de la ligne complète, en caractères (extrait tronqué à droite si
    /// `snippet_start + longueur de l'extrait < line_length`).
    pub line_length: u32,
}

impl LineMatch {
    fn new(line: u32, text: &str, ranges: &[Range<usize>]) -> Self {
        // Octets -> caractères, en un passage.
        let mut matches = Vec::with_capacity(ranges.len());
        let mut chars_before = 0usize;
        let mut byte_cursor = 0usize;
        for range in ranges {
            chars_before += text[byte_cursor..range.start].chars().count();
            let start = chars_before;
            let end = start + text[range.clone()].chars().count();
            chars_before = end;
            byte_cursor = range.end;
            matches.push(TextRange {
                start: TextPosition::new(line, start as u32),
                end: TextPosition::new(line, end as u32),
            });
        }
        let line_length = chars_before + text[byte_cursor..].chars().count();
        let (snippet, snippet_start) = if line_length <= SNIPPET_MAX_CHARS {
            (text.to_string(), 0)
        } else {
            let first = matches.first().map_or(0, |m| m.start.column as usize);
            let from = first.saturating_sub(SNIPPET_BEFORE_CHARS);
            let snippet: String = text.chars().skip(from).take(SNIPPET_MAX_CHARS).collect();
            (snippet, from)
        };
        Self {
            line,
            matches,
            snippet,
            snippet_start: snippet_start as u32,
            line_length: line_length as u32,
        }
    }
}

/// Les occurrences d'un fichier (toutes celles du fichier, dans l'ordre des
/// lignes, sauf à la limite globale).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileTextMatches {
    /// Chemin absolu.
    pub path: PathBuf,
    /// Chemin relatif, séparateurs `/`.
    pub relative: String,
    pub lines: Vec<LineMatch>,
    /// Cherché dans le contenu non enregistré fourni par l'appelant.
    pub unsaved: bool,
}

impl FileTextMatches {
    pub fn match_count(&self) -> usize {
        self.lines.iter().map(|l| l.matches.len()).sum()
    }
}

/// Bilan d'une recherche terminée.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TextSearchSummary {
    /// Occurrences (« n occurrences »).
    pub matches: usize,
    /// Lignes contenant au moins une occurrence.
    pub lines: usize,
    /// Fichiers contenant au moins une occurrence (« dans m fichiers »).
    pub files_with_matches: usize,
    /// Fichiers lus et examinés.
    pub files_searched: usize,
    /// Fichiers binaires ignorés (octet NUL dans les 8 000 premiers octets).
    pub skipped_binary: usize,
    /// Fichiers trop gros ignorés (« n fichiers trop gros ignorés »).
    pub skipped_too_large: usize,
    /// Fichiers illisibles (droits, disparus entre-temps).
    pub unreadable: usize,
    /// Arrêt à `max_matches` : « Plus de 10 000 résultats ».
    pub limit_reached: bool,
    pub elapsed: Duration,
}

/// Masque de fichiers compilé.
#[derive(Debug, Clone)]
pub struct FileMask {
    include_names: GlobSet,
    include_paths: GlobSet,
    exclude_names: GlobSet,
    exclude_paths: GlobSet,
    has_include: bool,
}

impl FileMask {
    pub fn parse(mask: &str) -> Result<Self> {
        let mut include_names = GlobSetBuilder::new();
        let mut include_paths = GlobSetBuilder::new();
        let mut exclude_names = GlobSetBuilder::new();
        let mut exclude_paths = GlobSetBuilder::new();
        let mut has_include = false;
        for raw in mask.split(',') {
            let raw = raw.trim();
            let (exclude, pattern) = match raw.strip_prefix('!') {
                Some(rest) => (true, rest.trim()),
                None => (false, raw),
            };
            if pattern.is_empty() {
                continue;
            }
            let pattern = pattern.replace('\\', "/");
            let glob = GlobBuilder::new(pattern.trim_start_matches('/'))
                .case_insensitive(true)
                .literal_separator(false)
                .build()
                .map_err(|error| IndexError::InvalidMask {
                    mask: raw.to_string(),
                    message: error.to_string(),
                })?;
            let on_path = pattern.contains('/');
            match (exclude, on_path) {
                (false, false) => {
                    include_names.add(glob);
                }
                (false, true) => {
                    include_paths.add(glob);
                }
                (true, false) => {
                    exclude_names.add(glob);
                }
                (true, true) => {
                    exclude_paths.add(glob);
                }
            }
            has_include |= !exclude;
        }
        let build = |builder: GlobSetBuilder| {
            builder.build().map_err(|error| IndexError::InvalidMask {
                mask: mask.to_string(),
                message: error.to_string(),
            })
        };
        Ok(Self {
            include_names: build(include_names)?,
            include_paths: build(include_paths)?,
            exclude_names: build(exclude_names)?,
            exclude_paths: build(exclude_paths)?,
            has_include,
        })
    }

    /// Le fichier (chemin relatif, `/`) passe-t-il le masque ?
    pub fn matches(&self, relative: &str) -> bool {
        let name = relative.rsplit('/').next().unwrap_or(relative);
        if self.exclude_names.is_match(name) || self.exclude_paths.is_match(relative) {
            return false;
        }
        !self.has_include
            || self.include_names.is_match(name)
            || self.include_paths.is_match(relative)
    }
}

/// Un fichier à examiner.
#[derive(Debug, Clone)]
pub struct SearchFile {
    pub path: PathBuf,
    pub relative: String,
    /// Contenu non enregistré, cherché à la place du disque.
    pub unsaved: Option<Arc<str>>,
}

/// Les fichiers à examiner : ceux de l'index dans la portée et le masque, plus
/// les fichiers non enregistrés du projet qui n'y seraient pas encore. Triés par
/// chemin relatif. Sans accès disque.
pub fn files_to_search(
    index: &FileIndex,
    query: &TextQuery,
    mask: &FileMask,
    unsaved: &UnsavedFiles,
) -> Result<Vec<SearchFile>> {
    let scope_prefix = match &query.scope {
        SearchScope::Project => None,
        SearchScope::Directory(dir) => {
            let relative = index
                .relative(dir)
                .ok_or_else(|| IndexError::OutsideProject(dir.clone()))?;
            if relative.is_empty() {
                None
            } else {
                Some(format!("{relative}/"))
            }
        }
    };
    let in_scope = |relative: &str| {
        scope_prefix
            .as_ref()
            .is_none_or(|p| relative.starts_with(p.as_str()))
    };

    let unsaved_by_relative: HashMap<String, &Arc<str>> = unsaved
        .iter()
        .filter_map(|(path, text)| Some((relative_string(index.root(), path)?, text)))
        .collect();

    let mut files: Vec<SearchFile> = index
        .entries()
        .iter()
        .filter(|e| in_scope(e.relative()) && mask.matches(e.relative()))
        .map(|e| SearchFile {
            path: index.absolute(e.relative()),
            relative: e.relative().to_string(),
            unsaved: unsaved_by_relative.get(e.relative()).map(|t| Arc::clone(t)),
        })
        .collect();
    for (relative, text) in &unsaved_by_relative {
        if relative.is_empty() || index.contains(relative) {
            continue;
        }
        if in_scope(relative) && mask.matches(relative) && !index.settings().is_excluded(relative) {
            files.push(SearchFile {
                path: absolute_path(index.root(), relative),
                relative: relative.clone(),
                unsaved: Some(Arc::clone(text)),
            });
        }
    }
    files.sort_unstable_by(|a, b| a.relative.cmp(&b.relative));
    Ok(files)
}

/// Options d'exécution de [`search_files`].
#[derive(Debug, Clone, Copy)]
pub struct SearchOptions {
    pub max_matches: usize,
    pub max_file_size: u64,
    pub threads: usize,
}

impl SearchOptions {
    pub fn new(query: &TextQuery, settings: &IndexSettings) -> Self {
        Self {
            max_matches: query.max_matches.max(1),
            max_file_size: settings.max_file_size,
            threads: settings.thread_count(),
        }
    }
}

enum Outcome {
    Matches(Vec<LineMatch>),
    NoMatch,
    Binary,
    TooLarge,
    Unreadable,
}

fn search_one(file: &SearchFile, matcher: &Matcher, options: &SearchOptions) -> Outcome {
    if let Some(text) = &file.unsaved {
        let lines = matcher.search_text(text, options.max_matches);
        return if lines.is_empty() {
            Outcome::NoMatch
        } else {
            Outcome::Matches(lines)
        };
    }
    let Ok(mut handle) = std::fs::File::open(&file.path) else {
        return Outcome::Unreadable;
    };
    let size = handle.metadata().map(|m| m.len()).unwrap_or(0);
    if size > options.max_file_size {
        return Outcome::TooLarge;
    }
    // Le début d'abord : un binaire (images…) n'est pas lu en entier.
    let mut bytes = Vec::with_capacity(size as usize + 1);
    if (&mut handle)
        .take(BINARY_PROBE as u64)
        .read_to_end(&mut bytes)
        .is_err()
    {
        return Outcome::Unreadable;
    }
    if memchr::memchr(0, &bytes).is_some() {
        return Outcome::Binary;
    }
    if bytes.len() == BINARY_PROBE && handle.read_to_end(&mut bytes).is_err() {
        return Outcome::Unreadable;
    }
    if bytes.len() as u64 > options.max_file_size {
        return Outcome::TooLarge;
    }
    let lines = search_bytes(bytes, matcher, options.max_matches);
    if lines.is_empty() {
        Outcome::NoMatch
    } else {
        Outcome::Matches(lines)
    }
}

/// Cherche dans le contenu brut d'un fichier, décodé comme le fait l'éditeur
/// (`julie_core::file::decode` : UTF-8, UTF-8 avec BOM, repli Windows-1252).
fn search_bytes(bytes: Vec<u8>, matcher: &Matcher, max_matches: usize) -> Vec<LineMatch> {
    const BOM: &[u8] = &[0xEF, 0xBB, 0xBF];
    let body = bytes.strip_prefix(BOM).unwrap_or(&bytes);
    // Chemin rapide : UTF-8 valide et fins de ligne `\n` ou `\r\n` (le `\r` final
    // est retiré ligne par ligne, ce qui donne les mêmes lignes que le décodage).
    let only_cr = memchr::memchr(b'\n', body).is_none() && memchr::memchr(b'\r', body).is_some();
    if !only_cr {
        if let Ok(text) = std::str::from_utf8(body) {
            return matcher.search_text(text, max_matches);
        }
    }
    let decoded = julie_core::file::decode(bytes);
    matcher.search_text(&decoded.text, max_matches)
}

/// Examine les fichiers en parallèle et remet les résultats par paquets
/// (premier paquet dès la première occurrence, puis toutes les 50 ms au plus).
/// Les fichiers arrivent dans un ordre quelconque ; chaque fichier arrive en une
/// fois. **Bloquant**, accès disque. `Err(Cancelled)` si `cancel` est déclenché.
pub fn search_files(
    files: &[SearchFile],
    matcher: &Matcher,
    options: &SearchOptions,
    cancel: &CancelToken,
    on_batch: &mut dyn FnMut(Vec<FileTextMatches>),
) -> Result<TextSearchSummary> {
    let started = Instant::now();
    let next = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    let searched = AtomicUsize::new(0);
    let binary = AtomicUsize::new(0);
    let too_large = AtomicUsize::new(0);
    let unreadable = AtomicUsize::new(0);
    let mut summary = TextSearchSummary::default();

    std::thread::scope(|scope| {
        let (sender, receiver) = mpsc::channel::<FileTextMatches>();
        for _ in 0..options.threads.max(1) {
            let sender = sender.clone();
            let (next, stop, searched, binary, too_large, unreadable) =
                (&next, &stop, &searched, &binary, &too_large, &unreadable);
            scope.spawn(move || loop {
                if cancel.is_cancelled() || stop.load(Ordering::Relaxed) {
                    break;
                }
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some(file) = files.get(i) else {
                    break;
                };
                match search_one(file, matcher, options) {
                    Outcome::Matches(lines) => {
                        searched.fetch_add(1, Ordering::Relaxed);
                        let result = FileTextMatches {
                            path: file.path.clone(),
                            relative: file.relative.clone(),
                            lines,
                            unsaved: file.unsaved.is_some(),
                        };
                        if sender.send(result).is_err() {
                            break;
                        }
                    }
                    Outcome::NoMatch => {
                        searched.fetch_add(1, Ordering::Relaxed);
                    }
                    Outcome::Binary => {
                        binary.fetch_add(1, Ordering::Relaxed);
                    }
                    Outcome::TooLarge => {
                        too_large.fetch_add(1, Ordering::Relaxed);
                    }
                    Outcome::Unreadable => {
                        unreadable.fetch_add(1, Ordering::Relaxed);
                    }
                }
            });
        }
        drop(sender);

        let mut batch: Vec<FileTextMatches> = Vec::new();
        let mut first_sent = false;
        let mut last_flush = Instant::now();
        loop {
            match receiver.recv_timeout(Duration::from_millis(20)) {
                Ok(mut result) => {
                    if summary.limit_reached || cancel.is_cancelled() {
                        continue;
                    }
                    let count = result.match_count();
                    let room = options.max_matches - summary.matches;
                    if count >= room {
                        truncate_matches(&mut result, room);
                        summary.limit_reached =
                            count > room || next.load(Ordering::Relaxed) < files.len();
                        stop.store(true, Ordering::Relaxed);
                    }
                    summary.matches += result.match_count();
                    summary.lines += result.lines.len();
                    summary.files_with_matches += 1;
                    batch.push(result);
                    if !first_sent || last_flush.elapsed() >= Duration::from_millis(50) {
                        on_batch(std::mem::take(&mut batch));
                        first_sent = true;
                        last_flush = Instant::now();
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if !batch.is_empty() && last_flush.elapsed() >= Duration::from_millis(50) {
                        on_batch(std::mem::take(&mut batch));
                        last_flush = Instant::now();
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        if !batch.is_empty() && !cancel.is_cancelled() {
            on_batch(batch);
        }
    });
    cancel.check()?;
    summary.files_searched = searched.into_inner();
    summary.skipped_binary = binary.into_inner();
    summary.skipped_too_large = too_large.into_inner();
    summary.unreadable = unreadable.into_inner();
    summary.elapsed = started.elapsed();
    Ok(summary)
}

fn truncate_matches(result: &mut FileTextMatches, keep: usize) {
    let mut left = keep;
    result.lines.retain_mut(|line| {
        if left == 0 {
            return false;
        }
        if line.matches.len() > left {
            line.matches.truncate(left);
        }
        left -= line.matches.len();
        true
    });
}

/// Une ligne de l'aperçu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewLine {
    /// Ligne, base zéro.
    pub line: u32,
    /// Texte complet de la ligne, sans fin de ligne.
    pub text: String,
    /// Occurrences du motif dans la ligne (colonnes en caractères), si un motif
    /// a été fourni.
    pub matches: Vec<TextRange>,
}

/// Lignes de contexte autour d'une ligne d'un fichier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preview {
    pub path: PathBuf,
    /// La ligne demandée (base zéro).
    pub line: u32,
    /// Lignes `line - before ..= line + after`, bornées au fichier.
    pub lines: Vec<PreviewLine>,
    /// Nombre de lignes du fichier.
    pub line_count: u32,
}

/// Aperçu d'un texte déjà en mémoire.
pub fn preview_text(
    path: PathBuf,
    text: &str,
    line: u32,
    before: u32,
    after: u32,
    matcher: Option<&Matcher>,
) -> Preview {
    let first = line.saturating_sub(before);
    let last = line.saturating_add(after);
    let mut lines = Vec::new();
    let mut line_count = 0u32;
    for (number, raw) in text.split('\n').enumerate() {
        let number = number as u32;
        line_count = number + 1;
        if number < first || number > last {
            continue;
        }
        let raw = raw.strip_suffix('\r').unwrap_or(raw);
        let matches = matcher
            .map(|m| {
                let ranges = m.find_in_line(raw);
                if ranges.is_empty() {
                    Vec::new()
                } else {
                    LineMatch::new(number, raw, &ranges).matches
                }
            })
            .unwrap_or_default();
        lines.push(PreviewLine {
            line: number,
            text: raw.to_string(),
            matches,
        });
    }
    if text.ends_with('\n') {
        line_count -= 1; // la « ligne » vide après le dernier saut n'en est pas une
        lines.retain(|l| l.line < line_count);
    }
    Preview {
        path,
        line,
        lines,
        line_count,
    }
}

/// Aperçu d'un fichier : contenu non enregistré s'il est fourni, sinon le disque
/// (décodé comme dans l'éditeur). **Bloquant** sans `unsaved`.
pub fn preview_file(
    path: &Path,
    unsaved: Option<&str>,
    line: u32,
    before: u32,
    after: u32,
    matcher: Option<&Matcher>,
) -> Result<Preview> {
    match unsaved {
        Some(text) => Ok(preview_text(
            path.to_path_buf(),
            text,
            line,
            before,
            after,
            matcher,
        )),
        None => {
            let file = julie_core::file::read_text(path)?;
            Ok(preview_text(
                path.to_path_buf(),
                &file.text,
                line,
                before,
                after,
                matcher,
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn query(pattern: &str) -> TextQuery {
        TextQuery::new(pattern)
    }

    fn columns(lines: &[LineMatch]) -> Vec<(u32, u32, u32)> {
        lines
            .iter()
            .flat_map(|l| {
                l.matches
                    .iter()
                    .map(|m| (m.start.line, m.start.column, m.end.column))
            })
            .collect()
    }

    #[test]
    fn literal_case_insensitive_by_default() {
        let m = Matcher::new(&query("foo")).unwrap();
        let lines = m.search_text("Foo bar\nnothing\nxfoo FOO\n", 100);
        assert_eq!(columns(&lines), vec![(0, 0, 3), (2, 1, 4), (2, 5, 8)]);
        assert_eq!(lines[1].snippet, "xfoo FOO");
    }

    #[test]
    fn case_sensitive_and_literal_special_characters() {
        let mut q = query("Foo");
        q.case_sensitive = true;
        let m = Matcher::new(&q).unwrap();
        assert!(m.search_text("foo\nfOO", 10).is_empty());
        let m = Matcher::new(&query("a.b(")).unwrap();
        assert_eq!(columns(&m.search_text("axb( a.b(", 10)), vec![(0, 5, 9)]);
    }

    #[test]
    fn whole_word_follows_git_rules() {
        let mut q = query("function");
        q.whole_word = true;
        let m = Matcher::new(&q).unwrap();
        assert_eq!(
            columns(&m.search_text("functions function _function\nfunction()", 10)),
            vec![(0, 10, 18), (1, 0, 8)]
        );
        // Un motif qui commence par un non-mot : `$id` précédé d'une espace.
        let mut q = query("$id");
        q.whole_word = true;
        let m = Matcher::new(&q).unwrap();
        assert_eq!(
            columns(&m.search_text("echo $id; $idx", 10)),
            vec![(0, 5, 8)]
        );
        // Réessai plus loin sur la même ligne.
        let mut q = query("ab");
        q.whole_word = true;
        let m = Matcher::new(&q).unwrap();
        assert_eq!(columns(&m.search_text("aab ab", 10)), vec![(0, 4, 6)]);
    }

    #[test]
    fn regex_php_variables_and_invalid_pattern() {
        let mut q = query(r"\$\w+Id\b");
        q.regex = true;
        q.case_sensitive = true;
        let m = Matcher::new(&q).unwrap();
        assert_eq!(
            columns(&m.search_text("$userId = $groupId + $Idx + $id;", 10)),
            vec![(0, 0, 7), (0, 10, 18)]
        );
        let mut q = query("(");
        q.regex = true;
        assert!(matches!(
            Matcher::new(&q),
            Err(IndexError::InvalidPattern { .. })
        ));
    }

    #[test]
    fn crlf_dollar_matches_before_carriage_return() {
        let mut q = query(r"end$");
        q.regex = true;
        let m = Matcher::new(&q).unwrap();
        let lines = m.search_text("the end\r\nnot end here\r\nend", 10);
        assert_eq!(columns(&lines), vec![(0, 4, 7), (2, 0, 3)]);
        assert_eq!(lines[0].snippet, "the end");
    }

    #[test]
    fn match_at_end_without_final_newline_and_multibyte_columns() {
        let m = Matcher::new(&query("été")).unwrap();
        let lines = m.search_text("x\nça et été", 10);
        assert_eq!(columns(&lines), vec![(1, 6, 9)]);
    }

    #[test]
    fn regex_spanning_lines_never_hides_a_line() {
        let mut q = query(r"a\s+b");
        q.regex = true;
        let m = Matcher::new(&q).unwrap();
        // Sur tout le texte, `a\nb` correspondrait en débordant ; ligne à ligne,
        // seule la seconde ligne a une occurrence.
        assert_eq!(columns(&m.search_text("xa\nb a b", 10)), vec![(1, 2, 5)]);
    }

    #[test]
    fn limit_truncates_inside_a_line() {
        let m = Matcher::new(&query("a")).unwrap();
        let lines = m.search_text("aaaa\naaaa", 6);
        assert_eq!(lines.iter().map(|l| l.matches.len()).sum::<usize>(), 6);
    }

    #[test]
    fn long_lines_give_a_snippet_around_the_match() {
        let line = format!("{}needle{}", "x".repeat(1000), "y".repeat(1000));
        let m = Matcher::new(&query("needle")).unwrap();
        let lines = m.search_text(&line, 10);
        let l = &lines[0];
        assert_eq!(l.line_length, 2006);
        assert_eq!(l.snippet_start, 1000 - 80);
        assert_eq!(l.snippet.chars().count(), 400);
        let start = (l.matches[0].start.column - l.snippet_start) as usize;
        assert_eq!(&l.snippet[start..start + 6], "needle");
    }

    #[test]
    fn windows_1252_is_decoded_like_the_editor() {
        // « café » en Windows-1252 : é = 0xE9.
        let bytes = b"un caf\xE9 noir\r\nfin".to_vec();
        let m = Matcher::new(&query("café")).unwrap();
        let lines = search_bytes(bytes, &m, 10);
        assert_eq!(columns(&lines), vec![(0, 3, 7)]);
        // UTF-8 avec BOM : les colonnes ne comptent pas la marque.
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice("café".as_bytes());
        assert_eq!(columns(&search_bytes(bytes, &m, 10)), vec![(0, 0, 4)]);
        // Fins de ligne `\r` seules.
        let lines = search_bytes(b"a\rcaf\xC3\xA9".to_vec(), &m, 10);
        assert_eq!(columns(&lines), vec![(1, 0, 4)]);
    }

    #[test]
    fn masks() {
        let mask = FileMask::parse("*.php, *.JS").unwrap();
        assert!(mask.matches("src/a.php"));
        assert!(mask.matches("web/app.js"));
        assert!(!mask.matches("web/app.css"));
        let mask = FileMask::parse("*.js, !*.min.js").unwrap();
        assert!(mask.matches("a.js"));
        assert!(!mask.matches("a.min.js"));
        let mask = FileMask::parse("!vendor/**").unwrap();
        assert!(mask.matches("src/a.php"));
        assert!(!mask.matches("vendor/x/a.php"));
        let mask = FileMask::parse("").unwrap();
        assert!(mask.matches("anything"));
        let mask = FileMask::parse("src/**/*.php").unwrap();
        assert!(mask.matches("src/a/b.php"));
        assert!(!mask.matches("lib/a/b.php"));
        assert!(matches!(
            FileMask::parse("a[,b"),
            Err(IndexError::InvalidMask { .. })
        ));
    }

    #[test]
    fn scope_mask_and_unsaved_files_select_what_is_searched() {
        let root = PathBuf::from("C:\\projet");
        let index = FileIndex::from_relative_paths(
            root.clone(),
            IndexSettings::default(),
            ["src/a.php", "src/sub/b.php", "src/c.js", "lib/d.php"].map(String::from),
        );
        let mut q = query("x");
        q.scope = SearchScope::Directory(root.join("src"));
        q.file_mask = "*.php".into();
        let mask = FileMask::parse(&q.file_mask).unwrap();
        let mut unsaved = UnsavedFiles::new();
        unsaved.insert(root.join("src").join("a.php"), Arc::from("x"));
        unsaved.insert(root.join("src").join("new.php"), Arc::from("x"));
        unsaved.insert(root.join("lib").join("new.php"), Arc::from("x"));
        let files = files_to_search(&index, &q, &mask, &unsaved).unwrap();
        let picked: Vec<(&str, bool)> = files
            .iter()
            .map(|f| (f.relative.as_str(), f.unsaved.is_some()))
            .collect();
        assert_eq!(
            picked,
            vec![
                ("src/a.php", true),
                ("src/new.php", true),
                ("src/sub/b.php", false)
            ]
        );
        q.scope = SearchScope::Directory(PathBuf::from("D:\\ailleurs"));
        assert!(matches!(
            files_to_search(&index, &q, &mask, &unsaved),
            Err(IndexError::OutsideProject(_))
        ));
    }

    #[test]
    fn binary_and_too_large_files_are_skipped_and_counted() {
        let dir = tempfile::tempdir().unwrap();
        let file = |name: &str, bytes: &[u8]| {
            let path = dir.path().join(name);
            std::fs::write(&path, bytes).unwrap();
            SearchFile {
                path,
                relative: name.into(),
                unsaved: None,
            }
        };
        let mut late_nul = b"needle\n".repeat(2000);
        late_nul.push(0);
        let files = vec![
            file("bin.dat", b"needle\0needle"),
            file("big.txt", &b"needle ".repeat(100)),
            file("ok.txt", b"needle"),
            file("late.txt", &late_nul),
        ];
        let m = Matcher::new(&query("needle")).unwrap();
        let options = SearchOptions {
            max_matches: 10_000,
            max_file_size: 600,
            threads: 2,
        };
        let mut found = Vec::new();
        let summary = search_files(&files, &m, &options, &CancelToken::new(), &mut |batch| {
            found.extend(batch.into_iter().map(|f| f.relative))
        })
        .unwrap();
        assert_eq!(found, vec!["ok.txt"]);
        assert_eq!(summary.skipped_binary, 1);
        assert_eq!(summary.skipped_too_large, 2, "{summary:?}");
    }

    #[test]
    fn preview_gives_context_lines_with_highlights() {
        let text = "zero\none foo\ntwo\nthree foo foo\nfour\n";
        let m = Matcher::new(&query("foo")).unwrap();
        let p = preview_text(PathBuf::from("x"), text, 3, 2, 5, Some(&m));
        assert_eq!(p.line_count, 5);
        assert_eq!(
            p.lines.iter().map(|l| l.line).collect::<Vec<_>>(),
            vec![1, 2, 3, 4]
        );
        assert_eq!(p.lines[2].matches.len(), 2);
        assert_eq!(p.lines[0].text, "one foo");
    }
}
