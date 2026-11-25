//! L'index des symboles en mémoire : déclarations par fichier, tables de
//! recherche par nom complet et par nom court, construction parallèle et mise
//! à jour fichier par fichier.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant, UNIX_EPOCH};

use julie_core::CancelToken;

use super::cache;
use super::model::{flags, fnv, key, php_const_key, Analysis, Decl, FileSymbols, Lang};
use super::{js, php, SymbolKind};
use crate::error::Result;
use crate::files::{absolute_path, relative_string, FileIndex};
use crate::text::UnsavedFiles;

/// Pile des fils d'analyse : l'extraction descend l'arbre récursivement.
pub(crate) const BIG_STACK: usize = 64 * 1024 * 1024;

/// Taille au-delà de laquelle un fichier n'entre pas dans l'index des symboles
/// (tables générées, polices, code empaqueté : l'analyse y coûte des secondes
/// pour aucune déclaration utile). La recherche de texte, elle, les lit.
pub const SYMBOL_MAX_FILE_SIZE: u64 = 1024 * 1024;

/// Profondeur maximale de l'arbre parcourue par l'extraction.
pub(crate) const MAX_DEPTH: usize = 1500;

/// Date et taille d'un fichier, pour savoir si le cache est à jour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Stamp {
    pub mtime: u64,
    pub size: u64,
}

impl Stamp {
    pub(crate) fn of(metadata: &std::fs::Metadata) -> Self {
        let mtime = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_nanos() as u64);
        Stamp {
            mtime,
            size: metadata.len(),
        }
    }
}

/// Référence d'une déclaration : (fichier, rang dans le fichier).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct DeclRef {
    pub file: u32,
    pub idx: u32,
}

/// Un fichier de l'index.
#[derive(Debug, Clone)]
pub(crate) struct Slot {
    pub relative: Box<str>,
    pub library: bool,
    pub stamp: Stamp,
    pub symbols: FileSymbols,
    /// Indexé depuis le contenu d'un onglet, pas depuis le disque.
    pub from_memory: bool,
}

/// L'index des symboles d'un projet.
#[derive(Debug, Clone)]
pub struct SymbolIndex {
    root: PathBuf,
    slots: Vec<Option<Slot>>,
    by_path: HashMap<Box<str>, u32>,
    free: Vec<u32>,
    by_key: HashMap<u64, Vec<DeclRef>>,
    by_name: HashMap<u64, Vec<DeclRef>>,
    decl_count: usize,
}

/// Bilan d'une construction de l'index des symboles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SymbolBuildSummary {
    /// Fichiers PHP et JS/TS dans l'index à la fin.
    pub files: usize,
    /// Dont analysés (le reste vient du cache).
    pub parsed: usize,
    pub from_cache: usize,
    pub skipped_too_large: usize,
    pub skipped_binary: usize,
    pub unreadable: usize,
    pub declarations: usize,
    /// Un cache valide a été relu.
    pub cache_loaded: bool,
    /// Le cache a été (ré)écrit.
    pub cache_written: bool,
    pub elapsed: Duration,
}

/// Clés d'une déclaration : (nom complet, nom court).
fn decl_keys(lang: Lang, is_module: bool, decl: &Decl) -> (Option<u64>, Option<u64>) {
    if lang.is_php() {
        let fqn = || decl.php_fqn();
        match decl.kind {
            k if k.is_class_like() => (
                Some(key("c", &fqn(), true)),
                Some(key("pc", &decl.name, true)),
            ),
            SymbolKind::Function => (
                Some(key("f", &fqn(), true)),
                Some(key("pf", &decl.name, true)),
            ),
            SymbolKind::Constant => (Some(key("k", &php_const_key(&fqn()), false)), None),
            SymbolKind::Method => (
                Some(member_key("m", &decl.container, &decl.name, true)),
                Some(key("pm", &decl.name, true)),
            ),
            SymbolKind::Property => (
                Some(member_key("p", &decl.container, &decl.name, false)),
                Some(key("pp", &decl.name, false)),
            ),
            SymbolKind::ClassConstant | SymbolKind::EnumCase => (
                Some(member_key("o", &decl.container, &decl.name, false)),
                Some(key("po", &decl.name, false)),
            ),
            _ => (None, None),
        }
    } else {
        match decl.kind {
            SymbolKind::Method => (None, Some(key("jm", &decl.name, false))),
            SymbolKind::Property => (None, Some(key("jp", &decl.name, false))),
            _ if !is_module && decl.has(flags::MODULE_LEVEL) => {
                (None, Some(key("jg", &decl.name, false)))
            }
            _ => (None, None),
        }
    }
}

