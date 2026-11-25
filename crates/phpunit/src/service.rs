//! Le service : lance la détection et les exécutions en tâches de fond, publie sur
//! le bus.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use julie_core::{CancelToken, Domain, Project, TaskHandle, TaskId, Tasks};

use crate::detect::{detect, Detection};
use crate::error::{PhpUnitError, Problem};
use crate::event::{AbortReason, TestEvent};
use crate::model::ResultTree;
use crate::refine::{refine, SourceLines};
use crate::run::{self, LaunchPlan, RunOutcome, RunRequest, RunScope, RunSummary};
use crate::settings::PhpUnitSettings;
use crate::teamcity::TeamCityParser;

/// Intervalle au bout duquel un lot d'événements part, même incomplet : l'arbre
/// de l'interface se met à jour au moins tous les 200 ms.
const BATCH_INTERVAL: Duration = Duration::from_millis(100);
/// Taille à partir de laquelle un lot part sans attendre.
const BATCH_MAX: usize = 500;
/// Période du fil de surveillance (annulation, délai, lots en retard).
const SUPERVISE_PERIOD: Duration = Duration::from_millis(20);

/// Ce que la crate publie sur le bus (sous [`Domain::PhpUnit`]). L'identifiant de
/// tâche de l'événement est celui de [`PhpUnit::run`] ou [`PhpUnit::detect`] : il
/// sert à écarter les événements d'une exécution périmée.
#[derive(Debug, Clone)]
pub enum PhpUnitEvent {
    /// Résultat de [`PhpUnit::detect`].
    Detected(Box<Detection>),
    /// L'exécution ne peut pas démarrer (ou PHP ne s'est pas lancé). Rien d'autre
    /// ne suit : pas de `RunFinished`.
    LaunchFailed(Problem),
    /// L'exécution démarre : repartir d'un arbre neuf
    /// ([`ResultTree::new`]).
    RunStarted(Box<RunStart>),
    /// Un lot de changements de l'arbre, à appliquer dans l'ordre
    /// ([`ResultTree::apply_all`]). Le premier lot contient [`TestEvent::Command`].
    Events(Vec<TestEvent>),
    /// Fin de l'exécution, quelle qu'en soit la cause : toujours le dernier
    /// événement d'une exécution lancée.
    RunFinished(Box<RunSummary>),
}

/// Début d'exécution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunStart {
    /// La commande, telle qu'elle apparaît en tête de la console.
    pub command_line: String,
    pub plan: LaunchPlan,
    pub scope: RunScope,
}

/// Le service PHPUnit d'un projet. Construit par `app` ; chaque méthode rend la
/// main tout de suite.
#[derive(Clone)]
pub struct PhpUnit {
    tasks: Tasks,
    project: Project,
    settings: PhpUnitSettings,
}

impl PhpUnit {
    pub fn new(tasks: Tasks, project: Project, settings: PhpUnitSettings) -> Self {
        Self {
            tasks,
            project,
            settings,
        }
    }

    pub fn settings(&self) -> &PhpUnitSettings {
        &self.settings
    }

    /// Détecte PHP, PHPUnit (versions comprises) et la configuration.
    /// Tâche `phpunit.detect` ; publie [`PhpUnitEvent::Detected`].
    pub fn detect(&self) -> TaskHandle {
        let root = self.project.root().to_path_buf();
        let settings = self.settings.clone();
        self.tasks
            .spawn(Domain::PhpUnit, "phpunit.detect", move |ctx| {
                let detection = detect(&root, &settings, true, ctx.cancel_token())?;
                ctx.publish(PhpUnitEvent::Detected(Box::new(detection)));
                Ok(())
            })
    }

