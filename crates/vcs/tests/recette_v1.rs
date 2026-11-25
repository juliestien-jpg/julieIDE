//! Corrections de la recette de la v1 (`docs/recette-v1.md`) côté `julie-vcs`,
//! sur des dépôts jetables : un dépôt nu local en guise de distant, le clone A
//! (testé) et le clone B (la collègue). Numéros de la recette entre crochets.

mod support;

use julie_vcs::*;
use support::{cancel, Team, TestRepo};

fn no_progress() -> impl FnMut(RemoteProgress) {
    |_| {}
}

fn stash_list(test: &TestRepo) -> String {
    test.run(&["stash", "list"])
}

fn text(test: &TestRepo, file: &str) -> String {
    String::from_utf8(test.read(file)).unwrap()
}

/// Le décor de M1 : A et B changent la ligne 2 de `a.txt` (B pousse) ; A a en
/// plus un changement local suivi (`LISEZMOI.md`) et un non suivi (`nv.txt`).
fn team_in_conflict() -> Team {
    let team = Team::new();
    let a = &team.a;
    a.write("LISEZMOI.md", "Lisez-moi\n");
    a.commit_all("Lisez-moi");
    a.run(&["push", "-q"]);
    team.b_run(&["pull", "-q"]);
    std::fs::write(team.b.join("a.txt"), "un\nDEUX (B)\ntrois\n").unwrap();
    team.b_run(&["commit", "-q", "-am", "Commit de B"]);
    team.b_run(&["push", "-q"]);
    a.write("a.txt", "un\nDEUX (A)\ntrois\n");
    a.commit_all("Commit de A");
    a.write("LISEZMOI.md", "Lisez-moi\nligne locale\n");
    a.write("nv.txt", "nouveau, non suivi\n");
    team
}

fn update(test: &TestRepo, method: IntegrationMethod) -> OperationOutcome {
    update_project(
        &test.git(),
        &test.repo(),
        &UpdateRequest {
            method,
            local_changes: LocalChanges::Stash,
        },
        &mut no_progress(),
        &cancel(),
    )
    .unwrap()
}

fn assert_local_work_is_back(a: &TestRepo) {
    assert_eq!(text(a, "LISEZMOI.md"), "Lisez-moi\nligne locale\n");
    assert_eq!(text(a, "nv.txt"), "nouveau, non suivi\n");
    assert_eq!(stash_list(a), "", "entrée supprimée une fois remise");
    assert!(!a.repo().git_dir().join(PENDING_STASH_FILE).exists());
    assert!(pending_auto_stashes(&a.git(), &a.repo(), &cancel())
        .unwrap()
        .is_empty());
}

fn resolve(a: &TestRepo, content: &str) {
    let path = a.path("a.txt");
    resolve_with_content(
        &a.git(),
        &a.repo(),
        &path,
        content.as_bytes(),
        None,
        &cancel(),
    )
    .unwrap();
}

// ---------------------------------------------------------------- M1

#[test]
fn m1_update_stopped_on_conflict_restores_the_stash_after_continue() {
    let team = team_in_conflict();
    let a = &team.a;
    let outcome = update(a, IntegrationMethod::Merge);
    assert!(!outcome.finished);
    let stash = outcome.stash.clone().unwrap();
    assert_eq!(stash.restore, StashRestore::Pending);
    assert!(!a.exists("nv.txt"), "rangé pendant la fusion");

    // Le bandeau : la mise de côté attend la fin de la fusion.
    let pending = pending_auto_stashes(&a.git(), &a.repo(), &cancel()).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(
        pending[0].state,
        PendingStashState::AwaitingOperation(OperationKind::Merge)
    );
    assert_eq!(pending[0].entry.commit, stash.commit);
    assert_eq!(
        pending[0].reason,
        Some(StashReason::Update {
            branch: "main".into()
        })
    );

    resolve(a, "un\nDEUX (A et B)\ntrois\n");
    let done = continue_operation(&a.git(), &a.repo(), None, &cancel()).unwrap();
    assert!(done.finished);
    let restored = done.stash.unwrap();
    assert_eq!(restored.restore, StashRestore::Restored);
    assert_eq!(restored.commit, stash.commit);
    assert_local_work_is_back(a);
    assert_eq!(text(a, "a.txt"), "un\nDEUX (A et B)\ntrois\n");
}

