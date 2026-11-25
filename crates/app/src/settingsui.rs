//! Fenêtre de réglages (Ctrl+Alt+S) : pages PHPUnit, Git, Terminal, Recherche,
//! Apparence ; portée « Ce projet » / « Tous mes projets », origine de chaque
//! valeur, retour à la valeur héritée, validation immédiate par sondes en tâche
//! de fond, commande PHPUnit affichée en direct.
//!
//! Ce module ne lit ni n'écrit aucun fichier : il garde un **brouillon** des deux
//! portées, rend des [`SettingsIntent`] (écrire, ouvrir le fichier, lancer une
//! sonde, parcourir) et l'application les exécute (`app/settings.rs`). Les règles
//! de lecture, d'écriture et de fusion sont celles de `julie-core` ; les règles
//! de détection et de commande sont celles de `julie-phpunit`.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui::{
    self, pos2, vec2, Align2, Color32, FontId, Frame, Id, Key, Margin, Modifiers, RichText, Sense,
    TextEdit, Ui,
};
use julie_core::settings::{Scope, SettingValue, SettingsEdit, Value};
use julie_core::{Settings, TaskId};
use julie_phpunit::{
    detect_files, join_arguments, preview_with, split_arguments, PhpUnitSettings, ProbeRequest,
    RunRequest,
};

use crate::kit::{self, Look};
use crate::texts as t;
use crate::theme::Palette;

/// Délai entre la dernière frappe et la sonde.
const DEBOUNCE: Duration = Duration::from_millis(300);

// ----- Description des champs ------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Page {
    Phpunit,
    Git,
    Terminal,
    Search,
    Appearance,
}

impl Page {
    pub const ALL: [Page; 5] = [
        Page::Phpunit,
        Page::Git,
        Page::Terminal,
        Page::Search,
        Page::Appearance,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Page::Phpunit => "phpunit",
            Page::Git => "git",
            Page::Terminal => "terminal",
            Page::Search => "search",
            Page::Appearance => "appearance",
        }
    }

    pub fn parse(text: &str) -> Option<Page> {
        Page::ALL.into_iter().find(|p| p.key() == text)
    }

    pub fn label(self) -> &'static str {
        match self {
            Page::Phpunit => t::SETTINGS_PAGE_PHPUNIT,
            Page::Git => t::SETTINGS_PAGE_GIT,
            Page::Terminal => t::SETTINGS_PAGE_TERMINAL,
            Page::Search => t::SETTINGS_PAGE_SEARCH,
            Page::Appearance => t::SETTINGS_PAGE_APPEARANCE,
        }
    }

    /// Portée proposée à l'ouverture.
    fn default_scope(self) -> Scope {
        match self {
            Page::Phpunit | Page::Search => Scope::Project,
            _ => Scope::User,
        }
    }

    /// La page n'existe qu'à la portée utilisateur.
    fn user_only(self) -> bool {
        self == Page::Appearance
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Path,
    Choice(&'static [&'static str]),
    Args,
    Lines,
    Env,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProbeKind {
    None,
    Php,
    PhpUnit,
    Config,
    Git,
    Exists,
}

struct Field {
    key: &'static str,
    page: Page,
    kind: Kind,
    probe: ProbeKind,
}

impl Field {
    fn path(&self) -> Vec<&'static str> {
        self.key.split('.').collect()
    }
}

const FIELDS: &[Field] = &[
    Field {
        key: "phpunit.php_path",
        page: Page::Phpunit,
        kind: Kind::Path,
        probe: ProbeKind::Php,
    },
    Field {
        key: "phpunit.phpunit_path",
        page: Page::Phpunit,
        kind: Kind::Path,
        probe: ProbeKind::PhpUnit,
    },
    Field {
        key: "phpunit.configuration",
        page: Page::Phpunit,
        kind: Kind::Path,
        probe: ProbeKind::Config,
    },
    Field {
        key: "phpunit.extra_args",
        page: Page::Phpunit,
        kind: Kind::Args,
        probe: ProbeKind::None,
    },
    Field {
        key: "phpunit.env",
        page: Page::Phpunit,
        kind: Kind::Env,
        probe: ProbeKind::None,
    },
    Field {
        key: "vcs.git_path",
        page: Page::Git,
        kind: Kind::Path,
        probe: ProbeKind::Git,
    },
    Field {
        key: "vcs.update_method",
        page: Page::Git,
        kind: Kind::Choice(&["fusion", "rebase"]),
        probe: ProbeKind::None,
    },
    Field {
        key: "vcs.protected_branches",
        page: Page::Git,
        kind: Kind::Lines,
        probe: ProbeKind::None,
    },
    Field {
        key: "terminal.default_profile",
        page: Page::Terminal,
        kind: Kind::Choice(&["git-bash", "cmd"]),
        probe: ProbeKind::None,
    },
    Field {
        key: "terminal.git_bash_path",
        page: Page::Terminal,
        kind: Kind::Path,
        probe: ProbeKind::Exists,
    },
    Field {
        key: "terminal.cmd_path",
        page: Page::Terminal,
        kind: Kind::Path,
        probe: ProbeKind::Exists,
    },
    Field {
        key: "index.excluded",
        page: Page::Search,
        kind: Kind::Lines,
        probe: ProbeKind::None,
    },
    Field {
        key: "index.libraries",
        page: Page::Search,
        kind: Kind::Lines,
        probe: ProbeKind::None,
    },
    Field {
        key: "app.theme",
        page: Page::Appearance,
        kind: Kind::Choice(&["sombre", "clair"]),
        probe: ProbeKind::None,
    },
];

fn field(key: &str) -> Option<&'static Field> {
    FIELDS.iter().find(|f| f.key == key)
}

// ----- Brouillon -------------------------------------------------------------------

/// Les valeurs des deux portées. Une clé absente de `vals` n'est pas réglée à
/// cette portée (elle hérite).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Draft {
    vals: HashMap<(Scope, &'static str), String>,
    env: HashMap<Scope, Vec<(String, String)>>,
}

fn value_text(kind: Kind, value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Array(items) => {
            let strings: Vec<String> = items
                .iter()
                .map(|v| match v {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                })
                .collect();
            Some(if kind == Kind::Args {
                join_arguments(&strings)
            } else {
                strings.join("\n")
            })
        }
        Value::Integer(n) => Some(n.to_string()),
        Value::Boolean(b) => Some(b.to_string()),
        _ => None,
    }
}

impl Draft {
    pub fn from_settings(settings: &Settings) -> Draft {
        let mut draft = Draft::default();
        for f in FIELDS {
            for scope in [Scope::User, Scope::Project] {
                if f.kind == Kind::Env {
                    if let Some(Value::Table(table)) = settings.get_in(scope, &f.path()) {
                        let rows: Vec<(String, String)> = table
                            .iter()
                            .map(|(k, v)| {
                                (
                                    k.clone(),
                                    match v {
                                        Value::String(s) => s.clone(),
                                        other => other.to_string(),
                                    },
                                )
                            })
                            .collect();
                        if !rows.is_empty() {
                            draft.env.insert(scope, rows);
                        }
                    }
                } else if let Some(value) = settings.get_in(scope, &f.path()) {
                    if let Some(text) = value_text(f.kind, value) {
                        draft.vals.insert((scope, f.key), text);
                    }
                }
            }
        }
        draft
    }

