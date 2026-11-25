//! Critères d'acceptation de `docs/specs/navigation.md` sur le projet d'essai
//! `tests/fixtures/navigation/`, copié dans un dossier temporaire.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use julie_core::{Bus, CancelToken, Domain, Event, Project, Tasks, TextPosition};
use julie_index::{
    find_usages, navigate, Declaration, FileIndex, Guess, IndexEvent, IndexService, IndexSettings,
    Navigation, NavigationRequest, NavigationResult, NoTarget, SymbolIndex, SymbolKind,
    UnsavedFiles, Usage, UsageCertainty, UsagesQuery, UsagesSummary,
};

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

struct Fx {
    _dir: tempfile::TempDir,
    root: PathBuf,
    files: FileIndex,
    symbols: SymbolIndex,
    settings: IndexSettings,
}

fn fixture() -> Fx {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/navigation");
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("navigation");
    copy_dir(&source, &root);
    let settings = IndexSettings::default();
    let cancel = CancelToken::new();
    let files = FileIndex::scan(&root, settings.clone(), &cancel).unwrap();
    let (symbols, _) = SymbolIndex::build(&files, None, &UnsavedFiles::new(), &cancel).unwrap();
    Fx {
        _dir: dir,
        root,
        files,
        symbols,
        settings,
    }
}

impl Fx {
    fn path(&self, relative: &str) -> PathBuf {
        self.files.absolute(relative)
    }

    fn text(&self, relative: &str) -> String {
        std::fs::read_to_string(self.path(relative)).unwrap()
    }

    /// Position de la `nth` occurrence (base zéro) de `needle`, décalée de
    /// `delta` caractères.
    fn request(&self, relative: &str, needle: &str, nth: usize, delta: usize) -> NavigationRequest {
        let text = self.text(relative);
        request_in(self.path(relative), &text, needle, nth, delta)
    }

    fn goto(&self, relative: &str, needle: &str, nth: usize, delta: usize) -> NavigationResult {
        navigate(
            &self.symbols,
            &self.request(relative, needle, nth, delta),
            &UnsavedFiles::new(),
        )
    }

    fn declaration(&self, relative: &str, needle: &str, nth: usize, delta: usize) -> Declaration {
        match self.goto(relative, needle, nth, delta).navigation {
            Navigation::Declaration(d) => d,
            other => panic!("pas une déclaration : {other:?}"),
        }
    }

    fn usages(&self, symbol: &Declaration, unsaved: &UnsavedFiles) -> (Vec<Usage>, UsagesSummary) {
        let mut all = Vec::new();
        let summary = find_usages(
            &self.symbols,
            &UsagesQuery::new(symbol.clone()),
            unsaved,
            &self.settings,
            true,
            &CancelToken::new(),
            &mut |batch| all.extend(batch),
        )
        .unwrap();
        all.sort_by(|a: &Usage, b: &Usage| {
            (
                a.relative.as_str(),
                a.range.start.line,
                a.range.start.column,
            )
                .cmp(&(
                    b.relative.as_str(),
                    b.range.start.line,
                    b.range.start.column,
                ))
        });
        (all, summary)
    }
}

fn request_in(
    path: PathBuf,
    text: &str,
    needle: &str,
    nth: usize,
    delta: usize,
) -> NavigationRequest {
    let at = text
        .match_indices(needle)
        .nth(nth)
        .unwrap_or_else(|| panic!("« {needle} » n°{nth} absent"))
        .0;
    let before = &text[..at];
    let line = before.matches('\n').count() as u32;
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    let column = text[line_start..at].chars().count() as u32 + delta as u32;
    NavigationRequest {
        path,
        text: Arc::from(text),
        position: TextPosition::new(line, column),
    }
}

/// Noms qualifiés des cibles (et la supposition éventuelle).
fn targets(result: &NavigationResult) -> (Vec<String>, Option<Guess>) {
    match &result.navigation {
        Navigation::Targets(t) => (
            t.targets.iter().map(|d| d.qualified_name.clone()).collect(),
            t.guess.clone(),
        ),
        other => panic!("pas de cibles : {other:?}"),
    }
}

