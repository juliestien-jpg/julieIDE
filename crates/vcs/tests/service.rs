//! Le service asynchrone : tâches de fond, événements sur le bus.

mod support;

use std::time::Duration;

use julie_core::{Bus, Domain, Event, TaskOutcome, TaskState, Tasks};
use julie_vcs::*;
use support::TestRepo;

struct Harness {
    bus: Bus,
    vcs: Vcs,
}

impl Harness {
    fn new(test: &TestRepo) -> Self {
        let bus = Bus::new();
        let tasks = Tasks::new(bus.publisher());
        let vcs = Vcs::with_git(tasks, test.git());
        Self { bus, vcs }
    }

    /// Les événements Vcs (hors journal des commandes et hors bandeaux
    /// `PendingStashes` / `DetachedHead`) et l'issue des tâches.
    fn drain(&self) -> (Vec<VcsEvent>, Vec<TaskOutcome>, usize) {
        self.drain_inner(false)
    }

    /// Comme [`Harness::drain`], bandeaux compris.
    fn drain_all(&self) -> (Vec<VcsEvent>, Vec<TaskOutcome>, usize) {
        self.drain_inner(true)
    }

    fn drain_inner(&self, banners: bool) -> (Vec<VcsEvent>, Vec<TaskOutcome>, usize) {
        let mut events = Vec::new();
        let mut outcomes = Vec::new();
        let mut commands = 0;
        for event in self.bus.drain() {
            match event {
                Event::Domain(event) => {
                    assert_eq!(event.domain, Domain::Vcs);
                    match event.take::<VcsEvent>() {
                        Ok(VcsEvent::Command(_)) => commands += 1,
                        Ok(VcsEvent::PendingStashes(_) | VcsEvent::DetachedHead(_)) if !banners => {
                        }
                        Ok(event) => events.push(event),
                        Err(other) => panic!("charge inattendue : {}", other.type_name()),
                    }
                }
                Event::Task(task) => {
                    if let TaskState::Finished(outcome) = task.state {
                        outcomes.push(outcome);
                    }
                }
                other => panic!("événement inattendu : {other:?}"),
            }
        }
        (events, outcomes, commands)
    }
}