    fn get(&self, scope: Scope, key: &'static str) -> Option<&String> {
        self.vals.get(&(scope, key))
    }

    /// Valeur effective : projet, puis utilisateur.
    fn eff(&self, key: &'static str) -> Option<&String> {
        self.get(Scope::Project, key)
            .or_else(|| self.get(Scope::User, key))
    }

    fn set(&mut self, scope: Scope, key: &'static str, text: String) {
        if text.is_empty() {
            self.vals.remove(&(scope, key));
        } else {
            self.vals.insert((scope, key), text);
        }
    }

    fn env_rows(&self, scope: Scope) -> &[(String, String)] {
        self.env.get(&scope).map_or(&[], Vec::as_slice)
    }
}

/// Les lignes d'un champ « liste » : sans espaces de bord, sans lignes vides.
fn list_items(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect()
}

fn quotes_unbalanced(text: &str) -> bool {
    let mut open = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if open && chars.peek() == Some(&'"') => {
                chars.next();
            }
            '"' => open = !open,
            _ => {}
        }
    }
    open
}

fn is_bare_name(text: &str) -> bool {
    !text.contains(['\\', '/', ':'])
}

// ----- Sondes ----------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeState {
    Idle,
    Running,
    /// 0 : réussi, 1 : avertissement.
    Done(u8, String),
}

#[derive(Debug, Clone)]
pub enum ProbeAction {
    Php(ProbeRequest),
    Git(PathBuf),
    Exists(PathBuf),
}

#[derive(Debug)]
pub struct ProbeSlot {
    pub task: Option<TaskId>,
    pub token: u64,
    pub state: ProbeState,
    wanted: Option<(String, Instant)>,
    probed: Option<String>,
}

impl ProbeSlot {
    fn new() -> Self {
        Self {
            task: None,
            token: 0,
            state: ProbeState::Idle,
            wanted: None,
            probed: None,
        }
    }
}

// ----- Intentions ------------------------------------------------------------------

#[derive(Debug)]
pub enum SettingsIntent {
    /// Écrire ces modifications, une écriture par portée ; `close` : fermer ensuite.
    Apply {
        close: bool,
        edits: Vec<(Scope, Vec<SettingsEdit>)>,
    },
    Close,
    OpenFile(Scope),
    Probe {
        field: &'static str,
        token: u64,
        action: ProbeAction,
    },
    Browse {
        field: &'static str,
        scope: Scope,
    },
}

/// Ce que l'application fournit à la fenêtre à chaque image.
pub struct Env<'a> {
    pub root: Option<&'a Path>,
    pub user_file: Option<&'a Path>,
    pub project_file: Option<&'a Path>,
}

// ----- Le dialogue -----------------------------------------------------------------

struct Auto {
    php: Option<PathBuf>,
    phpunit: Option<PathBuf>,
    config: Option<PathBuf>,
}

pub struct SettingsDialog {
    pub page: Page,
    pub filter: String,
    scopes: HashMap<Page, Scope>,
    cur: Draft,
    orig: Draft,
    has_project: bool,
    has_user: bool,
    pub probes: HashMap<&'static str, ProbeSlot>,
    preview_for: Option<PhpUnitSettings>,
    /// La commande, ou la raison pour laquelle il n'y en a pas.
    pub command: Option<Result<String, String>>,
    warnings: Vec<(&'static str, String)>,
    auto: Option<Auto>,
    pub message: Option<(bool, String)>,
    pub applying: bool,
    next_token: u64,
    focus_filter: bool,
}

impl SettingsDialog {
    pub fn new(settings: &Settings, page: Option<Page>, has_project: bool, has_user: bool) -> Self {
        let draft = Draft::from_settings(settings);
        let mut probes = HashMap::new();
        for f in FIELDS.iter().filter(|f| f.probe != ProbeKind::None) {
            probes.insert(f.key, ProbeSlot::new());
        }
        Self {
            page: page.unwrap_or(Page::Phpunit),
            filter: String::new(),
            scopes: HashMap::new(),
            cur: draft.clone(),
            orig: draft,
            has_project,
            has_user,
            probes,
            preview_for: None,
            command: None,
            warnings: Vec::new(),
            auto: None,
            message: None,
            applying: false,
            next_token: 1,
            focus_filter: false,
        }
    }

    pub fn scope(&self) -> Scope {
        self.scope_of(self.page)
    }

    fn scope_of(&self, page: Page) -> Scope {
        if page.user_only() {
            return Scope::User;
        }
        let wanted = self
            .scopes
            .get(&page)
            .copied()
            .unwrap_or_else(|| page.default_scope());
        match wanted {
            Scope::Project if !self.has_project => Scope::User,
            Scope::User if !self.has_user && self.has_project => Scope::Project,
            other => other,
        }
    }

    pub fn set_scope(&mut self, scope: Scope) -> Result<(), String> {
        if scope == Scope::Project && !self.has_project {
            return Err(t::SETTINGS_NO_PROJECT.to_owned());
        }
        if scope == Scope::User && !self.has_user {
            return Err(t::SETTINGS_NO_USER_FILE.to_owned());
        }
        if self.page.user_only() && scope == Scope::Project {
            return Err(t::SETTINGS_SCOPE_USER_ONLY.to_owned());
        }
        self.scopes.insert(self.page, scope);
        Ok(())
    }

    pub fn is_modified(&self) -> bool {
        self.cur != self.orig
    }

    fn field_modified(&self, f: &Field, scope: Scope) -> bool {
        if f.kind == Kind::Env {
            self.cur.env_rows(scope) != self.orig.env_rows(scope)
        } else {
            self.cur.get(scope, f.key) != self.orig.get(scope, f.key)
        }
    }

    fn page_modified(&self, page: Page) -> bool {
        FIELDS
            .iter()
            .filter(|f| f.page == page)
            .any(|f| self.field_modified(f, Scope::User) || self.field_modified(f, Scope::Project))
    }

    // ----- Édition (aussi utilisée par le mode scénario) ---------------------------

    /// Règle un champ texte, une liste (lignes séparées par `\n`) ou un choix.
    pub fn set_text(&mut self, key: &str, scope: Scope, text: &str) -> Result<(), String> {
        let f = field(key).ok_or_else(|| format!("réglage inconnu : « {key} »"))?;
        if f.kind == Kind::Env {
            return Err("variables : `settings.env <NOM>=<valeur>`".to_owned());
        }
        if f.page.user_only() && scope == Scope::Project {
            return Err(t::SETTINGS_SCOPE_USER_ONLY.to_owned());
        }
        if let Kind::Choice(values) = f.kind {
            if !text.is_empty() && !values.contains(&text) {
                return Err(format!("valeurs possibles : {}", values.join(", ")));
            }
        }
        self.cur.set(scope, f.key, text.to_owned());
        Ok(())
    }

