//! Modèle de projet : une racine et son dossier `.juliestorm/`.
//!
//! Volontairement minimal. La liste des fichiers ouverts appartient pour l'instant
//! à `app` ; la surveillance du disque viendra ici (voir `docs/contrats.md`).

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// Nom du dossier de l'IDE dans un projet : caches (index) et réglages du projet.
pub const DATA_DIR_NAME: &str = ".juliestorm";

/// Noms que la recherche et l'indexation excluent **toujours**, à toute
/// profondeur, quels que soient les réglages `[index] excluded` : le dépôt Git et
/// le dossier de l'IDE. L'interface les montre en tête de la liste des exclus,
/// grisés et non retirables.
pub const ALWAYS_EXCLUDED: [&str; 2] = [".git", DATA_DIR_NAME];

/// Un projet ouvert. Peu coûteux à cloner : à passer par valeur aux tâches de fond.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    root: PathBuf,
}

impl Project {
    /// Ouvre le dossier `root` comme projet. Fait un accès disque (vérifie que
    /// c'est un dossier) : à appeler depuis une tâche de fond. Ne crée rien.
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref();
        let absolute = std::path::absolute(root).map_err(|source| Error::io(root, source))?;
        let meta = fs::metadata(&absolute).map_err(|source| Error::io(&absolute, source))?;
        if !meta.is_dir() {
            return Err(Error::Invalid(format!(
                "{} n'est pas un dossier",
                absolute.display()
            )));
        }
        Ok(Self { root: absolute })
    }

    /// Racine du projet : chemin absolu, non canonisé (pas de préfixe `\\?\`).
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Nom à afficher : le dernier composant de la racine.
    pub fn name(&self) -> String {
        self.root
            .file_name()
            .unwrap_or(self.root.as_os_str())
            .to_string_lossy()
            .into_owned()
    }

    /// Chemin de `.juliestorm/`, qu'il existe ou non.
    pub fn data_dir(&self) -> PathBuf {
        self.root.join(DATA_DIR_NAME)
    }

    /// Crée `.juliestorm/` au besoin et rend son chemin. À la création, y dépose un
    /// `.gitignore` qui ignore tout le dossier, pour qu'il n'apparaisse pas dans
    /// les changements locaux. Accès disque : tâche de fond.
    pub fn ensure_data_dir(&self) -> Result<PathBuf> {
        let dir = self.data_dir();
        if !dir.is_dir() {
            fs::create_dir_all(&dir).map_err(|source| Error::io(&dir, source))?;
            let ignore = dir.join(".gitignore");
            fs::write(&ignore, "*\n").map_err(|source| Error::io(&ignore, source))?;
        }
        Ok(dir)
    }

    /// Fichier de réglages du projet (voir [`crate::settings::Settings`]).
    pub fn settings_path(&self) -> PathBuf {
        self.data_dir().join("settings.toml")
    }

    /// Chemin relatif à la racine, ou `None` si `path` est hors du projet.
    /// Comparaison purement lexicale, sans accès disque.
    pub fn relative<'a>(&self, path: &'a Path) -> Option<&'a Path> {
        path.strip_prefix(&self.root).ok()
    }

    /// Chemin absolu d'un chemin relatif à la racine.
    pub fn absolute(&self, relative: impl AsRef<Path>) -> PathBuf {
        self.root.join(relative)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_requires_an_existing_directory() {
        let dir = tempfile::tempdir().unwrap();
        let project = Project::open(dir.path()).unwrap();
        assert!(project.root().is_absolute());
        assert_eq!(
            project.name(),
            dir.path().file_name().unwrap().to_string_lossy()
        );

        assert!(matches!(
            Project::open(dir.path().join("absent")),
            Err(Error::Io { .. })
        ));
        let file = dir.path().join("fichier.txt");
        fs::write(&file, "x").unwrap();
        assert!(matches!(Project::open(&file), Err(Error::Invalid(_))));
    }

    #[test]
    fn data_dir_is_created_on_demand_and_ignored_by_git() {
        let dir = tempfile::tempdir().unwrap();
        let project = Project::open(dir.path()).unwrap();
        assert!(!project.data_dir().exists(), "ouvrir ne crée rien");

        let data = project.ensure_data_dir().unwrap();
        assert_eq!(data, project.root().join(".juliestorm"));
        assert_eq!(fs::read_to_string(data.join(".gitignore")).unwrap(), "*\n");

        // Un second appel ne réécrit pas un .gitignore modifié à la main.
        fs::write(data.join(".gitignore"), "cache/\n").unwrap();
        project.ensure_data_dir().unwrap();
        assert_eq!(
            fs::read_to_string(data.join(".gitignore")).unwrap(),
            "cache/\n"
        );
    }

    #[test]
    fn relative_and_absolute_paths() {
        let dir = tempfile::tempdir().unwrap();
        let project = Project::open(dir.path()).unwrap();
        let inside = project.absolute("src/Controller.php");
        assert_eq!(
            project.relative(&inside),
            Some(Path::new("src/Controller.php"))
        );
        assert_eq!(project.relative(dir.path().parent().unwrap()), None);
        assert_eq!(
            project.settings_path(),
            project.root().join(".juliestorm").join("settings.toml")
        );
    }
}
