//! Résolution : d'une référence vers ses déclarations (aller à la
//! déclaration), et d'une déclaration vers ses usages.
//!
//! PHP : noms de classes, fonctions, constantes résolus à l'analyse (espace de
//! noms, `use`) ; membres résolus ici par le type du receveur (`$this`,
//! `self`, `parent`, `X::`, paramètres et propriétés typés, `new` dans la même
//! fonction, types de retour déclarés), puis recherche du membre le plus
//! proche : la classe, ses traits, ses parents, ses interfaces. Type inconnu :
//! candidats par le nom, présentés comme devinés.
//!
//! JS/TS : portées du fichier, imports suivis à travers les réexports (5
//! niveaux), `this.m()` et `x.m()` quand `x = new C()`.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use julie_core::{CancelToken, TextPosition, TextRange};

use super::model::{
    flags, key, php_const_key, short_key, Analysis, BaseRel, Binding, Decl, ImportName, JsModule,
    JsRecv, Lang, MemberKind, Recv, Ref, RefKind, Span,
};
use super::store::{
    analyze_text, load_text, member_key, on_big_stack, DeclRef, Loaded, SymbolIndex, BIG_STACK,
};
use super::{
    Declaration, Guess, Navigation, NavigationRequest, NavigationResult, NoTarget, SymbolKind,
    SymbolLanguage, Targets, Usage, UsageCertainty, UsagesQuery, UsagesSummary,
};
use crate::error::Result;
use crate::files::relative_string;
use crate::settings::IndexSettings;
use crate::text::UnsavedFiles;

/// Profondeur des réexports JS suivis.
const MAX_REEXPORT: usize = 5;
/// Cibles rendues au plus (liste surgissante).
const MAX_TARGETS: usize = 200;

/// Une déclaration trouvée et son fichier.
#[derive(Debug, Clone)]
pub(crate) struct Hit {
    pub relative: Box<str>,
    pub library: bool,
    pub lang: Lang,
    pub decl: Decl,
}

impl Hit {
    fn from_ref(index: &SymbolIndex, r: DeclRef) -> Option<Hit> {
        let (slot, decl) = index.decl(r)?;
        Some(Hit {
            relative: slot.relative.clone(),
            library: slot.library,
            lang: slot.symbols.lang,
            decl: decl.clone(),
        })
    }

    /// Même déclaration que `d` (positions ignorées, sauf pour une locale).
    fn same_as(&self, d: &Declaration) -> bool {
        if *self.relative != *d.relative || self.decl.kind != d.kind {
            return false;
        }
        if d.kind.is_local() || d.kind == SymbolKind::Import {
            return self.decl.span.line == d.range.start.line
                && self.decl.span.col == d.range.start.column;
        }
        if self.lang.is_php() {
            let name_eq = match MemberKind::of(d.kind) {
                Some(m) if !m.case_insensitive() => *self.decl.name == *d.name,
                _ => self.decl.name.eq_ignore_ascii_case(&d.name),
            };
            name_eq && self.decl.container.eq_ignore_ascii_case(&d.container)
        } else {
            *self.decl.name == *d.name && *self.decl.container == *d.container
        }
    }

    fn php_namespace(&self) -> &str {
        if self.decl.kind.is_member() {
            self.decl
                .container
                .rsplit_once('\\')
                .map_or("", |(ns, _)| ns)
        } else {
            &self.decl.container
        }
    }
}

fn local_hit(
    relative: &str,
    library: bool,
    lang: Lang,
    analysis: &Analysis,
    local: u32,
) -> Option<Hit> {
    let l = analysis.locals.get(local as usize)?;
    Some(Hit {
        relative: relative.into(),
        library,
        lang,
        decl: Decl {
            kind: l.kind,
            flags: 0,
            name: l.name.clone(),
            container: l.container.clone(),
            span: l.span,
            types: Box::new([]),
            bases: Box::new([]),
        },
    })
}

// ---------------------------------------------------------------------------
// Textes des lignes.

/// Ligne d'un emplacement, sans indentation, 300 caractères au plus ; rend
/// aussi la colonne (caractères) où commence l'extrait.
pub(crate) fn line_excerpt(text: &str, span: Span) -> (String, u32) {
    let at = (span.start as usize).min(text.len());
    let at = (0..=at)
        .rev()
        .find(|&i| text.is_char_boundary(i))
        .unwrap_or(0);
    let start = text[..at].rfind('\n').map_or(0, |i| i + 1);
    let end = text[at..].find('\n').map_or(text.len(), |i| at + i);
    let line = text[start..end].trim_end_matches('\r');
    let trimmed = line.trim_start();
    let leading = line[..line.len() - trimmed.len()].chars().count() as u32;
    let count = trimmed.chars().count();
    if count <= 300 {
        return (trimmed.trim_end().to_string(), leading);
    }
    let from = span.col.saturating_sub(80).max(leading);
    let skip = (from - leading) as usize;
    (trimmed.chars().skip(skip).take(300).collect(), from)
}

/// Textes des fichiers cibles, lus une fois par demande.
struct Texts<'a> {
    root: &'a Path,
    unsaved: &'a UnsavedFiles,
    max_size: u64,
    cache: HashMap<Box<str>, Option<Arc<str>>>,
}

impl<'a> Texts<'a> {
    fn new(root: &'a Path, unsaved: &'a UnsavedFiles, max_size: u64) -> Self {
        Self {
            root,
            unsaved,
            max_size,
            cache: HashMap::new(),
        }
    }

    fn put(&mut self, relative: &str, text: Arc<str>) {
        self.cache.insert(relative.into(), Some(text));
    }

    fn get(&mut self, relative: &str) -> Option<Arc<str>> {
        if let Some(t) = self.cache.get(relative) {
            return t.clone();
        }
        let path = crate::files::absolute_path(self.root, relative);
        let text = match self.unsaved.get(&path) {
            Some(t) => Some(t.clone()),
            None => match load_text(&path, self.max_size).0 {
                Loaded::Text(t) => Some(Arc::from(t)),
                _ => None,
            },
        };
        self.cache.insert(relative.into(), text.clone());
        text
    }

    fn declaration(&mut self, hit: &Hit) -> Declaration {
        let line_text = self
            .get(&hit.relative)
            .map(|t| line_excerpt(&t, hit.decl.span).0)
            .unwrap_or_default();
        let qualified_name = if hit.decl.kind.is_local() || hit.decl.kind == SymbolKind::Import {
            hit.decl.name.to_string()
        } else {
            hit.decl.qualified(hit.lang)
        };
        Declaration {
            kind: hit.decl.kind,
            language: hit.lang.into(),
            name: hit.decl.name.to_string(),
            qualified_name,
            container: hit.decl.container.to_string(),
            path: crate::files::absolute_path(self.root, &hit.relative),
            relative: hit.relative.to_string(),
            range: span_range(hit.decl.span),
            line_text,
            library: hit.library,
        }
    }
}