    pub fn unset(&mut self, key: &str, scope: Scope) -> Result<(), String> {
        let f = field(key).ok_or_else(|| format!("réglage inconnu : « {key} »"))?;
        if f.kind == Kind::Env {
            self.cur.env.remove(&scope);
        } else {
            self.cur.vals.remove(&(scope, f.key));
        }
        Ok(())
    }

    pub fn unset_env(&mut self, scope: Scope, name: &str) {
        if let Some(rows) = self.cur.env.get_mut(&scope) {
            rows.retain(|(n, _)| n != name);
            if rows.is_empty() {
                self.cur.env.remove(&scope);
            }
        }
    }

    /// Plus de sonde en cours ni en attente de frappe (mode scénario).
    pub fn settled(&self) -> bool {
        !self.applying
            && self.probes.values().all(|s| {
                s.state != ProbeState::Running
                    && s.wanted.as_ref().map(|(w, _)| w.as_str())
                        == s.probed
                            .as_deref()
                            .or(s.wanted.as_ref().map(|(w, _)| w.as_str()))
                    && (s.wanted.is_some() || s.probed.is_none())
            })
    }

    pub fn set_env(&mut self, scope: Scope, name: &str, value: &str) {
        let rows = self.cur.env.entry(scope).or_default();
        match rows.iter_mut().find(|(n, _)| n == name) {
            Some(row) => row.1 = value.to_owned(),
            None => rows.push((name.to_owned(), value.to_owned())),
        }
    }

    /// Le chemin choisi par « Parcourir… » : relatif à la racine s'il est dedans
    /// (portée projet).
    pub fn browsed(&mut self, key: &str, scope: Scope, path: &Path, root: Option<&Path>) {
        let Some(f) = field(key) else { return };
        let text = match (scope, root.and_then(|r| path.strip_prefix(r).ok())) {
            (Scope::Project, Some(relative)) => relative.display().to_string(),
            _ => path.display().to_string(),
        };
        self.cur.set(scope, f.key, text);
    }

    // ----- Effectif, aperçu --------------------------------------------------------

    fn eff_phpunit(&self) -> PhpUnitSettings {
        let path = |key: &'static str| self.cur.eff(key).map(PathBuf::from);
        let mut env = BTreeMap::new();
        for scope in [Scope::User, Scope::Project] {
            for (name, value) in self.cur.env_rows(scope) {
                env.insert(name.clone(), value.clone());
            }
        }
        PhpUnitSettings {
            php_path: path("phpunit.php_path"),
            phpunit_path: path("phpunit.phpunit_path"),
            configuration: path("phpunit.configuration"),
            extra_args: self
                .cur
                .eff("phpunit.extra_args")
                .map(|line| split_arguments(line))
                .unwrap_or_default(),
            env,
            timeout_seconds: None,
        }
    }

    /// Recalcule la commande et les détections quand les valeurs ont changé.
    fn refresh_preview(&mut self, root: Option<&Path>) {
        let settings = self.eff_phpunit();
        if self.preview_for.as_ref() == Some(&settings) {
            return;
        }
        let Some(root) = root else {
            self.command = None;
            self.preview_for = Some(settings);
            return;
        };
        // Commande telle qu'elle partirait.
        let detection = detect_files(root, &settings);
        self.command = Some(
            preview_with(&detection, &settings, &RunRequest::all())
                .map(|preview| preview.command_line)
                .map_err(|problem| t::problem_text(&problem)),
        );
        // Ce que la détection trouverait sans réglage (valeur héritée affichée).
        let auto = detect_files(root, &PhpUnitSettings::default());
        self.auto = Some(Auto {
            php: auto.php.ok().map(|i| i.path),
            phpunit: auto.phpunit.ok().map(|i| i.path),
            config: auto.configuration.ok(),
        });
        self.warnings = settings
            .issues()
            .into_iter()
            .map(|issue| match issue {
                julie_phpunit::SettingsIssue::ReservedArgument(arg) => {
                    ("phpunit.extra_args", t::settings_reserved_arg(&arg))
                }
                julie_phpunit::SettingsIssue::EmptyArgument => {
                    ("phpunit.extra_args", t::SETTINGS_EMPTY_ARG.to_owned())
                }
                julie_phpunit::SettingsIssue::InvalidEnvName(_) => {
                    ("phpunit.env", t::SETTINGS_ERR_ENV_NAME.to_owned())
                }
            })
            .collect();
        self.preview_for = Some(settings);
    }

    fn rel(root: Option<&Path>, path: &Path) -> String {
        root.and_then(|r| path.strip_prefix(r).ok())
            .unwrap_or(path)
            .display()
            .to_string()
    }

    /// Ce qui serait vérifié pour ce champ.
    fn probe_action(&self, f: &Field, root: Option<&Path>) -> Option<ProbeAction> {
        let typed = |key: &'static str| self.cur.eff(key).map(PathBuf::from);
        let auto = self.auto.as_ref();
        match f.probe {
            ProbeKind::None => None,
            ProbeKind::Php => typed("phpunit.php_path")
                .or_else(|| auto.and_then(|a| a.php.clone()))
                .map(|p| ProbeAction::Php(ProbeRequest::Php(p))),
            ProbeKind::PhpUnit => {
                let script = typed("phpunit.phpunit_path")
                    .or_else(|| auto.and_then(|a| a.phpunit.clone()))?;
                let php = typed("phpunit.php_path").or_else(|| auto.and_then(|a| a.php.clone()));
                Some(ProbeAction::Php(ProbeRequest::PhpUnit { php, script }))
            }
            ProbeKind::Config => typed("phpunit.configuration")
                .or_else(|| auto.and_then(|a| a.config.clone()))
                .map(|p| ProbeAction::Php(ProbeRequest::Configuration(p))),
            ProbeKind::Git => Some(ProbeAction::Git(
                typed("vcs.git_path").unwrap_or_else(|| PathBuf::from("git")),
            )),
            ProbeKind::Exists => typed(f.key).map(|p| {
                let full = match root {
                    Some(root) if p.is_relative() => root.join(p),
                    _ => p,
                };
                ProbeAction::Exists(full)
            }),
        }
    }

    /// À appeler à chaque image : aperçu, puis sondes à lancer (après 300 ms de calme).
    fn tick(&mut self, root: Option<&Path>, ctx: &egui::Context) -> Vec<SettingsIntent> {
        self.refresh_preview(root);
        let now = Instant::now();
        let mut intents = Vec::new();
        for f in FIELDS.iter().filter(|f| f.probe != ProbeKind::None) {
            let action = self.probe_action(f, root);
            let target = action
                .as_ref()
                .map_or_else(String::new, |a| format!("{a:?}"));
            let first = self.probes.get(f.key).is_none_or(|s| s.probed.is_none());
            let Some(slot) = self.probes.get_mut(f.key) else {
                continue;
            };
            match &slot.wanted {
                Some((w, _)) if *w == target => {}
                _ => {
                    // Première mesure : tout de suite ; ensuite, après la frappe.
                    let at = if first { now - DEBOUNCE } else { now };
                    slot.wanted = Some((target.clone(), at));
                }
            }
            let Some((wanted, at)) = slot.wanted.clone() else {
                continue;
            };
            if slot.probed.as_deref() == Some(wanted.as_str()) {
                continue;
            }
            if now.duration_since(at) < DEBOUNCE {
                ctx.request_repaint_after(DEBOUNCE);
                continue;
            }
            slot.probed = Some(wanted);
            match action {
                Some(action) => {
                    self.next_token += 1;
                    slot.token = self.next_token;
                    slot.state = ProbeState::Running;
                    intents.push(SettingsIntent::Probe {
                        field: f.key,
                        token: slot.token,
                        action,
                    });
                }
                None => {
                    slot.token = 0;
                    slot.state = ProbeState::Idle;
                }
            }
        }
        intents
    }

