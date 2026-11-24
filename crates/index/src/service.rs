//! Le service asynchrone : chaque méthode lance une tâche `Domain::Index` et
//! rend la main tout de suite ; les résultats arrivent en [`IndexEvent`].

use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::time::{Duration, Instant};

use julie_core::{Domain, Project, TaskContext, TaskHandle, TaskId, Tasks};

use crate::error::Result;
use crate::files::{collect_update, walk, FileIndex, UpdateSummary};
use crate::fuzzy::{FileResults, DEFAULT_LIMIT};
use crate::settings::IndexSettings;
use crate::text::{
    files_to_search, preview_file, search_files, FileMask, FileTextMatches, Matcher, Preview,
    SearchOptions, TextQuery, TextSearchSummary, UnsavedFiles,
};

/// Ce que publie `julie-index` (sous `Domain::Index`). Le `task` du
/// `DomainEvent` dit de quelle demande vient le résultat : l'interface ignore
/// ceux d'une demande qui n'est plus la courante.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexEvent {
    /// Indexation initiale (ou complète) terminée.
    ScanFinished(ScanSummary),
    /// Mise à jour incrémentale appliquée.
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

struct Shared {
    index: RwLock<FileIndex>,
    phase: Mutex<Phase>,
    phase_changed: Condvar,
    /// Sérialise les mises à jour incrémentales entre elles.
    updating: Mutex<()>,
}

impl Shared {
    fn read(&self) -> RwLockReadGuard<'_, FileIndex> {
        self.index.read().unwrap_or_else(|p| p.into_inner())
    }

    fn write(&self) -> RwLockWriteGuard<'_, FileIndex> {
        self.index.write().unwrap_or_else(|p| p.into_inner())
    }

    fn phase(&self) -> MutexGuard<'_, Phase> {
        self.phase.lock().unwrap_or_else(|p| p.into_inner())
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
}

/// Le service d'index et de recherche d'un projet. Construit par `app` à
/// l'ouverture du projet ; ses méthodes s'appellent depuis le fil de
/// l'interface (aucune n'y fait d'accès disque).
pub struct IndexService {
    tasks: Tasks,
    project: Project,
    settings: IndexSettings,
    shared: Arc<Shared>,
    find_task: Mutex<Option<TaskId>>,
    search_task: Mutex<Option<TaskId>>,
    preview_task: Mutex<Option<TaskId>>,
}

impl IndexService {
    pub fn new(tasks: Tasks, project: Project, settings: IndexSettings) -> Self {
        let index = FileIndex::new(project.root().to_path_buf(), settings.clone());
        Self {
            tasks,
            project,
            settings,
            shared: Arc::new(Shared {
                index: RwLock::new(index),
                phase: Mutex::new(Phase::NotScanned),
                phase_changed: Condvar::new(),
                updating: Mutex::new(()),
            }),
            find_task: Mutex::new(None),
            search_task: Mutex::new(None),
            preview_task: Mutex::new(None),
        }
    }

    pub fn project(&self) -> &Project {
        &self.project
    }

    /// L'indexation initiale est finie (sinon « Indexation… »).
    pub fn is_ready(&self) -> bool {
        matches!(*self.shared.phase(), Phase::Ready)
    }

    /// Indexation complète (`index.scan`). Vide l'index et le remplit au fil du
    /// parcours : « aller au fichier » rend des résultats partiels pendant ce
    /// temps (`FileResults::index_complete == false`). Progression : événements
    /// de tâche `Progress { done: fichiers trouvés, total: 0 }`. À la fin :
    /// `IndexEvent::ScanFinished`. Une indexation en cours est annulée.
    pub fn scan(&self) -> TaskHandle {
        let shared = Arc::clone(&self.shared);
        let root = self.project.root().to_path_buf();
        let settings = self.settings.clone();
        // Le verrou de phase est tenu pendant `spawn` : la tâche ne peut pas
        // finir avant que sa phase soit posée.
        let mut phase = self.shared.phase();
        if let Phase::Scanning(previous) = *phase {
            self.tasks.cancel(previous);
        }
        let handle = self.tasks.spawn(Domain::Index, "index.scan", move |ctx| {
            let started = Instant::now();
            let generation = {
                let mut index = shared.write();
                index.clear();
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
            result?;
            let index = shared.read();
            let summary = ScanSummary {
                files: index.len(),
                library_files: index.entries().iter().filter(|e| e.is_library()).count(),
                elapsed: started.elapsed(),
            };
            drop(index);
            ctx.publish(IndexEvent::ScanFinished(summary));
            Ok(())
        });
        *phase = Phase::Scanning(handle.id());
        drop(phase);
        self.shared.phase_changed.notify_all();
        handle
    }

    /// Mise à jour incrémentale (`index.update`) pour des chemins absolus changés
    /// sur le disque (le contenu de `Event::FilesChangedOnDisk`). Attend la fin
    /// d'une indexation en cours. Publie `IndexEvent::Updated`. Un `.gitignore`
    /// changé fait revoir son dossier ; celui de la racine, tout le projet.
    pub fn update(&self, paths: Vec<PathBuf>) -> TaskHandle {
        let shared = Arc::clone(&self.shared);
        let root = self.project.root().to_path_buf();
        let settings = self.settings.clone();
        self.tasks.spawn(Domain::Index, "index.update", move |ctx| {
            shared.wait_scanned(ctx)?;
            let _serial = shared.updating.lock().unwrap_or_else(|p| p.into_inner());
            let plan = shared.read().plan_update(&paths);
            let summary = if plan.full_rescan {
                let fresh = FileIndex::scan(root, settings, ctx.cancel_token())?;
                shared.write().replace_with(fresh)
            } else {
                let walked = collect_update(&root, &settings, &plan, ctx.cancel_token())?;
                ctx.check_cancelled()?;
                shared.write().apply_update(&plan, walked)
            };
            ctx.publish(IndexEvent::Updated(summary));
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
        let options = SearchOptions::new(&query, &self.settings);
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

    /// Donne accès à l'index courant. **Peut bloquer** quelques millisecondes
    /// pendant une écriture : pour les tests et les tâches, pas pour le fil de
    /// l'interface.
    pub fn with_index<R>(&self, f: impl FnOnce(&FileIndex) -> R) -> R {
        f(&self.shared.read())
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|p| p.into_inner())
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
        let service = IndexService::new(tasks, project, IndexSettings::default());
        (dir, bus, service)
    }

    #[test]
    fn scan_then_find_then_search() {
        let (dir, bus, service) = setup();
        assert!(!service.is_ready());
        service.scan().join();
        assert!(service.is_ready());
        let events = index_events(&bus);
        assert!(
            matches!(
                &events[..],
                [(_, IndexEvent::ScanFinished(ScanSummary { files: 4, .. }))]
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
        assert!(service.with_index(|i| i.contains("src/Group.php")));

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
        let events = index_events(&bus);
        assert!(events
            .iter()
            .any(|(_, e)| matches!(e, IndexEvent::Files(_))));
        assert!(events
            .iter()
            .any(|(_, e)| matches!(e, IndexEvent::ScanFinished(s) if s.files == 3004)));
    }
}
