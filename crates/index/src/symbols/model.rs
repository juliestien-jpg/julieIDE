//! Types internes de l'index des symboles : déclarations gardées en mémoire
//! (et dans le cache), références et portées d'un fichier analysé.

use std::path::Path;

use super::SymbolKind;

/// Langage d'un fichier indexé.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Lang {
    Php,
    JavaScript,
    TypeScript,
    Tsx,
}

impl Lang {
    pub(crate) fn from_path(path: &Path) -> Option<Self> {
        Self::from_name(path.file_name()?.to_str()?)
    }

    pub(crate) fn from_name(name: &str) -> Option<Self> {
        let lower = name.to_ascii_lowercase();
        if lower.ends_with(".min.js") {
            return None; // code minifié : aucune déclaration utile, analyse coûteuse
        }
        let ext = lower.rsplit_once('.')?.1;
        Some(match ext {
            "php" | "phtml" | "inc" => Lang::Php,
            "js" | "mjs" | "cjs" | "jsx" => Lang::JavaScript,
            "ts" | "mts" | "cts" => Lang::TypeScript,
            "tsx" => Lang::Tsx,
            _ => return None,
        })
    }

    pub(crate) fn is_php(self) -> bool {
        self == Lang::Php
    }

    pub(crate) fn code(self) -> u8 {
        match self {
            Lang::Php => 0,
            Lang::JavaScript => 1,
            Lang::TypeScript => 2,
            Lang::Tsx => 3,
        }
    }

    pub(crate) fn from_code(code: u8) -> Option<Self> {
        Some(match code {
            0 => Lang::Php,
            1 => Lang::JavaScript,
            2 => Lang::TypeScript,
            3 => Lang::Tsx,
            _ => return None,
        })
    }

    pub(crate) fn grammar(self) -> tree_sitter::Language {
        match self {
            Lang::Php => tree_sitter_php::LANGUAGE_PHP.into(),
            Lang::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
            Lang::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Lang::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
        }
    }
}

/// Emplacement d'un nom : octets dans le texte, ligne et colonnes en caractères
/// (base zéro). Un nom tient sur une ligne ; sinon `end_col == col`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub(crate) struct Span {
    pub start: u32,
    pub end: u32,
    pub line: u32,
    pub col: u32,
    pub end_col: u32,
}

impl Span {
    pub(crate) fn contains(&self, byte: usize) -> bool {
        let byte = byte as u32;
        self.start <= byte && byte <= self.end && self.start < self.end
    }
}

/// Fanions d'une déclaration.
pub(crate) mod flags {
    pub const STATIC: u8 = 1;
    pub const EXPORTED: u8 = 2;
    pub const DEFAULT_EXPORT: u8 = 4;
    /// JS : déclaration au niveau du module (visible des autres fichiers si
    /// exportée, ou globale dans un script sans `import`/`export`).
    pub const MODULE_LEVEL: u8 = 8;
    /// PHP : constructeur promu en propriété.
    pub const PROMOTED: u8 = 16;
}

/// Lien d'une classe vers une autre.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BaseRel {
    /// `extends` (classe : parent ; interface : interfaces étendues).
    Extends,
    /// `implements`.
    Implements,
    /// `use Trait;` dans une classe ou un trait.
    Uses,
}

impl BaseRel {
    pub(crate) fn code(self) -> u8 {
        match self {
            BaseRel::Extends => 0,
            BaseRel::Implements => 1,
            BaseRel::Uses => 2,
        }
    }

    pub(crate) fn from_code(code: u8) -> Option<Self> {
        Some(match code {
            0 => BaseRel::Extends,
            1 => BaseRel::Implements,
            2 => BaseRel::Uses,
            _ => return None,
        })
    }
}

/// Une déclaration gardée dans l'index (et le cache).
///
/// - PHP : `container` = espace de noms pour une classe, une fonction, une
///   constante globale ; nom complet de la classe pour un membre. `types` =
///   types déclarés (propriété, retour), noms complets, `static` pour le type
///   tardif. `bases` : `extends`, `implements`, traits utilisés (noms complets).
/// - JS/TS : `container` = nom de la classe pour une méthode, vide sinon.
///   `types` d'une variable : `[C]` si elle reçoit `new C(…)`. `bases` d'une
///   classe : identifiant de `extends`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Decl {
    pub kind: SymbolKind,
    pub flags: u8,
    pub name: Box<str>,
    pub container: Box<str>,
    pub span: Span,
    pub types: Box<[Box<str>]>,
    pub bases: Box<[(BaseRel, Box<str>)]>,
}