    // ----- Écriture ----------------------------------------------------------------

    /// Ce qui bloque l'enregistrement, ou rien.
    pub fn blocking_reason(&self) -> Option<String> {
        for scope in [Scope::User, Scope::Project] {
            for f in FIELDS {
                if let Some(error) = self.error_of(f, scope) {
                    return Some(format!("{} : {error}", t::settings_label(f.key)));
                }
            }
        }
        None
    }

    fn error_of(&self, f: &Field, scope: Scope) -> Option<String> {
        match f.kind {
            Kind::Args => {
                let line = self.cur.get(scope, f.key)?;
                quotes_unbalanced(line).then(|| t::SETTINGS_ERR_QUOTE.to_owned())
            }
            Kind::Path => {
                let text = self.cur.get(scope, f.key)?;
                let path = Path::new(text);
                (scope == Scope::User && path.is_relative() && !is_bare_name(text))
                    .then(|| t::SETTINGS_ERR_RELATIVE_USER.to_owned())
            }
            Kind::Env => {
                let rows = self.cur.env_rows(scope);
                for (i, (name, _)) in rows.iter().enumerate() {
                    if name.trim().is_empty() || name.contains(['=', '\0']) {
                        return Some(t::SETTINGS_ERR_ENV_NAME.to_owned());
                    }
                    if rows[..i].iter().any(|(n, _)| n == name) {
                        return Some(t::SETTINGS_ERR_ENV_DUP.to_owned());
                    }
                }
                None
            }
            _ => None,
        }
    }

    /// Les modifications à écrire, par portée.
    pub fn build_edits(&self) -> Result<Vec<(Scope, Vec<SettingsEdit>)>, String> {
        if let Some(reason) = self.blocking_reason() {
            return Err(reason);
        }
        let mut out = Vec::new();
        for scope in [Scope::User, Scope::Project] {
            let mut edits = Vec::new();
            for f in FIELDS {
                let path = f.path();
                if f.kind == Kind::Env {
                    let now = self.cur.env_rows(scope);
                    let before = self.orig.env_rows(scope);
                    for (name, value) in now {
                        if before.iter().find(|(n, _)| n == name).map(|(_, v)| v) != Some(value) {
                            let mut p = path.clone();
                            p.push(name.as_str());
                            edits.push(SettingsEdit::set(&p, SettingValue::Text(value.clone())));
                        }
                    }
                    for (name, _) in before {
                        if !now.iter().any(|(n, _)| n == name) {
                            let mut p = path.clone();
                            p.push(name.as_str());
                            edits.push(SettingsEdit::unset(&p));
                        }
                    }
                    continue;
                }
                let (now, before) = (self.cur.get(scope, f.key), self.orig.get(scope, f.key));
                if now == before {
                    continue;
                }
                match now {
                    None => edits.push(SettingsEdit::unset(&path)),
                    Some(text) => {
                        let value = match f.kind {
                            Kind::Path => SettingValue::Path(PathBuf::from(text)),
                            Kind::Args => SettingValue::List(split_arguments(text)),
                            Kind::Lines => SettingValue::List(list_items(text)),
                            _ => SettingValue::Text(text.clone()),
                        };
                        edits.push(SettingsEdit::set(&path, value));
                    }
                }
            }
            if !edits.is_empty() {
                out.push((scope, edits));
            }
        }
        Ok(out)
    }

    /// Les écritures ont réussi : le brouillon devient la référence.
    pub fn committed(&mut self) {
        self.orig = self.cur.clone();
        self.applying = false;
        self.message = None;
    }

    pub fn failed(&mut self, text: String) {
        self.applying = false;
        self.message = Some((true, format!("{}{text}", t::SETTINGS_WRITE_FAILED_PREFIX)));
    }

    pub fn request_apply(&mut self, close: bool) -> Vec<SettingsIntent> {
        match self.build_edits() {
            Err(reason) => {
                self.message = Some((true, format!("{}{reason}", t::SETTINGS_BLOCKED_PREFIX)));
                Vec::new()
            }
            Ok(edits) if edits.is_empty() => {
                if close {
                    vec![SettingsIntent::Close]
                } else {
                    Vec::new()
                }
            }
            Ok(edits) => {
                self.applying = true;
                self.message = None;
                vec![SettingsIntent::Apply { close, edits }]
            }
        }
    }

    // ----- Résultat d'une sonde ----------------------------------------------------

    pub fn probe_result(&mut self, field: &'static str, token: u64, state: ProbeState) {
        if let Some(slot) = self.probes.get_mut(field) {
            if slot.token == token {
                slot.state = state;
            }
        }
    }

    pub fn probe_by_task(&mut self, task: TaskId, state: ProbeState) {
        if let Some(slot) = self.probes.values_mut().find(|s| s.task == Some(task)) {
            slot.state = state;
        }
    }

    // ----- État pour le mode scénario ----------------------------------------------

    pub fn state(&self, env: &Env) -> serde_json::Value {
        let scope_name = |s: Scope| match s {
            Scope::User => "user",
            Scope::Project => "project",
        };
        let fields: Vec<serde_json::Value> = FIELDS
            .iter()
            .map(|f| {
                let probe = self.probes.get(f.key).map(|s| match &s.state {
                    ProbeState::Idle => "idle".to_owned(),
                    ProbeState::Running => "running".to_owned(),
                    ProbeState::Done(0, text) => format!("ok: {text}"),
                    ProbeState::Done(_, text) => format!("warn: {text}"),
                });
                serde_json::json!({
                    "key": f.key,
                    "page": f.page.key(),
                    "user": self.cur.get(Scope::User, f.key),
                    "project": self.cur.get(Scope::Project, f.key),
                    "env_user": (f.kind == Kind::Env).then(|| self.cur.env_rows(Scope::User)),
                    "env_project": (f.kind == Kind::Env).then(|| self.cur.env_rows(Scope::Project)),
                    "modified": self.field_modified(f, Scope::User) || self.field_modified(f, Scope::Project),
                    "probe": probe,
                    "origin": self.origin_text(f, self.scope_of(f.page), env.root),
                })
            })
            .collect();
        serde_json::json!({
            "page": self.page.key(),
            "scope": scope_name(self.scope()),
            "filter": self.filter,
            "modified": self.is_modified(),
            "pages_modified": Page::ALL.iter().filter(|p| self.page_modified(**p)).map(|p| p.key()).collect::<Vec<_>>(),
            "blocking": self.blocking_reason(),
            "command": self.command.as_ref().map(|c| match c { Ok(s) | Err(s) => s.clone() }),
            "command_ok": self.command.as_ref().map(Result::is_ok),
            "warnings": self.warnings.iter().map(|(k, w)| format!("{k}: {w}")).collect::<Vec<_>>(),
            "message": self.message.as_ref().map(|(e, m)| format!("{}{m}", if *e { "erreur : " } else { "" })),
            "applying": self.applying,
            "user_file": env.user_file.map(|p| p.display().to_string()),
            "project_file": env.project_file.map(|p| p.display().to_string()),
            "fields": fields,
        })
    }

