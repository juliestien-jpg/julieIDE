//! Comparaison à Git : `git merge-file` pour la fusion, `git diff` pour le diff.
//!
//! La règle : **jamais « sans conflit » là où `git merge-file` voit un conflit.**
//! L'inverse est permis (nous sommes plus prudents) et chaque écart du corpus
//! est nommé et expliqué dans `tests/fixtures/diff/merge/cases.txt`.
//!
//! Git est lancé sur des fichiers écrits dans un dossier temporaire, hors de tout
//! dépôt.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use common::{join_lf, mutate, random_lines, Rng};
use julie_core::CancelToken;
use julie_diff::{diff_lines, merge, DiffOptions};

struct GitMerge {
    text: Vec<u8>,
    conflicts: i32,
}

fn git() -> Command {
    let mut command = Command::new("git");
    command
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .env("LC_ALL", "C");
    command
}

fn git_merge_file(dir: &Path, base: &[u8], ours: &[u8], theirs: &[u8]) -> GitMerge {
    fs::write(dir.join("base"), base).unwrap();
    fs::write(dir.join("ours"), ours).unwrap();
    fs::write(dir.join("theirs"), theirs).unwrap();
    let output = git()
        .current_dir(dir)
        .args(["merge-file", "-p", "ours", "base", "theirs"])
        .output()
        .expect("git doit être installé");
    let conflicts = output.status.code().expect("code de sortie");
    assert!(
        (0..=127).contains(&conflicts),
        "git merge-file a échoué : {}",
        String::from_utf8_lossy(&output.stderr)
    );
    GitMerge {
        text: output.stdout,
        conflicts,
    }
}

/// (lignes ajoutées, lignes supprimées) selon `git diff --numstat`.
fn git_numstat(dir: &Path, left: &[u8], right: &[u8]) -> (usize, usize) {
    fs::write(dir.join("left"), left).unwrap();
    fs::write(dir.join("right"), right).unwrap();
    let output = git()
        .current_dir(dir)
        .args([
            "-c",
            "core.autocrlf=false",
            "diff",
            "--no-index",
            "--numstat",
            "--minimal",
            "left",
            "right",
        ])
        .output()
        .expect("git doit être installé");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut fields = stdout.split_whitespace();
    match (fields.next(), fields.next()) {
        (Some(added), Some(removed)) => (added.parse().unwrap(), removed.parse().unwrap()),
        _ => (0, 0),
    }
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/diff/merge")
}

/// Relation entre notre fusion et celle de Git.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Relation {
    /// Même verdict ; et si c'est « propre », même texte à l'octet près.
    Same,
    /// Nous déclarons un conflit, Git fusionne : prudence voulue.
    Stricter,
    /// Écart voulu sur les fins de ligne (voir le cas).
    LineEndings,
}

fn parse_relation(word: &str) -> Relation {
    match word {
        "same" => Relation::Same,
        "stricter" => Relation::Stricter,
        "line-endings" => Relation::LineEndings,
        other => panic!("relation inconnue dans cases.txt : {other}"),
    }
}

