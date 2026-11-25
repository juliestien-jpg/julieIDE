//! Extraction des déclarations, liaisons et références d'un fichier
//! JavaScript ou TypeScript (JSX/TSX compris).
//!
//! Portées : programme, fonctions (`var`, `function` y sont hissées), blocs
//! (`let`, `const`, `class`). Les déclarations de premier niveau et les classes
//! vont dans l'index ; le reste n'existe que le temps d'une analyse.

use tree_sitter::{Node, Tree};

use super::model::{
    flags, module_stem, node_text, short_key, Analysis, BaseRel, Binding, Columns, Decl,
    FileSymbols, ImportName, JsImport, JsModule, JsRecv, JsReexport, JsScope, Lang, Local, Ref,
    RefKind,
};
use super::SymbolKind;

pub(crate) fn analyze(text: &str, tree: &Tree, content_hash: u64, lang: Lang) -> Analysis {
    let root = tree.root_node();
    let mut x = Extractor {
        text,
        cols: Columns::new(text),
        decls: Vec::new(),
        refs: Vec::new(),
        locals: Vec::new(),
        scopes: vec![JsScope {
            parent: None,
            start: 0,
            end: text.len() as u32,
            bindings: Vec::new(),
        }],
        module: JsModule::default(),
        block: vec![0],
        function: vec![0],
        classes: Vec::new(),
        export_flags: 0,
        depth: 0,
    };
    x.visit_children(root);

    let mut keys: Vec<u32> = Vec::with_capacity(x.refs.len());
    for r in &x.refs {
        match &r.kind {
            RefKind::JsIdent { .. } | RefKind::JsImportName { .. } | RefKind::JsMember { .. } => {
                keys.push(short_key("j", &r.name, false))
            }
            RefKind::ModulePath { specifier } => {
                keys.push(short_key("s", &module_stem(specifier), false))
            }
            _ => {}
        }
    }
    keys.sort_unstable();
    keys.dedup();
    Analysis {
        symbols: FileSymbols {
            lang,
            content_hash,
            decls: x.decls,
            ref_keys: keys.into_boxed_slice(),
            js: Some(Box::new(x.module)),
        },
        refs: x.refs,
        locals: x.locals,
        namespaces: Vec::new(),
        scopes: x.scopes,
    }
}

struct Extractor<'a> {
    text: &'a str,
    cols: Columns<'a>,
    decls: Vec<Decl>,
    refs: Vec<Ref>,
    locals: Vec<Local>,
    scopes: Vec<JsScope>,
    module: JsModule,
    /// Portées de bloc ouvertes (la dernière est la courante).
    block: Vec<u32>,
    /// Portées de fonction ouvertes (programme compris).
    function: Vec<u32>,
    /// Classes englobantes (index dans `decls`).
    classes: Vec<u32>,
    /// Fanions d'export de la déclaration en cours (`export`, `export default`).
    export_flags: u8,
    depth: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Hoist {
    Block,
    Function,
}

