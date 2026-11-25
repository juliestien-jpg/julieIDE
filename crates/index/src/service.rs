//! Le service asynchrone : chaque méthode lance une tâche `Domain::Index` et
//! rend la main tout de suite ; les résultats arrivent en [`IndexEvent`].

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::time::{Duration, Instant};

use julie_core::{Domain, Project, TaskContext, TaskHandle, TaskId, Tasks};

use crate::error::Result;
use crate::files::{collect_update, relative_string, walk, FileIndex, UpdateSummary};
use crate::fuzzy::{FileResults, DEFAULT_LIMIT};
use crate::settings::IndexSettings;
use crate::symbols::model::fnv;
use crate::symbols::{
    build_into, find_usages, update_into, Analysis, BuildInput, Lang, Navigation,
    NavigationRequest, NavigationResult, NoTarget, SymbolBuildSummary, SymbolIndex, Usage,
    UsagesQuery, UsagesSummary,
};
use crate::text::{
    files_to_search, preview_file, search_files, FileMask, FileTextMatches, Matcher, Preview,
    SearchOptions, TextQuery, TextSearchSummary, UnsavedFiles,
};

/// Ce que publie `julie-index` (sous `Domain::Index`). Le `task` du
/// `DomainEvent` dit de quelle demande vient le résultat : l'interface ignore
/// ceux d'une demande qui n'est plus la courante.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexEvent {
    /// Indexation initiale (ou complète) des fichiers terminée.
    ScanFinished(ScanSummary),
    /// Mise à jour incrémentale des fichiers appliquée.
    Updated(UpdateSummary),
    /// Résultat d'« aller au fichier » (un seul événement par demande).
    Files(FileResults),
    /// Un paquet de résultats de recherche dans les fichiers (plusieurs par
    /// recherche, chaque fichier en une fois, dans un ordre quelconque).
    TextMatches(Vec<FileTextMatches>),
    /// Recherche dans les fichiers terminée (n'arrive pas si elle est annulée).
    TextSearchFinished(TextSearchSummary),
    /// Lignes de contexte demandées par [`IndexService::preview`].
    Preview(Preview),
    /// Index des symboles construit (tâche `index.symbols`).
    SymbolsReady(SymbolBuildSummary),
    /// Fichiers réindexés dans l'index des symboles (disque, onglet).
    SymbolsUpdated { files: usize },
    /// Réponse d'[`IndexService::navigate`] (une par demande).
    Navigation(NavigationResult),
    /// Un paquet d'usages (plusieurs par recherche, chaque fichier en une fois).
    Usages(Vec<Usage>),
    /// Recherche d'usages terminée (n'arrive pas si elle est annulée).
    UsagesFinished(UsagesSummary),
}

/// Bilan de l'indexation initiale.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScanSummary {
    pub files: usize,
    /// Dont fichiers de bibliothèques (`vendor`, `node_modules`).
    pub library_files: usize,
    pub elapsed: Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    NotScanned,
    Scanning(TaskId),
    Ready,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SymbolPhase {
    NotBuilt,
    /// En construction (la tâche, fichiers faits, total).
    Building(Option<TaskId>, usize, usize),
    Ready,
}

/// Analyse gardée du dernier fichier où l'on a navigué (survol avec Ctrl :
/// la même analyse sert tant que le texte ne change pas).
type AnalysisCache = Option<(PathBuf, u64, Arc<Analysis>)>;

struct Shared {
    index: RwLock<FileIndex>,
    phase: Mutex<Phase>,
    phase_changed: Condvar,
    /// Sérialise les mises à jour incrémentales entre elles.
    updating: Mutex<()>,
    settings: RwLock<IndexSettings>,
    symbols: RwLock<SymbolIndex>,
    symbol_phase: Mutex<SymbolPhase>,
    symbol_phase_changed: Condvar,
    unsaved: Mutex<UnsavedFiles>,
    last_analysis: Mutex<AnalysisCache>,
}

fn read<T>(lock: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    lock.read().unwrap_or_else(|p| p.into_inner())
}

fn write<T>(lock: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
    lock.write().unwrap_or_else(|p| p.into_inner())
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|p| p.into_inner())
}

impl Shared {
    fn read(&self) -> RwLockReadGuard<'_, FileIndex> {
        read(&self.index)
    }

    fn write(&self) -> RwLockWriteGuard<'_, FileIndex> {
        write(&self.index)
    }

    fn phase(&self) -> MutexGuard<'_, Phase> {
        lock(&self.phase)
    }

    fn settings(&self) -> IndexSettings {
        read(&self.settings).clone()
    }

    fn unsaved(&self) -> UnsavedFiles {
        lock(&self.unsaved).clone()
    }

    /// Attend la fin d'une indexation en cours (annulable).
    fn wait_scanned(&self, ctx: &TaskContext) -> Result<()> {
        let mut phase = self.phase();
        while matches!(*phase, Phase::Scanning(_)) {
            ctx.check_cancelled()?;
            phase = self
                .phase_changed
                .wait_timeout(phase, Duration::from_millis(20))
                .unwrap_or_else(|p| p.into_inner())
                .0;
        }
        Ok(())
    }

    /// Attend la fin de la construction de l'index des symboles (annulable).
    fn wait_symbols(&self, ctx: &TaskContext) -> Result<()> {
        let mut phase = lock(&self.symbol_phase);
        while matches!(*phase, SymbolPhase::Building(..)) {
            ctx.check_cancelled()?;
            phase = self
                .symbol_phase_changed
                .wait_timeout(phase, Duration::from_millis(20))
                .unwrap_or_else(|p| p.into_inner())
                .0;
        }
        Ok(())
    }

    /// `None` : index des symboles prêt ; sinon pourcentage construit.
    fn symbols_progress(&self) -> Option<u8> {
        match *lock(&self.symbol_phase) {
            SymbolPhase::Ready => None,
            SymbolPhase::NotBuilt => Some(0),
            SymbolPhase::Building(_, done, total) => {
                Some((done * 100).checked_div(total).unwrap_or(0).min(99) as u8)
            }
        }
    }

    fn set_symbol_phase(&self, phase: SymbolPhase) {
        *lock(&self.symbol_phase) = phase;
        self.symbol_phase_changed.notify_all();
    }
}