fn span_range(span: Span) -> TextRange {
    TextRange::new(
        TextPosition::new(span.line, span.col),
        TextPosition::new(span.line, span.end_col),
    )
}

/// Octet d'une position (ligne, colonne en caractères) ; bornée au texte.
pub(crate) fn byte_of(text: &str, position: TextPosition) -> usize {
    let mut line = 0u32;
    let mut start = 0usize;
    if position.line > 0 {
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                line += 1;
                if line == position.line {
                    start = i + 1;
                    break;
                }
            }
        }
        if line < position.line {
            return text.len();
        }
    }
    let rest = &text[start..];
    let line_end = rest.find('\n').unwrap_or(rest.len());
    rest[..line_end]
        .char_indices()
        .nth(position.column as usize)
        .map_or(start + line_end, |(i, _)| start + i)
}

// ---------------------------------------------------------------------------
// Résolution PHP.

pub(crate) struct Resolver<'a> {
    pub index: &'a SymbolIndex,
}

/// Ce que donne un accès membre.
#[derive(Debug)]
pub(crate) enum MemberOutcome {
    Found(Vec<DeclRef>),
    /// Type du receveur inconnu (ou hiérarchie incomplète) : appel possible.
    Unknown,
    /// Classe connue, membre absent : rien.
    NotFound,
}

impl<'a> Resolver<'a> {
    pub(crate) fn php_classes(&self, fqn: &str) -> Vec<DeclRef> {
        let fqn = fqn.trim_start_matches('\\');
        self.index
            .by_key(key("c", fqn, true))
            .iter()
            .copied()
            .filter(|r| {
                self.index.decl(*r).is_some_and(|(slot, d)| {
                    slot.symbols.lang.is_php()
                        && d.kind.is_class_like()
                        && d.php_fqn().eq_ignore_ascii_case(fqn)
                })
            })
            .collect()
    }

    /// Membre le plus proche dans les classes données. `complete` : toute la
    /// hiérarchie parcourue est dans l'index.
    pub(crate) fn php_member(
        &self,
        classes: &[Box<str>],
        member: MemberKind,
        name: &str,
    ) -> (Vec<DeclRef>, bool) {
        let mut visited = HashSet::new();
        let mut out = Vec::new();
        let mut complete = true;
        for class in classes {
            let (found, c) = self.member_in(class, member, name, &mut visited, 0);
            complete &= c;
            for f in found {
                if !out.contains(&f) {
                    out.push(f);
                }
            }
        }
        (out, complete)
    }

    fn member_in(
        &self,
        class: &str,
        member: MemberKind,
        name: &str,
        visited: &mut HashSet<String>,
        depth: usize,
    ) -> (Vec<DeclRef>, bool) {
        if depth > 32 || !visited.insert(class.to_ascii_lowercase()) {
            return (Vec::new(), true); // héritage cyclique : on s'arrête
        }
        let classes = self.php_classes(class);
        if classes.is_empty() {
            return (Vec::new(), false);
        }
        let mut out = Vec::new();
        let mut complete = true;
        for c in classes {
            let Some((_, decl)) = self.index.decl(c) else {
                continue;
            };
            let fqn = decl.php_fqn();
            let own: Vec<DeclRef> = self
                .index
                .by_key(member_key(
                    member.tag(),
                    &fqn,
                    name,
                    member.case_insensitive(),
                ))
                .iter()
                .copied()
                .filter(|r| {
                    self.index.decl(*r).is_some_and(|(_, d)| {
                        member.matches(d.kind)
                            && d.container.eq_ignore_ascii_case(&fqn)
                            && if member.case_insensitive() {
                                d.name.eq_ignore_ascii_case(name)
                            } else {
                                *d.name == *name
                            }
                    })
                })
                .collect();
            if !own.is_empty() {
                out.extend(own);
                continue;
            }
            let bases = decl.bases.clone();
            let mut level_found = false;
            for rel in [BaseRel::Uses, BaseRel::Extends, BaseRel::Implements] {
                let mut found_here = Vec::new();
                for (r, base) in bases.iter() {
                    if *r != rel {
                        continue;
                    }
                    let (f, comp) = self.member_in(base, member, name, visited, depth + 1);
                    complete &= comp;
                    found_here.extend(f);
                }
                if !found_here.is_empty() {
                    out.extend(found_here);
                    level_found = true;
                    break;
                }
            }
            let _ = level_found;
        }
        (out, complete)
    }

    /// Classes d'un receveur ; `None` : type inconnu.
    pub(crate) fn php_recv(&self, recv: &Recv, depth: usize) -> Option<Vec<Box<str>>> {
        if depth > 8 {
            return None;
        }
        match recv {
            Recv::Classes(c) if !c.is_empty() => Some(c.clone()),
            Recv::Prop(inner, name) | Recv::Call(inner, name) => {
                let classes = self.php_recv(inner, depth + 1)?;
                let member = if matches!(recv, Recv::Prop(..)) {
                    MemberKind::Property
                } else {
                    MemberKind::Method
                };
                let (found, _) = self.php_member(&classes, member, name);
                let mut types = Vec::new();
                for r in found {
                    if let Some((_, d)) = self.index.decl(r) {
                        for t in d.types.iter() {
                            if &**t == "static" {
                                types.extend(classes.iter().cloned());
                            } else {
                                types.push(t.clone());
                            }
                        }
                    }
                }
                types.dedup();
                (!types.is_empty()).then_some(types)
            }
            Recv::Func(candidates) => {
                let found = self.php_function(candidates, false);
                let types: Vec<Box<str>> = found
                    .iter()
                    .filter_map(|r| self.index.decl(*r))
                    .flat_map(|(_, d)| d.types.iter().cloned())
                    .filter(|t| &**t != "static")
                    .collect();
                (!types.is_empty()).then_some(types)
            }
            _ => None,
        }
    }

    pub(crate) fn php_member_outcome(
        &self,
        member: MemberKind,
        name: &str,
        recv: &Recv,
    ) -> MemberOutcome {
        let Some(classes) = self.php_recv(recv, 0) else {
            return MemberOutcome::Unknown;
        };
        let (found, complete) = self.php_member(&classes, member, name);
        if !found.is_empty() {
            MemberOutcome::Found(found)
        } else if !complete {
            MemberOutcome::Unknown
        } else {
            MemberOutcome::NotFound
        }
    }