/// Clé d'un membre PHP : classe insensible à la casse, nom selon le genre.
pub(crate) fn member_key(tag: &str, class: &str, name: &str, lower_name: bool) -> u64 {
    let name = if lower_name {
        name.to_ascii_lowercase()
    } else {
        name.to_string()
    };
    key(
        tag,
        &format!("{}::{}", class.to_ascii_lowercase(), name),
        false,
    )
}

impl SymbolIndex {
    /// Index vide.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            slots: Vec::new(),
            by_path: HashMap::new(),
            free: Vec::new(),
            by_key: HashMap::new(),
            by_name: HashMap::new(),
            decl_count: 0,
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Nombre de fichiers indexés.
    pub fn file_count(&self) -> usize {
        self.by_path.len()
    }

    pub fn declaration_count(&self) -> usize {
        self.decl_count
    }

    /// Le fichier (chemin relatif, `/`) est dans l'index des symboles.
    pub fn contains(&self, relative: &str) -> bool {
        self.by_path.contains_key(relative)
    }

    /// Mémoire occupée, estimée (octets) : pour les mesures.
    pub fn approximate_memory(&self) -> usize {
        let mut total = 0usize;
        for slot in self.slots.iter().flatten() {
            total += std::mem::size_of::<Slot>() + slot.relative.len() * 2 + 48;
            total += slot.symbols.ref_keys.len() * 4;
            for d in &slot.symbols.decls {
                total += std::mem::size_of::<Decl>() + d.name.len() + d.container.len();
                total += d.types.iter().map(|t| t.len() + 16).sum::<usize>();
                total += d.bases.iter().map(|(_, t)| t.len() + 24).sum::<usize>();
            }
            if let Some(js) = &slot.symbols.js {
                total +=
                    64 + js.imports.len() * 80 + js.exports.len() * 48 + js.reexports.len() * 64;
            }
        }
        for map in [&self.by_key, &self.by_name] {
            total += map.capacity() * (8 + 24 + 8);
            total += map.values().map(|v| v.capacity() * 8).sum::<usize>();
        }
        total
    }

    pub(crate) fn file_id(&self, relative: &str) -> Option<u32> {
        self.by_path.get(relative).copied()
    }

    pub(crate) fn slot(&self, file: u32) -> Option<&Slot> {
        self.slots.get(file as usize)?.as_ref()
    }

    pub(crate) fn slot_by_path(&self, relative: &str) -> Option<&Slot> {
        self.slot(self.file_id(relative)?)
    }

