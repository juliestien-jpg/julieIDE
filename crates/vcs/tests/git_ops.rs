//! Fonctions synchrones de `julie-vcs` sur des dépôts jetables.

mod support;

use std::path::PathBuf;

use julie_vcs::*;
use support::{cancel, TestRepo};

fn kinds(status: &Status) -> Vec<(String, FileKind)> {
    status
        .files
        .iter()
        .map(|file| (file.repo_path.clone(), file.kind))
        .collect()
}

// ---------------------------------------------------------------- dépôt

#[test]
fn opening_finds_the_repository_from_a_subfolder() {
    let test = TestRepo::new();
    test.write("sous dossier/é/x.txt", "x");
    let git = test.git();
    let repo = open_repository(&git, &test.path("sous dossier/é"), &cancel()).unwrap();
    assert_eq!(repo.work_tree(), test.root.as_path());
    assert!(repo.git_dir().ends_with(".git"), "{:?}", repo.git_dir());
    assert_eq!(
        repo.to_repo_path(&test.path("sous dossier/é/x.txt"))
            .unwrap(),
        "sous dossier/é/x.txt"
    );
}

#[test]
fn a_plain_folder_or_the_git_folder_is_not_a_repository() {
    let plain = TestRepo::empty_dir();
    let error = open_repository(&plain.git(), &plain.root, &cancel()).unwrap_err();
    assert!(matches!(error, VcsError::NotARepository(_)), "{error:?}");

    let test = TestRepo::new();
    let error = open_repository(&test.git(), &test.path(".git"), &cancel()).unwrap_err();
    assert!(matches!(error, VcsError::NotARepository(_)), "{error:?}");
}

#[test]
fn a_missing_git_executable_is_reported() {
    let test = TestRepo::new();
    let git = Git::new(&VcsSettings {
        git_path: Some(PathBuf::from(r"C:\nulle part\git-introuvable.exe")),
        ..VcsSettings::default()
    });
    let error = open_repository(&git, &test.root, &cancel()).unwrap_err();
    assert!(matches!(error, VcsError::GitNotFound { .. }), "{error:?}");
}

#[test]
fn a_cancelled_token_runs_nothing() {
    let test = TestRepo::new();
    let repo = test.repo();
    let token = cancel();
    token.cancel();
    let error = status(&test.git(), &repo, &token).unwrap_err();
    assert!(error.is_cancelled());
}

#[cfg(windows)]
#[test]
fn paths_with_another_case_are_accepted_on_windows() {
    let test = TestRepo::new();
    test.write("Fichier.txt", "un\n");
    test.commit_all("premier");
    test.write("Fichier.txt", "deux\n");
    let repo = test.repo();
    let shouted = PathBuf::from(test.root.to_string_lossy().to_uppercase()).join("Fichier.txt");
    assert_eq!(repo.to_repo_path(&shouted).unwrap(), "Fichier.txt");
    let head = read_version(&test.git(), &repo, &shouted, &Revision::Head, &cancel()).unwrap();
    assert_eq!(head.bytes(), Some(&b"un\n"[..]));
    // Git (core.ignorecase) ne voit pas un changement de casse seul.
    std::fs::rename(test.path("Fichier.txt"), test.path("FICHIER.txt")).unwrap();
    let state = status(&test.git(), &repo, &cancel()).unwrap();
    assert_eq!(
        kinds(&state),
        vec![("Fichier.txt".to_owned(), FileKind::Modified)]
    );
}

// ---------------------------------------------------------------- état

#[test]
fn status_groups_modified_deleted_added_and_untracked() {
    let test = TestRepo::new();
    test.write("modifié.txt", "a\n");
    test.write("à supprimer.txt", "b\n");
    test.commit_all("premier");
    test.write("modifié.txt", "a2\n");
    std::fs::remove_file(test.path("à supprimer.txt")).unwrap();
    test.write("ajouté [1].txt", "c\n");
    test.run(&["add", "ajouté [1].txt"]);
    test.write("non suivi/l'autre fichier.txt", "d\n");

    let repo = test.repo();
    let state = status(&test.git(), &repo, &cancel()).unwrap();
    assert!(matches!(&state.head, Head::Branch { name, commit: Some(_) } if name == "main"));
    assert_eq!(state.upstream, None);
    assert_eq!(state.operation, None);
    let mut found = kinds(&state);
    found.sort();
    assert_eq!(
        found,
        vec![
            ("ajouté [1].txt".to_owned(), FileKind::Added),
            ("modifié.txt".to_owned(), FileKind::Modified),
            (
                "non suivi/l'autre fichier.txt".to_owned(),
                FileKind::Untracked
            ),
            ("à supprimer.txt".to_owned(), FileKind::Deleted),
        ]
    );
    assert_eq!(state.changed().count(), 3);
    assert_eq!(state.untracked().count(), 1);
    assert_eq!(state.conflicts().count(), 0);
    let untracked = state.untracked().next().unwrap();
    assert_eq!(untracked.path, test.path("non suivi/l'autre fichier.txt"));
}