    // ----- Textes d'origine --------------------------------------------------------

    fn default_text(&self, f: &Field, root: Option<&Path>) -> String {
        let rel = |p: &Option<PathBuf>| p.as_deref().map(|p| Self::rel(root, p));
        match f.key {
            "phpunit.php_path" => self
                .auto
                .as_ref()
                .and_then(|a| rel(&a.php))
                .unwrap_or_else(|| "php".to_owned()),
            "phpunit.phpunit_path" => self
                .auto
                .as_ref()
                .and_then(|a| rel(&a.phpunit))
                .unwrap_or_default(),
            "phpunit.configuration" => self
                .auto
                .as_ref()
                .and_then(|a| rel(&a.config))
                .unwrap_or_default(),
            "vcs.git_path" => "git".to_owned(),
            "vcs.protected_branches" => "main\nmaster".to_owned(),
            "terminal.git_bash_path" => r"C:\Program Files\Git\bin\bash.exe".to_owned(),
            "index.excluded" => "/target".to_owned(),
            "index.libraries" => "vendor\nnode_modules".to_owned(),
            _ => String::new(),
        }
    }

    fn origin_text(&self, f: &Field, scope: Scope, root: Option<&Path>) -> String {
        let here = if f.kind == Kind::Env {
            !self.cur.env_rows(scope).is_empty()
        } else {
            self.cur.get(scope, f.key).is_some()
        };
        if here {
            return match scope {
                Scope::Project => t::SETTINGS_ORIGIN_PROJECT,
                Scope::User => t::SETTINGS_ORIGIN_USER,
            }
            .to_owned();
        }
        if scope == Scope::Project && self.cur.get(Scope::User, f.key).is_some() {
            return t::SETTINGS_ORIGIN_USER_FROM_PROJECT.to_owned();
        }
        match f.probe {
            ProbeKind::Php | ProbeKind::PhpUnit | ProbeKind::Config => {
                let detected = self.default_text(f, root);
                if self.auto.is_none() || detected.is_empty() {
                    t::SETTINGS_NOT_DETECTED.to_owned()
                } else if f.key == "phpunit.php_path" && detected == "php" {
                    t::settings_default_path_in_path("php")
                } else {
                    t::settings_detected(&detected)
                }
            }
            ProbeKind::Git => t::settings_default_path_in_path("git"),
            _ => t::SETTINGS_ORIGIN_DEFAULT.to_owned(),
        }
    }

    // ----- Dessin ------------------------------------------------------------------

    fn matches_filter(&self, f: &Field) -> bool {
        let q = self.filter.trim().to_lowercase();
        q.is_empty() || t::settings_label(f.key).to_lowercase().contains(&q)
    }

    fn page_visible(&self, page: Page) -> bool {
        let q = self.filter.trim().to_lowercase();
        q.is_empty()
            || page.label().to_lowercase().contains(&q)
            || FIELDS
                .iter()
                .any(|f| f.page == page && self.matches_filter(f))
    }

    pub fn focus_filter(&mut self) {
        self.focus_filter = true;
    }

    /// Dessine la fenêtre ; rend ce que l'application doit faire.
    pub fn show(&mut self, ctx: &egui::Context, p: &Palette, env: &Env) -> Vec<SettingsIntent> {
        let mut intents = self.tick(env.root, ctx);
        let screen = ctx.content_rect();
        let size = vec2(
            (screen.width() - 60.0).clamp(560.0, 940.0),
            (screen.height() - 50.0).clamp(380.0, 600.0),
        );
        let mut escape = ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
        let frame = Frame::new()
            .fill(p.bg)
            .stroke(egui::Stroke::new(1.0, p.border))
            .corner_radius(10)
            .shadow(egui::Shadow {
                offset: [0, 10],
                blur: 36,
                spread: 2,
                color: Color32::from_black_alpha(130),
            });
        let mut enter_ok = false;
        kit::modal("reglages").frame(frame).show(ctx, |ui| {
            let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
            let bar = egui::Rect::from_min_size(rect.min, vec2(rect.width(), 38.0));
            ui.painter().rect_filled(
                bar,
                egui::CornerRadius {
                    nw: 10,
                    ne: 10,
                    sw: 0,
                    se: 0,
                },
                p.bg_alt,
            );
            ui.painter().text(
                pos2(rect.left() + 16.0, bar.center().y),
                Align2::LEFT_CENTER,
                t::SETTINGS_TITLE,
                FontId::proportional(15.0),
                p.text,
            );
            let foot_h = 52.0;
            let foot =
                egui::Rect::from_min_max(pos2(rect.left(), rect.bottom() - foot_h), rect.max);
            let left = egui::Rect::from_min_max(
                pos2(rect.left(), bar.bottom()),
                pos2(rect.left() + 184.0, foot.top()),
            );
            let right = egui::Rect::from_min_max(
                pos2(left.right() + 1.0, bar.bottom()),
                pos2(rect.right(), foot.top()),
            );
            ui.painter().vline(
                left.right(),
                left.y_range(),
                egui::Stroke::new(1.0, p.border),
            );
            ui.painter()
                .hline(rect.x_range(), foot.top(), egui::Stroke::new(1.0, p.border));
            self.show_left(ui, left, p);
            enter_ok = self.show_right(ui, right, p, env, &mut intents);
            self.show_foot(ui, foot, p, env, &mut intents);
        });
        if escape {
            // Un champ en cours d'édition garde Échap pour lui seulement s'il l'a
            // consommé ; ici Échap = Annuler.
            escape = true;
        }
        if escape {
            intents.push(SettingsIntent::Close);
        } else if enter_ok {
            intents.extend(self.request_apply(true));
        }
        intents
    }