    pub(crate) fn slots(&self) -> impl Iterator<Item = (u32, &Slot)> {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(i, s)| s.as_ref().map(|s| (i as u32, s)))
    }

    pub(crate) fn decl(&self, r: DeclRef) -> Option<(&Slot, &Decl)> {
        let slot = self.slot(r.file)?;
        Some((slot, slot.symbols.decls.get(r.idx as usize)?))
    }

    pub(crate) fn by_key(&self, key: u64) -> &[DeclRef] {
        self.by_key.get(&key).map_or(&[], |v| v.as_slice())
    }

    pub(crate) fn by_name(&self, key: u64) -> &[DeclRef] {
        self.by_name.get(&key).map_or(&[], |v| v.as_slice())
    }

    pub(crate) fn absolute(&self, relative: &str) -> PathBuf {
        absolute_path(&self.root, relative)
    }

    fn add_keys(&mut self, file: u32) {
        let Some(slot) = self.slots[file as usize].as_ref() else {
            return;
        };
        let lang = slot.symbols.lang;
        let is_module = slot.symbols.js.as_ref().is_some_and(|j| j.is_module);
        let mut pairs = Vec::with_capacity(slot.symbols.decls.len());
        for (i, d) in slot.symbols.decls.iter().enumerate() {
            pairs.push((decl_keys(lang, is_module, d), i as u32));
        }
        self.decl_count += slot.symbols.decls.len();
        for ((q, n), idx) in pairs {
            let r = DeclRef { file, idx };
            if let Some(q) = q {
                self.by_key.entry(q).or_default().push(r);
            }
            if let Some(n) = n {
                self.by_name.entry(n).or_default().push(r);
            }
        }
    }

    fn remove_keys(&mut self, file: u32) {
        let Some(slot) = self.slots[file as usize].as_ref() else {
            return;
        };
        let lang = slot.symbols.lang;
        let is_module = slot.symbols.js.as_ref().is_some_and(|j| j.is_module);
        let keys: Vec<_> = slot
            .symbols
            .decls
            .iter()
            .map(|d| decl_keys(lang, is_module, d))
            .collect();
        self.decl_count -= slot.symbols.decls.len();
        for (q, n) in keys {
            for (map, k) in [(&mut self.by_key, q), (&mut self.by_name, n)] {
                if let Some(k) = k {
                    if let Some(list) = map.get_mut(&k) {
                        list.retain(|r| r.file != file);
                        if list.is_empty() {
                            map.remove(&k);
                        }
                    }
                }
            }
        }
    }

    /// Ajoute ou remplace un fichier.
    pub(crate) fn insert(
        &mut self,
        relative: &str,
        library: bool,
        stamp: Stamp,
        symbols: FileSymbols,
        from_memory: bool,
    ) {
        let file = match self.by_path.get(relative) {
            Some(&id) => {
                self.remove_keys(id);
                id
            }
            None => {
                let id = match self.free.pop() {
                    Some(id) => id,
                    None => {
                        self.slots.push(None);
                        (self.slots.len() - 1) as u32
                    }
                };
                self.by_path.insert(relative.into(), id);
                id
            }
        };
        self.slots[file as usize] = Some(Slot {
            relative: relative.into(),
            library,
            stamp,
            symbols,
            from_memory,
        });
        self.add_keys(file);
    }

    /// Retire un fichier ; rend vrai s'il était là.
    pub(crate) fn remove(&mut self, relative: &str) -> bool {
        let Some(id) = self.by_path.remove(relative) else {
            return false;
        };
        self.remove_keys(id);
        self.slots[id as usize] = None;
        self.free.push(id);
        true
    }

    fn clear(&mut self) {
        self.slots.clear();
        self.by_path.clear();
        self.free.clear();
        self.by_key.clear();
        self.by_name.clear();
        self.decl_count = 0;
    }

    /// Construit l'index de tous les fichiers PHP et JS/TS de `files`.
    /// **Bloquant**, accès disque. `cache_dir` : dossier du cache
    /// (`.juliestorm/index` du projet ; un dossier temporaire pour les
    /// mesures) ; relu s'il est valide, réécrit à la fin. `None` : sans cache.
    /// `unsaved` : contenus d'onglets non enregistrés, indexés à la place du
    /// disque.
    pub fn build(
        files: &FileIndex,
        cache_dir: Option<&Path>,
        unsaved: &UnsavedFiles,
        cancel: &CancelToken,
    ) -> Result<(Self, SymbolBuildSummary)> {
        let started = Instant::now();
        let mut index = SymbolIndex::new(files.root());
        let (input, cache_loaded) = BuildInput::new(files, cache_dir, unsaved);
        let mut summary = build_into(input, cancel, &mut |batch, _, _| {
            for b in batch {
                if let Some(symbols) = b.symbols {
                    index.insert(&b.relative, b.library, b.stamp, symbols, b.from_memory);
                }
            }
        })?;
        summary.cache_loaded = cache_loaded;
        if let Some(dir) = cache_dir {
            summary.cache_written = index.save_cache(dir).is_ok();
        }
        summary.files = index.file_count();
        summary.declarations = index.declaration_count();
        summary.elapsed = started.elapsed();
        Ok((index, summary))
    }

    /// Écrit le cache dans `dir` (créé au besoin), atomiquement. **Accès disque.**
    pub fn save_cache(&self, dir: &Path) -> Result<()> {
        cache::save(self, dir)
    }

    /// Met l'index à jour pour des chemins absolus changés sur le disque
    /// (fichiers ou dossiers), d'après `files` **déjà à jour**. **Bloquant.**
    /// Rend le nombre de fichiers réindexés ou retirés.
    pub fn update(
        &mut self,
        files: &FileIndex,
        changed: &[PathBuf],
        unsaved: &UnsavedFiles,
        cancel: &CancelToken,
    ) -> Result<usize> {
        let work = self.affected(files, changed);
        let mut count = 0;
        update_into(
            self.root.clone(),
            work,
            files.settings().max_file_size,
            unsaved,
            cancel,
            &mut |relative, result| {
                count += 1;
                match result {
                    Some((library, stamp, symbols, from_memory)) => {
                        self.insert(relative, library, stamp, symbols, from_memory)
                    }
                    None => {
                        self.remove(relative);
                    }
                }
            },
        )?;
        Ok(count)
    }

    /// Réindexe un fichier depuis un texte en mémoire (onglet non enregistré).
    /// Sans effet si le texte est celui déjà indexé.
    pub fn set_text(&mut self, path: &Path, text: &str, library: bool) -> bool {
        let Some(relative) = relative_string(&self.root, path) else {
            return false;
        };
        let Some(lang) = Lang::from_name(&relative) else {
            return false;
        };
        let hash = fnv(text.as_bytes());
        if let Some(slot) = self.slot_by_path(&relative) {
            if slot.symbols.content_hash == hash {
                return false;
            }
        }
        let Some(analysis) = analyze_text(lang, text) else {
            return false;
        };
        self.insert(&relative, library, Stamp::default(), analysis.symbols, true);
        true
    }

    pub(crate) fn slot_content_hash(&self, relative: &str) -> Option<u64> {
        self.slot_by_path(relative).map(|s| s.symbols.content_hash)
    }

    pub(crate) fn insert_analysis(&mut self, relative: &str, library: bool, analysis: &Analysis) {
        let unchanged = self
            .slot_by_path(relative)
            .is_some_and(|s| s.symbols.content_hash == analysis.symbols.content_hash);
        if !unchanged {
            self.insert(
                relative,
                library,
                Stamp::default(),
                analysis.symbols.clone(),
                true,
            );
        }
    }

    /// Chemins relatifs à revoir pour une liste de chemins changés : (chemin,
    /// bibliothèque si le fichier est toujours dans `files`).
    pub(crate) fn affected(
        &self,
        files: &FileIndex,
        changed: &[PathBuf],
    ) -> Vec<(String, Option<bool>)> {
        let mut out: Vec<(String, Option<bool>)> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for path in changed {
            let Some(relative) = files.relative(path) else {
                continue;
            };
            let prefix = format!("{relative}/");
            let mut push = |rel: &str, out: &mut Vec<(String, Option<bool>)>| {
                if Lang::from_name(rel).is_some() && seen.insert(rel.to_string()) {
                    let present = files.get(rel).map(|e| e.is_library());
                    out.push((rel.to_string(), present));
                }
            };
            push(&relative, &mut out);
            if !relative.is_empty() {
                for e in files.entries() {
                    if e.relative().starts_with(&prefix) {
                        push(e.relative(), &mut out);
                    }
                }
                let mine: Vec<String> = self
                    .by_path
                    .keys()
                    .filter(|k| k.starts_with(&prefix))
                    .map(|k| k.to_string())
                    .collect();
                for k in mine {
                    push(&k, &mut out);
                }
            } else {
                for e in files.entries() {
                    push(e.relative(), &mut out);
                }
            }
        }
        out
    }

    pub(crate) fn reset(&mut self, root: PathBuf) {
        self.clear();
        self.root = root;
    }
}

