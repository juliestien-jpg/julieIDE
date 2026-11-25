//! Branches, distant et opérations sur des dépôts jetables : un dépôt nu local
//! en guise de distant, le clone A (testé) et le clone B (la collègue).
//! Critères d'acceptation de `docs/specs/branches.md` (numéros entre
//! parenthèses).

mod support;

use std::time::{Duration, Instant};

use julie_core::CancelToken;
use julie_vcs::*;
use support::{cancel, Team, TestRepo};

fn no_progress() -> impl FnMut(RemoteProgress) {
    |_| {}
}

fn list(test: &TestRepo) -> BranchList {
    list_branches(&test.git(), &test.repo(), &cancel()).unwrap()
}

fn fetch_all(test: &TestRepo) -> FetchOutcome {
    fetch(
        &test.git(),
        &test.repo(),
        &FetchRequest::default(),
        &mut no_progress(),
        &cancel(),
    )
    .unwrap()
}

fn switch(
    test: &TestRepo,
    target: SwitchTarget,
    local_changes: LocalChanges,
) -> Result<SwitchOutcome> {
    switch_branch(
        &test.git(),
        &test.repo(),
        &SwitchRequest {
            target,
            local_changes,
        },
        &cancel(),
    )
}

fn create(
    test: &TestRepo,
    name: &str,
    start: Option<&str>,
    switch: bool,
) -> Result<CreateBranchOutcome> {
    create_branch(
        &test.git(),
        &test.repo(),
        &CreateBranchRequest {
            name: name.into(),
            start: start.map(str::to_owned),
            switch,
            local_changes: LocalChanges::Keep,
        },
        &cancel(),
    )
}

fn stash_list(test: &TestRepo) -> String {
    test.run(&["stash", "list"])
}

/// Les branches de départ du critère : `feature/login`, `feature/search`,
/// `bugfix/crash`, `juliestorm-essai/a`, chacune avec un commit, poussées.
fn team_with_branches() -> Team {
    let team = Team::new();
    let a = &team.a;
    for (branch, file) in [
        ("feature/login", "login.txt"),
        ("feature/search", "search.txt"),
        ("bugfix/crash", "crash.txt"),
        ("juliestorm-essai/a", "essai.txt"),
    ] {
        a.run(&["switch", "-q", "-c", branch, "main"]);
        a.write(file, format!("{branch}\n"));
        a.commit_all(&format!("Travail sur {branch}"));
        a.run(&["push", "-q", "-u", "origin", branch]);
    }
    a.run(&["switch", "-q", "main"]);
    team
}

// ---------------------------------------------------------------- liste

#[test]
fn ahead_and_behind_follow_fetches_and_local_commits() {
    // (1)
    let team = Team::new();
    team.b_commit("b1.txt", "1\n", "B un");
    team.b_commit("b2.txt", "2\n", "B deux");
    team.b_run(&["push", "-q"]);
    let before = list(&team.a);
    let main = before.find_local("main").unwrap();
    assert_eq!(
        main.tracking.as_ref().map(|t| (t.ahead, t.behind)),
        Some((0, 0))
    );

    let fetched = fetch_all(&team.a);
    assert_eq!(fetched.updated(), 1, "{fetched:?}");
    let main = list(&team.a).find_local("main").cloned().unwrap();
    assert_eq!(
        main.tracking.as_ref().map(|t| (t.ahead, t.behind)),
        Some((0, 2))
    );
    assert_eq!(main.tracking.as_ref().unwrap().upstream, "origin/main");

    team.a.write("local.txt", "x\n");
    team.a.commit_all("A local");
    let after = list(&team.a);
    let main = after.find_local("main").unwrap();
    assert_eq!(
        main.tracking.as_ref().map(|t| (t.ahead, t.behind)),
        Some((1, 2))
    );
    assert!(main.current);
    assert_eq!(main.last_commit.subject, "A local");
    assert_eq!(after.current.as_deref(), Some("main"));
    assert_eq!(after.remotes, vec!["origin".to_owned()]);
    assert!(after.remote.iter().all(|b| b.name != "origin/HEAD"));
    assert!(after.find_remote("origin/main").is_some());
}

#[test]
fn branches_are_listed_in_prefix_folders() {
    // (2) côté données : l'arbre par préfixe.
    let team = team_with_branches();
    let branches = list(&team.a);
    let names: Vec<&str> = branches.local.iter().map(|b| b.name.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "bugfix/crash",
            "feature/login",
            "feature/search",
            "juliestorm-essai/a",
            "main"
        ]
    );
    let tree = branch_tree(&names);
    assert!(matches!(&tree[0], BranchNode::Branch { label, .. } if label == "bugfix/crash"));
    assert!(
        matches!(&tree[1], BranchNode::Folder { name, children, .. } if name == "feature" && children.len() == 2)
    );
}

#[test]
fn recent_branches_come_from_the_reflog_including_the_terminal() {
    // (3)
    let team = team_with_branches();
    let a = &team.a;
    for branch in ["feature/login", "bugfix/crash", "main"] {
        switch(a, SwitchTarget::Branch(branch.into()), LocalChanges::Keep).unwrap();
    }
    assert_eq!(
        list(a).recent,
        vec![
            "bugfix/crash",
            "feature/login",
            "juliestorm-essai/a",
            "feature/search"
        ]
    );
    // Une bascule faite dans le terminal compte aussi.
    a.run(&["switch", "-q", "feature/search"]);
    a.run(&["switch", "-q", "main"]);
    assert_eq!(list(a).recent[0], "feature/search");
}

#[test]
fn a_repository_without_commits_lists_its_unborn_branch() {
    let test = TestRepo::new();
    let branches = list(&test);
    assert!(branches.unborn);
    assert_eq!(branches.current.as_deref(), Some("main"));
    assert!(branches.local.is_empty());
    // Nouvelle branche : renomme la branche à naître, sans toucher à l'index.
    test.write("indexé.txt", "x");
    test.run(&["add", "indexé.txt"]);
    let created = create(&test, "départ", None, true).unwrap();
    assert_eq!(created.commit, None);
    assert_eq!(list(&test).current.as_deref(), Some("départ"));
    assert!(test
        .run(&["-c", "core.quotepath=false", "ls-files"])
        .contains("indexé.txt"));
    assert!(matches!(
        create(&test, "autre", None, false),
        Err(VcsError::NoCommits)
    ));
}

// ---------------------------------------------------------------- créer, noms

#[test]
fn branch_names_are_checked_before_creation() {
    // (5)
    let team = team_with_branches();
    let (git, repo) = (team.a.git(), team.a.repo());
    let check = |name: &str| check_branch_name(&git, &repo, name, None, &cancel()).unwrap();
    assert_eq!(
        check("ma branche"),
        Some(BranchNameProblem::Invalid {
            suggestion: Some("ma-branche".into())
        })
    );
    assert_eq!(
        check("feature"),
        Some(BranchNameProblem::FolderOfExisting("feature/login".into()))
    );
    assert_eq!(
        check("Main"),
        Some(BranchNameProblem::SameNameDifferentCase("main".into()))
    );
    assert_eq!(check(""), Some(BranchNameProblem::Empty));
    assert!(matches!(
        check("a..b"),
        Some(BranchNameProblem::Invalid { .. })
    ));
    assert!(matches!(
        check("fin.lock"),
        Some(BranchNameProblem::Invalid { .. })
    ));
    assert_eq!(check("fonctionnalité/é"), None);
    assert!(matches!(
        create(&team.a, "ma branche", None, true),
        Err(VcsError::InvalidBranchName(_))
    ));

    let crash = team.a.rev("bugfix/crash");
    let created = create(&team.a, "feature/x", Some("bugfix/crash"), true).unwrap();
    assert_eq!(created.commit.as_deref(), Some(crash.as_str()));
    assert_eq!(team.a.head(), crash);
    assert_eq!(list(&team.a).current.as_deref(), Some("feature/x"));
}

