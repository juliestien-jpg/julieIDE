//! Compare la recherche dans les fichiers à `git grep` sur un clone jetable de
//! `labmate_paper` (l'original n'est que lu par `git clone`).
//!
//! Le test est sauté (avec un message) si le dépôt source n'existe pas.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use julie_core::CancelToken;
use julie_index::{
    files_to_search, search_files, FileIndex, FileMask, IndexSettings, Matcher, SearchOptions,
    TextQuery, UnsavedFiles,
};

const SOURCE: &str = r"C:\Program Files (x86)\AgileBio\LabCollector\www\labmate_paper";

fn git(dir: &Path, args: &[&str]) -> Vec<u8> {
    let output = Command::new("git")
        .args(["-c", "core.quotepath=false", "-c", "color.ui=false"])
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git introuvable");
    output.stdout
}

fn clone() -> Option<(tempfile::TempDir, PathBuf)> {
    if !Path::new(SOURCE).join(".git").exists() {
        eprintln!("dépôt {SOURCE} absent : comparaison sautée");
        return None;
    }
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("labmate_paper");
    let status = Command::new("git")
        .args(["clone", "--quiet", "--no-hardlinks", SOURCE])
        .arg(&target)
        .status()
        .unwrap();
    assert!(status.success());
    Some((dir, target))
}

/// Nombre par fichier, lu dans une sortie `chemin:valeur` de git grep.
fn per_file(output: &[u8], count_lines: bool) -> BTreeMap<String, usize> {
    let text = String::from_utf8_lossy(output);
    let mut map = BTreeMap::new();
    for line in text.lines() {
        // `-z` : chemin NUL valeur.
        let Some((path, value)) = line.split_once('\0') else {
            continue;
        };
        if count_lines {
            map.insert(path.to_string(), value.parse().unwrap());
        } else {
            *map.entry(path.to_string()).or_insert(0) += 1;
        }
    }
    map
}

struct Case {
    label: &'static str,
    query: TextQuery,
    git_args: Vec<&'static str>,
}