    fn show_left(&mut self, ui: &mut Ui, rect: egui::Rect, p: &Palette) {
        kit::child(ui, rect.shrink2(vec2(8.0, 8.0)), |ui| {
            ui.set_width(rect.width() - 16.0);
            let edit = ui.add(
                TextEdit::singleline(&mut self.filter)
                    .hint_text(t::SETTINGS_FILTER_HINT)
                    .desired_width(f32::INFINITY),
            );
            if self.focus_filter {
                self.focus_filter = false;
                edit.request_focus();
            }
            ui.add_space(6.0);
            let mut any = false;
            for page in Page::ALL {
                if !self.page_visible(page) {
                    continue;
                }
                any = true;
                let selected = self.page == page;
                let (row, response) =
                    ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
                if selected {
                    ui.painter().rect_filled(row, 5.0, p.accent_soft);
                } else if response.hovered() {
                    ui.painter().rect_filled(row, 5.0, p.bg_alt);
                }
                ui.painter().text(
                    pos2(row.left() + 10.0, row.center().y),
                    Align2::LEFT_CENTER,
                    page.label(),
                    FontId::proportional(14.0),
                    p.text,
                );
                if self.page_modified(page) {
                    ui.painter().circle_filled(
                        pos2(row.right() - 12.0, row.center().y),
                        3.5,
                        p.warn,
                    );
                }
                if response.clicked() {
                    self.page = page;
                }
            }
            if !any {
                ui.label(RichText::new(t::SETTINGS_NO_MATCH).color(p.text_dim));
            } else if !self.page_visible(self.page) {
                if let Some(first) = Page::ALL.into_iter().find(|pg| self.page_visible(*pg)) {
                    self.page = first;
                }
            }
        });
    }