/// Le service d'index et de recherche d'un projet. Construit par `app` à
/// l'ouverture du projet ; ses méthodes s'appellent depuis le fil de
/// l'interface (aucune n'y fait d'accès disque).
pub struct IndexService {
    tasks: Tasks,
    project: Project,
    shared: Arc<Shared>,
    /// Dossier du cache des symboles (`None` : pas de cache).
    cache_dir: Option<PathBuf>,
    find_task: Mutex<Option<TaskId>>,
    search_task: Mutex<Option<TaskId>>,
    preview_task: Mutex<Option<TaskId>>,
    navigate_task: Mutex<Option<TaskId>>,
    usages_task: Mutex<Option<TaskId>>,
}

impl IndexService {
    /// Le cache des symboles va dans `<projet>/.juliestorm/index/`.
    pub fn new(tasks: Tasks, project: Project, settings: IndexSettings) -> Self {
        let index = FileIndex::new(project.root().to_path_buf(), settings.clone());
        let cache_dir = Some(project.data_dir().join("index"));
        Self {
            tasks,
            shared: Arc::new(Shared {
                index: RwLock::new(index),
                phase: Mutex::new(Phase::NotScanned),
                phase_changed: Condvar::new(),
                updating: Mutex::new(()),
                settings: RwLock::new(settings),
                symbols: RwLock::new(SymbolIndex::new(project.root())),
                symbol_phase: Mutex::new(SymbolPhase::NotBuilt),
                symbol_phase_changed: Condvar::new(),
                unsaved: Mutex::new(UnsavedFiles::new()),
                last_analysis: Mutex::new(None),
            }),
            project,
            cache_dir,
            find_task: Mutex::new(None),
            search_task: Mutex::new(None),
            preview_task: Mutex::new(None),
            navigate_task: Mutex::new(None),
            usages_task: Mutex::new(None),
        }
    }

    /// Change le dossier du cache des symboles (`None` : sans cache). **Pour
    /// les mesures et les essais** sur un projet où l'on ne doit rien écrire ;
    /// l'IDE garde le dossier du projet.
    pub fn with_symbol_cache(mut self, dir: Option<PathBuf>) -> Self {
        self.cache_dir = dir;
        self
    }

    pub fn project(&self) -> &Project {
        &self.project
    }

    /// Réglages en vigueur.
    pub fn settings(&self) -> IndexSettings {
        self.shared.settings()
    }

    /// Nouveaux réglages `[index]` (après `Event::SettingsChanged` qui concerne
    /// `index`). Si quelque chose change, toute l'indexation est relancée
    /// (fichiers puis symboles) et la tâche `index.scan` est rendue ; sinon
    /// `None`. Les recherches en cours finissent avec les anciens réglages.
    pub fn set_settings(&self, settings: IndexSettings) -> Option<TaskHandle> {
        {
            let mut current = write(&self.shared.settings);
            if *current == settings {
                return None;
            }
            *current = settings;
        }
        Some(self.scan())
    }

    /// L'indexation initiale des fichiers est finie (sinon « Indexation… »).
    pub fn is_ready(&self) -> bool {
        matches!(*self.shared.phase(), Phase::Ready)
    }

    /// `None` quand l'index des symboles est prêt ; sinon le pourcentage déjà
    /// construit (barre d'état « Indexation des symboles… n % »).
    pub fn symbols_progress(&self) -> Option<u8> {
        self.shared.symbols_progress()
    }

