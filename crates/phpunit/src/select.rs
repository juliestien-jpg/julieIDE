//! Lancer une partie de la suite : un fichier, un dossier, une classe, une méthode,
//! un jeu de données, un nœud de l'arbre, ou « relancer les échecs ».
//!
//! # Le filtre
//!
//! PHPUnit compare `--filter` au nom complet de chaque test, de la forme
//! `Espace\Classe::méthode` ou `Espace\Classe::méthode with data set "nom"` (ou
//! `#3` pour un jeu sans nom). Les motifs produits ici sont des expressions
//! régulières **déjà délimitées** par `/` et ancrées, de sorte que PHPUnit ne les
//! réinterprète pas (il lit autrement les suffixes `#3` et `@nom` d'un motif non
//! délimité) :
//!
//! | Cible | Motif |
//! |---|---|
//! | classe | `/^Ns\\Classe::/s` |
//! | méthode (tous ses jeux) | `/^Ns\\Classe::méthode(?: with data set .*)?$/s` |
//! | jeu de données | `/^Ns\\Classe::méthode with data set "nom"$/s` |
//! | plusieurs tests | les motifs sans délimiteur, séparés par `\|` |
//!
//! Tout le texte venant d'un nom de test passe par [`quote_regex`] : `'`, `|`,
//! `[`, `]`, `"`, `\`, `/`, `#`, retours à la ligne et octets nuls compris.
//!
//! # Le chemin
//!
//! Un fichier ou un dossier est donné à PHPUnit en **dernier** argument ; il remplace
//! les suites de `phpunit.xml`. Le lancer avec le filtre permet de lancer un test
//! qui n'est pas dans la suite par défaut (`defaultTestSuite`). Quand le nom du
//! fichier ne correspond pas à celui de la classe, PHPUnit ne retrouverait pas la
//! classe : le chemin est alors omis et l'on s'en remet à la base de la demande
//! précédente (ou à la suite par défaut).

use std::path::{Path, PathBuf};

use crate::model::{NodeId, NodeKind, ResultTree, TestNode, TestState};
use crate::run::{Base, RunRequest, RunScope, Selection};

/// Longueur au-delà de laquelle un filtre de « relancer les échecs » est remplacé
/// par un filtre par classe (la ligne de commande Windows est limitée à 32 767
/// caractères).
pub const MAX_FILTER_LEN: usize = 20_000;

const DATA_SET_MARK: &str = " with data set ";

