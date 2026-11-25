//! Extraction des déclarations et des références d'un fichier PHP à partir de
//! l'arbre tree-sitter (grammaire `tree-sitter-php`, PHP mêlé de HTML).
//!
//! Les noms de classes, de fonctions et de constantes sont résolus ici, avec
//! l'espace de noms et les `use` du fichier (règles de PHP). Les receveurs des
//! accès membres sont décrits ([`Recv`]) et résolus plus tard avec l'index.

use std::collections::{HashMap, HashSet};

use tree_sitter::{Node, Tree};

use super::model::{
    flags, node_text, short_key, Analysis, BaseRel, ClassContext, Columns, Decl, FileSymbols, Lang,
    Local, MemberKind, Recv, Ref, RefKind, Span,
};
use super::SymbolKind;

/// Profondeur maximale d'une chaîne de receveurs (`$a->b()->c()->d()`).
const MAX_CHAIN: usize = 4;

/// declarations_only : pour l'index (déclarations et clés des noms) ; sans
/// les types des receveurs, les variables locales ni le relevé des affectations,
/// inutiles hors navigation.
pub(crate) fn analyze(
    text: &str,
    tree: &Tree,
    content_hash: u64,
    declarations_only: bool,
) -> Analysis {
    let mut x = Extractor {
        text,
        cols: Columns::new(text),
        ns: String::new(),
        uses_class: HashMap::new(),
        uses_fn: HashMap::new(),
        uses_const: HashMap::new(),
        classes: Vec::new(),
        scopes: Vec::new(),
        next_scope: 0,
        decls: Vec::new(),
        refs: Vec::new(),
        locals: Vec::new(),
        namespaces: vec![(0, Box::from(""))],
        depth: 0,
        parents: Vec::new(),
        declarations_only,
    };
    let root = tree.root_node();
    x.enter_scope(root, None, false, &[]);
    x.visit_children(root);
    x.scopes.pop();

    let mut keys: Vec<u32> = Vec::with_capacity(x.refs.len());
    for r in &x.refs {
        match &r.kind {
            RefKind::Class { fqn, keyword, .. } => {
                if !keyword {
                    keys.push(short_key("c", fqn, true));
                }
            }
            RefKind::Function { candidates } => {
                keys.extend(candidates.iter().map(|c| short_key("f", c, true)))
            }
            RefKind::Constant { candidates } => keys.extend(
                candidates
                    .iter()
                    .map(|c| short_key("k", &super::model::php_const_key(c), false)),
            ),
            RefKind::Member { member, .. } => keys.push(short_key(member.tag(), &r.name, true)),
            _ => {}
        }
    }
    keys.sort_unstable();
    keys.dedup();
    Analysis {
        symbols: FileSymbols {
            lang: Lang::Php,
            content_hash,
            decls: x.decls,
            ref_keys: keys.into_boxed_slice(),
            js: None,
        },
        refs: x.refs,
        locals: x.locals,
        namespaces: x.namespaces,
        scopes: Vec::new(),
    }
}

/// Étiquette de documentation : (nom, types résolus, variable).
type DocTag = (String, Vec<Box<str>>, Option<String>);

struct ClassCtx {
    /// Nom complet ; `None` pour une classe anonyme.
    fqn: Option<String>,
    parent: Option<String>,
    decl: Option<usize>,
}

#[derive(Default)]
struct VarFacts<'t> {
    param_types: Option<Vec<Box<str>>>,
    doc: Option<Vec<Box<str>>>,
    assigns: Vec<Node<'t>>,
    poisoned: bool,
}

struct Scope<'t> {
    id: u32,
    /// Fonction fléchée : voit les variables de la portée parente.
    inherit: bool,
    facts: HashMap<String, VarFacts<'t>>,
    resolved: HashMap<String, Recv>,
    in_progress: HashSet<String>,
    locals: HashMap<String, u32>,
}

struct Extractor<'a, 't> {
    text: &'a str,
    cols: Columns<'a>,
    ns: String,
    /// alias en minuscules → nom complet
    uses_class: HashMap<String, String>,
    uses_fn: HashMap<String, String>,
    /// alias exact → nom complet
    uses_const: HashMap<String, String>,
    classes: Vec<ClassCtx>,
    scopes: Vec<Scope<'t>>,
    next_scope: u32,
    decls: Vec<Decl>,
    refs: Vec<Ref>,
    locals: Vec<Local>,
    namespaces: Vec<(u32, Box<str>)>,
    depth: usize,
    parents: Vec<Node<'t>>,
    declarations_only: bool,
}

fn is_scalar_type(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "int"
            | "integer"
            | "float"
            | "double"
            | "string"
            | "bool"
            | "boolean"
            | "array"
            | "object"
            | "mixed"
            | "void"
            | "null"
            | "callable"
            | "iterable"
            | "resource"
            | "true"
            | "false"
            | "never"
            | "list"
            | "scalar"
            | "numeric"
            | "number"
            | "callback"
            | "$this"
            | "this"
    )
}