    /// Indexation complète (`index.scan`). Vide l'index et le remplit au fil du
    /// parcours : « aller au fichier » rend des résultats partiels pendant ce
    /// temps (`FileResults::index_complete == false`). Progression : événements
    /// de tâche `Progress { done: fichiers trouvés, total: 0 }`. À la fin :
    /// `IndexEvent::ScanFinished`, puis la construction de l'index des
    /// symboles est lancée (tâche `index.symbols`). Une indexation en cours est
    /// annulée.
    pub fn scan(&self) -> TaskHandle {
        let shared = Arc::clone(&self.shared);
        let root = self.project.root().to_path_buf();
        let settings = self.shared.settings();
        let tasks = self.tasks.clone();
        let project = self.project.clone();
        let cache_dir = self.cache_dir.clone();
        // Le verrou de phase est tenu pendant `spawn` : la tâche ne peut pas
        // finir avant que sa phase soit posée.
        let mut phase = self.shared.phase();
        if let Phase::Scanning(previous) = *phase {
            self.tasks.cancel(previous);
        }
        if let SymbolPhase::Building(Some(previous), _, _) = *lock(&self.shared.symbol_phase) {
            self.tasks.cancel(previous);
        }
        self.shared
            .set_symbol_phase(SymbolPhase::Building(None, 0, 0));
        let handle = self.tasks.spawn(Domain::Index, "index.scan", move |ctx| {
            let started = Instant::now();
            let generation = {
                let mut index = shared.write();
                index.reset(settings.clone());
                index.generation
            };
            let mut done = 0u64;
            let result = walk(
                &root,
                &root,
                None,
                &settings,
                ctx.cancel_token(),
                &mut |batch| {
                    let mut index = shared.write();
                    if index.generation == generation {
                        done += batch
                            .iter()
                            .filter(|w| matches!(w, crate::files::Walked::File(_)))
                            .count() as u64;
                        index.insert_walked(batch);
                    }
                    drop(index);
                    ctx.progress(done, 0);
                },
            );
            {
                let mut phase = shared.phase();
                if *phase == Phase::Scanning(ctx.id()) {
                    *phase = Phase::Ready;
                    shared.phase_changed.notify_all();
                }
            }
            if let Err(error) = result {
                shared.set_symbol_phase(SymbolPhase::NotBuilt);
                return Err(error.into());
            }
            let index = shared.read();
            let summary = ScanSummary {
                files: index.len(),
                library_files: index.entries().iter().filter(|e| e.is_library()).count(),
                elapsed: started.elapsed(),
            };
            drop(index);
            ctx.publish(IndexEvent::ScanFinished(summary));
            spawn_symbols(&tasks, &shared, &project, cache_dir);
            Ok(())
        });
        *phase = Phase::Scanning(handle.id());
        drop(phase);
        self.shared.phase_changed.notify_all();
        handle
    }

    /// Reconstruit l'index des symboles (`index.symbols`) sans reparcourir les
    /// fichiers (attend la fin d'un `scan`). `scan()` le fait déjà.
    pub fn build_symbols(&self) -> TaskHandle {
        if let SymbolPhase::Building(Some(previous), _, _) = *lock(&self.shared.symbol_phase) {
            self.tasks.cancel(previous);
        }
        self.shared
            .set_symbol_phase(SymbolPhase::Building(None, 0, 0));
        spawn_symbols(
            &self.tasks,
            &self.shared,
            &self.project,
            self.cache_dir.clone(),
        )
    }

    /// Mise à jour incrémentale (`index.update`) pour des chemins absolus changés
    /// sur le disque (le contenu de `Event::FilesChangedOnDisk`). Attend la fin
    /// d'une indexation en cours. Publie `IndexEvent::Updated`, puis
    /// `IndexEvent::SymbolsUpdated` si des fichiers PHP ou JS/TS ont été
    /// réindexés. Un `.gitignore` changé fait revoir son dossier ; celui de la
    /// racine, tout le projet.
    pub fn update(&self, paths: Vec<PathBuf>) -> TaskHandle {
        let shared = Arc::clone(&self.shared);
        let root = self.project.root().to_path_buf();
        self.tasks.spawn(Domain::Index, "index.update", move |ctx| {
            shared.wait_scanned(ctx)?;
            let _serial = shared.updating.lock().unwrap_or_else(|p| p.into_inner());
            let settings = shared.settings();
            let plan = shared.read().plan_update(&paths);
            let summary = if plan.full_rescan {
                let fresh = FileIndex::scan(root.clone(), settings.clone(), ctx.cancel_token())?;
                shared.write().replace_with(fresh)
            } else {
                let walked = collect_update(&root, &settings, &plan, ctx.cancel_token())?;
                ctx.check_cancelled()?;
                shared.write().apply_update(&plan, walked)
            };
            ctx.publish(IndexEvent::Updated(summary));

            shared.wait_symbols(ctx)?;
            let work = {
                let files = shared.read();
                let symbols = read(&shared.symbols);
                if summary.full_rescan {
                    symbols.affected(&files, std::slice::from_ref(&root))
                } else {
                    symbols.affected(&files, &paths)
                }
            };
            let count = apply_symbol_work(&shared, &root, work, &settings, ctx)?;
            if count > 0 {
                ctx.publish(IndexEvent::SymbolsUpdated { files: count });
            }
            Ok(())
        })
    }