#[test]
fn renames_staged_and_unstaged() {
    let test = TestRepo::new();
    test.write("ancien nom.txt", "contenu assez long pour un renommage\n");
    test.write("autre.txt", "contenu d'un autre fichier, assez long\n");
    test.commit_all("premier");
    test.run(&["mv", "ancien nom.txt", "nouveau nom é.txt"]);
    std::fs::rename(test.path("autre.txt"), test.path("autre renommé.txt")).unwrap();

    let repo = test.repo();
    let state = status(&test.git(), &repo, &cancel()).unwrap();
    let renamed = state
        .files
        .iter()
        .find(|file| file.kind == FileKind::Renamed)
        .expect("renommage indexé");
    assert_eq!(renamed.repo_path, "nouveau nom é.txt");
    assert_eq!(
        renamed.original_repo_path.as_deref(),
        Some("ancien nom.txt")
    );
    assert_eq!(renamed.original_path, Some(test.path("ancien nom.txt")));
    // Renommage non indexé : supprimé + non suivi, comme Git.
    let mut rest: Vec<_> = kinds(&state)
        .into_iter()
        .filter(|(_, kind)| *kind != FileKind::Renamed)
        .collect();
    rest.sort();
    assert_eq!(
        rest,
        vec![
            ("autre renommé.txt".to_owned(), FileKind::Untracked),
            ("autre.txt".to_owned(), FileKind::Deleted),
        ]
    );

    // Le diff d'un renommage lit la gauche à l'ancien chemin.
    let sides = load_diff(
        &test.git(),
        &repo,
        DiffRequest::local_change(renamed),
        &cancel(),
    )
    .unwrap();
    assert_eq!(sides.left.bytes(), sides.right.bytes());
    assert!(sides.left.exists());

    // Valider le seul nouveau chemin emporte l'ancien.
    commit(
        &test.git(),
        &repo,
        &CommitRequest {
            paths: vec![test.path("nouveau nom é.txt")],
            message: "Renommage".into(),
            amend: false,
        },
        &cancel(),
    )
    .unwrap();
    let show = test.run(&[
        "-c",
        "core.quotepath=false",
        "show",
        "-M",
        "--name-status",
        "--format=",
        "HEAD",
    ]);
    assert!(
        show.starts_with("R100\tancien nom.txt\tnouveau nom é.txt"),
        "{show}"
    );
    let state = status(&test.git(), &repo, &cancel()).unwrap();
    assert_eq!(state.files.len(), 2, "{state:?}");
}

#[test]
fn unborn_repository_lists_files_and_makes_a_first_commit() {
    let test = TestRepo::new();
    test.write("a.txt", "a\n");
    test.write("dossier é/b.txt", "b\n");
    let repo = test.repo();
    let git = test.git();

    let state = status(&git, &repo, &cancel()).unwrap();
    assert!(state.head.is_unborn());
    assert_eq!(state.untracked().count(), 2);
    let page = log(&git, &repo, &LogQuery::default(), &cancel()).unwrap();
    assert!(page.commits.is_empty() && !page.has_more);
    let head = read_version(&git, &repo, &test.path("a.txt"), &Revision::Head, &cancel()).unwrap();
    assert_eq!(head, Content::Missing);
    let error = commit(
        &git,
        &repo,
        &CommitRequest {
            paths: vec![],
            message: "x".into(),
            amend: true,
        },
        &cancel(),
    )
    .unwrap_err();
    assert!(matches!(error, VcsError::NoCommits));

    let outcome = commit(
        &git,
        &repo,
        &CommitRequest {
            paths: vec![test.path("a.txt")],
            message: "Premier commit\n\nCorps.".into(),
            amend: false,
        },
        &cancel(),
    )
    .unwrap();
    assert_eq!(outcome.subject, "Premier commit");
    assert_eq!(outcome.commit, test.head());
    assert_eq!(test.files_in_head(), vec!["a.txt"]);
    let state = status(&git, &repo, &cancel()).unwrap();
    assert_eq!(
        kinds(&state),
        vec![("dossier é/b.txt".to_owned(), FileKind::Untracked)]
    );
    let page = log(&git, &repo, &LogQuery::default(), &cancel()).unwrap();
    assert_eq!(page.commits.len(), 1);
}

#[test]
fn detached_head_is_reported() {
    let test = TestRepo::new();
    test.write("a.txt", "1\n");
    test.commit_all("un");
    test.write("a.txt", "2\n");
    test.commit_all("deux");
    test.run(&["checkout", "-q", "--detach", "HEAD~1"]);
    let state = status(&test.git(), &test.repo(), &cancel()).unwrap();
    let expected = test.head();
    assert_eq!(state.head, Head::Detached { commit: expected });
}

#[test]
fn upstream_ahead_and_behind() {
    let test = TestRepo::new();
    test.write("a.txt", "1\n");
    test.commit_all("un");
    test.run(&["branch", "amont"]);
    test.run(&["branch", "--set-upstream-to=amont"]);
    test.write("a.txt", "2\n");
    test.commit_all("deux");
    let state = status(&test.git(), &test.repo(), &cancel()).unwrap();
    assert_eq!(
        state.upstream,
        Some(Upstream {
            name: "amont".into(),
            ahead_behind: Some((1, 0))
        })
    );
}