#[test]
fn open_publishes_the_repository_then_its_status() {
    let test = TestRepo::new();
    test.write("a.txt", "a\n");
    let harness = Harness::new(&test);
    harness.vcs.open(test.root.clone()).join();
    let (events, outcomes, commands) = harness.drain();
    assert_eq!(outcomes, vec![TaskOutcome::Done]);
    assert_eq!(commands, 4, "début et fin de rev-parse et de status");
    match events.as_slice() {
        [VcsEvent::Opened(repo), VcsEvent::Status(status)] => {
            assert_eq!(repo.work_tree(), test.root.as_path());
            assert_eq!(status.untracked().count(), 1);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(harness.vcs.command_log().len(), 2);
}

#[test]
fn a_plain_folder_publishes_no_repository() {
    let plain = TestRepo::empty_dir();
    let harness = Harness::new(&plain);
    harness.vcs.open(plain.root.clone()).join();
    let (events, outcomes, _) = harness.drain();
    assert_eq!(outcomes, vec![TaskOutcome::Done]);
    assert!(matches!(events.as_slice(), [VcsEvent::NoRepository { dir }] if *dir == plain.root));
}

#[test]
fn a_commit_publishes_its_outcome_then_a_fresh_status() {
    let test = TestRepo::new();
    test.write("a.txt", "a\n");
    let harness = Harness::new(&test);
    let repo = test.repo();
    harness
        .vcs
        .commit(
            &repo,
            CommitRequest {
                paths: vec![test.path("a.txt")],
                message: "Premier".into(),
                amend: false,
            },
        )
        .join();
    let (events, outcomes, _) = harness.drain();
    assert_eq!(outcomes, vec![TaskOutcome::Done]);
    match events.as_slice() {
        [VcsEvent::Committed(outcome), VcsEvent::Status(status)] => {
            assert_eq!(outcome.subject, "Premier");
            assert!(status.is_clean());
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_failed_commit_publishes_status_then_the_failure() {
    let test = TestRepo::new();
    test.write(".git/hooks/pre-commit", "#!/bin/sh\necho non >&2\nexit 1\n");
    test.write("a.txt", "a\n");
    let harness = Harness::new(&test);
    let repo = test.repo();
    harness
        .vcs
        .commit(
            &repo,
            CommitRequest {
                paths: vec![test.path("a.txt")],
                message: "Refusé".into(),
                amend: false,
            },
        )
        .join();
    let (events, outcomes, _) = harness.drain();
    assert!(matches!(outcomes.as_slice(), [TaskOutcome::Failed(_)]));
    match events.as_slice() {
        [VcsEvent::Status(status), VcsEvent::Failed(Failure {
            action: Action::Commit,
            error: VcsError::GitFailed { stderr, .. },
        })] => {
            assert_eq!(status.untracked().count(), 1, "l'ajout a été défait");
            assert!(stderr.contains("non"));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn conflict_resolution_through_the_service() {
    let test = TestRepo::new();
    test.write("f.txt", "base\n");
    test.commit_all("base");
    test.run(&["checkout", "-q", "-b", "eux"]);
    test.write("f.txt", "leur\n");
    test.commit_all("leur");
    test.run(&["checkout", "-q", "main"]);
    test.write("f.txt", "notre\n");
    test.commit_all("notre");
    assert!(!test.try_run(&["merge", "eux"]).status.success());

    let harness = Harness::new(&test);
    let repo = test.repo();
    harness.vcs.load_conflict(&repo, test.path("f.txt")).join();
    let (mut events, _, _) = harness.drain();
    let Some(VcsEvent::Conflict(versions)) = events.pop() else {
        panic!("versions attendues")
    };
    assert_eq!(versions.base.bytes(), Some(&b"base\n"[..]));

    harness
        .vcs
        .resolve_conflict(
            &repo,
            test.path("f.txt"),
            b"notre et leur\n".to_vec(),
            versions.work_tree.into_bytes(),
        )
        .join();
    let (events, outcomes, _) = harness.drain();
    assert_eq!(outcomes, vec![TaskOutcome::Done]);
    assert!(matches!(&events[0], VcsEvent::Resolved { path } if *path == test.path("f.txt")));
    assert!(matches!(&events[1], VcsEvent::Status(status) if status.conflicts().count() == 0));

    harness.vcs.continue_operation(&repo, None).join();
    let (events, _, _) = harness.drain();
    assert!(matches!(&events[0], VcsEvent::OperationContinued(outcome) if outcome.finished));
    assert!(matches!(&events[1], VcsEvent::Status(status) if status.operation.is_none()));
}

#[test]
fn disk_changes_are_debounced_into_one_status() {
    let test = TestRepo::new();
    test.write("a.txt", "a\n");
    let harness = Harness::new(&test);
    let repo = test.repo();

    // Le bruit de `.git` (objets, verrous) et `.juliestorm` ne relance rien.
    assert!(!is_relevant_change(
        &repo,
        &test.path(".git/objects/ab/cdef")
    ));
    assert!(!is_relevant_change(&repo, &test.path(".git/index.lock")));
    assert!(!is_relevant_change(
        &repo,
        &test.path(".juliestorm/index/cache")
    ));
    assert!(is_relevant_change(&repo, &test.path(".git/index")));
    assert!(is_relevant_change(&repo, &test.path(".git/MERGE_HEAD")));
    assert!(is_relevant_change(&repo, &test.path("a.txt")));
    assert!(harness
        .vcs
        .notify_disk_change(&repo, &[test.path(".git/objects/x")])
        .is_none());

    let first = harness
        .vcs
        .notify_disk_change(&repo, &[test.path("a.txt")])
        .expect("une relecture est prévue");
    for _ in 0..5 {
        std::thread::sleep(Duration::from_millis(30));
        test.write("b.txt", "b\n");
        assert!(harness
            .vcs
            .notify_disk_change(&repo, &[test.path("b.txt")])
            .is_none());
    }
    first.join();
    let (events, outcomes, _) = harness.drain();
    assert_eq!(outcomes, vec![TaskOutcome::Done]);
    match events.as_slice() {
        [VcsEvent::Status(status)] => assert_eq!(status.untracked().count(), 2),
        other => panic!("une seule relecture attendue : {other:?}"),
    }
    // Une fois la relecture faite, un nouveau changement en relance une.
    let again = harness
        .vcs
        .notify_disk_change(&repo, &[])
        .expect("nouvelle relecture");
    again.join();
}

#[test]
fn cancelling_a_task_publishes_no_failure() {
    let test = TestRepo::new();
    let harness = Harness::new(&test);
    let repo = test.repo();
    test.write("a.txt", "a\n");
    let handle = harness.vcs.notify_disk_change(&repo, &[]).unwrap();
    handle.cancel();
    handle.join();
    let (events, outcomes, _) = harness.drain();
    assert_eq!(outcomes, vec![TaskOutcome::Cancelled]);
    assert!(events.is_empty(), "{events:?}");
    // L'anti-rebond est libéré : on peut relancer.
    harness
        .vcs
        .notify_disk_change(&repo, &[])
        .expect("relance")
        .join();
}

// ---------------------------------------------------------------- v1 : branches et distant

#[test]
fn a_fetch_publishes_its_outcome_then_status_and_branches() {
    let team = support::Team::new();
    team.b_commit("b.txt", "b\n", "B");
    team.b_run(&["push", "-q"]);
    let harness = Harness::new(&team.a);
    let repo = team.a.repo();
    harness.vcs.fetch(&repo, FetchRequest::default()).join();
    let (events, outcomes, _) = harness.drain();
    assert_eq!(outcomes, vec![TaskOutcome::Done]);
    let events: Vec<&VcsEvent> = events
        .iter()
        .filter(|event| !matches!(event, VcsEvent::Progress(_)))
        .collect();
    match events.as_slice() {
        [VcsEvent::Fetched(fetched), VcsEvent::Status(status), VcsEvent::Branches(branches)] => {
            assert_eq!(fetched.updated(), 1);
            assert_eq!(status.upstream.as_ref().unwrap().ahead_behind, Some((0, 1)));
            let main = branches.find_local("main").unwrap();
            assert_eq!(main.tracking.as_ref().unwrap().behind, 1);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn an_unmerged_delete_fails_with_its_commits_then_force_deletes() {
    let test = TestRepo::new();
    test.write("a.txt", "a\n");
    test.commit_all("Base");
    test.run(&["switch", "-q", "-c", "seule"]);
    test.write("s.txt", "s\n");
    test.commit_all("Seul commit");
    test.run(&["switch", "-q", "main"]);
    let harness = Harness::new(&test);
    let repo = test.repo();
    harness.vcs.delete_branch(&repo, "seule".into()).join();
    let (events, outcomes, _) = harness.drain();
    assert!(matches!(outcomes.as_slice(), [TaskOutcome::Failed(_)]));
    match events.as_slice() {
        [VcsEvent::Status(_), VcsEvent::Branches(branches), VcsEvent::Failed(Failure {
            action: Action::DeleteBranch,
            error: VcsError::BranchNotMerged { commits: 1, .. },
        })] => assert!(branches.find_local("seule").is_some()),
        other => panic!("{other:?}"),
    }
    harness
        .vcs
        .force_delete_branch(&repo, "seule".into())
        .join();
    let (events, outcomes, _) = harness.drain();
    assert_eq!(outcomes, vec![TaskOutcome::Done]);
    match events.as_slice() {
        [VcsEvent::BranchDeleted(deleted), VcsEvent::Status(_), VcsEvent::Branches(branches)] => {
            assert_eq!(deleted.name, "seule");
            assert!(branches.find_local("seule").is_none());
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_second_write_waits_for_the_running_operation() {
    let team = support::Team::new();
    let a = &team.a;
    let script = a.base.join("ssh lent.sh");
    std::fs::write(&script, "#!/bin/sh\nsleep 30\n").unwrap();
    let ssh = format!("\"{}\"", script.to_string_lossy().replace('\\', "/"));
    a.run(&["config", "core.sshCommand", &ssh]);
    a.run(&[
        "remote",
        "add",
        "lent",
        "ssh://git@example.invalid/lent.git",
    ]);
    a.write("n.txt", "n");
    let harness = Harness::new(a);
    let repo = a.repo();
    let slow = harness.vcs.fetch(
        &repo,
        FetchRequest {
            remote: Some("lent".into()),
            prune: false,
        },
    );
    std::thread::sleep(Duration::from_millis(500));
    let stage = harness.vcs.stage(&repo, vec![a.path("n.txt")]);
    std::thread::sleep(Duration::from_millis(500));
    slow.cancel();
    slow.join();
    stage.join();
    let (events, outcomes, _) = harness.drain();
    assert!(outcomes.contains(&TaskOutcome::Cancelled), "{outcomes:?}");
    assert!(events.iter().any(|e| matches!(
        e,
        VcsEvent::Waiting {
            action: Action::Stage
        }
    )));
    assert!(events.iter().any(|e| matches!(e, VcsEvent::Staged { .. })));
    assert!(
        !events.iter().any(|e| matches!(e, VcsEvent::Failed(_))),
        "une annulation n'est pas un échec : {events:?}"
    );
}

#[test]
fn switching_through_the_service_reports_kept_stashes() {
    let test = TestRepo::new();
    test.write("a.txt", "un\n");
    test.commit_all("Base");
    test.run(&["switch", "-q", "-c", "autre"]);
    test.write("a.txt", "UN autre\n");
    test.commit_all("Autre");
    test.run(&["switch", "-q", "main"]);
    test.write("a.txt", "un local\n");
    let harness = Harness::new(&test);
    let repo = test.repo();
    harness
        .vcs
        .switch_branch(
            &repo,
            SwitchRequest {
                target: SwitchTarget::Branch("autre".into()),
                local_changes: LocalChanges::Keep,
            },
        )
        .join();
    let (events, _, _) = harness.drain();
    assert!(events.iter().any(|e| matches!(
        e,
        VcsEvent::Failed(Failure {
            action: Action::Switch,
            error: VcsError::WouldOverwrite(_)
        })
    )));
    harness
        .vcs
        .switch_branch(
            &repo,
            SwitchRequest {
                target: SwitchTarget::Branch("autre".into()),
                local_changes: LocalChanges::Stash,
            },
        )
        .join();
    let (events, outcomes, _) = harness.drain();
    assert_eq!(outcomes, vec![TaskOutcome::Done]);
    match events.first() {
        Some(VcsEvent::Switched(outcome)) => {
            let stash = outcome.stash.as_ref().unwrap();
            assert!(matches!(stash.restore, StashRestore::Conflicts(_)));
            // Une fois résolu, l'entrée gardée se restaure… ou se lit dans la liste.
            harness.vcs.load_stashes(&repo).join();
            let (events, _, _) = harness.drain();
            assert!(
                matches!(events.as_slice(), [VcsEvent::Stashes(list)] if list.len() == 1 && list[0].commit == stash.commit)
            );
        }
        other => panic!("{other:?}"),
    }
}

// ---------------------------------------------------------------- recette v1

/// M1 par le service : mise à jour en conflit, bandeau, abandon, remise.
#[test]
fn an_aborted_update_publishes_the_restored_stash_and_clears_the_banner() {
    let team = support::Team::new();
    let a = &team.a;
    std::fs::write(team.b.join("a.txt"), "un\nDEUX (B)\ntrois\n").unwrap();
    team.b_run(&["commit", "-q", "-am", "B"]);
    team.b_run(&["push", "-q"]);
    a.write("a.txt", "un\nDEUX (A)\ntrois\n");
    a.commit_all("A");
    a.write("local.txt", "local\n");
    let harness = Harness::new(a);
    let repo = a.repo();
    harness
        .vcs
        .update_project(
            &repo,
            UpdateRequest {
                method: IntegrationMethod::Merge,
                local_changes: LocalChanges::Stash,
            },
        )
        .join();
    let (events, _, _) = harness.drain_all();
    let pending = events
        .iter()
        .find_map(|event| match event {
            VcsEvent::PendingStashes(list) => Some(list.clone()),
            _ => None,
        })
        .expect("bandeau publié après l'écriture");
    assert_eq!(pending.len(), 1);
    assert_eq!(
        pending[0].state,
        PendingStashState::AwaitingOperation(OperationKind::Merge)
    );

    harness.vcs.abort_operation(&repo).join();
    let (events, outcomes, _) = harness.drain_all();
    assert_eq!(outcomes, vec![TaskOutcome::Done]);
    match events.as_slice() {
        [VcsEvent::OperationAborted(OperationKind::Merge), VcsEvent::StashRestored(stash), VcsEvent::Status(status), VcsEvent::PendingStashes(list)] =>
        {
            assert_eq!(stash.restore, StashRestore::Restored);
            assert_eq!(status.untracked().count(), 1, "local.txt est revenu");
            assert!(list.is_empty(), "plus de bandeau");
        }
        other => panic!("{other:?}"),
    }
}

/// M3 par le service : un commit sur une HEAD détachée publie le bandeau.
#[test]
fn a_commit_on_a_detached_head_publishes_the_banner() {
    let test = TestRepo::new();
    test.write("a.txt", "a\n");
    test.commit_all("Base");
    let base = test.head();
    test.write("b.txt", "b\n");
    test.commit_all("Deux");
    test.run(&["switch", "-q", "--detach", &base]);
    test.write("c.txt", "c\n");
    let harness = Harness::new(&test);
    let repo = test.repo();
    harness
        .vcs
        .commit(
            &repo,
            CommitRequest {
                paths: vec![test.path("c.txt")],
                message: "Orphelin".into(),
                amend: false,
            },
        )
        .join();
    let (events, _, _) = harness.drain_all();
    match events.as_slice() {
        [VcsEvent::Committed(_), VcsEvent::Status(_), VcsEvent::DetachedHead(report)] => {
            assert_eq!(report.orphan_commits, 1);
            assert_eq!(report.subjects, vec!["Orphelin".to_owned()]);
        }
        other => panic!("{other:?}"),
    }
    harness
        .vcs
        .switch_branch(
            &repo,
            SwitchRequest {
                target: SwitchTarget::Branch("main".into()),
                local_changes: LocalChanges::Keep,
            },
        )
        .join();
    let (events, _, _) = harness.drain();
    assert!(events.iter().any(|event| matches!(
        event,
        VcsEvent::Failed(Failure {
            action: Action::Switch,
            error: VcsError::DetachedCommitsLeft { count: 1, .. }
        })
    )));
}
