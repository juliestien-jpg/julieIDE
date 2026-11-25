//! Index des fichiers du projet : parcours parallèle respectant `.gitignore`,
//! exclusions, bibliothèques, mise à jour incrémentale.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf, MAIN_SEPARATOR_STR};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use ignore::{WalkBuilder, WalkState};
use julie_core::CancelToken;

use crate::error::Result;
use crate::settings::IndexSettings;

/// Un fichier de l'index.
///
/// Le chemin relatif utilise `/` comme séparateur, quel que soit le système :
/// c'est sur lui que portent la correspondance floue et l'affichage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    relative: Box<str>,
    /// `relative` en minuscules, caractère pour caractère (même nombre de
    /// caractères, pas forcément le même nombre d'octets).
    pub(crate) lower: Box<str>,
    /// Début du nom du fichier dans `relative`, en octets.
    name_byte: u32,
    /// Début du nom du fichier dans `lower`, en octets.
    pub(crate) lower_name_byte: u32,
    /// Début du nom du fichier, en caractères.
    name_char: u32,
    library: bool,
}

impl FileEntry {
    pub(crate) fn new(relative: String, library: bool) -> Self {
        let name_byte = relative.rfind('/').map_or(0, |i| i + 1);
        let name_char = relative[..name_byte].chars().count();
        let mut lower = String::with_capacity(relative.len());
        let mut lower_name_byte = 0;
        for (byte, c) in relative.char_indices() {
            if byte == name_byte {
                lower_name_byte = lower.len();
            }
            lower.push(lower_char(c));
        }
        if name_byte == relative.len() {
            lower_name_byte = lower.len();
        }
        Self {
            relative: relative.into_boxed_str(),
            lower: lower.into_boxed_str(),
            name_byte: name_byte as u32,
            lower_name_byte: lower_name_byte as u32,
            name_char: name_char as u32,
            library,
        }
    }

    /// Chemin relatif à la racine, séparateurs `/`.
    pub fn relative(&self) -> &str {
        &self.relative
    }

    /// Nom du fichier (dernier composant).
    pub fn name(&self) -> &str {
        &self.relative[self.name_byte as usize..]
    }

    /// Dossier relatif (vide à la racine), sans `/` final.
    pub fn directory(&self) -> &str {
        self.relative[..self.name_byte as usize].trim_end_matches('/')
    }

    /// Position du nom dans le chemin relatif, en caractères.
    pub fn name_start(&self) -> usize {
        self.name_char as usize
    }

    /// Le fichier est dans une bibliothèque (`vendor`, `node_modules`…).
    pub fn is_library(&self) -> bool {
        self.library
    }
}

/// Minuscule d'un caractère, en un seul caractère (garde l'alignement des
/// positions entre le chemin et sa version en minuscules).
pub(crate) fn lower_char(c: char) -> char {
    if c.is_ascii() {
        c.to_ascii_lowercase()
    } else {
        c.to_lowercase().next().unwrap_or(c)
    }
}

/// Chemin relatif (`/`) d'un chemin absolu du projet, comparaison lexicale.
pub(crate) fn relative_string(root: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    let text = relative.to_str()?;
    Some(text.replace('\\', "/").trim_matches('/').to_string())
}

/// Chemin absolu d'un chemin relatif (`/`), avec les séparateurs du système.
pub(crate) fn absolute_path(root: &Path, relative: &str) -> PathBuf {
    if relative.is_empty() {
        root.to_path_buf()
    } else {
        root.join(relative.replace('/', MAIN_SEPARATOR_STR))
    }
}

/// Ce que le parcours trouve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Walked {
    File(String),
    Dir(String),
}

/// Bilan d'une mise à jour incrémentale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UpdateSummary {
    /// Fichiers apparus.
    pub added: usize,
    /// Fichiers disparus.
    pub removed: usize,
    /// Nombre de fichiers de l'index après la mise à jour.
    pub files: usize,
    /// La mise à jour a demandé un parcours complet (`.gitignore` racine changé…).
    pub full_rescan: bool,
}

/// L'index des fichiers d'un projet.
#[derive(Debug, Clone)]
pub struct FileIndex {
    root: PathBuf,
    settings: IndexSettings,
    entries: Vec<FileEntry>,
    by_path: HashMap<Box<str>, u32>,
    /// Dossiers parcourus (chemins relatifs), pour la mise à jour incrémentale.
    dirs: HashSet<Box<str>>,
    pub(crate) generation: u64,
}