    /// Contenu d'un onglet modifié non enregistré (`Some`), ou retour au disque
    /// (`None` : enregistré, fermé, modifications annulées). La navigation, les
    /// usages et l'index des symboles le prennent en compte ; la tâche
    /// `index.unsaved` réindexe ce fichier et publie `SymbolsUpdated`. À
    /// appeler après une pause de frappe (300 ms), pas à chaque touche.
    pub fn set_unsaved(&self, path: PathBuf, text: Option<Arc<str>>) -> TaskHandle {
        {
            let mut unsaved = lock(&self.shared.unsaved);
            match &text {
                Some(t) => {
                    unsaved.insert(path.clone(), t.clone());
                }
                None => {
                    unsaved.remove(&path);
                }
            }
        }
        let shared = Arc::clone(&self.shared);
        let root = self.project.root().to_path_buf();
        self.tasks
            .spawn(Domain::Index, "index.unsaved", move |ctx| {
                shared.wait_symbols(ctx)?;
                let settings = shared.settings();
                let Some(relative) = relative_string(&root, &path) else {
                    return Ok(());
                };
                let present = shared.read().get(&relative).map(|e| e.is_library());
                let library = present.unwrap_or_else(|| settings.is_library(&relative));
                let work = match (&text, present) {
                    (Some(_), _) => vec![(relative, Some(library))],
                    (None, present) => vec![(relative, present)],
                };
                let count = apply_symbol_work(&shared, &root, work, &settings, ctx)?;
                if count > 0 {
                    ctx.publish(IndexEvent::SymbolsUpdated { files: count });
                }
                Ok(())
            })
    }

    /// « Aller au fichier » (`index.find_files`) : annule la demande précédente
    /// et publie un `IndexEvent::Files` (100 résultats au plus). `recent` : les
    /// fichiers récemment ouverts, le plus récent d'abord.
    pub fn find_files(&self, query: impl Into<String>, recent: Vec<PathBuf>) -> TaskHandle {
        let query = query.into();
        let shared = Arc::clone(&self.shared);
        let mut current = lock(&self.find_task);
        if let Some(previous) = current.take() {
            self.tasks.cancel(previous);
        }
        let handle = self
            .tasks
            .spawn(Domain::Index, "index.find_files", move |ctx| {
                let complete = matches!(*shared.phase(), Phase::Ready);
                let mut results =
                    shared
                        .read()
                        .find_files(&query, &recent, DEFAULT_LIMIT, ctx.cancel_token())?;
                results.index_complete = complete;
                ctx.publish(IndexEvent::Files(results));
                Ok(())
            });
        *current = Some(handle.id());
        handle
    }

    /// Recherche dans les fichiers (`index.search_text`). Le motif et le masque
    /// sont vérifiés tout de suite : une erreur (`InvalidPattern`,
    /// `InvalidMask`) revient ici, sans tâche. La recherche précédente est
    /// annulée. Publie des `IndexEvent::TextMatches` en flux puis
    /// `IndexEvent::TextSearchFinished`. `unsaved` : contenus des onglets
    /// modifiés non enregistrés, cherchés à la place du disque. Si l'indexation
    /// initiale n'est pas finie, la tâche l'attend.
    pub fn search_text(&self, query: TextQuery, unsaved: UnsavedFiles) -> Result<TaskHandle> {
        let matcher = Matcher::new(&query)?;
        let mask = FileMask::parse(&query.file_mask)?;
        let shared = Arc::clone(&self.shared);
        let options = SearchOptions::new(&query, &self.shared.settings());
        // La portée est vérifiée tout de suite aussi.
        if let crate::text::SearchScope::Directory(dir) = &query.scope {
            if self.project.relative(dir).is_none() {
                return Err(crate::error::IndexError::OutsideProject(dir.clone()));
            }
        }
        let mut current = lock(&self.search_task);
        if let Some(previous) = current.take() {
            self.tasks.cancel(previous);
        }
        let handle = self
            .tasks
            .spawn(Domain::Index, "index.search_text", move |ctx| {
                shared.wait_scanned(ctx)?;
                let files = files_to_search(&shared.read(), &query, &mask, &unsaved)?;
                let summary = search_files(
                    &files,
                    &matcher,
                    &options,
                    ctx.cancel_token(),
                    &mut |batch| ctx.publish(IndexEvent::TextMatches(batch)),
                )?;
                ctx.publish(IndexEvent::TextSearchFinished(summary));
                Ok(())
            });
        *current = Some(handle.id());
        Ok(handle)
    }

    /// Annule la recherche dans les fichiers en cours (fenêtre fermée).
    pub fn cancel_search(&self) {
        if let Some(previous) = lock(&self.search_task).take() {
            self.tasks.cancel(previous);
        }
    }

    /// Aperçu (`index.preview`) : `context` lignes avant et après `line` (base
    /// zéro) de `path`, occurrences de `query` surlignées si elle est donnée.
    /// `unsaved` : contenu non enregistré du fichier, s'il y en a. Annule
    /// l'aperçu précédent. Publie `IndexEvent::Preview`.
    pub fn preview(
        &self,
        path: PathBuf,
        line: u32,
        context: u32,
        query: Option<TextQuery>,
        unsaved: Option<Arc<str>>,
    ) -> Result<TaskHandle> {
        let matcher = query.as_ref().map(Matcher::new).transpose()?;
        let mut current = lock(&self.preview_task);
        if let Some(previous) = current.take() {
            self.tasks.cancel(previous);
        }
        let handle = self
            .tasks
            .spawn(Domain::Index, "index.preview", move |ctx| {
                let preview = preview_file(
                    &path,
                    unsaved.as_deref(),
                    line,
                    context,
                    context,
                    matcher.as_ref(),
                )?;
                ctx.check_cancelled()?;
                ctx.publish(IndexEvent::Preview(preview));
                Ok(())
            });
        *current = Some(handle.id());
        Ok(handle)
    }