    /// Fonction (ou constante) : premier candidat déclaré, dans l'ordre de PHP.
    pub(crate) fn php_function(&self, candidates: &[Box<str>], constant: bool) -> Vec<DeclRef> {
        for c in candidates {
            let k = if constant {
                key("k", &php_const_key(c), false)
            } else {
                key("f", c, true)
            };
            let found: Vec<DeclRef> = self
                .index
                .by_key(k)
                .iter()
                .copied()
                .filter(|r| {
                    self.index.decl(*r).is_some_and(|(slot, d)| {
                        slot.symbols.lang.is_php()
                            && if constant {
                                d.kind == SymbolKind::Constant
                                    && php_const_key(&d.php_fqn()) == php_const_key(c)
                            } else {
                                d.kind == SymbolKind::Function
                                    && d.php_fqn().eq_ignore_ascii_case(c)
                            }
                    })
                })
                .collect();
            if !found.is_empty() {
                return found;
            }
        }
        Vec::new()
    }

    /// Candidats devinés : membres PHP du même genre et du même nom.
    fn php_guess(&self, member: MemberKind, name: &str) -> Vec<DeclRef> {
        let tag = format!("p{}", member.tag());
        self.index
            .by_name(key(&tag, name, member.case_insensitive()))
            .iter()
            .copied()
            .filter(|r| {
                self.index.decl(*r).is_some_and(|(slot, d)| {
                    slot.symbols.lang.is_php()
                        && member.matches(d.kind)
                        && if member.case_insensitive() {
                            d.name.eq_ignore_ascii_case(name)
                        } else {
                            *d.name == *name
                        }
                })
            })
            .collect()
    }

    // --- JS -----------------------------------------------------------------

    /// Fichier visé par un chemin d'import relatif (`./util` → `web/js/util.js`).
    pub(crate) fn js_module(&self, from: &str, specifier: &str) -> Option<String> {
        if !(specifier.starts_with("./")
            || specifier.starts_with("../")
            || specifier.starts_with('/'))
        {
            return None;
        }
        let mut parts: Vec<&str> = if specifier.starts_with('/') {
            Vec::new()
        } else {
            from.split('/').collect()
        };
        if !specifier.starts_with('/') {
            parts.pop(); // le nom du fichier
        }
        for seg in specifier.split('/') {
            match seg {
                "" | "." => {}
                ".." => {
                    parts.pop();
                }
                s => parts.push(s),
            }
        }
        let base = parts.join("/");
        let exts = ["js", "ts", "tsx", "jsx", "mjs", "cjs"];
        let mut candidates = vec![base.clone()];
        candidates.extend(exts.iter().map(|e| format!("{base}.{e}")));
        candidates.extend(exts.iter().map(|e| format!("{base}/index.{e}")));
        candidates.into_iter().find(|c| self.index.contains(c))
    }

    fn js_view(&self, relative: &str) -> Option<JsView<'a>> {
        let slot = self.index.slot_by_path(relative)?;
        Some(JsView {
            relative: slot.relative.clone(),
            library: slot.library,
            lang: slot.symbols.lang,
            decls: &slot.symbols.decls,
            module: slot.symbols.js.as_deref()?,
        })
    }

    /// Ce qu'exporte un module sous `name` (`default` compris).
    fn js_export(&self, relative: &str, name: &str, depth: usize) -> Vec<Hit> {
        if depth > MAX_REEXPORT {
            return Vec::new();
        }
        let Some(view) = self.js_view(relative) else {
            return Vec::new();
        };
        let direct: Vec<Hit> = view
            .decls
            .iter()
            .filter(|d| {
                d.has(flags::EXPORTED)
                    && if name == "default" {
                        d.has(flags::DEFAULT_EXPORT)
                    } else {
                        !d.has(flags::DEFAULT_EXPORT) && *d.name == *name
                    }
            })
            .map(|d| view.hit(d))
            .collect();
        if !direct.is_empty() {
            return direct;
        }
        for (local, exported) in &view.module.exports {
            if **exported == *name {
                let found = self.js_module_binding(&view, local, depth + 1);
                if !found.is_empty() {
                    return found;
                }
            }
        }
        let mut out = Vec::new();
        for re in &view.module.reexports {
            let Some(target) = self.js_module(&view.relative, &re.source) else {
                continue;
            };
            match &re.names {
                Some(names) => {
                    for (imported, exported) in names {
                        if **exported == *name {
                            out.extend(self.js_export(&target, imported, depth + 1));
                        }
                    }
                }
                None if name != "default" => out.extend(self.js_export(&target, name, depth + 1)),
                None => {}
            }
            if !out.is_empty() {
                break;
            }
        }
        out
    }

    /// Liaison de premier niveau `local` d'un module (déclaration ou import).
    fn js_module_binding(&self, view: &JsView, local: &str, depth: usize) -> Vec<Hit> {
        let found: Vec<Hit> = view
            .decls
            .iter()
            .filter(|d| d.has(flags::MODULE_LEVEL) && *d.name == *local)
            .map(|d| view.hit(d))
            .collect();
        if !found.is_empty() {
            return found;
        }
        for import in &view.module.imports {
            if *import.local == *local {
                if let JsOutcome::Hits(h) = self.js_follow(view, import, depth) {
                    return h;
                }
            }
        }
        Vec::new()
    }

    fn js_follow(&self, view: &JsView, import: &super::model::JsImport, depth: usize) -> JsOutcome {
        let Some(target) = self.js_module(&view.relative, &import.source) else {
            return JsOutcome::External(import.source.to_string());
        };
        match &import.imported {
            ImportName::Named(n) => JsOutcome::Hits(self.js_export(&target, n, depth + 1)),
            ImportName::Default => JsOutcome::Hits(self.js_export(&target, "default", depth + 1)),
            ImportName::Namespace => JsOutcome::File(target),
        }
    }

    /// Membre `name` d'une classe JS (puis de ses parents) ; `complete` faux si
    /// un parent n'a pas été trouvé.
    fn js_class_member(
        &self,
        view: &JsView,
        class: &Decl,
        name: &str,
        depth: usize,
    ) -> (Vec<Hit>, bool) {
        if depth > 10 {
            return (Vec::new(), true);
        }
        let own: Vec<Hit> = view
            .decls
            .iter()
            .filter(|d| {
                matches!(d.kind, SymbolKind::Method | SymbolKind::Property)
                    && *d.container == *class.name
                    && *d.name == *name
            })
            .map(|d| view.hit(d))
            .collect();
        if !own.is_empty() {
            return (own, true);
        }
        let mut complete = true;
        for (rel, base) in class.bases.iter() {
            if *rel != BaseRel::Extends {
                continue;
            }
            let parents = self.js_module_binding(view, base, 0);
            if parents.is_empty() {
                complete = false;
            }
            for p in parents {
                if p.decl.kind != SymbolKind::Class {
                    complete = false;
                    continue;
                }
                let Some(pview) = self.js_view(&p.relative) else {
                    complete = false;
                    continue;
                };
                let (found, c) = self.js_class_member(&pview, &p.decl, name, depth + 1);
                complete &= c;
                if !found.is_empty() {
                    return (found, complete);
                }
            }
        }
        (Vec::new(), complete)
    }

    fn js_guess_members(&self, name: &str) -> Vec<DeclRef> {
        let mut out: Vec<DeclRef> = Vec::new();
        for tag in ["jm", "jp"] {
            out.extend(
                self.index
                    .by_name(key(tag, name, false))
                    .iter()
                    .copied()
                    .filter(|r| {
                        self.index
                            .decl(*r)
                            .is_some_and(|(_, d)| *d.name == *name && d.kind.is_member())
                    }),
            );
        }
        out
    }

    fn js_guess_globals(&self, name: &str, exclude: &str) -> Vec<DeclRef> {
        self.index
            .by_name(key("jg", name, false))
            .iter()
            .copied()
            .filter(|r| {
                self.index
                    .decl(*r)
                    .is_some_and(|(slot, d)| *d.name == *name && *slot.relative != *exclude)
            })
            .collect()
    }
}