#[test]
fn a_branch_can_be_created_on_an_old_commit_without_switching() {
    // (20)
    let team = team_with_branches();
    let a = &team.a;
    let first = a.rev("main");
    a.write("plus.txt", "+");
    a.commit_all("Plus");
    let created = create(a, "essai", Some(&first), false).unwrap();
    assert!(created.switched.is_none());
    assert_eq!(a.rev("essai"), first);
    assert_eq!(list(a).current.as_deref(), Some("main"));
}

#[test]
fn creating_from_a_remote_branch_tracks_it() {
    let team = team_with_branches();
    let created = create(&team.a, "copie", Some("origin/feature/login"), false).unwrap();
    assert_eq!(created.upstream.as_deref(), Some("origin/feature/login"));
}

#[test]
fn local_changes_follow_a_new_branch_from_head() {
    let team = Team::new();
    team.a.write("a.txt", "modifié\n");
    let created = create(&team.a, "suite", None, true).unwrap();
    assert!(created.switched.unwrap().created);
    assert_eq!(team.a.read("a.txt"), b"modifi\xc3\xa9\n");
}

// ---------------------------------------------------------------- basculer

#[test]
fn switching_to_a_remote_only_branch_creates_a_tracking_local() {
    // (4)
    let team = Team::new();
    team.b_run(&["switch", "-q", "-c", "feature/remote-only"]);
    team.b_commit("r.txt", "r\n", "Distante");
    team.b_run(&["push", "-q", "-u", "origin", "feature/remote-only"]);
    fetch_all(&team.a);
    let outcome = switch(
        &team.a,
        SwitchTarget::Remote("origin/feature/remote-only".into()),
        LocalChanges::Keep,
    )
    .unwrap();
    assert!(outcome.created);
    assert_eq!(outcome.branch.as_deref(), Some("feature/remote-only"));
    let branches = list(&team.a);
    let local = branches.find_local("feature/remote-only").unwrap();
    assert!(local.current);
    assert_eq!(
        local.tracking.as_ref().unwrap().upstream,
        "origin/feature/remote-only"
    );

    // Une seconde fois : la locale existe, on bascule dessus ; en retard, on le dit.
    team.b_commit("r2.txt", "r2\n", "Encore");
    team.b_run(&["push", "-q"]);
    fetch_all(&team.a);
    switch(
        &team.a,
        SwitchTarget::Branch("main".into()),
        LocalChanges::Keep,
    )
    .unwrap();
    let again = switch(
        &team.a,
        SwitchTarget::Remote("origin/feature/remote-only".into()),
        LocalChanges::Keep,
    )
    .unwrap();
    assert!(!again.created);
    assert_eq!(again.behind_upstream, Some(1));
}

/// `a.txt` : trois lignes ; `autre` change la première ligne.
fn team_with_diverging_file() -> Team {
    let team = Team::new();
    let a = &team.a;
    a.run(&["switch", "-q", "-c", "autre"]);
    a.write("a.txt", "UN\ndeux\ntrois\n");
    a.commit_all("Autre première ligne");
    a.run(&["switch", "-q", "main"]);
    team
}

