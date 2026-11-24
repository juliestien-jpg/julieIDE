//! Réglages de la section `[index]`.

use serde::Deserialize;

/// Section `[index]` des réglages (`settings.section::<IndexSettings>("index")`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct IndexSettings {
    /// Dossiers et fichiers jamais indexés, en plus de `.gitignore`. Un nom seul
    /// (`.git`) s'applique à toute profondeur ; un nom précédé de `/` (`/target`)
    /// seulement à la racine du projet. Défaut : `.git`, `.juliestorm`, `/target`.
    pub excluded: Vec<String>,
    /// Noms de dossiers dont le contenu est marqué « bibliothèque » (indexé, mais
    /// classé après les fichiers du projet). Défaut : `vendor`, `node_modules`.
    pub libraries: Vec<String>,
    /// Taille au-delà de laquelle la recherche de texte ignore un fichier (octets).
    /// Défaut : 10 Mio.
    pub max_file_size: u64,
    /// Fils de travail du parcours et de la recherche. Défaut : 4 (machine
    /// partagée).
    pub threads: usize,
}

impl Default for IndexSettings {
    fn default() -> Self {
        Self {
            excluded: vec![".git".into(), ".juliestorm".into(), "/target".into()],
            libraries: vec!["vendor".into(), "node_modules".into()],
            max_file_size: 10 * 1024 * 1024,
            threads: 4,
        }
    }
}

impl IndexSettings {
    pub(crate) fn thread_count(&self) -> usize {
        self.threads.clamp(1, 32)
    }

    /// Le chemin relatif (séparateurs `/`) est-il exclu ?
    pub(crate) fn is_excluded(&self, relative: &str) -> bool {
        let name = relative.rsplit('/').next().unwrap_or(relative);
        self.excluded
            .iter()
            .any(|rule| match rule.strip_prefix('/') {
                Some(rooted) => rooted.trim_end_matches('/') == relative,
                None => rule.trim_end_matches('/') == name,
            })
    }

    /// Un des dossiers du chemin relatif est-il une bibliothèque ?
    pub(crate) fn is_library(&self, relative: &str) -> bool {
        let mut parts = relative.split('/');
        parts.next_back();
        parts.any(|part| self.libraries.iter().any(|lib| lib == part))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exclusions_by_name_and_at_root() {
        let settings = IndexSettings::default();
        assert!(settings.is_excluded(".git"));
        assert!(settings.is_excluded("sub/module/.git"));
        assert!(settings.is_excluded("target"));
        assert!(!settings.is_excluded("src/target"));
        assert!(!settings.is_excluded("src/git"));
    }

    #[test]
    fn libraries_are_recognised_by_folder() {
        let settings = IndexSettings::default();
        assert!(settings.is_library("vendor/a/b.php"));
        assert!(settings.is_library("web/node_modules/x/index.js"));
        assert!(!settings.is_library("src/vendor.php"));
    }
}
