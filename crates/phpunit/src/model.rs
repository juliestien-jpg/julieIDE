//! Modèle de résultats : l'arbre suite -> classe -> méthode -> jeu de données.
//!
//! [`ResultTree`] se remplit en appliquant des [`TestEvent`] dans l'ordre. Le
//! service en tient un pour décider de la fin de l'exécution ; l'interface en tient
//! un autre, nourri par les mêmes événements : les deux sont identiques (mêmes
//! [`NodeId`]), sans que l'un ait à copier l'autre.
//!
//! Un nœud n'est créé que lorsque PHPUnit l'annonce : les tests jamais commencés ne
//! sont pas inventés.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::Duration;

use julie_core::process::Stream;
use julie_core::{Location, TextPosition};

use crate::detect::PhpUnitVersion;
use crate::event::{AbortReason, Comparison, ReportedSummary, TestEvent};
use crate::trace::{parse_trace, TraceEntry};

/// Au-delà, la console d'un nœud est tronquée (un test qui boucle sur `echo`).
const MAX_CONSOLE_BYTES: usize = 1 << 20;
/// Lignes d'erreur standard gardées pour expliquer un arrêt brutal.
const STDERR_TAIL_LINES: usize = 40;

/// Identifiant d'un nœud : indice dans l'arbre, stable pour toute son existence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(u32);

impl NodeId {
    /// La racine de tout arbre.
    pub const ROOT: NodeId = NodeId(0);

    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// État d'un nœud. Pour un conteneur : `Running` jusqu'à sa fin, puis l'état le
/// plus grave de ses enfants (voir [`TestState::severity`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TestState {
    /// Annoncé, pas encore commencé (réservé : relance et découverte, v1).
    Pending,
    Running,
    Passed,
    Skipped,
    Incomplete,
    Risky,
    /// Arrêté avant la fin (arrêt demandé, délai dépassé).
    Interrupted,
    Failed,
    Error,
}

impl TestState {
    /// Gravité croissante : `Passed` < `Skipped` < `Incomplete` < `Risky` <
    /// `Interrupted` < `Failed` < `Error`. `Pending` et `Running` valent 0 : ils
    /// ne sont pas des résultats.
    pub fn severity(self) -> u8 {
        match self {
            TestState::Pending | TestState::Running => 0,
            TestState::Passed => 1,
            TestState::Skipped => 2,
            TestState::Incomplete => 3,
            TestState::Risky => 4,
            TestState::Interrupted => 5,
            TestState::Failed => 6,
            TestState::Error => 7,
        }
    }

    /// Échec ou erreur : ce qui rougit la barre de progression.
    pub fn is_failure(self) -> bool {
        matches!(self, TestState::Failed | TestState::Error)
    }

    /// Un résultat est connu (ni en attente ni en cours).
    pub fn is_final(self) -> bool {
        !matches!(self, TestState::Pending | TestState::Running)
    }
}

/// Place d'un nœud dans la hiérarchie.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeKind {
    Root,
    /// Suite de `phpunit.xml` (`main`) ou dossier.
    Suite,
    Class,
    /// Une méthode de test. Quand elle a un fournisseur de données, elle contient
    /// des [`NodeKind::DataSet`].
    Method,
    DataSet,
}

/// Compteurs en direct, tenus à jour à chaque événement.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Counts {
    /// Nombre de tests annoncé par PHPUnit (`testCount`), inconnu avant.
    pub total: Option<usize>,
    /// Tests commencés (feuilles : méthodes sans jeux de données, et jeux).
    pub started: usize,
    pub running: usize,
    pub passed: usize,
    pub failed: usize,
    pub errors: usize,
    pub skipped: usize,
    pub incomplete: usize,
    pub risky: usize,
    pub interrupted: usize,
}

impl Counts {
    /// Tests dont le résultat est connu.
    pub fn finished(&self) -> usize {
        self.passed
            + self.failed
            + self.errors
            + self.skipped
            + self.incomplete
            + self.risky
            + self.interrupted
    }

    /// Ignorés au sens large : sautés ou incomplets.
    pub fn ignored(&self) -> usize {
        self.skipped + self.incomplete
    }

    /// Échecs et erreurs.
    pub fn failures(&self) -> usize {
        self.failed + self.errors
    }

    fn bump(&mut self, state: TestState, up: bool) {
        let slot = match state {
            TestState::Pending => return,
            TestState::Running => &mut self.running,
            TestState::Passed => &mut self.passed,
            TestState::Skipped => &mut self.skipped,
            TestState::Incomplete => &mut self.incomplete,
            TestState::Risky => &mut self.risky,
            TestState::Interrupted => &mut self.interrupted,
            TestState::Failed => &mut self.failed,
            TestState::Error => &mut self.errors,
        };
        if up {
            *slot += 1;
        } else {
            *slot = slot.saturating_sub(1);
        }
    }
}

