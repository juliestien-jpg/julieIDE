//! Les fichiers de réglages et leur écriture clé par clé.
//!
//! L'écriture ne touche que les clés demandées : le reste du fichier
//! (commentaires, ordre, clés et tables inconnues, encodage, fins de ligne) est
//! rendu octet pour octet. Un fichier que l'on ne sait pas lire n'est jamais
//! réécrit.
//!
//! Tout ce module fait des entrées-sorties **bloquantes** : tâche de fond.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use toml_edit::{Array, Decor, DocumentMut, InlineTable, Item, Table, TableLike, Value};

use super::{read_table, Scope, Settings, SettingsProblem, NOT_UTF8};
use crate::error::{Error, Result};
use crate::file::{self, Encoding, LineEnding, TextFormat};
use crate::project::Project;

/// Première ligne d'un fichier de réglages créé par l'IDE.
const USER_HEADER: &str =
    "# Réglages de JulieStorm pour tous les projets. Ce fichier peut être modifié à la main.";
const PROJECT_HEADER: &str =
    "# Réglages de JulieStorm pour ce projet. Ce fichier peut être modifié à la main.";

/// Un fichier neuf : UTF-8 sans BOM, fins de ligne CRLF.
const NEW_FILE_FORMAT: TextFormat = TextFormat {
    encoding: Encoding::Utf8,
    line_ending: LineEnding::CrLf,
};

/// Au-delà de cette largeur, un tableau s'écrit un élément par ligne.
const ARRAY_LINE_WIDTH: usize = 80;

/// Valeur d'un réglage à écrire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingValue {
    Text(String),
    /// Chemin : écrit en chaîne littérale TOML (`'C:\…'`, barres obliques inverses
    /// non doublées), sauf s'il contient une apostrophe.
    Path(PathBuf),
    Bool(bool),
    Integer(i64),
    /// Tableau de chaînes (`extra_args`, `excluded`, `protected_branches`).
    List(Vec<String>),
}

/// Une modification d'un fichier de réglages. Le chemin d'une clé commence par la
/// section : `["phpunit", "php_path"]`, `["phpunit", "env", "APP_ENV"]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsEdit {
    /// Écrit la clé (en créant les tables qui manquent).
    Set {
        path: Vec<String>,
        value: SettingValue,
    },
    /// Retire la clé : « revenir à la valeur héritée ».
    Unset { path: Vec<String> },
}

impl SettingsEdit {
    pub fn set(path: &[&str], value: SettingValue) -> Self {
        SettingsEdit::Set {
            path: path.iter().map(|part| (*part).to_owned()).collect(),
            value,
        }
    }

    pub fn unset(path: &[&str]) -> Self {
        SettingsEdit::Unset {
            path: path.iter().map(|part| (*part).to_owned()).collect(),
        }
    }
}

/// Où sont les fichiers de réglages des deux portées. Peu coûteux à cloner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsFiles {
    user: Option<PathBuf>,
    project: Option<Project>,
}

impl SettingsFiles {
    /// Les fichiers réels : `%APPDATA%\JulieStorm\settings.toml` et celui du projet.
    pub fn locate(project: Option<&Project>) -> Self {
        Self::new(super::user_settings_path(), project.cloned())
    }

    /// Avec un fichier utilisateur explicite : **les tests passent par ici**, pour
    /// ne jamais toucher aux réglages réels de la machine.
    pub fn new(user_file: Option<PathBuf>, project: Option<Project>) -> Self {
        Self {
            user: user_file,
            project,
        }
    }

    /// Chemin du fichier de cette portée, qu'il existe ou non. `None` : pas de
    /// projet ouvert, ou pas de dossier de configuration connu.
    pub fn path(&self, scope: Scope) -> Option<PathBuf> {
        match scope {
            Scope::User => self.user.clone(),
            Scope::Project => self.project.as_ref().map(Project::settings_path),
        }
    }

    /// Lecture stricte : un fichier illisible est une erreur ([`Error::Settings`]).
    pub fn load(&self) -> Result<Settings> {
        Settings::load_files(
            self.path(Scope::User).as_deref(),
            self.path(Scope::Project).as_deref(),
        )
    }

    /// Lecture tolérante : un fichier illisible compte pour vide et est signalé.
    /// Pour le démarrage ; ensuite, [`super::SettingsWatcher`] garde les derniers
    /// réglages valides.
    pub fn load_lenient(&self) -> (Settings, Vec<SettingsProblem>) {
        let mut problems = Vec::new();
        let mut table = |scope| match self.path(scope) {
            None => toml::Table::new(),
            Some(path) => read_table(scope, &path).unwrap_or_else(|problem| {
                problems.push(problem);
                toml::Table::new()
            }),
        };
        let user = table(Scope::User);
        let project = table(Scope::Project);
        (Settings::from_tables(user, project), problems)
    }