    /// Aller à la déclaration, ou survol avec Ctrl (`index.navigate`) : ce
    /// qu'il y a sous `request.position` dans `request.text` (le texte de
    /// l'éditeur, enregistré ou non). Annule la demande précédente ; publie un
    /// `IndexEvent::Navigation`. Répond avec l'index tel qu'il est, même en
    /// construction (`NavigationResult::indexing`). Le fichier est réindexé
    /// avec ce texte s'il a changé. Analyse gardée tant que le texte ne change
    /// pas : un survol répété coûte une recherche dans l'index.
    pub fn navigate(&self, request: NavigationRequest) -> TaskHandle {
        let shared = Arc::clone(&self.shared);
        let mut current = lock(&self.navigate_task);
        if let Some(previous) = current.take() {
            self.tasks.cancel(previous);
        }
        let handle = self
            .tasks
            .spawn(Domain::Index, "index.navigate", move |ctx| {
                let result = navigate_now(&shared, &request);
                ctx.check_cancelled()?;
                ctx.publish(IndexEvent::Navigation(result));
                Ok(())
            });
        *current = Some(handle.id());
        handle
    }

    /// Trouver les usages (`index.usages`) : annule la recherche précédente,
    /// publie des `IndexEvent::Usages` en flux puis `IndexEvent::UsagesFinished`.
    /// Contenus non enregistrés : ceux donnés à [`IndexService::set_unsaved`].
    /// Pendant la construction de l'index des symboles, cherche avec ce qui est
    /// indexé (`UsagesSummary::index_complete == false`).
    pub fn find_usages(&self, query: UsagesQuery) -> TaskHandle {
        let shared = Arc::clone(&self.shared);
        let mut current = lock(&self.usages_task);
        if let Some(previous) = current.take() {
            self.tasks.cancel(previous);
        }
        let handle = self.tasks.spawn(Domain::Index, "index.usages", move |ctx| {
            let complete = shared.symbols_progress().is_none();
            let settings = shared.settings();
            let unsaved = shared.unsaved();
            let symbols = read(&shared.symbols);
            let summary = find_usages(
                &symbols,
                &query,
                &unsaved,
                &settings,
                complete,
                ctx.cancel_token(),
                &mut |batch| ctx.publish(IndexEvent::Usages(batch)),
            )?;
            drop(symbols);
            ctx.publish(IndexEvent::UsagesFinished(summary));
            Ok(())
        });
        *current = Some(handle.id());
        handle
    }

    /// Annule la recherche d'usages en cours (onglet de la fenêtre Usages
    /// fermé, liste surgissante fermée).
    pub fn cancel_usages(&self) {
        if let Some(previous) = lock(&self.usages_task).take() {
            self.tasks.cancel(previous);
        }
    }

    /// Écrit le cache des symboles maintenant (`index.symbols_cache`), par
    /// exemple à la fermeture du projet. La construction l'écrit déjà.
    pub fn save_symbol_cache(&self) -> TaskHandle {
        let shared = Arc::clone(&self.shared);
        let dir = self.cache_dir.clone();
        let project = self.project.clone();
        self.tasks
            .spawn(Domain::Index, "index.symbols_cache", move |ctx| {
                shared.wait_symbols(ctx)?;
                if let Some(dir) = dir {
                    prepare_cache_dir(&project, &dir)?;
                    read(&shared.symbols).save_cache(&dir)?;
                }
                Ok(())
            })
    }

    /// Donne accès à l'index courant. **Peut bloquer** quelques millisecondes
    /// pendant une écriture : pour les tests et les tâches, pas pour le fil de
    /// l'interface.
    pub fn with_index<R>(&self, f: impl FnOnce(&FileIndex) -> R) -> R {
        f(&self.shared.read())
    }

    /// Accès à l'index des symboles. **Peut bloquer** : tests et tâches.
    pub fn with_symbols<R>(&self, f: impl FnOnce(&SymbolIndex) -> R) -> R {
        f(&read(&self.shared.symbols))
    }
}

/// Le dossier du cache dans le projet a besoin de `.juliestorm/` (et de son
/// `.gitignore`) ; un dossier hors du projet (mesures) est créé tel quel.
fn prepare_cache_dir(project: &Project, dir: &Path) -> Result<()> {
    if dir.starts_with(project.data_dir()) {
        project.ensure_data_dir()?;
    }
    std::fs::create_dir_all(dir).map_err(|e| julie_core::Error::io(dir, e))?;
    Ok(())
}