/// Vue d'un module JS (fichier de l'index ou fichier analysé).
struct JsView<'v> {
    relative: Box<str>,
    library: bool,
    lang: Lang,
    decls: &'v [Decl],
    module: &'v JsModule,
}

impl JsView<'_> {
    fn hit(&self, d: &Decl) -> Hit {
        Hit {
            relative: self.relative.clone(),
            library: self.library,
            lang: self.lang,
            decl: d.clone(),
        }
    }
}

enum JsOutcome {
    Hits(Vec<Hit>),
    File(String),
    External(String),
}

/// Ce que donne une référence (aller à la déclaration).
enum Resolved {
    Sure(Vec<Hit>),
    Guessed(Vec<Hit>, Guess),
    /// `new X` : la classe et le constructeur.
    Class(Vec<Hit>, Option<Hit>),
    File(String),
    Nothing(NoTarget),
    /// Une locale à sa déclaration.
    AtDeclaration(Hit),
}

/// Le fichier analysé et sa place dans le projet.
pub(crate) struct Current<'c> {
    pub relative: &'c str,
    pub library: bool,
    pub analysis: &'c Analysis,
}

impl<'c> Current<'c> {
    fn lang(&self) -> Lang {
        self.analysis.symbols.lang
    }

    fn view(&self) -> Option<JsView<'c>> {
        Some(JsView {
            relative: self.relative.into(),
            library: self.library,
            lang: self.lang(),
            decls: &self.analysis.symbols.decls,
            module: self.analysis.symbols.js.as_deref()?,
        })
    }
}

impl<'a> Resolver<'a> {
    fn hits(&self, refs: &[DeclRef]) -> Vec<Hit> {
        refs.iter()
            .filter_map(|r| Hit::from_ref(self.index, *r))
            .collect()
    }

    /// Résout une référence du fichier courant.
    fn resolve(&self, cur: &Current, r: &Ref) -> Resolved {
        match &r.kind {
            RefKind::Class { fqn, context, .. } => {
                let classes = self.hits(&self.php_classes(fqn));
                if classes.is_empty() {
                    return Resolved::Nothing(NoTarget::NotInProject {
                        name: r.name.to_string(),
                    });
                }
                if *context == super::model::ClassContext::New {
                    let (ctor, _) = self.php_member(
                        std::slice::from_ref(fqn),
                        MemberKind::Method,
                        "__construct",
                    );
                    let ctor = ctor.first().and_then(|c| Hit::from_ref(self.index, *c));
                    return Resolved::Class(classes, ctor);
                }
                Resolved::Sure(classes)
            }
            RefKind::Function { candidates } | RefKind::Constant { candidates } => {
                let constant = matches!(r.kind, RefKind::Constant { .. });
                let found = self.hits(&self.php_function(candidates, constant));
                if found.is_empty() {
                    Resolved::Nothing(NoTarget::NotInProject {
                        name: r.name.to_string(),
                    })
                } else {
                    Resolved::Sure(found)
                }
            }
            RefKind::Member {
                member,
                recv,
                recv_text,
            } => match self.php_member_outcome(*member, &r.name, recv) {
                MemberOutcome::Found(found) => Resolved::Sure(self.hits(&found)),
                MemberOutcome::NotFound => Resolved::Nothing(NoTarget::NotFound {
                    name: r.name.to_string(),
                }),
                MemberOutcome::Unknown => {
                    let guesses = self.hits(&self.php_guess(*member, &r.name));
                    if guesses.is_empty() {
                        return Resolved::Nothing(NoTarget::NotFound {
                            name: r.name.to_string(),
                        });
                    }
                    Resolved::Guessed(
                        guesses,
                        Guess::UnknownReceiver {
                            receiver: recv_text.to_string(),
                            member: r.name.to_string(),
                            kind: match member {
                                MemberKind::Method => SymbolKind::Method,
                                MemberKind::Property => SymbolKind::Property,
                                MemberKind::Constant => SymbolKind::ClassConstant,
                            },
                        },
                    )
                }
            },
            RefKind::Local { local } => {
                let Some(hit) =
                    local_hit(cur.relative, cur.library, cur.lang(), cur.analysis, *local)
                else {
                    return Resolved::Nothing(NoTarget::NotASymbol);
                };
                if hit.decl.span == r.span {
                    Resolved::AtDeclaration(hit)
                } else {
                    Resolved::Sure(vec![hit])
                }
            }
            RefKind::JsIdent { scope } => self.js_ident(cur, *scope, &r.name),
            RefKind::JsImportName { import, .. } => {
                let (Some(view), Some(module)) = (cur.view(), cur.analysis.symbols.js.as_deref())
                else {
                    return Resolved::Nothing(NoTarget::NotASymbol);
                };
                let Some(imp) = module.imports.get(*import as usize) else {
                    return Resolved::Nothing(NoTarget::NotASymbol);
                };
                self.js_outcome(self.js_follow(&view, imp, 0), &r.name)
            }
            RefKind::JsMember { recv, recv_text } => self.js_member(cur, recv, recv_text, &r.name),
            RefKind::ModulePath { specifier } => match self.js_module(cur.relative, specifier) {
                Some(file) => Resolved::File(file),
                None => Resolved::Nothing(NoTarget::ExternalModule {
                    specifier: specifier.to_string(),
                }),
            },
        }
    }