    /// Écrit une clé. Voir [`SettingsFiles::apply`].
    pub fn set(&self, scope: Scope, path: &[&str], value: SettingValue) -> Result<()> {
        self.apply(scope, &[SettingsEdit::set(path, value)])
    }

    /// Retire une clé (« revenir à la valeur héritée »). Voir
    /// [`SettingsFiles::apply`].
    pub fn unset(&self, scope: Scope, path: &[&str]) -> Result<()> {
        self.apply(scope, &[SettingsEdit::unset(path)])
    }

    /// Applique des modifications au fichier d'une portée, en **une** écriture
    /// atomique.
    ///
    /// - Le fichier est relu juste avant : les clés qu'une autre instance de l'IDE
    ///   (ou la main) vient d'y écrire sont conservées.
    /// - Seules les clés demandées changent ; commentaires, ordre, clés et tables
    ///   inconnues, encodage et fins de ligne sont conservés. Une valeur déjà
    ///   égale n'est pas réécrite ; sans changement, le fichier n'est pas touché.
    /// - Fichier absent : créé (avec son dossier) à la première clé écrite, avec
    ///   une ligne de commentaire en tête ; retirer une clé ne crée rien.
    /// - Retirer une clé garde le commentaire qui la précédait. Une table devenue
    ///   vide est retirée, sauf si elle porte un commentaire.
    /// - Fichier mal formé ou qui n'est pas de l'UTF-8 :
    ///   [`Error::SettingsUnreadable`], rien n'est écrit. Fichier en lecture
    ///   seule : [`Error::Io`] qui nomme le fichier.
    pub fn apply(&self, scope: Scope, edits: &[SettingsEdit]) -> Result<()> {
        let path = self.path(scope).ok_or_else(|| {
            Error::Invalid(match scope {
                Scope::User => "aucun dossier de réglages utilisateur".to_owned(),
                Scope::Project => "aucun projet ouvert".to_owned(),
            })
        })?;
        for edit in edits {
            let (SettingsEdit::Set { path: key, .. } | SettingsEdit::Unset { path: key }) = edit;
            if key.len() < 2 || key.iter().any(String::is_empty) {
                return Err(Error::Invalid(format!(
                    "chemin de réglage invalide : {key:?} (attendu : section, puis clé)"
                )));
            }
        }

        let existing = read_existing(&path)?;
        if existing.is_none() && !edits.iter().any(|e| matches!(e, SettingsEdit::Set { .. })) {
            return Ok(());
        }
        let is_new = existing.is_none();
        let (original, format) = match existing {
            Some(file) => (file.text, file.format),
            None => (format!("{}\n", self.header(scope)), NEW_FILE_FORMAT),
        };

        let mut document: DocumentMut =
            original
                .parse()
                .map_err(|error: toml_edit::TomlError| Error::SettingsUnreadable {
                    path: path.clone(),
                    line: Some(super::line_of(
                        &original,
                        error.span().map_or(0, |span| span.start),
                    )),
                    message: error.message().to_owned(),
                })?;
        let baseline = document.to_string();
        for edit in edits {
            match edit {
                SettingsEdit::Set { path: key, value } => set_value(&mut document, key, value)?,
                SettingsEdit::Unset { path: key } => {
                    unset_value(document.as_table_mut(), key);
                }
            }
        }
        let mut updated = document.to_string();
        if !is_new && updated == baseline {
            return Ok(());
        }
        // Un fichier sans fin de ligne finale le reste.
        if !original.ends_with('\n')
            && baseline.strip_suffix('\n') == Some(original.as_str())
            && updated.ends_with('\n')
        {
            updated.pop();
        }
        // Garde-fou : on n'écrit jamais un fichier que l'on ne saurait pas relire.
        if let Err(error) = updated.parse::<toml::Table>() {
            return Err(Error::Invalid(format!(
                "réglages non écrits dans {} : le résultat ne serait pas du TOML valide ({})",
                path.display(),
                error.message()
            )));
        }

        self.ensure_dir(scope, &path)?;
        file::write_text(&path, &updated, format)
    }

