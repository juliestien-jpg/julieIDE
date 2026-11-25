//! Index des symboles et navigation dans le code (aller à la déclaration,
//! trouver les usages). Spécification : `docs/specs/navigation.md`.
//!
//! - [`SymbolIndex`] : déclarations de tous les fichiers PHP et JS/TS du projet,
//!   construites par tree-sitter, mises à jour fichier par fichier, gardées
//!   dans un cache disque versionné ([`SymbolIndex::build`]).
//! - [`navigate`] : ce qu'il y a sous une position (cibles de « aller à la
//!   déclaration », ou déclaration dont on veut les usages).
//! - [`find_usages`] : usages d'une déclaration, en flux, séparés en sûrs et
//!   possibles.

mod cache;
mod js;
pub(crate) mod model;
mod php;
mod resolve;
mod store;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use julie_core::{TextPosition, TextRange};

pub use resolve::{find_usages, navigate};
pub use store::{SymbolBuildSummary, SymbolIndex, SYMBOL_MAX_FILE_SIZE};

pub(crate) use model::{Analysis, Lang};
pub(crate) use resolve::navigate_with;
pub(crate) use store::{build_into, update_into, BuildInput};

/// Analyse un texte sur un fil à grande pile.
pub(crate) fn store_analyze(lang: Lang, text: &str) -> Option<Analysis> {
    store::on_big_stack(|| store::analyze_text(lang, text)).flatten()
}

/// Nom du fichier de cache dans son dossier (`.juliestorm/index/symbols.bin`).
pub const SYMBOL_CACHE_FILE: &str = cache::CACHE_FILE_NAME;

/// Genre d'une déclaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SymbolKind {
    Namespace,
    Class,
    Interface,
    Trait,
    Enum,
    /// Cas d'une énumération PHP (`case Rouge;`).
    EnumCase,
    Function,
    Method,
    Property,
    /// Constante de classe PHP.
    ClassConstant,
    /// Constante globale PHP (`const`, `define`), `const` JS.
    Constant,
    /// Variable locale (PHP, JS `let`/`var`).
    Variable,
    Parameter,
    /// Liaison d'un `import` JS.
    Import,
    /// Alias de type TypeScript.
    TypeAlias,
}

impl SymbolKind {
    /// Méthode, propriété, constante de classe, cas d'énumération.
    pub fn is_member(self) -> bool {
        matches!(
            self,
            SymbolKind::Method
                | SymbolKind::Property
                | SymbolKind::ClassConstant
                | SymbolKind::EnumCase
        )
    }

    /// Classe, interface, trait, énumération.
    pub fn is_class_like(self) -> bool {
        matches!(
            self,
            SymbolKind::Class | SymbolKind::Interface | SymbolKind::Trait | SymbolKind::Enum
        )
    }

    /// Variable ou paramètre : n'existe que dans sa fonction.
    pub fn is_local(self) -> bool {
        matches!(self, SymbolKind::Variable | SymbolKind::Parameter)
    }

    pub(crate) fn code(self) -> u8 {
        self as u8
    }

    pub(crate) fn from_code(code: u8) -> Option<Self> {
        use SymbolKind::*;
        const ALL: [SymbolKind; 15] = [
            Namespace,
            Class,
            Interface,
            Trait,
            Enum,
            EnumCase,
            Function,
            Method,
            Property,
            ClassConstant,
            Constant,
            Variable,
            Parameter,
            Import,
            TypeAlias,
        ];
        ALL.get(code as usize).copied()
    }
}

/// Langage d'un symbole.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SymbolLanguage {
    Php,
    JavaScript,
    TypeScript,
}

impl From<Lang> for SymbolLanguage {
    fn from(lang: Lang) -> Self {
        match lang {
            Lang::Php => SymbolLanguage::Php,
            Lang::JavaScript => SymbolLanguage::JavaScript,
            Lang::TypeScript | Lang::Tsx => SymbolLanguage::TypeScript,
        }
    }
}

/// Une déclaration (cible d'un saut, ou symbole dont on cherche les usages).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declaration {
    pub kind: SymbolKind,
    pub language: SymbolLanguage,
    /// Nom tel que déclaré (`getName`, `User`, `$label`).
    pub name: String,
    /// Nom qualifié affiché (`App\Model\User::getName`, `App\format_name`,
    /// `Api.get`, `$label`).
    pub qualified_name: String,
    /// PHP : espace de noms d'une classe ou d'une fonction, classe d'un membre ;
    /// JS : classe d'une méthode ; vide sinon.
    pub container: String,
    /// Chemin absolu.
    pub path: PathBuf,
    /// Chemin relatif à la racine, séparateurs `/`.
    pub relative: String,
    /// Plage du nom déclaré (colonnes en caractères) : où placer le curseur.
    pub range: TextRange,
    /// Ligne de la déclaration, sans l'indentation (300 caractères au plus).
    pub line_text: String,
    /// Dans `vendor/`, `node_modules/`…
    pub library: bool,
}

/// Demande de navigation : où est le pointeur, et le texte tel qu'il est dans
/// l'éditeur (non enregistré compris).
#[derive(Debug, Clone)]
pub struct NavigationRequest {
    /// Chemin absolu du fichier.
    pub path: PathBuf,
    pub text: Arc<str>,
    /// Base zéro, colonne en caractères (`julie_core::TextPosition`).
    pub position: TextPosition,
}