fn single(result: &NavigationResult) -> Declaration {
    match &result.navigation {
        Navigation::Targets(t) => {
            assert_eq!(t.targets.len(), 1, "{:?}", t.targets);
            assert!(t.guess.is_none(), "{:?}", t.guess);
            t.targets[0].clone()
        }
        other => panic!("pas de cible : {other:?}"),
    }
}

const US: &str = "src/Service/UserService.php";

// --- PHP, aller à la déclaration ----------------------------------------------

#[test]
fn critere_01_methode_d_un_parametre_type() {
    let fx = fixture();
    let d = single(&fx.goto(US, "$user->getName()", 0, 7));
    assert_eq!(d.qualified_name, "App\\Model\\User::getName");
    assert_eq!(d.relative, "src/Model/User.php");
    assert_eq!(d.kind, SymbolKind::Method);
    // Curseur sur le nom déclaré.
    let text = fx.text("src/Model/User.php");
    let line = text.lines().nth(d.range.start.line as usize).unwrap();
    let name: String = line
        .chars()
        .skip(d.range.start.column as usize)
        .take((d.range.end.column - d.range.start.column) as usize)
        .collect();
    assert_eq!(name, "getName");
    assert_eq!(d.line_text, "public function getName(): string");
}

#[test]
fn critere_02_propriete_typee_new_et_alias() {
    let fx = fixture();
    let d = single(&fx.goto(US, "$this->current->getName()", 0, 16));
    assert_eq!(d.qualified_name, "App\\Model\\User::getName");
    let d = single(&fx.goto(US, "$fresh->getName()", 0, 8));
    assert_eq!(d.qualified_name, "App\\Model\\User::getName");
    let d = single(&fx.goto(US, "$boss->getName()", 0, 7));
    assert_eq!(d.qualified_name, "App\\Model\\Admin::getName");
}

#[test]
fn critere_03_type_inconnu_trois_candidats_dans_l_ordre() {
    let fx = fixture();
    let (names, guess) = targets(&fx.goto(US, "$other->getName()", 0, 8));
    assert_eq!(
        names,
        vec![
            "App\\Legacy\\Order::getName",
            "App\\Model\\Admin::getName",
            "App\\Model\\User::getName"
        ]
    );
    assert_eq!(
        guess,
        Some(Guess::UnknownReceiver {
            receiver: "$other".into(),
            member: "getName".into(),
            kind: SymbolKind::Method
        })
    );
}

#[test]
fn critere_04_heritage_traits_parent() {
    let fx = fixture();
    assert_eq!(
        single(&fx.goto(US, "$user->save()", 0, 7)).qualified_name,
        "App\\Model\\Entity::save"
    );
    assert_eq!(
        single(&fx.goto(US, "$user->touch()", 0, 7)).qualified_name,
        "App\\Model\\Timestamps::touch"
    );
    assert_eq!(
        single(&fx.goto("src/Model/User.php", "$this->getId()", 0, 7)).qualified_name,
        "App\\Model\\Entity::getId"
    );
    assert_eq!(
        single(&fx.goto("src/Model/Admin.php", "parent::getName()", 0, 8)).qualified_name,
        "App\\Model\\User::getName"
    );
}

#[test]
fn critere_05_classes_use_alias_static() {
    let fx = fixture();
    let d = single(&fx.goto(US, "use App\\Model\\User;", 0, 15));
    assert_eq!(
        (d.qualified_name.as_str(), d.kind),
        ("App\\Model\\User", SymbolKind::Class)
    );
    // `new Boss` : la classe Admin ; le constructeur (hérité de User) est proposé.
    let r = fx.goto(US, "new Boss('b')", 0, 5);
    let d = single(&r);
    assert_eq!(d.qualified_name, "App\\Model\\Admin");
    let Navigation::Targets(t) = &r.navigation else {
        unreachable!()
    };
    assert_eq!(
        t.constructor.as_ref().map(|c| c.qualified_name.as_str()),
        Some("App\\Model\\User::__construct")
    );
    let d = single(&fx.goto("src/Model/User.php", "new static($name)", 0, 5));
    assert_eq!(d.qualified_name, "App\\Model\\User");
}