#[test]
fn submodules_are_flagged_and_their_versions_are_commits() {
    let test = TestRepo::new();
    // Le dépôt du sous-module, à côté (hors du dépôt principal).
    let sub_source = test.base.join("source du module");
    std::fs::create_dir_all(&sub_source).unwrap();
    test.raw_in(&sub_source, &["init", "-q", "-b", "main"]);
    std::fs::write(sub_source.join("m.txt"), "m\n").unwrap();
    test.raw_in(&sub_source, &["add", "-A"]);
    test.raw_in(&sub_source, &["commit", "-q", "-m", "module"]);
    let source = sub_source.to_string_lossy().replace('\\', "/");
    test.run(&["submodule", "add", "-q", &source, "module"]);
    test.commit_all("ajout du module");

    let module = test.path("module");
    std::fs::write(module.join("m.txt"), "changé\n").unwrap();
    test.raw_in(&module, &["commit", "-q", "-am", "dans le module"]);

    let repo = test.repo();
    let git = test.git();
    let state = status(&git, &repo, &cancel()).unwrap();
    let entry = state
        .files
        .iter()
        .find(|f| f.repo_path == "module")
        .expect("module");
    assert!(entry.submodule);
    assert_eq!(entry.kind, FileKind::Modified);
    assert!(matches!(
        read_version(&git, &repo, &module, &Revision::Head, &cancel()).unwrap(),
        Content::Submodule { .. }
    ));
    assert_eq!(
        read_version(&git, &repo, &module, &Revision::WorkTree, &cancel()).unwrap(),
        Content::Directory
    );
}

// ---------------------------------------------------------------- versions

#[test]
fn diff_sides_for_modified_added_deleted_and_binary_files() {
    let test = TestRepo::new();
    test.write("dossier é/modifié.php", "<?php\necho 1;\n");
    test.write("supprimé.txt", "x\n");
    test.write("image.bin", [0u8, 1, 2, 3]);
    test.commit_all("premier");
    test.write("dossier é/modifié.php", "<?php\necho 2;\n");
    std::fs::remove_file(test.path("supprimé.txt")).unwrap();
    test.write("ajouté.txt", "nouveau\n");
    test.run(&["add", "ajouté.txt"]);
    test.write("ajouté.txt", "nouveau, modifié\n");

    let repo = test.repo();
    let git = test.git();
    let state = status(&git, &repo, &cancel()).unwrap();
    let sides = |repo_path: &str| {
        let file = state
            .files
            .iter()
            .find(|f| f.repo_path == repo_path)
            .unwrap();
        load_diff(&git, &repo, DiffRequest::local_change(file), &cancel()).unwrap()
    };

    let modified = sides("dossier é/modifié.php");
    assert_eq!(modified.left.bytes(), Some(&b"<?php\necho 1;\n"[..]));
    assert_eq!(modified.right.bytes(), Some(&b"<?php\necho 2;\n"[..]));
    let added = sides("ajouté.txt");
    assert_eq!(added.left, Content::Missing);
    assert_eq!(added.right.bytes(), Some("nouveau, modifié\n".as_bytes()));
    let index = read_version(
        &git,
        &repo,
        &test.path("ajouté.txt"),
        &Revision::Index,
        &cancel(),
    )
    .unwrap();
    assert_eq!(index.bytes(), Some(&b"nouveau\n"[..]));
    let deleted = sides("supprimé.txt");
    assert_eq!(deleted.right, Content::Missing);
    assert_eq!(deleted.left.bytes(), Some(&b"x\n"[..]));

    let binary = read_version(
        &git,
        &repo,
        &test.path("image.bin"),
        &Revision::Head,
        &cancel(),
    )
    .unwrap();
    assert!(binary.is_binary());
    assert_eq!(binary.size(), Some(4));
}

#[test]
fn autocrlf_versions_have_the_bytes_of_the_work_tree() {
    let test = TestRepo::new();
    test.run(&["config", "core.autocrlf", "true"]);
    test.write("crlf.txt", "un\r\ndeux\r\n");
    test.commit_all("premier");
    // Stocké en LF dans Git…
    assert_eq!(
        test.run(&["cat-file", "blob", "HEAD:crlf.txt"]),
        "un\ndeux\n"
    );

    let repo = test.repo();
    let git = test.git();
    let state = status(&git, &repo, &cancel()).unwrap();
    assert!(state.is_clean(), "{state:?}");
    // … mais lu comme Git l'écrirait sur le disque.
    let head = read_version(
        &git,
        &repo,
        &test.path("crlf.txt"),
        &Revision::Head,
        &cancel(),
    )
    .unwrap();
    assert_eq!(head.bytes(), Some(&b"un\r\ndeux\r\n"[..]));

    test.write("crlf.txt", "un\r\ntrois\r\n");
    let file = status(&git, &repo, &cancel()).unwrap().files.remove(0);
    let sides = load_diff(&git, &repo, DiffRequest::local_change(&file), &cancel()).unwrap();
    assert_eq!(sides.left.bytes(), Some(&b"un\r\ndeux\r\n"[..]));
    assert_eq!(sides.right.bytes(), Some(&b"un\r\ntrois\r\n"[..]));
}

// ---------------------------------------------------------------- commit