#[test]
fn switching_over_local_changes_names_the_files_and_changes_nothing() {
    // (6, premier temps)
    let team = team_with_diverging_file();
    let a = &team.a;
    a.write("a.txt", "un\ndeux\nTROIS\n");
    let head = a.head();
    let error = switch(a, SwitchTarget::Branch("autre".into()), LocalChanges::Keep).unwrap_err();
    match error {
        VcsError::WouldOverwrite(conflict) => {
            assert_eq!(conflict.tracked, vec![a.path("a.txt")]);
            assert!(conflict.untracked.is_empty());
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(a.head(), head);
    assert_eq!(a.read("a.txt"), b"un\ndeux\nTROIS\n");
    assert_eq!(list(a).current.as_deref(), Some("main"));
}

#[test]
fn stash_and_switch_restores_the_changes_and_drops_the_entry() {
    // (6)
    let team = team_with_diverging_file();
    let a = &team.a;
    a.write("a.txt", "un\ndeux\nTROIS\n");
    a.write("nouveau non suivi.txt", "n");
    let outcome = switch(a, SwitchTarget::Branch("autre".into()), LocalChanges::Stash).unwrap();
    assert_eq!(outcome.branch.as_deref(), Some("autre"));
    let stash = outcome.stash.expect("mise de côté");
    assert_eq!(stash.restore, StashRestore::Restored);
    assert!(
        stash
            .message
            .starts_with("JulieStorm : avant bascule de main vers autre"),
        "{}",
        stash.message
    );
    assert_eq!(a.read("a.txt"), b"UN\ndeux\nTROIS\n");
    assert_eq!(a.read("nouveau non suivi.txt"), b"n");
    assert_eq!(stash_list(a), "");
}

#[test]
fn a_conflicting_restore_keeps_the_stash_entry() {
    // (7)
    let team = team_with_diverging_file();
    let a = &team.a;
    a.write("a.txt", "Un modifié ici\ndeux\ntrois\n");
    let outcome = switch(a, SwitchTarget::Branch("autre".into()), LocalChanges::Stash).unwrap();
    assert_eq!(outcome.branch.as_deref(), Some("autre"));
    let stash = outcome.stash.unwrap();
    match &stash.restore {
        StashRestore::Conflicts(files) => assert_eq!(files, &vec![a.path("a.txt")]),
        other => panic!("{other:?}"),
    }
    assert!(stash.kept());
    assert!(stash_list(a).contains("JulieStorm : avant bascule de main vers autre"));
    let entries = list_stashes(&a.git(), &a.repo(), &cancel()).unwrap();
    assert_eq!(entries.len(), 1);
    assert!(entries[0].automatic);
    assert_eq!(entries[0].commit, stash.commit);
    // Le dialogue de fusion existant peut s'ouvrir sur ce fichier.
    let versions = conflict_versions(&a.git(), &a.repo(), &a.path("a.txt"), &cancel()).unwrap();
    assert_eq!(versions.kind, ConflictKind::BothModified);
}

#[test]
fn forcing_the_switch_discards_only_the_named_files() {
    // (8)
    let team = team_with_diverging_file();
    let a = &team.a;
    a.write("a.txt", "un\ndeux\nTROIS\n");
    a.write("b.txt", "autre changement\n");
    let conflict = match switch(a, SwitchTarget::Branch("autre".into()), LocalChanges::Keep) {
        Err(VcsError::WouldOverwrite(conflict)) => conflict,
        other => panic!("{other:?}"),
    };
    let outcome = switch_discarding_changes(
        &a.git(),
        &a.repo(),
        &SwitchTarget::Branch("autre".into()),
        &conflict,
        &cancel(),
    )
    .unwrap();
    assert_eq!(outcome.branch.as_deref(), Some("autre"));
    assert_eq!(a.read("a.txt"), b"UN\ndeux\ntrois\n");
    assert_eq!(
        a.read("b.txt"),
        b"autre changement\n",
        "les autres changements suivent"
    );
}

#[test]
fn an_untracked_file_in_the_way_is_reported() {
    let team = Team::new();
    let a = &team.a;
    a.run(&["switch", "-q", "-c", "avec-fichier"]);
    a.write("gêneur.txt", "suivi\n");
    a.commit_all("Ajout");
    a.run(&["switch", "-q", "main"]);
    a.write("gêneur.txt", "non suivi\n");
    match switch(
        a,
        SwitchTarget::Branch("avec-fichier".into()),
        LocalChanges::Keep,
    ) {
        Err(VcsError::WouldOverwrite(conflict)) => {
            assert_eq!(conflict.untracked, vec![a.path("gêneur.txt")]);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn switching_is_refused_during_a_merge() {
    let team = team_with_conflicting_login();
    let a = &team.a;
    let outcome = do_merge(a, "feature/login");
    assert!(!outcome.finished);
    assert!(matches!(
        switch(
            a,
            SwitchTarget::Branch("feature/login".into()),
            LocalChanges::Keep
        ),
        Err(VcsError::OperationInProgress(OperationKind::Merge))
    ));
}

#[test]
fn a_failing_post_checkout_hook_does_not_undo_the_switch() {
    let team = team_with_branches();
    let a = &team.a;
    std::fs::write(
        a.path(".git/hooks/post-checkout"),
        "#!/bin/sh\necho \"crochet après bascule en échec\" >&2\nexit 1\n",
    )
    .unwrap();
    let outcome = switch(
        a,
        SwitchTarget::Branch("feature/login".into()),
        LocalChanges::Keep,
    )
    .unwrap();
    assert_eq!(outcome.branch.as_deref(), Some("feature/login"));
    assert!(outcome
        .hook_output
        .unwrap()
        .contains("crochet après bascule en échec"));
}

#[test]
fn a_detached_head_counts_its_orphan_commits() {
    // (24)
    let team = team_with_branches();
    let a = &team.a;
    let first = a.rev("main");
    let outcome = switch(a, SwitchTarget::Detach(first.clone()), LocalChanges::Keep).unwrap();
    assert_eq!(outcome.branch, None);
    assert_eq!(outcome.head, first);
    let branches = list(a);
    assert_eq!(branches.current, None);
    assert_eq!(branches.detached_at.as_deref(), Some(first.as_str()));
    assert!(matches!(
        push_target(&a.git(), &a.repo(), None, &cancel()),
        Err(VcsError::DetachedHead)
    ));
    a.write("détaché.txt", "d");
    a.commit_all("Sur aucune branche");
    assert_eq!(
        commits_only_on_detached_head(&a.git(), &a.repo(), &cancel()).unwrap(),
        1
    );
    // Quitter sans accord est refusé (recette v1, M3) ; avec l'accord, on part.
    assert!(matches!(
        switch(a, SwitchTarget::Branch("main".into()), LocalChanges::Keep),
        Err(VcsError::DetachedCommitsLeft { count: 1, .. })
    ));
    switch_branch_with(
        &a.git(),
        &a.repo(),
        &SwitchRequest {
            target: SwitchTarget::Branch("main".into()),
            local_changes: LocalChanges::Keep,
        },
        SwitchOptions {
            leave_detached_commits: true,
        },
        &cancel(),
    )
    .unwrap();
    assert_eq!(
        commits_only_on_detached_head(&a.git(), &a.repo(), &cancel()).unwrap(),
        0
    );
}

// ---------------------------------------------------------------- renommer, supprimer

#[test]
fn renaming_reports_the_upstream_that_keeps_its_name() {
    let team = team_with_branches();
    let a = &team.a;
    let renamed = rename_branch(
        &a.git(),
        &a.repo(),
        "feature/login",
        "feature/connexion",
        &cancel(),
    )
    .unwrap();
    assert_eq!(renamed.upstream.as_deref(), Some("origin/feature/login"));
    let branches = list(a);
    assert!(branches.find_local("feature/connexion").is_some());
    assert!(branches.find_local("feature/login").is_none());
    assert!(matches!(
        rename_branch(&a.git(), &a.repo(), "feature/connexion", "main", &cancel()),
        Err(VcsError::InvalidBranchName(
            BranchNameProblem::AlreadyExists(_)
        ))
    ));
}

#[test]
fn deleting_an_unmerged_branch_asks_then_can_be_restored() {
    // (18)
    let team = team_with_branches();
    let a = &team.a;
    a.run(&["switch", "-q", "bugfix/crash"]);
    a.write("crash2.txt", "non poussé\n");
    a.commit_all("Correctif non poussé");
    a.run(&["switch", "-q", "main"]);
    let tip = a.rev("bugfix/crash");
    let error = delete_branch(&a.git(), &a.repo(), "bugfix/crash", &cancel()).unwrap_err();
    match error {
        VcsError::BranchNotMerged {
            commits,
            subjects,
            tip: reported,
            ..
        } => {
            assert_eq!(commits, 1);
            assert_eq!(subjects, vec!["Correctif non poussé".to_owned()]);
            assert_eq!(reported, tip);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(a.rev("bugfix/crash"), tip, "rien de supprimé");
    let deleted = force_delete_branch(&a.git(), &a.repo(), "bugfix/crash", &cancel()).unwrap();
    assert_eq!(deleted.tip, tip);
    assert_eq!(deleted.upstream.as_deref(), Some("origin/bugfix/crash"));
    assert!(list(a).find_local("bugfix/crash").is_none());
    restore_branch(&a.git(), &a.repo(), "bugfix/crash", &deleted.tip, &cancel()).unwrap();
    assert_eq!(a.rev("bugfix/crash"), tip);
    assert!(matches!(
        delete_branch(&a.git(), &a.repo(), "main", &cancel()),
        Err(VcsError::CurrentBranch(_))
    ));
}

#[test]
fn deleting_a_remote_branch_respects_protection() {
    // (19)
    let team = team_with_branches();
    let a = &team.a;
    let protected = VcsSettings::default().protected_branches;
    assert!(matches!(
        delete_remote_branch(
            &a.git(),
            &a.repo(),
            "origin",
            "main",
            &protected,
            &mut no_progress(),
            &cancel()
        ),
        Err(VcsError::ProtectedBranch(_))
    ));
    assert!(team.bare_tip("main").is_some());
    let deleted = delete_remote_branch(
        &a.git(),
        &a.repo(),
        "origin",
        "feature/search",
        &protected,
        &mut no_progress(),
        &cancel(),
    )
    .unwrap();
    assert!(deleted.tip.is_some());
    assert_eq!(team.bare_tip("feature/search"), None);
    assert!(list(a).find_remote("origin/feature/search").is_none());
}

// ---------------------------------------------------------------- fetch, mise à jour, pull

#[test]
fn update_by_rebase_keeps_history_linear_and_local_changes() {
    // (9, 10)
    let team = Team::new();
    team.b_commit("b1.txt", "1\n", "B un");
    team.b_commit("b2.txt", "2\n", "B deux");
    team.b_run(&["push", "-q"]);
    let a = &team.a;
    a.write("local.txt", "commit local\n");
    a.commit_all("A local");
    a.write("a.txt", "un\ndeux\ntrois\nquatre non validé\n");
    let outcome = update_project(
        &a.git(),
        &a.repo(),
        &UpdateRequest {
            method: IntegrationMethod::Rebase,
            local_changes: LocalChanges::Stash,
        },
        &mut no_progress(),
        &cancel(),
    )
    .unwrap();
    assert!(outcome.finished);
    assert_eq!(outcome.received, 2);
    assert_eq!(
        outcome.stash.as_ref().map(|s| &s.restore),
        Some(&StashRestore::Restored)
    );
    let log = a.run(&["log", "--format=%s"]);
    assert_eq!(
        log.lines().collect::<Vec<_>>(),
        vec!["A local", "B deux", "B un", "Premier"]
    );
    assert_eq!(
        a.run(&["rev-list", "--merges", "--count", "HEAD"]).trim(),
        "0"
    );
    assert_eq!(
        a.read("a.txt"),
        b"un\ndeux\ntrois\nquatre non valid\xc3\xa9\n"
    );
    assert_eq!(stash_list(a), "");
    // Déjà à jour ensuite.
    let again = update_project(
        &a.git(),
        &a.repo(),
        &UpdateRequest {
            method: IntegrationMethod::Rebase,
            local_changes: LocalChanges::Stash,
        },
        &mut no_progress(),
        &cancel(),
    )
    .unwrap();
    assert!(again.up_to_date);
}

#[test]
fn update_by_merge_reports_what_arrived() {
    let team = Team::new();
    team.b_commit("b1.txt", "1\n", "B un");
    team.b_run(&["push", "-q"]);
    let a = &team.a;
    let outcome = update_project(
        &a.git(),
        &a.repo(),
        &UpdateRequest {
            method: IntegrationMethod::Merge,
            local_changes: LocalChanges::Keep,
        },
        &mut no_progress(),
        &cancel(),
    )
    .unwrap();
    assert!(outcome.finished && outcome.fast_forward);
    assert_eq!((outcome.received, outcome.changed_files), (1, 1));
    assert!(outcome.fetch.is_some());
}

#[test]
fn update_without_upstream_runs_neither_fetch_nor_merge() {
    // (11)
    let team = Team::new();
    let a = &team.a;
    a.run(&["switch", "-q", "-c", "sans-amont"]);
    let git = a.git();
    git.log().clear();
    let error = update_project(
        &git,
        &a.repo(),
        &UpdateRequest {
            method: IntegrationMethod::Merge,
            local_changes: LocalChanges::Stash,
        },
        &mut no_progress(),
        &cancel(),
    )
    .unwrap_err();
    assert!(
        matches!(error, VcsError::NoUpstream { ref branch } if branch == "sans-amont"),
        "{error:?}"
    );
    for record in git.log().entries() {
        let command = record.args.first().map(String::as_str);
        assert!(
            !matches!(
                command,
                Some("fetch" | "merge" | "rebase" | "pull" | "stash")
            ),
            "{:?}",
            record.args
        );
    }
}

#[test]
fn a_deleted_upstream_is_marked_gone_and_explained() {
    // (25)
    let team = team_with_branches();
    team.b_run(&["push", "-q", "origin", "--delete", "feature/login"]);
    let a = &team.a;
    let fetched = fetch_all(a);
    assert_eq!(fetched.pruned(), 1);
    let branches = list(a);
    assert!(
        branches
            .find_local("feature/login")
            .unwrap()
            .tracking
            .as_ref()
            .unwrap()
            .gone
    );
    a.run(&["switch", "-q", "feature/login"]);
    let error = update_project(
        &a.git(),
        &a.repo(),
        &UpdateRequest {
            method: IntegrationMethod::Merge,
            local_changes: LocalChanges::Keep,
        },
        &mut no_progress(),
        &cancel(),
    )
    .unwrap_err();
    assert!(
        matches!(error, VcsError::UpstreamGone { ref upstream, .. } if upstream == "origin/feature/login"),
        "{error:?}"
    );
}

#[test]
fn a_wrong_remote_url_fails_with_gits_message_and_changes_nothing() {
    // (26)
    let team = Team::new();
    let a = &team.a;
    let missing = a.base.join("nulle part.git");
    a.run(&["remote", "set-url", "origin", missing.to_str().unwrap()]);
    let head = a.head();
    let error = fetch(
        &a.git(),
        &a.repo(),
        &FetchRequest::default(),
        &mut no_progress(),
        &cancel(),
    )
    .unwrap_err();
    match error {
        VcsError::GitFailed { stderr, .. } => assert!(
            stderr.contains("does not appear to be a git repository"),
            "{stderr}"
        ),
        other => panic!("{other:?}"),
    }
    assert_eq!(a.head(), head);
}

/// Un faux `ssh` (script) : jamais de réseau.
fn fake_ssh(test: &TestRepo, name: &str, body: &str) -> String {
    let script = test.base.join(name);
    std::fs::write(&script, format!("#!/bin/sh\n{body}\n")).unwrap();
    format!("\"{}\"", script.to_string_lossy().replace('\\', "/"))
}

#[test]
fn an_authentication_failure_gets_its_own_error() {
    let team = Team::new();
    let a = &team.a;
    let ssh = fake_ssh(
        a,
        "faux ssh.sh",
        "echo 'git@example.invalid: Permission denied (publickey).' >&2\nexit 255",
    );
    a.run(&["config", "core.sshCommand", &ssh]);
    a.run(&[
        "remote",
        "add",
        "forge",
        "ssh://git@example.invalid/depot.git",
    ]);
    let error = fetch(
        &a.git(),
        &a.repo(),
        &FetchRequest {
            remote: Some("forge".into()),
            prune: false,
        },
        &mut no_progress(),
        &cancel(),
    )
    .unwrap_err();
    assert!(
        matches!(error, VcsError::AuthenticationFailed { .. }),
        "{error:?}"
    );
    assert!(is_authentication_failure(
        "fatal: could not read Username for 'https://x': terminal prompts disabled"
    ));
    assert!(!is_authentication_failure("fatal: 'x' does not appear to be a git repository\nfatal: Could not read from remote repository."));
}

#[test]
fn a_slow_network_operation_is_cancelled_quickly() {
    // (27)
    let team = Team::new();
    let a = &team.a;
    let ssh = fake_ssh(a, "ssh lent.sh", "sleep 30");
    a.run(&["config", "core.sshCommand", &ssh]);
    a.run(&[
        "remote",
        "add",
        "lent",
        "ssh://git@example.invalid/lent.git",
    ]);
    let token = CancelToken::new();
    let trigger = token.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(400));
        trigger.cancel();
    });
    let started = Instant::now();
    let error = fetch(
        &a.git(),
        &a.repo(),
        &FetchRequest {
            remote: Some("lent".into()),
            prune: false,
        },
        &mut no_progress(),
        &token,
    )
    .unwrap_err();
    assert!(error.is_cancelled(), "{error:?}");
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
}

#[test]
fn fetch_reports_progress() {
    let team = Team::new();
    for i in 0..30 {
        team.b_commit(
            &format!("f{i}.txt"),
            &"x".repeat(2_000 + i),
            &format!("c{i}"),
        );
    }
    team.b_run(&["push", "-q"]);
    let a = &team.a;
    let mut seen = Vec::new();
    fetch(
        &a.git(),
        &a.repo(),
        &FetchRequest::default(),
        &mut |p| seen.push(p),
        &cancel(),
    )
    .unwrap();
    assert!(!seen.is_empty(), "aucune progression");
    assert!(seen.iter().any(|p| p.total > 0));
}

#[test]
fn pull_by_merge_and_fast_forward_only() {
    let team = Team::new();
    team.b_commit("b.txt", "b\n", "B");
    team.b_run(&["push", "-q"]);
    let a = &team.a;
    a.write("l.txt", "l\n");
    a.commit_all("Local");
    let refused = pull(
        &a.git(),
        &a.repo(),
        &PullRequest {
            remote: "origin".into(),
            branch: "main".into(),
            mode: PullMode::FastForwardOnly,
            local_changes: LocalChanges::Keep,
        },
        &mut no_progress(),
        &cancel(),
    );
    assert!(refused.is_err(), "{refused:?}");
    let merged = pull(
        &a.git(),
        &a.repo(),
        &PullRequest {
            remote: "origin".into(),
            branch: "main".into(),
            mode: PullMode::Merge,
            local_changes: LocalChanges::Keep,
        },
        &mut no_progress(),
        &cancel(),
    )
    .unwrap();
    assert!(merged.finished);
    assert_eq!(merged.received, 1);
    assert_eq!(
        a.run(&["rev-list", "--merges", "--count", "HEAD"]).trim(),
        "1"
    );
}

#[test]
fn another_branch_is_fast_forwarded_without_switching() {
    let team = team_with_branches();
    team.b_run(&["fetch", "-q"]);
    team.b_run(&["switch", "-q", "feature/search"]);
    team.b_commit("s2.txt", "s2\n", "Recherche 2");
    team.b_run(&["push", "-q"]);
    let a = &team.a;
    fast_forward_branch(
        &a.git(),
        &a.repo(),
        "feature/search",
        &mut no_progress(),
        &cancel(),
    )
    .unwrap();
    assert_eq!(
        a.rev("feature/search"),
        team.bare_tip("feature/search").unwrap()
    );
    assert_eq!(list(a).current.as_deref(), Some("main"));
    // Divergée : refusée.
    a.run(&["switch", "-q", "feature/search"]);
    a.write("s3.txt", "local\n");
    a.commit_all("Local");
    a.run(&["switch", "-q", "main"]);
    team.b_commit("s4.txt", "s4\n", "Recherche 4");
    team.b_run(&["push", "-q"]);
    let before = a.rev("feature/search");
    let error = fast_forward_branch(
        &a.git(),
        &a.repo(),
        "feature/search",
        &mut no_progress(),
        &cancel(),
    )
    .unwrap_err();
    assert!(matches!(error, VcsError::Diverged { .. }), "{error:?}");
    assert_eq!(a.rev("feature/search"), before);
}

// ---------------------------------------------------------------- push

#[test]
fn pushing_a_new_branch_lists_outgoing_commits_and_sets_upstream() {
    // (12)
    let team = Team::new();
    let a = &team.a;
    a.run(&["switch", "-q", "-c", "feature/x"]);
    a.write("x1.txt", "1");
    a.commit_all("X un");
    a.write("x2.txt", "2");
    a.commit_all("X deux");
    let protected = VcsSettings::default().protected_branches;
    let target = push_target(&a.git(), &a.repo(), None, &cancel()).unwrap();
    assert_eq!(
        target,
        PushTarget {
            branch: "feature/x".into(),
            remote: "origin".into(),
            remote_branch: "feature/x".into()
        }
    );
    let preview = push_preview(&a.git(), &a.repo(), &target, &protected, &cancel()).unwrap();
    assert!(preview.new_branch && preview.sets_upstream && !preview.protected);
    let expected: Vec<String> = a
        .run(&["log", "--format=%H", "feature/x", "--not", "--remotes"])
        .lines()
        .map(str::to_owned)
        .collect();
    let listed: Vec<String> = preview.outgoing.iter().map(|c| c.id.clone()).collect();
    assert_eq!(listed, expected);
    let pushed = push(
        &a.git(),
        &a.repo(),
        &PushRequest {
            target,
            set_upstream: true,
            follow_tags: false,
            force: PushForce::No,
        },
        &protected,
        &mut no_progress(),
        &cancel(),
    )
    .unwrap();
    assert_eq!((pushed.kind, pushed.pushed), (PushedKind::New, 2));
    assert!(team.bare_run(&["branch"]).contains("feature/x"));
    assert_eq!(
        a.run(&["rev-parse", "--abbrev-ref", "feature/x@{u}"])
            .trim(),
        "origin/feature/x"
    );
    // Plus rien à pousser.
    let preview = push_preview(
        &a.git(),
        &a.repo(),
        &push_target(&a.git(), &a.repo(), None, &cancel()).unwrap(),
        &protected,
        &cancel(),
    )
    .unwrap();
    assert!(preview.outgoing.is_empty() && !preview.new_branch);
}

#[test]
fn a_non_fast_forward_push_is_explained_then_rebase_and_push_works() {
    // (13)
    let team = Team::new();
    team.b_commit("b.txt", "b\n", "Commit de B");
    team.b_run(&["push", "-q"]);
    let a = &team.a;
    a.write("a2.txt", "a\n");
    a.commit_all("Commit de A");
    let protected = VcsSettings::default().protected_branches;
    let request = PushRequest {
        target: push_target(&a.git(), &a.repo(), None, &cancel()).unwrap(),
        set_upstream: false,
        follow_tags: false,
        force: PushForce::No,
    };
    let before = team.bare_tip("main");
    match push(
        &a.git(),
        &a.repo(),
        &request,
        &protected,
        &mut no_progress(),
        &cancel(),
    ) {
        Err(VcsError::PushRejected {
            rejection: PushRejection::NonFastForward { .. },
            output,
        }) => {
            assert!(output.contains("rejected"), "{output}");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(team.bare_tip("main"), before);
    // « Rebaser puis pousser ».
    update_project(
        &a.git(),
        &a.repo(),
        &UpdateRequest {
            method: IntegrationMethod::Rebase,
            local_changes: LocalChanges::Stash,
        },
        &mut no_progress(),
        &cancel(),
    )
    .unwrap();
    let pushed = push(
        &a.git(),
        &a.repo(),
        &request,
        &protected,
        &mut no_progress(),
        &cancel(),
    )
    .unwrap();
    assert_eq!((pushed.kind, pushed.pushed), (PushedKind::FastForward, 1));
    let remote_log = team.bare_run(&["log", "--format=%s", "main"]);
    assert_eq!(
        remote_log.lines().take(2).collect::<Vec<_>>(),
        vec!["Commit de A", "Commit de B"]
    );
}

#[test]
fn force_with_lease_is_refused_on_protected_branches_and_when_stale() {
    // (14)
    let team = Team::new();
    let a = &team.a;
    let protected = VcsSettings::default().protected_branches;
    let lease = |target: PushTarget| PushRequest {
        target,
        set_upstream: false,
        follow_tags: false,
        force: PushForce::WithLease { expected: None },
    };
    let main = push_target(&a.git(), &a.repo(), None, &cancel()).unwrap();
    assert!(
        push_preview(&a.git(), &a.repo(), &main, &protected, &cancel())
            .unwrap()
            .protected
    );
    assert!(matches!(
        push(
            &a.git(),
            &a.repo(),
            &lease(main),
            &protected,
            &mut no_progress(),
            &cancel()
        ),
        Err(VcsError::ProtectedBranch(_))
    ));

    a.run(&["switch", "-q", "-c", "feature/x"]);
    a.write("x.txt", "1");
    a.commit_all("X un");
    a.write("x.txt", "2");
    a.commit_all("X deux");
    a.run(&["push", "-q", "-u", "origin", "feature/x"]);
    a.run(&["reset", "-q", "--hard", "HEAD~1"]);
    a.write("x.txt", "2 bis");
    a.commit_all("X deux bis");
    let target = push_target(&a.git(), &a.repo(), None, &cancel()).unwrap();
    let preview = push_preview(&a.git(), &a.repo(), &target, &protected, &cancel()).unwrap();
    assert_eq!(
        preview.remote_only, 1,
        "un commit du distant serait remplacé"
    );
    let forced = push(
        &a.git(),
        &a.repo(),
        &lease(target.clone()),
        &protected,
        &mut no_progress(),
        &cancel(),
    )
    .unwrap();
    assert_eq!(forced.kind, PushedKind::Forced);
    assert_eq!(team.bare_tip("feature/x"), Some(a.head()));

    // B pousse entre-temps : le bail tombe, rien n'est écrasé.
    team.b_run(&["fetch", "-q"]);
    team.b_run(&["switch", "-q", "feature/x"]);
    team.b_commit("y.txt", "y", "Par B");
    team.b_run(&["push", "-q"]);
    let remote_now = team.bare_tip("feature/x");
    a.write("x.txt", "3");
    a.commit_all("X trois");
    a.run(&["commit", "-q", "--amend", "-m", "X trois amendé"]);
    match push(
        &a.git(),
        &a.repo(),
        &lease(target),
        &protected,
        &mut no_progress(),
        &cancel(),
    ) {
        Err(VcsError::PushRejected {
            rejection: PushRejection::StaleLease,
            ..
        }) => {}
        other => panic!("{other:?}"),
    }
    assert_eq!(team.bare_tip("feature/x"), remote_now);
}

#[test]
fn a_failing_pre_push_hook_stops_the_push_and_shows_its_output() {
    // (15)
    let team = Team::new();
    let a = &team.a;
    std::fs::write(
        a.path(".git/hooks/pre-push"),
        "#!/bin/sh\necho \"refusé par le crochet\" >&2\nexit 1\n",
    )
    .unwrap();
    a.write("h.txt", "h");
    a.commit_all("Avec crochet");
    let before = team.bare_tip("main");
    let error = push(
        &a.git(),
        &a.repo(),
        &PushRequest {
            target: push_target(&a.git(), &a.repo(), None, &cancel()).unwrap(),
            set_upstream: false,
            follow_tags: false,
            force: PushForce::No,
        },
        &VcsSettings::default().protected_branches,
        &mut no_progress(),
        &cancel(),
    )
    .unwrap_err();
    match error {
        VcsError::GitFailed { stderr, .. } => {
            assert!(stderr.contains("refusé par le crochet"), "{stderr}")
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(team.bare_tip("main"), before);
}

// ---------------------------------------------------------------- fusion, rebase

/// `feature/login` et `main` changent la même ligne de `a.txt`.
fn team_with_conflicting_login() -> Team {
    let team = Team::new();
    let a = &team.a;
    a.run(&["switch", "-q", "-c", "feature/login"]);
    a.write("a.txt", "un\nDEUX côté login\ntrois\n");
    a.commit_all("Login change deux");
    a.run(&["switch", "-q", "main"]);
    a.write("a.txt", "un\nDEUX côté main\ntrois\n");
    a.commit_all("Main change deux");
    team
}

fn do_merge(test: &TestRepo, revision: &str) -> OperationOutcome {
    merge(
        &test.git(),
        &test.repo(),
        &MergeRequest {
            revision: revision.into(),
            ..MergeRequest::default()
        },
        &cancel(),
    )
    .unwrap()
}

#[test]
fn a_conflicting_merge_can_be_aborted_or_resolved() {
    // (16)
    let team = team_with_conflicting_login();
    let a = &team.a;
    let head = a.head();
    let outcome = do_merge(a, "feature/login");
    assert!(!outcome.finished);
    assert_eq!(outcome.conflicts, vec![a.path("a.txt")]);
    let state = status(&a.git(), &a.repo(), &cancel()).unwrap();
    assert_eq!(
        state.operation.as_ref().map(|o| o.kind),
        Some(OperationKind::Merge)
    );
    abort_operation(&a.git(), &a.repo(), &cancel()).unwrap();
    assert!(status(&a.git(), &a.repo(), &cancel()).unwrap().is_clean());
    assert_eq!(a.head(), head);

    let outcome = do_merge(a, "feature/login");
    assert!(!outcome.finished);
    resolve_with_content(
        &a.git(),
        &a.repo(),
        &a.path("a.txt"),
        b"un\nDEUX r\xc3\xa9solu\ntrois\n",
        None,
        &cancel(),
    )
    .unwrap();
    let done = continue_operation(&a.git(), &a.repo(), None, &cancel()).unwrap();
    assert!(done.finished);
    let parents = a.run(&["rev-list", "--parents", "-n", "1", "HEAD"]);
    assert_eq!(
        parents.split_whitespace().count(),
        3,
        "commit à deux parents"
    );
}

#[test]
fn merge_options_fast_forward_and_no_ff() {
    let team = team_with_branches();
    let a = &team.a;
    let ff = do_merge(a, "feature/login");
    assert!(ff.finished && ff.fast_forward);
    assert_eq!(a.head(), a.rev("feature/login"));
    let no_ff = merge(
        &a.git(),
        &a.repo(),
        &MergeRequest {
            revision: "feature/search".into(),
            fast_forward: FastForward::Never,
            message: Some("Fusion de la recherche".into()),
            ..MergeRequest::default()
        },
        &cancel(),
    )
    .unwrap();
    assert!(no_ff.finished && !no_ff.fast_forward);
    assert_eq!(
        a.run(&["log", "-1", "--format=%s"]).trim(),
        "Fusion de la recherche"
    );
    assert!(do_merge(a, "feature/search").up_to_date);
}

/// `feature/login` : trois commits, le deuxième en conflit avec `main`.
fn team_with_three_login_commits() -> Team {
    let team = Team::new();
    let a = &team.a;
    a.run(&["switch", "-q", "-c", "feature/login"]);
    a.write("l1.txt", "1\n");
    a.commit_all("Login 1");
    a.write("a.txt", "un\nDEUX côté login\ntrois\n");
    a.commit_all("Login 2");
    a.write("l3.txt", "3\n");
    a.commit_all("Login 3");
    a.run(&["switch", "-q", "main"]);
    a.write("a.txt", "un\nDEUX côté main\ntrois\n");
    a.commit_all("Main change deux");
    a.run(&["switch", "-q", "feature/login"]);
    team
}

#[test]
fn a_conflicting_rebase_shows_its_step_and_continues() {
    // (17)
    let team = team_with_three_login_commits();
    let a = &team.a;
    let outcome = rebase(
        &a.git(),
        &a.repo(),
        &RebaseRequest {
            onto: "main".into(),
            branch: None,
            local_changes: LocalChanges::Keep,
        },
        &cancel(),
    )
    .unwrap();
    assert!(!outcome.finished);
    assert_eq!(outcome.conflicts, vec![a.path("a.txt")]);
    let operation = a.repo().operation().unwrap();
    assert_eq!(operation.kind, OperationKind::Rebase);
    assert_eq!(operation.step, Some((2, 3)));
    assert_eq!(operation.rebase_branch.as_deref(), Some("feature/login"));
    resolve_with_content(
        &a.git(),
        &a.repo(),
        &a.path("a.txt"),
        b"un\nDEUX les deux\ntrois\n",
        None,
        &cancel(),
    )
    .unwrap();
    let done = continue_operation(&a.git(), &a.repo(), None, &cancel()).unwrap();
    assert!(done.finished, "{done:?}");
    assert_eq!(
        a.run(&["rev-list", "--count", "main..feature/login"])
            .trim(),
        "3"
    );
    assert_eq!(
        a.run(&["merge-base", "main", "feature/login"]).trim(),
        a.rev("main")
    );
}

#[test]
fn an_aborted_rebase_restores_the_branch() {
    // (17)
    let team = team_with_three_login_commits();
    let a = &team.a;
    let tip = a.head();
    let outcome = rebase(
        &a.git(),
        &a.repo(),
        &RebaseRequest {
            onto: "main".into(),
            branch: None,
            local_changes: LocalChanges::Keep,
        },
        &cancel(),
    )
    .unwrap();
    assert!(!outcome.finished);
    abort_operation(&a.git(), &a.repo(), &cancel()).unwrap();
    assert_eq!(a.rev("feature/login"), tip);
}

#[test]
fn skipping_a_rebase_commit_goes_on() {
    let team = team_with_three_login_commits();
    let a = &team.a;
    rebase(
        &a.git(),
        &a.repo(),
        &RebaseRequest {
            onto: "main".into(),
            branch: None,
            local_changes: LocalChanges::Keep,
        },
        &cancel(),
    )
    .unwrap();
    let skipped = skip_operation(&a.git(), &a.repo(), &cancel()).unwrap();
    assert!(skipped.finished);
    assert_eq!(
        a.run(&["rev-list", "--count", "main..feature/login"])
            .trim(),
        "2"
    );
}

#[test]
fn rebase_with_local_changes_refuses_or_stashes() {
    let team = team_with_branches();
    let a = &team.a;
    a.write("main2.txt", "x");
    a.commit_all("Main avance");
    a.run(&["switch", "-q", "feature/login"]);
    a.write("a.txt", "en cours\n");
    let error = rebase(
        &a.git(),
        &a.repo(),
        &RebaseRequest {
            onto: "main".into(),
            branch: None,
            local_changes: LocalChanges::Keep,
        },
        &cancel(),
    )
    .unwrap_err();
    assert!(
        matches!(error, VcsError::WouldOverwrite(ref c) if c.tracked == vec![a.path("a.txt")]),
        "{error:?}"
    );
    let outcome = rebase(
        &a.git(),
        &a.repo(),
        &RebaseRequest {
            onto: "main".into(),
            branch: None,
            local_changes: LocalChanges::Stash,
        },
        &cancel(),
    )
    .unwrap();
    assert!(outcome.finished);
    assert!(outcome
        .stash
        .unwrap()
        .message
        .contains("avant rebase de feature/login sur main"));
    assert_eq!(a.read("a.txt"), b"en cours\n");
}

// ---------------------------------------------------------------- cherry-pick, revert, reset

#[test]
fn cherry_pick_copies_a_commit_and_refuses_merges() {
    // (21)
    let team = team_with_branches();
    let a = &team.a;
    let picked = a.rev("feature/search");
    let outcome = cherry_pick(
        &a.git(),
        &a.repo(),
        std::slice::from_ref(&picked),
        LocalChanges::Keep,
        &cancel(),
    )
    .unwrap();
    assert!(outcome.finished);
    assert_ne!(a.head(), picked);
    assert_eq!(
        a.run(&["diff", &format!("{picked}^"), &picked]),
        a.run(&["diff", "HEAD^", "HEAD"])
    );
    merge(
        &a.git(),
        &a.repo(),
        &MergeRequest {
            revision: "feature/login".into(),
            fast_forward: FastForward::Never,
            ..MergeRequest::default()
        },
        &cancel(),
    )
    .unwrap();
    let merge_commit = a.head();
    assert!(matches!(
        cherry_pick(
            &a.git(),
            &a.repo(),
            std::slice::from_ref(&merge_commit),
            LocalChanges::Keep,
            &cancel()
        ),
        Err(VcsError::MergeCommitNotSupported(_))
    ));
    assert!(matches!(
        revert_commits(
            &a.git(),
            &a.repo(),
            &[merge_commit],
            LocalChanges::Keep,
            &cancel()
        ),
        Err(VcsError::MergeCommitNotSupported(_))
    ));
    assert!(is_ancestor(
        &a.git(),
        &a.repo(),
        &a.rev("feature/login"),
        "HEAD",
        &cancel()
    )
    .unwrap());
}

#[test]
fn revert_inverts_a_commit() {
    // (22)
    let team = Team::new();
    let a = &team.a;
    a.write("a.txt", "un\ndeux modifié\ntrois\n");
    a.commit_all("Change deux");
    let target = a.head();
    let outcome = revert_commits(
        &a.git(),
        &a.repo(),
        std::slice::from_ref(&target),
        LocalChanges::Keep,
        &cancel(),
    )
    .unwrap();
    assert!(outcome.finished);
    assert_eq!(
        a.run(&["diff", &target, "HEAD^"]),
        "",
        "même arbre que le commit annulé… "
    );
    assert_eq!(a.read("a.txt"), b"un\ndeux\ntrois\n");
    let forward = a.run(&["diff", &format!("{target}^"), &target]);
    let backward = a.run(&["diff", "HEAD", "HEAD^"]);
    assert_eq!(forward, backward, "diff inverse");
}

#[test]
fn reset_keeps_or_discards_as_asked() {
    // (23)
    let team = Team::new();
    let a = &team.a;
    for i in 1..=2 {
        a.write("a.txt", format!("un\ndeux\ntrois\nligne {i}\n"));
        a.commit_all(&format!("Étape {i}"));
    }
    let preview = reset_preview(&a.git(), &a.repo(), "HEAD~2", &cancel()).unwrap();
    assert_eq!(
        (preview.commits_leaving, preview.unreachable, preview.pushed),
        (2, 2, false)
    );
    let first = a.rev("HEAD~2");
    let done = reset(&a.git(), &a.repo(), "HEAD~2", ResetMode::Mixed, &cancel()).unwrap();
    assert_eq!(done.head, first);
    assert_eq!(a.head(), first);
    assert_eq!(a.read("a.txt"), b"un\ndeux\ntrois\nligne 2\n");
    let state = status(&a.git(), &a.repo(), &cancel()).unwrap();
    assert_eq!(state.files[0].index_state(), IndexState::NotStaged);

    // « Tout jeter », avec mise de côté d'abord.
    let preview = reset_preview(&a.git(), &a.repo(), "HEAD", &cancel()).unwrap();
    assert_eq!(preview.local_changes, vec![a.path("a.txt")]);
    let hard = reset_hard(&a.git(), &a.repo(), "HEAD", true, &cancel()).unwrap();
    assert_eq!(
        hard.stash.as_ref().map(|s| &s.restore),
        Some(&StashRestore::Kept)
    );
    assert_eq!(a.read("a.txt"), b"un\ndeux\ntrois\n");
    assert!(stash_list(a).contains("avant réinitialisation de main"));

    // Des commits déjà poussés : le prochain push devra forcer.
    a.write("p.txt", "p");
    a.commit_all("Poussé");
    a.run(&["push", "-q"]);
    let preview = reset_preview(&a.git(), &a.repo(), "HEAD~1", &cancel()).unwrap();
    assert!(preview.pushed);
    assert_eq!(preview.unreachable, 0, "encore sur origin/main");
}

// ---------------------------------------------------------------- journal et graphe

#[test]
fn the_graph_of_a_real_history_links_every_merge_to_both_parents() {
    // (28), en petit.
    let team = team_with_branches();
    let a = &team.a;
    for branch in ["feature/login", "feature/search", "bugfix/crash"] {
        merge(
            &a.git(),
            &a.repo(),
            &MergeRequest {
                revision: branch.into(),
                fast_forward: FastForward::Never,
                ..MergeRequest::default()
            },
            &cancel(),
        )
        .unwrap();
        a.write("main.txt", format!("après {branch}\n"));
        a.commit_all(&format!("Après {branch}"));
    }
    let query = LogQuery {
        all_refs: true,
        limit: 4,
        ..LogQuery::default()
    };
    let git = a.git();
    let repo = a.repo();
    let mut builder = GraphBuilder::new();
    let mut commits = Vec::new();
    let mut rows = Vec::new();
    let mut page = log(&git, &repo, &query, &cancel()).unwrap();
    loop {
        rows.extend(builder.push_page(&page.commits));
        commits.extend(page.commits.clone());
        if !page.has_more {
            break;
        }
        page = log(&git, &repo, &page.query.next_page(), &cancel()).unwrap();
    }
    assert_eq!(commits.len(), 11, "1 + 4 branches + 3 fusions + 3 après");
    assert_eq!(rows, graph_rows(&commits), "pages = un seul tenant");
    let merges: Vec<_> = commits
        .iter()
        .zip(&rows)
        .filter(|(c, _)| c.is_merge())
        .collect();
    assert_eq!(merges.len(), 3);
    for (_, row) in merges {
        assert_eq!(row.parent_edges().count(), 2);
    }
    // Ligne de premier parent droite : tous les commits de main dans le couloir 0.
    let main_line: Vec<String> = a
        .run(&["rev-list", "--first-parent", "main"])
        .lines()
        .map(str::to_owned)
        .collect();
    for (commit, row) in commits.iter().zip(&rows) {
        if main_line.contains(&commit.id) {
            assert_eq!(row.lane, 0, "{}", commit.subject);
        }
    }
}

#[test]
fn log_filters_by_author_text_and_follows_renames() {
    let test = TestRepo::new();
    test.write("ancien nom.txt", "contenu\n");
    test.commit_all("Création du fichier");
    test.write("autre.txt", "x");
    test.commit_all("Sans rapport");
    test.run(&["mv", "ancien nom.txt", "nouveau nom.txt"]);
    test.commit_all("Renommage");
    test.run(&[
        "-c",
        "user.name=Collègue Zoé",
        "commit",
        "-q",
        "--allow-empty",
        "-m",
        "Message [spécial] *",
    ]);
    let (git, repo) = (test.git(), test.repo());
    let subjects = |query: LogQuery| -> Vec<String> {
        log(&git, &repo, &query, &cancel())
            .unwrap()
            .commits
            .into_iter()
            .map(|c| c.subject)
            .collect()
    };
    assert_eq!(
        subjects(LogQuery::file_history(test.path("nouveau nom.txt"))),
        vec!["Renommage", "Création du fichier"]
    );
    assert_eq!(
        subjects(LogQuery {
            author: Some("zoé".into()),
            ..LogQuery::default()
        }),
        vec!["Message [spécial] *"]
    );
    assert_eq!(
        subjects(LogQuery {
            text: Some("[spécial] *".into()),
            ..LogQuery::default()
        }),
        vec!["Message [spécial] *"]
    );
    let path_only = log(
        &git,
        &repo,
        &LogQuery {
            paths: vec![test.path("autre.txt")],
            ..LogQuery::default()
        },
        &cancel(),
    )
    .unwrap();
    assert_eq!(path_only.commits.len(), 1);
    assert!(
        path_only.commits[0].parents.is_empty(),
        "parents réécrits : rien avant dans ce chemin"
    );
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    assert_eq!(
        subjects(LogQuery {
            since: Some(now + 3600),
            ..LogQuery::default()
        })
        .len(),
        0
    );
    assert_eq!(
        subjects(LogQuery {
            until: Some(now + 3600),
            ..LogQuery::default()
        })
        .len(),
        4
    );
    assert!(matches!(
        log(
            &git,
            &repo,
            &LogQuery {
                follow: true,
                ..LogQuery::default()
            },
            &cancel()
        ),
        Err(VcsError::InvalidRequest(_))
    ));
}

// ---------------------------------------------------------------- menus des fichiers

#[test]
fn stage_and_unstage_with_and_without_commits() {
    let test = TestRepo::new();
    test.write("dossier é/nouveau.txt", "n");
    let (git, repo) = (test.git(), test.repo());
    stage(&git, &repo, &[test.path("dossier é")], &cancel()).unwrap();
    let state = status(&git, &repo, &cancel()).unwrap();
    assert_eq!(state.files[0].kind, FileKind::Added);
    assert_eq!(state.files[0].index_state(), IndexState::Staged);
    // Dépôt sans commit : `rm --cached`, le fichier reste.
    unstage(
        &git,
        &repo,
        &[test.path("dossier é/nouveau.txt")],
        &cancel(),
    )
    .unwrap();
    let state = status(&git, &repo, &cancel()).unwrap();
    assert_eq!(state.files[0].kind, FileKind::Untracked);
    assert!(test.exists("dossier é/nouveau.txt"));

    test.commit_all("Premier");
    test.write("dossier é/nouveau.txt", "modifié une fois");
    stage(
        &git,
        &repo,
        &[test.path("dossier é/nouveau.txt")],
        &cancel(),
    )
    .unwrap();
    test.write("dossier é/nouveau.txt", "modifié deux fois");
    let state = status(&git, &repo, &cancel()).unwrap();
    assert_eq!(state.files[0].index_state(), IndexState::PartiallyStaged);
    unstage(
        &git,
        &repo,
        &[test.path("dossier é/nouveau.txt")],
        &cancel(),
    )
    .unwrap();
    let state = status(&git, &repo, &cancel()).unwrap();
    assert_eq!(state.files[0].index_state(), IndexState::NotStaged);
    assert_eq!(
        test.read("dossier é/nouveau.txt"),
        "modifié deux fois".as_bytes()
    );

    test.write(".gitignore", "*.log\n");
    test.write("trace.log", "l");
    assert!(matches!(
        stage(&git, &repo, &[test.path("trace.log")], &cancel()),
        Err(VcsError::IgnoredPaths(ref paths)) if paths == &vec![test.path("trace.log")]
    ));
}

#[test]
fn discarding_restores_modified_files_and_keeps_added_ones_on_disk() {
    let test = TestRepo::new();
    test.write("modifié.txt", "origine\n");
    test.write("supprimé.txt", "origine\n");
    test.write("renommé avant.txt", "renommé\n");
    test.commit_all("Base");
    test.write("modifié.txt", "changé\n");
    std::fs::remove_file(test.path("supprimé.txt")).unwrap();
    test.write("ajouté.txt", "nouveau\n");
    test.run(&["add", "ajouté.txt"]);
    test.run(&["mv", "renommé avant.txt", "renommé après.txt"]);
    test.write("non suivi.txt", "reste\n");
    let (git, repo) = (test.git(), test.repo());
    let outcome =
        discard_changes(&git, &repo, std::slice::from_ref(&test.root), &cancel()).unwrap();
    assert!(outcome.unstaged_added.contains(&test.path("ajouté.txt")));
    assert!(outcome
        .unstaged_added
        .contains(&test.path("renommé après.txt")));
    assert_eq!(test.read("modifié.txt"), b"origine\n");
    assert_eq!(test.read("supprimé.txt"), b"origine\n");
    assert_eq!(test.read("renommé avant.txt"), "renommé\n".as_bytes());
    assert!(
        test.exists("ajouté.txt"),
        "un ajouté n'est jamais supprimé ici"
    );
    assert!(test.exists("renommé après.txt"));
    assert_eq!(test.read("non suivi.txt"), b"reste\n");
    let state = status(&git, &repo, &cancel()).unwrap();
    assert!(
        state.files.iter().all(|f| f.kind == FileKind::Untracked),
        "{state:?}"
    );
}

#[test]
fn git_mv_handles_case_only_renames() {
    let test = TestRepo::new();
    test.write("user.php", "<?php\n");
    test.commit_all("Base");
    let (git, repo) = (test.git(), test.repo());
    move_path(
        &git,
        &repo,
        &test.path("user.php"),
        &test.path("User.php"),
        &cancel(),
    )
    .unwrap();
    let staged = test.run(&[
        "-c",
        "core.quotepath=false",
        "diff",
        "--cached",
        "--name-status",
        "-M",
    ]);
    assert!(
        staged.starts_with('R') && staged.contains("User.php"),
        "{staged}"
    );
    test.write("libre.txt", "x");
    assert!(matches!(
        move_path(
            &git,
            &repo,
            &test.path("libre.txt"),
            &test.path("ailleurs.txt"),
            &cancel()
        ),
        Err(VcsError::NotTracked(_))
    ));
    move_path(
        &git,
        &repo,
        &test.path("User.php"),
        &test.path("src/Modèle.php"),
        &cancel(),
    )
    .unwrap();
    assert!(test.exists("src/Modèle.php"));
}

#[test]
fn gitignore_lines_are_appended_with_the_files_line_endings() {
    let test = TestRepo::new();
    test.write("suivi.txt", "s");
    test.commit_all("Base");
    test.write(".gitignore", "node_modules/\r\n*.tmp\r\n");
    let (git, repo) = (test.git(), test.repo());
    let added = add_to_gitignore(
        &git,
        &repo,
        &IgnoreRule::File(test.path("logs/a b.log")),
        &cancel(),
    )
    .unwrap();
    assert_eq!(added.line, "/logs/a b.log");
    assert_eq!(
        test.read(".gitignore"),
        b"node_modules/\r\n*.tmp\r\n/logs/a b.log\r\n"
    );
    let again = add_to_gitignore(
        &git,
        &repo,
        &IgnoreRule::File(test.path("logs/a b.log")),
        &cancel(),
    )
    .unwrap();
    assert!(again.already_present);
    add_to_gitignore(
        &git,
        &repo,
        &IgnoreRule::Extension("cache".into()),
        &cancel(),
    )
    .unwrap();
    assert!(test.read(".gitignore").ends_with(b"*.cache\r\n"));
    assert!(matches!(
        add_to_gitignore(
            &git,
            &repo,
            &IgnoreRule::File(test.path("suivi.txt")),
            &cancel()
        ),
        Err(VcsError::AlreadyTracked(_))
    ));
    // Créé s'il manque.
    let fresh = TestRepo::new();
    add_to_gitignore(
        &fresh.git(),
        &fresh.repo(),
        &IgnoreRule::Folder(fresh.path("cache")),
        &cancel(),
    )
    .unwrap();
    assert_eq!(fresh.read(".gitignore"), b"/cache/\n");
}

#[test]
fn branch_names_with_accents_and_files_with_spaces_survive_a_round_trip() {
    let team = Team::new();
    let a = &team.a;
    create(a, "fonctionnalité/été", None, true).unwrap();
    a.write("dossier avec espaces/fichier é.txt", "x");
    a.commit_all("Accents");
    let protected = VcsSettings::default().protected_branches;
    push(
        &a.git(),
        &a.repo(),
        &PushRequest {
            target: push_target(&a.git(), &a.repo(), None, &cancel()).unwrap(),
            set_upstream: true,
            follow_tags: false,
            force: PushForce::No,
        },
        &protected,
        &mut no_progress(),
        &cancel(),
    )
    .unwrap();
    let branches = list(a);
    assert!(branches.find_remote("origin/fonctionnalité/été").is_some());
    assert_eq!(
        branches
            .find_local("fonctionnalité/été")
            .unwrap()
            .tracking
            .as_ref()
            .unwrap()
            .upstream,
        "origin/fonctionnalité/été"
    );
}

#[test]
fn protected_patterns_accept_a_trailing_star() {
    let protected = vec!["main".to_owned(), "release/*".to_owned()];
    assert!(is_protected(&protected, "main"));
    assert!(is_protected(&protected, "release/1.2"));
    assert!(!is_protected(&protected, "mainline"));
    let settings: VcsSettings = VcsSettings::default();
    assert_eq!(settings.protected_branches, vec!["main", "master"]);
}