/// D'où vient un test, d'après le `locationHint` de PHPUnit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestSource {
    pub file: PathBuf,
    /// `Fixture\Tests\CalculatorTest` (sans antislash initial).
    pub class: Option<String>,
    /// Nom de la méthode, sans le suffixe de jeu de données.
    pub method: Option<String>,
}

/// Une ligne de sortie rattachée à un nœud.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsoleLine {
    pub stream: Stream,
    pub text: String,
}

/// Pourquoi un nœud est en échec (ou ignoré).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// Message de PHPUnit ; pour un test emporté par un arrêt brutal, la fin de
    /// la sortie d'erreur de PHP (peut être vide).
    pub message: String,
    /// Trace ligne à ligne ; les lignes qui désignent un fichier ont un
    /// [`Location`].
    pub trace: Vec<TraceEntry>,
    /// Attendu et obtenu, quand PHPUnit les donne.
    pub comparison: Option<Comparison>,
    pub cause: DiagnosticCause,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticCause {
    /// Rapporté par PHPUnit.
    Reported,
    /// Le processus PHP s'est arrêté pendant ce test.
    ProcessExited { exit_code: Option<i32> },
}

/// Un nœud de l'arbre.
#[derive(Debug, Clone)]
pub struct TestNode {
    pub id: NodeId,
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
    pub kind: NodeKind,
    /// Le nom tel que PHPUnit le donne.
    pub name: String,
    /// Le nom à afficher : classe sans espace de noms, jeu de données sans
    /// préfixe.
    pub label: String,
    pub source: Option<TestSource>,
    /// `true` pour une feuille annoncée par `testStarted` (celle qu'on compte).
    pub is_test: bool,
    pub state: TestState,
    /// Le plus grave des résultats des descendants vus jusqu'ici (`Pending` s'il
    /// n'y en a aucun) : de quoi colorer un conteneur encore en cours.
    pub worst: TestState,
    /// Conteneur : sa fin a été annoncée (ou l'exécution s'est arrêtée).
    pub finished: bool,
    /// Feuille : durée du test. Conteneur : somme des durées de ses tests.
    pub duration: Option<Duration>,
    /// Sortie non reconnue rattachée à ce nœud (racine : toute la sortie).
    pub console: Vec<ConsoleLine>,
    pub console_truncated: bool,
    pub diagnostic: Option<Diagnostic>,
    console_bytes: usize,
}

impl TestNode {
    /// Où ouvrir ce test : son fichier, au début faute de ligne dans le
    /// `locationHint` (à affiner avec l'index des symboles : `source.method`).
    pub fn jump_location(&self) -> Option<Location> {
        self.source.as_ref().map(|source| Location {
            path: source.file.clone(),
            position: TextPosition::new(0, 0),
        })
    }

    /// Tout le texte de la console, une ligne par ligne, avec saut de ligne final.
    pub fn console_text(&self) -> String {
        let mut text = String::with_capacity(self.console_bytes + self.console.len());
        for line in &self.console {
            text.push_str(&line.text);
            text.push('\n');
        }
        text
    }
}

/// L'arbre de résultats d'une exécution.
#[derive(Debug, Clone)]
pub struct ResultTree {
    project_root: PathBuf,
    nodes: Vec<TestNode>,
    /// Suites ouvertes, de la plus externe à la plus interne (racine exclue).
    open: Vec<NodeId>,
    /// Tests commencés et pas finis, avec le nom de leur `testStarted`.
    running: Vec<(String, NodeId)>,
    current_test: Option<NodeId>,
    counts: Counts,
    command: Option<String>,
    phpunit_version: Option<PhpUnitVersion>,
    reported: Option<ReportedSummary>,
    aborted: Option<AbortReason>,
    stderr_tail: VecDeque<String>,
    saw_report: bool,
    ended: bool,
    version: u64,
}

impl ResultTree {
    /// Un arbre vide. `project_root` sert à lire les chemins relatifs des traces.
    pub fn new(project_root: impl Into<PathBuf>) -> Self {
        let mut root = new_node(NodeId::ROOT, None, NodeKind::Root, String::new(), None);
        root.state = TestState::Running;
        Self {
            project_root: project_root.into(),
            nodes: vec![root],
            open: Vec::new(),
            running: Vec::new(),
            current_test: None,
            counts: Counts::default(),
            command: None,
            phpunit_version: None,
            reported: None,
            aborted: None,
            stderr_tail: VecDeque::new(),
            saw_report: false,
            ended: false,
            version: 0,
        }
    }

