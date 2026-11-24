//! Mesures de performance, en lecture seule, sur un vrai projet.
//!
//! ```text
//! cargo run -p julie-index --release --example mesure -- "C:\chemin\du\projet"
//! ```
//!
//! N'écrit rien dans le projet (aucun cache) : parcours, aller au fichier,
//! recherche de texte, tout en mémoire.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use julie_core::{Bus, CancelToken, Domain, Event, Project, TaskState, Tasks};
use julie_index::{
    files_to_search, search_files, FileIndex, FileMask, IndexEvent, IndexService, IndexSettings,
    Matcher, SearchOptions, TextQuery, UnsavedFiles,
};

fn ms(d: Duration) -> String {
    format!("{:.1} ms", d.as_secs_f64() * 1000.0)
}

fn main() {
    let root = PathBuf::from(std::env::args().nth(1).expect("chemin du projet"));
    let settings = IndexSettings::default();
    let cancel = CancelToken::new();

    // 1. Indexation initiale, synchrone (deux fois : disque froid puis chaud).
    for run in 1..=2 {
        let started = Instant::now();
        let index = FileIndex::scan(&root, settings.clone(), &cancel).unwrap();
        let libraries = index.entries().iter().filter(|e| e.is_library()).count();
        println!(
            "indexation #{run} : {} fichiers (dont {libraries} de bibliothèques) en {}",
            index.len(),
            ms(started.elapsed())
        );
    }

    // 2. Par le service : délai de la première réponse d'« aller au fichier »
    //    pendant l'indexation, et progression.
    let bus = Bus::new();
    let tasks = Tasks::new(bus.publisher());
    let service = IndexService::new(tasks, Project::open(&root).unwrap(), settings.clone());
    let started = Instant::now();
    let scan = service.scan();
    let find = service.find_files("usct", vec![]);
    find.join();
    println!(
        "service : réponse d'« aller au fichier » pendant l'indexation après {}",
        ms(started.elapsed())
    );
    scan.join();
    let scan_time = started.elapsed();
    let mut progress_events = 0;
    let mut partial = None;
    for event in bus.drain() {
        match event {
            Event::Task(t)
                if t.domain == Domain::Index && matches!(t.state, TaskState::Progress { .. }) =>
            {
                progress_events += 1
            }
            Event::Domain(e) => {
                if let Some(IndexEvent::Files(r)) = e.get::<IndexEvent>() {
                    partial = Some((r.total, r.index_complete));
                }
            }
            _ => {}
        }
    }
    println!(
        "service : indexation finie en {} ; {progress_events} événements de progression ; réponse partielle {:?}",
        ms(scan_time),
        partial
    );

    // 3. Aller au fichier sur l'index complet.
    let index = FileIndex::scan(&root, settings.clone(), &cancel).unwrap();
    let recent: Vec<PathBuf> = index
        .entries()
        .iter()
        .take(20)
        .map(|e| index.absolute(e.relative()))
        .collect();
    for query in [
        "",
        "u",
        "us",
        "usct",
        "contr",
        "index.php:12",
        "class/user",
        "ajax upload",
        "zzzzqx",
    ] {
        let mut times = Vec::new();
        let mut last = None;
        for _ in 0..7 {
            let started = Instant::now();
            let results = index.find_files(query, &recent, 100, &cancel).unwrap();
            times.push(started.elapsed());
            last = Some(results);
        }
        times.sort();
        let results = last.unwrap();
        println!(
            "aller au fichier {query:>14?} : premier {} médiane {} max {} — {} résultats, premier : {:?}",
            ms(times[0]),
            ms(times[3]),
            ms(times[6]),
            results.total,
            results.matches.first().map(|m| m.relative.as_str())
        );
    }

    // 3 bis. Même chose sur un index de plus de 50 000 fichiers (les chemins du
    //        projet recopiés sous trois préfixes, en mémoire seulement).
    let big = FileIndex::from_relative_paths(
        root.clone(),
        settings.clone(),
        ["", "copie1/", "copie2/"].iter().flat_map(|prefix| {
            index
                .entries()
                .iter()
                .map(move |e| format!("{prefix}{}", e.relative()))
        }),
    );
    for query in ["u", "usct", "contr", "class/user"] {
        let mut times = Vec::new();
        for _ in 0..7 {
            let started = Instant::now();
            big.find_files(query, &recent, 100, &cancel).unwrap();
            times.push(started.elapsed());
        }
        times.sort();
        println!(
            "aller au fichier ({} fichiers) {query:>12?} : médiane {} max {}",
            big.len(),
            ms(times[3]),
            ms(times[6])
        );
    }

    // 4. Recherche de texte.
    let searches: Vec<(&str, TextQuery)> = vec![
        ("littéral « function »", TextQuery::new("function")),
        ("regex \\$\\w+Id\\b", {
            let mut q = TextQuery::new(r"\$\w+Id\b");
            q.regex = true;
            q.case_sensitive = true;
            q.max_matches = usize::MAX / 2;
            q
        }),
        ("mot entier « class » *.php", {
            let mut q = TextQuery::new("class");
            q.whole_word = true;
            q.file_mask = "*.php".into();
            q.max_matches = usize::MAX / 2;
            q
        }),
        (
            "rare « juliestorm_absent »",
            TextQuery::new("juliestorm_absent"),
        ),
    ];
    for (label, query) in searches {
        for run in 1..=2 {
            let started = Instant::now();
            let matcher = Matcher::new(&query).unwrap();
            let mask = FileMask::parse(&query.file_mask).unwrap();
            let files = files_to_search(&index, &query, &mask, &UnsavedFiles::new()).unwrap();
            let options = SearchOptions::new(&query, &settings);
            let mut first_batch = None;
            let summary = search_files(&files, &matcher, &options, &cancel, &mut |_| {
                first_batch.get_or_insert(started.elapsed());
            })
            .unwrap();
            println!(
                "recherche {label} #{run} : {} occurrences dans {} fichiers ({} lus, {} binaires, {} trop gros, {} illisibles, limite {}) ; premier paquet {} ; total {}",
                summary.matches,
                summary.files_with_matches,
                summary.files_searched,
                summary.skipped_binary,
                summary.skipped_too_large,
                summary.unreadable,
                summary.limit_reached,
                first_batch.map_or("—".into(), ms),
                ms(started.elapsed())
            );
        }
    }
}