    fn js_outcome(&self, outcome: JsOutcome, name: &str) -> Resolved {
        match outcome {
            JsOutcome::Hits(h) if !h.is_empty() => Resolved::Sure(h),
            JsOutcome::Hits(_) => Resolved::Nothing(NoTarget::NotFound {
                name: name.to_string(),
            }),
            JsOutcome::File(f) => Resolved::File(f),
            JsOutcome::External(s) => Resolved::Nothing(NoTarget::ExternalModule { specifier: s }),
        }
    }

    fn js_binding(&self, cur: &Current, binding: Binding, name: &str) -> Resolved {
        match binding {
            Binding::Decl(i) => match cur.analysis.symbols.decls.get(i as usize) {
                Some(d) => Resolved::Sure(vec![Hit {
                    relative: cur.relative.into(),
                    library: cur.library,
                    lang: cur.lang(),
                    decl: d.clone(),
                }]),
                None => Resolved::Nothing(NoTarget::NotASymbol),
            },
            Binding::Local(i) => {
                match local_hit(cur.relative, cur.library, cur.lang(), cur.analysis, i) {
                    Some(h) => Resolved::Sure(vec![h]),
                    None => Resolved::Nothing(NoTarget::NotASymbol),
                }
            }
            Binding::Import(i) => {
                let (Some(view), Some(module)) = (cur.view(), cur.analysis.symbols.js.as_deref())
                else {
                    return Resolved::Nothing(NoTarget::NotASymbol);
                };
                match module.imports.get(i as usize) {
                    Some(imp) => self.js_outcome(self.js_follow(&view, imp, 0), name),
                    None => Resolved::Nothing(NoTarget::NotASymbol),
                }
            }
        }
    }

    fn js_ident(&self, cur: &Current, scope: u32, name: &str) -> Resolved {
        match cur.analysis.lookup(scope, name) {
            Some(binding) => self.js_binding(cur, binding, name),
            None => {
                let found = self.hits(&self.js_guess_globals(name, cur.relative));
                if found.is_empty() {
                    Resolved::Nothing(NoTarget::NotInProject {
                        name: name.to_string(),
                    })
                } else {
                    Resolved::Guessed(
                        found,
                        Guess::GlobalName {
                            name: name.to_string(),
                        },
                    )
                }
            }
        }
    }

    /// Classes (déclarations) désignées par la valeur d'un identifiant JS :
    /// la classe elle-même, ou `new C()` affecté à une variable.
    fn js_value_classes(&self, cur: &Current, scope: u32, name: &str) -> Option<Vec<Hit>> {
        let binding = cur.analysis.lookup(scope, name)?;
        let (class_name, class_scope) = match binding {
            Binding::Decl(i) => {
                let d = cur.analysis.symbols.decls.get(i as usize)?;
                if d.kind == SymbolKind::Class {
                    return Some(vec![Hit {
                        relative: cur.relative.into(),
                        library: cur.library,
                        lang: cur.lang(),
                        decl: d.clone(),
                    }]);
                }
                (d.types.first()?.clone(), 0)
            }
            Binding::Local(i) => {
                let l = cur.analysis.locals.get(i as usize)?;
                (l.init_new.clone()?, l.scope)
            }
            Binding::Import(_) => {
                return match self.js_binding(cur, binding, name) {
                    Resolved::Sure(h) if h.iter().all(|h| h.decl.kind == SymbolKind::Class) => {
                        Some(h)
                    }
                    _ => None,
                };
            }
        };
        match self.js_binding(
            cur,
            cur.analysis.lookup(class_scope, &class_name)?,
            &class_name,
        ) {
            Resolved::Sure(h) if h.iter().all(|h| h.decl.kind == SymbolKind::Class) => Some(h),
            _ => None,
        }
    }

    fn js_member(&self, cur: &Current, recv: &JsRecv, recv_text: &str, name: &str) -> Resolved {
        let classes: Option<Vec<Hit>> = match recv {
            JsRecv::This(Some(ci)) => cur.analysis.symbols.decls.get(*ci as usize).map(|d| {
                vec![Hit {
                    relative: cur.relative.into(),
                    library: cur.library,
                    lang: cur.lang(),
                    decl: d.clone(),
                }]
            }),
            JsRecv::Ident(n, scope) => {
                // `import * as ns` : membre = export du module.
                if let Some(Binding::Import(i)) = cur.analysis.lookup(*scope, n) {
                    let module = cur.analysis.symbols.js.as_deref();
                    if let Some(imp) = module.and_then(|m| m.imports.get(i as usize)) {
                        if imp.imported == ImportName::Namespace {
                            return match self.js_module(cur.relative, &imp.source) {
                                Some(file) => {
                                    let h = self.js_export(&file, name, 0);
                                    if h.is_empty() {
                                        Resolved::Nothing(NoTarget::NotFound {
                                            name: name.to_string(),
                                        })
                                    } else {
                                        Resolved::Sure(h)
                                    }
                                }
                                None => Resolved::Nothing(NoTarget::ExternalModule {
                                    specifier: imp.source.to_string(),
                                }),
                            };
                        }
                    }
                }
                self.js_value_classes(cur, *scope, n)
            }
            _ => None,
        };
        if let Some(classes) = classes {
            let mut found = Vec::new();
            let mut complete = true;
            for class in &classes {
                let view = if *class.relative == *cur.relative {
                    cur.view()
                } else {
                    self.js_view(&class.relative)
                };
                let Some(view) = view else {
                    complete = false;
                    continue;
                };
                let (f, c) = self.js_class_member(&view, &class.decl, name, 0);
                complete &= c;
                found.extend(f);
            }
            if !found.is_empty() {
                return Resolved::Sure(found);
            }
            if complete {
                return Resolved::Nothing(NoTarget::NotFound {
                    name: name.to_string(),
                });
            }
        }
        let guesses = self.hits(&self.js_guess_members(name));
        if guesses.is_empty() {
            Resolved::Nothing(NoTarget::NotFound {
                name: name.to_string(),
            })
        } else {
            Resolved::Guessed(
                guesses,
                Guess::UnknownReceiver {
                    receiver: recv_text.to_string(),
                    member: name.to_string(),
                    kind: SymbolKind::Method,
                },
            )
        }
    }
}