    pub fn root(&self) -> NodeId {
        NodeId::ROOT
    }

    pub fn node(&self, id: NodeId) -> &TestNode {
        &self.nodes[id.index()]
    }

    pub fn children(&self, id: NodeId) -> &[NodeId] {
        &self.nodes[id.index()].children
    }

    /// Nombre de nœuds, racine comprise.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Vrai tant qu'aucun nœud n'est arrivé (la racine seule).
    pub fn is_empty(&self) -> bool {
        self.nodes.len() == 1
    }

    /// Tous les nœuds, dans l'ordre d'arrivée (racine d'abord).
    pub fn iter(&self) -> impl Iterator<Item = &TestNode> {
        self.nodes.iter()
    }

    /// Le chemin de la racine au nœud, nœud compris.
    pub fn path_of(&self, id: NodeId) -> Vec<NodeId> {
        let mut path = vec![id];
        let mut current = self.node(id).parent;
        while let Some(parent) = current {
            path.push(parent);
            current = self.node(parent).parent;
        }
        path.reverse();
        path
    }

    pub fn counts(&self) -> &Counts {
        &self.counts
    }

    /// La ligne de commande lancée (en tête de la console de la racine).
    pub fn command(&self) -> Option<&str> {
        self.command.as_deref()
    }

    /// La version de PHPUnit lue sur la première ligne de sa sortie.
    pub fn phpunit_version(&self) -> Option<&PhpUnitVersion> {
        self.phpunit_version.as_ref()
    }

    /// Le résumé que PHPUnit a écrit lui-même, s'il est arrivé.
    pub fn reported_summary(&self) -> Option<&ReportedSummary> {
        self.reported.as_ref()
    }

    pub fn aborted(&self) -> Option<AbortReason> {
        self.aborted
    }

    /// `Ended` a été appliqué : plus rien ne changera.
    pub fn is_ended(&self) -> bool {
        self.ended
    }

    /// Au moins un message TeamCity de structure est arrivé.
    pub fn saw_report(&self) -> bool {
        self.saw_report
    }

    /// Une suite ou un test est ouvert : la sortie s'est arrêtée au milieu du
    /// rapport.
    pub fn has_open_work(&self) -> bool {
        !self.open.is_empty() || !self.running.is_empty()
    }

    /// Le test en cours, s'il y en a un.
    pub fn current_test(&self) -> Option<NodeId> {
        self.current_test
    }

    /// Augmente à chaque événement appliqué : de quoi invalider un cache d'affichage.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Applique plusieurs événements, dans l'ordre.
    pub fn apply_all(&mut self, events: &[TestEvent]) {
        for event in events {
            self.apply(event);
        }
    }

    /// Applique un événement.
    pub fn apply(&mut self, event: &TestEvent) {
        self.version += 1;
        match event {
            TestEvent::Command(command) => self.command = Some(command.clone()),
            TestEvent::Version(version) => self.phpunit_version = Some(version.clone()),
            TestEvent::TestCount(count) => {
                self.saw_report = true;
                self.counts.total = Some(*count);
            }
            TestEvent::SuiteStarted {
                name,
                location_hint,
            } => {
                self.saw_report = true;
                self.start_suite(name, location_hint.as_deref());
            }
            TestEvent::SuiteFinished { name } => self.finish_suite(name),
            TestEvent::TestStarted {
                name,
                location_hint,
            } => {
                self.saw_report = true;
                self.start_test(name, location_hint.as_deref());
            }
            TestEvent::TestFinished { name, duration } => self.finish_test(name, *duration),
            TestEvent::TestFailed {
                name,
                state,
                message,
                details,
                duration,
                comparison,
            } => {
                self.saw_report = true;
                self.report(
                    name,
                    *state,
                    message,
                    details,
                    *duration,
                    comparison.clone(),
                );
            }
            TestEvent::TestIgnored {
                name,
                state,
                message,
                details,
                duration,
            } => {
                self.saw_report = true;
                self.report(name, *state, message, details, *duration, None);
            }
            TestEvent::Output { stream, text } => self.output(*stream, text),
            TestEvent::Summary(summary) => self.reported = Some(summary.clone()),
            TestEvent::Aborted(reason) => self.abort(*reason),
            TestEvent::Ended => self.end(),
        }
    }

    // --- structure ---------------------------------------------------------