#[test]
fn commit_takes_the_work_tree_of_selected_files_and_leaves_the_others_alone() {
    let test = TestRepo::new();
    for name in ["x.txt", "y.txt", "z.txt"] {
        test.write(name, "v1\n");
    }
    test.commit_all("premier");
    test.write("x.txt", "x-travail\n");
    test.write("y.txt", "y-index\n");
    test.run(&["add", "y.txt"]);
    test.write("y.txt", "y-travail\n");
    test.write("z.txt", "z-index\n");
    test.run(&["add", "z.txt"]);
    test.write("z.txt", "z-travail\n");
    test.write("nouveau é.txt", "u\n");
    test.write("laissé.txt", "non suivi, non choisi\n");

    let repo = test.repo();
    let outcome = commit(
        &test.git(),
        &repo,
        &CommitRequest {
            paths: vec![
                test.path("x.txt"),
                test.path("y.txt"),
                test.path("nouveau é.txt"),
            ],
            message: "Deux sur trois".into(),
            amend: false,
        },
        &cancel(),
    )
    .unwrap();
    assert_eq!(outcome.subject, "Deux sur trois");
    assert_eq!(
        test.files_in_head(),
        vec!["nouveau é.txt", "x.txt", "y.txt"]
    );
    assert_eq!(test.run(&["show", "HEAD:y.txt"]), "y-travail\n");
    // z : index et disque inchangés.
    assert_eq!(test.run(&["show", ":z.txt"]), "z-index\n");
    assert_eq!(test.read("z.txt"), b"z-travail\n");
    let state = status(&test.git(), &repo, &cancel()).unwrap();
    let mut found = kinds(&state);
    found.sort();
    assert_eq!(
        found,
        vec![
            ("laissé.txt".to_owned(), FileKind::Untracked),
            ("z.txt".to_owned(), FileKind::Modified),
        ]
    );
}

#[test]
fn brackets_in_names_are_not_patterns() {
    let test = TestRepo::new();
    test.write("a[1].txt", "crochets\n");
    test.write("a1.txt", "sans crochets\n");
    let repo = test.repo();
    commit(
        &test.git(),
        &repo,
        &CommitRequest {
            paths: vec![test.path("a[1].txt")],
            message: "Crochets".into(),
            amend: false,
        },
        &cancel(),
    )
    .unwrap();
    assert_eq!(test.files_in_head(), vec!["a[1].txt"]);
}

