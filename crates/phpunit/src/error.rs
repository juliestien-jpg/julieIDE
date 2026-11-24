//! Erreurs de la crate.

use std::path::PathBuf;

/// Pourquoi une exécution ne peut pas être lancée. Des données, pas des messages :
/// l'interface choisit son texte dans le catalogue d'après la variante.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Problem {
    /// Aucun PHP dans le `PATH` ni à l'emplacement par défaut.
    #[error("PHP introuvable (cherché : {})", paths(.tried))]
    PhpNotFound { tried: Vec<PathBuf> },
    /// Le réglage `php_path` ne désigne pas un fichier.
    #[error("chemin de PHP invalide : {}", .0.display())]
    PhpPathInvalid(PathBuf),
    /// Le fichier existe mais ne s'exécute pas comme PHP.
    #[error("PHP ne s'exécute pas ({}) : {reason}", path.display())]
    PhpNotRunnable { path: PathBuf, reason: String },
    /// Ni `vendor/bin/phpunit`, ni phar.
    #[error("PHPUnit introuvable (cherché : {})", paths(.searched))]
    PhpUnitNotFound { searched: Vec<PathBuf> },
    /// Le réglage `phpunit_path` ne désigne pas un fichier.
    #[error("chemin de PHPUnit invalide : {}", .0.display())]
    PhpUnitPathInvalid(PathBuf),
    /// Ni `phpunit.xml` ni `phpunit.xml.dist` à la racine du projet.
    #[error("aucun phpunit.xml ni phpunit.xml.dist dans {}", root.display())]
    NoConfiguration { root: PathBuf },
    /// Le réglage `configuration` ne désigne pas un fichier.
    #[error("fichier de configuration PHPUnit introuvable : {}", .0.display())]
    ConfigurationPathInvalid(PathBuf),
}

fn paths(list: &[PathBuf]) -> String {
    list.iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Erreur d'une opération de la crate. `Core(Cancelled)` n'est pas un échec.
#[derive(Debug, thiserror::Error)]
pub enum PhpUnitError {
    #[error(transparent)]
    Core(#[from] julie_core::Error),
    #[error(transparent)]
    Launch(#[from] Problem),
}

impl PhpUnitError {
    pub fn is_cancelled(&self) -> bool {
        matches!(self, PhpUnitError::Core(error) if error.is_cancelled())
    }
}