#[test]
fn critere_06_appels_statiques_constantes_self_static() {
    let fx = fixture();
    assert_eq!(
        single(&fx.goto(US, "User::create('root')", 0, 7)).qualified_name,
        "App\\Model\\User::create"
    );
    let d = single(&fx.goto(US, "User::ROLE_ADMIN", 0, 7));
    assert_eq!(
        (d.qualified_name.as_str(), d.kind),
        ("App\\Model\\User::ROLE_ADMIN", SymbolKind::ClassConstant)
    );
    assert_eq!(
        single(&fx.goto(US, "self::check()", 0, 7)).qualified_name,
        "App\\Service\\UserService::check"
    );
    assert_eq!(
        single(&fx.goto(US, "static::check()", 0, 9)).qualified_name,
        "App\\Service\\UserService::check"
    );
}

#[test]
fn critere_07_fonctions_et_constantes_globales() {
    let fx = fixture();
    let d = single(&fx.goto(US, "format_name($user", 0, 2));
    assert_eq!(
        (d.qualified_name.as_str(), d.relative.as_str()),
        ("App\\format_name", "src/functions.php")
    );
    let d = single(&fx.goto(US, "log(VERSION)", 0, 5));
    assert_eq!(
        (d.qualified_name.as_str(), d.relative.as_str(), d.kind),
        ("App\\VERSION", "src/functions.php", SymbolKind::Constant)
    );
    let d = single(&fx.goto(US, "legacy_helper();", 0, 3));
    assert_eq!(
        (d.qualified_name.as_str(), d.relative.as_str()),
        ("legacy_helper", "src/global.php")
    );
}

#[test]
fn critere_08_methode_d_une_bibliotheque() {
    let fx = fixture();
    let d = single(&fx.goto(US, "->log(VERSION)", 0, 3));
    assert_eq!(d.qualified_name, "Acme\\Logger::log");
    assert_eq!(d.relative, "vendor/acme/logger/src/Logger.php");
    assert!(d.library);
}

#[test]
fn critere_09_variable_locale() {
    let fx = fixture();
    let d = single(&fx.goto(US, "strlen($label)", 0, 9));
    assert_eq!(d.kind, SymbolKind::Variable);
    assert_eq!(d.line_text, "$label = 'getName';");
}

#[test]
fn critere_10_fonction_de_php() {
    let fx = fixture();
    let r = fx.goto(US, "strlen($label)", 0, 2);
    assert_eq!(
        r.navigation,
        Navigation::Nothing(NoTarget::NotInProject {
            name: "strlen".into()
        })
    );
    assert_eq!(r.word, None);
}

#[test]
fn critere_11_commentaire_et_chaine() {
    let fx = fixture();
    let r = fx.goto(US, "getName dans un commentaire", 0, 2);
    assert_eq!(r.navigation, Navigation::Nothing(NoTarget::NotASymbol));
    let r = fx.goto(US, "'getName'", 0, 3);
    assert_eq!(r.navigation, Navigation::Nothing(NoTarget::NotASymbol));
}

// --- PHP, usages ------------------------------------------------------------------

fn counts(usages: &[Usage]) -> (usize, usize) {
    let sure = usages
        .iter()
        .filter(|u| u.certainty == UsageCertainty::Sure)
        .count();
    (sure, usages.len() - sure)
}