// ---------------------------------------------------------------------------
// Analyse d'un texte.

thread_local! {
    static PARSERS: RefCell<[Option<tree_sitter::Parser>; 4]> = const { RefCell::new([None, None, None, None]) };
}

/// Analyse un texte (déclarations, références, portées). `None` si l'analyse
/// échoue (ce qui n'arrive pas avec du texte ordinaire : un fichier faux est
/// analysé au mieux). À appeler sur un fil à grande pile ([`on_big_stack`]).
pub(crate) fn analyze_text(lang: Lang, text: &str) -> Option<Analysis> {
    analyze_text_mode(lang, text, false)
}

/// Déclarations et clés seulement (construction de l'index).
pub(crate) fn analyze_declarations(lang: Lang, text: &str) -> Option<FileSymbols> {
    analyze_text_mode(lang, text, true).map(|a| a.symbols)
}

fn analyze_text_mode(lang: Lang, text: &str, declarations_only: bool) -> Option<Analysis> {
    let tree = PARSERS.with(|parsers| {
        let mut parsers = parsers.borrow_mut();
        let slot = &mut parsers[lang.code() as usize];
        if slot.is_none() {
            let mut parser = tree_sitter::Parser::new();
            parser.set_language(&lang.grammar()).ok()?;
            *slot = Some(parser);
        }
        slot.as_mut()?.parse(text, None)
    })?;
    let hash = fnv(text.as_bytes());
    Some(match lang {
        Lang::Php => php::analyze(text, &tree, hash, declarations_only),
        _ => js::analyze(text, &tree, hash, lang),
    })
}