impl Decl {
    pub(crate) fn has(&self, flag: u8) -> bool {
        self.flags & flag != 0
    }

    /// Nom qualifié affiché (`App\Model\User::getName`, `App\format_name`,
    /// `Api.get`).
    pub(crate) fn qualified(&self, lang: Lang) -> String {
        if lang.is_php() {
            match self.kind {
                k if k.is_member() => format!("{}::{}", self.container, self.display_name()),
                _ if self.container.is_empty() => self.name.to_string(),
                SymbolKind::Namespace => self.name.to_string(),
                _ => format!("{}\\{}", self.container, self.name),
            }
        } else if self.container.is_empty() {
            self.name.to_string()
        } else {
            format!("{}.{}", self.container, self.name)
        }
    }

    /// Le nom tel qu'on l'écrit à l'usage (`$` des propriétés statiques non
    /// compris : on montre le nom nu).
    pub(crate) fn display_name(&self) -> &str {
        &self.name
    }

    /// Nom complet en minuscules d'une classe PHP (`app\model\user`).
    pub(crate) fn php_fqn(&self) -> String {
        if self.container.is_empty() {
            self.name.to_string()
        } else {
            format!("{}\\{}", self.container, self.name)
        }
    }
}

/// Ce qu'un module JS importe, exporte et réexporte.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct JsModule {
    /// Le fichier a au moins un `import` ou un `export` (sinon c'est un script :
    /// ses déclarations de premier niveau sont globales).
    pub is_module: bool,
    pub imports: Vec<JsImport>,
    /// `export { a as b }`, `export default a` : (nom local, nom exporté).
    pub exports: Vec<(Box<str>, Box<str>)>,
    pub reexports: Vec<JsReexport>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct JsImport {
    pub local: Box<str>,
    pub source: Box<str>,
    pub imported: ImportName,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ImportName {
    Named(Box<str>),
    Default,
    Namespace,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct JsReexport {
    pub source: Box<str>,
    /// `None` : `export * from` ; sinon (nom importé, nom exporté).
    pub names: Option<Vec<(Box<str>, Box<str>)>>,
}

/// Ce que l'index garde d'un fichier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FileSymbols {
    pub lang: Lang,
    /// Empreinte du texte analysé (pour ne pas réanalyser le même contenu).
    pub content_hash: u64,
    pub decls: Vec<Decl>,
    /// Clés (empreintes) des noms référencés dans le code, triées : filtre des
    /// fichiers où chercher des usages (aucun faux négatif, quelques faux
    /// positifs).
    pub ref_keys: Box<[u32]>,
    pub js: Option<Box<JsModule>>,
}

impl FileSymbols {
    pub(crate) fn mentions(&self, key: u32) -> bool {
        self.ref_keys.binary_search(&key).is_ok()
    }
}

/// Genre de membre PHP ou JS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MemberKind {
    Method,
    Property,
    /// Constante de classe ou cas d'énumération.
    Constant,
}

impl MemberKind {
    pub(crate) fn tag(self) -> &'static str {
        match self {
            MemberKind::Method => "m",
            MemberKind::Property => "p",
            MemberKind::Constant => "o",
        }
    }

    pub(crate) fn matches(self, kind: SymbolKind) -> bool {
        match self {
            MemberKind::Method => kind == SymbolKind::Method,
            MemberKind::Property => kind == SymbolKind::Property,
            MemberKind::Constant => {
                matches!(kind, SymbolKind::ClassConstant | SymbolKind::EnumCase)
            }
        }
    }

    pub(crate) fn of(kind: SymbolKind) -> Option<Self> {
        Some(match kind {
            SymbolKind::Method => MemberKind::Method,
            SymbolKind::Property => MemberKind::Property,
            SymbolKind::ClassConstant | SymbolKind::EnumCase => MemberKind::Constant,
            _ => return None,
        })
    }

    /// Les noms de méthodes PHP sont insensibles à la casse, pas les propriétés
    /// ni les constantes.
    pub(crate) fn case_insensitive(self) -> bool {
        self == MemberKind::Method
    }
}