// ---------------------------------------------------------------------------
// Aller à la déclaration.

/// La référence (ou la déclaration) sous l'octet `byte`.
enum AtPosition<'r> {
    Ref(&'r Ref),
    Decl(&'r Decl),
    Nothing,
}

fn at_position(analysis: &Analysis, byte: usize) -> AtPosition<'_> {
    let best_ref = analysis
        .refs
        .iter()
        .filter(|r| r.span.contains(byte))
        .min_by_key(|r| r.span.end - r.span.start);
    if let Some(r) = best_ref {
        return AtPosition::Ref(r);
    }
    match analysis
        .symbols
        .decls
        .iter()
        .filter(|d| d.span.contains(byte))
        .min_by_key(|d| d.span.end - d.span.start)
    {
        Some(d) => AtPosition::Decl(d),
        None => AtPosition::Nothing,
    }
}

/// Aller à la déclaration (synchrone) : analyse `request.text` et résout ce
/// qu'il y a sous `request.position` avec `index`. **Accès disque** (lignes des
/// cibles). `unsaved` : contenus des onglets non enregistrés.
pub fn navigate(
    index: &SymbolIndex,
    request: &NavigationRequest,
    unsaved: &UnsavedFiles,
) -> NavigationResult {
    let lang = Lang::from_path(&request.path);
    let analysis = lang.and_then(|lang| {
        let text = request.text.clone();
        on_big_stack(move || analyze_text(lang, &text)).flatten()
    });
    match analysis {
        Some(a) => navigate_with(index, request, &a, unsaved, None, 10 * 1024 * 1024),
        None => NavigationResult {
            path: request.path.clone(),
            position: request.position,
            word: None,
            navigation: Navigation::Nothing(NoTarget::NotASymbol),
            indexing: None,
        },
    }
}

pub(crate) fn navigate_with(
    index: &SymbolIndex,
    request: &NavigationRequest,
    analysis: &Analysis,
    unsaved: &UnsavedFiles,
    indexing: Option<u8>,
    max_size: u64,
) -> NavigationResult {
    let relative = relative_string(index.root(), &request.path)
        .unwrap_or_else(|| request.path.to_string_lossy().replace('\\', "/"));
    let library = index.slot_by_path(&relative).is_some_and(|s| s.library);
    let cur = Current {
        relative: &relative,
        library,
        analysis,
    };
    let byte = byte_of(&request.text, request.position);
    let mut texts = Texts::new(index.root(), unsaved, max_size);
    texts.put(&relative, request.text.clone());
    let resolver = Resolver { index };
    let result = |word: Option<Span>, navigation| NavigationResult {
        path: request.path.clone(),
        position: request.position,
        word: word.map(span_range),
        navigation,
        indexing,
    };
    let lang = analysis.symbols.lang;
    match at_position(analysis, byte) {
        AtPosition::Nothing => result(None, Navigation::Nothing(NoTarget::NotASymbol)),
        AtPosition::Decl(d) => {
            let hit = Hit {
                relative: relative.as_str().into(),
                library,
                lang,
                decl: d.clone(),
            };
            result(
                Some(d.span),
                Navigation::Declaration(texts.declaration(&hit)),
            )
        }
        AtPosition::Ref(r) => {
            let ns = analysis.namespace_at(r.span.start).to_string();
            let order = |hits: &mut Vec<Hit>| sort_hits(hits, &relative, &ns, lang);
            let navigation = match resolver.resolve(&cur, r) {
                Resolved::Sure(mut hits) => {
                    dedup_hits(&mut hits);
                    order(&mut hits);
                    hits.truncate(MAX_TARGETS);
                    Navigation::Targets(Targets {
                        targets: hits.iter().map(|h| texts.declaration(h)).collect(),
                        guess: None,
                        constructor: None,
                    })
                }
                Resolved::Class(mut hits, ctor) => {
                    dedup_hits(&mut hits);
                    order(&mut hits);
                    Navigation::Targets(Targets {
                        targets: hits.iter().map(|h| texts.declaration(h)).collect(),
                        guess: None,
                        constructor: ctor.map(|c| texts.declaration(&c)),
                    })
                }
                Resolved::Guessed(mut hits, guess) => {
                    dedup_hits(&mut hits);
                    order(&mut hits);
                    hits.truncate(MAX_TARGETS);
                    Navigation::Targets(Targets {
                        targets: hits.iter().map(|h| texts.declaration(h)).collect(),
                        guess: Some(guess),
                        constructor: None,
                    })
                }
                Resolved::File(file) => Navigation::OpenFile(index.absolute(&file)),
                Resolved::AtDeclaration(hit) => Navigation::Declaration(texts.declaration(&hit)),
                Resolved::Nothing(reason) => Navigation::Nothing(reason),
            };
            let word = match &navigation {
                Navigation::Nothing(_) => None,
                _ => Some(r.span),
            };
            result(word, navigation)
        }
    }
}

fn dedup_hits(hits: &mut Vec<Hit>) {
    let mut seen = HashSet::new();
    hits.retain(|h| seen.insert((h.relative.clone(), h.decl.span.start, h.decl.kind)));
}

/// Ordre de la liste : fichier courant, même espace de noms (même dossier en
/// JS), projet, bibliothèques ; puis nom qualifié.
fn sort_hits(hits: &mut [Hit], current: &str, namespace: &str, lang: Lang) {
    let dir = current.rsplit_once('/').map_or("", |(d, _)| d);
    hits.sort_by_cached_key(|h| {
        let rank = if *h.relative == *current {
            0
        } else if h.library {
            3
        } else if (lang.is_php()
            && h.lang.is_php()
            && h.php_namespace().eq_ignore_ascii_case(namespace))
            || (!lang.is_php() && h.relative.rsplit_once('/').map_or("", |(d, _)| d) == dir)
        {
            1
        } else {
            2
        };
        (
            rank,
            h.decl.qualified(h.lang).to_lowercase(),
            h.relative.clone(),
            h.decl.span.start,
        )
    });
}

// ---------------------------------------------------------------------------
// Usages.

/// Ce que l'on cherche.
enum Wanted {
    PhpClass(String),
    PhpFunction,
    PhpConstant,
    PhpMember(MemberKind),
    JsBinding,
    JsMember,
    Local,
    None,
}