impl<'a, 't> Extractor<'a, 't> {
    fn t(&self, node: Node) -> &'a str {
        node_text(self.text, node)
    }

    fn span(&mut self, node: Node) -> Span {
        self.cols.span(node)
    }

    fn current_class(&self) -> Option<&str> {
        self.classes.last().and_then(|c| c.fqn.as_deref())
    }

    fn current_parent(&self) -> Option<&str> {
        self.classes.last().and_then(|c| c.parent.as_deref())
    }

    fn qualify(&self, name: &str) -> String {
        if self.ns.is_empty() {
            name.to_string()
        } else {
            format!("{}\\{}", self.ns, name)
        }
    }

    // --- Résolution des noms (règles de PHP) --------------------------------

    /// Nom de classe tel qu'écrit → nom complet. `None` pour `self`/`static`/
    /// `parent` hors contexte.
    fn resolve_class(&self, raw: &str) -> Option<(String, bool)> {
        let raw = raw.trim();
        let lower = raw.to_ascii_lowercase();
        match lower.as_str() {
            "self" | "static" => return self.current_class().map(|c| (c.to_string(), true)),
            "parent" => return self.current_parent().map(|c| (c.to_string(), true)),
            _ => {}
        }
        if let Some(rest) = raw.strip_prefix('\\') {
            return Some((rest.to_string(), false));
        }
        if lower.starts_with("namespace\\") {
            return Some((self.qualify(&raw["namespace\\".len()..]), false));
        }
        match raw.split_once('\\') {
            Some((first, rest)) => match self.uses_class.get(&first.to_ascii_lowercase()) {
                Some(full) => Some((format!("{full}\\{rest}"), false)),
                None => Some((self.qualify(raw), false)),
            },
            None => match self.uses_class.get(&lower) {
                Some(full) => Some((full.clone(), false)),
                None => Some((self.qualify(raw), false)),
            },
        }
    }

    /// Candidats d'un appel de fonction (ou d'une constante si `constant`).
    fn resolve_function(&self, raw: &str, constant: bool) -> Vec<Box<str>> {
        let raw = raw.trim();
        if let Some(rest) = raw.strip_prefix('\\') {
            return vec![rest.into()];
        }
        if raw.to_ascii_lowercase().starts_with("namespace\\") {
            return vec![self.qualify(&raw["namespace\\".len()..]).into()];
        }
        if let Some((first, rest)) = raw.split_once('\\') {
            return match self.uses_class.get(&first.to_ascii_lowercase()) {
                Some(full) => vec![format!("{full}\\{rest}").into()],
                None => vec![self.qualify(raw).into()],
            };
        }
        let imported = if constant {
            self.uses_const.get(raw)
        } else {
            self.uses_fn.get(&raw.to_ascii_lowercase())
        };
        if let Some(full) = imported {
            return vec![full.as_str().into()];
        }
        if self.ns.is_empty() {
            vec![raw.into()]
        } else {
            vec![self.qualify(raw).into(), raw.into()]
        }
    }

    // --- Portées -------------------------------------------------------------

    /// Ouvre une portée ; `params` : (nom avec `$`, types, nœud du nom).
    fn enter_scope(
        &mut self,
        body: Node<'t>,
        params_node: Option<Node<'t>>,
        inherit: bool,
        doc_params: &[(String, Vec<Box<str>>)],
    ) {
        let id = self.next_scope;
        self.next_scope += 1;
        let mut scope = Scope {
            id,
            inherit,
            facts: HashMap::new(),
            resolved: HashMap::new(),
            in_progress: HashSet::new(),
            locals: HashMap::new(),
        };
        let mut facts: HashMap<String, VarFacts<'t>> = HashMap::new();
        if !self.declarations_only {
            prescan(self.text, body, &mut facts, true);
        }
        for (name, types) in doc_params {
            facts.entry(name.clone()).or_default().doc = Some(types.clone());
        }
        scope.facts = facts;
        self.scopes.push(scope);
        if let Some(params) = params_node {
            self.declare_params(params);
        }
    }

    fn declare_params(&mut self, params: Node<'t>) {
        let mut cursor = params.walk();
        let children: Vec<Node<'t>> = params.named_children(&mut cursor).collect();
        for p in children {
            if !matches!(
                p.kind(),
                "simple_parameter" | "variadic_parameter" | "property_promotion_parameter"
            ) {
                continue;
            }
            if let Some(attrs) = p.child_by_field_name("attributes") {
                self.visit(attrs);
            }
            let types = p
                .child_by_field_name("type")
                .map(|t| self.visit_type(t))
                .unwrap_or_default();
            let Some(mut name_node) = p.child_by_field_name("name") else {
                continue;
            };
            if name_node.kind() == "by_ref" {
                match name_node.named_child(0) {
                    Some(inner) => name_node = inner,
                    None => continue,
                }
            }
            if name_node.kind() != "variable_name" {
                continue;
            }
            let name = self.t(name_node).to_string();
            let span = self.span(name_node);
            let container = self.current_class().unwrap_or("").into();
            let local = self.locals.len() as u32;
            self.locals.push(Local {
                name: name.as_str().into(),
                kind: SymbolKind::Parameter,
                span,
                scope: self.scopes.last().map_or(0, |s| s.id),
                init_new: None,
                container,
            });
            self.refs.push(Ref {
                span,
                name: name.as_str().into(),
                kind: RefKind::Local { local },
            });
            if let Some(scope) = self.scopes.last_mut() {
                scope.locals.insert(name.clone(), local);
                if !types.is_empty() {
                    scope.facts.entry(name.clone()).or_default().param_types = Some(types.clone());
                }
            }
            if p.kind() == "property_promotion_parameter" {
                if let (Some(class), Some(decl_name)) = (
                    self.current_class().map(str::to_string),
                    name_node.named_child(0),
                ) {
                    let span = self.span(decl_name);
                    self.decls.push(Decl {
                        kind: SymbolKind::Property,
                        flags: flags::PROMOTED,
                        name: name.trim_start_matches('$').into(),
                        container: class.into(),
                        span,
                        types: types.clone().into_boxed_slice(),
                        bases: Box::new([]),
                    });
                }
            }
            if let Some(default) = p.child_by_field_name("default_value") {
                self.visit(default);
            }
        }
    }

    /// Variable `$nom` dans la portée courante : référence (et déclaration à
    /// la première occurrence).
    fn variable(&mut self, node: Node<'t>) {
        if self.declarations_only {
            return;
        }
        let name = self.t(node);
        if name == "$this" || !name.starts_with('$') {
            return;
        }
        let span = self.span(node);
        let local = self.find_local(name).unwrap_or_else(|| {
            let local = self.locals.len() as u32;
            let container = self.current_class().unwrap_or("").into();
            self.locals.push(Local {
                name: name.into(),
                kind: SymbolKind::Variable,
                span,
                scope: self.scopes.last().map_or(0, |s| s.id),
                init_new: None,
                container,
            });
            if let Some(scope) = self.scopes.last_mut() {
                scope.locals.insert(name.to_string(), local);
            }
            local
        });
        self.refs.push(Ref {
            span,
            name: name.into(),
            kind: RefKind::Local { local },
        });
    }

    fn find_local(&self, name: &str) -> Option<u32> {
        for scope in self.scopes.iter().rev() {
            if let Some(&l) = scope.locals.get(name) {
                return Some(l);
            }
            if !scope.inherit {
                break;
            }
        }
        None
    }

    /// Type d'une variable de la portée courante (ou des parentes pour une
    /// fonction fléchée).
    fn var_recv(&mut self, name: &str, depth: usize) -> Recv {
        let mut index = self.scopes.len();
        while index > 0 {
            index -= 1;
            if self.scopes[index].facts.contains_key(name) {
                return self.var_recv_in(index, name, depth);
            }
            if !self.scopes[index].inherit {
                break;
            }
        }
        Recv::Unknown
    }

    fn var_recv_in(&mut self, index: usize, name: &str, depth: usize) -> Recv {
        if let Some(r) = self.scopes[index].resolved.get(name) {
            return r.clone();
        }
        if depth > MAX_CHAIN || !self.scopes[index].in_progress.insert(name.to_string()) {
            return Recv::Unknown;
        }
        let (doc, param, assigns, poisoned) = {
            let f = &self.scopes[index].facts[name];
            (
                f.doc.clone(),
                f.param_types.clone(),
                f.assigns.clone(),
                f.poisoned,
            )
        };
        let result = if let Some(doc) = doc.filter(|d| !d.is_empty()) {
            Recv::Classes(doc)
        } else if poisoned {
            Recv::Unknown
        } else {
            let mut seen: Option<Recv> = param.filter(|p| !p.is_empty()).map(Recv::Classes);
            let mut ok = seen.is_some() || !assigns.is_empty();
            for rhs in assigns {
                let r = self.recv(rhs, depth + 1);
                if r == Recv::Unknown {
                    ok = false;
                    break;
                }
                match &seen {
                    None => seen = Some(r),
                    Some(s) if *s == r => {}
                    Some(_) => {
                        ok = false;
                        break;
                    }
                }
            }
            if ok {
                seen.unwrap_or(Recv::Unknown)
            } else {
                Recv::Unknown
            }
        };
        let scope = &mut self.scopes[index];
        scope.in_progress.remove(name);
        scope.resolved.insert(name.to_string(), result.clone());
        result
    }

    /// Type d'une expression receveur, sans émettre de référence.
    fn recv(&mut self, node: Node<'t>, depth: usize) -> Recv {
        if depth > MAX_CHAIN {
            return Recv::Unknown;
        }
        match node.kind() {
            "variable_name" => {
                let name = self.t(node);
                if name == "$this" {
                    match self.current_class() {
                        Some(c) => Recv::Classes(vec![c.into()]),
                        None => Recv::Unknown,
                    }
                } else {
                    self.var_recv(name, depth)
                }
            }
            "member_access_expression" | "nullsafe_member_access_expression" => {
                match (node.child_by_field_name("object"), member_name(node)) {
                    (Some(object), Some(name)) if name.kind() == "name" => {
                        let inner = self.recv(object, depth + 1);
                        if inner == Recv::Unknown {
                            Recv::Unknown
                        } else {
                            Recv::Prop(Box::new(inner), self.t(name).into())
                        }
                    }
                    _ => Recv::Unknown,
                }
            }
            "member_call_expression" | "nullsafe_member_call_expression" => {
                match (node.child_by_field_name("object"), member_name(node)) {
                    (Some(object), Some(name)) if name.kind() == "name" => {
                        let inner = self.recv(object, depth + 1);
                        if inner == Recv::Unknown {
                            Recv::Unknown
                        } else {
                            Recv::Call(Box::new(inner), self.t(name).into())
                        }
                    }
                    _ => Recv::Unknown,
                }
            }
            "scoped_call_expression" => {
                match (node.child_by_field_name("scope"), member_name(node)) {
                    (Some(scope), Some(name)) if name.kind() == "name" => {
                        let inner = self.scope_recv(scope, depth + 1);
                        if inner == Recv::Unknown {
                            Recv::Unknown
                        } else {
                            Recv::Call(Box::new(inner), self.t(name).into())
                        }
                    }
                    _ => Recv::Unknown,
                }
            }
            "scoped_property_access_expression" => {
                match (
                    node.child_by_field_name("scope"),
                    node.child_by_field_name("name"),
                ) {
                    (Some(scope), Some(name)) if name.kind() == "variable_name" => {
                        let inner = self.scope_recv(scope, depth + 1);
                        if inner == Recv::Unknown {
                            Recv::Unknown
                        } else {
                            Recv::Prop(Box::new(inner), self.t(name).trim_start_matches('$').into())
                        }
                    }
                    _ => Recv::Unknown,
                }
            }
            "object_creation_expression" => match node.named_child(0) {
                Some(n) if matches!(n.kind(), "name" | "qualified_name" | "relative_name") => {
                    match self.resolve_class(self.t(n)) {
                        Some((fqn, _)) => Recv::Classes(vec![fqn.into()]),
                        None => Recv::Unknown,
                    }
                }
                _ => Recv::Unknown,
            },
            "function_call_expression" => match node.child_by_field_name("function") {
                Some(f) if matches!(f.kind(), "name" | "qualified_name" | "relative_name") => {
                    Recv::Func(self.resolve_function(self.t(f), false).into_boxed_slice())
                }
                _ => Recv::Unknown,
            },
            "parenthesized_expression" | "clone_expression" => match node.named_child(0) {
                Some(inner) => self.recv(inner, depth),
                None => Recv::Unknown,
            },
            "assignment_expression" => match node.child_by_field_name("right") {
                Some(right) => self.recv(right, depth + 1),
                None => Recv::Unknown,
            },
            _ => Recv::Unknown,
        }
    }

    /// Receveur d'un `X::` (`self`, `static`, `parent`, nom, expression).
    fn scope_recv(&mut self, scope: Node<'t>, depth: usize) -> Recv {
        match scope.kind() {
            "relative_scope" | "name" | "qualified_name" | "relative_name" => {
                match self.resolve_class(self.t(scope)) {
                    Some((fqn, _)) => Recv::Classes(vec![fqn.into()]),
                    None => Recv::Unknown,
                }
            }
            _ => self.recv(scope, depth),
        }
    }

    // --- Références ------------------------------------------------------------

    fn class_ref(&mut self, node: Node<'t>, context: ClassContext) -> Option<String> {
        let raw = self.t(node);
        let (fqn, keyword) = self.resolve_class(raw)?;
        let span = self.span(node);
        let name = raw.rsplit('\\').next().unwrap_or(raw);
        self.refs.push(Ref {
            span,
            name: name.into(),
            kind: RefKind::Class {
                fqn: fqn.as_str().into(),
                keyword,
                context,
            },
        });
        Some(fqn)
    }

    fn member_ref(&mut self, name: Node<'t>, member: MemberKind, recv: Recv, recv_node: Node<'t>) {
        let span = self.span(name);
        let text = self.t(name).trim_start_matches('$');
        let recv_text: Box<str> = if recv == Recv::Unknown && !self.declarations_only {
            let t = self.t(recv_node);
            if t.chars().count() > 40 {
                let cut: String = t.chars().take(40).collect();
                format!("{cut}…").into()
            } else {
                t.into()
            }
        } else {
            "".into()
        };
        self.refs.push(Ref {
            span,
            name: text.into(),
            kind: RefKind::Member {
                member,
                recv,
                recv_text,
            },
        });
    }

    /// Un type (`?User`, `User|Admin`…) : références aux classes, et noms
    /// complets rendus (`static` pour le type tardif).
    fn visit_type(&mut self, node: Node<'t>) -> Vec<Box<str>> {
        let mut out = Vec::new();
        self.collect_type(node, &mut out);
        out
    }

    fn collect_type(&mut self, node: Node<'t>, out: &mut Vec<Box<str>>) {
        match node.kind() {
            "named_type" => {
                if let Some(n) = node.named_child(0) {
                    self.collect_type(n, out);
                }
            }
            "name" | "qualified_name" | "relative_name" => {
                let raw = self.t(node);
                if raw.eq_ignore_ascii_case("static") {
                    self.class_ref(node, ClassContext::Type);
                    out.push("static".into());
                } else if let Some(fqn) = self.class_ref(node, ClassContext::Type) {
                    out.push(fqn.into());
                }
            }
            "primitive_type" | "bottom_type" => {}
            _ => {
                let mut cursor = node.walk();
                let children: Vec<Node<'t>> = node.named_children(&mut cursor).collect();
                for c in children {
                    self.collect_type(c, out);
                }
            }
        }
    }

    // --- Commentaires de documentation -----------------------------------------

    /// Références aux types d'un commentaire `/** … */` ; rend les étiquettes
    /// (`param`, `var`, `return`…) avec leurs types résolus et leur variable.
    fn doc_comment(&mut self, node: Node<'t>) -> Vec<DocTag> {
        let text = self.t(node);
        if !text.starts_with("/**") {
            return Vec::new();
        }
        let base = node.start_byte();
        let start_pos = node.start_position();
        let mut tags = Vec::new();
        let bytes = text.as_bytes();
        let mut i = 0;
        while let Some(at) = memchr::memchr(b'@', &bytes[i..]) {
            let at = i + at;
            i = at + 1;
            let tag_end = bytes[i..]
                .iter()
                .position(|b| !(b.is_ascii_alphanumeric() || *b == b'-'))
                .map_or(bytes.len(), |p| i + p);
            let tag = &text[i..tag_end];
            if !matches!(
                tag,
                "param"
                    | "var"
                    | "return"
                    | "throws"
                    | "property"
                    | "property-read"
                    | "property-write"
                    | "method"
                    | "mixin"
                    | "see"
            ) {
                continue;
            }
            let mut j = tag_end;
            while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'\t') {
                j += 1;
            }
            let type_start = j;
            while j < bytes.len() && !bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            let type_text = &text[type_start..j];
            let mut k = j;
            while k < bytes.len() && (bytes[k] == b' ' || bytes[k] == b'\t') {
                k += 1;
            }
            let var = if k < bytes.len() && bytes[k] == b'$' {
                let end = bytes[k + 1..]
                    .iter()
                    .position(|b| !(b.is_ascii_alphanumeric() || *b == b'_'))
                    .map_or(bytes.len(), |p| k + 1 + p);
                Some(text[k..end].to_string())
            } else {
                None
            };
            let mut types = Vec::new();
            let mut part_start = None;
            for (p, c) in type_text
                .char_indices()
                .chain(std::iter::once((type_text.len(), ' ')))
            {
                let ident = c.is_ascii_alphanumeric() || c == '_' || c == '\\' || c == '-';
                match (ident, part_start) {
                    (true, None) => part_start = Some(p),
                    (false, Some(s)) => {
                        let part = &type_text[s..p];
                        part_start = None;
                        if part.contains('-')
                            || part.is_empty()
                            || part.as_bytes()[0].is_ascii_digit()
                            || is_scalar_type(part)
                        {
                            continue;
                        }
                        if part.eq_ignore_ascii_case("static") {
                            types.push("static".into());
                            continue;
                        }
                        let Some((fqn, keyword)) = self.resolve_class(part) else {
                            continue;
                        };
                        let offset = type_start + s;
                        let before = &text[..offset];
                        let newlines = before.matches('\n').count();
                        let row = start_pos.row + newlines;
                        let byte_col = match before.rfind('\n') {
                            Some(nl) => offset - nl - 1,
                            None => start_pos.column + offset,
                        };
                        let abs = base + offset;
                        let col = self.cols.col(row, abs, byte_col);
                        let len = part.chars().count();
                        self.refs.push(Ref {
                            span: Span {
                                start: abs as u32,
                                end: (abs + part.len()) as u32,
                                line: row as u32,
                                col: col as u32,
                                end_col: (col + len) as u32,
                            },
                            name: part.rsplit('\\').next().unwrap_or(part).into(),
                            kind: RefKind::Class {
                                fqn: fqn.as_str().into(),
                                keyword,
                                context: ClassContext::Doc,
                            },
                        });
                        types.push(fqn.into());
                    }
                    _ => {}
                }
            }
            tags.push((tag.to_string(), types, var));
        }
        tags
    }

    /// Commentaire de documentation placé juste avant `node`.
    fn doc_before(&mut self, node: Node<'t>) -> Vec<DocTag> {
        let mut prev = node.prev_named_sibling();
        while let Some(p) = prev {
            if p.kind() == "attribute_list" {
                prev = p.prev_named_sibling();
                continue;
            }
            if p.kind() == "comment" && self.t(p).starts_with("/**") {
                // Déjà émis par le parcours général : on ne relit que les types.
                let refs_before = self.refs.len();
                let tags = self.doc_comment(p);
                self.refs.truncate(refs_before);
                return tags;
            }
            break;
        }
        Vec::new()
    }

    // --- Parcours ----------------------------------------------------------------

    fn visit_children(&mut self, node: Node<'t>) {
        let mut cursor = node.walk();
        let children: Vec<Node<'t>> = node.named_children(&mut cursor).collect();
        self.parents.push(node);
        for child in children {
            self.visit(child);
        }
        self.parents.pop();
    }

    /// Parent d'un nœud atteint par le parcours général (`Node::parent` de
    /// tree-sitter repart de la racine : trop lent nœud par nœud). Un nœud visité
    /// directement par un traitement particulier a pour « parent » le dernier
    /// nœud parcouru en général qui le contient.
    fn parent_of(&self, node: Node<'t>) -> Option<Node<'t>> {
        self.parents
            .last()
            .copied()
            .filter(|p| p.start_byte() <= node.start_byte() && node.end_byte() <= p.end_byte())
    }

    fn visit(&mut self, node: Node<'t>) {
        if self.depth > super::store::MAX_DEPTH {
            return;
        }
        self.depth += 1;
        self.visit_inner(node);
        self.depth -= 1;
    }

    fn visit_inner(&mut self, node: Node<'t>) {
        match node.kind() {
            "namespace_definition" => self.namespace(node),
            "namespace_use_declaration" => self.use_declaration(node),
            "class_declaration"
            | "interface_declaration"
            | "trait_declaration"
            | "enum_declaration" => self.class_like(node),
            "anonymous_class" => self.anonymous_class(node),
            "use_declaration" => self.trait_use(node),
            "method_declaration" => self.method(node),
            "function_definition" => self.function(node),
            "anonymous_function" => self.closure(node, false),
            "arrow_function" => self.closure(node, true),
            "property_declaration" => self.property(node),
            "const_declaration" => self.constants(node),
            "enum_case" => self.enum_case(node),
            "function_call_expression" => self.function_call(node),
            "member_call_expression" | "nullsafe_member_call_expression" => {
                self.member_access(node, MemberKind::Method)
            }
            "member_access_expression" | "nullsafe_member_access_expression" => {
                self.member_access(node, MemberKind::Property)
            }
            "scoped_call_expression" => self.scoped(node, MemberKind::Method),
            "scoped_property_access_expression" => self.scoped(node, MemberKind::Property),
            "class_constant_access_expression" => self.class_constant(node),
            "object_creation_expression" => self.creation(node),
            "binary_expression" => self.binary(node),
            "catch_clause" => self.catch(node),
            "named_type"
            | "optional_type"
            | "union_type"
            | "intersection_type"
            | "disjunctive_normal_form_type" => {
                self.visit_type(node);
            }
            "attribute" => {
                if let Some(n) = node.named_child(0) {
                    if matches!(n.kind(), "name" | "qualified_name" | "relative_name") {
                        self.class_ref(n, ClassContext::Attribute);
                    }
                }
                if let Some(params) = node.child_by_field_name("parameters") {
                    self.visit(params);
                }
            }
            "variable_name" => self.variable(node),
            "comment" => {
                // `/** @var Type $v */` dans un corps : type de la variable.
                let tags = self.doc_comment(node);
                for (tag, types, var) in tags {
                    if let (true, Some(var), false) = (tag == "var", var, types.is_empty()) {
                        if let Some(scope) = self.scopes.last_mut() {
                            scope.resolved.remove(&var);
                            scope.facts.entry(var).or_default().doc = Some(types);
                        }
                    }
                }
            }
            "name" | "qualified_name" | "relative_name" => self.bare_name(node),
            // Ni nom ni code dans une chaîne simple.
            "string" | "string_content" | "nowdoc" | "text" | "text_interpolation" => {
                if node.kind() == "text_interpolation" {
                    self.visit_children(node);
                }
            }
            _ => self.visit_children(node),
        }
    }

    fn namespace(&mut self, node: Node<'t>) {
        let name = node
            .child_by_field_name("name")
            .map(|n| (self.t(n).to_string(), n));
        let body = node.child_by_field_name("body");
        let ns = name.as_ref().map(|(n, _)| n.clone()).unwrap_or_default();
        if let Some((full, n)) = &name {
            let span = self.span(*n);
            self.decls.push(Decl {
                kind: SymbolKind::Namespace,
                flags: 0,
                name: full.as_str().into(),
                container: "".into(),
                span,
                types: Box::new([]),
                bases: Box::new([]),
            });
        }
        match body {
            Some(body) => {
                let saved = (
                    std::mem::replace(&mut self.ns, ns.clone()),
                    std::mem::take(&mut self.uses_class),
                    std::mem::take(&mut self.uses_fn),
                    std::mem::take(&mut self.uses_const),
                );
                self.namespaces
                    .push((node.start_byte() as u32, ns.as_str().into()));
                self.visit_children(body);
                self.ns = saved.0;
                self.uses_class = saved.1;
                self.uses_fn = saved.2;
                self.uses_const = saved.3;
                self.namespaces
                    .push((node.end_byte() as u32, self.ns.as_str().into()));
            }
            None => {
                self.ns = ns.clone();
                self.uses_class.clear();
                self.uses_fn.clear();
                self.uses_const.clear();
                self.namespaces
                    .push((node.start_byte() as u32, ns.as_str().into()));
            }
        }
    }

    fn use_kind(&self, node: Node<'t>) -> Option<&'static str> {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "function" => return Some("function"),
                "const" => return Some("const"),
                _ => {}
            }
        }
        None
    }

    fn use_declaration(&mut self, node: Node<'t>) {
        let decl_kind = self.use_kind(node);
        let mut cursor = node.walk();
        let children: Vec<Node<'t>> = node.named_children(&mut cursor).collect();
        let mut prefix: Option<String> = None;
        for child in children {
            match child.kind() {
                "namespace_name" => prefix = Some(self.t(child).trim_start_matches('\\').into()),
                "namespace_use_clause" => self.use_clause(child, None, decl_kind),
                "namespace_use_group" => {
                    let mut c = child.walk();
                    let clauses: Vec<Node<'t>> = child.named_children(&mut c).collect();
                    for clause in clauses {
                        if clause.kind() == "namespace_use_clause" {
                            self.use_clause(clause, prefix.as_deref(), decl_kind);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn use_clause(&mut self, clause: Node<'t>, prefix: Option<&str>, decl_kind: Option<&str>) {
        let kind = self.use_kind(clause).or(decl_kind);
        let mut cursor = clause.walk();
        let name_node = clause
            .named_children(&mut cursor)
            .find(|n| matches!(n.kind(), "name" | "qualified_name"));
        let Some(name_node) = name_node else {
            return;
        };
        let written = self.t(name_node).trim_start_matches('\\');
        let full = match prefix {
            Some(p) => format!("{p}\\{written}"),
            None => written.to_string(),
        };
        let alias = clause
            .child_by_field_name("alias")
            .map(|a| self.t(a).to_string())
            .unwrap_or_else(|| full.rsplit('\\').next().unwrap_or(&full).to_string());
        let span = self.span(name_node);
        let short: Box<str> = full.rsplit('\\').next().unwrap_or(&full).into();
        match kind {
            Some("function") => {
                self.uses_fn
                    .insert(alias.to_ascii_lowercase(), full.clone());
                self.refs.push(Ref {
                    span,
                    name: short,
                    kind: RefKind::Function {
                        candidates: vec![full.into()].into_boxed_slice(),
                    },
                });
            }
            Some("const") => {
                self.uses_const.insert(alias, full.clone());
                self.refs.push(Ref {
                    span,
                    name: short,
                    kind: RefKind::Constant {
                        candidates: vec![full.into()].into_boxed_slice(),
                    },
                });
            }
            _ => {
                self.uses_class
                    .insert(alias.to_ascii_lowercase(), full.clone());
                self.refs.push(Ref {
                    span,
                    name: short,
                    kind: RefKind::Class {
                        fqn: full.into(),
                        keyword: false,
                        context: ClassContext::Use,
                    },
                });
            }
        }
    }

    fn class_like(&mut self, node: Node<'t>) {
        let kind = match node.kind() {
            "interface_declaration" => SymbolKind::Interface,
            "trait_declaration" => SymbolKind::Trait,
            "enum_declaration" => SymbolKind::Enum,
            _ => SymbolKind::Class,
        };
        if let Some(attrs) = node.child_by_field_name("attributes") {
            self.visit(attrs);
        }
        let Some(name) = node.child_by_field_name("name") else {
            self.visit_children(node);
            return;
        };
        let short = self.t(name).to_string();
        let fqn = self.qualify(&short);
        let span = self.span(name);
        // Le contexte de la classe est posé avant les clauses pour `self`.
        self.classes.push(ClassCtx {
            fqn: Some(fqn.clone()),
            parent: None,
            decl: None,
        });
        let mut bases: Vec<(BaseRel, Box<str>)> = Vec::new();
        let mut cursor = node.walk();
        let children: Vec<Node<'t>> = node.named_children(&mut cursor).collect();
        for child in &children {
            match child.kind() {
                "base_clause" => {
                    let mut c = child.walk();
                    let names: Vec<Node<'t>> = child.named_children(&mut c).collect();
                    for n in names {
                        if let Some(f) = self.class_ref(n, ClassContext::Extends) {
                            bases.push((BaseRel::Extends, f.into()));
                        }
                    }
                }
                "class_interface_clause" => {
                    let mut c = child.walk();
                    let names: Vec<Node<'t>> = child.named_children(&mut c).collect();
                    for n in names {
                        if let Some(f) = self.class_ref(n, ClassContext::Implements) {
                            bases.push((BaseRel::Implements, f.into()));
                        }
                    }
                }
                _ => {}
            }
        }
        let parent = if kind == SymbolKind::Class {
            bases
                .iter()
                .find(|(r, _)| *r == BaseRel::Extends)
                .map(|(_, f)| f.to_string())
        } else {
            None
        };
        let decl = self.decls.len();
        self.decls.push(Decl {
            kind,
            flags: 0,
            name: short.into(),
            container: self.ns.as_str().into(),
            span,
            types: Box::new([]),
            bases: bases.into_boxed_slice(),
        });
        if let Some(ctx) = self.classes.last_mut() {
            ctx.parent = parent;
            ctx.decl = Some(decl);
        }
        if let Some(body) = node.child_by_field_name("body") {
            self.visit_children(body);
        }
        self.classes.pop();
    }

    fn anonymous_class(&mut self, node: Node<'t>) {
        let mut parent = None;
        let mut cursor = node.walk();
        let children: Vec<Node<'t>> = node.named_children(&mut cursor).collect();
        for child in &children {
            match child.kind() {
                "base_clause" | "class_interface_clause" => {
                    let context = if child.kind() == "base_clause" {
                        ClassContext::Extends
                    } else {
                        ClassContext::Implements
                    };
                    let mut c = child.walk();
                    let names: Vec<Node<'t>> = child.named_children(&mut c).collect();
                    for n in names {
                        let f = self.class_ref(n, context);
                        if context == ClassContext::Extends && parent.is_none() {
                            parent = f;
                        }
                    }
                }
                "arguments" | "attribute_list" => self.visit(*child),
                _ => {}
            }
        }
        self.classes.push(ClassCtx {
            fqn: None,
            parent,
            decl: None,
        });
        if let Some(body) = node.child_by_field_name("body") {
            self.visit_children(body);
        }
        self.classes.pop();
    }

    fn trait_use(&mut self, node: Node<'t>) {
        let mut cursor = node.walk();
        let children: Vec<Node<'t>> = node.named_children(&mut cursor).collect();
        for child in children {
            if matches!(child.kind(), "name" | "qualified_name" | "relative_name") {
                if let Some(f) = self.class_ref(child, ClassContext::TraitUse) {
                    if let Some(decl) = self.classes.last().and_then(|c| c.decl) {
                        let d = &mut self.decls[decl];
                        let mut bases = std::mem::take(&mut d.bases).into_vec();
                        bases.push((BaseRel::Uses, f.into()));
                        d.bases = bases.into_boxed_slice();
                    }
                }
            } else {
                self.visit(child);
            }
        }
    }

    fn is_static(&self, node: Node<'t>) -> bool {
        let mut cursor = node.walk();
        let found = node
            .children(&mut cursor)
            .any(|c| c.kind() == "static_modifier");
        found
    }

    /// Types d'un `@return`/`@var` de documentation, à défaut du type déclaré.
    fn doc_types(tags: &[DocTag], tag: &str) -> Vec<Box<str>> {
        tags.iter()
            .find(|(t, types, _)| t == tag && !types.is_empty())
            .map(|(_, types, _)| types.clone())
            .unwrap_or_default()
    }

    fn doc_params(tags: &[DocTag]) -> Vec<(String, Vec<Box<str>>)> {
        tags.iter()
            .filter(|(t, types, var)| t == "param" && !types.is_empty() && var.is_some())
            .map(|(_, types, var)| (var.clone().unwrap_or_default(), types.clone()))
            .collect()
    }

    fn method(&mut self, node: Node<'t>) {
        let tags = self.doc_before(node);
        if let Some(attrs) = node.child_by_field_name("attributes") {
            self.visit(attrs);
        }
        let mut types = node
            .child_by_field_name("return_type")
            .map(|t| self.visit_type(t))
            .unwrap_or_default();
        if types.is_empty() {
            types = Self::doc_types(&tags, "return");
        }
        if let (Some(name), Some(class)) = (
            node.child_by_field_name("name"),
            self.current_class().map(str::to_string),
        ) {
            let span = self.span(name);
            let flags = if self.is_static(node) {
                flags::STATIC
            } else {
                0
            };
            self.decls.push(Decl {
                kind: SymbolKind::Method,
                flags,
                name: self.t(name).into(),
                container: class.into(),
                span,
                types: types.into_boxed_slice(),
                bases: Box::new([]),
            });
        }
        self.function_body(node, &tags, false);
    }

    fn function(&mut self, node: Node<'t>) {
        let tags = self.doc_before(node);
        if let Some(attrs) = node.child_by_field_name("attributes") {
            self.visit(attrs);
        }
        let mut types = node
            .child_by_field_name("return_type")
            .map(|t| self.visit_type(t))
            .unwrap_or_default();
        if types.is_empty() {
            types = Self::doc_types(&tags, "return");
        }
        if let Some(name) = node.child_by_field_name("name") {
            let span = self.span(name);
            self.decls.push(Decl {
                kind: SymbolKind::Function,
                flags: 0,
                name: self.t(name).into(),
                container: self.ns.as_str().into(),
                span,
                types: types.into_boxed_slice(),
                bases: Box::new([]),
            });
        }
        self.function_body(node, &tags, false);
    }

    fn closure(&mut self, node: Node<'t>, arrow: bool) {
        if let Some(attrs) = node.child_by_field_name("attributes") {
            self.visit(attrs);
        }
        if let Some(t) = node.child_by_field_name("return_type") {
            self.visit_type(t);
        }
        // Variables importées par `use (…)` : vues dans la portée englobante.
        let mut cursor = node.walk();
        let children: Vec<Node<'t>> = node.named_children(&mut cursor).collect();
        for c in &children {
            if c.kind() == "anonymous_function_use_clause" {
                self.visit_children(*c);
            }
        }
        self.function_body(node, &[], arrow);
    }

    fn function_body(&mut self, node: Node<'t>, tags: &[DocTag], inherit: bool) {
        let params = node.child_by_field_name("parameters");
        let body = node.child_by_field_name("body");
        let doc_params = Self::doc_params(tags);
        match body {
            Some(body) => {
                self.enter_scope(body, params, inherit, &doc_params);
                self.visit(body);
                self.scopes.pop();
            }
            None => {
                // Méthode abstraite ou d'interface : les types des paramètres
                // restent des usages.
                if let Some(params) = params {
                    self.enter_scope(params, Some(params), inherit, &doc_params);
                    self.scopes.pop();
                }
            }
        }
    }

    fn property(&mut self, node: Node<'t>) {
        let tags = self.doc_before(node);
        if let Some(attrs) = node.child_by_field_name("attributes") {
            self.visit(attrs);
        }
        let mut types = node
            .child_by_field_name("type")
            .map(|t| self.visit_type(t))
            .unwrap_or_default();
        if types.is_empty() {
            types = Self::doc_types(&tags, "var");
        }
        let flags = if self.is_static(node) {
            flags::STATIC
        } else {
            0
        };
        let class = self.current_class().map(str::to_string);
        let mut cursor = node.walk();
        let children: Vec<Node<'t>> = node.named_children(&mut cursor).collect();
        for child in children {
            if child.kind() != "property_element" {
                continue;
            }
            if let (Some(var), Some(class)) = (child.child_by_field_name("name"), &class) {
                let name_node = var.named_child(0).unwrap_or(var);
                let span = self.span(name_node);
                self.decls.push(Decl {
                    kind: SymbolKind::Property,
                    flags,
                    name: self.t(var).trim_start_matches('$').into(),
                    container: class.as_str().into(),
                    span,
                    types: types.clone().into_boxed_slice(),
                    bases: Box::new([]),
                });
            }
            if let Some(default) = child.child_by_field_name("default_value") {
                self.visit(default);
            }
        }
    }

    fn constants(&mut self, node: Node<'t>) {
        let in_class = self
            .parent_of(node)
            .is_some_and(|p| p.kind() == "declaration_list" || p.kind() == "enum_declaration_list");
        let class = if in_class {
            self.current_class().map(str::to_string)
        } else {
            None
        };
        let mut cursor = node.walk();
        let children: Vec<Node<'t>> = node.named_children(&mut cursor).collect();
        for child in children {
            if child.kind() != "const_element" {
                if child.kind() != "visibility_modifier" {
                    self.visit(child);
                }
                continue;
            }
            let mut c = child.walk();
            let parts: Vec<Node<'t>> = child.named_children(&mut c).collect();
            let Some(name) = parts.first().copied() else {
                continue;
            };
            if name.kind() == "name" {
                let span = self.span(name);
                let (kind, container) = match (&class, in_class) {
                    (Some(c), _) => (SymbolKind::ClassConstant, c.clone()),
                    (None, true) => {
                        for p in parts.iter().skip(1) {
                            self.visit(*p);
                        }
                        continue;
                    }
                    (None, false) => (SymbolKind::Constant, self.ns.clone()),
                };
                self.decls.push(Decl {
                    kind,
                    flags: 0,
                    name: self.t(name).into(),
                    container: container.into(),
                    span,
                    types: Box::new([]),
                    bases: Box::new([]),
                });
            }
            for p in parts.iter().skip(1) {
                self.visit(*p);
            }
        }
    }

    fn enum_case(&mut self, node: Node<'t>) {
        if let (Some(name), Some(class)) = (
            node.child_by_field_name("name"),
            self.current_class().map(str::to_string),
        ) {
            let span = self.span(name);
            self.decls.push(Decl {
                kind: SymbolKind::EnumCase,
                flags: 0,
                name: self.t(name).into(),
                container: class.into(),
                span,
                types: Box::new([]),
                bases: Box::new([]),
            });
        }
        if let Some(value) = node.child_by_field_name("value") {
            self.visit(value);
        }
    }

    fn function_call(&mut self, node: Node<'t>) {
        let function = node.child_by_field_name("function");
        let args = node.child_by_field_name("arguments");
        match function {
            Some(f) if matches!(f.kind(), "name" | "qualified_name" | "relative_name") => {
                let raw = self.t(f);
                if raw.eq_ignore_ascii_case("define") || raw.eq_ignore_ascii_case("\\define") {
                    self.define(args);
                }
                let span = self.span(f);
                let candidates = self.resolve_function(raw, false);
                self.refs.push(Ref {
                    span,
                    name: raw.rsplit('\\').next().unwrap_or(raw).into(),
                    kind: RefKind::Function {
                        candidates: candidates.into_boxed_slice(),
                    },
                });
            }
            Some(f) => self.visit(f),
            None => {}
        }
        if let Some(args) = args {
            self.visit(args);
        }
    }

    /// `define('NOM', …)` avec un nom littéral : constante globale.
    fn define(&mut self, args: Option<Node<'t>>) {
        let Some(args) = args else {
            return;
        };
        let Some(first) = args.named_child(0) else {
            return;
        };
        let Some(string) = first.named_child(0) else {
            return;
        };
        if !matches!(string.kind(), "string" | "encapsed_string") {
            return;
        }
        let mut cursor = string.walk();
        let contents: Vec<Node<'t>> = string.named_children(&mut cursor).collect();
        if contents.len() != 1 || contents[0].kind() != "string_content" {
            return;
        }
        let content = contents[0];
        let full = self.t(content).trim_start_matches('\\');
        if full.is_empty()
            || !full
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'\\')
        {
            return;
        }
        let (container, name) = match full.rsplit_once('\\') {
            Some((ns, n)) => (ns.to_string(), n.to_string()),
            None => (String::new(), full.to_string()),
        };
        let span = self.span(content);
        self.decls.push(Decl {
            kind: SymbolKind::Constant,
            flags: 0,
            name: name.into(),
            container: container.into(),
            span,
            types: Box::new([]),
            bases: Box::new([]),
        });
    }

    fn member_access(&mut self, node: Node<'t>, member: MemberKind) {
        let object = node.child_by_field_name("object");
        let name = member_name(node);
        if let (Some(object), Some(name)) = (object, name) {
            if name.kind() == "name" {
                let recv = if self.declarations_only {
                    Recv::Unknown
                } else {
                    self.recv(object, 1)
                };
                self.member_ref(name, member, recv, object);
            } else {
                self.visit(name); // `$obj->$nom` : la variable
            }
        }
        if let Some(object) = object {
            self.visit(object);
        }
        if let Some(args) = node.child_by_field_name("arguments") {
            self.visit(args);
        }
    }

    fn scoped(&mut self, node: Node<'t>, member: MemberKind) {
        let scope = node.child_by_field_name("scope");
        let name = if member == MemberKind::Method {
            member_name(node)
        } else {
            node.child_by_field_name("name")
        };
        if let Some(scope) = scope {
            let recv = if self.declarations_only {
                Recv::Unknown
            } else {
                self.scope_recv(scope, 1)
            };
            if let Some(name) = name {
                let ok = match member {
                    MemberKind::Method => name.kind() == "name",
                    _ => name.kind() == "variable_name",
                };
                if ok {
                    self.member_ref(name, member, recv, scope);
                } else {
                    self.visit(name);
                }
            }
            self.scope_refs(scope);
        }
        if let Some(args) = node.child_by_field_name("arguments") {
            self.visit(args);
        }
    }

    /// Références d'un `X::` : la classe (ou `self`…), ou l'expression.
    fn scope_refs(&mut self, scope: Node<'t>) {
        match scope.kind() {
            "relative_scope" | "name" | "qualified_name" | "relative_name" => {
                self.class_ref(scope, ClassContext::Static);
            }
            _ => self.visit(scope),
        }
    }

    fn class_constant(&mut self, node: Node<'t>) {
        let mut cursor = node.walk();
        let children: Vec<Node<'t>> = node.named_children(&mut cursor).collect();
        let (Some(scope), Some(name)) = (children.first().copied(), children.last().copied())
        else {
            return;
        };
        if children.len() >= 2 && name.kind() == "name" {
            if self.t(name).eq_ignore_ascii_case("class") {
                // `X::class` : la classe seule.
            } else {
                let recv = if self.declarations_only {
                    Recv::Unknown
                } else {
                    self.scope_recv(scope, 1)
                };
                self.member_ref(name, MemberKind::Constant, recv, scope);
            }
        }
        match scope.kind() {
            "relative_scope" | "name" | "qualified_name" | "relative_name" => {
                self.class_ref(scope, ClassContext::ClassConstant);
            }
            _ => self.visit(scope),
        }
    }

    fn creation(&mut self, node: Node<'t>) {
        let mut cursor = node.walk();
        let children: Vec<Node<'t>> = node.named_children(&mut cursor).collect();
        for child in children {
            match child.kind() {
                "name" | "qualified_name" | "relative_name" => {
                    self.class_ref(child, ClassContext::New);
                }
                _ => self.visit(child),
            }
        }
    }

    fn binary(&mut self, node: Node<'t>) {
        let is_instanceof = node
            .child_by_field_name("operator")
            .is_some_and(|op| self.t(op).eq_ignore_ascii_case("instanceof"));
        if let Some(left) = node.child_by_field_name("left") {
            self.visit(left);
        }
        if let Some(right) = node.child_by_field_name("right") {
            if is_instanceof && matches!(right.kind(), "name" | "qualified_name" | "relative_name")
            {
                self.class_ref(right, ClassContext::InstanceOf);
            } else {
                self.visit(right);
            }
        }
    }

    fn catch(&mut self, node: Node<'t>) {
        if let Some(types) = node.child_by_field_name("type") {
            let mut cursor = types.walk();
            let names: Vec<Node<'t>> = types.named_children(&mut cursor).collect();
            for n in names {
                let inner = if n.kind() == "named_type" {
                    n.named_child(0).unwrap_or(n)
                } else {
                    n
                };
                self.class_ref(inner, ClassContext::Catch);
            }
        }
        if let Some(name) = node.child_by_field_name("name") {
            self.variable(name);
        }
        if let Some(body) = node.child_by_field_name("body") {
            self.visit(body);
        }
    }

    /// Un nom isolé dans une expression : constante (`VERSION`, `\App\X`).
    fn bare_name(&mut self, node: Node<'t>) {
        let Some(parent) = self.parent_of(node) else {
            return;
        };
        let constant_context = matches!(
            parent.kind(),
            "argument"
                | "binary_expression"
                | "unary_op_expression"
                | "echo_statement"
                | "return_statement"
                | "assignment_expression"
                | "augmented_assignment_expression"
                | "array_element_initializer"
                | "parenthesized_expression"
                | "conditional_expression"
                | "expression_statement"
                | "subscript_expression"
                | "const_element"
                | "property_element"
                | "simple_parameter"
                | "property_promotion_parameter"
                | "sequence_expression"
                | "print_intrinsic"
                | "match_condition_list"
                | "match_conditional_expression"
                | "case_statement"
                | "if_statement"
                | "while_statement"
                | "cast_expression"
                | "variadic_unpacking"
                | "enum_case"
        );
        if !constant_context {
            return;
        }
        if parent.kind() == "argument"
            && parent
                .child_by_field_name("name")
                .is_some_and(|n| n.id() == node.id())
        {
            return; // argument nommé
        }
        let raw = self.t(node);
        if matches!(raw.to_ascii_lowercase().as_str(), "true" | "false" | "null") {
            return;
        }
        let span = self.span(node);
        let candidates = self.resolve_function(raw, true);
        self.refs.push(Ref {
            span,
            name: raw.rsplit('\\').next().unwrap_or(raw).into(),
            kind: RefKind::Constant {
                candidates: candidates.into_boxed_slice(),
            },
        });
    }
}

/// Nom d'un accès membre (champ `name`).
fn member_name(node: Node) -> Option<Node> {
    node.child_by_field_name("name")
}

/// Relevé des affectations d'une portée, sans descendre dans les fonctions et
/// classes imbriquées.
fn prescan<'t>(text: &str, node: Node<'t>, facts: &mut HashMap<String, VarFacts<'t>>, top: bool) {
    prescan_at(text, node, facts, top, 0)
}

fn prescan_at<'t>(
    text: &str,
    node: Node<'t>,
    facts: &mut HashMap<String, VarFacts<'t>>,
    top: bool,
    depth: usize,
) {
    if depth > super::store::MAX_DEPTH {
        return;
    }
    if !top
        && matches!(
            node.kind(),
            "function_definition"
                | "method_declaration"
                | "anonymous_function"
                | "arrow_function"
                | "class_declaration"
                | "anonymous_class"
                | "interface_declaration"
                | "trait_declaration"
                | "enum_declaration"
        )
    {
        return;
    }
    match node.kind() {
        "assignment_expression" => {
            if let (Some(left), Some(right)) = (
                node.child_by_field_name("left"),
                node.child_by_field_name("right"),
            ) {
                if left.kind() == "variable_name" {
                    facts
                        .entry(node_text(text, left).to_string())
                        .or_default()
                        .assigns
                        .push(right);
                } else {
                    poison_vars(text, left, facts);
                }
            }
        }
        "reference_assignment_expression" | "augmented_assignment_expression" => {
            if let Some(left) = node.child_by_field_name("left") {
                poison_vars(text, left, facts);
            }
        }
        "foreach_statement" => {
            let mut cursor = node.walk();
            let children: Vec<Node<'t>> = node.named_children(&mut cursor).collect();
            // Tout ce qui suit l'expression parcourue, hors corps.
            for c in children.iter().skip(1) {
                if node
                    .child_by_field_name("body")
                    .is_some_and(|b| b.id() == c.id())
                {
                    continue;
                }
                poison_vars(text, *c, facts);
            }
        }
        "global_declaration" | "static_variable_declaration" | "list_literal" => {
            poison_vars(text, node, facts);
        }
        "catch_clause" => {
            if let Some(name) = node.child_by_field_name("name") {
                let mut types = Vec::new();
                if let Some(t) = node.child_by_field_name("type") {
                    let mut cursor = t.walk();
                    for n in t.named_children(&mut cursor) {
                        types.push(node_text(text, n).to_string());
                    }
                }
                // Le type d'une exception attrapée se résout avec les noms du
                // fichier : on la marque inconnue plutôt que de deviner ici.
                let _ = types;
                facts
                    .entry(node_text(text, name).to_string())
                    .or_default()
                    .poisoned = true;
            }
        }
        "comment" => {
            // `/** @var Type $v */` dans le corps : géré par le parcours (les
            // noms ne sont pas résolus ici).
        }
        _ => {}
    }
    let mut cursor = node.walk();
    let children: Vec<Node<'t>> = node.named_children(&mut cursor).collect();
    for c in children {
        prescan_at(text, c, facts, false, depth + 1);
    }
}

fn poison_vars<'t>(text: &str, node: Node<'t>, facts: &mut HashMap<String, VarFacts<'t>>) {
    if node.end_byte() - node.start_byte() > 4096 {
        return; // garde-fou : une affectation déstructurée n'est pas si grande
    }
    if node.kind() == "variable_name" {
        facts
            .entry(node_text(text, node).to_string())
            .or_default()
            .poisoned = true;
        return;
    }
    let mut cursor = node.walk();
    let children: Vec<Node<'t>> = node.named_children(&mut cursor).collect();
    for c in children {
        poison_vars(text, c, facts);
    }
}