#[test]
fn corpus_matches_git_merge_file_or_explains_why_not() {
    let root = fixtures();
    let manifest = fs::read_to_string(root.join("cases.txt")).expect("cases.txt");
    let dir = tempfile::tempdir().unwrap();
    let cancel = CancelToken::new();
    let mut seen = 0;
    for line in manifest.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut fields = line.splitn(3, '|').map(str::trim);
        let name = fields.next().unwrap();
        let declared = parse_relation(fields.next().expect("relation"));
        let why = fields.next().unwrap_or("");
        if declared != Relation::Same {
            assert!(!why.is_empty(), "{name} : un écart doit être expliqué");
        }

        let read = |file: &str| fs::read(root.join(name).join(file)).expect("fichier du corpus");
        let (base, ours, theirs) = (read("base"), read("ours"), read("theirs"));
        let text = |bytes: &[u8]| String::from_utf8(bytes.to_vec()).expect("corpus en UTF-8");
        let mine = merge(&text(&base), &text(&ours), &text(&theirs), &cancel).unwrap();
        let theirs_merge = git_merge_file(dir.path(), &base, &ours, &theirs);

        let actual = match (mine.conflicts == 0, theirs_merge.conflicts == 0) {
            (true, true) if mine.text.as_bytes() == theirs_merge.text.as_slice() => Relation::Same,
            (true, true) => Relation::LineEndings,
            (false, false) => Relation::Same,
            (false, true) => Relation::Stricter,
            (true, false) => {
                assert!(
                    declared == Relation::LineEndings,
                    "{name} : nous fusionnons sans conflit là où Git voit un conflit"
                );
                Relation::LineEndings
            }
        };
        println!(
            "{name} => {actual:?} (nous {} conflit(s), git {})",
            mine.conflicts, theirs_merge.conflicts
        );
        // Attendu écrit à la main, quand le cas en a un : fusion propre, et ce
        // texte-là à l'octet près.
        if let Ok(expected) = fs::read(root.join(name).join("expected")) {
            assert_eq!(mine.conflicts, 0, "{name} : fusion propre attendue");
            assert_eq!(
                mine.text.as_bytes(),
                expected.as_slice(),
                "{name} : texte fusionné différent de l'attendu"
            );
        }
        assert_eq!(actual, declared, "{name} : relation à Git inattendue");

        // Un écart « fins de ligne » doit disparaître quand Git reçoit des
        // fichiers aux fins de ligne normalisées : c'est bien le seul motif.
        if declared == Relation::LineEndings {
            let lf = |bytes: &[u8]| text(bytes).replace("\r\n", "\n").into_bytes();
            let normalised = git_merge_file(dir.path(), &lf(&base), &lf(&ours), &lf(&theirs));
            assert!(
                mine.conflicts > 0 || normalised.conflicts == 0,
                "{name} : même sans les fins de ligne, Git voit un conflit"
            );
            if mine.conflicts == 0 {
                assert_eq!(
                    mine.text.replace("\r\n", "\n").as_bytes(),
                    normalised.text.as_slice(),
                    "{name} : texte différent de Git aux fins de ligne près"
                );
            }
        }
        seen += 1;
    }
    assert!(seen >= 30, "corpus trop maigre : {seen} cas");
}

#[test]
fn corpus_diffs_are_as_small_as_git_diff_minimal() {
    let root = fixtures();
    let manifest = fs::read_to_string(root.join("cases.txt")).expect("cases.txt");
    let dir = tempfile::tempdir().unwrap();
    let cancel = CancelToken::new();
    for line in manifest.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let name = line.split('|').next().unwrap().trim();
        let read = |file: &str| fs::read(root.join(name).join(file)).expect("fichier du corpus");
        let base = read("base");
        for side in ["ours", "theirs"] {
            let other = read(side);
            let left = String::from_utf8(base.clone()).unwrap();
            let right = String::from_utf8(other.clone()).unwrap();
            // Git ne connaît pas la BOM : elle fait partie de sa première ligne.
            if left.starts_with('\u{feff}') != right.starts_with('\u{feff}') {
                continue;
            }
            let diff = diff_lines(&left, &right, &DiffOptions::exact(), &cancel).unwrap();
            let (removed, added) = diff.changed_line_counts();
            let (git_added, git_removed) = git_numstat(dir.path(), &base, &other);
            assert_eq!(
                (added, removed),
                (git_added, git_removed),
                "{name}/{side} : nombre de lignes ajoutées/supprimées"
            );
        }
    }
}

/// Compare une fusion tirée au hasard à celle de Git. Rend (propre des deux
/// côtés, plus stricte que Git). Panique si nous fusionnons là où Git voit un
/// conflit, ou si deux fusions propres diffèrent.
fn check_against_git(
    dir: &Path,
    label: &str,
    base: &str,
    ours: &str,
    theirs: &str,
) -> (bool, bool) {
    let mine = merge(base, ours, theirs, &CancelToken::new()).unwrap();
    let theirs_merge = git_merge_file(dir, base.as_bytes(), ours.as_bytes(), theirs.as_bytes());
    let context = || format!("{label}\nbase   {base:?}\nnôtre  {ours:?}\nleur   {theirs:?}");
    match (mine.conflicts == 0, theirs_merge.conflicts == 0) {
        (true, false) => panic!(
            "fusion déclarée sans conflit là où Git voit un conflit\n{}\nnous   {:?}",
            context(),
            mine.text
        ),
        (true, true) => {
            assert_eq!(
                mine.text.as_bytes(),
                theirs_merge.text.as_slice(),
                "fusions propres mais différentes\n{}",
                context()
            );
            (true, false)
        }
        (false, true) => {
            if std::env::var_os("JULIE_DIFF_SHOW_STRICTER").is_some() {
                println!("plus strict que Git : {}", context());
            }
            (false, true)
        }
        (false, false) => (false, false),
    }
}