#[test]
fn critere_12_usages_de_user_get_name() {
    let fx = fixture();
    let d = fx.declaration("src/Model/User.php", "function getName", 0, 10);
    assert_eq!(d.qualified_name, "App\\Model\\User::getName");
    let (usages, summary) = fx.usages(&d, &UnsavedFiles::new());
    assert_eq!(counts(&usages), (5, 1), "{usages:#?}");
    assert_eq!((summary.sure, summary.possible), (5, 1));
    assert!(summary.dynamic_not_searched);
    let sure: Vec<(&str, &str)> = usages
        .iter()
        .filter(|u| u.certainty == UsageCertainty::Sure)
        .map(|u| (u.relative.as_str(), u.line_text.as_str()))
        .collect();
    assert_eq!(
        sure,
        vec![
            (
                "src/Model/Admin.php",
                "return 'admin:' . parent::getName();"
            ),
            (
                "src/Model/User.php",
                "return ['id' => $this->getId(), 'name' => $this->getName()];"
            ),
            (
                "src/Service/UserService.php",
                "return format_name($user->getName())"
            ),
            ("src/Service/UserService.php", ". $this->current->getName()"),
            ("src/Service/UserService.php", ". $fresh->getName()"),
        ]
    );
    let possible: Vec<&str> = usages
        .iter()
        .filter(|u| u.certainty == UsageCertainty::Possible)
        .map(|u| u.line_text.as_str())
        .collect();
    assert_eq!(possible, vec![". $other->getName();"]);
}

#[test]
fn critere_13_usages_d_admin_et_d_order() {
    let fx = fixture();
    let d = fx.declaration("src/Model/Admin.php", "function getName", 0, 10);
    let (usages, _) = fx.usages(&d, &UnsavedFiles::new());
    assert_eq!(counts(&usages), (1, 1), "{usages:#?}");
    assert!(
        usages[0].line_text.contains("$boss->getName()")
            || usages[1].line_text.contains("$boss->getName()")
    );
    let d = fx.declaration("src/Legacy/Order.php", "function getName", 0, 10);
    let (usages, summary) = fx.usages(&d, &UnsavedFiles::new());
    assert_eq!(counts(&usages), (0, 1), "{usages:#?}");
    assert_eq!((summary.sure, summary.possible), (0, 1));
}

#[test]
fn critere_14_usages_de_la_classe_user() {
    let fx = fixture();
    let d = fx.declaration("src/Model/User.php", "class User", 0, 7);
    assert_eq!(d.kind, SymbolKind::Class);
    let (usages, summary) = fx.usages(&d, &UnsavedFiles::new());
    assert_eq!(counts(&usages), (7, 0), "{usages:#?}");
    assert_eq!(summary.files_with_sure, 2);
    let files: Vec<&str> = usages.iter().map(|u| u.relative.as_str()).collect();
    assert_eq!(
        files
            .iter()
            .filter(|f| **f == "src/Model/Admin.php")
            .count(),
        1
    );
    assert_eq!(files.iter().filter(|f| **f == US).count(), 6);
    assert!(!summary.dynamic_not_searched);
}

#[test]
fn critere_15_fonction_methode_statique_methode_heritee() {
    let fx = fixture();
    let d = fx.declaration("src/functions.php", "function format_name", 0, 10);
    let (usages, _) = fx.usages(&d, &UnsavedFiles::new());
    assert_eq!(counts(&usages), (2, 0), "{usages:#?}");
    let d = fx.declaration(US, "function check", 0, 10);
    let (usages, _) = fx.usages(&d, &UnsavedFiles::new());
    assert_eq!(counts(&usages), (2, 0), "{usages:#?}");
    let d = fx.declaration("src/Model/Entity.php", "function save", 0, 10);
    let (usages, summary) = fx.usages(&d, &UnsavedFiles::new());
    assert_eq!(counts(&usages), (1, 0), "{usages:#?}");
    assert_eq!(summary.possible, 0);
}

#[test]
fn critere_16_usage_d_une_methode_de_bibliotheque() {
    let fx = fixture();
    let d = fx.declaration("vendor/acme/logger/src/Logger.php", "function log", 0, 10);
    assert!(d.library);
    let (usages, _) = fx.usages(&d, &UnsavedFiles::new());
    assert_eq!(counts(&usages), (1, 0), "{usages:#?}");
    assert_eq!(usages[0].line_text, "$this->logger->log(VERSION);");
}