fn wanted(symbol: &Declaration) -> (Wanted, Vec<u32>) {
    let php = symbol.language == SymbolLanguage::Php;
    let fqn = || {
        if symbol.container.is_empty() {
            symbol.name.clone()
        } else {
            format!("{}\\{}", symbol.container, symbol.name)
        }
    };
    if symbol.kind.is_local() || (symbol.kind == SymbolKind::Import) {
        return (Wanted::Local, Vec::new());
    }
    if php {
        match symbol.kind {
            k if k.is_class_like() => {
                let f = fqn();
                let keys = vec![short_key("c", &f, true)];
                (Wanted::PhpClass(f), keys)
            }
            SymbolKind::Function => {
                let keys = vec![short_key("f", &fqn(), true)];
                (Wanted::PhpFunction, keys)
            }
            SymbolKind::Constant => {
                let keys = vec![short_key("k", &php_const_key(&fqn()), false)];
                (Wanted::PhpConstant, keys)
            }
            k if k.is_member() => {
                let Some(m) = MemberKind::of(k) else {
                    return (Wanted::None, Vec::new());
                };
                let keys = vec![short_key(m.tag(), &symbol.name, true)];
                (Wanted::PhpMember(m), keys)
            }
            _ => (Wanted::None, Vec::new()),
        }
    } else if symbol.kind.is_member() {
        (Wanted::JsMember, vec![short_key("j", &symbol.name, false)])
    } else {
        let stem = super::model::module_stem(&symbol.relative);
        let mut keys = vec![
            short_key("j", &symbol.name, false),
            short_key("s", &stem, false),
        ];
        if stem == "index" {
            if let Some((dir, _)) = symbol.relative.rsplit_once('/') {
                let d = dir.rsplit('/').next().unwrap_or(dir);
                keys.push(short_key("s", &d.to_ascii_lowercase(), false));
            }
        }
        (Wanted::JsBinding, keys)
    }
}

/// Résultat d'un fichier candidat.
struct FileUsages {
    usages: Vec<Usage>,
    readable: bool,
}