#[test]
fn m1_aborting_the_update_restores_the_stash() {
    let team = team_in_conflict();
    let a = &team.a;
    let before = a.head();
    let outcome = update(a, IntegrationMethod::Merge);
    assert!(!outcome.finished);
    let aborted = abort_operation_restoring(&a.git(), &a.repo(), &cancel()).unwrap();
    assert_eq!(aborted.kind, OperationKind::Merge);
    assert_eq!(aborted.stash.unwrap().restore, StashRestore::Restored);
    assert_eq!(a.head(), before);
    assert_eq!(text(a, "a.txt"), "un\nDEUX (A)\ntrois\n");
    assert_local_work_is_back(a);
}

#[test]
fn m1_rebase_update_restores_the_stash_once_the_rebase_is_done() {
    let team = team_in_conflict();
    let a = &team.a;
    let outcome = update(a, IntegrationMethod::Rebase);
    assert!(!outcome.finished);
    assert_eq!(outcome.stash.unwrap().restore, StashRestore::Pending);
    resolve(a, "un\nDEUX (B puis A)\ntrois\n");
    let done = continue_operation(&a.git(), &a.repo(), None, &cancel()).unwrap();
    assert!(done.finished, "{}", done.output);
    assert_eq!(done.stash.unwrap().restore, StashRestore::Restored);
    assert_local_work_is_back(a);
    let log = a.run(&["log", "--format=%s", "-3"]);
    assert_eq!(
        log.lines().collect::<Vec<_>>(),
        vec!["Commit de A", "Commit de B", "Lisez-moi"]
    );
}

#[test]
fn m1_abort_of_a_rebase_with_the_sync_function_also_restores() {
    let team = team_in_conflict();
    let a = &team.a;
    update(a, IntegrationMethod::Rebase);
    assert_eq!(
        abort_operation(&a.git(), &a.repo(), &cancel()).unwrap(),
        OperationKind::Rebase
    );
    assert_local_work_is_back(a);
}

#[test]
fn m1_the_link_survives_a_restart_and_an_operation_finished_elsewhere() {
    let team = team_in_conflict();
    let a = &team.a;
    let stash = update(a, IntegrationMethod::Merge).stash.unwrap();
    // « Redémarrage » : un nouveau dépôt ouvert, rien en mémoire.
    let reopened = a.repo();
    let pending = pending_auto_stashes(&a.git(), &reopened, &cancel()).unwrap();
    assert_eq!(
        pending[0].state,
        PendingStashState::AwaitingOperation(OperationKind::Merge)
    );
    // Remettre pendant la fusion : refusé, rien ne bouge.
    assert!(matches!(
        restore_pending_stash(&a.git(), &reopened, &cancel()),
        Err(VcsError::OperationInProgress(OperationKind::Merge))
    ));
    // La fusion est abandonnée au terminal : rien ne remet la mise de côté…
    a.run(&["merge", "--abort"]);
    let pending = pending_auto_stashes(&a.git(), &reopened, &cancel()).unwrap();
    assert_eq!(pending[0].state, PendingStashState::OperationOver);
    // … sauf « Remettre mes changements ».
    let restored = restore_pending_stash(&a.git(), &reopened, &cancel())
        .unwrap()
        .unwrap();
    assert_eq!(restored.commit, stash.commit);
    assert_eq!(restored.restore, StashRestore::Restored);
    assert_local_work_is_back(a);
    assert!(restore_pending_stash(&a.git(), &reopened, &cancel())
        .unwrap()
        .is_none());
}