    /// Crée le fichier de cette portée s'il n'existe pas, avec seulement sa ligne
    /// de commentaire, et rend son chemin (« Ouvrir le fichier de réglages »).
    pub fn ensure_file(&self, scope: Scope) -> Result<PathBuf> {
        let path = self
            .path(scope)
            .ok_or_else(|| Error::Invalid("aucun fichier de réglages à cette portée".to_owned()))?;
        if existing_is_new(&path) {
            self.ensure_dir(scope, &path)?;
            let text = format!("{}\n", self.header(scope));
            file::write_text(&path, &text, NEW_FILE_FORMAT)?;
        }
        Ok(path)
    }

    fn header(&self, scope: Scope) -> &'static str {
        match scope {
            Scope::User => USER_HEADER,
            Scope::Project => PROJECT_HEADER,
        }
    }

    /// Crée le dossier du fichier : `.juliestorm/` (avec son `.gitignore`) pour le
    /// projet, `%APPDATA%\JulieStorm\` pour l'utilisateur.
    fn ensure_dir(&self, scope: Scope, path: &Path) -> Result<()> {
        match (scope, &self.project) {
            (Scope::Project, Some(project)) => project.ensure_data_dir().map(|_| ()),
            _ => match path.parent() {
                Some(parent) => {
                    fs::create_dir_all(parent).map_err(|source| Error::io(parent, source))
                }
                None => Ok(()),
            },
        }
    }
}

fn existing_is_new(path: &Path) -> bool {
    matches!(fs::symlink_metadata(path), Err(error) if error.kind() == io::ErrorKind::NotFound)
}

/// Le fichier existant, décodé ; `None` s'il n'existe pas.
fn read_existing(path: &Path) -> Result<Option<file::TextFile>> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(Error::io(path, source)),
    };
    let decoded = file::decode(bytes);
    if decoded.format.encoding == Encoding::Windows1252 {
        return Err(Error::SettingsUnreadable {
            path: path.to_path_buf(),
            line: None,
            message: NOT_UTF8.to_owned(),
        });
    }
    Ok(Some(decoded))
}

fn invalid_key(path: &[String], reason: &str) -> Error {
    Error::Invalid(format!("réglage {} : {reason}", path.join(".")))
}

impl SettingValue {
    fn to_toml(&self, path: &[String]) -> Result<Value> {
        Ok(match self {
            SettingValue::Text(text) => Value::from(text.as_str()),
            SettingValue::Path(value) => {
                let text = value
                    .to_str()
                    .ok_or_else(|| invalid_key(path, "le chemin n'est pas de l'Unicode valide"))?;
                literal_string(text).unwrap_or_else(|| Value::from(text))
            }
            SettingValue::Bool(value) => Value::from(*value),
            SettingValue::Integer(value) => Value::from(*value),
            SettingValue::List(items) => {
                let mut array: Array = items.iter().map(String::as_str).collect();
                if array.to_string().len() > ARRAY_LINE_WIDTH {
                    for item in array.iter_mut() {
                        item.decor_mut().set_prefix("\n    ");
                    }
                    array.set_trailing_comma(true);
                    array.set_trailing("\n");
                }
                Value::Array(array)
            }
        })
    }

    /// La valeur déjà écrite est-elle celle-ci ? (On ne réécrit pas une valeur
    /// égale : la façon dont elle a été tapée à la main est conservée.)
    fn matches(&self, existing: &Value) -> bool {
        match self {
            SettingValue::Text(text) => existing.as_str() == Some(text.as_str()),
            SettingValue::Path(path) => path.to_str().is_some_and(|p| existing.as_str() == Some(p)),
            SettingValue::Bool(value) => existing.as_bool() == Some(*value),
            SettingValue::Integer(value) => existing.as_integer() == Some(*value),
            SettingValue::List(items) => existing.as_array().is_some_and(|array| {
                array.len() == items.len()
                    && array
                        .iter()
                        .zip(items)
                        .all(|(have, want)| have.as_str() == Some(want.as_str()))
            }),
        }
    }
}

/// `'texte'` si le texte peut s'écrire en chaîne littérale TOML d'une ligne.
fn literal_string(text: &str) -> Option<Value> {
    if text.contains('\'') || text.chars().any(char::is_control) {
        return None;
    }
    format!("'{text}'").parse::<Value>().ok()
}