#[test]
fn a_failing_pre_commit_hook_commits_nothing_and_restores_the_index() {
    let test = TestRepo::new();
    test.write("suivi.txt", "1\n");
    test.commit_all("premier");
    let before = test.head();
    test.write(
        ".git/hooks/pre-commit",
        "#!/bin/sh\necho 'refusé par le crochet' >&2\nexit 1\n",
    );
    test.write("suivi.txt", "2\n");
    test.write("non suivi.txt", "n\n");

    let repo = test.repo();
    let error = commit(
        &test.git(),
        &repo,
        &CommitRequest {
            paths: vec![test.path("suivi.txt"), test.path("non suivi.txt")],
            message: "Refusé".into(),
            amend: false,
        },
        &cancel(),
    )
    .unwrap_err();
    match &error {
        VcsError::GitFailed { stderr, .. } => {
            assert!(stderr.contains("refusé par le crochet"), "{stderr}")
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(test.head(), before);
    let state = status(&test.git(), &repo, &cancel()).unwrap();
    let mut found: Vec<_> = state
        .files
        .iter()
        .map(|f| (f.repo_path.clone(), f.kind, f.staged))
        .collect();
    found.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        found,
        vec![
            (
                "non suivi.txt".to_owned(),
                FileKind::Untracked,
                Change::Unmodified
            ),
            (
                "suivi.txt".to_owned(),
                FileKind::Modified,
                Change::Unmodified
            ),
        ]
    );
}

#[test]
fn amend_can_change_only_the_message() {
    let test = TestRepo::new();
    test.write("a.txt", "1\n");
    test.commit_all("premier");
    test.write("a.txt", "2\n");
    test.commit_all("deuxième avec faute");
    let parent = test.run(&["rev-parse", "HEAD~1"]);
    test.write("a.txt", "3 indexé\n");
    test.run(&["add", "a.txt"]);

    let repo = test.repo();
    let outcome = commit(
        &test.git(),
        &repo,
        &CommitRequest {
            paths: vec![],
            message: "Deuxième, corrigé".into(),
            amend: true,
        },
        &cancel(),
    )
    .unwrap();
    assert!(outcome.amended);
    assert_eq!(test.run(&["rev-parse", "HEAD~1"]), parent);
    assert_eq!(
        test.run(&["log", "-1", "--format=%s"]).trim(),
        "Deuxième, corrigé"
    );
    assert_eq!(test.run(&["show", "HEAD:a.txt"]), "2\n");
    assert_eq!(test.run(&["show", ":a.txt"]), "3 indexé\n");

    // Amender avec un fichier : son contenu de travail rejoint le commit.
    commit(
        &test.git(),
        &repo,
        &CommitRequest {
            paths: vec![test.path("a.txt")],
            message: "Deuxième, avec a".into(),
            amend: true,
        },
        &cancel(),
    )
    .unwrap();
    assert_eq!(test.run(&["rev-parse", "HEAD~1"]), parent);
    assert_eq!(test.run(&["show", "HEAD:a.txt"]), "3 indexé\n");
}

#[test]
fn empty_message_or_selection_is_refused() {
    let test = TestRepo::new();
    test.write("a.txt", "1\n");
    let repo = test.repo();
    let request = |paths: Vec<PathBuf>, message: &str| CommitRequest {
        paths,
        message: message.into(),
        amend: false,
    };
    assert!(matches!(
        commit(
            &test.git(),
            &repo,
            &request(vec![test.path("a.txt")], "  \n "),
            &cancel()
        ),
        Err(VcsError::EmptyMessage)
    ));
    assert!(matches!(
        commit(&test.git(), &repo, &request(vec![], "message"), &cancel()),
        Err(VcsError::NothingSelected)
    ));
}

#[test]
fn a_held_index_lock_is_reported() {
    let test = TestRepo::new();
    test.write("a.txt", "1\n");
    test.commit_all("premier");
    test.write("a.txt", "2\n");
    test.write(".git/index.lock", "");
    let error = commit(
        &test.git(),
        &test.repo(),
        &CommitRequest {
            paths: vec![test.path("a.txt")],
            message: "verrou".into(),
            amend: false,
        },
        &cancel(),
    )
    .unwrap_err();
    assert!(matches!(error, VcsError::IndexLocked(_)), "{error:?}");
}

// ---------------------------------------------------------------- journal

#[test]
fn log_is_paged_and_details_list_files() {
    let test = TestRepo::new();
    for index in 1..=5 {
        test.write(&format!("f{index}.txt"), format!("{index}\n"));
        test.commit_all(&format!("commit {index}"));
        if index == 2 {
            test.run(&["tag", "v2"]);
        }
    }
    let repo = test.repo();
    let git = test.git();
    let first = log(
        &git,
        &repo,
        &LogQuery {
            limit: 2,
            ..LogQuery::default()
        },
        &cancel(),
    )
    .unwrap();
    assert_eq!(
        first
            .commits
            .iter()
            .map(|c| c.subject.as_str())
            .collect::<Vec<_>>(),
        vec!["commit 5", "commit 4"]
    );
    assert!(first.has_more);
    let top = &first.commits[0];
    assert_eq!(top.id, test.head());
    assert!(top.id.starts_with(&top.short_id));
    assert_eq!(top.author.name, "Essai Julie");
    assert_eq!(top.author.email, "essai@example.invalid");
    assert!(top.author.time > 1_600_000_000);
    assert!(top
        .refs
        .iter()
        .any(|r| r.kind == RefKind::LocalBranch && r.name == "main" && r.current));
    assert!(top.refs.iter().any(|r| r.kind == RefKind::Head));
    assert!(!top.is_merge());

    let second = log(&git, &repo, &first.query.next_page(), &cancel()).unwrap();
    assert_eq!(second.commits[1].subject, "commit 2");
    assert!(second.commits[1]
        .refs
        .iter()
        .any(|r| r.kind == RefKind::Tag && r.name == "v2"));
    let last = log(&git, &repo, &second.query.next_page(), &cancel()).unwrap();
    assert_eq!(last.commits.len(), 1);
    assert!(!last.has_more);
    let root = &last.commits[0];
    assert!(root.parents.is_empty());

    let details = commit_details(&git, &repo, &root.id, &cancel()).unwrap();
    assert_eq!(details.message, "commit 1");
    assert_eq!(details.files.len(), 1);
    assert_eq!(details.files[0].change, Change::Added);
    assert_eq!(details.files[0].parent, None);
    assert_eq!(details.files[0].path, test.path("f1.txt"));

    let details = commit_details(&git, &repo, &top.id, &cancel()).unwrap();
    assert_eq!(details.files[0].repo_path, "f5.txt");
    let request = DiffRequest::commit_file(&top.id, &details.files[0]);
    let sides = load_diff(&git, &repo, request, &cancel()).unwrap();
    assert_eq!(sides.left, Content::Missing);
    assert_eq!(sides.right.bytes(), Some(&b"5\n"[..]));

    assert!(matches!(
        commit_details(&git, &repo, "--output=x", &cancel()),
        Err(VcsError::InvalidRevision(_))
    ));
}

// ---------------------------------------------------------------- conflits

/// `main` et `eux` divergent sur trois fichiers ; fusion de `eux` dans `main`.
fn conflicted_merge() -> TestRepo {
    let test = TestRepo::new();
    test.write("c1.txt", "base 1\n");
    test.write("dossier é/c2.txt", "base 2\n");
    test.write("supprimé par eux.txt", "base 3\n");
    test.write("tranquille.txt", "base 4\n");
    test.commit_all("base");
    test.run(&["checkout", "-q", "-b", "eux"]);
    test.write("c1.txt", "leur 1\n");
    test.write("dossier é/c2.txt", "leur 2\n");
    std::fs::remove_file(test.path("supprimé par eux.txt")).unwrap();
    test.write("tranquille.txt", "leur 4\n");
    test.commit_all("leurs changements");
    test.run(&["checkout", "-q", "main"]);
    test.write("c1.txt", "notre 1\n");
    test.write("dossier é/c2.txt", "notre 2\n");
    test.write("supprimé par eux.txt", "notre 3\n");
    test.commit_all("nos changements");
    let output = test.try_run(&["merge", "eux"]);
    assert!(
        !output.status.success(),
        "la fusion doit s'arrêter sur des conflits"
    );
    test
}

#[test]
fn conflicts_are_listed_with_their_three_versions() {
    let test = conflicted_merge();
    let repo = test.repo();
    let git = test.git();
    let state = status(&git, &repo, &cancel()).unwrap();
    let operation = state.operation.clone().expect("fusion en cours");
    assert_eq!(operation.kind, OperationKind::Merge);
    assert!(operation
        .message
        .as_deref()
        .unwrap()
        .starts_with("Merge branch 'eux'"));
    let mut conflicts: Vec<_> = state
        .conflicts()
        .map(|f| (f.repo_path.clone(), f.conflict.unwrap()))
        .collect();
    conflicts.sort();
    assert_eq!(
        conflicts,
        vec![
            ("c1.txt".to_owned(), ConflictKind::BothModified),
            ("dossier é/c2.txt".to_owned(), ConflictKind::BothModified),
            (
                "supprimé par eux.txt".to_owned(),
                ConflictKind::DeletedByThem
            ),
        ]
    );
    // Le fichier fusionné sans conflit est indexé, pas en conflit.
    assert!(state
        .changed()
        .any(|f| f.repo_path == "tranquille.txt" && f.staged == Change::Modified));

    let versions = conflict_versions(&git, &repo, &test.path("c1.txt"), &cancel()).unwrap();
    assert_eq!(versions.kind, ConflictKind::BothModified);
    assert_eq!(versions.operation, Some(OperationKind::Merge));
    assert_eq!(versions.base.bytes(), Some(&b"base 1\n"[..]));
    assert_eq!(versions.ours.bytes(), Some(&b"notre 1\n"[..]));
    assert_eq!(versions.theirs.bytes(), Some(&b"leur 1\n"[..]));
    assert!(String::from_utf8_lossy(versions.work_tree.bytes().unwrap()).contains("<<<<<<<"));

    let deleted =
        conflict_versions(&git, &repo, &test.path("supprimé par eux.txt"), &cancel()).unwrap();
    assert_eq!(deleted.kind, ConflictKind::DeletedByThem);
    assert_eq!(deleted.theirs, Content::Missing);
    assert!(deleted.kind.involves_deletion());

    assert!(matches!(
        conflict_versions(&git, &repo, &test.path("tranquille.txt"), &cancel()),
        Err(VcsError::NotConflicted(_))
    ));
    assert!(matches!(
        commit(
            &git,
            &repo,
            &CommitRequest {
                paths: vec![test.path("tranquille.txt")],
                message: "partiel".into(),
                amend: false
            },
            &cancel()
        ),
        Err(VcsError::OperationInProgress(OperationKind::Merge))
    ));
}

#[test]
fn conflicts_are_resolved_and_the_merge_is_concluded() {
    let test = conflicted_merge();
    let repo = test.repo();
    let git = test.git();
    let merge_msg = String::from_utf8(test.read(".git/MERGE_MSG")).unwrap();

    // Prendre la leur.
    let done = accept_side(
        &git,
        &repo,
        &[test.path("dossier é/c2.txt")],
        ConflictSide::Theirs,
        &cancel(),
    )
    .unwrap();
    assert_eq!(
        done,
        vec![SideResolution {
            path: test.path("dossier é/c2.txt"),
            deleted: false
        }]
    );
    assert_eq!(test.read("dossier é/c2.txt"), b"leur 2\n");
    let unmerged = unmerged_paths(&git, &repo, &cancel()).unwrap();
    assert_eq!(unmerged.len(), 2);
    assert!(!unmerged.contains(&test.path("dossier é/c2.txt")));

    // Conclure trop tôt : refusé.
    assert!(matches!(
        continue_operation(&git, &repo, None, &cancel()),
        Err(VcsError::UnresolvedConflicts(paths)) if paths.len() == 2
    ));

    // Enregistrer un résultat fusionné, avec la garde « changé sur le disque ».
    let versions = conflict_versions(&git, &repo, &test.path("c1.txt"), &cancel()).unwrap();
    let on_disk = versions.work_tree.bytes().unwrap().to_vec();
    assert!(matches!(
        resolve_with_content(
            &git,
            &repo,
            &test.path("c1.txt"),
            b"fusion\n",
            Some(b"autre chose"),
            &cancel()
        ),
        Err(VcsError::ChangedOnDisk(_))
    ));
    assert_eq!(test.read("c1.txt"), on_disk, "rien n'est écrit");
    resolve_with_content(
        &git,
        &repo,
        &test.path("c1.txt"),
        b"notre 1\nleur 1\n",
        Some(&on_disk),
        &cancel(),
    )
    .unwrap();
    assert_eq!(test.read("c1.txt"), b"notre 1\nleur 1\n");
    assert!(matches!(
        resolve_with_content(&git, &repo, &test.path("c1.txt"), b"x", None, &cancel()),
        Err(VcsError::NotConflicted(_))
    ));

    // Prendre la leur sur un fichier qu'ils ont supprimé : supprimé.
    let done = accept_side(
        &git,
        &repo,
        &[test.path("supprimé par eux.txt")],
        ConflictSide::Theirs,
        &cancel(),
    )
    .unwrap();
    assert!(done[0].deleted);
    assert!(!test.exists("supprimé par eux.txt"));
    assert!(unmerged_paths(&git, &repo, &cancel()).unwrap().is_empty());

    // Valider termine la fusion : deux parents, message de MERGE_MSG.
    let outcome = continue_operation(&git, &repo, None, &cancel()).unwrap();
    assert!(outcome.finished);
    assert_eq!(outcome.kind, OperationKind::Merge);
    assert_eq!(outcome.head.as_deref(), Some(test.head().as_str()));
    let parents = test.run(&["log", "-1", "--format=%P"]);
    assert_eq!(parents.split_whitespace().count(), 2);
    let subject = test.run(&["log", "-1", "--format=%s"]);
    assert_eq!(subject.trim(), merge_msg.lines().next().unwrap());
    let state = status(&git, &repo, &cancel()).unwrap();
    assert!(state.is_clean(), "{state:?}");
    assert_eq!(state.operation, None);

    // Le journal voit la fusion ; ses fichiers sont comparés au premier parent.
    let page = log(&git, &repo, &LogQuery::default(), &cancel()).unwrap();
    assert!(page.commits[0].is_merge());
    let details = commit_details(&git, &repo, &page.commits[0].id, &cancel()).unwrap();
    let mut changed: Vec<_> = details.files.iter().map(|f| f.repo_path.as_str()).collect();
    changed.sort();
    assert_eq!(
        changed,
        vec![
            "c1.txt",
            "dossier é/c2.txt",
            "supprimé par eux.txt",
            "tranquille.txt"
        ]
    );
}

#[test]
fn keeping_ours_and_concluding_with_a_message() {
    let test = conflicted_merge();
    let repo = test.repo();
    let git = test.git();
    let paths = vec![
        test.path("c1.txt"),
        test.path("dossier é/c2.txt"),
        test.path("supprimé par eux.txt"),
    ];
    let done = accept_side(&git, &repo, &paths, ConflictSide::Ours, &cancel()).unwrap();
    assert!(done.iter().all(|d| !d.deleted));
    assert_eq!(test.read("supprimé par eux.txt"), b"notre 3\n");
    assert_eq!(test.read("c1.txt"), b"notre 1\n");
    let outcome = continue_operation(&git, &repo, Some("Fusion à la main"), &cancel()).unwrap();
    assert!(outcome.finished);
    assert_eq!(
        test.run(&["log", "-1", "--format=%s"]).trim(),
        "Fusion à la main"
    );
}

#[test]
fn deleted_by_us_and_mark_resolved() {
    let test = TestRepo::new();
    test.write("f.txt", "base\n");
    test.commit_all("base");
    test.run(&["checkout", "-q", "-b", "eux"]);
    test.write("f.txt", "leur\n");
    test.commit_all("leur");
    test.run(&["checkout", "-q", "main"]);
    test.run(&["rm", "-q", "f.txt"]);
    test.run(&["commit", "-q", "-m", "supprimé chez nous"]);
    assert!(!test.try_run(&["merge", "eux"]).status.success());

    let repo = test.repo();
    let git = test.git();
    let versions = conflict_versions(&git, &repo, &test.path("f.txt"), &cancel()).unwrap();
    assert_eq!(versions.kind, ConflictKind::DeletedByUs);
    assert_eq!(versions.ours, Content::Missing);
    // Résolu à la main hors de l'IDE, puis marqué : le fichier gardé est indexé.
    test.write("f.txt", "gardé\n");
    mark_resolved(&git, &repo, &[test.path("f.txt")], &cancel()).unwrap();
    assert!(unmerged_paths(&git, &repo, &cancel()).unwrap().is_empty());
    assert_eq!(test.run(&["show", ":f.txt"]), "gardé\n");
}

#[test]
fn aborting_restores_the_state_before_the_merge() {
    let test = conflicted_merge();
    let before = test.head();
    let repo = test.repo();
    let git = test.git();
    assert_eq!(
        abort_operation(&git, &repo, &cancel()).unwrap(),
        OperationKind::Merge
    );
    let state = status(&git, &repo, &cancel()).unwrap();
    assert!(state.is_clean() && state.operation.is_none(), "{state:?}");
    assert_eq!(test.head(), before);
    assert_eq!(test.read("c1.txt"), b"notre 1\n");
    assert!(matches!(
        abort_operation(&git, &repo, &cancel()),
        Err(VcsError::NoOperationInProgress)
    ));
}

#[test]
fn cherry_pick_and_rebase_conflicts_are_detected_and_continued() {
    let test = TestRepo::new();
    test.write("f.txt", "base\n");
    test.commit_all("base");
    test.run(&["checkout", "-q", "-b", "sujet"]);
    test.write("f.txt", "sujet 1\n");
    test.commit_all("sujet 1");
    test.write("g.txt", "sujet 2\n");
    test.commit_all("sujet 2");
    test.run(&["checkout", "-q", "main"]);
    test.write("f.txt", "main\n");
    test.commit_all("main");
    let repo = test.repo();
    let git = test.git();

    // Cherry-pick en conflit, puis abandon.
    assert!(!test.try_run(&["cherry-pick", "sujet~1"]).status.success());
    let state = status(&git, &repo, &cancel()).unwrap();
    assert_eq!(
        state.operation.as_ref().unwrap().kind,
        OperationKind::CherryPick
    );
    assert_eq!(
        abort_operation(&git, &repo, &cancel()).unwrap(),
        OperationKind::CherryPick
    );

    // Rebase de `sujet` sur `main` : conflit sur le premier commit.
    test.run(&["checkout", "-q", "sujet"]);
    assert!(!test.try_run(&["rebase", "main"]).status.success());
    let state = status(&git, &repo, &cancel()).unwrap();
    let operation = state.operation.clone().unwrap();
    assert_eq!(operation.kind, OperationKind::Rebase);
    assert_eq!(operation.rebase_branch.as_deref(), Some("sujet"));
    assert!(matches!(state.head, Head::Detached { .. }));
    let versions = conflict_versions(&git, &repo, &test.path("f.txt"), &cancel()).unwrap();
    // Pendant un rebase, « la nôtre » est la branche sur laquelle on rejoue.
    assert_eq!(versions.ours.bytes(), Some(&b"main\n"[..]));
    assert_eq!(versions.theirs.bytes(), Some(&b"sujet 1\n"[..]));
    accept_side(
        &git,
        &repo,
        &[test.path("f.txt")],
        ConflictSide::Theirs,
        &cancel(),
    )
    .unwrap();
    let outcome = continue_operation(&git, &repo, None, &cancel()).unwrap();
    assert!(outcome.finished, "{outcome:?}");
    let state = status(&git, &repo, &cancel()).unwrap();
    assert!(matches!(&state.head, Head::Branch { name, .. } if name == "sujet"));
    assert_eq!(
        test.run(&["log", "-3", "--format=%s"]),
        "sujet 2\nsujet 1\nmain\n"
    );
}

#[test]
fn a_file_deleted_then_recreated_untracked_can_be_committed_back() {
    let test = TestRepo::new();
    test.write("revenant.txt", "1\n");
    test.commit_all("premier");
    test.run(&["rm", "-q", "revenant.txt"]);
    test.write("revenant.txt", "2\n");
    let repo = test.repo();
    let state = status(&test.git(), &repo, &cancel()).unwrap();
    // Comme Git : supprimé de l'index, et non suivi sur le disque.
    assert_eq!(
        kinds(&state),
        vec![
            ("revenant.txt".to_owned(), FileKind::Deleted),
            ("revenant.txt".to_owned(), FileKind::Untracked),
        ]
    );
    commit(
        &test.git(),
        &repo,
        &CommitRequest {
            paths: vec![test.path("revenant.txt")],
            message: "Revenu".into(),
            amend: false,
        },
        &cancel(),
    )
    .unwrap();
    assert_eq!(test.run(&["show", "HEAD:revenant.txt"]), "2\n");
    assert!(status(&test.git(), &repo, &cancel()).unwrap().is_clean());
}

#[test]
fn five_thousand_untracked_files_are_listed_quickly() {
    let test = TestRepo::new();
    for index in 0..5_000 {
        test.write(&format!("lot {}/fichier é {index}.txt", index / 500), "x");
    }
    let repo = test.repo();
    let started = std::time::Instant::now();
    let state = status(&test.git(), &repo, &cancel()).unwrap();
    let elapsed = started.elapsed();
    assert_eq!(state.untracked().count(), 5_000);
    assert!(elapsed < std::time::Duration::from_secs(2), "{elapsed:?}");
}

// ---------------------------------------------------------------- journal des commandes

#[test]
fn every_command_is_logged() {
    let test = TestRepo::new();
    test.write("a.txt", "1\n");
    let git = test.git();
    let repo = open_repository(&git, &test.root, &cancel()).unwrap();
    status(&git, &repo, &cancel()).unwrap();
    let entries = git.log().entries();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].args[0], "rev-parse");
    assert_eq!(
        entries[1].args[..2],
        ["status".to_owned(), "--porcelain=v2".to_owned()]
    );
    let result = entries[1].result.as_ref().unwrap();
    assert_eq!(result.exit_code, Some(0));
    assert_eq!(result.outcome, CommandOutcome::Exited);
    assert!(result.stdout.contains("a.txt"));
    assert_eq!(entries[1].cwd, test.root);
}