impl FileIndex {
    /// Index vide (rien n'est lu).
    pub fn new(root: impl Into<PathBuf>, settings: IndexSettings) -> Self {
        Self {
            root: root.into(),
            settings,
            entries: Vec::new(),
            by_path: HashMap::new(),
            dirs: HashSet::new(),
            generation: 0,
        }
    }

    /// Parcourt tout le projet. **Bloquant**, accès disque : dans une tâche.
    pub fn scan(
        root: impl Into<PathBuf>,
        settings: IndexSettings,
        cancel: &CancelToken,
    ) -> Result<Self> {
        let mut index = Self::new(root, settings);
        let root = index.root.clone();
        let settings = index.settings.clone();
        walk(&root, &root, None, &settings, cancel, &mut |batch| {
            index.insert_walked(batch)
        })?;
        Ok(index)
    }

    /// Index construit à partir de chemins relatifs (`/`) déjà connus, sans
    /// accès disque (essais, mesures, futur cache). Les dossiers intermédiaires
    /// sont déduits des chemins.
    pub fn from_relative_paths(
        root: impl Into<PathBuf>,
        settings: IndexSettings,
        paths: impl IntoIterator<Item = String>,
    ) -> Self {
        let mut index = Self::new(root, settings);
        for path in paths {
            let mut dir = parent_of(&path);
            while !dir.is_empty() && index.dirs.insert(dir.into()) {
                dir = parent_of(dir);
            }
            index.insert_file(path);
        }
        index
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn settings(&self) -> &IndexSettings {
        &self.settings
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Les fichiers, dans un ordre quelconque.
    pub fn entries(&self) -> &[FileEntry] {
        &self.entries
    }

    /// Le fichier de ce chemin relatif (`/`), s'il est indexé.
    pub fn get(&self, relative: &str) -> Option<&FileEntry> {
        self.by_path
            .get(relative)
            .map(|&i| &self.entries[i as usize])
    }

    pub fn contains(&self, relative: &str) -> bool {
        self.by_path.contains_key(relative)
    }

    /// Chemin absolu d'un chemin relatif de l'index.
    pub fn absolute(&self, relative: &str) -> PathBuf {
        absolute_path(&self.root, relative)
    }

    /// Chemin relatif (`/`) d'un chemin absolu, ou `None` hors du projet.
    pub fn relative(&self, path: &Path) -> Option<String> {
        if path.is_relative() {
            return path
                .to_str()
                .map(|p| p.replace('\\', "/").trim_matches('/').to_string());
        }
        relative_string(&self.root, path)
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.by_path.clear();
        self.dirs.clear();
        self.generation += 1;
    }

    /// Vide l'index et change ses réglages (nouvelle indexation à suivre).
    pub(crate) fn reset(&mut self, settings: IndexSettings) {
        self.clear();
        self.settings = settings;
    }

    pub(crate) fn insert_walked(&mut self, batch: Vec<Walked>) {
        for item in batch {
            match item {
                Walked::File(relative) => {
                    self.insert_file(relative);
                }
                Walked::Dir(relative) => {
                    self.dirs.insert(relative.into_boxed_str());
                }
            }
        }
    }

    fn insert_file(&mut self, relative: String) -> bool {
        if self.by_path.contains_key(relative.as_str()) {
            return false;
        }
        let library = self.settings.is_library(&relative);
        let entry = FileEntry::new(relative, library);
        self.by_path
            .insert(entry.relative.clone(), self.entries.len() as u32);
        self.entries.push(entry);
        true
    }

    fn remove_file(&mut self, relative: &str) -> bool {
        let Some(index) = self.by_path.remove(relative) else {
            return false;
        };
        let index = index as usize;
        self.entries.swap_remove(index);
        if let Some(moved) = self.entries.get(index) {
            self.by_path.insert(moved.relative.clone(), index as u32);
        }
        true
    }

    /// Retire un dossier et tout ce qu'il contient ; rend les fichiers retirés.
    fn remove_tree(&mut self, relative: &str) -> Vec<Box<str>> {
        let prefix = format!("{relative}/");
        let mut removed = Vec::new();
        self.entries.retain(|entry| {
            let inside = entry.relative.starts_with(&prefix);
            if inside {
                removed.push(entry.relative.clone());
            }
            !inside
        });
        self.dirs
            .retain(|dir| &**dir != relative && !dir.starts_with(&prefix));
        if !removed.is_empty() {
            self.by_path = self
                .entries
                .iter()
                .enumerate()
                .map(|(i, entry)| (entry.relative.clone(), i as u32))
                .collect();
        }
        removed
    }

    /// Met l'index à jour pour une liste de chemins changés sur le disque
    /// (créés, modifiés, supprimés, renommés : ancien et nouveau chemin), fichiers
    /// ou dossiers. **Bloquant**, accès disque.
    pub fn update(&mut self, paths: &[PathBuf], cancel: &CancelToken) -> Result<UpdateSummary> {
        let plan = self.plan_update(paths);
        if plan.full_rescan {
            let fresh = Self::scan(self.root.clone(), self.settings.clone(), cancel)?;
            return Ok(self.replace_with(fresh));
        }
        let walked = collect_update(&self.root, &self.settings, &plan, cancel)?;
        Ok(self.apply_update(&plan, walked))
    }

    /// Remplace tout le contenu par celui d'un index fraîchement parcouru.
    pub(crate) fn replace_with(&mut self, fresh: FileIndex) -> UpdateSummary {
        let before: HashSet<&str> = self.by_path.keys().map(|k| &**k).collect();
        let after: HashSet<&str> = fresh.by_path.keys().map(|k| &**k).collect();
        let added = after.difference(&before).count();
        let removed = before.difference(&after).count();
        let generation = self.generation + 1;
        *self = fresh;
        self.generation = generation;
        UpdateSummary {
            added,
            removed,
            files: self.len(),
            full_rescan: true,
        }
    }

    /// Ce qu'il faut retirer et reparcourir, sans accès disque.
    pub(crate) fn plan_update(&self, paths: &[PathBuf]) -> UpdatePlan {
        let mut plan = UpdatePlan::default();
        let mut seen_walks = HashSet::new();
        for path in paths {
            let Some(mut relative) = self.relative(path) else {
                continue;
            };
            if is_inside_excluded(&self.settings, &relative) {
                continue;
            }
            // Un .gitignore changé : tout son dossier est à revoir.
            if relative == ".gitignore" || relative.ends_with("/.gitignore") {
                relative = parent_of(&relative).to_string();
            }
            if relative.is_empty() {
                plan.full_rescan = true;
                return plan;
            }
            if self.dirs.contains(relative.as_str()) {
                plan.remove_trees.push(relative.clone());
            } else {
                plan.remove_files.push(relative.clone());
            }
            // Le plus proche ancêtre déjà connu, et l'enfant qui y mène.
            let mut ancestor = parent_of(&relative);
            while !ancestor.is_empty() && !self.dirs.contains(ancestor) {
                ancestor = parent_of(ancestor);
            }
            let base = if ancestor.is_empty() {
                0
            } else {
                ancestor.len() + 1
            };
            let child_end = relative[base..]
                .find('/')
                .map_or(relative.len(), |i| base + i);
            let child = relative[..child_end].to_string();
            if seen_walks.insert(child.clone()) {
                plan.walks.push((ancestor.to_string(), child));
            }
        }
        plan
    }

    pub(crate) fn apply_update(&mut self, plan: &UpdatePlan, walked: Vec<Walked>) -> UpdateSummary {
        let mut removed: HashSet<Box<str>> = HashSet::new();
        for tree in &plan.remove_trees {
            removed.extend(self.remove_tree(tree));
        }
        for file in &plan.remove_files {
            if self.remove_file(file) {
                removed.insert(file.clone().into_boxed_str());
            }
        }
        let mut added = 0;
        for item in walked {
            match item {
                Walked::File(relative) => {
                    let was_removed = removed.remove(relative.as_str());
                    if self.insert_file(relative) && !was_removed {
                        added += 1;
                    }
                }
                Walked::Dir(relative) => {
                    self.dirs.insert(relative.into_boxed_str());
                }
            }
        }
        UpdateSummary {
            added,
            removed: removed.len(),
            files: self.len(),
            full_rescan: false,
        }
    }
}

/// Plan d'une mise à jour incrémentale.
#[derive(Debug, Default, Clone)]
pub(crate) struct UpdatePlan {
    pub(crate) full_rescan: bool,
    remove_files: Vec<String>,
    remove_trees: Vec<String>,
    /// (ancêtre connu, enfant à revoir) : on relit l'ancêtre sur un niveau pour
    /// savoir si l'enfant est ignoré, puis on parcourt l'enfant s'il est un dossier.
    walks: Vec<(String, String)>,
}

fn parent_of(relative: &str) -> &str {
    relative.rfind('/').map_or("", |i| &relative[..i])
}

fn is_inside_excluded(settings: &IndexSettings, relative: &str) -> bool {
    let mut end = 0;
    loop {
        let next = relative[end..].find('/').map(|i| end + i);
        let prefix = &relative[..next.unwrap_or(relative.len())];
        if settings.is_excluded(prefix) {
            return true;
        }
        match next {
            Some(i) => end = i + 1,
            None => return false,
        }
    }
}

/// Relit le disque pour un plan de mise à jour (sans toucher à l'index).
pub(crate) fn collect_update(
    root: &Path,
    settings: &IndexSettings,
    plan: &UpdatePlan,
    cancel: &CancelToken,
) -> Result<Vec<Walked>> {
    let mut out = Vec::new();
    for (ancestor, child) in &plan.walks {
        cancel.check()?;
        let start = absolute_path(root, ancestor);
        if !start.is_dir() {
            continue;
        }
        let mut found = None;
        walk(root, &start, Some(1), settings, cancel, &mut |batch| {
            for item in batch {
                let name = match &item {
                    Walked::File(r) | Walked::Dir(r) => r,
                };
                if name == child {
                    found = Some(item);
                }
            }
        })?;
        match found {
            Some(Walked::File(file)) => out.push(Walked::File(file)),
            Some(Walked::Dir(dir)) => {
                out.push(Walked::Dir(dir.clone()));
                let start = absolute_path(root, &dir);
                walk(root, &start, None, settings, cancel, &mut |batch| {
                    out.extend(batch)
                })?;
            }
            None => {}
        }
    }
    Ok(out)
}

/// Taille des paquets remis par le parcours.
const WALK_BATCH: usize = 512;

/// Parcourt `start` (dans le projet `root`) en respectant `.gitignore`,
/// `.git/info/exclude`, l'exclusion globale de Git et les exclusions des
/// réglages. Remet des paquets de chemins relatifs à `root` ; `start` lui-même
/// n'est pas remis. Parallèle quand `max_depth` vaut `None`.
pub(crate) fn walk(
    root: &Path,
    start: &Path,
    max_depth: Option<usize>,
    settings: &IndexSettings,
    cancel: &CancelToken,
    sink: &mut dyn FnMut(Vec<Walked>),
) -> Result<()> {
    let mut builder = WalkBuilder::new(start);
    builder
        .hidden(false)
        .ignore(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .parents(true)
        .require_git(false)
        .follow_links(false)
        .max_depth(max_depth)
        .threads(settings.thread_count());
    {
        let root = root.to_path_buf();
        let settings = settings.clone();
        builder.filter_entry(move |entry| {
            if entry.depth() == 0 {
                return true;
            }
            match relative_string(&root, entry.path()) {
                Some(relative) => !settings.is_excluded(&relative),
                None => false,
            }
        });
    }

    let classify = |entry: &ignore::DirEntry| -> Option<Walked> {
        if entry.depth() == 0 {
            return None;
        }
        let file_type = entry.file_type()?;
        let relative = relative_string(root, entry.path())?;
        if file_type.is_file() {
            Some(Walked::File(relative))
        } else if file_type.is_dir() {
            Some(Walked::Dir(relative))
        } else {
            None // liens symboliques et autres : ignorés
        }
    };

    if max_depth.is_some() {
        let mut batch = Vec::new();
        for entry in builder.build() {
            cancel.check()?;
            if let Some(item) = entry.ok().as_ref().and_then(classify) {
                batch.push(item);
            }
        }
        sink(batch);
        return cancel.check().map_err(Into::into);
    }

    let (sender, receiver) = mpsc::channel::<Vec<Walked>>();
    std::thread::scope(|scope| {
        let walker = builder.build_parallel();
        let classify = &classify;
        scope.spawn(move || {
            walker.run(|| {
                let mut batch = Batch {
                    sender: sender.clone(),
                    items: Vec::with_capacity(WALK_BATCH),
                };
                Box::new(move |entry| {
                    if cancel.is_cancelled() {
                        return WalkState::Quit;
                    }
                    if let Some(item) = entry.ok().as_ref().and_then(classify) {
                        batch.items.push(item);
                        if batch.items.len() >= WALK_BATCH {
                            batch.flush();
                        }
                    }
                    WalkState::Continue
                })
            });
            drop(sender);
        });
        // Les paquets sont regroupés ici pour limiter les prises de verrou de l'appelant.
        let mut pending: Vec<Walked> = Vec::new();
        let mut last = Instant::now();
        loop {
            match receiver.recv_timeout(Duration::from_millis(50)) {
                Ok(mut items) => {
                    if pending.is_empty() {
                        pending = items;
                    } else {
                        pending.append(&mut items);
                    }
                    if pending.len() >= 4 * WALK_BATCH
                        || last.elapsed() >= Duration::from_millis(50)
                    {
                        sink(std::mem::take(&mut pending));
                        last = Instant::now();
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if !pending.is_empty() {
                        sink(std::mem::take(&mut pending));
                        last = Instant::now();
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        if !pending.is_empty() {
            sink(pending);
        }
    });
    cancel.check().map_err(Into::into)
}

/// Paquet d'un fil du parcours, remis à la destruction s'il reste quelque chose.
struct Batch {
    sender: mpsc::Sender<Vec<Walked>>,
    items: Vec<Walked>,
}

impl Batch {
    fn flush(&mut self) {
        if !self.items.is_empty() {
            let items = std::mem::replace(&mut self.items, Vec::with_capacity(WALK_BATCH));
            let _ = self.sender.send(items);
        }
    }
}

impl Drop for Batch {
    fn drop(&mut self) {
        self.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    pub(crate) fn write(root: &Path, relative: &str, content: &str) {
        let path = absolute_path(root, relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn sorted(index: &FileIndex) -> Vec<String> {
        let mut all: Vec<String> = index
            .entries()
            .iter()
            .map(|e| e.relative().to_string())
            .collect();
        all.sort();
        all
    }

    fn sample() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir(root.join(".git")).unwrap();
        write(root, ".git/config", "");
        write(root, ".gitignore", "/cache/\n*.log\n!keep.log\n");
        write(root, "index.php", "<?php");
        write(
            root,
            "src/Controller/UserController.php",
            "<?php class UserController {}",
        );
        write(root, "src/Model/User.php", "<?php");
        write(root, "cache/data.php", "");
        write(root, "logs/app.log", "");
        write(root, "logs/keep.log", "");
        write(root, ".htaccess", "");
        write(root, ".juliestorm/index/x", "");
        write(root, "target/debug/x.rs", "");
        write(root, "src/target/y.php", "");
        write(root, "vendor/acme/lib/Thing.php", "");
        write(root, "web/node_modules/pkg/index.js", "");
        write(root, "docs/accentué/café.md", "");
        dir
    }

    #[test]
    fn scan_respects_gitignore_exclusions_and_marks_libraries() {
        let dir = sample();
        let index =
            FileIndex::scan(dir.path(), IndexSettings::default(), &CancelToken::new()).unwrap();
        assert_eq!(
            sorted(&index),
            vec![
                ".gitignore",
                ".htaccess",
                "docs/accentué/café.md",
                "index.php",
                "logs/keep.log",
                "src/Controller/UserController.php",
                "src/Model/User.php",
                "src/target/y.php",
                "vendor/acme/lib/Thing.php",
                "web/node_modules/pkg/index.js",
            ]
        );
        assert!(index.get("vendor/acme/lib/Thing.php").unwrap().is_library());
        assert!(index
            .get("web/node_modules/pkg/index.js")
            .unwrap()
            .is_library());
        assert!(!index.get("src/Model/User.php").unwrap().is_library());
        let entry = index.get("docs/accentué/café.md").unwrap();
        assert_eq!(entry.name(), "café.md");
        assert_eq!(entry.directory(), "docs/accentué");
        assert_eq!(entry.name_start(), "docs/accentué/".chars().count());
        assert_eq!(
            index.absolute("src/Model/User.php"),
            dir.path().join("src").join("Model").join("User.php")
        );
    }

    #[test]
    fn gitignore_applies_without_git_repository() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), ".gitignore", "secret.txt\n");
        write(dir.path(), "secret.txt", "");
        write(dir.path(), "a.txt", "");
        let index =
            FileIndex::scan(dir.path(), IndexSettings::default(), &CancelToken::new()).unwrap();
        assert_eq!(sorted(&index), vec![".gitignore", "a.txt"]);
    }

    #[test]
    fn project_without_gitignore() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a/b/c.txt", "");
        let index =
            FileIndex::scan(dir.path(), IndexSettings::default(), &CancelToken::new()).unwrap();
        assert_eq!(sorted(&index), vec!["a/b/c.txt"]);
    }

    #[test]
    fn cancelled_scan_stops() {
        let dir = sample();
        let cancel = CancelToken::new();
        cancel.cancel();
        let result = FileIndex::scan(dir.path(), IndexSettings::default(), &cancel);
        assert!(result.unwrap_err().is_cancelled());
    }

    #[test]
    fn incremental_update_follows_the_disk() {
        let dir = sample();
        let root = dir.path();
        let cancel = CancelToken::new();
        let mut index = FileIndex::scan(root, IndexSettings::default(), &cancel).unwrap();

        // Création d'un fichier, d'un fichier ignoré, d'une arborescence neuve.
        write(root, "src/Model/Group.php", "");
        write(root, "src/Model/debug.log", "");
        write(root, "new/deep/tree/A.php", "");
        write(root, "cache/more.php", "");
        let summary = index
            .update(
                &[
                    root.join("src/Model/Group.php"),
                    root.join("src/Model/debug.log"),
                    root.join("new/deep/tree/A.php"),
                    root.join("cache/more.php"),
                    root.join(".git/index"),
                ],
                &cancel,
            )
            .unwrap();
        assert_eq!(summary.added, 2, "{summary:?}");
        assert_eq!(summary.removed, 0);
        assert!(index.contains("src/Model/Group.php"));
        assert!(index.contains("new/deep/tree/A.php"));
        assert!(!index.contains("src/Model/debug.log"));
        assert!(!index.contains("cache/more.php"));

        // Modification seule : rien ne change dans l'index.
        let summary = index.update(&[root.join("index.php")], &cancel).unwrap();
        assert_eq!((summary.added, summary.removed), (0, 0));

        // Suppression d'un fichier et d'un dossier entier.
        fs::remove_file(root.join("index.php")).unwrap();
        fs::remove_dir_all(root.join("src/Controller")).unwrap();
        let summary = index
            .update(
                &[root.join("index.php"), root.join("src/Controller")],
                &cancel,
            )
            .unwrap();
        assert_eq!(summary.removed, 2);
        assert!(!index.contains("index.php"));
        assert!(!index.contains("src/Controller/UserController.php"));

        // Renommage d'un dossier : ancien et nouveau chemin signalés.
        fs::rename(root.join("src/Model"), root.join("src/Entity")).unwrap();
        let summary = index
            .update(&[root.join("src/Model"), root.join("src/Entity")], &cancel)
            .unwrap();
        assert_eq!((summary.added, summary.removed), (2, 2), "{summary:?}");
        assert!(index.contains("src/Entity/User.php"));
        assert!(!index.contains("src/Model/User.php"));

        // Un .gitignore de sous-dossier ajouté : son dossier est revu.
        write(root, "src/Entity/.gitignore", "Group.php\n");
        index
            .update(&[root.join("src/Entity/.gitignore")], &cancel)
            .unwrap();
        assert!(!index.contains("src/Entity/Group.php"));
        assert!(index.contains("src/Entity/.gitignore"));

        // Le .gitignore racine change : parcours complet.
        write(root, ".gitignore", "");
        let summary = index.update(&[root.join(".gitignore")], &cancel).unwrap();
        assert!(summary.full_rescan);
        assert!(index.contains("cache/data.php"));
        assert!(index.contains("logs/app.log"));

        // Le résultat est celui d'un parcours neuf.
        let fresh = FileIndex::scan(root, IndexSettings::default(), &cancel).unwrap();
        assert_eq!(sorted(&index), sorted(&fresh));
    }

    #[test]
    fn lowercase_keeps_character_alignment() {
        let entry = FileEntry::new("Dossier/İstanbul/ÉTÉ.PHP".into(), false);
        assert_eq!(
            entry.lower.chars().count(),
            entry.relative().chars().count()
        );
        assert_eq!(&entry.lower[entry.lower_name_byte as usize..], "été.php");
    }
}