    /// Lance une exécution. Tâche `phpunit.run` ; publie `RunStarted`, des lots
    /// `Events`, puis `RunFinished` (ou `LaunchFailed`). Un arrêt demandé par
    /// [`PhpUnit::stop`] finit la tâche en `Cancelled`, après `RunFinished`.
    pub fn run(&self, request: RunRequest) -> TaskHandle {
        let project = self.project.clone();
        let settings = self.settings.clone();
        self.tasks
            .spawn(Domain::PhpUnit, "phpunit.run", move |ctx| {
                let publish = |event: PhpUnitEvent| ctx.publish(event);
                match execute(&project, &settings, &request, ctx.cancel_token(), &publish) {
                    Ok(summary) if summary.outcome == RunOutcome::Stopped => {
                        Err(julie_core::Error::Cancelled.into())
                    }
                    Ok(_) => Ok(()),
                    Err(PhpUnitError::Launch(problem)) => {
                        ctx.publish(PhpUnitEvent::LaunchFailed(problem.clone()));
                        Err(PhpUnitError::Launch(problem).into())
                    }
                    Err(error) => Err(error.into()),
                }
            })
    }

    /// Sonde un réglage (chemin de PHP, de PHPUnit, fichier de configuration) en
    /// tâche de fond ; publie un [`crate::ProbeEvent`]. Voir [`crate::probe`].
    pub fn probe(&self, request: crate::probe::ProbeRequest) -> TaskHandle {
        crate::probe::spawn(&self.tasks, self.project.root().to_path_buf(), request)
    }

    /// Arrête une exécution : PHP et ses descendants sont tués. Sans effet si la
    /// tâche est déjà finie.
    pub fn stop(&self, run: TaskId) {
        self.tasks.cancel(run);
    }
}

/// L'exécution elle-même, **bloquante**, sans bus : c'est elle que les tests
/// appellent. `publish` reçoit les événements dans l'ordre ; il peut être appelé
/// depuis deux fils (le fil de surveillance expédie les lots en retard), jamais en
/// même temps.
///
/// Rend `Err(Launch(..))` si l'on ne peut pas lancer (aucun événement n'a été
/// publié), `Err(Core(Cancelled))` si l'annulation précède le lancement.
pub fn execute(
    project: &Project,
    settings: &PhpUnitSettings,
    request: &RunRequest,
    cancel: &CancelToken,
    publish: &(dyn Fn(PhpUnitEvent) + Sync),
) -> Result<RunSummary, PhpUnitError> {
    let root = project.root();
    let detection = detect(root, settings, false, cancel)?;
    let plan = detection.plan()?;
    // Options et variables du réglage d'abord, celles de la demande ensuite.
    let effective = request.with_settings(settings);
    let request = &effective;
    let command = run::command(&plan, request);
    let command_line = run::command_line(&plan, request);
    let timeout = request.timeout.or_else(|| settings.timeout());

    publish(PhpUnitEvent::RunStarted(Box::new(RunStart {
        command_line: command_line.clone(),
        plan: plan.clone(),
        scope: request.scope.clone(),
    })));

    let started = Instant::now();
    let batcher = Batcher::new(publish);
    let mut pipeline = Pipeline {
        root,
        tree: ResultTree::new(root),
        parser: TeamCityParser::new(),
        sources: SourceLines::default(),
        batcher: &batcher,
    };
    pipeline.ingest(vec![TestEvent::Command(command_line)]);

    let inner = CancelToken::new();
    let done = AtomicBool::new(false);
    let timed_out = AtomicBool::new(false);
    let deadline = timeout.map(|limit| started + limit);

    let result = std::thread::scope(|scope| {
        scope.spawn(|| {
            while !done.load(Ordering::SeqCst) {
                if cancel.is_cancelled() {
                    inner.cancel();
                } else if deadline.is_some_and(|at| Instant::now() >= at) {
                    timed_out.store(true, Ordering::SeqCst);
                    inner.cancel();
                }
                batcher.flush_if_due();
                std::thread::sleep(SUPERVISE_PERIOD);
            }
        });
        // Même si l'analyse panique, le fil de surveillance doit pouvoir s'arrêter.
        let _stop = StopOnDrop(&done);
        command.run_lines(&inner, |stream, bytes| {
            let line = String::from_utf8_lossy(bytes);
            let events = pipeline.parser.feed(stream, &line);
            pipeline.ingest(events);
        })
    });

    let (outcome, exit_code) = match result {
        Ok(status) => {
            let events = pipeline.parser.finish();
            pipeline.ingest(events);
            let exit_code = status.code;
            let crashed = pipeline.tree.has_open_work()
                || (!pipeline.tree.saw_report() && exit_code != Some(0));
            if crashed {
                pipeline.ingest(vec![TestEvent::Aborted(AbortReason::ProcessExited {
                    exit_code,
                })]);
                (RunOutcome::Aborted { exit_code }, exit_code)
            } else {
                (RunOutcome::Completed, exit_code)
            }
        }
        Err(error) if error.is_cancelled() => {
            let events = pipeline.parser.finish();
            pipeline.ingest(events);
            let (reason, outcome) = if timed_out.load(Ordering::SeqCst) {
                (AbortReason::TimedOut, RunOutcome::TimedOut)
            } else {
                (AbortReason::Stopped, RunOutcome::Stopped)
            };
            pipeline.ingest(vec![TestEvent::Aborted(reason)]);
            (outcome, None)
        }
        Err(julie_core::Error::ProcessSpawn { source, .. }) => {
            return Err(Problem::PhpNotRunnable {
                path: plan.php,
                reason: source.to_string(),
            }
            .into());
        }
        Err(error) => return Err(error.into()),
    };
    pipeline.ingest(vec![TestEvent::Ended]);
    batcher.flush();

    let summary = RunSummary {
        outcome,
        exit_code,
        counts: pipeline.tree.counts().clone(),
        duration: started.elapsed(),
        reported: pipeline.tree.reported_summary().cloned(),
    };
    publish(PhpUnitEvent::RunFinished(Box::new(summary.clone())));
    Ok(summary)
}