fn case(
    label: &'static str,
    pattern: &str,
    f: impl FnOnce(&mut TextQuery),
    git_args: &[&'static str],
) -> Case {
    let mut query = TextQuery::new(pattern);
    query.max_matches = usize::MAX / 2;
    f(&mut query);
    Case {
        label,
        query,
        git_args: git_args.to_vec(),
    }
}

#[test]
fn text_search_agrees_with_git_grep() {
    let Some((_dir, root)) = clone() else {
        return;
    };
    let settings = IndexSettings::default();
    let index = FileIndex::scan(&root, settings.clone(), &CancelToken::new()).unwrap();

    // Les fichiers : l'index contre `git ls-files` (clone neuf, rien de non suivi).
    let tracked: Vec<String> = String::from_utf8(git(&root, &["ls-files", "-z"]))
        .unwrap()
        .split('\0')
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect();
    let mut indexed: Vec<String> = index
        .entries()
        .iter()
        .map(|e| e.relative().to_string())
        .collect();
    indexed.sort();
    let mut tracked_sorted = tracked.clone();
    tracked_sorted.sort();
    let only_git: Vec<_> = tracked_sorted
        .iter()
        .filter(|p| !index.contains(p))
        .collect();
    let only_index: Vec<_> = indexed
        .iter()
        .filter(|p| tracked_sorted.binary_search(p).is_err())
        .collect();
    eprintln!(
        "fichiers : index {} / git {}",
        indexed.len(),
        tracked_sorted.len()
    );
    eprintln!("  suivis par Git mais pas indexés : {only_git:?}");
    eprintln!("  indexés mais pas suivis : {only_index:?}");

    let cases = vec![
        case(
            "littéral, sans casse",
            "function",
            |_| {},
            &["-F", "-i", "function"],
        ),
        case(
            "littéral, casse",
            "Math",
            |q| q.case_sensitive = true,
            &["-F", "Math"],
        ),
        case(
            "accent, casse",
            "é",
            |q| q.case_sensitive = true,
            &["-F", "é"],
        ),
        case("accent, sans casse", "été", |_| {}, &["-F", "-i", "été"]),
        case(
            "mot entier",
            "const",
            |q| {
                q.whole_word = true;
                q.case_sensitive = true
            },
            &["-F", "-w", "const"],
        ),
        case(
            "regex",
            r"import\s+\w+\s+from",
            |q| {
                q.regex = true;
                q.case_sensitive = true
            },
            &["-P", r"import\s+\w+\s+from"],
        ),
        case(
            "regex sans casse, fin de ligne",
            r";\s*$",
            |q| q.regex = true,
            &["-P", "-i", r";\s*$"],
        ),
        case(
            "masque *.js",
            "export",
            |q| {
                q.case_sensitive = true;
                q.file_mask = "*.js".into()
            },
            &["-F", "export", "--", "*.js"],
        ),
        case(
            "mot entier sans casse",
            "data",
            |q| q.whole_word = true,
            &["-F", "-w", "-i", "data"],
        ),
    ];

    let mut failures = Vec::new();
    for case in cases {
        let matcher = Matcher::new(&case.query).unwrap();
        let mask = FileMask::parse(&case.query.file_mask).unwrap();
        let files = files_to_search(&index, &case.query, &mask, &UnsavedFiles::new()).unwrap();
        let options = SearchOptions::new(&case.query, &settings);
        let mut ours_lines = BTreeMap::new();
        let mut ours_matches = BTreeMap::new();
        let summary = search_files(
            &files,
            &matcher,
            &options,
            &CancelToken::new(),
            &mut |batch| {
                for file in batch {
                    ours_lines.insert(file.relative.clone(), file.lines.len());
                    ours_matches.insert(file.relative.clone(), file.match_count());
                }
            },
        )
        .unwrap();

        let (pattern_args, pathspec) = match case.git_args.iter().position(|a| *a == "--") {
            Some(i) => (&case.git_args[..i], &case.git_args[i..]),
            None => (&case.git_args[..], &[][..]),
        };
        let mut count_args = vec!["grep", "-I", "-z", "-c"];
        count_args.extend_from_slice(pattern_args);
        count_args.extend_from_slice(pathspec);
        let git_lines = per_file(&git(&root, &count_args), true);
        let mut only_args = vec!["grep", "-I", "-z", "-o"];
        only_args.extend_from_slice(pattern_args);
        only_args.extend_from_slice(pathspec);
        let git_matches = per_file(&git(&root, &only_args), false);

        let git_total_lines: usize = git_lines.values().sum();
        let git_total_matches: usize = git_matches.values().sum();
        eprintln!(
            "[{}] nous : {} occurrences, {} lignes, {} fichiers | git grep : {} occurrences (-o), {} lignes (-c), {} fichiers",
            case.label,
            summary.matches,
            summary.lines,
            summary.files_with_matches,
            git_total_matches,
            git_total_lines,
            git_lines.len()
        );
        for (path, &count) in &git_lines {
            let ours = ours_lines.get(path).copied().unwrap_or(0);
            if ours != count && index.contains(path) {
                failures.push(format!(
                    "[{}] {path} : lignes {ours} contre {count} (git)",
                    case.label
                ));
            }
        }
        for (path, &count) in &ours_lines {
            if !git_lines.contains_key(path) {
                failures.push(format!(
                    "[{}] {path} : {count} lignes, rien chez git",
                    case.label
                ));
            }
        }
        for (path, &count) in &git_matches {
            let ours = ours_matches.get(path).copied().unwrap_or(0);
            if ours != count && index.contains(path) {
                eprintln!("  occurrences {path} : {ours} contre {count} (git -o)");
            }
        }
    }
    assert!(
        failures.is_empty(),
        "écarts avec git grep :\n{}",
        failures.join("\n")
    );
}