/// Ce que donne une position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavigationResult {
    pub path: PathBuf,
    pub position: TextPosition,
    /// Le mot sous la position (pour le soulignement au survol avec Ctrl),
    /// s'il est navigable.
    pub word: Option<TextRange>,
    pub navigation: Navigation,
    /// `Some(pourcentage)` pendant la construction de l'index des symboles :
    /// le résultat peut être incomplet (« Indexation en cours (45 %) »).
    pub indexing: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Navigation {
    /// Aller à la déclaration : une cible (saut direct) ou plusieurs (liste
    /// surgissante, dans l'ordre donné).
    Targets(Targets),
    /// La position est sur une déclaration : l'interface cherche ses usages
    /// ([`crate::IndexService::find_usages`]).
    Declaration(Declaration),
    /// Chemin d'un `import`/`require` : ouvrir ce fichier (chemin absolu).
    OpenFile(PathBuf),
    /// Rien à faire (le curseur ne bouge pas) ; la raison donne le message.
    Nothing(NoTarget),
}

/// Cibles d'un « aller à la déclaration ».
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Targets {
    /// Jamais vide. Ordre : fichier courant, même espace de noms (même dossier
    /// en JS), projet, bibliothèques ; puis nom qualifié.
    pub targets: Vec<Declaration>,
    /// `Some` : cibles trouvées **par le seul nom**, jamais sûres (titre de la
    /// liste : « Type de $other inconnu : méthodes nommées getName »).
    pub guess: Option<Guess>,
    /// `new X(…)` : le constructeur (le sien ou hérité), proposé en plus de la
    /// classe (décision n° 9) ; il ne compte pas comme une seconde cible.
    pub constructor: Option<Declaration>,
}

/// Pourquoi les cibles sont devinées.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Guess {
    /// Accès membre sur une valeur de type inconnu : toutes les déclarations du
    /// même genre et du même nom.
    UnknownReceiver {
        /// Texte du receveur (`$other`, `this.items`), 40 caractères au plus.
        receiver: String,
        member: String,
        kind: SymbolKind,
    },
    /// JS : identifiant global (script sans modules) déclaré dans d'autres
    /// fichiers.
    GlobalName { name: String },
}

/// Pourquoi il n'y a pas de cible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoTarget {
    /// Pas un nom de code : commentaire, chaîne, espace, mot-clé.
    NotASymbol,
    /// Nom global sans déclaration dans le projet (fonction de PHP comme
    /// `strlen`, `fetch` du navigateur…) : « Aucune déclaration dans le projet ».
    NotInProject { name: String },
    /// Membre absent de sa classe (et de ses parents), ou aucun candidat :
    /// « Aucune déclaration trouvée ».
    NotFound { name: String },
    /// Nom calculé à l'exécution (`$obj->$nom()`, `new $classe`).
    DynamicName,
    /// Module nu (`'react'`) ou alias : « Module externe : non résolu ».
    ExternalModule { specifier: String },
}

/// Demande d'usages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsagesQuery {
    /// La déclaration (telle que rendue par [`Navigation::Declaration`] ou
    /// [`Targets`]).
    pub symbol: Declaration,
    /// Chercher aussi dans `vendor/`, `node_modules/` (décochée par défaut).
    pub include_libraries: bool,
}

impl UsagesQuery {
    pub fn new(symbol: Declaration) -> Self {
        Self {
            symbol,
            include_libraries: false,
        }
    }
}

/// Sûr (receveur résolu) ou possible (même nom, type inconnu).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UsageCertainty {
    Sure,
    Possible,
}

/// Un usage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Usage {
    pub certainty: UsageCertainty,
    pub path: PathBuf,
    pub relative: String,
    /// Plage de l'occurrence, colonnes en caractères de la ligne entière (à
    /// passer telle quelle à l'éditeur pour la sélectionner).
    pub range: TextRange,
    /// Ligne de l'usage, sans l'indentation, 300 caractères au plus.
    pub line_text: String,
    /// Colonne (caractères) de la ligne où commence `line_text`.
    pub line_text_start: u32,
    pub library: bool,
    /// Cherché dans le contenu non enregistré d'un onglet.
    pub unsaved: bool,
}

/// Bilan d'une recherche d'usages (ce qui a été cherché, et ce qui ne l'a pas
/// été : jamais « tous les usages »).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsagesSummary {
    pub symbol: Declaration,
    pub sure: usize,
    pub possible: usize,
    pub files_with_sure: usize,
    pub files_with_possible: usize,
    /// Fichiers lus et analysés.
    pub files_searched: usize,
    /// Fichiers de bibliothèques qui mentionnent le nom et n'ont pas été lus
    /// (case « Inclure les bibliothèques » décochée).
    pub library_files_skipped: usize,
    /// Fichiers candidats illisibles, binaires ou trop gros.
    pub unreadable: usize,
    /// Membre : les appels dynamiques (`$obj->$nom()`, `call_user_func`, noms
    /// dans des chaînes) ne sont pas cherchés (ligne d'aide du pied).
    pub dynamic_not_searched: bool,
    /// L'index des symboles était complet au départ (sinon « résultats
    /// incomplets »).
    pub index_complete: bool,
    pub elapsed: Duration,
}