/// Exécute `f` sur un fil à grande pile (l'extraction est récursive).
pub(crate) fn on_big_stack<R: Send>(f: impl FnOnce() -> R + Send) -> Option<R> {
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .name("julie-index-analyse".into())
            .stack_size(BIG_STACK)
            .spawn_scoped(scope, f)
            .ok()?
            .join()
            .ok()
    })
}

/// Contenu d'un fichier à indexer.
pub(crate) enum Loaded {
    Text(String),
    TooLarge,
    Binary,
    Unreadable,
}

/// Lit et décode un fichier (comme l'éditeur : UTF-8, BOM, repli
/// Windows-1252). **Accès disque.**
pub(crate) fn load_text(path: &Path, max_size: u64) -> (Loaded, Stamp) {
    let Ok(metadata) = std::fs::metadata(path) else {
        return (Loaded::Unreadable, Stamp::default());
    };
    let stamp = Stamp::of(&metadata);
    if metadata.len() > max_size.min(SYMBOL_MAX_FILE_SIZE) {
        return (Loaded::TooLarge, stamp);
    }
    let Ok(bytes) = std::fs::read(path) else {
        return (Loaded::Unreadable, stamp);
    };
    let head = &bytes[..bytes.len().min(8000)];
    if memchr::memchr(0, head).is_some() {
        return (Loaded::Binary, stamp);
    }
    (Loaded::Text(julie_core::file::decode(bytes).text), stamp)
}

// ---------------------------------------------------------------------------
// Construction parallèle.

/// Ce qu'il faut pour construire l'index.
pub(crate) struct BuildInput {
    pub root: PathBuf,
    /// (chemin relatif, bibliothèque)
    pub entries: Vec<(String, bool)>,
    pub max_file_size: u64,
    pub threads: usize,
    pub cached: HashMap<Box<str>, (Stamp, FileSymbols)>,
    pub unsaved: HashMap<PathBuf, Arc<str>>,
}