#[test]
fn critere_17_onglet_non_enregistre() {
    let fx = fixture();
    let d = fx.declaration("src/Model/User.php", "function getName", 0, 10);
    let text = fx.text(US).replace(
        "        strlen($label);\n",
        "        strlen($label);\n        $this->current->getName();\n",
    );
    let mut unsaved = UnsavedFiles::new();
    unsaved.insert(fx.path(US), Arc::from(text.as_str()));
    let (usages, _) = fx.usages(&d, &unsaved);
    assert_eq!(counts(&usages), (6, 1), "{usages:#?}");
    assert!(usages.iter().any(|u| u.unsaved));
}

// --- JavaScript / TypeScript ---------------------------------------------------------

const APP: &str = "web/js/app.js";

#[test]
fn critere_18_imports_classes_membres_locaux() {
    let fx = fixture();
    let d = single(&fx.goto(APP, "fd(new Date())", 0, 1));
    assert_eq!(
        (d.name.as_str(), d.relative.as_str()),
        ("formatDate", "web/js/util.js")
    );
    let d = single(&fx.goto(APP, "formatDate(new Date())", 0, 3));
    assert_eq!(
        (d.name.as_str(), d.relative.as_str()),
        ("formatDate", "web/js/util.js")
    );
    let d = single(&fx.goto(APP, "new Api()", 0, 5));
    assert_eq!(
        (d.name.as_str(), d.kind, d.relative.as_str()),
        ("Api", SymbolKind::Class, "web/js/util.js")
    );
    let d = single(&fx.goto(APP, "api.get('/x')", 0, 5));
    assert_eq!(
        (d.qualified_name.as_str(), d.relative.as_str()),
        ("Api.get", "web/js/util.js")
    );
    let d = single(&fx.goto(APP, "local();", 0, 2));
    assert_eq!((d.name.as_str(), d.relative.as_str()), ("local", APP));
    let d = single(&fx.goto("web/js/types.ts", "formatDate(new Date())", 0, 3));
    assert_eq!(
        (d.name.as_str(), d.relative.as_str()),
        ("formatDate", "web/js/util.js")
    );
}

#[test]
fn critere_19_chemin_d_import_et_global_absent() {
    let fx = fixture();
    let r = fx.goto(APP, "'./util'", 0, 3);
    assert_eq!(
        r.navigation,
        Navigation::OpenFile(fx.path("web/js/util.js"))
    );
    let r = fx.goto("web/js/util.js", "fetch(url)", 0, 2);
    assert_eq!(
        r.navigation,
        Navigation::Nothing(NoTarget::NotInProject {
            name: "fetch".into()
        })
    );
}

#[test]
fn critere_20_usages_de_format_date() {
    let fx = fixture();
    let d = fx.declaration("web/js/util.js", "function formatDate", 0, 10);
    let (usages, summary) = fx.usages(&d, &UnsavedFiles::new());
    assert_eq!(counts(&usages), (6, 0), "{usages:#?}");
    assert_eq!(summary.files_with_sure, 2);
    assert_eq!(usages.iter().filter(|u| u.relative == APP).count(), 4);
    assert_eq!(
        usages
            .iter()
            .filter(|u| u.relative == "web/js/types.ts")
            .count(),
        2
    );
}

// --- Robustesse ---------------------------------------------------------------------

#[test]
fn critere_21_soulignement_seulement_avec_une_cible() {
    let fx = fixture();
    let r = fx.goto(US, "$user->getName()", 0, 9);
    assert!(r.word.is_some());
    let r = fx.goto(US, "strlen($label)", 0, 1);
    assert!(r.word.is_none());
}