    fn push_node(&mut self, mut node: TestNode) -> NodeId {
        let id = NodeId(self.nodes.len() as u32);
        node.id = id;
        if let Some(parent) = node.parent {
            self.nodes[parent.index()].children.push(id);
        }
        self.nodes.push(node);
        id
    }

    fn container(&self) -> NodeId {
        self.open.last().copied().unwrap_or(NodeId::ROOT)
    }

    fn source_of(&self, hint: Option<&str>) -> Option<TestSource> {
        hint.and_then(|hint| parse_location_hint(hint, &self.project_root))
    }

    fn start_suite(&mut self, name: &str, hint: Option<&str>) {
        let source = self.source_of(hint);
        let kind = match &source {
            Some(source) if source.method.is_some() => NodeKind::Method,
            Some(source) if source.class.is_some() => NodeKind::Class,
            _ if name.contains("::") => NodeKind::Method,
            _ => NodeKind::Suite,
        };
        let parent = self.container();
        let mut node = new_node(NodeId::ROOT, Some(parent), kind, name.to_owned(), source);
        node.label = suite_label(kind, name);
        node.state = TestState::Running;
        let id = self.push_node(node);
        self.open.push(id);
        // Une suite de plus après une fin : la racine repart.
        let root = &mut self.nodes[0];
        root.finished = false;
        root.state = TestState::Running;
    }

    fn finish_suite(&mut self, name: &str) {
        let Some(position) = self
            .open
            .iter()
            .rposition(|id| self.nodes[id.index()].name == name)
        else {
            return;
        };
        while self.open.len() > position {
            if let Some(id) = self.open.pop() {
                self.close(id);
            }
        }
    }

    fn close(&mut self, id: NodeId) {
        let node = &mut self.nodes[id.index()];
        node.finished = true;
        node.state = if node.worst == TestState::Pending {
            TestState::Passed
        } else {
            node.worst
        };
    }

    fn start_test(&mut self, name: &str, hint: Option<&str>) -> NodeId {
        let source = self.source_of(hint);
        let parent = self.container();
        let in_data_group = self.nodes[parent.index()].kind == NodeKind::Method;
        let (kind, label) = test_label(name, in_data_group);
        let mut node = new_node(NodeId::ROOT, Some(parent), kind, name.to_owned(), source);
        node.label = label;
        node.is_test = true;
        let id = self.push_node(node);
        self.counts.started += 1;
        self.set_state(id, TestState::Running);
        self.running.push((name.to_owned(), id));
        self.current_test = Some(id);
        id
    }

    fn running_test(&self, name: &str) -> Option<NodeId> {
        self.running
            .iter()
            .rposition(|(running, _)| running == name)
            .map(|at| self.running[at].1)
    }

    fn finish_test(&mut self, name: &str, duration: Option<Duration>) {
        let Some(at) = self
            .running
            .iter()
            .rposition(|(running, _)| running == name)
        else {
            return;
        };
        let (_, id) = self.running.remove(at);
        let node = &mut self.nodes[id.index()];
        let duration = duration.or(node.duration).unwrap_or_default();
        node.duration = Some(duration);
        if node.state == TestState::Running {
            self.set_state(id, TestState::Passed);
        }
        let mut parent = self.nodes[id.index()].parent;
        while let Some(current) = parent {
            let ancestor = &mut self.nodes[current.index()];
            ancestor.duration = Some(ancestor.duration.unwrap_or_default() + duration);
            parent = ancestor.parent;
        }
        if self.current_test == Some(id) {
            self.current_test = self.running.last().map(|(_, id)| *id);
        }
    }

    fn report(
        &mut self,
        name: &str,
        state: TestState,
        message: &str,
        details: &str,
        duration: Option<Duration>,
        comparison: Option<Comparison>,
    ) {
        let id = match self.running_test(name) {
            Some(id) => id,
            None => self.start_test(name, None),
        };
        let diagnostic = Diagnostic {
            message: message.to_owned(),
            trace: parse_trace(details, &self.project_root),
            comparison,
            cause: DiagnosticCause::Reported,
        };
        let node = &mut self.nodes[id.index()];
        if duration.is_some() {
            node.duration = duration;
        }
        if node.diagnostic.is_none() || state.severity() >= node.state.severity() {
            node.diagnostic = Some(diagnostic);
        }
        if state.severity() >= node.state.severity() {
            self.set_state(id, state);
        }
    }