/// Lance la construction de l'index des symboles (`index.symbols`).
fn spawn_symbols(
    tasks: &Tasks,
    shared: &Arc<Shared>,
    project: &Project,
    cache_dir: Option<PathBuf>,
) -> TaskHandle {
    let shared = Arc::clone(shared);
    let project = project.clone();
    // La phase est posée sous le verrou pendant `spawn` (voir `scan`).
    let mut phase = lock(&shared.symbol_phase);
    let task_shared = Arc::clone(&shared);
    let handle = tasks.spawn(Domain::Index, "index.symbols", move |ctx| {
        let shared = task_shared;
        shared.wait_scanned(ctx)?;
        let started = Instant::now();
        let unsaved = shared.unsaved();
        let (input, cache_loaded) = {
            let files = shared.read();
            BuildInput::new(&files, cache_dir.as_deref(), &unsaved)
        };
        let total = input.entries.len();
        {
            let mut phase = lock(&shared.symbol_phase);
            if *phase == SymbolPhase::Building(Some(ctx.id()), 0, 0) {
                *phase = SymbolPhase::Building(Some(ctx.id()), 0, total);
            }
        }
        write(&shared.symbols).reset(project.root().to_path_buf());
        let result = build_into(input, ctx.cancel_token(), &mut |batch, done, total| {
            {
                let mut symbols = write(&shared.symbols);
                for b in batch {
                    match b.symbols {
                        Some(s) => {
                            symbols.insert(&b.relative, b.library, b.stamp, s, b.from_memory)
                        }
                        None => {
                            symbols.remove(&b.relative);
                        }
                    }
                }
            }
            {
                let mut phase = lock(&shared.symbol_phase);
                if matches!(*phase, SymbolPhase::Building(Some(id), _, _) if id == ctx.id()) {
                    *phase = SymbolPhase::Building(Some(ctx.id()), done, total);
                }
            }
            ctx.progress(done as u64, total as u64);
        });
        let mut summary = match result {
            Ok(s) => s,
            Err(e) => {
                let mut phase = lock(&shared.symbol_phase);
                if matches!(*phase, SymbolPhase::Building(Some(id), _, _) if id == ctx.id()) {
                    *phase = SymbolPhase::NotBuilt;
                    shared.symbol_phase_changed.notify_all();
                }
                return Err(e.into());
            }
        };
        {
            let mut phase = lock(&shared.symbol_phase);
            if matches!(*phase, SymbolPhase::Building(Some(id), _, _) if id == ctx.id()) {
                *phase = SymbolPhase::Ready;
                shared.symbol_phase_changed.notify_all();
            }
        }
        summary.cache_loaded = cache_loaded;
        if let Some(dir) = &cache_dir {
            if summary.parsed > 0 || !cache_loaded {
                summary.cache_written = prepare_cache_dir(&project, dir).is_ok()
                    && read(&shared.symbols).save_cache(dir).is_ok();
            }
        }
        {
            let symbols = read(&shared.symbols);
            summary.files = symbols.file_count();
            summary.declarations = symbols.declaration_count();
        }
        summary.elapsed = started.elapsed();
        ctx.publish(IndexEvent::SymbolsReady(summary));
        Ok(())
    });
    *phase = SymbolPhase::Building(Some(handle.id()), 0, 0);
    drop(phase);
    shared.symbol_phase_changed.notify_all();
    handle
}

/// Réindexe ou retire des fichiers de l'index des symboles.
fn apply_symbol_work(
    shared: &Shared,
    root: &Path,
    work: Vec<(String, Option<bool>)>,
    settings: &IndexSettings,
    ctx: &TaskContext,
) -> Result<usize> {
    if work.is_empty() {
        return Ok(0);
    }
    let unsaved = shared.unsaved();
    let mut results = Vec::new();
    update_into(
        root.to_path_buf(),
        work,
        settings.max_file_size,
        &unsaved,
        ctx.cancel_token(),
        &mut |relative, result| results.push((relative.to_string(), result)),
    )?;
    let count = results.len();
    let mut symbols = write(&shared.symbols);
    for (relative, result) in results {
        match result {
            Some((library, stamp, s, from_memory)) => {
                symbols.insert(&relative, library, stamp, s, from_memory)
            }
            None => {
                symbols.remove(&relative);
            }
        }
    }
    drop(symbols);
    *lock(&shared.last_analysis) = None;
    Ok(count)
}

