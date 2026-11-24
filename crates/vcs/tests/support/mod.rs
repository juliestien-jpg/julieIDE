//! Dépôts jetables pour les tests : dans le dossier temporaire, chemins avec
//! espaces, accents et apostrophe, configuration Git **isolée** de la machine
//! (ni `~/.gitconfig`, ni configuration système : pas de crochets globaux, pas
//! d'`autocrlf` imposé).

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Output;

use julie_core::CancelToken;
use julie_vcs::{open_repository, Git, Repository, VcsSettings};
use tempfile::TempDir;

pub struct TestRepo {
    _dir: TempDir,
    /// Dossier temporaire de base (hors dépôt).
    pub base: PathBuf,
    /// Racine de l'arbre de travail.
    pub root: PathBuf,
    pub global_config: PathBuf,
}

pub fn cancel() -> CancelToken {
    CancelToken::new()
}

impl TestRepo {
    /// Dépôt vide (sans commit) sur la branche `main`.
    pub fn new() -> Self {
        let repo = Self::empty_dir();
        repo.run(&["init", "-q", "-b", "main"]);
        repo
    }

    /// Dossier qui n'est pas un dépôt (même configuration isolée).
    pub fn empty_dir() -> Self {
        let dir = tempfile::Builder::new()
            .prefix("julie vcs é ")
            .tempdir()
            .expect("dossier temporaire");
        let base = dir.path().to_path_buf();
        let global_config = base.join("config globale.gitconfig");
        std::fs::write(
            &global_config,
            "[user]\n\tname = Essai Julie\n\temail = essai@example.invalid\n\
             [init]\n\tdefaultBranch = main\n\
             [commit]\n\tgpgsign = false\n\
             [core]\n\tautocrlf = false\n\
             [protocol \"file\"]\n\tallow = always\n",
        )
        .unwrap();
        let root = base.join("dépôt d'essai");
        std::fs::create_dir_all(&root).unwrap();
        Self {
            _dir: dir,
            base,
            root,
            global_config,
        }
    }

    /// Le `Git` de la bibliothèque, isolé de la configuration de la machine.
    pub fn git(&self) -> Git {
        Git::new(&VcsSettings::default())
            .with_env("GIT_CONFIG_GLOBAL", &self.global_config)
            .with_env("GIT_CONFIG_NOSYSTEM", "1")
            .with_env("GIT_CEILING_DIRECTORIES", &self.base)
    }

    pub fn repo(&self) -> Repository {
        open_repository(&self.git(), &self.root, &cancel()).expect("dépôt")
    }

    pub fn path(&self, relative: &str) -> PathBuf {
        let mut path = self.root.clone();
        for part in relative.split('/') {
            path.push(part);
        }
        path
    }

    /// `git` brut (préparation des tests), dans `dir`.
    pub fn raw_in(&self, dir: &Path, args: &[&str]) -> Output {
        std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", &self.global_config)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CEILING_DIRECTORIES", &self.base)
            .env("LC_ALL", "C")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_EDITOR", ":")
            .env("GIT_MERGE_AUTOEDIT", "no")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .output()
            .expect("git lancé")
    }

    pub fn try_run(&self, args: &[&str]) -> Output {
        self.raw_in(&self.root, args)
    }

    /// `git` brut ; panique en cas d'échec. Rend la sortie standard.
    pub fn run(&self, args: &[&str]) -> String {
        let output = self.try_run(args);
        assert!(
            output.status.success(),
            "git {args:?} : {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    pub fn write(&self, relative: &str, content: impl AsRef<[u8]>) {
        let path = self.path(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    pub fn read(&self, relative: &str) -> Vec<u8> {
        std::fs::read(self.path(relative)).unwrap()
    }

    pub fn exists(&self, relative: &str) -> bool {
        self.path(relative).exists()
    }

    pub fn commit_all(&self, message: &str) {
        self.run(&["add", "-A"]);
        self.run(&["commit", "-q", "-m", message]);
    }

    pub fn head(&self) -> String {
        self.run(&["rev-parse", "HEAD"]).trim().to_owned()
    }

    /// Fichiers du dernier commit (`git show --name-only`).
    pub fn files_in_head(&self) -> Vec<String> {
        let mut files: Vec<String> = self
            .run(&[
                "-c",
                "core.quotepath=false",
                "show",
                "--name-only",
                "--format=",
                "HEAD",
            ])
            .lines()
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect();
        files.sort();
        files
    }
}