    /// Rend `true` si Entrée a été pressée dans un champ d'une ligne (= OK).
    fn show_right(
        &mut self,
        ui: &mut Ui,
        rect: egui::Rect,
        p: &Palette,
        env: &Env,
        intents: &mut Vec<SettingsIntent>,
    ) -> bool {
        let mut enter_ok = false;
        let page = self.page;
        let scope = self.scope();
        kit::child(ui, rect.shrink2(vec2(16.0, 10.0)), |ui| {
            ui.set_width(rect.width() - 32.0);
            // En-tête : titre de la page et portée.
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(page.label())
                        .size(17.0)
                        .strong()
                        .color(p.text),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if page.user_only() {
                        ui.label(RichText::new(t::SETTINGS_SCOPE_USER_ONLY).color(p.text_dim));
                    } else {
                        let mut chosen = None;
                        for (s, label) in [
                            (Scope::User, t::SETTINGS_SCOPE_USER),
                            (Scope::Project, t::SETTINGS_SCOPE_PROJECT),
                        ] {
                            let enabled = match s {
                                Scope::Project => self.has_project,
                                Scope::User => self.has_user,
                            };
                            if ui
                                .add_enabled(enabled, egui::Button::selectable(scope == s, label))
                                .clicked()
                            {
                                chosen = Some(s);
                            }
                        }
                        ui.label(RichText::new(t::SETTINGS_SCOPE_LABEL).color(p.text_dim));
                        if let Some(s) = chosen {
                            self.scopes.insert(page, s);
                        }
                    }
                });
            });
            if !self.has_project && !page.user_only() {
                ui.label(
                    RichText::new(t::SETTINGS_NO_PROJECT)
                        .small()
                        .color(p.text_dim),
                );
            }
            if !self.has_user {
                ui.label(
                    RichText::new(t::SETTINGS_NO_USER_FILE)
                        .small()
                        .color(p.warn),
                );
            }
            ui.add_space(4.0);
            ui.separator();
            // La commande PHPUnit reste visible sous la liste des champs.
            let full = ui.max_rect();
            let top = ui.cursor().top();
            let command_height = if page == Page::Phpunit { 104.0 } else { 0.0 };
            let scroll_rect = egui::Rect::from_min_max(
                pos2(full.left(), top),
                pos2(full.right(), full.bottom() - command_height),
            );
            kit::child(ui, scroll_rect, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt(("reglages-page", page.key()))
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.set_width(scroll_rect.width() - 14.0);
                        for f in FIELDS.iter().filter(|f| f.page == page) {
                            if self.show_field(ui, p, f, scope, env, intents) {
                                enter_ok = true;
                            }
                            ui.add_space(8.0);
                        }
                        if page == Page::Search {
                            ui.label(
                                RichText::new(t::SETTINGS_EXCLUDED_ALWAYS)
                                    .small()
                                    .color(p.text_dim),
                            );
                        }
                    });
            });
            if page == Page::Phpunit {
                let command_rect = egui::Rect::from_min_max(
                    pos2(full.left(), full.bottom() - command_height + 4.0),
                    full.max,
                );
                kit::child(ui, command_rect, |ui| self.show_command(ui, p));
            }
        });
        enter_ok
    }

    fn show_command(&self, ui: &mut Ui, p: &Palette) {
        ui.add_space(4.0);
        ui.label(
            RichText::new(t::SETTINGS_COMMAND_TITLE)
                .strong()
                .color(p.text),
        );
        let (text, color) = match &self.command {
            Some(Ok(line)) => (line.clone(), p.text),
            Some(Err(problem)) => (problem.clone(), p.warn),
            None => (t::SETTINGS_NO_PROJECT.to_owned(), p.text_dim),
        };
        Frame::new()
            .fill(p.bg_alt)
            .stroke(egui::Stroke::new(1.0, p.border))
            .corner_radius(6)
            .inner_margin(Margin::same(8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.add(
                    egui::Label::new(RichText::new(text).monospace().color(color))
                        .wrap()
                        .selectable(true),
                );
            });
        for (_, warning) in self
            .warnings
            .iter()
            .filter(|(k, _)| *k == "phpunit.extra_args")
        {
            ui.label(RichText::new(warning).small().color(p.warn));
        }
    }

    fn show_field(
        &mut self,
        ui: &mut Ui,
        p: &Palette,
        f: &'static Field,
        scope: Scope,
        env: &Env,
        intents: &mut Vec<SettingsIntent>,
    ) -> bool {
        let mut enter = false;
        let hit = !self.filter.trim().is_empty() && self.matches_filter(f);
        let set_here = if f.kind == Kind::Env {
            !self.cur.env_rows(scope).is_empty()
        } else {
            self.cur.get(scope, f.key).is_some()
        };
        let writable = match scope {
            Scope::User => self.has_user,
            Scope::Project => self.has_project,
        };
        ui.add_enabled_ui(writable, |ui| {
            // Libellé, marque « modifié », retour à l'héritage.
            ui.horizontal(|ui| {
                let label = RichText::new(t::settings_label(f.key)).strong();
                if hit {
                    ui.label(label.color(p.accent).background_color(p.accent_soft));
                } else {
                    ui.label(label.color(p.text));
                }
                if self.field_modified(f, scope) {
                    ui.label(
                        RichText::new(format!("• {}", t::SETTINGS_MODIFIED_MARK))
                            .small()
                            .color(p.warn),
                    );
                }
                if set_here && !matches!(f.kind, Kind::Choice(_)) {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button(t::SETTINGS_REVERT).clicked() {
                            let _ = self.unset(f.key, scope);
                        }
                    });
                }
            });
            let hint = self.inherited_hint(f, scope, env.root);
            match f.kind {
                Kind::Path => {
                    ui.horizontal(|ui| {
                        let mut text = self.cur.get(scope, f.key).cloned().unwrap_or_default();
                        let edit = ui.add(
                            TextEdit::singleline(&mut text)
                                .hint_text(hint)
                                .id_salt(("reglage", f.key, scope == Scope::User))
                                .desired_width((ui.available_width() - 100.0).max(120.0)),
                        );
                        if edit.changed() {
                            self.cur.set(scope, f.key, text);
                        }
                        if edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                            enter = true;
                        }
                        if ui.button(t::SETTINGS_BROWSE).clicked() {
                            intents.push(SettingsIntent::Browse {
                                field: f.key,
                                scope,
                            });
                        }
                    });
                }
                Kind::Args => {
                    let mut text = self.cur.get(scope, f.key).cloned().unwrap_or_default();
                    let edit = ui.add(
                        TextEdit::singleline(&mut text)
                            .hint_text(hint)
                            .id_salt(("reglage", f.key, scope == Scope::User))
                            .desired_width(f32::INFINITY),
                    );
                    if edit.changed() {
                        self.cur.set(scope, f.key, text.clone());
                    }
                    if edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                        enter = true;
                    }
                    let args = split_arguments(&text);
                    let shown = args
                        .iter()
                        .map(|a| format!("`{a}`"))
                        .collect::<Vec<_>>()
                        .join(" ");
                    ui.label(
                        RichText::new(if text.is_empty() {
                            t::SETTINGS_ARGS_HINT.to_owned()
                        } else {
                            t::settings_arguments_count(args.len(), &shown)
                        })
                        .small()
                        .color(p.text_dim),
                    );
                }
                Kind::Lines => {
                    if f.key == "index.excluded" {
                        for always in julie_core::project::ALWAYS_EXCLUDED {
                            ui.label(RichText::new(always).monospace().color(p.ignored));
                        }
                    }
                    let mut text = self.cur.get(scope, f.key).cloned().unwrap_or_default();
                    let edit = ui.add(
                        TextEdit::multiline(&mut text)
                            .hint_text(hint)
                            .id_salt(("reglage", f.key, scope == Scope::User))
                            .desired_rows(3)
                            .desired_width(f32::INFINITY),
                    );
                    if edit.changed() {
                        self.cur.set(scope, f.key, text);
                    }
                    ui.label(
                        RichText::new(if f.key == "index.excluded" {
                            t::SETTINGS_EXCLUDED_RULE
                        } else {
                            t::SETTINGS_LINES_HINT
                        })
                        .small()
                        .color(p.text_dim),
                    );
                }
                Kind::Choice(values) => {
                    ui.horizontal(|ui| {
                        let current = self.cur.get(scope, f.key).cloned();
                        if !f.page.user_only()
                            && ui
                                .add(egui::Button::selectable(
                                    current.is_none(),
                                    t::SETTINGS_INHERITED_CHOICE,
                                ))
                                .clicked()
                        {
                            let _ = self.unset(f.key, scope);
                        }
                        for value in values {
                            let selected = current.as_deref() == Some(*value)
                                || (f.page.user_only() && current.is_none() && *value == "sombre");
                            if ui
                                .add(egui::Button::selectable(
                                    selected,
                                    t::settings_choice_label(f.key, value),
                                ))
                                .clicked()
                            {
                                self.cur.set(scope, f.key, (*value).to_owned());
                            }
                        }
                    });
                }
                Kind::Env => self.show_env(ui, p, scope),
            }
            // Origine, sonde, erreurs.
            let mut origin = self.origin_text(f, scope, env.root);
            if scope == Scope::User && self.cur.get(Scope::Project, f.key).is_some() {
                origin = format!("{origin} — {}", t::SETTINGS_PROJECT_OVERRIDES);
            }
            if f.kind == Kind::Env
                && scope == Scope::User
                && !self.cur.env_rows(Scope::Project).is_empty()
            {
                origin = format!("{origin} — {}", t::SETTINGS_PROJECT_OVERRIDES);
            }
            ui.label(RichText::new(origin).small().color(p.text_dim));
            if let Some(slot) = self.probes.get(f.key) {
                match &slot.state {
                    ProbeState::Idle => {}
                    ProbeState::Running => {
                        ui.label(
                            RichText::new(t::SETTINGS_PROBE_RUNNING)
                                .small()
                                .color(p.text_dim),
                        );
                    }
                    ProbeState::Done(level, text) => {
                        let color = if *level == 0 { p.ok } else { p.warn };
                        ui.label(RichText::new(text).small().color(color));
                    }
                }
            }
            if let Some(error) = self.error_of(f, scope) {
                ui.label(RichText::new(error).small().color(p.error));
            }
            for (key, warning) in &self.warnings {
                if *key == f.key && f.kind != Kind::Args {
                    ui.label(RichText::new(warning).small().color(p.warn));
                }
            }
        });
        enter
    }

    fn inherited_hint(&self, f: &Field, scope: Scope, root: Option<&Path>) -> String {
        if scope == Scope::Project {
            if let Some(user) = self.cur.get(Scope::User, f.key) {
                return user.clone();
            }
        }
        self.default_text(f, root)
    }

    fn show_env(&mut self, ui: &mut Ui, p: &Palette, scope: Scope) {
        let mut remove = None;
        let rows = self.cur.env.entry(scope).or_default();
        if rows.is_empty() {
            ui.label(
                RichText::new(t::SETTINGS_ENV_EMPTY)
                    .small()
                    .color(p.text_dim),
            );
        }
        for (i, (name, value)) in rows.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.add(
                    TextEdit::singleline(name)
                        .hint_text(t::SETTINGS_ENV_NAME_HINT)
                        .id_salt(("env-nom", i, scope == Scope::User))
                        .desired_width(150.0),
                );
                ui.label("=");
                ui.add(
                    TextEdit::singleline(value)
                        .hint_text(t::SETTINGS_ENV_VALUE_HINT)
                        .id_salt(("env-val", i, scope == Scope::User))
                        .desired_width((ui.available_width() - 40.0).max(100.0)),
                );
                if ui
                    .small_button("×")
                    .on_hover_text(t::SETTINGS_ENV_REMOVE)
                    .clicked()
                {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = remove {
            rows.remove(i);
        }
        if ui.small_button(t::SETTINGS_ENV_ADD).clicked() {
            rows.push((String::new(), String::new()));
        }
        if rows.is_empty() {
            self.cur.env.remove(&scope);
        }
    }

    fn show_foot(
        &mut self,
        ui: &mut Ui,
        rect: egui::Rect,
        p: &Palette,
        env: &Env,
        intents: &mut Vec<SettingsIntent>,
    ) {
        let blocking = self.blocking_reason();
        let modified = self.is_modified();
        let y = rect.center().y;
        // Message d'erreur ou de refus, à gauche au-dessus du bouton.
        let open_w = kit::text_width(ui, t::SETTINGS_OPEN_FILE);
        let open =
            egui::Rect::from_min_size(pos2(rect.left() + 16.0, y - 14.0), vec2(open_w, 28.0));
        if kit::text_button_at(
            ui,
            p,
            open,
            Id::unique("reglages-ouvrir"),
            t::SETTINGS_OPEN_FILE,
            Look::Normal,
            true,
            "",
        ) {
            intents.push(SettingsIntent::OpenFile(self.scope()));
        }
        if let Some((error, message)) = &self.message {
            ui.painter().text(
                pos2(open.right() + 14.0, y),
                Align2::LEFT_CENTER,
                message,
                FontId::proportional(12.5),
                if *error { p.error } else { p.ok },
            );
        } else if let Some(file) = match self.scope() {
            Scope::Project => env.project_file,
            Scope::User => env.user_file,
        } {
            let shown = match (self.scope(), env.root) {
                (Scope::Project, Some(root)) => Self::rel(Some(root), file),
                _ => file.display().to_string(),
            };
            let clip = egui::Rect::from_min_max(
                pos2(open.right() + 14.0, rect.top()),
                pos2(rect.right() - 3.0 * 100.0 - 24.0, rect.bottom()),
            );
            ui.painter().with_clip_rect(clip).text(
                pos2(open.right() + 14.0, y),
                Align2::LEFT_CENTER,
                shown,
                FontId::proportional(11.5),
                p.text_dim,
            );
        }
        let w = 92.0;
        let mut x = rect.right() - 16.0 - w;
        let apply_enabled = modified && blocking.is_none() && !self.applying;
        let tip = blocking.clone().unwrap_or_default();
        let apply = egui::Rect::from_min_size(pos2(x, y - 14.0), vec2(w, 28.0));
        if kit::text_button_at(
            ui,
            p,
            apply,
            Id::unique("reglages-appliquer"),
            t::SETTINGS_APPLY,
            Look::Normal,
            apply_enabled,
            &tip,
        ) {
            intents.extend(self.request_apply(false));
        }
        x -= w + 8.0;
        let cancel = egui::Rect::from_min_size(pos2(x, y - 14.0), vec2(w, 28.0));
        if kit::text_button_at(
            ui,
            p,
            cancel,
            Id::unique("reglages-annuler"),
            t::SETTINGS_CANCEL,
            Look::Normal,
            true,
            "",
        ) {
            intents.push(SettingsIntent::Close);
        }
        x -= w + 8.0;
        let ok = egui::Rect::from_min_size(pos2(x, y - 14.0), vec2(w, 28.0));
        if kit::text_button_at(
            ui,
            p,
            ok,
            Id::unique("reglages-ok"),
            t::SETTINGS_OK,
            Look::Primary(p.accent),
            blocking.is_none() && !self.applying,
            &tip,
        ) {
            intents.extend(self.request_apply(true));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(user: &str, project: &str) -> Settings {
        Settings::from_layers(&[user, project]).unwrap()
    }

    #[test]
    fn draft_reads_each_scope_and_effective_prefers_the_project() {
        let s = settings(
            "[phpunit]\nphp_path = 'C:\\php.exe'\n",
            "[phpunit]\nphp_path = 'D:\\php.exe'\nextra_args = [\"--group\", \"très lent\"]\n[phpunit.env]\nAPP_ENV = \"test\"\n",
        );
        let draft = Draft::from_settings(&s);
        assert_eq!(
            draft.get(Scope::User, "phpunit.php_path").unwrap(),
            "C:\\php.exe"
        );
        assert_eq!(draft.eff("phpunit.php_path").unwrap(), "D:\\php.exe");
        assert_eq!(
            draft.get(Scope::Project, "phpunit.extra_args").unwrap(),
            "--group \"très lent\""
        );
        assert_eq!(
            draft.env_rows(Scope::Project),
            &[("APP_ENV".to_owned(), "test".to_owned())]
        );
    }

    #[test]
    fn edits_only_contain_what_changed_and_revert_unsets() {
        let s = settings("", "[phpunit]\nphp_path = 'D:\\php.exe'\n");
        let mut dialog = SettingsDialog::new(&s, None, true, true);
        assert!(dialog.build_edits().unwrap().is_empty());
        dialog
            .set_text(
                "phpunit.phpunit_path",
                Scope::Project,
                "outils\\phpunit.phar",
            )
            .unwrap();
        dialog.unset("phpunit.php_path", Scope::Project).unwrap();
        dialog
            .set_text(
                "phpunit.extra_args",
                Scope::Project,
                "--group \"très lent\"",
            )
            .unwrap();
        dialog.set_env(Scope::Project, "APP_ENV", "test");
        let edits = dialog.build_edits().unwrap();
        assert_eq!(edits.len(), 1);
        let (scope, list) = &edits[0];
        assert_eq!(*scope, Scope::Project);
        assert!(list.contains(&SettingsEdit::unset(&["phpunit", "php_path"])));
        assert!(list.contains(&SettingsEdit::set(
            &["phpunit", "phpunit_path"],
            SettingValue::Path("outils\\phpunit.phar".into())
        )));
        assert!(list.contains(&SettingsEdit::set(
            &["phpunit", "extra_args"],
            SettingValue::List(vec!["--group".into(), "très lent".into()])
        )));
        assert!(list.contains(&SettingsEdit::set(
            &["phpunit", "env", "APP_ENV"],
            SettingValue::Text("test".into())
        )));
        assert_eq!(list.len(), 4);
    }

    #[test]
    fn errors_block_and_say_why() {
        let mut dialog = SettingsDialog::new(&Settings::default(), None, true, true);
        dialog
            .set_text("phpunit.extra_args", Scope::Project, "--group \"lent")
            .unwrap();
        assert!(dialog.blocking_reason().unwrap().contains("Guillemet"));
        dialog.unset("phpunit.extra_args", Scope::Project).unwrap();
        dialog.set_env(Scope::Project, "A", "1");
        dialog.set_env(Scope::Project, "", "2");
        assert!(dialog.blocking_reason().is_some());
        let mut dialog = SettingsDialog::new(&Settings::default(), None, true, true);
        dialog
            .set_text("phpunit.php_path", Scope::User, "bin\\php.exe")
            .unwrap();
        assert!(dialog.blocking_reason().unwrap().contains("relatif"));
        dialog
            .set_text("phpunit.php_path", Scope::User, "php")
            .unwrap();
        assert!(dialog.blocking_reason().is_none());
    }

    #[test]
    fn empty_text_means_inherit_and_choices_are_checked() {
        let mut dialog = SettingsDialog::new(&Settings::default(), None, true, true);
        dialog
            .set_text("vcs.update_method", Scope::Project, "rebase")
            .unwrap();
        assert!(dialog
            .set_text("vcs.update_method", Scope::Project, "fusionner")
            .is_err());
        dialog
            .set_text("vcs.update_method", Scope::Project, "")
            .unwrap();
        assert!(dialog.build_edits().unwrap().is_empty());
        assert!(dialog
            .set_text("app.theme", Scope::Project, "clair")
            .is_err());
    }

    #[test]
    fn list_fields_drop_blank_lines() {
        assert_eq!(list_items("docs\n\n  /target \n"), vec!["docs", "/target"]);
        assert!(quotes_unbalanced("--group \"lent"));
        assert!(!quotes_unbalanced("--group \"très lent\" -v"));
    }
}