    /// Change l'état d'un test et tient à jour compteurs et conteneurs.
    fn set_state(&mut self, id: NodeId, state: TestState) {
        let old = self.nodes[id.index()].state;
        if old == state {
            return;
        }
        self.counts.bump(old, false);
        self.counts.bump(state, true);
        self.nodes[id.index()].state = state;
        if state.is_final() {
            let mut parent = self.nodes[id.index()].parent;
            while let Some(current) = parent {
                let ancestor = &mut self.nodes[current.index()];
                if state.severity() > ancestor.worst.severity() {
                    ancestor.worst = state;
                }
                parent = ancestor.parent;
            }
        }
    }

    // --- sortie --------------------------------------------------------------

    fn output(&mut self, stream: Stream, text: &str) {
        if stream == Stream::Stderr {
            if self.stderr_tail.len() == STDERR_TAIL_LINES {
                self.stderr_tail.pop_front();
            }
            self.stderr_tail.push_back(text.to_owned());
        }
        let target = self.current_test.unwrap_or(NodeId::ROOT);
        self.push_console(target, stream, text);
        if target != NodeId::ROOT {
            self.push_console(NodeId::ROOT, stream, text);
        }
    }

    fn push_console(&mut self, id: NodeId, stream: Stream, text: &str) {
        let node = &mut self.nodes[id.index()];
        if node.console_bytes + text.len() > MAX_CONSOLE_BYTES {
            node.console_truncated = true;
            return;
        }
        node.console_bytes += text.len();
        node.console.push(ConsoleLine {
            stream,
            text: text.to_owned(),
        });
    }

    // --- fin ---------------------------------------------------------------

    fn abort(&mut self, reason: AbortReason) {
        self.aborted = Some(reason);
        let tail = self
            .stderr_tail
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join("\n");
        let running: Vec<NodeId> = self.running.drain(..).map(|(_, id)| id).collect();
        for id in running {
            match reason {
                AbortReason::ProcessExited { exit_code } => {
                    self.nodes[id.index()].diagnostic = Some(Diagnostic {
                        message: tail.clone(),
                        trace: Vec::new(),
                        comparison: None,
                        cause: DiagnosticCause::ProcessExited { exit_code },
                    });
                    self.set_state(id, TestState::Error);
                }
                AbortReason::Stopped | AbortReason::TimedOut => {
                    self.set_state(id, TestState::Interrupted);
                }
            }
        }
        self.current_test = None;
        while let Some(id) = self.open.pop() {
            self.close(id);
        }
    }

    fn end(&mut self) {
        while let Some(id) = self.open.pop() {
            self.close(id);
        }
        self.running.clear();
        self.current_test = None;
        self.ended = true;
        let aborted = self.aborted;
        let root = &mut self.nodes[0];
        root.finished = true;
        let worst = if root.worst == TestState::Pending {
            TestState::Passed
        } else {
            root.worst
        };
        let floor = match aborted {
            Some(AbortReason::ProcessExited { .. }) => TestState::Error,
            Some(AbortReason::Stopped | AbortReason::TimedOut) => TestState::Interrupted,
            None => TestState::Passed,
        };
        root.state = if floor.severity() > worst.severity() {
            floor
        } else {
            worst
        };
    }
}

fn new_node(
    id: NodeId,
    parent: Option<NodeId>,
    kind: NodeKind,
    name: String,
    source: Option<TestSource>,
) -> TestNode {
    TestNode {
        id,
        parent,
        children: Vec::new(),
        kind,
        label: name.clone(),
        name,
        source,
        is_test: false,
        state: TestState::Pending,
        worst: TestState::Pending,
        finished: false,
        duration: None,
        console: Vec::new(),
        console_truncated: false,
        diagnostic: None,
        console_bytes: 0,
    }
}

fn suite_label(kind: NodeKind, name: &str) -> String {
    match kind {
        NodeKind::Class => name.rsplit('\\').next().unwrap_or(name).to_owned(),
        NodeKind::Method => name.rsplit("::").next().unwrap_or(name).to_owned(),
        _ => name.to_owned(),
    }
}

const DATA_SET_MARK: &str = " with data set ";

/// Type et libellé d'un test d'après son nom. `in_data_group` : son parent est la
/// suite d'un fournisseur de données.
fn test_label(name: &str, in_data_group: bool) -> (NodeKind, String) {
    if let Some(at) = name.find(DATA_SET_MARK) {
        let data = name[at + DATA_SET_MARK.len()..].trim();
        let label = data
            .strip_prefix('"')
            .and_then(|rest| rest.strip_suffix('"'))
            .unwrap_or(data);
        return (NodeKind::DataSet, label.to_owned());
    }
    // `Classe::méthode` (PHPUnit 10 et suivants, supposé) : on garde la méthode.
    let method = match name.split_once("::") {
        Some((class, method)) if !class.contains([' ', '"', '\'']) => method,
        _ => name,
    };
    let kind = if in_data_group {
        NodeKind::DataSet
    } else {
        NodeKind::Method
    };
    (kind, method.to_owned())
}