impl BuildInput {
    /// Fichiers PHP et JS/TS de l'index des fichiers ; cache relu s'il est
    /// valide (rend `true` alors). **Accès disque** (lecture du cache).
    pub(crate) fn new(
        files: &FileIndex,
        cache_dir: Option<&Path>,
        unsaved: &UnsavedFiles,
    ) -> (Self, bool) {
        let entries: Vec<(String, bool)> = files
            .entries()
            .iter()
            .filter(|e| Lang::from_name(e.name()).is_some())
            .map(|e| (e.relative().to_string(), e.is_library()))
            .collect();
        let cached = cache_dir.and_then(|d| cache::load(d).ok());
        let loaded = cached.is_some();
        (
            BuildInput {
                root: files.root().to_path_buf(),
                entries,
                max_file_size: files.settings().max_file_size,
                threads: files.settings().threads.clamp(1, 32),
                cached: cached.unwrap_or_default(),
                unsaved: unsaved.clone(),
            },
            loaded,
        )
    }
}

/// `BuildInput` sans le cache (partagé à part, sous verrou).
struct BuildRefs {
    root: PathBuf,
    entries: Vec<(String, bool)>,
    max_file_size: u64,
    threads: usize,
    unsaved: HashMap<PathBuf, Arc<str>>,
}

/// Un fichier traité.
pub(crate) struct Built {
    pub relative: Box<str>,
    pub library: bool,
    pub stamp: Stamp,
    /// `None` : pas indexé (trop gros, binaire, illisible).
    pub symbols: Option<FileSymbols>,
    pub from_memory: bool,
    outcome: Outcome,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Parsed,
    Cached,
    TooLarge,
    Binary,
    Unreadable,
}

fn process(
    root: &Path,
    relative: &str,
    library: bool,
    max_size: u64,
    cached: &Mutex<HashMap<Box<str>, (Stamp, FileSymbols)>>,
    unsaved: &HashMap<PathBuf, Arc<str>>,
) -> Built {
    let path = absolute_path(root, relative);
    let lang = Lang::from_name(relative).unwrap_or(Lang::Php);
    let built = |stamp, symbols, from_memory, outcome| Built {
        relative: relative.into(),
        library,
        stamp,
        symbols,
        from_memory,
        outcome,
    };
    if let Some(text) = unsaved.get(&path) {
        let symbols = analyze_declarations(lang, text);
        return built(Stamp::default(), symbols, true, Outcome::Parsed);
    }
    let previous = cached
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .remove(relative);
    if let Some((stamp, symbols)) = previous {
        if let Ok(metadata) = std::fs::metadata(&path) {
            if Stamp::of(&metadata) == stamp && stamp != Stamp::default() {
                return built(stamp, Some(symbols), false, Outcome::Cached);
            }
        }
    }
    let (loaded, stamp) = load_text(&path, max_size);
    match loaded {
        Loaded::Text(text) => {
            let symbols = analyze_declarations(lang, &text);
            built(stamp, symbols, false, Outcome::Parsed)
        }
        Loaded::TooLarge => built(stamp, None, false, Outcome::TooLarge),
        Loaded::Binary => built(stamp, None, false, Outcome::Binary),
        Loaded::Unreadable => built(stamp, None, false, Outcome::Unreadable),
    }
}