/// Tire des fusions au hasard (petit alphabet : alignements ambigus garantis) et
/// compare à Git. Rend (cas, propres des deux côtés, plus stricts que Git).
fn random_against_git(seed: u64, rounds: usize) -> (usize, usize, usize) {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = Rng(seed);
    let (mut both_clean, mut stricter) = (0, 0);
    for round in 0..rounds {
        let alphabet = 3 + rng.below(9);
        // Quatre tirages, trois gabarits : petits fichiers très ambigus, moyens, plus longs.
        let (max_len, max_edits) = [(14, 3), (14, 3), (30, 5), (60, 8)][rng.below(4)];
        let base_lines = random_lines(&mut rng, max_len, alphabet);
        let ours_lines = mutate(&mut rng, &base_lines, alphabet, max_edits);
        let theirs_lines = mutate(&mut rng, &base_lines, alphabet, max_edits);
        let mut texts = [
            join_lf(&base_lines),
            join_lf(&ours_lines),
            join_lf(&theirs_lines),
        ];
        // Une fois sur six, un des textes perd son saut de ligne final.
        if rng.chance(6) {
            let which = rng.below(3);
            texts[which].pop();
        }
        let [base, ours, theirs] = texts;
        let label = format!("tour {round} (graine {seed})");
        let (clean, strict) = check_against_git(dir.path(), &label, &base, &ours, &theirs);
        both_clean += usize::from(clean);
        stricter += usize::from(strict);
    }
    (rounds, both_clean, stricter)
}

/// Du faux code : une classe, des méthodes séparées par des lignes vides, des
/// instructions qui se répètent, des accolades seules sur leur ligne. C'est le
/// terrain de l'écart M1 de la recette (lignes vides et accolades répétées
/// autour des changements).
fn code_like(rng: &mut Rng, methods: usize) -> Vec<String> {
    const STATEMENTS: [&str; 6] = [
        "        return;",
        "        $this->save();",
        "        $x = 1;",
        "        if ($x) {",
        "        }",
        "        // rien",
    ];
    let mut lines = vec!["<?php".to_string(), "class A".to_string(), "{".to_string()];
    for method in 0..methods {
        if method > 0 {
            lines.push(String::new());
        }
        lines.push(format!("    public function m{method}()"));
        lines.push("    {".to_string());
        for _ in 0..rng.below(4) {
            lines.push(STATEMENTS[rng.below(STATEMENTS.len())].to_string());
        }
        lines.push("    }".to_string());
    }
    lines.push("}".to_string());
    lines
}

/// Quelques retouches comme on en fait dans du code : modifier une ligne,
/// ajouter une méthode entière (ligne vide comprise, avant ou après), ajouter ou
/// retirer des instructions, retirer ou doubler une ligne.
fn mutate_code(rng: &mut Rng, lines: &[String], max_edits: usize) -> Vec<String> {
    let mut out = lines.to_vec();
    for _ in 0..1 + rng.below(max_edits) {
        if out.is_empty() {
            break;
        }
        let at = rng.below(out.len());
        let tag = rng.below(4);
        match rng.below(7) {
            0 | 1 => out[at] = format!("{} // m{tag}", out[at]),
            2 => {
                let block = [
                    String::new(),
                    format!("    public function n{tag}()"),
                    "    {".to_string(),
                    "        return;".to_string(),
                    "    }".to_string(),
                ];
                out.splice(at..at, block);
            }
            3 => {
                let block = [
                    format!("    public function n{tag}()"),
                    "    {".to_string(),
                    "    }".to_string(),
                    String::new(),
                ];
                out.splice(at..at, block);
            }
            4 => {
                let count = 1 + rng.below(2);
                let added = vec!["        $x = 1;".to_string(); count];
                out.splice(at..at, added);
            }
            5 => {
                let end = (at + 1 + rng.below(3)).min(out.len());
                out.drain(at..end);
            }
            _ => {
                let copy = out[at].clone();
                out.insert(at, copy);
            }
        }
    }
    out
}