fn set_value(document: &mut DocumentMut, path: &[String], value: &SettingValue) -> Result<()> {
    let Some((key, parents)) = path.split_last() else {
        return Ok(());
    };
    let new_table_position = max_position(document.as_table()) + 1;
    let root_is_empty = document.as_table().is_empty();
    let mut created_top_level = false;

    let mut table: &mut dyn TableLike = document.as_table_mut();
    for (depth, name) in parents.iter().enumerate() {
        if table.get(name).is_none() {
            let mut created = Table::new();
            // Une table qui ne portera que des sous-tables n'a pas d'en-tête.
            created.set_implicit(depth + 1 < parents.len());
            if depth == 0 {
                created.set_position(Some(new_table_position));
                created_top_level = true;
            }
            table.insert(name, Item::Table(created));
        }
        table = table
            .get_mut(name)
            .and_then(Item::as_table_like_mut)
            .ok_or_else(|| invalid_key(path, "une clé parente n'est pas une table"))?;
    }

    match table.get_mut(key) {
        Some(Item::Value(existing)) => {
            if !value.matches(existing) {
                let mut replacement = value.to_toml(path)?;
                // Espaces et commentaire de fin de ligne de l'ancienne valeur.
                *replacement.decor_mut() = existing.decor().clone();
                *existing = replacement;
            }
        }
        Some(_) => return Err(invalid_key(path, "la clé existante est une table")),
        None => {
            table.insert(key, Item::Value(value.to_toml(path)?));
        }
    }

    if created_top_level {
        // La nouvelle table va à la fin du fichier : ce qui terminait le fichier
        // (commentaires, lignes vides) reste avant elle.
        let trailing = document.trailing().as_str().unwrap_or("").to_owned();
        let mut prefix = trailing;
        if !prefix.is_empty() && !prefix.ends_with('\n') {
            prefix.push('\n');
        }
        if !(prefix.is_empty() && root_is_empty) && !prefix.ends_with("\n\n") {
            prefix.push('\n');
        }
        document.set_trailing("");
        if let Some(Item::Table(created)) = document.as_table_mut().get_mut(&parents[0]) {
            created.decor_mut().set_prefix(prefix);
        }
    }
    Ok(())
}

/// Plus grande position d'une table dans le document (ordre d'écriture).
fn max_position(table: &Table) -> isize {
    let mut max = table.position().unwrap_or(0);
    for (_, item) in table.iter() {
        match item {
            Item::Table(child) => max = max.max(max_position(child)),
            Item::ArrayOfTables(tables) => {
                for child in tables.iter() {
                    max = max.max(max_position(child));
                }
            }
            _ => {}
        }
    }
    max
}

/// Retire la clé ; rend `true` si elle existait.
fn unset_value(table: &mut Table, path: &[String]) -> bool {
    let Some((name, rest)) = path.split_first() else {
        return false;
    };
    if rest.is_empty() {
        return remove_key(table, name);
    }
    let (removed, now_empty) = match table.get_mut(name) {
        Some(Item::Table(child)) => {
            let removed = unset_value(child, rest);
            let empty = child.is_empty() && !has_comment(child.decor());
            (removed, empty)
        }
        Some(Item::Value(Value::InlineTable(inline))) => (unset_inline(inline, rest), false),
        _ => (false, false),
    };
    if removed && now_empty {
        table.remove(name);
    }
    removed
}

fn unset_inline(table: &mut InlineTable, path: &[String]) -> bool {
    match path.split_first() {
        None => false,
        Some((name, [])) => table.remove(name).is_some(),
        Some((name, rest)) => match table.get_mut(name) {
            Some(Value::InlineTable(child)) => unset_inline(child, rest),
            _ => false,
        },
    }
}

/// Retire une clé d'une table en gardant le commentaire qui la précédait : il est
/// raccroché à ce qui se trouve juste avant (la valeur précédente, sinon l'en-tête
/// de la table), donc ne change pas de place dans le fichier.
fn remove_key(table: &mut Table, name: &str) -> bool {
    let previous_value: Option<String> = table
        .iter()
        .take_while(|(key, _)| *key != name)
        .filter(|(_, item)| item.is_value())
        .last()
        .map(|(key, _)| key.to_owned());
    let Some((key, _)) = table.remove_entry(name) else {
        return false;
    };

    let prefix = key
        .leaf_decor()
        .prefix()
        .and_then(|raw| raw.as_str())
        .unwrap_or("");
    if !prefix.contains('#') {
        return true;
    }
    // Le préfixe finit par l'indentation de la clé retirée : on s'arrête à la fin
    // de la dernière ligne de commentaire.
    let kept = match prefix.rfind('\n') {
        Some(end) => &prefix[..end],
        None => prefix,
    };
    let append = |decor: &mut Decor| {
        let mut suffix = decor
            .suffix()
            .and_then(|raw| raw.as_str())
            .unwrap_or("")
            .to_owned();
        suffix.push('\n');
        suffix.push_str(kept);
        decor.set_suffix(suffix);
    };
    match previous_value
        .and_then(|previous| table.get_mut(&previous))
        .and_then(Item::as_value_mut)
    {
        Some(previous) => append(previous.decor_mut()),
        None => append(table.decor_mut()),
    }
    true
}