/// Échappe `text` pour qu'une expression régulière PCRE le lise littéralement.
/// `/` est échappé aussi : le délimiteur est toujours `/`.
pub fn quote_regex(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    for c in text.chars() {
        match c {
            '\\' | '.' | '+' | '*' | '?' | '[' | '^' | ']' | '$' | '(' | ')' | '{' | '}' | '='
            | '!' | '<' | '>' | '|' | ':' | '-' | '#' | '/' => {
                out.push('\\');
                out.push(c);
            }
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\0' => out.push_str("\\x00"),
            c if c.is_control() => out.push_str(&format!("\\x{{{:x}}}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// Entoure des motifs (voir [`class_pattern`]...) de délimiteurs : `/a|b/s`. Le
/// drapeau `s` fait couvrir les retours à la ligne des noms de jeux de données.
pub fn delimit(patterns: &[String]) -> String {
    format!("/{}/s", patterns.join("|"))
}

/// Tous les tests d'une classe.
pub fn class_pattern(class: &str) -> String {
    format!("^{}::", quote_regex(class.trim_start_matches('\\')))
}

/// Une méthode et tous ses jeux de données.
pub fn method_pattern(class: &str, method: &str) -> String {
    format!(
        "^{}::{}(?: with data set .*)?$",
        quote_regex(class.trim_start_matches('\\')),
        quote_regex(method)
    )
}

/// Un jeu de données : `raw` est le texte que PHPUnit écrit après `with data set`,
/// guillemets compris (`"mon jeu"`) ou `#3`.
pub fn data_set_pattern(class: &str, method: &str, raw: &str) -> String {
    format!(
        "^{}::{} with data set {}$",
        quote_regex(class.trim_start_matches('\\')),
        quote_regex(method),
        quote_regex(raw)
    )
}

/// Un jeu de données, désigné comme dans le code PHP : par son nom ou son rang.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataSetKey {
    Name(String),
    Index(u64),
}

impl DataSetKey {
    /// Comme PHPUnit l'écrit : `"nom"` ou `#3`. Le nom n'est pas échappé par
    /// PHPUnit (un `"` y reste tel quel).
    pub fn raw(&self) -> String {
        match self {
            DataSetKey::Name(name) => format!("\"{name}\""),
            DataSetKey::Index(index) => format!("#{index}"),
        }
    }
}

impl RunRequest {
    /// Un fichier ou un dossier de tests.
    pub fn path(path: impl Into<PathBuf>) -> Self {
        Self::selection(Selection {
            base: Base::Path(path.into()),
            filter: None,
        })
    }

    /// Une classe de test, dans son fichier.
    pub fn class(file: impl Into<PathBuf>, class: &str) -> Self {
        let filter = delimit(&[class_pattern(class)]);
        Self::targeted(file.into(), class, filter, &Base::Default)
    }

    /// Une méthode de test (tous ses jeux de données), dans son fichier.
    pub fn method(file: impl Into<PathBuf>, class: &str, method: &str) -> Self {
        let filter = delimit(&[method_pattern(class, method)]);
        Self::targeted(file.into(), class, filter, &Base::Default)
    }

    /// Un seul jeu de données d'une méthode.
    pub fn data_set(file: impl Into<PathBuf>, class: &str, method: &str, key: &DataSetKey) -> Self {
        let filter = delimit(&[data_set_pattern(class, method, &key.raw())]);
        Self::targeted(file.into(), class, filter, &Base::Default)
    }

    fn selection(selection: Selection) -> Self {
        Self {
            scope: RunScope::Selection(selection),
            ..Self::default()
        }
    }

    fn targeted(file: PathBuf, class: &str, filter: String, fallback: &Base) -> Self {
        let base = if file_matches_class(&file, class) {
            Base::Path(file)
        } else {
            fallback.clone()
        };
        Self::selection(Selection {
            base,
            filter: Some(filter),
        })
    }

    /// Lancer (ou relancer) un nœud de l'arbre des résultats : la racine relance
    /// la demande précédente, une suite de premier niveau se lance par son nom,
    /// une classe, une méthode ou un jeu de données par leur filtre et leur
    /// fichier. `None` : nœud sans source connue (l'entrée de menu est grisée).
    ///
    /// `previous` fournit les options, variables, dossier et délai à reprendre, et
    /// la base (suite ou chemin) quand le nœud ne désigne qu'un filtre.
    pub fn for_node(
        tree: &ResultTree,
        id: NodeId,
        previous: Option<&RunRequest>,
    ) -> Option<RunRequest> {
        let node = tree.node(id);
        let mut request = match node.kind {
            NodeKind::Root => {
                let mut request = carry(previous);
                if let Some(previous) = previous {
                    request.scope = previous.scope.clone();
                }
                return Some(request);
            }
            NodeKind::Suite => suite_request(tree, node, previous)?,
            NodeKind::Class | NodeKind::Method | NodeKind::DataSet => {
                node_request(tree, node, previous)?
            }
        };
        let carried = carry(previous);
        request.extra_args = carried.extra_args;
        request.env = carried.env;
        request.working_dir = carried.working_dir;
        request.timeout = carried.timeout;
        Some(request)
    }

    /// Relancer les tests en échec ou en erreur de `tree`. `None` : aucun.
    ///
    /// Un seul fichier en cause : on le passe à PHPUnit ; sinon on reprend la base
    /// de `previous`. Si un test en échec n'a pas de source connue, on ne sait pas
    /// le désigner : la demande reprend alors la portée précédente sans filtre
    /// (tout est relancé, jamais moins). Au-delà de [`MAX_FILTER_LEN`] caractères,
    /// le filtre passe de « un test » à « sa classe ».
    pub fn rerun_failures(tree: &ResultTree, previous: Option<&RunRequest>) -> Option<RunRequest> {
        let failed: Vec<&TestNode> = tree
            .iter()
            .filter(|node| {
                node.is_test && matches!(node.state, TestState::Failed | TestState::Error)
            })
            .collect();
        if failed.is_empty() {
            return None;
        }
        let mut request = carry(previous);
        let previous_base = base_of(previous);
        let leaves: Option<Vec<Leaf>> = failed.iter().map(|node| Leaf::of(node)).collect();
        let Some(leaves) = leaves else {
            if let Some(previous) = previous {
                request.scope = previous.scope.clone();
            }
            return Some(request);
        };

        let mut patterns: Vec<String> = Vec::new();
        for leaf in &leaves {
            let pattern = match &leaf.data_set {
                Some(raw) => data_set_pattern(&leaf.class, &leaf.method, raw),
                None => method_pattern(&leaf.class, &leaf.method),
            };
            if !patterns.contains(&pattern) {
                patterns.push(pattern);
            }
        }
        let mut filter = delimit(&patterns);
        if filter.len() > MAX_FILTER_LEN {
            patterns.clear();
            for leaf in &leaves {
                let pattern = class_pattern(&leaf.class);
                if !patterns.contains(&pattern) {
                    patterns.push(pattern);
                }
            }
            filter = delimit(&patterns);
        }
        let base = common_path(&leaves).unwrap_or(previous_base);
        request.scope = if filter.len() > MAX_FILTER_LEN {
            previous.map_or(RunScope::All, |previous| previous.scope.clone())
        } else {
            RunScope::Selection(Selection {
                base,
                filter: Some(filter),
            })
        };
        Some(request)
    }
}

/// Une demande vide qui garde ce que `previous` règle hors portée.
fn carry(previous: Option<&RunRequest>) -> RunRequest {
    previous.map_or_else(RunRequest::default, |previous| RunRequest {
        scope: RunScope::All,
        extra_args: previous.extra_args.clone(),
        env: previous.env.clone(),
        working_dir: previous.working_dir.clone(),
        timeout: previous.timeout,
    })
}

/// La base (suite ou chemin) de la demande précédente.
fn base_of(previous: Option<&RunRequest>) -> Base {
    match previous.map(|request| &request.scope) {
        Some(RunScope::TestSuite(name)) => Base::TestSuite(name.clone()),
        Some(RunScope::Selection(selection)) => selection.base.clone(),
        _ => Base::Default,
    }
}

/// Un test identifié sans ambiguïté.
struct Leaf {
    file: PathBuf,
    class: String,
    method: String,
    /// Texte après `with data set`, tel que PHPUnit l'écrit.
    data_set: Option<String>,
}

impl Leaf {
    fn of(node: &TestNode) -> Option<Leaf> {
        let source = node.source.as_ref()?;
        let data_set = if node.kind == NodeKind::DataSet {
            node.name
                .split_once(DATA_SET_MARK)
                .map(|(_, raw)| raw.trim().to_owned())
        } else {
            None
        };
        Some(Leaf {
            file: source.file.clone(),
            class: source.class.clone()?,
            method: source.method.clone()?,
            data_set,
        })
    }
}

/// Le fichier commun à tous les tests, s'il est unique et que PHPUnit peut y
/// retrouver la classe.
fn common_path(leaves: &[Leaf]) -> Option<Base> {
    let first = leaves.first()?;
    leaves
        .iter()
        .all(|leaf| leaf.file == first.file && file_matches_class(&leaf.file, &leaf.class))
        .then(|| Base::Path(first.file.clone()))
}

/// PHPUnit retrouve la classe d'un fichier d'après son nom : `CalculatorTest.php`
/// contient `…\CalculatorTest`.
fn file_matches_class(file: &Path, class: &str) -> bool {
    let stem = file.file_stem().and_then(|stem| stem.to_str());
    let short = class.rsplit('\\').next().unwrap_or(class);
    stem.is_some_and(|stem| stem.eq_ignore_ascii_case(short))
}

/// Les feuilles (tests annoncés) sous `node`, lui compris.
fn leaves_under<'a>(tree: &'a ResultTree, node: &'a TestNode) -> Vec<&'a TestNode> {
    let mut out = Vec::new();
    let mut stack = vec![node];
    while let Some(current) = stack.pop() {
        if current.is_test {
            out.push(current);
        }
        for child in current.children.iter().rev() {
            stack.push(tree.node(*child));
        }
    }
    out
}

fn suite_request(
    tree: &ResultTree,
    node: &TestNode,
    previous: Option<&RunRequest>,
) -> Option<RunRequest> {
    // Une suite de premier niveau sans source est une suite de `phpunit.xml`.
    if node.parent == Some(tree.root()) && node.source.is_none() {
        return Some(RunRequest::test_suite(node.name.clone()));
    }
    // Une suite qui désigne un dossier ou un fichier.
    if let Some(source) = &node.source {
        if source.class.is_none() {
            return Some(RunRequest::path(source.file.clone()));
        }
    }
    // Sinon : les classes de ses tests.
    let leaves: Vec<Leaf> = leaves_under(tree, node)
        .into_iter()
        .map(Leaf::of)
        .collect::<Option<_>>()?;
    if leaves.is_empty() {
        return None;
    }
    let mut patterns: Vec<String> = Vec::new();
    for leaf in &leaves {
        let pattern = class_pattern(&leaf.class);
        if !patterns.contains(&pattern) {
            patterns.push(pattern);
        }
    }
    let base = common_path(&leaves).unwrap_or_else(|| base_of(previous));
    Some(RunRequest::selection(Selection {
        base,
        filter: Some(delimit(&patterns)),
    }))
}

fn node_request(
    tree: &ResultTree,
    node: &TestNode,
    previous: Option<&RunRequest>,
) -> Option<RunRequest> {
    // Un conteneur (classe, méthode à jeux de données) peut n'avoir pas de source
    // propre : on prend celle de son premier test.
    let leaves = leaves_under(tree, node);
    let source = node
        .source
        .as_ref()
        .or_else(|| leaves.iter().find_map(|leaf| leaf.source.as_ref()))?;
    let file = source.file.clone();
    let class = source
        .class
        .clone()
        .or_else(|| leaves.iter().find_map(|l| l.source.as_ref()?.class.clone()))?;
    let pattern = match node.kind {
        NodeKind::Class => class_pattern(&class),
        NodeKind::Method => {
            let method = source.method.clone().or_else(|| {
                leaves
                    .iter()
                    .find_map(|l| l.source.as_ref()?.method.clone())
            })?;
            method_pattern(&class, &method)
        }
        NodeKind::DataSet => {
            let method = source.method.clone()?;
            match Leaf::of(node).and_then(|leaf| leaf.data_set) {
                Some(raw) => data_set_pattern(&class, &method, &raw),
                // Nom de jeu inconnu : toute la méthode, jamais moins.
                None => method_pattern(&class, &method),
            }
        }
        NodeKind::Root | NodeKind::Suite => return None,
    };
    Some(RunRequest::targeted(
        file,
        &class,
        delimit(&[pattern]),
        &base_of(previous),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::TestEvent;

    fn arg_filter(request: &RunRequest) -> Option<&str> {
        match &request.scope {
            RunScope::Selection(selection) => selection.filter.as_deref(),
            _ => None,
        }
    }

    fn base(request: &RunRequest) -> &Base {
        match &request.scope {
            RunScope::Selection(selection) => &selection.base,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn regex_quoting_escapes_every_metacharacter() {
        assert_eq!(quote_regex("a.b"), r"a\.b");
        assert_eq!(quote_regex(r"Ns\Classe"), r"Ns\\Classe");
        assert_eq!(
            quote_regex("l'apostrophe | la barre [et] les crochets"),
            r"l'apostrophe \| la barre \[et\] les crochets"
        );
        assert_eq!(quote_regex(r#"a "b" / c"#), r#"a "b" \/ c"#);
        assert_eq!(quote_regex("(x)+{2}?*^$"), r"\(x\)\+\{2\}\?\*\^\$");
        assert_eq!(quote_regex("a#3 @b = !<>:-"), r"a\#3 @b \= \!\<\>\:\-");
        assert_eq!(quote_regex("l1\nl2\r\n\t\0"), r"l1\nl2\r\n\t\x00");
        assert_eq!(quote_regex("é à ü 日本"), "é à ü 日本");
        assert_eq!(quote_regex("\u{7}"), r"\x{7}");
    }

    #[test]
    fn patterns_are_anchored_and_delimited() {
        assert_eq!(
            class_pattern(r"\Fixture\Tests\CalculatorTest"),
            r"^Fixture\\Tests\\CalculatorTest::"
        );
        assert_eq!(
            method_pattern("A\\BTest", "testAdd"),
            r"^A\\BTest::testAdd(?: with data set .*)?$"
        );
        assert_eq!(
            data_set_pattern("A\\BTest", "testAdd", r#""a|b""#),
            r#"^A\\BTest::testAdd with data set "a\|b"$"#
        );
        assert_eq!(delimit(&["^a$".to_owned(), "^b$".to_owned()]), "/^a$|^b$/s");
        assert_eq!(DataSetKey::Name("x y".into()).raw(), "\"x y\"");
        assert_eq!(DataSetKey::Index(3).raw(), "#3");
    }

    #[test]
    fn a_method_request_carries_its_file_last_and_its_filter() {
        let request = RunRequest::method(
            "C:/p/tests/CalculatorTest.php",
            r"Fixture\Tests\CalculatorTest",
            "testAdd",
        );
        assert_eq!(
            base(&request),
            &Base::Path(PathBuf::from("C:/p/tests/CalculatorTest.php"))
        );
        assert_eq!(
            arg_filter(&request),
            Some(r"/^Fixture\\Tests\\CalculatorTest::testAdd(?: with data set .*)?$/s")
        );
    }

    #[test]
    fn a_file_whose_name_is_not_the_class_name_is_not_passed_to_phpunit() {
        let request = RunRequest::class("C:/p/tests/Helpers.php", r"App\FooTest");
        assert_eq!(base(&request), &Base::Default);
        assert!(arg_filter(&request).is_some());
    }

    // --- à partir d'un arbre -----------------------------------------------------

    fn hint(class: &str, rest: &str) -> String {
        format!(r"php_qn://C:\p\tests\{class}.php::\App\{class}{rest}")
    }

    fn suite(name: &str, hint: Option<String>) -> TestEvent {
        TestEvent::SuiteStarted {
            name: name.into(),
            location_hint: hint,
        }
    }

    fn start(name: &str, hint: String) -> TestEvent {
        TestEvent::TestStarted {
            name: name.into(),
            location_hint: Some(hint),
        }
    }

    fn pass(name: &str) -> TestEvent {
        TestEvent::TestFinished {
            name: name.into(),
            duration: None,
        }
    }

    fn fail(name: &str) -> TestEvent {
        TestEvent::TestFailed {
            name: name.into(),
            state: TestState::Failed,
            message: "x".into(),
            details: String::new(),
            duration: None,
            comparison: None,
        }
    }

    fn end_suite(name: &str) -> TestEvent {
        TestEvent::SuiteFinished { name: name.into() }
    }

    /// main > FooTest (testA ok, testB échoue, testC avec 3 jeux dont 2 échouent) ;
    /// main > BarTest (testD échoue).
    fn sample_tree() -> ResultTree {
        let weird = r#"l'apostrophe | la barre [et] les "crochets" / #1"#;
        let mut events = vec![
            suite("main", None),
            suite(r"App\FooTest", Some(hint("FooTest", ""))),
            start("testA", hint("FooTest", "::testA")),
            pass("testA"),
            start("testB", hint("FooTest", "::testB")),
            fail("testB"),
        ];
        events.push(suite(
            r"App\FooTest::testC",
            Some(hint("FooTest", "::testC")),
        ));
        for (n, set) in [("ok", false), (weird, true), ("ko", true)] {
            let name = format!(r#"testC with data set "{n}""#);
            events.push(start(
                &name,
                hint("FooTest", &format!(r#"::testC with data set "{n}""#)),
            ));
            events.push(if set { fail(&name) } else { pass(&name) });
        }
        events.extend([
            end_suite(r"App\FooTest::testC"),
            end_suite(r"App\FooTest"),
            suite(r"App\BarTest", Some(hint("BarTest", ""))),
            start("testD", hint("BarTest", "::testD")),
            fail("testD"),
            end_suite(r"App\BarTest"),
            end_suite("main"),
            TestEvent::Ended,
        ]);
        let mut tree = ResultTree::new(r"C:\p");
        tree.apply_all(&events);
        tree
    }

    fn find(tree: &ResultTree, kind: NodeKind, name: &str) -> NodeId {
        tree.iter()
            .find(|node| node.kind == kind && node.name.contains(name))
            .unwrap_or_else(|| panic!("{kind:?} {name}"))
            .id
    }

    #[test]
    fn nodes_become_requests() {
        let tree = sample_tree();
        let previous = RunRequest {
            extra_args: vec!["--stop-on-failure".into()],
            env: vec![("A".into(), "1".into())],
            ..RunRequest::default()
        };

        let suite_id = find(&tree, NodeKind::Suite, "main");
        let request = RunRequest::for_node(&tree, suite_id, Some(&previous)).unwrap();
        assert_eq!(request.scope, RunScope::TestSuite("main".into()));
        assert_eq!(request.extra_args, previous.extra_args);
        assert_eq!(request.env, previous.env);

        let class_id = find(&tree, NodeKind::Class, "FooTest");
        let request = RunRequest::for_node(&tree, class_id, None).unwrap();
        assert_eq!(
            base(&request),
            &Base::Path(PathBuf::from(r"C:\p\tests\FooTest.php"))
        );
        assert_eq!(arg_filter(&request), Some(r"/^App\\FooTest::/s"));

        let method_id = find(&tree, NodeKind::Method, "testC");
        let request = RunRequest::for_node(&tree, method_id, None).unwrap();
        assert_eq!(
            arg_filter(&request),
            Some(r"/^App\\FooTest::testC(?: with data set .*)?$/s")
        );

        let set_id = find(&tree, NodeKind::DataSet, "crochets");
        let request = RunRequest::for_node(&tree, set_id, None).unwrap();
        assert_eq!(
            arg_filter(&request),
            Some(
                r#"/^App\\FooTest::testC with data set "l'apostrophe \| la barre \[et\] les "crochets" \/ \#1"$/s"#
            )
        );

        let root = RunRequest::for_node(&tree, tree.root(), Some(&previous)).unwrap();
        assert_eq!(
            root,
            RunRequest {
                scope: RunScope::All,
                ..previous.clone()
            }
        );
    }

    #[test]
    fn rerun_failures_names_each_failed_test_exactly() {
        let tree = sample_tree();
        let request = RunRequest::rerun_failures(&tree, None).unwrap();
        // Tests en échec dans deux fichiers : pas de chemin.
        assert_eq!(base(&request), &Base::Default);
        assert_eq!(
            arg_filter(&request),
            Some(concat!(
                r"/^App\\FooTest::testB(?: with data set .*)?$",
                r#"|^App\\FooTest::testC with data set "l'apostrophe \| la barre \[et\] les "crochets" \/ \#1"$"#,
                r#"|^App\\FooTest::testC with data set "ko"$"#,
                r"|^App\\BarTest::testD(?: with data set .*)?$/s"
            ))
        );
    }

    #[test]
    fn rerun_failures_keeps_the_previous_base_and_one_common_file() {
        let tree = sample_tree();
        let previous = RunRequest::test_suite("main");
        let request = RunRequest::rerun_failures(&tree, Some(&previous)).unwrap();
        assert_eq!(base(&request), &Base::TestSuite("main".into()));

        // Un seul fichier en cause : on le passe.
        let mut one = ResultTree::new(r"C:\p");
        one.apply_all(&[
            suite("main", None),
            suite(r"App\BarTest", Some(hint("BarTest", ""))),
            start("testD", hint("BarTest", "::testD")),
            fail("testD"),
            end_suite(r"App\BarTest"),
            end_suite("main"),
            TestEvent::Ended,
        ]);
        let request = RunRequest::rerun_failures(&one, Some(&previous)).unwrap();
        assert_eq!(
            base(&request),
            &Base::Path(PathBuf::from(r"C:\p\tests\BarTest.php"))
        );
    }

    #[test]
    fn rerun_failures_is_none_without_failures() {
        let mut tree = ResultTree::new(r"C:\p");
        tree.apply_all(&[
            suite("main", None),
            suite(r"App\FooTest", Some(hint("FooTest", ""))),
            start("testA", hint("FooTest", "::testA")),
            pass("testA"),
            end_suite(r"App\FooTest"),
            end_suite("main"),
            TestEvent::Ended,
        ]);
        assert!(RunRequest::rerun_failures(&tree, None).is_none());
    }

    #[test]
    fn rerun_failures_falls_back_to_the_class_when_the_filter_is_too_long() {
        let mut events = vec![suite("main", None)];
        for class in 0..30 {
            let class_name = format!("Class{class:02}Test");
            events.push(suite(
                &format!(r"App\{class_name}"),
                Some(hint(&class_name, "")),
            ));
            for method in 0..40 {
                let name = format!("testWithAVeryLongDescriptiveName{method:03}");
                events.push(start(&name, hint(&class_name, &format!("::{name}"))));
                events.push(fail(&name));
            }
            events.push(end_suite(&format!(r"App\{class_name}")));
        }
        events.extend([end_suite("main"), TestEvent::Ended]);
        let mut tree = ResultTree::new(r"C:\p");
        tree.apply_all(&events);
        let request = RunRequest::rerun_failures(&tree, None).unwrap();
        let filter = arg_filter(&request).unwrap();
        assert!(filter.len() < MAX_FILTER_LEN, "{}", filter.len());
        assert!(filter.starts_with(r"/^App\\Class00Test::|"));
        assert_eq!(filter.matches("::").count(), 30);
    }

    #[test]
    fn a_failure_without_source_reruns_the_previous_scope_whole() {
        let mut tree = ResultTree::new(r"C:\p");
        tree.apply_all(&[
            suite("main", None),
            TestEvent::TestStarted {
                name: "testSansSource".into(),
                location_hint: None,
            },
            fail("testSansSource"),
            end_suite("main"),
            TestEvent::Ended,
        ]);
        let previous = RunRequest::test_suite("main");
        let request = RunRequest::rerun_failures(&tree, Some(&previous)).unwrap();
        assert_eq!(request.scope, RunScope::TestSuite("main".into()));
        let node = find(&tree, NodeKind::Method, "testSansSource");
        assert!(RunRequest::for_node(&tree, node, None).is_none());
    }
}