/// Retouches lourdes : une longue méthode ajoutée (lignes vides et accolades à
/// l'intérieur), ou une dizaine de lignes réécrites en gardant telles quelles
/// les lignes banales du milieu. Dans un fichier assez long, Git écarte alors de
/// sa recherche les lignes banales noyées dans le neuf : son diff n'est plus
/// forcément le plus court.
fn mutate_code_heavily(rng: &mut Rng, lines: &[String]) -> Vec<String> {
    let mut out = lines.to_vec();
    if out.is_empty() {
        return out;
    }
    let at = rng.below(out.len());
    let tag = rng.below(1000);
    if rng.chance(2) {
        let block = [
            String::new(),
            format!("    public function big{tag}()"),
            "    {".to_string(),
            format!("        $a = {tag};"),
            format!("        $b = {tag} + 1;"),
            String::new(),
            "        if ($x) {".to_string(),
            format!("            $c = $a + $b + {tag};"),
            "        }".to_string(),
            String::new(),
            format!("        return $c * {tag};"),
            "    }".to_string(),
        ];
        out.splice(at..at, block);
    } else {
        let end = (at + 6 + rng.below(8)).min(out.len());
        for (offset, line) in out[at..end].iter_mut().enumerate() {
            let banal = line.is_empty() || line.trim() == "{" || line.trim() == "}";
            if !banal {
                *line = format!("        $r{tag}_{offset} = {offset};");
            }
        }
    }
    out
}

/// Même comparaison que [`random_against_git`], sur du faux code. Un tour sur
/// trois, le fichier est long (une vingtaine de méthodes) et chaque côté reçoit
/// en plus une retouche lourde.
fn random_code_against_git(seed: u64, rounds: usize) -> (usize, usize, usize) {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = Rng(seed);
    let (mut both_clean, mut stricter) = (0, 0);
    for round in 0..rounds {
        let heavy = round % 3 == 2;
        let methods = if heavy {
            12 + rng.below(12)
        } else {
            1 + rng.below(5)
        };
        let base_lines = code_like(&mut rng, methods);
        let max_edits = 1 + rng.below(3);
        let mut ours_lines = mutate_code(&mut rng, &base_lines, max_edits);
        let mut theirs_lines = mutate_code(&mut rng, &base_lines, max_edits);
        if heavy {
            ours_lines = mutate_code_heavily(&mut rng, &ours_lines);
            theirs_lines = mutate_code_heavily(&mut rng, &theirs_lines);
        }
        let label = format!("faux code, tour {round} (graine {seed})");
        let (clean, strict) = check_against_git(
            dir.path(),
            &label,
            &join_lf(&base_lines),
            &join_lf(&ours_lines),
            &join_lf(&theirs_lines),
        );
        both_clean += usize::from(clean);
        stricter += usize::from(strict);
    }
    (rounds, both_clean, stricter)
}

#[test]
fn random_merges_never_hide_a_conflict_git_sees() {
    let (rounds, both_clean, stricter) = random_against_git(0x5eed_0001, 400);
    println!("{rounds} fusions : {both_clean} propres des deux côtés, {stricter} où nous sommes plus stricts que Git");
    assert!(
        both_clean > rounds / 10,
        "le tirage doit produire des fusions propres"
    );
}

#[test]
fn random_code_merges_never_hide_a_conflict_git_sees() {
    let (rounds, both_clean, stricter) = random_code_against_git(0xc0de_0001, 300);
    println!("{rounds} fusions de faux code : {both_clean} propres des deux côtés, {stricter} où nous sommes plus stricts que Git");
    assert!(
        both_clean > rounds / 10,
        "le tirage doit produire des fusions propres"
    );
}

/// Version longue, à lancer à la main :
/// `cargo test -p julie-diff --test git_compare -- --ignored --nocapture`
/// (`JULIE_DIFF_SOAK` = nombre de tours, 5000 par défaut ; `JULIE_DIFF_SEED` =
/// graine du tirage, pour explorer d'autres cas ; `JULIE_DIFF_SHOW_STRICTER=1`
/// affiche les cas où nous sommes plus stricts que Git). Le tirage en petit
/// alphabet est suivi d'un tirage de faux code, moitié moins long
/// (`JULIE_DIFF_SOAK_CODE` pour en fixer le nombre de tours).
#[test]
#[ignore = "long : des milliers de lancements de git"]
fn random_merges_soak() {
    let rounds = std::env::var("JULIE_DIFF_SOAK")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(5000);
    let seed = std::env::var("JULIE_DIFF_SEED")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0x5eed_0002);
    let (count, both_clean, stricter) = random_against_git(seed, rounds);
    println!("{count} fusions : {both_clean} propres des deux côtés, {stricter} où nous sommes plus stricts que Git");
    let code_rounds = std::env::var("JULIE_DIFF_SOAK_CODE")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(rounds / 2);
    let (count, both_clean, stricter) = random_code_against_git(seed, code_rounds);
    println!("{count} fusions de faux code : {both_clean} propres des deux côtés, {stricter} où nous sommes plus stricts que Git");
}