#[test]
fn m1_a_restore_in_conflict_after_continue_keeps_the_entry() {
    // Le changement local touche la ligne que la fusion change aussi.
    let team = team_in_conflict();
    let a = &team.a;
    a.write("LISEZMOI.md", "Lisez-moi\n");
    a.write("a.txt", "un\nDEUX (local)\ntrois\n");
    // `a.txt` local modifié : la mise de côté le range ; la fusion le change.
    let outcome = update(a, IntegrationMethod::Merge);
    assert!(!outcome.finished);
    resolve(a, "un\nDEUX (A et B)\ntrois\n");
    let done = continue_operation(&a.git(), &a.repo(), None, &cancel()).unwrap();
    let stash = done.stash.unwrap();
    assert!(
        matches!(&stash.restore, StashRestore::Conflicts(paths) if paths == &vec![a.path("a.txt")]),
        "{stash:?}"
    );
    let pending = pending_auto_stashes(&a.git(), &a.repo(), &cancel()).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].state, PendingStashState::Kept);
    assert!(!a.repo().git_dir().join(PENDING_STASH_FILE).exists());
    // Pendant cette remise en conflit, votre travail est l'étage 3.
    let versions = conflict_versions(&a.git(), &a.repo(), &a.path("a.txt"), &cancel()).unwrap();
    assert_eq!(versions.operation, None);
    assert_eq!(versions.labels.yours(), ConflictSide::Theirs);
    assert_eq!(versions.labels.theirs.role, Some(SideRole::StashedChanges));
}

#[test]
fn m1_a_cherry_pick_stopped_then_skipped_restores_the_stash() {
    let team = Team::new();
    let a = &team.a;
    a.run(&["switch", "-q", "-c", "autre"]);
    a.write("a.txt", "un\nDEUX (autre)\ntrois\n");
    a.commit_all("Sur autre");
    let picked = a.head();
    a.run(&["switch", "-q", "main"]);
    a.write("a.txt", "un\nDEUX (main)\ntrois\n");
    a.commit_all("Sur main");
    a.write("local.txt", "local\n");
    let outcome = cherry_pick(
        &a.git(),
        &a.repo(),
        std::slice::from_ref(&picked),
        LocalChanges::Stash,
        &cancel(),
    )
    .unwrap();
    assert!(!outcome.finished);
    assert!(!a.exists("local.txt"));
    let skipped = skip_operation(&a.git(), &a.repo(), &cancel()).unwrap();
    assert!(skipped.finished);
    assert_eq!(skipped.stash.unwrap().restore, StashRestore::Restored);
    assert_eq!(text(a, "local.txt"), "local\n");
    assert_eq!(stash_list(a), "");
}

#[test]
fn stash_messages_are_parsed_back() {
    let reasons = [
        StashReason::Switch {
            from: "main".into(),
            to: "feature/x".into(),
        },
        StashReason::Update {
            branch: "main".into(),
        },
        StashReason::Rebase {
            branch: "feature/x".into(),
            onto: "origin/main".into(),
        },
        StashReason::Merge {
            branch: "main".into(),
            revision: "feature/x".into(),
        },
        StashReason::CherryPick {
            branch: "main".into(),
        },
        StashReason::Revert {
            branch: "main".into(),
        },
        StashReason::Reset {
            branch: "main".into(),
        },
    ];
    for reason in reasons {
        let message = reason.message(std::time::SystemTime::now());
        assert_eq!(StashReason::parse(&message).as_ref(), Some(&reason));
        assert_eq!(
            StashReason::parse(&format!("On main: {message}")).as_ref(),
            Some(&reason)
        );
    }
    assert_eq!(StashReason::parse("On main: WIP"), None);
}

// ---------------------------------------------------------------- M3