/// Lit un `locationHint` : `php_qn://C:\chemin\Test.php::\Classe::méthode with
/// data set "x"`. Un chemin relatif se lit depuis `project_root`.
pub fn parse_location_hint(hint: &str, project_root: &Path) -> Option<TestSource> {
    let rest = hint
        .strip_prefix("php_qn://")
        .or_else(|| hint.strip_prefix("file://"))?;
    let (file, tail) = match rest.split_once("::") {
        Some((file, tail)) => (file, Some(tail)),
        None => (rest, None),
    };
    if file.is_empty() {
        return None;
    }
    let file = Path::new(file);
    let file = if file.is_absolute() {
        file.to_path_buf()
    } else {
        project_root.join(file)
    };
    let (class, method) = match tail {
        None => (None, None),
        Some(tail) => {
            let tail = tail.trim_start_matches('\\');
            let (class, method) = match tail.split_once("::") {
                Some((class, method)) => (class, Some(method)),
                None => (tail, None),
            };
            let method = method
                .map(|method| method.split(DATA_SET_MARK).next().unwrap_or(method))
                .filter(|method| !method.is_empty())
                .map(str::to_owned);
            let class = (!class.is_empty()).then(|| class.to_owned());
            (class, method)
        }
    };
    Some(TestSource {
        file,
        class,
        method,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn suite(name: &str, hint: Option<&str>) -> TestEvent {
        TestEvent::SuiteStarted {
            name: name.into(),
            location_hint: hint.map(str::to_owned),
        }
    }

    fn end_suite(name: &str) -> TestEvent {
        TestEvent::SuiteFinished { name: name.into() }
    }

    fn start(name: &str) -> TestEvent {
        TestEvent::TestStarted {
            name: name.into(),
            location_hint: None,
        }
    }

    fn finish(name: &str, millis: u64) -> TestEvent {
        TestEvent::TestFinished {
            name: name.into(),
            duration: Some(Duration::from_millis(millis)),
        }
    }

    fn fail(name: &str, state: TestState, message: &str, details: &str) -> TestEvent {
        TestEvent::TestFailed {
            name: name.into(),
            state,
            message: message.into(),
            details: details.into(),
            duration: None,
            comparison: None,
        }
    }

    fn output(text: &str) -> TestEvent {
        TestEvent::Output {
            stream: Stream::Stdout,
            text: text.into(),
        }
    }

    const CLASS_HINT: &str = r"php_qn://C:\p\tests\ATest.php::\App\ATest";

    fn sample() -> ResultTree {
        let mut tree = ResultTree::new(r"C:\p");
        tree.apply_all(&[
            TestEvent::TestCount(4),
            suite("main", None),
            suite("App\\ATest", Some(CLASS_HINT)),
            start("testA"),
            finish("testA", 10),
            suite(
                "testData",
                Some(r"php_qn://C:\p\tests\ATest.php::\App\ATest::testData"),
            ),
            start("testData with data set \"un\""),
            finish("testData with data set \"un\"", 5),
            start("testData with data set #1"),
            fail(
                "testData with data set #1",
                TestState::Failed,
                "Failed asserting",
                " C:\\p\\tests\\ATest.php:30\n ",
            ),
            finish("testData with data set #1", 7),
            end_suite("testData"),
            end_suite("App\\ATest"),
            end_suite("main"),
        ]);
        tree
    }

    #[test]
    fn the_tree_has_suite_class_method_and_data_sets() {
        let tree = sample();
        let main = tree.children(tree.root())[0];
        assert_eq!(tree.node(main).kind, NodeKind::Suite);
        let class = tree.children(main)[0];
        assert_eq!(tree.node(class).kind, NodeKind::Class);
        assert_eq!(tree.node(class).label, "ATest");
        let [method, group] = tree.children(class) else {
            panic!("deux enfants attendus");
        };
        assert_eq!(tree.node(*method).kind, NodeKind::Method);
        assert!(tree.node(*method).is_test);
        assert_eq!(tree.node(*group).kind, NodeKind::Method);
        assert!(!tree.node(*group).is_test);
        let sets = tree.children(*group);
        assert_eq!(sets.len(), 2);
        assert_eq!(tree.node(sets[0]).kind, NodeKind::DataSet);
        assert_eq!(tree.node(sets[0]).label, "un");
        assert_eq!(tree.node(sets[1]).label, "#1");
        assert_eq!(
            tree.path_of(sets[1]),
            vec![tree.root(), main, class, *group, sets[1]]
        );
    }

    #[test]
    fn states_counts_and_worst_state_bubble_up() {
        let tree = sample();
        let counts = tree.counts();
        assert_eq!(counts.total, Some(4));
        assert_eq!((counts.started, counts.passed, counts.failed), (3, 2, 1));
        assert_eq!(counts.running, 0);
        assert_eq!(counts.finished(), 3);
        let main = tree.children(tree.root())[0];
        assert_eq!(tree.node(main).state, TestState::Failed);
        assert!(tree.node(main).finished);
        let class = tree.children(main)[0];
        assert_eq!(tree.node(tree.children(class)[0]).state, TestState::Passed);
        assert_eq!(tree.node(class).state, TestState::Failed);
    }

    #[test]
    fn durations_are_summed_in_the_containers() {
        let tree = sample();
        let main = tree.children(tree.root())[0];
        assert_eq!(tree.node(main).duration, Some(Duration::from_millis(22)));
        assert_eq!(
            tree.node(tree.root()).duration,
            Some(Duration::from_millis(22))
        );
    }

    #[test]
    fn a_failure_keeps_its_message_and_a_link_to_the_line() {
        let tree = sample();
        let failed = tree
            .iter()
            .find(|node| node.name == "testData with data set #1")
            .unwrap();
        let diagnostic = failed.diagnostic.as_ref().unwrap();
        assert_eq!(diagnostic.message, "Failed asserting");
        let location = diagnostic.trace[0].location.as_ref().unwrap();
        assert_eq!(location.path, PathBuf::from(r"C:\p\tests\ATest.php"));
        assert_eq!(location.position.line, 29);
        let class = tree.iter().find(|n| n.kind == NodeKind::Class).unwrap();
        assert_eq!(
            class.jump_location().unwrap().path,
            PathBuf::from(r"C:\p\tests\ATest.php")
        );
    }

    #[test]
    fn output_goes_to_the_running_test_and_to_the_root() {
        let mut tree = ResultTree::new(r"C:\p");
        tree.apply_all(&[
            output("avant tout"),
            suite("main", None),
            start("t"),
            output("pendant t"),
            finish("t", 1),
            output("après t"),
        ]);
        let test = tree.iter().find(|node| node.name == "t").unwrap();
        assert_eq!(test.console_text(), "pendant t\n");
        let root = tree.node(tree.root());
        let texts: Vec<_> = root.console.iter().map(|line| line.text.as_str()).collect();
        assert_eq!(texts, ["avant tout", "pendant t", "après t"]);
    }

    #[test]
    fn a_crash_turns_the_running_test_into_an_error_with_the_stderr_tail() {
        let mut tree = ResultTree::new(r"C:\p");
        tree.apply_all(&[
            suite("main", None),
            start("avant"),
            finish("avant", 1),
            start("boum"),
            TestEvent::Output {
                stream: Stream::Stderr,
                text: "PHP Fatal error: Allowed memory size".into(),
            },
            TestEvent::Aborted(AbortReason::ProcessExited {
                exit_code: Some(255),
            }),
            TestEvent::Ended,
        ]);
        let boum = tree.iter().find(|node| node.name == "boum").unwrap();
        assert_eq!(boum.state, TestState::Error);
        let diagnostic = boum.diagnostic.as_ref().unwrap();
        assert_eq!(diagnostic.message, "PHP Fatal error: Allowed memory size");
        assert_eq!(
            diagnostic.cause,
            DiagnosticCause::ProcessExited {
                exit_code: Some(255)
            }
        );
        assert_eq!(tree.node(tree.root()).state, TestState::Error);
        assert_eq!(tree.counts().errors, 1);
        assert_eq!(tree.counts().passed, 1);
        assert!(tree.is_ended());
        assert!(!tree.has_open_work());
        // Les tests jamais commencés ne sont pas inventés.
        assert_eq!(tree.iter().filter(|node| node.is_test).count(), 2);
    }

    #[test]
    fn stopping_interrupts_the_running_test_without_a_diagnostic() {
        let mut tree = ResultTree::new(r"C:\p");
        tree.apply_all(&[
            suite("main", None),
            start("lent"),
            TestEvent::Aborted(AbortReason::Stopped),
            TestEvent::Ended,
        ]);
        let lent = tree.iter().find(|node| node.name == "lent").unwrap();
        assert_eq!(lent.state, TestState::Interrupted);
        assert!(lent.diagnostic.is_none());
        assert_eq!(tree.counts().interrupted, 1);
        assert_eq!(tree.node(tree.root()).state, TestState::Interrupted);
    }

    #[test]
    fn an_empty_run_ends_green_and_a_dead_start_ends_red() {
        let mut tree = ResultTree::new(r"C:\p");
        tree.apply(&TestEvent::Ended);
        assert_eq!(tree.node(tree.root()).state, TestState::Passed);
        assert_eq!(tree.counts().started, 0);

        let mut tree = ResultTree::new(r"C:\p");
        tree.apply_all(&[
            output("Could not load phpunit.xml"),
            TestEvent::Aborted(AbortReason::ProcessExited { exit_code: Some(1) }),
            TestEvent::Ended,
        ]);
        assert_eq!(tree.node(tree.root()).state, TestState::Error);
        assert!(!tree.saw_report());
    }

    #[test]
    fn a_worse_report_replaces_a_milder_one_but_not_the_reverse() {
        let mut tree = ResultTree::new(r"C:\p");
        tree.apply_all(&[
            suite("main", None),
            start("t"),
            fail("t", TestState::Failed, "premier", ""),
            fail("t", TestState::Risky, "plus doux", ""),
            fail("t", TestState::Error, "plus grave", ""),
            finish("t", 1),
        ]);
        let node = tree.iter().find(|node| node.name == "t").unwrap();
        assert_eq!(node.state, TestState::Error);
        assert_eq!(node.diagnostic.as_ref().unwrap().message, "plus grave");
        assert_eq!(tree.counts().errors, 1);
        assert_eq!(tree.counts().failed, 0);
    }

    #[test]
    fn a_failure_without_start_creates_the_test() {
        let mut tree = ResultTree::new(r"C:\p");
        tree.apply_all(&[
            suite("main", None),
            fail("orphelin", TestState::Failed, "m", ""),
            finish("orphelin", 2),
        ]);
        assert_eq!(tree.counts().failed, 1);
        assert_eq!(tree.counts().started, 1);
    }

    #[test]
    fn finishing_an_outer_suite_closes_the_inner_ones() {
        let mut tree = ResultTree::new(r"C:\p");
        tree.apply_all(&[
            suite("a", None),
            suite("b", None),
            end_suite("a"),
            end_suite("inconnue"),
        ]);
        assert!(!tree.has_open_work());
        assert!(tree
            .iter()
            .filter(|n| n.kind == NodeKind::Suite)
            .all(|n| n.finished));
    }

    #[test]
    fn a_huge_console_is_truncated() {
        let mut tree = ResultTree::new(r"C:\p");
        tree.apply(&suite("main", None));
        tree.apply(&start("t"));
        let line = "x".repeat(100_000);
        for _ in 0..20 {
            tree.apply(&output(&line));
        }
        let node = tree.iter().find(|node| node.name == "t").unwrap();
        assert!(node.console_truncated);
        assert!(node.console_text().len() <= MAX_CONSOLE_BYTES + node.console.len());
    }

    #[test]
    fn location_hints_are_read() {
        let hint = r#"php_qn://C:\Program Files (x86)\x\T.php::\A\B::testM with data set "a::b""#;
        let source = parse_location_hint(hint, Path::new(r"C:\p")).unwrap();
        assert_eq!(
            source.file,
            PathBuf::from(r"C:\Program Files (x86)\x\T.php")
        );
        assert_eq!(source.class.as_deref(), Some(r"A\B"));
        assert_eq!(source.method.as_deref(), Some("testM"));

        let only_class =
            parse_location_hint(r"php_qn://C:\x\T.php::\A\B", Path::new("C:/p")).unwrap();
        assert_eq!(
            (only_class.class.as_deref(), only_class.method),
            (Some(r"A\B"), None)
        );

        let relative = parse_location_hint("php_qn://tests/T.php", Path::new(r"C:\p")).unwrap();
        assert_eq!(relative.file, Path::new(r"C:\p").join("tests/T.php"));
        assert!(parse_location_hint("http://x", Path::new("C:/p")).is_none());
    }

    #[test]
    fn names_with_a_class_prefix_lose_it_in_the_label() {
        assert_eq!(
            test_label("App\\ATest::testA", false),
            (NodeKind::Method, "testA".into())
        );
        assert_eq!(
            test_label("testA with data set \"x y\"", false),
            (NodeKind::DataSet, "x y".into())
        );
        assert_eq!(
            test_label("with data set sans espace", true).0,
            NodeKind::DataSet
        );
    }
}