// ---------------------------------------------------------------- labmate_paper (clone local)

const LABMATE: &str = r"C:\Program Files (x86)\AgileBio\LabCollector\www\labmate_paper";

/// Clone local de labmate_paper dans le dossier temporaire ; l'original n'est
/// que lu. Les branches d'essai `juliestorm-essai/nous` et `eux` donnent un
/// conflit sur README.md.
#[test]
fn labmate_paper_clone_conflict_on_readme() {
    if !std::path::Path::new(LABMATE).join(".git").is_dir() {
        eprintln!("labmate_paper absent : test sauté");
        return;
    }
    let test = TestRepo::empty_dir();
    let clone = test.base.join("labmate clone é");
    let clone_text = clone.to_string_lossy().into_owned();
    let output = test.raw_in(
        &test.base,
        &[
            "-c",
            "safe.directory=*",
            "clone",
            "-q",
            "--no-hardlinks",
            LABMATE,
            &clone_text,
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = test.raw_in(
        &clone,
        &[
            "-c",
            "safe.directory=*",
            "fetch",
            "-q",
            LABMATE,
            "refs/heads/juliestorm-essai/*:refs/heads/juliestorm-essai/*",
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(test
        .raw_in(&clone, &["checkout", "-q", "juliestorm-essai/nous"])
        .status
        .success());
    let merge = test.raw_in(&clone, &["merge", "juliestorm-essai/eux"]);
    assert!(!merge.status.success(), "conflit attendu");

    let git = test.git();
    let repo = open_repository(&git, &clone, &cancel()).unwrap();
    assert_eq!(repo.work_tree(), clone.as_path());
    let state = status(&git, &repo, &cancel()).unwrap();
    assert_eq!(state.operation.as_ref().unwrap().kind, OperationKind::Merge);
    let conflicts: Vec<_> = state.conflicts().map(|f| f.repo_path.as_str()).collect();
    assert!(conflicts.contains(&"README.md"), "{conflicts:?}");
    let versions = conflict_versions(&git, &repo, &clone.join("README.md"), &cancel()).unwrap();
    assert!(versions.ours.bytes().is_some() && versions.theirs.bytes().is_some());
    assert_ne!(versions.ours, versions.theirs);
    let page = log(
        &git,
        &repo,
        &LogQuery {
            limit: 5,
            ..LogQuery::default()
        },
        &cancel(),
    )
    .unwrap();
    assert!(!page.commits.is_empty());
    assert_eq!(
        abort_operation(&git, &repo, &cancel()).unwrap(),
        OperationKind::Merge
    );
    assert!(status(&git, &repo, &cancel()).unwrap().is_clean());
}