#[test]
fn critere_23_fichier_casse_partout() {
    let fx = fixture();
    let text = fx.text("src/Broken.php");
    let path = fx.path("src/Broken.php");
    for (line, l) in text.lines().enumerate() {
        for column in 0..=l.chars().count() + 2 {
            let request = NavigationRequest {
                path: path.clone(),
                text: Arc::from(text.as_str()),
                position: TextPosition::new(line as u32, column as u32),
            };
            let _ = navigate(&fx.symbols, &request, &UnsavedFiles::new());
        }
    }
    // Position hors du texte.
    let request = NavigationRequest {
        path,
        text: Arc::from(text.as_str()),
        position: TextPosition::new(50, 50),
    };
    let _ = navigate(&fx.symbols, &request, &UnsavedFiles::new());
    critere_01_methode_d_un_parametre_type();
}

#[test]
fn critere_24_modification_hors_de_l_ide() {
    let mut fx = fixture();
    let order = fx.path("src/Legacy/Order.php");
    let text = fx.text("src/Legacy/Order.php").replace("getName", "title");
    // Date de modification différente garantie.
    std::thread::sleep(Duration::from_millis(20));
    std::fs::write(&order, text).unwrap();
    let cancel = CancelToken::new();
    fx.files
        .update(std::slice::from_ref(&order), &cancel)
        .unwrap();
    let changed = fx
        .symbols
        .update(&fx.files, &[order], &UnsavedFiles::new(), &cancel)
        .unwrap();
    assert_eq!(changed, 1);
    let (names, _) = targets(&fx.goto(US, "$other->getName()", 0, 8));
    assert_eq!(
        names,
        vec!["App\\Model\\Admin::getName", "App\\Model\\User::getName"]
    );
}

#[test]
fn critere_25_cache_relu_a_la_reouverture() {
    let fx = fixture();
    let cache = tempfile::tempdir().unwrap();
    let cancel = CancelToken::new();
    let (first, s1) =
        SymbolIndex::build(&fx.files, Some(cache.path()), &UnsavedFiles::new(), &cancel).unwrap();
    assert!(!s1.cache_loaded && s1.cache_written);
    assert_eq!(s1.parsed, s1.files);
    let started = Instant::now();
    let (second, s2) =
        SymbolIndex::build(&fx.files, Some(cache.path()), &UnsavedFiles::new(), &cancel).unwrap();
    assert!(started.elapsed() < Duration::from_secs(3));
    assert!(s2.cache_loaded);
    assert_eq!((s2.parsed, s2.from_cache), (0, s1.files));
    assert_eq!(second.declaration_count(), first.declaration_count());
    // Cache abîmé : jeté et reconstruit, sans erreur.
    let file = cache.path().join(julie_index::SYMBOL_CACHE_FILE);
    let mut bytes = std::fs::read(&file).unwrap();
    let middle = bytes.len() / 2;
    bytes[middle] ^= 0xFF;
    std::fs::write(&file, &bytes).unwrap();
    let (_, s3) =
        SymbolIndex::build(&fx.files, Some(cache.path()), &UnsavedFiles::new(), &cancel).unwrap();
    assert!(!s3.cache_loaded);
    assert_eq!(s3.parsed, s1.files);
    // Un fichier changé seulement est relu.
    std::thread::sleep(Duration::from_millis(20));
    std::fs::write(
        fx.path("src/global.php"),
        "<?php\nfunction legacy_helper2(): void {}\n",
    )
    .unwrap();
    let (_, s4) =
        SymbolIndex::build(&fx.files, Some(cache.path()), &UnsavedFiles::new(), &cancel).unwrap();
    assert_eq!((s4.parsed, s4.from_cache), (1, s1.files - 1));
}

#[test]
fn le_projet_d_essai_compte_ses_fichiers() {
    let fx = fixture();
    // 11 fichiers PHP (dont vendor) et 4 JS/TS ; Broken.php indexé au mieux.
    assert_eq!(fx.symbols.file_count(), 15);
    assert!(fx.symbols.contains("src/Broken.php"));
    let _ = &fx.root;
}

// --- Par le service (bus) ---------------------------------------------------------------

