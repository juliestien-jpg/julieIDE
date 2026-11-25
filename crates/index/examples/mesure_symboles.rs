//! Mesures de l'index des symboles, en lecture seule, sur un vrai projet.
//!
//! ```text
//! cargo run -p julie-index --release --example mesure_symboles -- "C:\chemin\du\projet"
//! ```
//!
//! N'écrit **rien** dans le projet : le cache va dans le dossier temporaire
//! (`%TEMP%\juliestorm-mesure-symboles`), effacé au départ.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use julie_core::{CancelToken, TextPosition};
use julie_index::{
    find_usages, navigate, Declaration, FileIndex, IndexSettings, Navigation, NavigationRequest,
    SymbolIndex, UnsavedFiles, UsagesQuery,
};

fn ms(d: Duration) -> String {
    format!("{:.1} ms", d.as_secs_f64() * 1000.0)
}

fn position_of(text: &str, byte: usize) -> TextPosition {
    let before = &text[..byte];
    let line = before.matches('\n').count() as u32;
    let start = before.rfind('\n').map_or(0, |i| i + 1);
    TextPosition::new(line, text[start..byte].chars().count() as u32)
}

fn main() {
    let root = PathBuf::from(std::env::args().nth(1).expect("chemin du projet"));
    let cache = std::env::temp_dir().join("juliestorm-mesure-symboles");
    let _ = std::fs::remove_dir_all(&cache);
    let settings = IndexSettings::default();
    let cancel = CancelToken::new();
    let unsaved = UnsavedFiles::new();

    let started = Instant::now();
    let files = FileIndex::scan(&root, settings.clone(), &cancel).unwrap();
    println!("fichiers : {} en {}", files.len(), ms(started.elapsed()));

    let (index, s) = SymbolIndex::build(&files, Some(&cache), &unsaved, &cancel).unwrap();
    println!(
        "symboles sans cache : {} fichiers, {} déclarations, analysés {}, trop gros {}, binaires {}, illisibles {}, en {} (cache écrit : {})",
        s.files, s.declarations, s.parsed, s.skipped_too_large, s.skipped_binary, s.unreadable,
        ms(s.elapsed), s.cache_written
    );
    println!(
        "mémoire estimée de l'index : {:.1} Mio ; cache : {:.1} Mio",
        index.approximate_memory() as f64 / 1048576.0,
        std::fs::metadata(cache.join(julie_index::SYMBOL_CACHE_FILE)).map_or(0, |m| m.len()) as f64
            / 1048576.0
    );
    let (_, s2) = SymbolIndex::build(&files, Some(&cache), &unsaved, &cancel).unwrap();
    println!(
        "symboles avec cache : relu {}, {} du cache, {} analysés, en {}",
        s2.cache_loaded,
        s2.from_cache,
        s2.parsed,
        ms(s2.elapsed)
    );

    // Choix des symboles : la classe la plus instanciée (`new X(`).
    let php: Vec<_> = files
        .entries()
        .iter()
        .filter(|e| e.name().ends_with(".php") && !e.is_library())
        .collect();
    let new_re = regex::Regex::new(r"new ([A-Z][A-Za-z0-9_]+)\(").unwrap();
    let mut counts: HashMap<String, usize> = HashMap::new();
    let mut texts: HashMap<String, String> = HashMap::new();
    for e in &php {
        if let Ok(bytes) = std::fs::read(files.absolute(e.relative())) {
            let text = julie_core::file::decode(bytes).text;
            for c in new_re.captures_iter(&text) {
                *counts.entry(c[1].to_string()).or_default() += 1;
            }
            texts.insert(e.relative().to_string(), text);
        }
    }
    let mut ranked: Vec<_> = counts.into_iter().collect();
    ranked.sort_by_key(|r| std::cmp::Reverse(r.1));

    let decl_at = |needle: &regex::Regex, group: usize| -> Option<Declaration> {
        let mut names: Vec<&String> = texts.keys().collect();
        names.sort();
        for rel in names {
            let text = &texts[rel];
            if let Some(c) = needle.captures(text) {
                let m = c.get(group)?;
                let request = NavigationRequest {
                    path: files.absolute(rel),
                    text: Arc::from(text.as_str()),
                    position: position_of(text, m.start() + 1),
                };
                if let Navigation::Declaration(d) = navigate(&index, &request, &unsaved).navigation
                {
                    return Some(d);
                }
            }
        }
        None
    };

    let mut symbols: Vec<Declaration> = Vec::new();
    for (name, n) in ranked.iter().take(5) {
        let re = regex::Regex::new(&format!(r"\bclass ({name})\b")).unwrap();
        if let Some(d) = decl_at(&re, 1) {
            println!("classe choisie : {} ({n} « new »)", d.qualified_name);
            symbols.push(d);
            break;
        }
    }
    for method in ["save", "getName", "getId", "load"] {
        let re = regex::Regex::new(&format!(r"function ({method})\s*\(")).unwrap();
        if let Some(d) = decl_at(&re, 1) {
            symbols.push(d);
        }
    }

    // Aller à la déclaration : 20 appels `->m(` répartis dans le fichier qui en
    // a le plus, puis dans le plus long fichier PHP du projet.
    let call_re = regex::Regex::new(r"->([A-Za-z_][A-Za-z0-9_]*)\(").unwrap();
    let busiest = texts
        .iter()
        .max_by_key(|(_, t)| call_re.find_iter(t).count())
        .map(|(r, _)| r.clone());
    let longest = texts
        .iter()
        .max_by_key(|(_, t)| t.lines().count())
        .map(|(r, _)| r.clone());
    for rel in [busiest, longest].into_iter().flatten() {
        let text = &texts[&rel];
        let arc: Arc<str> = Arc::from(text.as_str());
        let calls: Vec<usize> = call_re
            .captures_iter(text)
            .map(|c| c.get(1).unwrap().start())
            .collect();
        let step = (calls.len() / 20).max(1);
        let mut times = Vec::new();
        let mut kinds: HashMap<&'static str, usize> = HashMap::new();
        for &at in calls.iter().step_by(step).take(20) {
            let request = NavigationRequest {
                path: files.absolute(&rel),
                text: arc.clone(),
                position: position_of(text, at + 1),
            };
            let started = Instant::now();
            let r = navigate(&index, &request, &unsaved);
            times.push(started.elapsed());
            let k = match &r.navigation {
                Navigation::Targets(t) if t.guess.is_some() => "devinées",
                Navigation::Targets(_) => "sûres",
                Navigation::Nothing(_) => "aucune",
                _ => "autre",
            };
            *kinds.entry(k).or_default() += 1;
        }
        times.sort();
        if !times.is_empty() {
            println!(
                "aller à la déclaration ({} essais dans {}, {} lignes, analyse complète du fichier à chaque essai) : médiane {} max {} ; cibles {:?}",
                times.len(),
                rel,
                text.lines().count(),
                ms(times[times.len() / 2]),
                ms(*times.last().unwrap()),
                kinds
            );
        }
    }
    // Par le service (cache désactivé : rien d'écrit dans le projet) : la
    // première demande analyse le fichier, les suivantes réutilisent l'analyse.
    if let Some(rel) = texts
        .iter()
        .max_by_key(|(_, t)| call_re.find_iter(t).count())
        .map(|(r, _)| r.clone())
    {
        let bus = julie_core::Bus::new();
        let service = julie_index::IndexService::new(
            julie_core::Tasks::new(bus.publisher()),
            julie_core::Project::open(&root).unwrap(),
            settings.clone(),
        )
        .with_symbol_cache(None);
        service.scan().join();
        while service.symbols_progress().is_some() {
            std::thread::sleep(Duration::from_millis(20));
        }
        let text = &texts[&rel];
        let arc: Arc<str> = Arc::from(text.as_str());
        let calls: Vec<usize> = call_re
            .captures_iter(text)
            .map(|c| c.get(1).unwrap().start())
            .collect();
        let step = (calls.len() / 21).max(1);
        let mut times = Vec::new();
        for &at in calls.iter().step_by(step).take(21) {
            let request = NavigationRequest {
                path: files.absolute(&rel),
                text: arc.clone(),
                position: position_of(text, at + 1),
            };
            let started = Instant::now();
            service.navigate(request).join();
            times.push(started.elapsed());
        }
        let first = times.remove(0);
        times.sort();
        println!(
            "service, {} : première demande (analyse) {} ; 20 suivantes (analyse gardée) médiane {} max {}",
            rel,
            ms(first),
            ms(times[times.len() / 2]),
            ms(*times.last().unwrap())
        );
        drop(bus);
    }
    // Usages.
    for symbol in &symbols {
        for run in 1..=2 {
            let started = Instant::now();
            let mut first = None;
            let summary = find_usages(
                &index,
                &UsagesQuery::new(symbol.clone()),
                &unsaved,
                &settings,
                true,
                &cancel,
                &mut |_| {
                    first.get_or_insert(started.elapsed());
                },
            )
            .unwrap();
            println!(
                "usages de {} #{run} : {} sûrs dans {} fichiers, {} possibles ; {} fichiers lus, {} de bibliothèques non lus ; premier paquet {} ; fin {}",
                symbol.qualified_name,
                summary.sure,
                summary.files_with_sure,
                summary.possible,
                summary.files_searched,
                summary.library_files_skipped,
                first.map_or("—".into(), ms),
                ms(summary.elapsed)
            );
        }
    }
    let _ = std::fs::remove_dir_all(&cache);
}