/// Le travail d'`index.navigate`, sans bus.
fn navigate_now(shared: &Shared, request: &NavigationRequest) -> NavigationResult {
    let nothing = || NavigationResult {
        path: request.path.clone(),
        position: request.position,
        word: None,
        navigation: Navigation::Nothing(NoTarget::NotASymbol),
        indexing: shared.symbols_progress(),
    };
    let Some(lang) = Lang::from_path(&request.path) else {
        return nothing();
    };
    let hash = fnv(request.text.as_bytes());
    let cached = lock(&shared.last_analysis)
        .as_ref()
        .filter(|(p, h, _)| *p == request.path && *h == hash)
        .map(|(_, _, a)| Arc::clone(a));
    let analysis = match cached {
        Some(a) => a,
        None => {
            let text = request.text.clone();
            let Some(a) = crate::symbols::store_analyze(lang, &text) else {
                return nothing();
            };
            let a = Arc::new(a);
            *lock(&shared.last_analysis) = Some((request.path.clone(), hash, Arc::clone(&a)));
            a
        }
    };
    let root = read(&shared.symbols).root().to_path_buf();
    if let Some(relative) = relative_string(&root, &request.path) {
        let library = shared
            .read()
            .get(&relative)
            .map(|e| e.is_library())
            .unwrap_or_else(|| shared.settings().is_library(&relative));
        let unchanged = read(&shared.symbols)
            .slot_content_hash(&relative)
            .is_some_and(|h| h == analysis.symbols.content_hash);
        // Sans attendre : si une écriture est impossible tout de suite (usages en
        // cours…), on résout avec l'index tel qu'il est.
        if !unchanged {
            if let Ok(mut symbols) = shared.symbols.try_write() {
                symbols.insert_analysis(&relative, library, &analysis);
            }
        }
    }
    let unsaved: HashMap<PathBuf, Arc<str>> = shared.unsaved();
    let settings = shared.settings();
    let symbols = read(&shared.symbols);
    crate::symbols::navigate_with(
        &symbols,
        request,
        &analysis,
        &unsaved,
        shared.symbols_progress(),
        settings.max_file_size,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use julie_core::{Bus, Event, TaskOutcome, TaskState};
    use std::fs;

    fn write(root: &std::path::Path, relative: &str, content: &str) {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn index_events(bus: &Bus) -> Vec<(Option<TaskId>, IndexEvent)> {
        bus.drain()
            .into_iter()
            .filter_map(|event| match event {
                Event::Domain(event) => {
                    let task = event.task;
                    event.get::<IndexEvent>().cloned().map(|e| (task, e))
                }
                _ => None,
            })
            .collect()
    }

    fn setup() -> (tempfile::TempDir, Bus, IndexService) {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), ".gitignore", "ignored/\n");
        write(
            dir.path(),
            "src/UserController.php",
            "<?php\nclass UserController {}\n",
        );
        write(dir.path(), "src/User.php", "<?php\nclass User {}\n");
        write(dir.path(), "ignored/User.php", "class User");
        write(dir.path(), "web/app.js", "const user = 1;\n");
        let bus = Bus::new();
        let tasks = Tasks::new(bus.publisher());
        let project = Project::open(dir.path()).unwrap();
        let service =
            IndexService::new(tasks, project, IndexSettings::default()).with_symbol_cache(None);
        (dir, bus, service)
    }

    /// Attend la fin de la construction des symboles lancée par `scan`.
    fn wait_symbols(service: &IndexService) {
        let started = Instant::now();
        while service.symbols_progress().is_some() {
            assert!(
                started.elapsed() < Duration::from_secs(30),
                "symboles trop lents"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Événements jusqu'à `SymbolsReady` compris (publié juste après le
    /// passage à « prêt »).
    fn events_until_ready(bus: &Bus) -> Vec<(Option<TaskId>, IndexEvent)> {
        let started = Instant::now();
        let mut out = Vec::new();
        loop {
            assert!(
                started.elapsed() < Duration::from_secs(30),
                "SymbolsReady absent"
            );
            if let Some(Event::Domain(e)) = bus.recv_timeout(Duration::from_millis(100)) {
                let task = e.task;
                if let Some(event) = e.get::<IndexEvent>().cloned() {
                    let done = matches!(event, IndexEvent::SymbolsReady(_));
                    out.push((task, event));
                    if done {
                        return out;
                    }
                }
            }
        }
    }

    #[test]
    fn scan_then_find_then_search() {
        let (dir, bus, service) = setup();
        assert!(!service.is_ready());
        service.scan().join();
        assert!(service.is_ready());
        let events = events_until_ready(&bus);
        assert!(
            matches!(
                &events[..],
                [
                    (_, IndexEvent::ScanFinished(ScanSummary { files: 4, .. })),
                    (
                        _,
                        IndexEvent::SymbolsReady(SymbolBuildSummary { files: 3, .. })
                    )
                ]
            ),
            "{events:?}"
        );

        let handle = service.find_files("usct", vec![]);
        let id = handle.id();
        handle.join();
        let events = index_events(&bus);
        let [(task, IndexEvent::Files(results))] = &events[..] else {
            panic!("{events:?}")
        };
        assert_eq!(*task, Some(id));
        assert!(results.index_complete);
        assert_eq!(results.matches[0].relative, "src/UserController.php");

        // Un onglet non enregistré : son contenu en mémoire fait foi.
        let mut unsaved = UnsavedFiles::new();
        unsaved.insert(
            dir.path().join("web").join("app.js"),
            Arc::from("let User = 2;\n// user\n"),
        );
        unsaved.insert(
            dir.path().join("src").join("New.php"),
            Arc::from("new User"),
        );
        let mut query = TextQuery::new("user");
        query.whole_word = true;
        service.search_text(query, unsaved).unwrap().join();
        let events = index_events(&bus);
        let mut files: Vec<FileTextMatches> = Vec::new();
        let mut summary = None;
        for (_, event) in events {
            match event {
                IndexEvent::TextMatches(batch) => files.extend(batch),
                IndexEvent::TextSearchFinished(s) => summary = Some(s),
                other => panic!("{other:?}"),
            }
        }
        files.sort_by(|a, b| a.relative.cmp(&b.relative));
        let found: Vec<(&str, usize, bool)> = files
            .iter()
            .map(|f| (f.relative.as_str(), f.match_count(), f.unsaved))
            .collect();
        assert_eq!(
            found,
            vec![
                ("src/New.php", 1, true),
                ("src/User.php", 1, false),
                ("web/app.js", 2, true)
            ]
        );
        let summary = summary.unwrap();
        assert_eq!((summary.matches, summary.files_with_matches), (4, 3));
    }

    #[test]
    fn invalid_pattern_is_refused_without_a_task() {
        let (_dir, _bus, service) = setup();
        let mut query = TextQuery::new("(");
        query.regex = true;
        assert!(matches!(
            service.search_text(query, UnsavedFiles::new()),
            Err(crate::error::IndexError::InvalidPattern { .. })
        ));
        let mut query = TextQuery::new("x");
        query.scope = crate::text::SearchScope::Directory(PathBuf::from("Z:\\ailleurs"));
        assert!(service.search_text(query, UnsavedFiles::new()).is_err());
    }

    #[test]
    fn a_new_search_cancels_the_previous_one() {
        let (dir, bus, service) = setup();
        for i in 0..400 {
            write(
                dir.path(),
                &format!("many/f{i}.txt"),
                &"needle haystack\n".repeat(200),
            );
        }
        service.scan().join();
        bus.drain();
        let first = service
            .search_text(TextQuery::new("needle"), UnsavedFiles::new())
            .unwrap();
        let first_id = first.id();
        let second = service
            .search_text(TextQuery::new("class"), UnsavedFiles::new())
            .unwrap();
        let second_id = second.id();
        first.join();
        second.join();
        let mut first_finished = None;
        let mut second_summary = None;
        for event in bus.drain() {
            match event {
                Event::Task(t) if t.id == first_id => {
                    if let TaskState::Finished(outcome) = t.state {
                        first_finished = Some(outcome);
                    }
                }
                Event::Domain(e) if e.task == Some(second_id) => {
                    if let Some(IndexEvent::TextSearchFinished(s)) = e.get::<IndexEvent>() {
                        second_summary = Some(s.clone());
                    }
                }
                _ => {}
            }
        }
        // La première a été annulée (ou avait déjà fini, si la machine est très rapide).
        assert!(matches!(
            first_finished,
            Some(TaskOutcome::Cancelled) | Some(TaskOutcome::Done)
        ));
        assert_eq!(second_summary.unwrap().matches, 2);
    }

    #[test]
    fn update_and_preview() {
        let (dir, bus, service) = setup();
        service.scan().join();
        events_until_ready(&bus);
        write(
            dir.path(),
            "src/Group.php",
            "<?php\n// a\n// b\nclass Group {}\n// c\n",
        );
        service
            .update(vec![dir.path().join("src").join("Group.php")])
            .join();
        let events = index_events(&bus);
        assert!(events
            .iter()
            .any(|(_, e)| matches!(e, IndexEvent::Updated(UpdateSummary { added: 1, .. }))));
        assert!(events
            .iter()
            .any(|(_, e)| matches!(e, IndexEvent::SymbolsUpdated { files: 1 })));
        assert!(service.with_index(|i| i.contains("src/Group.php")));
        assert!(service.with_symbols(|s| s.contains("src/Group.php")));

        let query = TextQuery::new("group");
        service
            .preview(
                dir.path().join("src").join("Group.php"),
                3,
                1,
                Some(query),
                None,
            )
            .unwrap()
            .join();
        let events = index_events(&bus);
        let [(_, IndexEvent::Preview(preview))] = &events[..] else {
            panic!("{events:?}")
        };
        assert_eq!(
            preview.lines.iter().map(|l| l.line).collect::<Vec<_>>(),
            vec![2, 3, 4]
        );
        assert_eq!(preview.lines[1].matches.len(), 1);
    }

    #[test]
    fn find_files_during_scan_is_partial_but_answers() {
        let (dir, bus, service) = setup();
        for i in 0..3000 {
            write(dir.path(), &format!("bulk/d{}/file{i}.php", i % 30), "");
        }
        let scan = service.scan();
        let find = service.find_files("file1", vec![]);
        find.join();
        scan.join();
        wait_symbols(&service);
        let events = index_events(&bus);
        assert!(events
            .iter()
            .any(|(_, e)| matches!(e, IndexEvent::Files(_))));
        assert!(events
            .iter()
            .any(|(_, e)| matches!(e, IndexEvent::ScanFinished(s) if s.files == 3004)));
    }

    #[test]
    fn reconfiguring_rescans_with_the_new_settings_and_keeps_always_excluded() {
        let (dir, bus, service) = setup();
        write(dir.path(), "build/out.php", "<?php\n");
        write(dir.path(), ".juliestorm/index/x.php", "<?php\n");
        write(dir.path(), "sub/.git/config", "");
        service.scan().join();
        wait_symbols(&service);
        assert!(service.with_index(|i| i.contains("build/out.php")));
        assert!(!service.with_index(|i| i.contains(".juliestorm/index/x.php")));
        assert!(!service.with_index(|i| i.contains("sub/.git/config")));
        bus.drain();

        // Mêmes réglages : rien à faire.
        assert!(service.set_settings(IndexSettings::default()).is_none());
        let settings = IndexSettings {
            excluded: vec!["build".into()],
            ..IndexSettings::default()
        };
        service.set_settings(settings.clone()).unwrap().join();
        wait_symbols(&service);
        assert_eq!(service.settings(), settings);
        assert!(!service.with_index(|i| i.contains("build/out.php")));
        assert!(!service.with_symbols(|s| s.contains("build/out.php")));
        // `.git` et `.juliestorm` restent exclus même sans la liste.
        let settings = IndexSettings {
            excluded: vec![],
            ..IndexSettings::default()
        };
        service.set_settings(settings).unwrap().join();
        wait_symbols(&service);
        assert!(service.with_index(|i| i.contains("build/out.php")));
        assert!(!service.with_index(|i| i.contains(".juliestorm/index/x.php")));
        assert!(!service.with_index(|i| i.contains("sub/.git/config")));
    }
}