fn drain_index(bus: &Bus) -> Vec<IndexEvent> {
    bus.drain()
        .into_iter()
        .filter_map(|e| match e {
            Event::Domain(e) if e.domain == Domain::Index => e.get::<IndexEvent>().cloned(),
            _ => None,
        })
        .collect()
}

/// Attend `SymbolsReady` (publié après le passage de l'index à « prêt »).
fn wait_ready(bus: &Bus) -> julie_index::SymbolBuildSummary {
    let started = Instant::now();
    loop {
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "SymbolsReady absent"
        );
        if let Some(Event::Domain(e)) = bus.recv_timeout(Duration::from_millis(100)) {
            if let Some(IndexEvent::SymbolsReady(s)) = e.get::<IndexEvent>() {
                return *s;
            }
        }
    }
}

#[test]
fn service_navigation_usages_onglet_et_cache_dans_le_projet() {
    let fx = fixture();
    let bus = Bus::new();
    let tasks = Tasks::new(bus.publisher());
    let project = Project::open(&fx.root).unwrap();
    let service = IndexService::new(tasks.clone(), project.clone(), IndexSettings::default());
    service.scan().join();
    let ready = wait_ready(&bus);
    assert_eq!(ready.files, 15);
    assert!(ready.cache_written);
    // Le cache est dans le projet ouvert, et `.juliestorm/` a son `.gitignore`.
    assert!(fx
        .root
        .join(".juliestorm/index")
        .join(julie_index::SYMBOL_CACHE_FILE)
        .is_file());
    assert!(fx.root.join(".juliestorm/.gitignore").is_file());

    // Aller à la déclaration.
    service
        .navigate(fx.request(US, "$user->getName()", 0, 7))
        .join();
    let events = drain_index(&bus);
    let [IndexEvent::Navigation(result)] = &events[..] else {
        panic!("{events:?}")
    };
    assert_eq!(single(result).qualified_name, "App\\Model\\User::getName");
    assert_eq!(result.indexing, None);

    // Usages en flux, puis le bilan.
    let d = fx.declaration("src/Model/User.php", "function getName", 0, 10);
    service.find_usages(UsagesQuery::new(d.clone())).join();
    let events = drain_index(&bus);
    let mut found = Vec::new();
    let mut summary = None;
    for e in events {
        match e {
            IndexEvent::Usages(batch) => found.extend(batch),
            IndexEvent::UsagesFinished(s) => summary = Some(s),
            other => panic!("{other:?}"),
        }
    }
    assert_eq!(counts(&found), (5, 1));
    assert_eq!(summary.unwrap().sure, 5);

    // Onglet non enregistré (critère 17), puis retour au disque.
    let text = fx.text(US).replace(
        "        strlen($label);\n",
        "        strlen($label);\n        $this->current->getName();\n",
    );
    service
        .set_unsaved(fx.path(US), Some(Arc::from(text.as_str())))
        .join();
    service.find_usages(UsagesQuery::new(d.clone())).join();
    let sure = drain_index(&bus)
        .iter()
        .filter_map(|e| match e {
            IndexEvent::UsagesFinished(s) => Some(s.sure),
            _ => None,
        })
        .next();
    assert_eq!(sure, Some(6));
    service.set_unsaved(fx.path(US), None).join();
    service.find_usages(UsagesQuery::new(d)).join();
    let sure = drain_index(&bus)
        .iter()
        .filter_map(|e| match e {
            IndexEvent::UsagesFinished(s) => Some(s.sure),
            _ => None,
        })
        .next();
    assert_eq!(sure, Some(5));

    // Réouverture : le cache sert (critère 25).
    let bus2 = Bus::new();
    let service2 = IndexService::new(
        Tasks::new(bus2.publisher()),
        project,
        IndexSettings::default(),
    );
    let started = Instant::now();
    service2.scan().join();
    let ready = wait_ready(&bus2);
    assert!(started.elapsed() < Duration::from_secs(3));
    assert!(ready.cache_loaded);
    assert_eq!(ready.parsed, 0);
}
