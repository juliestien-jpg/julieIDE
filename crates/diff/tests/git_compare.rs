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

/// Tire des fusions au hasard (petit alphabet : alignements ambigus garantis) et
/// compare à Git. Rend (cas, propres des deux côtés, plus stricts que Git).
fn random_against_git(seed: u64, rounds: usize) -> (usize, usize, usize) {
    let dir = tempfile::tempdir().unwrap();
    let cancel = CancelToken::new();
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
        let mine = merge(&base, &ours, &theirs, &cancel).unwrap();
        let theirs_merge = git_merge_file(
            dir.path(),
            base.as_bytes(),
            ours.as_bytes(),
            theirs.as_bytes(),
        );
        let context = || {
            format!(
                "tour {round} (graine {seed})\nbase   {base:?}\nnôtre  {ours:?}\nleur   {theirs:?}"
            )
        };
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
                both_clean += 1;
            }
            (false, true) => stricter += 1,
            (false, false) => {}
        }
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

/// Version longue, à lancer à la main :
/// `cargo test -p julie-diff --test git_compare -- --ignored --nocapture`
/// (`JULIE_DIFF_SOAK` = nombre de tours, 5000 par défaut ; `JULIE_DIFF_SEED` =
/// graine du tirage, pour explorer d'autres cas).
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
    let (rounds, both_clean, stricter) = random_against_git(seed, rounds);
    println!("{rounds} fusions : {both_clean} propres des deux côtés, {stricter} où nous sommes plus stricts que Git");
}