fn has_comment(decor: &Decor) -> bool {
    [decor.prefix(), decor.suffix()]
        .into_iter()
        .flatten()
        .any(|raw| raw.as_str().is_some_and(|text| text.contains('#')))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Origin;

    struct Fixture {
        _dir: tempfile::TempDir,
        files: SettingsFiles,
        user: PathBuf,
        project: PathBuf,
    }

    /// Un dossier utilisateur et un projet jetables, avec espaces et accents.
    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let user = dir
            .path()
            .join("AppData é (x86)")
            .join("JulieStorm")
            .join("settings.toml");
        let root = dir.path().join("mon projet été");
        fs::create_dir_all(&root).unwrap();
        let project = Project::open(&root).unwrap();
        let project_file = project.settings_path();
        Fixture {
            files: SettingsFiles::new(Some(user.clone()), Some(project)),
            user,
            project: project_file,
            _dir: dir,
        }
    }

    fn text(path: &Path) -> String {
        String::from_utf8(fs::read(path).unwrap()).unwrap()
    }

    #[test]
    fn the_first_key_creates_the_file_with_its_comment_line() {
        let f = fixture();
        assert_eq!(f.files.path(Scope::Project), Some(f.project.clone()));
        f.files
            .set(
                Scope::Project,
                &["phpunit", "phpunit_path"],
                SettingValue::Path(PathBuf::from(r"outils\phpunit.phar")),
            )
            .unwrap();
        assert_eq!(
            text(&f.project),
            format!(
                "{PROJECT_HEADER}\r\n\r\n[phpunit]\r\nphpunit_path = 'outils\\phpunit.phar'\r\n"
            )
        );
        // .juliestorm/ a été créé avec son .gitignore.
        assert_eq!(
            fs::read_to_string(f.project.with_file_name(".gitignore")).unwrap(),
            "*\n"
        );

        // %APPDATA%\JulieStorm\ est créé à la première écriture.
        assert!(!f.user.parent().unwrap().exists());
        f.files
            .set(
                Scope::User,
                &["app", "theme"],
                SettingValue::Text("clair".into()),
            )
            .unwrap();
        assert_eq!(
            text(&f.user),
            format!("{USER_HEADER}\r\n\r\n[app]\r\ntheme = \"clair\"\r\n")
        );
    }

    #[test]
    fn changing_one_key_changes_one_line_only() {
        let f = fixture();
        fs::create_dir_all(f.project.parent().unwrap()).unwrap();
        let before = "\u{FEFF}# Mes réglages, écrits à la main\r\n\
            \r\n\
            [perso]\r\n\
            couleur = \"bleu\"   # table inconnue de l'IDE\r\n\
            \r\n\
            [phpunit]\r\n\
            # le PHP de LabCollector\r\n\
            php_path = 'C:\\Program Files (x86)\\AgileBio\\php\\php.exe'  # ne pas toucher\r\n\
            phpunit_path   =   \"tools/phpunit.phar\" # à changer\r\n\
            timeout_seconds = 30\r\n\
            \r\n\
            # fin du fichier\r\n";
        fs::write(&f.project, before).unwrap();

        f.files
            .set(
                Scope::Project,
                &["phpunit", "phpunit_path"],
                SettingValue::Path(PathBuf::from(r"outils\phpunit.phar")),
            )
            .unwrap();

        let after = text(&f.project);
        let expected = before.replace(
            "phpunit_path   =   \"tools/phpunit.phar\" # à changer",
            "phpunit_path   =   'outils\\phpunit.phar' # à changer",
        );
        assert_eq!(after, expected);
    }

    #[test]
    fn an_equal_value_or_a_missing_key_does_not_rewrite_the_file() {
        let f = fixture();
        fs::create_dir_all(f.project.parent().unwrap()).unwrap();
        // Chemin écrit à la main en chaîne ordinaire, sans fin de ligne finale.
        let before = "[phpunit]\nphp_path = \"C:\\\\php\\\\php.exe\"";
        fs::write(&f.project, before).unwrap();
        let stamp = fs::metadata(&f.project).unwrap().modified().unwrap();

        f.files
            .apply(
                Scope::Project,
                &[
                    SettingsEdit::set(
                        &["phpunit", "php_path"],
                        SettingValue::Path(PathBuf::from(r"C:\php\php.exe")),
                    ),
                    SettingsEdit::unset(&["phpunit", "configuration"]),
                    SettingsEdit::unset(&["vcs", "git_path"]),
                ],
            )
            .unwrap();
        assert_eq!(text(&f.project), before);
        assert_eq!(fs::metadata(&f.project).unwrap().modified().unwrap(), stamp);

        // Une vraie modification ne lui ajoute pas de fin de ligne finale.
        f.files
            .set(
                Scope::Project,
                &["phpunit", "php_path"],
                SettingValue::Path("php".into()),
            )
            .unwrap();
        assert_eq!(text(&f.project), "[phpunit]\nphp_path = 'php'");
        fs::write(&f.project, before).unwrap();

        // Retirer une clé d'un fichier absent ne crée rien.
        f.files.unset(Scope::User, &["vcs", "git_path"]).unwrap();
        assert!(!f.user.exists());
    }

    #[test]
    fn unsetting_a_key_keeps_comments_and_drops_tables_left_empty() {
        let f = fixture();
        fs::create_dir_all(f.project.parent().unwrap()).unwrap();
        fs::write(
            &f.project,
            "# en-tête\n\
             [phpunit]\n\
             php_path = 'php.exe' # le mien\n\
             \n\
             # note sur le script\n\
             phpunit_path = 'a.phar'\n\
             extra_args = ['--testdox']\n\
             \n\
             [phpunit.env]\n\
             APP_ENV = 'test'\n\
             \n\
             [vcs]\n\
             git_path = 'git.exe'\n\
             \n\
             # commentée : gardée même vide\n\
             [terminal]\n\
             scrollback = 5000\n\
             \n\
             [index]\n\
             # pourquoi j'exclus docs\n\
             excluded = ['docs']\n",
        )
        .unwrap();

        f.files
            .apply(
                Scope::Project,
                &[
                    SettingsEdit::unset(&["phpunit", "phpunit_path"]),
                    SettingsEdit::unset(&["phpunit", "env", "APP_ENV"]),
                    SettingsEdit::unset(&["vcs", "git_path"]),
                    SettingsEdit::unset(&["terminal", "scrollback"]),
                    SettingsEdit::unset(&["index", "excluded"]),
                ],
            )
            .unwrap();

        assert_eq!(
            text(&f.project),
            "# en-tête\n\
             [phpunit]\n\
             php_path = 'php.exe' # le mien\n\
             \n\
             # note sur le script\n\
             extra_args = ['--testdox']\n\
             \n\
             # commentée : gardée même vide\n\
             [terminal]\n\
             \n\
             [index]\n\
             # pourquoi j'exclus docs\n"
        );
        // Le résultat se relit, et la valeur héritée reprend la main.
        let settings = f.files.load().unwrap();
        assert_eq!(
            settings.origin(&["phpunit", "phpunit_path"]),
            Origin::Default
        );
        assert_eq!(settings.origin(&["phpunit", "php_path"]), Origin::Project);
    }

    #[test]
    fn a_malformed_file_is_never_written() {
        let f = fixture();
        fs::create_dir_all(f.project.parent().unwrap()).unwrap();
        let broken =
            "# mes réglages\r\n[vcs]\r\ngit_path = 'git'\r\n[phpunit\r\nphp_path = 'x'\r\n";
        fs::write(&f.project, broken).unwrap();

        let error = f
            .files
            .set(
                Scope::Project,
                &["phpunit", "php_path"],
                SettingValue::Path(PathBuf::from("php")),
            )
            .unwrap_err();
        match &error {
            Error::SettingsUnreadable { path, line, .. } => {
                assert_eq!(path, &f.project);
                assert_eq!(*line, Some(4));
            }
            other => panic!("erreur inattendue : {other:?}"),
        }
        assert!(error.to_string().contains("ligne 4"), "{error}");
        assert_eq!(text(&f.project), broken);
        assert!(f.files.unset(Scope::Project, &["vcs", "git_path"]).is_err());
        assert_eq!(text(&f.project), broken);

        // Pas de l'UTF-8 : refusé aussi.
        fs::write(&f.project, b"[vcs]\nnote = '\xE9'\n").unwrap();
        assert!(matches!(
            f.files.unset(Scope::Project, &["vcs", "note"]),
            Err(Error::SettingsUnreadable { line: None, .. })
        ));
        assert_eq!(fs::read(&f.project).unwrap(), b"[vcs]\nnote = '\xE9'\n");

        // La lecture tolérante signale le fichier et garde l'autre portée.
        fs::write(&f.project, broken).unwrap();
        fs::create_dir_all(f.user.parent().unwrap()).unwrap();
        fs::write(&f.user, "[app]\ntheme = 'clair'\n").unwrap();
        let (settings, problems) = f.files.load_lenient();
        assert_eq!(settings.origin(&["app", "theme"]), Origin::User);
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].scope, Scope::Project);
        assert_eq!(problems[0].line, Some(4));
        assert!(f.files.load().is_err());
    }

    #[test]
    fn a_read_only_file_is_refused_with_its_name() {
        let f = fixture();
        fs::create_dir_all(f.project.parent().unwrap()).unwrap();
        fs::write(&f.project, "[vcs]\ngit_path = 'git'\n").unwrap();
        let mut permissions = fs::metadata(&f.project).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&f.project, permissions.clone()).unwrap();

        let error = f.files.unset(Scope::Project, &["vcs", "git_path"]);
        assert!(matches!(&error, Err(Error::Io { path, .. }) if path == &f.project));
        assert_eq!(text(&f.project), "[vcs]\ngit_path = 'git'\n");

        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        fs::set_permissions(&f.project, permissions).unwrap();
    }

    #[test]
    fn every_kind_of_value_round_trips_through_the_file() {
        let f = fixture();
        let quoted = PathBuf::from(r"C:\l'outil\php.exe");
        let long_list: Vec<String> = (0..12).map(|i| format!("dossier-exclu-{i}")).collect();
        f.files
            .apply(
                Scope::Project,
                &[
                    SettingsEdit::set(
                        &["phpunit", "php_path"],
                        SettingValue::Path(PathBuf::from(
                            r"C:\Program Files (x86)\AgileBio\LabCollector\php\php.exe",
                        )),
                    ),
                    SettingsEdit::set(
                        &["phpunit", "extra_args"],
                        SettingValue::List(vec!["--group".into(), "très lent".into()]),
                    ),
                    SettingsEdit::set(
                        &["phpunit", "env", "APP_ENV"],
                        SettingValue::Text("test".into()),
                    ),
                    SettingsEdit::set(
                        &["phpunit", "env", "nom bizarre"],
                        SettingValue::Text("a \"b\" \\ c".into()),
                    ),
                    SettingsEdit::set(&["phpunit", "timeout_seconds"], SettingValue::Integer(30)),
                    SettingsEdit::set(&["vcs", "git_path"], SettingValue::Path(quoted.clone())),
                    SettingsEdit::set(
                        &["vcs", "update_method"],
                        SettingValue::Text("rebase".into()),
                    ),
                    SettingsEdit::set(
                        &["vcs", "protected_branches"],
                        SettingValue::List(vec!["main".into(), "release/*".into()]),
                    ),
                    SettingsEdit::set(
                        &["index", "excluded"],
                        SettingValue::List(long_list.clone()),
                    ),
                    SettingsEdit::set(&["text", "wrap"], SettingValue::Bool(true)),
                ],
            )
            .unwrap();

        let written = text(&f.project);
        assert!(
            written.contains(
                "php_path = 'C:\\Program Files (x86)\\AgileBio\\LabCollector\\php\\php.exe'\r\n"
            ),
            "{written}"
        );
        assert!(written.contains("[phpunit.env]\r\n"), "{written}");
        assert!(
            written.contains("update_method = \"rebase\"\r\n"),
            "{written}"
        );
        // Un long tableau s'écrit un élément par ligne.
        assert!(written.contains("excluded = [\r\n    \"dossier-exclu-0\",\r\n"));

        let settings = f.files.load().unwrap();
        let get = |path: &[&str]| settings.get(path).cloned().unwrap();
        assert_eq!(
            get(&["phpunit", "php_path"]).as_str(),
            Some(r"C:\Program Files (x86)\AgileBio\LabCollector\php\php.exe")
        );
        assert_eq!(
            get(&["phpunit", "extra_args"]),
            toml::Value::Array(vec!["--group".into(), "très lent".into()])
        );
        assert_eq!(get(&["phpunit", "env", "APP_ENV"]).as_str(), Some("test"));
        assert_eq!(
            get(&["phpunit", "env", "nom bizarre"]).as_str(),
            Some("a \"b\" \\ c")
        );
        assert_eq!(get(&["phpunit", "timeout_seconds"]).as_integer(), Some(30));
        assert_eq!(get(&["vcs", "git_path"]).as_str(), quoted.to_str());
        assert_eq!(
            get(&["index", "excluded"]).as_array().map(Vec::len),
            Some(long_list.len())
        );
        assert_eq!(get(&["text", "wrap"]).as_bool(), Some(true));

        // Une seconde passe identique ne change pas un octet.
        f.files
            .set(
                Scope::Project,
                &["index", "excluded"],
                SettingValue::List(long_list),
            )
            .unwrap();
        assert_eq!(text(&f.project), written);
    }

    #[test]
    fn a_new_table_goes_after_the_comments_that_ended_the_file() {
        let f = fixture();
        fs::create_dir_all(f.project.parent().unwrap()).unwrap();
        fs::write(
            &f.project,
            "[phpunit]\nphp_path = 'php'\n# dernière ligne de phpunit\n",
        )
        .unwrap();
        f.files
            .apply(
                Scope::Project,
                &[
                    SettingsEdit::set(&["vcs", "git_path"], SettingValue::Path("git".into())),
                    SettingsEdit::set(
                        &["phpunit", "env", "APP_ENV"],
                        SettingValue::Text("test".into()),
                    ),
                ],
            )
            .unwrap();
        assert_eq!(
            text(&f.project),
            "[phpunit]\nphp_path = 'php'\n\n[phpunit.env]\nAPP_ENV = \"test\"\n# dernière ligne de phpunit\n\n[vcs]\ngit_path = 'git'\n"
        );
    }

    #[test]
    fn invalid_requests_are_refused_before_any_write() {
        let f = fixture();
        assert!(matches!(
            f.files
                .set(Scope::Project, &["theme"], SettingValue::Bool(true)),
            Err(Error::Invalid(_))
        ));
        assert!(matches!(
            f.files
                .set(Scope::Project, &["app", ""], SettingValue::Bool(true)),
            Err(Error::Invalid(_))
        ));
        assert!(!f.project.exists());

        // Une clé parente qui n'est pas une table, ou une table là où l'on écrit
        // une valeur : refus, fichier intact.
        fs::create_dir_all(f.project.parent().unwrap()).unwrap();
        let before = "phpunit = 3\n[vcs.git_path]\nx = 1\n";
        fs::write(&f.project, before).unwrap();
        assert!(f
            .files
            .set(
                Scope::Project,
                &["phpunit", "php_path"],
                SettingValue::Bool(true)
            )
            .is_err());
        assert!(f
            .files
            .set(
                Scope::Project,
                &["vcs", "git_path"],
                SettingValue::Bool(true)
            )
            .is_err());
        assert_eq!(text(&f.project), before);

        // Sans projet ouvert, pas de portée projet.
        let user_only = SettingsFiles::new(Some(f.user.clone()), None);
        assert_eq!(user_only.path(Scope::Project), None);
        assert!(matches!(
            user_only.set(Scope::Project, &["app", "theme"], SettingValue::Bool(true)),
            Err(Error::Invalid(_))
        ));
    }

    #[test]
    fn ensure_file_creates_only_the_comment_line_and_keeps_an_existing_file() {
        let f = fixture();
        let path = f.files.ensure_file(Scope::User).unwrap();
        assert_eq!(path, f.user);
        assert_eq!(text(&f.user), format!("{USER_HEADER}\r\n"));
        assert_eq!(f.files.load().unwrap(), Settings::default());

        fs::write(&f.user, "[app]\ntheme = 'clair'\n").unwrap();
        f.files.ensure_file(Scope::User).unwrap();
        assert_eq!(text(&f.user), "[app]\ntheme = 'clair'\n");
    }

    #[test]
    fn keys_inside_an_inline_table_can_be_set_and_unset() {
        let f = fixture();
        fs::create_dir_all(f.project.parent().unwrap()).unwrap();
        fs::write(
            &f.project,
            "phpunit = { php_path = 'a', keep = 1 } # en ligne\n",
        )
        .unwrap();
        f.files
            .set(
                Scope::Project,
                &["phpunit", "php_path"],
                SettingValue::Path("b".into()),
            )
            .unwrap();
        assert_eq!(
            text(&f.project),
            "phpunit = { php_path = 'b', keep = 1 } # en ligne\n"
        );
        f.files
            .unset(Scope::Project, &["phpunit", "php_path"])
            .unwrap();
        let settings = f.files.load().unwrap();
        assert_eq!(settings.get(&["phpunit", "php_path"]), None);
        assert_eq!(
            settings
                .get(&["phpunit", "keep"])
                .and_then(|v| v.as_integer()),
            Some(1)
        );
    }
}