#[test]
fn m3_leaving_a_detached_head_with_orphans_is_refused_then_allowed() {
    let team = Team::new();
    let a = &team.a;
    let first = a.head();
    a.write("b.txt", "b\n");
    a.commit_all("Deuxième");
    let switch = |target: SwitchTarget, leave: bool| {
        switch_branch_with(
            &a.git(),
            &a.repo(),
            &SwitchRequest {
                target,
                local_changes: LocalChanges::Keep,
            },
            SwitchOptions {
                leave_detached_commits: leave,
            },
            &cancel(),
        )
    };
    switch(SwitchTarget::Detach(first.clone()), false).unwrap();
    // Rien d'orphelin encore : bandeau sans alerte.
    let report = detached_head_report(&a.git(), &a.repo(), &cancel())
        .unwrap()
        .unwrap();
    assert_eq!(
        (report.head.as_str(), report.orphan_commits),
        (first.as_str(), 0)
    );

    a.write("orphelin.txt", "o\n");
    a.commit_all("Commit orphelin");
    let orphan = a.head();
    let report = detached_head_report(&a.git(), &a.repo(), &cancel())
        .unwrap()
        .unwrap();
    assert_eq!(report.orphan_commits, 1);
    assert_eq!(report.subjects, vec!["Commit orphelin".to_owned()]);

    // Bascule ordinaire : refusée, rien n'a changé.
    match switch(SwitchTarget::Branch("main".into()), false) {
        Err(VcsError::DetachedCommitsLeft {
            head,
            count,
            subjects,
        }) => {
            assert_eq!((head.as_str(), count), (orphan.as_str(), 1));
            assert_eq!(subjects, vec!["Commit orphelin".to_owned()]);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(a.head(), orphan);
    // « Forcer la bascule » aussi : refusée avant de jeter quoi que ce soit.
    a.write("orphelin.txt", "modifié\n");
    let discard = OverwriteConflict {
        tracked: vec![a.path("orphelin.txt")],
        ..OverwriteConflict::default()
    };
    assert!(matches!(
        switch_discarding_changes(
            &a.git(),
            &a.repo(),
            &SwitchTarget::Branch("main".into()),
            &discard,
            &cancel()
        ),
        Err(VcsError::DetachedCommitsLeft { .. })
    ));
    assert_eq!(text(a, "orphelin.txt"), "modifié\n", "rien de jeté");
    a.write("orphelin.txt", "o\n");
    // Le message d'erreur se lit.
    let message = switch(SwitchTarget::Branch("main".into()), false)
        .unwrap_err()
        .to_string();
    assert!(
        message.contains("1 commit(s) ne sont sur aucune branche"),
        "{message}"
    );

    // « Basculer quand même ».
    let outcome = switch(SwitchTarget::Branch("main".into()), true).unwrap();
    assert_eq!(outcome.left_detached, Some((1, orphan.clone())));
    assert_eq!(outcome.branch.as_deref(), Some("main"));
    assert!(detached_head_report(&a.git(), &a.repo(), &cancel())
        .unwrap()
        .is_none());
}

#[test]
fn m3_moving_to_a_descendant_or_creating_a_branch_is_not_refused() {
    let team = Team::new();
    let a = &team.a;
    switch_branch(
        &a.git(),
        &a.repo(),
        &SwitchRequest {
            target: SwitchTarget::Detach(a.head()),
            local_changes: LocalChanges::Keep,
        },
        &cancel(),
    )
    .unwrap();
    a.write("o1.txt", "1\n");
    a.commit_all("Orphelin 1");
    let first = a.head();
    a.write("o2.txt", "2\n");
    a.commit_all("Orphelin 2");
    let second = a.head();
    // Revenir en arrière sur un orphelin : seul le second serait perdu.
    assert!(matches!(
        switch_branch(
            &a.git(),
            &a.repo(),
            &SwitchRequest {
                target: SwitchTarget::Detach(first.clone()),
                local_changes: LocalChanges::Keep,
            },
            &cancel()
        ),
        Err(VcsError::DetachedCommitsLeft { count: 1, .. })
    ));
    // « Créer une branche d'abord » : depuis HEAD, les commits sont gardés.
    let created = create_branch(
        &a.git(),
        &a.repo(),
        &CreateBranchRequest {
            name: "sauvetage".into(),
            start: None,
            switch: true,
            local_changes: LocalChanges::Keep,
        },
        &cancel(),
    )
    .unwrap();
    assert_eq!(created.commit.as_deref(), Some(second.as_str()));
    assert_eq!(
        commits_only_on_detached_head(&a.git(), &a.repo(), &cancel()).unwrap(),
        0
    );
    switch_branch(
        &a.git(),
        &a.repo(),
        &SwitchRequest {
            target: SwitchTarget::Branch("main".into()),
            local_changes: LocalChanges::Keep,
        },
        &cancel(),
    )
    .unwrap();
}

// ---------------------------------------------------------------- M5

#[test]
fn m5_rebase_conflict_names_your_commit_and_the_upstream() {
    let team = team_in_conflict();
    let a = &team.a;
    a.write("LISEZMOI.md", "Lisez-moi\n");
    std::fs::remove_file(a.path("nv.txt")).unwrap();
    let mine = a.head();
    let outcome = update(a, IntegrationMethod::Rebase);
    assert!(!outcome.finished);
    let versions = conflict_versions(&a.git(), &a.repo(), &a.path("a.txt"), &cancel()).unwrap();
    let labels = &versions.labels;
    assert_eq!(labels.yours(), ConflictSide::Theirs, "pendant un rebase");
    assert_eq!(labels.incoming(), ConflictSide::Ours);
    assert_eq!(labels.ours.role, Some(SideRole::RebaseBase));
    assert_eq!(labels.ours.name.as_deref(), Some("origin/main"));
    assert_eq!(labels.ours.subject.as_deref(), Some("Commit de B"));
    assert_eq!(labels.theirs.role, Some(SideRole::ReplayedCommit));
    assert_eq!(labels.theirs.commit.as_deref(), Some(mine.as_str()));
    assert_eq!(labels.theirs.name.as_deref(), Some("main"));
    assert_eq!(labels.theirs.subject.as_deref(), Some("Commit de A"));
    // L'étage « votre travail » porte bien votre changement.
    assert_eq!(
        versions.theirs.clone().into_bytes().unwrap(),
        b"un\nDEUX (A)\ntrois\n".to_vec()
    );
    abort_operation(&a.git(), &a.repo(), &cancel()).unwrap();
}

#[test]
fn m5_merge_and_cherry_pick_conflicts_name_their_sides() {
    let team = team_in_conflict();
    let a = &team.a;
    a.write("LISEZMOI.md", "Lisez-moi\n");
    std::fs::remove_file(a.path("nv.txt")).unwrap();
    update(a, IntegrationMethod::Merge);
    let versions = conflict_versions(&a.git(), &a.repo(), &a.path("a.txt"), &cancel()).unwrap();
    let labels = &versions.labels;
    assert_eq!(labels.yours(), ConflictSide::Ours);
    assert_eq!(labels.ours.role, Some(SideRole::CurrentBranch));
    assert_eq!(labels.ours.name.as_deref(), Some("main"));
    assert_eq!(labels.ours.subject.as_deref(), Some("Commit de A"));
    assert_eq!(labels.theirs.role, Some(SideRole::MergedRevision));
    assert_eq!(labels.theirs.name.as_deref(), Some("origin/main"));
    abort_operation(&a.git(), &a.repo(), &cancel()).unwrap();

    let theirs = a.rev("origin/main");
    cherry_pick(
        &a.git(),
        &a.repo(),
        std::slice::from_ref(&theirs),
        LocalChanges::Keep,
        &cancel(),
    )
    .unwrap();
    let versions = conflict_versions(&a.git(), &a.repo(), &a.path("a.txt"), &cancel()).unwrap();
    assert_eq!(versions.labels.yours(), ConflictSide::Ours);
    assert_eq!(versions.labels.theirs.role, Some(SideRole::PickedCommit));
    assert_eq!(
        versions.labels.theirs.commit.as_deref(),
        Some(theirs.as_str())
    );
    assert_eq!(
        versions.labels.theirs.subject.as_deref(),
        Some("Commit de B")
    );
    abort_operation(&a.git(), &a.repo(), &cancel()).unwrap();
    assert_eq!(
        ConflictLabels::for_operation(Some(OperationKind::Revert))
            .theirs
            .role,
        Some(SideRole::RevertedCommit)
    );
}

// ---------------------------------------------------------------- M2

#[test]
fn m2_forced_switch_and_hard_reset_report_what_was_thrown_away() {
    let team = Team::new();
    let a = &team.a;
    a.run(&["switch", "-q", "-c", "feature/login"]);
    a.write("a.txt", "un\nLOGIN\ntrois\n");
    a.write("login.txt", "suivi sur feature/login\n");
    a.commit_all("Login");
    a.run(&["switch", "-q", "main"]);
    a.write("a.txt", "un\nlocal\ntrois\n");
    a.write("login.txt", "mon non suivi\n");
    a.write("u.txt", "reste\n");
    let conflict = match switch_branch(
        &a.git(),
        &a.repo(),
        &SwitchRequest {
            target: SwitchTarget::Branch("feature/login".into()),
            local_changes: LocalChanges::Keep,
        },
        &cancel(),
    ) {
        Err(VcsError::WouldOverwrite(conflict)) => conflict,
        other => panic!("{other:?}"),
    };
    // m1 : un résumé lisible, pas une structure Rust.
    let message = VcsError::WouldOverwrite(conflict.clone()).to_string();
    assert!(!message.contains("OverwriteConflict"), "{message}");
    assert!(message.contains("a.txt"), "{message}");
    assert_eq!(conflict.len(), 2);
    let outcome = switch_discarding_changes(
        &a.git(),
        &a.repo(),
        &SwitchTarget::Branch("feature/login".into()),
        &conflict,
        &cancel(),
    )
    .unwrap();
    let mut discarded = outcome.discarded.clone();
    discarded.sort();
    assert_eq!(discarded, vec![a.path("a.txt"), a.path("login.txt")]);
    assert_eq!(text(a, "u.txt"), "reste\n", "le reste a suivi");

    // Reset « tout jeter » sans mise de côté.
    a.write("a.txt", "jeté\n");
    a.write("u.txt", "toujours là\n");
    let reset = reset_hard(&a.git(), &a.repo(), "main", false, &cancel()).unwrap();
    assert_eq!(reset.discarded, vec![a.path("a.txt")]);
    assert_eq!(text(a, "u.txt"), "toujours là\n");
    // Avec mise de côté : rien de jeté, tout est dans l'entrée.
    a.write("a.txt", "rangé\n");
    let reset = reset_hard(&a.git(), &a.repo(), "main", true, &cancel()).unwrap();
    assert!(reset.discarded.is_empty());
    assert_eq!(reset.stash.unwrap().restore, StashRestore::Kept);
    let pending = pending_auto_stashes(&a.git(), &a.repo(), &cancel()).unwrap();
    assert_eq!(pending[0].state, PendingStashState::Kept);
}

// ---------------------------------------------------------------- m2

#[test]
fn m2_cherry_picking_a_commit_already_there_is_refused_without_side_effect() {
    let team = Team::new();
    let a = &team.a;
    a.write("b.txt", "b\n");
    a.commit_all("Sommet");
    let tip = a.head();
    let presence =
        commits_presence(&a.git(), &a.repo(), std::slice::from_ref(&tip), &cancel()).unwrap();
    assert_eq!(presence, vec![(tip.clone(), CommitPresence::Contained)]);
    match cherry_pick(
        &a.git(),
        &a.repo(),
        std::slice::from_ref(&tip),
        LocalChanges::Keep,
        &cancel(),
    ) {
        Err(VcsError::AlreadyApplied { commits }) => assert_eq!(commits, vec![tip.clone()]),
        other => panic!("{other:?}"),
    }
    assert!(
        a.repo().operation().is_none(),
        "pas de cherry-pick vide en cours"
    );
    assert_eq!(a.head(), tip);

    // Même changement sur une autre branche (autre commit, même patch).
    a.run(&["switch", "-q", "-c", "copie", "HEAD~1"]);
    a.write("b.txt", "b\n");
    a.commit_all("Le même changement");
    let copy = a.head();
    a.run(&["switch", "-q", "main"]);
    let presence =
        commits_presence(&a.git(), &a.repo(), std::slice::from_ref(&copy), &cancel()).unwrap();
    assert_eq!(presence, vec![(copy.clone(), CommitPresence::SameChange)]);
    a.run(&["switch", "-q", "-c", "nouvelle", "HEAD~1"]);
    a.write("c.txt", "c\n");
    a.commit_all("Nouveau");
    let fresh = a.head();
    a.run(&["switch", "-q", "main"]);
    let presence =
        commits_presence(&a.git(), &a.repo(), std::slice::from_ref(&fresh), &cancel()).unwrap();
    assert_eq!(presence, vec![(fresh, CommitPresence::Absent)]);
}

#[test]
fn m2_a_pick_that_becomes_empty_on_the_way_is_skipped() {
    // Le second commit ajoute `y.txt`, déjà sur main par un commit qui fait
    // plus (patch différent : `git cherry` ne le voit pas).
    let team = Team::new();
    let a = &team.a;
    a.run(&["switch", "-q", "-c", "source"]);
    a.write("x.txt", "x\n");
    a.commit_all("Ajoute x");
    let first = a.head();
    a.write("y.txt", "y\n");
    a.commit_all("Ajoute y");
    let second = a.head();
    a.run(&["switch", "-q", "main"]);
    a.write("y.txt", "y\n");
    a.write("z.txt", "z\n");
    a.commit_all("y et z");
    let presence = commits_presence(
        &a.git(),
        &a.repo(),
        &[first.clone(), second.clone()],
        &cancel(),
    )
    .unwrap();
    assert!(presence.iter().all(|(_, p)| *p == CommitPresence::Absent));
    let outcome = cherry_pick(
        &a.git(),
        &a.repo(),
        &[first, second.clone()],
        LocalChanges::Keep,
        &cancel(),
    )
    .unwrap();
    assert!(outcome.finished, "{}", outcome.output);
    assert_eq!(outcome.empty_skipped, vec![second]);
    assert!(
        a.repo().operation().is_none(),
        "pas de « cherry-pick en cours — 0 fichier »"
    );
    assert_eq!(a.run(&["log", "-1", "--format=%s"]).trim(), "Ajoute x");
}

// ---------------------------------------------------------------- m3, amont

#[test]
fn m3_a_restored_branch_gets_its_upstream_back() {
    let team = Team::new();
    let a = &team.a;
    a.run(&["switch", "-q", "-c", "bugfix/crash"]);
    a.write("crash.txt", "c\n");
    a.commit_all("Correctif");
    a.run(&["push", "-q", "-u", "origin", "bugfix/crash"]);
    a.run(&["switch", "-q", "main"]);
    let deleted = force_delete_branch(&a.git(), &a.repo(), "bugfix/crash", &cancel()).unwrap();
    assert_eq!(
        deleted.upstream_config,
        Some(BranchUpstream {
            remote: "origin".into(),
            merge: "refs/heads/bugfix/crash".into()
        })
    );
    restore_deleted_branch(&a.git(), &a.repo(), &deleted, &cancel()).unwrap();
    let vv = a.run(&["branch", "-vv", "--list", "bugfix/crash"]);
    assert!(vv.contains("[origin/bugfix/crash]"), "{vv}");
    assert_eq!(a.rev("bugfix/crash"), deleted.tip);
}

#[test]
fn upstream_can_be_chosen_and_removed() {
    let team = Team::new();
    let a = &team.a;
    a.run(&["switch", "-q", "-c", "seule"]);
    assert!(matches!(
        set_upstream(
            &a.git(),
            &a.repo(),
            "seule",
            "origin/nexiste-pas",
            &cancel()
        ),
        Err(VcsError::BranchNotFound(_))
    ));
    set_upstream(&a.git(), &a.repo(), "seule", "origin/main", &cancel()).unwrap();
    let vv = a.run(&["branch", "-vv", "--list", "seule"]);
    assert!(vv.contains("[origin/main]"), "{vv}");
    unset_upstream(&a.git(), &a.repo(), "seule", &cancel()).unwrap();
    let vv = a.run(&["branch", "-vv", "--list", "seule"]);
    assert!(!vv.contains("[origin"), "{vv}");
    // Sans amont : rien à faire, pas d'erreur.
    unset_upstream(&a.git(), &a.repo(), "seule", &cancel()).unwrap();
}

// ---------------------------------------------------------------- m13

#[test]
fn m13_an_untracked_file_replaced_on_restore_is_reported() {
    let team = Team::new();
    let a = &team.a;
    a.run(&["switch", "-q", "-c", "feature/login"]);
    a.write("login.txt", "suivi sur feature/login\n");
    a.write("a.txt", "un\nLOGIN\ntrois\n");
    a.commit_all("Login");
    a.run(&["switch", "-q", "main"]);
    a.write("a.txt", "un\ndeux\ntrois\nlocal\n");
    a.write("login.txt", "mon contenu\n");
    let outcome = switch_branch(
        &a.git(),
        &a.repo(),
        &SwitchRequest {
            target: SwitchTarget::Branch("feature/login".into()),
            local_changes: LocalChanges::Stash,
        },
        &cancel(),
    )
    .unwrap();
    let stash = outcome.stash.unwrap();
    assert!(stash.kept(), "{stash:?}");
    assert_eq!(stash.untracked_not_restored, vec![a.path("login.txt")]);
    // Votre version reste lisible dans l'entrée.
    let mine = a.run(&["show", &format!("{}^3:login.txt", stash.commit)]);
    assert_eq!(mine, "mon contenu\n");
}