/// Type d'un receveur PHP, tel qu'on peut le dire sans l'index ; résolu
/// ensuite avec l'index (types des propriétés, retours des méthodes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Recv {
    /// Une ou plusieurs classes connues (noms complets).
    Classes(Vec<Box<str>>),
    /// Type de la propriété `nom` du receveur.
    Prop(Box<Recv>, Box<str>),
    /// Type de retour de la méthode `nom` du receveur.
    Call(Box<Recv>, Box<str>),
    /// Type de retour d'une fonction (candidats comme pour un appel).
    Func(Box<[Box<str>]>),
    Unknown,
}

/// Contexte d'une référence à une classe (pour l'affichage, pas le tri).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClassContext {
    Use,
    Extends,
    Implements,
    TraitUse,
    New,
    Static,
    InstanceOf,
    Catch,
    Type,
    Doc,
    Attribute,
    ClassConstant,
}

/// Receveur d'un accès membre JS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum JsRecv {
    /// `this` dans une méthode : index de la classe dans les déclarations.
    This(Option<u32>),
    /// Un identifiant, résolu par les portées.
    Ident(Box<str>, u32),
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RefKind {
    /// PHP : nom de classe résolu. `keyword` : `self`, `static`, `parent`
    /// (navigables, pas comptés comme usages).
    Class {
        fqn: Box<str>,
        keyword: bool,
        context: ClassContext,
    },
    /// PHP : appel de fonction ; candidats dans l'ordre de PHP (espace courant
    /// puis global), un seul s'il est qualifié ou importé.
    Function { candidates: Box<[Box<str>]> },
    /// PHP : constante globale, mêmes règles.
    Constant { candidates: Box<[Box<str>]> },
    /// PHP : accès à un membre.
    Member {
        member: MemberKind,
        recv: Recv,
        /// Texte du receveur (`$other`), pour le titre des candidats devinés.
        recv_text: Box<str>,
    },
    /// Variable locale ou paramètre (index dans `Analysis::locals`).
    Local { local: u32 },
    /// JS : identifiant à résoudre par les portées.
    JsIdent { scope: u32 },
    /// JS : nom dans un spécificateur d'import (`import { a as b }` : `a`
    /// compté, `b` non compté ; `import d from` : `d` compté).
    JsImportName { import: u32, counted: bool },
    /// JS : `x.nom`.
    JsMember { recv: JsRecv, recv_text: Box<str> },
    /// JS : chaîne du chemin d'un import ou d'un `require`.
    ModulePath { specifier: Box<str> },
}

/// Une référence dans un fichier analysé.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Ref {
    pub span: Span,
    pub name: Box<str>,
    pub kind: RefKind,
}

/// Variable locale, paramètre (PHP, JS), ou liaison locale JS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Local {
    pub name: Box<str>,
    pub kind: SymbolKind,
    /// Première occurrence (paramètre, première affectation, déclaration).
    pub span: Span,
    pub scope: u32,
    /// JS : `const x = new C()` → `C`.
    pub init_new: Option<Box<str>>,
    /// Classe englobante (PHP : nom complet ; JS : vide), pour le titre.
    pub container: Box<str>,
}

/// Portée JS (programme, bloc, fonction).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct JsScope {
    pub parent: Option<u32>,
    pub start: u32,
    pub end: u32,
    pub bindings: Vec<(Box<str>, Binding)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Binding {
    /// Déclaration de l'index (niveau module, classes).
    Decl(u32),
    Local(u32),
    Import(u32),
}

/// Résultat complet de l'analyse d'un fichier (transitoire : on ne le garde
/// que pour le fichier où l'on navigue et pendant une recherche d'usages).
#[derive(Debug, Clone)]
pub(crate) struct Analysis {
    pub symbols: FileSymbols,
    pub refs: Vec<Ref>,
    pub locals: Vec<Local>,
    /// PHP : (début en octets, espace de noms) dans l'ordre du fichier.
    pub namespaces: Vec<(u32, Box<str>)>,
    pub scopes: Vec<JsScope>,
}

impl Analysis {
    pub(crate) fn namespace_at(&self, byte: u32) -> &str {
        self.namespaces
            .iter()
            .rev()
            .find(|(start, _)| *start <= byte)
            .map_or("", |(_, ns)| ns)
    }