impl<'a> Extractor<'a> {
    fn t(&self, node: Node) -> &'a str {
        node_text(self.text, node)
    }

    fn scope(&self) -> u32 {
        *self.block.last().unwrap_or(&0)
    }

    fn open_scope(&mut self, node: Node, function: bool) -> u32 {
        let id = self.scopes.len() as u32;
        self.scopes.push(JsScope {
            parent: Some(self.scope()),
            start: node.start_byte() as u32,
            end: node.end_byte() as u32,
            bindings: Vec::new(),
        });
        self.block.push(id);
        if function {
            self.function.push(id);
        }
        id
    }

    fn close_scope(&mut self, function: bool) {
        self.block.pop();
        if function {
            self.function.pop();
        }
    }

    fn bind(&mut self, scope: u32, name: &str, binding: Binding) {
        let s = &mut self.scopes[scope as usize];
        if !s.bindings.iter().any(|(n, _)| &**n == name) {
            s.bindings.push((name.into(), binding));
        }
    }

    /// Déclare `node` (un identifiant) : déclaration de l'index au niveau du
    /// module, liaison locale ailleurs.
    fn declare(
        &mut self,
        node: Node,
        kind: SymbolKind,
        hoist: Hoist,
        init_new: Option<Box<str>>,
    ) -> Binding {
        let name = self.t(node);
        let scope = match hoist {
            Hoist::Block => self.scope(),
            Hoist::Function => *self.function.last().unwrap_or(&0),
        };
        let span = self.cols.span(node);
        let binding = if scope == 0 || kind == SymbolKind::Class {
            let mut f = self.export_flags;
            if scope == 0 {
                f |= flags::MODULE_LEVEL;
            }
            let idx = self.decls.len() as u32;
            self.decls.push(Decl {
                kind,
                flags: f,
                name: name.into(),
                container: "".into(),
                span,
                types: init_new
                    .map(|c| vec![c].into_boxed_slice())
                    .unwrap_or_default(),
                bases: Box::new([]),
            });
            Binding::Decl(idx)
        } else {
            let idx = self.locals.len() as u32;
            self.locals.push(Local {
                name: name.into(),
                kind,
                span,
                scope,
                init_new,
                container: "".into(),
            });
            Binding::Local(idx)
        };
        self.bind(scope, name, binding);
        binding
    }

    fn ident_ref(&mut self, node: Node) {
        let span = self.cols.span(node);
        self.refs.push(Ref {
            span,
            name: self.t(node).into(),
            kind: RefKind::JsIdent {
                scope: self.scope(),
            },
        });
    }

    fn module_path(&mut self, string: Node) {
        let fragment = string
            .named_child(0)
            .filter(|c| c.kind() == "string_fragment");
        let Some(fragment) = fragment else {
            return;
        };
        let specifier = self.t(fragment);
        let span = self.cols.span(string);
        self.refs.push(Ref {
            span,
            name: module_stem(specifier).into(),
            kind: RefKind::ModulePath {
                specifier: specifier.into(),
            },
        });
    }

    fn string_value(&self, string: Node) -> Option<&'a str> {
        let fragment = string.named_child(0)?;
        (fragment.kind() == "string_fragment").then(|| self.t(fragment))
    }

    fn visit_children(&mut self, node: Node) {
        let mut cursor = node.walk();
        let children: Vec<Node> = node.named_children(&mut cursor).collect();
        for child in children {
            self.visit(child);
        }
    }

    fn visit(&mut self, node: Node) {
        if self.depth > super::store::MAX_DEPTH {
            return;
        }
        self.depth += 1;
        self.visit_inner(node);
        self.depth -= 1;
    }

    fn visit_inner(&mut self, node: Node) {
        match node.kind() {
            "import_statement" => self.import(node),
            "export_statement" => self.export(node),
            "function_declaration" | "generator_function_declaration" => {
                let flags = std::mem::take(&mut self.export_flags);
                if let Some(name) = node.child_by_field_name("name") {
                    self.export_flags = flags;
                    self.declare(name, SymbolKind::Function, Hoist::Function, None);
                    self.export_flags = 0;
                }
                self.function_like(node);
            }
            "function_expression" | "function" | "generator_function" | "arrow_function" => {
                self.function_like(node)
            }
            "class_declaration" | "class" | "abstract_class_declaration" => self.class(node),
            "lexical_declaration" | "variable_declaration" => self.variables(node),
            "interface_declaration" | "type_alias_declaration" | "enum_declaration" => {
                let kind = match node.kind() {
                    "interface_declaration" => SymbolKind::Interface,
                    "type_alias_declaration" => SymbolKind::TypeAlias,
                    _ => SymbolKind::Enum,
                };
                let flags = std::mem::take(&mut self.export_flags);
                let name = node.child_by_field_name("name");
                if let Some(name) = name {
                    self.export_flags = flags;
                    self.declare(name, kind, Hoist::Block, None);
                    self.export_flags = 0;
                }
                let mut cursor = node.walk();
                let children: Vec<Node> = node.named_children(&mut cursor).collect();
                for c in children {
                    if Some(c.id()) != name.map(|n| n.id()) {
                        self.visit(c);
                    }
                }
            }
            "statement_block" | "class_body" => {
                self.open_scope(node, false);
                self.visit_children(node);
                self.close_scope(false);
            }
            "for_statement" | "for_in_statement" | "catch_clause" => {
                self.open_scope(node, false);
                if node.kind() == "for_in_statement" {
                    if let Some(left) = node.child_by_field_name("left") {
                        let declares = node.child_by_field_name("kind").is_some();
                        if declares {
                            self.pattern(left, SymbolKind::Variable, Hoist::Block);
                        } else {
                            self.visit(left);
                        }
                    }
                    for field in ["right", "body"] {
                        if let Some(c) = node.child_by_field_name(field) {
                            self.visit(c);
                        }
                    }
                } else if node.kind() == "catch_clause" {
                    if let Some(p) = node.child_by_field_name("parameter") {
                        self.pattern(p, SymbolKind::Parameter, Hoist::Block);
                    }
                    if let Some(b) = node.child_by_field_name("body") {
                        self.visit(b);
                    }
                } else {
                    self.visit_children(node);
                }
                self.close_scope(false);
            }
            "identifier" | "shorthand_property_identifier" | "type_identifier" => {
                self.ident_ref(node)
            }
            "member_expression" => self.member(node),
            "call_expression" => {
                let function = node.child_by_field_name("function");
                let args = node.child_by_field_name("arguments");
                if let (Some(f), Some(a)) = (function, args) {
                    if f.kind() == "identifier" && self.t(f) == "require" {
                        if let Some(s) = a.named_child(0).filter(|s| s.kind() == "string") {
                            self.module_path(s);
                        }
                    }
                }
                self.visit_children(node);
            }
            "jsx_opening_element" | "jsx_self_closing_element" | "jsx_closing_element" => {
                if let Some(name) = node.child_by_field_name("name") {
                    match name.kind() {
                        "identifier" => {
                            if self.t(name).starts_with(|c: char| c.is_ascii_uppercase()) {
                                self.ident_ref(name);
                            }
                        }
                        _ => self.visit(name),
                    }
                }
                let mut cursor = node.walk();
                let children: Vec<Node> = node
                    .children_by_field_name("attribute", &mut cursor)
                    .collect();
                for c in children {
                    self.visit(c);
                }
            }
            "jsx_attribute" => {
                let mut cursor = node.walk();
                let children: Vec<Node> = node.named_children(&mut cursor).collect();
                for c in children.into_iter().skip(1) {
                    self.visit(c);
                }
            }
            "public_field_definition" | "field_definition" => {
                if let (Some(class), Some(name)) = (
                    self.classes.last().copied(),
                    node.child_by_field_name("name")
                        .or_else(|| node.child_by_field_name("property")),
                ) {
                    let container = self.decls[class as usize].name.clone();
                    let span = self.cols.span(name);
                    self.decls.push(Decl {
                        kind: SymbolKind::Property,
                        flags: 0,
                        name: self.t(name).into(),
                        container,
                        span,
                        types: Box::new([]),
                        bases: Box::new([]),
                    });
                }
                if let Some(v) = node.child_by_field_name("value") {
                    self.visit(v);
                }
                if let Some(t) = node.child_by_field_name("type") {
                    self.visit(t);
                }
            }
            "string"
            | "comment"
            | "regex"
            | "property_identifier"
            | "statement_identifier"
            | "private_property_identifier"
            | "html_comment" => {}
            _ => self.visit_children(node),
        }
    }

    fn import(&mut self, node: Node) {
        self.module.is_module = true;
        let Some(source) = node.child_by_field_name("source") else {
            return;
        };
        self.module_path(source);
        let Some(specifier) = self.string_value(source) else {
            return;
        };
        let specifier: Box<str> = specifier.into();
        let mut cursor = node.walk();
        let clause = node
            .named_children(&mut cursor)
            .find(|c| c.kind() == "import_clause");
        let Some(clause) = clause else {
            return;
        };
        let mut c = clause.walk();
        let parts: Vec<Node> = clause.named_children(&mut c).collect();
        for part in parts {
            match part.kind() {
                "identifier" => {
                    self.add_import(part, &specifier, ImportName::Default, Some(part), true);
                }
                "namespace_import" => {
                    if let Some(id) = part.named_child(0) {
                        self.add_import(id, &specifier, ImportName::Namespace, Some(id), true);
                    }
                }
                "named_imports" => {
                    let mut c2 = part.walk();
                    let specs: Vec<Node> = part.named_children(&mut c2).collect();
                    for spec in specs {
                        if spec.kind() != "import_specifier" {
                            continue;
                        }
                        let Some(name) = spec.child_by_field_name("name") else {
                            continue;
                        };
                        let imported = if name.kind() == "string" {
                            self.string_value(name).unwrap_or("").to_string()
                        } else {
                            self.t(name).to_string()
                        };
                        let alias = spec.child_by_field_name("alias");
                        let local = alias.unwrap_or(name);
                        let idx = self.add_import(
                            local,
                            &specifier,
                            ImportName::Named(imported.as_str().into()),
                            None,
                            false,
                        );
                        // Le nom importé est l'usage compté ; l'alias, non.
                        let span = self.cols.span(name);
                        self.refs.push(Ref {
                            span,
                            name: imported.as_str().into(),
                            kind: RefKind::JsImportName {
                                import: idx,
                                counted: true,
                            },
                        });
                        if let Some(alias) = alias {
                            let span = self.cols.span(alias);
                            self.refs.push(Ref {
                                span,
                                name: self.t(alias).into(),
                                kind: RefKind::JsImportName {
                                    import: idx,
                                    counted: false,
                                },
                            });
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn add_import(
        &mut self,
        local: Node,
        source: &str,
        imported: ImportName,
        ref_node: Option<Node>,
        counted: bool,
    ) -> u32 {
        let idx = self.module.imports.len() as u32;
        let local_name = self.t(local);
        self.module.imports.push(JsImport {
            local: local_name.into(),
            source: source.into(),
            imported,
        });
        self.bind(0, local_name, Binding::Import(idx));
        if let Some(n) = ref_node {
            let span = self.cols.span(n);
            self.refs.push(Ref {
                span,
                name: local_name.into(),
                kind: RefKind::JsImportName {
                    import: idx,
                    counted,
                },
            });
        }
        idx
    }

    fn export(&mut self, node: Node) {
        self.module.is_module = true;
        let mut cursor = node.walk();
        let is_default = node.children(&mut cursor).any(|c| c.kind() == "default");
        let source = node.child_by_field_name("source");
        if let Some(s) = source {
            self.module_path(s);
        }
        if let Some(declaration) = node.child_by_field_name("declaration") {
            self.export_flags =
                flags::EXPORTED | if is_default { flags::DEFAULT_EXPORT } else { 0 };
            self.visit(declaration);
            self.export_flags = 0;
            return;
        }
        if let Some(value) = node.child_by_field_name("value") {
            match value.kind() {
                "identifier" => {
                    self.module
                        .exports
                        .push((self.t(value).into(), "default".into()));
                    self.ident_ref(value);
                }
                "class" | "function_expression" | "function"
                    if value.child_by_field_name("name").is_some() =>
                {
                    self.export_flags = flags::EXPORTED | flags::DEFAULT_EXPORT;
                    if value.kind() == "class" {
                        self.class(value);
                    } else {
                        if let Some(name) = value.child_by_field_name("name") {
                            self.declare(name, SymbolKind::Function, Hoist::Function, None);
                        }
                        self.export_flags = 0;
                        self.function_like(value);
                    }
                    self.export_flags = 0;
                }
                _ => self.visit(value),
            }
            return;
        }
        let source_text = source
            .and_then(|s| self.string_value(s))
            .map(Box::<str>::from);
        let mut cursor = node.walk();
        let clause = node
            .named_children(&mut cursor)
            .find(|c| c.kind() == "export_clause");
        match (clause, source_text) {
            (Some(clause), Some(src)) => {
                let mut names = Vec::new();
                let mut c = clause.walk();
                let specs: Vec<Node> = clause.named_children(&mut c).collect();
                for spec in specs {
                    if let Some(name) = spec.child_by_field_name("name") {
                        let imported = self.t(name);
                        let exported = spec
                            .child_by_field_name("alias")
                            .map(|a| self.t(a))
                            .unwrap_or(imported);
                        names.push((imported.into(), exported.into()));
                    }
                }
                self.module.reexports.push(JsReexport {
                    source: src,
                    names: Some(names),
                });
            }
            (Some(clause), None) => {
                let mut c = clause.walk();
                let specs: Vec<Node> = clause.named_children(&mut c).collect();
                for spec in specs {
                    if let Some(name) = spec.child_by_field_name("name") {
                        let local = self.t(name);
                        let exported = spec
                            .child_by_field_name("alias")
                            .map(|a| self.t(a))
                            .unwrap_or(local);
                        self.module.exports.push((local.into(), exported.into()));
                        self.ident_ref(name);
                    }
                }
            }
            (None, Some(src)) => {
                let mut cursor = node.walk();
                let namespace = node
                    .named_children(&mut cursor)
                    .any(|c| c.kind() == "namespace_export");
                if !namespace {
                    self.module.reexports.push(JsReexport {
                        source: src,
                        names: None,
                    });
                }
            }
            (None, None) => self.visit_children(node),
        }
    }

    fn function_like(&mut self, node: Node) {
        self.open_scope(node, true);
        if let Some(p) = node.child_by_field_name("parameter") {
            self.pattern(p, SymbolKind::Parameter, Hoist::Block);
        }
        if let Some(params) = node.child_by_field_name("parameters") {
            let mut cursor = params.walk();
            let children: Vec<Node> = params.named_children(&mut cursor).collect();
            for p in children {
                self.parameter(p);
            }
        }
        for field in ["return_type", "type_parameters"] {
            if let Some(t) = node.child_by_field_name(field) {
                self.visit(t);
            }
        }
        if let Some(body) = node.child_by_field_name("body") {
            if body.kind() == "statement_block" {
                // Le corps partage la portée de la fonction.
                self.visit_children(body);
            } else {
                self.visit(body);
            }
        }
        self.close_scope(true);
    }

    fn parameter(&mut self, p: Node) {
        match p.kind() {
            "required_parameter" | "optional_parameter" => {
                if let Some(pattern) = p.child_by_field_name("pattern") {
                    self.pattern(pattern, SymbolKind::Parameter, Hoist::Block);
                }
                for field in ["type", "value"] {
                    if let Some(c) = p.child_by_field_name(field) {
                        self.visit(c);
                    }
                }
            }
            _ => self.pattern(p, SymbolKind::Parameter, Hoist::Block),
        }
    }

    /// Lie les identifiants d'un motif (`a`, `{a, b: c}`, `[x, ...y]`, `a = 1`).
    fn pattern(&mut self, node: Node, kind: SymbolKind, hoist: Hoist) {
        match node.kind() {
            "identifier" | "shorthand_property_identifier_pattern" => {
                self.declare(node, kind, hoist, None);
            }
            "assignment_pattern" => {
                if let Some(l) = node.child_by_field_name("left") {
                    self.pattern(l, kind, hoist);
                }
                if let Some(r) = node.child_by_field_name("right") {
                    self.visit(r);
                }
            }
            "pair_pattern" => {
                if let Some(v) = node.child_by_field_name("value") {
                    self.pattern(v, kind, hoist);
                }
            }
            "object_assignment_pattern" => {
                if let Some(l) = node.child_by_field_name("left") {
                    self.pattern(l, kind, hoist);
                }
                if let Some(r) = node.child_by_field_name("right") {
                    self.visit(r);
                }
            }
            "object_pattern" | "array_pattern" | "rest_pattern" => {
                let mut cursor = node.walk();
                let children: Vec<Node> = node.named_children(&mut cursor).collect();
                for c in children {
                    self.pattern(c, kind, hoist);
                }
            }
            "this" | "accessibility_modifier" => {}
            _ => self.visit(node),
        }
    }

    fn variables(&mut self, node: Node) {
        let flags = std::mem::take(&mut self.export_flags);
        let (hoist, base_kind) = if node.kind() == "variable_declaration" {
            (Hoist::Function, SymbolKind::Variable)
        } else {
            let mut cursor = node.walk();
            let is_const = node.children(&mut cursor).any(|c| c.kind() == "const");
            (
                Hoist::Block,
                if is_const {
                    SymbolKind::Constant
                } else {
                    SymbolKind::Variable
                },
            )
        };
        let mut cursor = node.walk();
        let declarators: Vec<Node> = node.named_children(&mut cursor).collect();
        for d in declarators {
            if d.kind() != "variable_declarator" {
                self.visit(d);
                continue;
            }
            let value = d.child_by_field_name("value");
            let init_new = value.and_then(|v| {
                (v.kind() == "new_expression")
                    .then(|| v.child_by_field_name("constructor"))
                    .flatten()
                    .filter(|c| c.kind() == "identifier")
                    .map(|c| Box::<str>::from(self.t(c)))
            });
            let kind = match value.map(|v| v.kind()) {
                Some("arrow_function" | "function_expression" | "function") => SymbolKind::Function,
                _ => base_kind,
            };
            if let Some(name) = d.child_by_field_name("name") {
                if name.kind() == "identifier" {
                    self.export_flags = flags;
                    self.declare(name, kind, hoist, init_new);
                    self.export_flags = 0;
                } else {
                    self.pattern(name, base_kind, hoist);
                }
            }
            if let Some(t) = d.child_by_field_name("type") {
                self.visit(t);
            }
            if let Some(v) = value {
                self.visit(v);
            }
        }
    }

    fn class(&mut self, node: Node) {
        let flags = std::mem::take(&mut self.export_flags);
        let name = node.child_by_field_name("name");
        let mut class_idx = None;
        if let Some(name) = name {
            self.export_flags = flags;
            if let Binding::Decl(idx) = self.declare(name, SymbolKind::Class, Hoist::Block, None) {
                class_idx = Some(idx);
            }
            self.export_flags = 0;
        }
        let mut cursor = node.walk();
        let children: Vec<Node> = node.named_children(&mut cursor).collect();
        for c in &children {
            if c.kind() == "class_heritage" {
                let mut bases: Vec<(BaseRel, Box<str>)> = Vec::new();
                self.heritage(*c, &mut bases);
                if let Some(idx) = class_idx {
                    self.decls[idx as usize].bases = bases.into_boxed_slice();
                }
            } else if c.kind() == "decorator" || c.kind() == "type_parameters" {
                self.visit(*c);
            }
        }
        if let Some(body) = node.child_by_field_name("body") {
            if let Some(idx) = class_idx {
                self.classes.push(idx);
            }
            self.open_scope(body, false);
            let mut c = body.walk();
            let members: Vec<Node> = body.named_children(&mut c).collect();
            for m in members {
                if m.kind() == "method_definition" || m.kind() == "method_signature" {
                    self.method(m, class_idx);
                } else {
                    self.visit(m);
                }
            }
            self.close_scope(false);
            if class_idx.is_some() {
                self.classes.pop();
            }
        }
    }

    fn heritage(&mut self, node: Node, bases: &mut Vec<(BaseRel, Box<str>)>) {
        let mut cursor = node.walk();
        let children: Vec<Node> = node.named_children(&mut cursor).collect();
        for c in children {
            match c.kind() {
                "identifier" | "type_identifier" => {
                    let rel = if node.kind() == "implements_clause" {
                        BaseRel::Implements
                    } else {
                        BaseRel::Extends
                    };
                    bases.push((rel, self.t(c).into()));
                    self.ident_ref(c);
                }
                "extends_clause" | "implements_clause" => self.heritage(c, bases),
                _ => self.visit(c),
            }
        }
    }

    fn method(&mut self, node: Node, class: Option<u32>) {
        if let (Some(name), Some(class)) = (node.child_by_field_name("name"), class) {
            let mut cursor = node.walk();
            let is_static = node.children(&mut cursor).any(|c| c.kind() == "static");
            let container = self.decls[class as usize].name.clone();
            let span = self.cols.span(name);
            self.decls.push(Decl {
                kind: SymbolKind::Method,
                flags: if is_static { flags::STATIC } else { 0 },
                name: self.t(name).into(),
                container,
                span,
                types: Box::new([]),
                bases: Box::new([]),
            });
        }
        self.function_like(node);
    }

    fn member(&mut self, node: Node) {
        let object = node.child_by_field_name("object");
        let property = node.child_by_field_name("property");
        if let (Some(object), Some(property)) = (object, property) {
            let recv = match object.kind() {
                "this" => JsRecv::This(self.classes.last().copied()),
                "identifier" => JsRecv::Ident(self.t(object).into(), self.scope()),
                _ => JsRecv::Unknown,
            };
            let recv_text: Box<str> = {
                let t = self.t(object);
                if t.chars().count() > 40 {
                    format!("{}…", t.chars().take(40).collect::<String>()).into()
                } else {
                    t.into()
                }
            };
            let span = self.cols.span(property);
            let name = self.t(property);
            self.visit(object);
            self.refs.push(Ref {
                span,
                name: name.into(),
                kind: RefKind::JsMember { recv, recv_text },
            });
        } else if let Some(object) = object {
            self.visit(object);
        }
    }
}
