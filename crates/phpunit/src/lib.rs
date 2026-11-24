//! Détection, exécution et résultats de PHPUnit. Propriétaire : dev-phpunit.
//!
//! Cette crate ne dépend que de `julie-core` : ni egui, ni autre crate métier.
//! Elle lance `php <phpunit> --configuration <xml> --teamcity`, analyse la sortie
//! **en flux** et publie sur le bus des lots de changements qui remplissent un
//! arbre de résultats pendant que les tests tournent.
//!
//! # Utilisation côté `app`
//!
//! ```ignore
//! // Une fois par projet ouvert :
//! let settings = PhpUnitSettings::from_settings(&settings)?;          // section [phpunit]
//! let phpunit = PhpUnit::new(tasks.clone(), project.clone(), settings);
//! phpunit.detect();                                                   // -> PhpUnitEvent::Detected
//!
//! // Ctrl+Maj+F10 :
//! let handle = phpunit.run(RunRequest::all());                        // rend la main aussitôt
//! let run_id = handle.id();
//!
//! // Ctrl+F2 :
//! phpunit.stop(run_id);
//!
//! // Une fois par image, dans la boucle du bus :
//! Domain::PhpUnit => if let Some(e) = event.get::<julie_phpunit::PhpUnitEvent>() {
//!     match e {
//!         PhpUnitEvent::RunStarted(start) => view = RunView::new(ResultTree::new(project.root()), start),
//!         PhpUnitEvent::Events(events)    => view.tree.apply_all(events),  // l'arbre se remplit
//!         PhpUnitEvent::RunFinished(sum)  => view.summary = Some(sum),
//!         PhpUnitEvent::LaunchFailed(p)   => show_problem(p),              // message du catalogue
//!         PhpUnitEvent::Detected(d)       => self.detection = d,
//!     }
//! }
//! ```
//!
//! # Service ([`PhpUnit`])
//!
//! | Méthode | Clé de tâche | Événements publiés ([`PhpUnitEvent`]) |
//! |---|---|---|
//! | `detect()` | `phpunit.detect` | `Detected(Detection)` |
//! | `run(RunRequest)` | `phpunit.run` | `RunStarted`, des `Events(Vec<TestEvent>)`, `RunFinished(RunSummary)` ; ou `LaunchFailed(Problem)` seul |
//! | `stop(TaskId)` | | l'exécution publie `Aborted(Stopped)` puis `RunFinished`, et sa tâche finit `Cancelled` |
//!
//! - Tous les événements d'une exécution portent le `TaskId` de `run()` : écarter
//!   ceux d'une exécution périmée.
//! - **Lots** : le premier lot part tout de suite, les suivants toutes les 100 ms
//!   (ou à 500 événements), même pendant un test de plusieurs secondes : l'arbre
//!   se remplit en direct sans qu'un événement coûte une image.
//! - `RunFinished` est le dernier événement de toute exécution lancée, quelle que
//!   soit la fin : terminée, [`RunOutcome::Aborted`] (PHP s'est arrêté avant la fin
//!   du rapport : `exit()`, erreur fatale, mémoire), `Stopped`, `TimedOut`
//!   (réglage `timeout_seconds`). Un code de sortie non nul n'est pas un échec en
//!   soi : voir [`RunSummary::is_green`].
//! - `LaunchFailed` : un [`Problem`] (PHP introuvable, chemin de PHP invalide,
//!   PHPUnit introuvable, aucun `phpunit.xml`...) ; aucun test n'a été lancé. La
//!   tâche finit `Failed`.
//! - Version synchrone, sans bus, pour les tests : [`execute`].
//!
//! # Détection ([`detect()`])
//!
//! PHP : réglage `php_path`, sinon `php` du `PATH`, sinon [`DEFAULT_PHP_PATH`].
//! PHPUnit : réglage `phpunit_path`, sinon `vendor/bin/phpunit`, le script du paquet,
//! un phar. Configuration : réglage `configuration`, sinon `phpunit.xml`,
//! `phpunit.xml.dist`. [`Detection`] garde chaque résultat séparément
//! (`Result<_, Problem>`) pour montrer ce qui va et ce qui manque ;
//! [`Detection::plan`] donne un [`LaunchPlan`] ou le premier problème. Les
//! versions (`php --version`, `phpunit --version`) ne sont sondées que sur
//! demande (deux processus) : `PhpUnit::detect()` le fait, l'exécution non.
//!
//! # Arbre de résultats ([`ResultTree`])
//!
//! Racine -> suite -> classe -> méthode -> jeu de données ([`NodeKind`]), dans un
//! tableau ; [`NodeId`] est stable. L'interface garde son propre arbre et lui
//! applique les mêmes événements que le service : `tree.apply(&TestEvent)` ou
//! `tree.apply_all(&[..])`. `tree.version()` invalide un cache d'affichage.
//!
//! Par nœud ([`TestNode`]) : `state` ([`TestState`] : en attente, en cours, réussi,
//! ignoré, incomplet, risqué, interrompu, échoué, erreur ; un conteneur prend le
//! plus grave de ses enfants une fois fini, et `worst` donne le plus grave vu
//! jusque-là), `duration` (somme pour un conteneur), `console` (la sortie non
//! reconnue arrivée pendant ce nœud ; celle de la racine est **toute** la sortie),
//! `diagnostic` ([`Diagnostic`] : message, trace en lignes dont certaines ont un
//! [`julie_core::Location`], attendu/obtenu), `source` ([`TestSource`] : fichier,
//! classe, méthode d'après le `locationHint`) et `jump_location()`.
//!
//! `tree.counts()` ([`Counts`]) : total annoncé (`None` tant qu'inconnu : « sur ? »),
//! commencés, en cours, réussis, échoués, erreurs, ignorés, incomplets, risqués,
//! interrompus. `tree.command()` : la ligne de commande, à écrire en tête de la
//! console. `tree.reported_summary()` : le résumé écrit par PHPUnit (témoin).
//!
//! # Liens et diff
//!
//! - Traces : `diagnostic.trace[i].location` (ligne PHP 1-based convertie en
//!   [`julie_core::TextPosition`] 0-based).
//! - Console : [`find_links`] repère les `chemin:ligne` qui désignent un fichier
//!   existant (le prédicat `exists` est à l'appelant) et rend les plages à
//!   souligner.
//! - Attendu/obtenu : `diagnostic.comparison` ([`Comparison`]) ;
//!   `expected_text()` / `actual_text()` rendent le texte pour un diff
//!   (`julie_diff::diff_lines`), avec les apostrophes d'export retirées pour une
//!   chaîne.
//!
//! # Analyseur ([`TeamCityParser`])
//!
//! Pur : une ligne en entrée, des [`TestEvent`] en sortie. Échappements `|n`, `|'`,
//! `||`, `|[`, `|]`, `|0xXXXX`. La sortie non reconnue (avertissements PHP, `echo`,
//! messages inconnus ou tronqués) devient `TestEvent::Output`, jamais perdue.
//!
//! # Prévu pour la v1, pas livré
//!
//! Découverte des tests (icônes de gouttière), configurations nommées, relance des
//! échecs ou d'un nœud, historique. La place est faite : [`RunScope`] et
//! [`RunRequest`] (`extra_args`, `env`, `working_dir`) savent déjà lancer une suite
//! ou un filtre ; relancer un nœud, c'est un `RunScope::Filter` construit depuis
//! `TestNode::source`.
//!
//! # Ce qui est vérifié contre quoi
//!
//! Le format a été enregistré avec PHPUnit **9.6.34** (`tests/fixtures/phpunit/
//! recordings/`). Les variantes de PHPUnit 10, 11 et 12 sont écrites de mémoire
//! (`unverified-*`) : l'analyseur est tolérant, mais ces formats n'ont pas été
//! confrontés à un vrai PHPUnit.

pub mod detect;
pub mod error;
pub mod event;
pub mod model;
pub mod refine;
pub mod run;
pub mod service;
pub mod settings;
pub mod teamcity;
pub mod trace;

pub use detect::{
    detect, locate_configuration, locate_php, locate_phpunit, parse_php_version,
    parse_phpunit_version, Detection, PhpInfo, PhpSource, PhpUnitInfo, PhpUnitKind, PhpUnitSource,
    PhpUnitVersion, DEFAULT_PHP_PATH,
};
pub use error::{PhpUnitError, Problem};
pub use event::{AbortReason, Comparison, ReportedSummary, TestEvent};
pub use model::{
    parse_location_hint, ConsoleLine, Counts, Diagnostic, DiagnosticCause, NodeId, NodeKind,
    ResultTree, TestNode, TestSource, TestState,
};
pub use refine::{refine, SourceLines};
pub use run::{
    arguments, command, command_line, LaunchPlan, RunOutcome, RunRequest, RunScope, RunSummary,
};
pub use service::{execute, PhpUnit, PhpUnitEvent, RunStart};
pub use settings::PhpUnitSettings;
pub use teamcity::{classify_failure, TeamCityParser};
pub use trace::{find_links, parse_trace, unexport, TextLink, TraceEntry};