/// Construit en parallèle ; `sink(paquet, faits, total)` reçoit les fichiers
/// par paquets (toutes les 50 ms au plus). **Bloquant**, accès disque.
pub(crate) fn build_into(
    input: BuildInput,
    cancel: &CancelToken,
    sink: &mut dyn FnMut(Vec<Built>, usize, usize),
) -> Result<SymbolBuildSummary> {
    let started = Instant::now();
    let total = input.entries.len();
    let next = AtomicUsize::new(0);
    let BuildInput {
        root,
        entries,
        max_file_size,
        threads,
        cached,
        unsaved,
    } = input;
    let input = BuildRefs {
        root,
        entries,
        max_file_size,
        threads,
        unsaved,
    };
    let cached = Mutex::new(cached);
    let mut summary = SymbolBuildSummary::default();
    let (sender, receiver) = mpsc::channel::<Built>();
    std::thread::scope(|scope| {
        for i in 0..input.threads {
            let sender = sender.clone();
            let (next, cached, input_ref) = (&next, &cached, &input);
            let _ = std::thread::Builder::new()
                .name(format!("julie-index-symboles-{i}"))
                .stack_size(BIG_STACK)
                .spawn_scoped(scope, move || loop {
                    if cancel.is_cancelled() {
                        break;
                    }
                    let n = next.fetch_add(1, Ordering::Relaxed);
                    let Some((relative, library)) = input_ref.entries.get(n) else {
                        break;
                    };
                    let built = process(
                        &input_ref.root,
                        relative,
                        *library,
                        input_ref.max_file_size,
                        cached,
                        &input_ref.unsaved,
                    );
                    if sender.send(built).is_err() {
                        break;
                    }
                });
        }
        drop(sender);
        let mut pending = Vec::new();
        let mut done = 0usize;
        let mut last = Instant::now();
        loop {
            match receiver.recv_timeout(Duration::from_millis(50)) {
                Ok(built) => {
                    done += 1;
                    match built.outcome {
                        Outcome::Parsed => summary.parsed += 1,
                        Outcome::Cached => summary.from_cache += 1,
                        Outcome::TooLarge => summary.skipped_too_large += 1,
                        Outcome::Binary => summary.skipped_binary += 1,
                        Outcome::Unreadable => summary.unreadable += 1,
                    }
                    pending.push(built);
                    if pending.len() >= 512 || last.elapsed() >= Duration::from_millis(50) {
                        sink(std::mem::take(&mut pending), done, total);
                        last = Instant::now();
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if !pending.is_empty() {
                        sink(std::mem::take(&mut pending), done, total);
                        last = Instant::now();
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        if !pending.is_empty() {
            sink(pending, done, total);
        }
    });
    cancel.check()?;
    summary.elapsed = started.elapsed();
    Ok(summary)
}

/// Réindexe une liste de fichiers (fil courant, sur une grande pile) ;
/// `apply(chemin, Some((bibliothèque, date, symboles, depuis la mémoire)))` ou
/// `None` pour retirer.
#[allow(clippy::type_complexity)]
pub(crate) fn update_into(
    root: PathBuf,
    work: Vec<(String, Option<bool>)>,
    max_size: u64,
    unsaved: &UnsavedFiles,
    cancel: &CancelToken,
    apply: &mut dyn FnMut(&str, Option<(bool, Stamp, FileSymbols, bool)>),
) -> Result<()> {
    let empty = Mutex::new(HashMap::new());
    let results = on_big_stack(|| {
        let mut out = Vec::new();
        for (relative, present) in work {
            if cancel.is_cancelled() {
                break;
            }
            let result = match present {
                Some(library) => {
                    let b = process(&root, &relative, library, max_size, &empty, unsaved);
                    b.symbols.map(|s| (library, b.stamp, s, b.from_memory))
                }
                None => None,
            };
            out.push((relative, result));
        }
        out
    })
    .unwrap_or_default();
    cancel.check()?;
    for (relative, result) in results {
        apply(&relative, result);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// L'index mêle les deux modes (construction : déclarations seules ; onglet
    /// et navigation : analyse complète) : ils doivent rendre les mêmes symboles.
    #[test]
    fn both_analysis_modes_give_the_same_symbols() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/navigation");
        let mut checked = 0;
        let mut stack = vec![root];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                let Some(lang) = Lang::from_path(&path) else {
                    continue;
                };
                let text = std::fs::read_to_string(&path).unwrap();
                let full = on_big_stack(|| analyze_text(lang, &text))
                    .flatten()
                    .unwrap();
                let light = on_big_stack(|| analyze_declarations(lang, &text))
                    .flatten()
                    .unwrap();
                assert_eq!(full.symbols, light, "{}", path.display());
                checked += 1;
            }
        }
        assert_eq!(checked, 15);
    }
}