struct StopOnDrop<'a>(&'a AtomicBool);

impl Drop for StopOnDrop<'_> {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

/// Analyse -> correction -> arbre du service -> lot publié.
struct Pipeline<'a> {
    root: &'a std::path::Path,
    tree: ResultTree,
    parser: TeamCityParser,
    sources: SourceLines,
    batcher: &'a Batcher<'a>,
}

impl Pipeline<'_> {
    fn ingest(&mut self, events: Vec<TestEvent>) {
        for mut event in events {
            refine(&mut event, self.root, &mut self.sources);
            self.tree.apply(&event);
            self.batcher.push(event);
        }
    }
}

/// Regroupe les événements en lots : le premier part tout de suite, les suivants
/// toutes les 100 ms ou à 500 événements ; un fil de surveillance expédie ce qui
/// resterait en attente pendant un long test.
struct Batcher<'a> {
    publish: &'a (dyn Fn(PhpUnitEvent) + Sync),
    state: Mutex<BatchState>,
}

struct BatchState {
    events: Vec<TestEvent>,
    last_flush: Instant,
}

impl<'a> Batcher<'a> {
    fn new(publish: &'a (dyn Fn(PhpUnitEvent) + Sync)) -> Self {
        Self {
            publish,
            state: Mutex::new(BatchState {
                events: Vec::new(),
                last_flush: Instant::now()
                    .checked_sub(BATCH_INTERVAL)
                    .unwrap_or_else(Instant::now),
            }),
        }
    }

    fn lock(&self) -> MutexGuard<'_, BatchState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn push(&self, event: TestEvent) {
        let mut state = self.lock();
        state.events.push(event);
        if state.events.len() >= BATCH_MAX || state.last_flush.elapsed() >= BATCH_INTERVAL {
            self.flush_locked(&mut state);
        }
    }

    fn flush_if_due(&self) {
        let mut state = self.lock();
        if !state.events.is_empty() && state.last_flush.elapsed() >= BATCH_INTERVAL {
            self.flush_locked(&mut state);
        }
    }

    fn flush(&self) {
        let mut state = self.lock();
        self.flush_locked(&mut state);
    }

    /// Publie sous le verrou : l'ordre des lots est celui des événements.
    fn flush_locked(&self, state: &mut BatchState) {
        if !state.events.is_empty() {
            (self.publish)(PhpUnitEvent::Events(std::mem::take(&mut state.events)));
        }
        state.last_flush = Instant::now();
    }
}