fn usages_in_file(
    resolver: &Resolver,
    symbol: &Declaration,
    wanted: &Wanted,
    relative: &str,
    library: bool,
    text: &str,
    unsaved: bool,
) -> Vec<Usage> {
    let Some(lang) = Lang::from_name(relative) else {
        return Vec::new();
    };
    let Some(analysis) = analyze_text(lang, text) else {
        return Vec::new();
    };
    let cur = Current {
        relative,
        library,
        analysis: &analysis,
    };
    let root = resolver.index.root();
    let mut out = Vec::new();
    let mut push = |r: &Ref, certainty: UsageCertainty| {
        let (line_text, line_text_start) = line_excerpt(text, r.span);
        out.push(Usage {
            certainty,
            path: crate::files::absolute_path(root, relative),
            relative: relative.to_string(),
            range: span_range(r.span),
            line_text,
            line_text_start,
            library,
            unsaved,
        });
    };
    let contains = |hits: &[Hit]| hits.iter().any(|h| h.same_as(symbol));
    // Locale : la déclaration est retrouvée par sa position.
    let target_local = if matches!(wanted, Wanted::Local) {
        analysis.locals.iter().position(|l| {
            l.span.line == symbol.range.start.line && l.span.col == symbol.range.start.column
        })
    } else {
        None
    };
    for r in &analysis.refs {
        match (wanted, &r.kind) {
            (
                Wanted::PhpClass(fqn),
                RefKind::Class {
                    fqn: f, keyword, ..
                },
            ) => {
                if !keyword && f.eq_ignore_ascii_case(fqn) {
                    push(r, UsageCertainty::Sure);
                }
            }
            (Wanted::PhpFunction, RefKind::Function { candidates }) => {
                if r.name.eq_ignore_ascii_case(&symbol.name)
                    && contains(&resolver.hits(&resolver.php_function(candidates, false)))
                {
                    push(r, UsageCertainty::Sure);
                }
            }
            (Wanted::PhpConstant, RefKind::Constant { candidates }) => {
                if *r.name == *symbol.name
                    && contains(&resolver.hits(&resolver.php_function(candidates, true)))
                {
                    push(r, UsageCertainty::Sure);
                }
            }
            (Wanted::PhpMember(kind), RefKind::Member { member, recv, .. }) => {
                if member != kind {
                    continue;
                }
                let same_name = if kind.case_insensitive() {
                    r.name.eq_ignore_ascii_case(&symbol.name)
                } else {
                    *r.name == *symbol.name
                };
                if !same_name {
                    continue;
                }
                match resolver.php_member_outcome(*kind, &r.name, recv) {
                    MemberOutcome::Found(found) => {
                        if contains(&resolver.hits(&found)) {
                            push(r, UsageCertainty::Sure);
                        }
                    }
                    MemberOutcome::Unknown => push(r, UsageCertainty::Possible),
                    MemberOutcome::NotFound => {}
                }
            }
            (Wanted::JsBinding, RefKind::JsIdent { scope }) => {
                let relevant = *r.name == *symbol.name
                    || matches!(analysis.lookup(*scope, &r.name), Some(Binding::Import(_)));
                if relevant {
                    if let Resolved::Sure(h) = resolver.resolve(&cur, r) {
                        if contains(&h) {
                            push(r, UsageCertainty::Sure);
                        }
                    }
                }
            }
            (Wanted::JsBinding, RefKind::JsImportName { counted: true, .. })
            | (Wanted::JsBinding, RefKind::JsMember { .. }) => {
                let relevant =
                    *r.name == *symbol.name || matches!(&r.kind, RefKind::JsImportName { .. });
                if relevant {
                    if let Resolved::Sure(h) = resolver.resolve(&cur, r) {
                        if contains(&h) {
                            push(r, UsageCertainty::Sure);
                        }
                    }
                }
            }
            (Wanted::JsMember, RefKind::JsMember { .. }) => {
                if *r.name != *symbol.name {
                    continue;
                }
                match resolver.resolve(&cur, r) {
                    Resolved::Sure(h) => {
                        if contains(&h) {
                            push(r, UsageCertainty::Sure);
                        }
                    }
                    Resolved::Guessed(..) => push(r, UsageCertainty::Possible),
                    _ => {}
                }
            }
            (Wanted::Local, RefKind::Local { local }) => {
                if Some(*local as usize) == target_local {
                    let decl_span = analysis.locals[*local as usize].span;
                    if r.span != decl_span {
                        push(r, UsageCertainty::Sure);
                    }
                }
            }
            (Wanted::Local, RefKind::JsIdent { scope }) => {
                if let (Some(Binding::Local(l)), Some(t)) =
                    (analysis.lookup(*scope, &r.name), target_local)
                {
                    if l as usize == t {
                        push(r, UsageCertainty::Sure);
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// Trouve les usages d'une déclaration (synchrone). `sink` reçoit les usages
/// par paquets (un fichier entier à la fois ; le premier dès qu'il est
/// trouvé, puis toutes les 50 ms au plus). **Bloquant**, accès disque.
/// Annulable par `cancel` (`Err(Cancelled)`).
pub fn find_usages(
    index: &SymbolIndex,
    query: &UsagesQuery,
    unsaved: &UnsavedFiles,
    settings: &IndexSettings,
    index_complete: bool,
    cancel: &CancelToken,
    sink: &mut dyn FnMut(Vec<Usage>),
) -> Result<UsagesSummary> {
    let started = Instant::now();
    let symbol = &query.symbol;
    let (wanted, keys) = wanted(symbol);
    let php = symbol.language == SymbolLanguage::Php;
    let mut summary = UsagesSummary {
        symbol: symbol.clone(),
        sure: 0,
        possible: 0,
        files_with_sure: 0,
        files_with_possible: 0,
        files_searched: 0,
        library_files_skipped: 0,
        unreadable: 0,
        dynamic_not_searched: symbol.kind.is_member(),
        index_complete,
        elapsed: Duration::ZERO,
    };
    if matches!(wanted, Wanted::None) {
        summary.elapsed = started.elapsed();
        return Ok(summary);
    }

    // Fichiers candidats : ceux qui mentionnent le nom, plus le fichier de la
    // déclaration et les onglets non enregistrés du même langage.
    let target_rel = symbol.relative.as_str();
    let mut candidates: Vec<(String, bool)> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let same_family = |lang: Lang| lang.is_php() == php;
    if let Some(slot) = index.slot_by_path(target_rel) {
        seen.insert(target_rel.to_string());
        candidates.push((target_rel.to_string(), slot.library));
    } else {
        seen.insert(target_rel.to_string());
        candidates.push((target_rel.to_string(), symbol.library));
    }
    if !matches!(wanted, Wanted::Local) {
        for (_, slot) in index.slots() {
            if !same_family(slot.symbols.lang) || seen.contains(&*slot.relative) {
                continue;
            }
            if !keys.iter().any(|k| slot.symbols.mentions(*k)) {
                continue;
            }
            if slot.library && !query.include_libraries {
                summary.library_files_skipped += 1;
                continue;
            }
            seen.insert(slot.relative.to_string());
            candidates.push((slot.relative.to_string(), slot.library));
        }
        for path in unsaved.keys() {
            let Some(rel) = relative_string(index.root(), path) else {
                continue;
            };
            let Some(lang) = Lang::from_name(&rel) else {
                continue;
            };
            if !same_family(lang) || seen.contains(&rel) {
                continue;
            }
            let library = index
                .slot_by_path(&rel)
                .map_or_else(|| settings.is_library(&rel), |s| s.library);
            if library && !query.include_libraries {
                continue;
            }
            seen.insert(rel.clone());
            candidates.push((rel, library));
        }
    }
    // Le fichier de la déclaration d'abord, puis son dossier, puis le reste.
    let dir = target_rel
        .rsplit_once('/')
        .map_or("", |(d, _)| d)
        .to_string();
    candidates[1..].sort_by_cached_key(|(rel, lib)| {
        let same_dir = rel.rsplit_once('/').map_or("", |(d, _)| d) == dir;
        (*lib, !same_dir, rel.clone())
    });

    let resolver = Resolver { index };
    let next = AtomicUsize::new(0);
    let threads = settings.threads.clamp(1, 32).min(candidates.len().max(1));
    let (sender, receiver) = mpsc::channel::<FileUsages>();
    let root = index.root().to_path_buf();
    let max_size = settings.max_file_size;
    std::thread::scope(|scope| {
        for i in 0..threads {
            let sender = sender.clone();
            let (next, candidates, resolver, wanted, root) =
                (&next, &candidates, &resolver, &wanted, &root);
            let _ = std::thread::Builder::new()
                .name(format!("julie-index-usages-{i}"))
                .stack_size(BIG_STACK)
                .spawn_scoped(scope, move || loop {
                    if cancel.is_cancelled() {
                        break;
                    }
                    let n = next.fetch_add(1, Ordering::Relaxed);
                    let Some((relative, library)) = candidates.get(n) else {
                        break;
                    };
                    let path: PathBuf = crate::files::absolute_path(root, relative);
                    let (text, from_memory) = match unsaved.get(&path) {
                        Some(t) => (Some(t.to_string()), true),
                        None => match load_text(&path, max_size).0 {
                            Loaded::Text(t) => (Some(t), false),
                            _ => (None, false),
                        },
                    };
                    let result = match text {
                        Some(text) => FileUsages {
                            usages: usages_in_file(
                                resolver,
                                symbol,
                                wanted,
                                relative,
                                *library,
                                &text,
                                from_memory,
                            ),
                            readable: true,
                        },
                        None => FileUsages {
                            usages: Vec::new(),
                            readable: false,
                        },
                    };
                    if sender.send(result).is_err() {
                        break;
                    }
                });
        }
        drop(sender);
        let mut pending: Vec<Usage> = Vec::new();
        let mut sent_first = false;
        let mut last = Instant::now();
        loop {
            match receiver.recv_timeout(Duration::from_millis(50)) {
                Ok(file) => {
                    if !file.readable {
                        summary.unreadable += 1;
                        continue;
                    }
                    summary.files_searched += 1;
                    let sure = file
                        .usages
                        .iter()
                        .filter(|u| u.certainty == UsageCertainty::Sure)
                        .count();
                    let possible = file.usages.len() - sure;
                    summary.sure += sure;
                    summary.possible += possible;
                    summary.files_with_sure += (sure > 0) as usize;
                    summary.files_with_possible += (possible > 0) as usize;
                    pending.extend(file.usages);
                    if !pending.is_empty()
                        && (!sent_first || last.elapsed() >= Duration::from_millis(50))
                    {
                        sink(std::mem::take(&mut pending));
                        sent_first = true;
                        last = Instant::now();
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if !pending.is_empty() {
                        sink(std::mem::take(&mut pending));
                        sent_first = true;
                        last = Instant::now();
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
            if cancel.is_cancelled() {
                break;
            }
        }
        if !pending.is_empty() && !cancel.is_cancelled() {
            sink(pending);
        }
    });
    cancel.check()?;
    summary.elapsed = started.elapsed();
    Ok(summary)
}