    /// Liaison JS de `name` vue depuis la portée `scope`.
    pub(crate) fn lookup(&self, mut scope: u32, name: &str) -> Option<Binding> {
        loop {
            let s = self.scopes.get(scope as usize)?;
            if let Some((_, b)) = s.bindings.iter().find(|(n, _)| &**n == name) {
                return Some(*b);
            }
            scope = s.parent?;
        }
    }
}

// ---------------------------------------------------------------------------
// Empreintes stables (FNV-1a) : elles vont dans le cache, elles ne doivent pas
// changer d'une compilation à l'autre (contrairement au hachage de `std`).

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

pub(crate) fn fnv(bytes: &[u8]) -> u64 {
    let mut hash = FNV_OFFSET;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// Empreinte de `tag:texte`, texte mis en minuscules ASCII si `lower`.
pub(crate) fn key(tag: &str, text: &str, lower: bool) -> u64 {
    let mut hash = FNV_OFFSET;
    for &b in tag.as_bytes().iter().chain(b":") {
        hash ^= b as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    for &b in text.as_bytes() {
        let b = if lower { b.to_ascii_lowercase() } else { b };
        hash ^= b as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// Clé courte (filtre des fichiers à lire pour les usages).
pub(crate) fn short_key(tag: &str, text: &str, lower: bool) -> u32 {
    let k = key(tag, text, lower);
    (k ^ (k >> 32)) as u32
}

/// Clé du nom complet d'une constante PHP : espace de noms insensible à la
/// casse, nom sensible.
pub(crate) fn php_const_key(fqn: &str) -> String {
    match fqn.rsplit_once('\\') {
        Some((ns, name)) => format!("{}\\{}", ns.to_ascii_lowercase(), name),
        None => fqn.to_string(),
    }
}

/// Tête du chemin d'un module JS, pour le filtre (`./util.js` → `util`,
/// `./dir/index` → `dir`).
pub(crate) fn module_stem(specifier: &str) -> String {
    let last = specifier
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(specifier);
    let stem = last.split('.').next().unwrap_or(last);
    stem.to_ascii_lowercase()
}

/// Colonnes en caractères, calculées au fil d'un parcours à peu près ordonné
/// (une ligne de 100 000 caractères ne coûte pas un parcours par nom).
pub(crate) struct Columns<'a> {
    text: &'a [u8],
    last_line: usize,
    last_byte: usize,
    last_col: usize,
}

impl<'a> Columns<'a> {
    pub(crate) fn new(text: &'a str) -> Self {
        Self {
            text: text.as_bytes(),
            last_line: usize::MAX,
            last_byte: 0,
            last_col: 0,
        }
    }

    /// Colonne (caractères) de l'octet `byte` sur la ligne `row` dont la colonne
    /// en octets est `byte_col`.
    pub(crate) fn col(&mut self, row: usize, byte: usize, byte_col: usize) -> usize {
        let line_start = byte.saturating_sub(byte_col);
        let (from, mut col) =
            if row == self.last_line && byte >= self.last_byte && self.last_byte >= line_start {
                (self.last_byte, self.last_col)
            } else {
                (line_start, 0)
            };
        let end = byte.min(self.text.len());
        if from < end {
            col += self.text[from..end]
                .iter()
                .filter(|&&b| (b & 0xC0) != 0x80)
                .count();
        }
        self.last_line = row;
        self.last_byte = byte;
        self.last_col = col;
        col
    }

    pub(crate) fn span(&mut self, node: tree_sitter::Node) -> Span {
        self.span_bytes(
            node.start_byte(),
            node.end_byte(),
            node.start_position(),
            node.end_position(),
        )
    }

    pub(crate) fn span_bytes(
        &mut self,
        start: usize,
        end: usize,
        start_pos: tree_sitter::Point,
        end_pos: tree_sitter::Point,
    ) -> Span {
        let col = self.col(start_pos.row, start, start_pos.column);
        let end_col = if end_pos.row == start_pos.row {
            col + self.text[start..end.min(self.text.len())]
                .iter()
                .filter(|&&b| (b & 0xC0) != 0x80)
                .count()
        } else {
            col
        };
        Span {
            start: start as u32,
            end: end as u32,
            line: start_pos.row as u32,
            col: col as u32,
            end_col: end_col as u32,
        }
    }
}

/// Texte d'un nœud (vide s'il n'est pas sur des frontières de caractères).
pub(crate) fn node_text<'a>(text: &'a str, node: tree_sitter::Node) -> &'a str {
    text.get(node.byte_range()).unwrap_or("")
}
