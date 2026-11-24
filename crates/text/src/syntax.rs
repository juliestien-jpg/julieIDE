//! Coloration syntaxique par tree-sitter : langages, catégories, calcul des
//! plages colorées, langages imbriqués.
//!
//! Le calcul lui-même ([`Highlighter`]) est synchrone et sans état partagé ; la
//! [`crate::SyntaxSession`] le fait tourner dans un fil de fond, de façon
//! incrémentale.

use std::collections::HashMap;
use std::ops::Range;
use std::path::Path;
use std::sync::OnceLock;

use ropey::Rope;
use tree_sitter::{
    InputEdit, Node, Parser, Point, Query, QueryCursor, Range as TsRange, StreamingIterator, Tree,
};

use crate::buffer::ByteEdit;

/// Langage d'un document (ou d'une zone imbriquée).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Language {
    /// Texte brut : pas de coloration.
    #[default]
    Plain,
    /// PHP, avec le HTML hors des balises `<?php ?>` (et ses `<script>`, `<style>`).
    Php,
    /// HTML, avec le JavaScript de `<script>` et le CSS de `<style>`.
    Html,
    Css,
    /// JavaScript, JSX compris.
    JavaScript,
    TypeScript,
    /// TypeScript avec JSX.
    Tsx,
}

impl Language {
    /// D'après l'extension du fichier ; texte brut si elle est inconnue.
    pub fn from_path(path: &Path) -> Self {
        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase);
        match extension.as_deref() {
            Some("php" | "phtml" | "php3" | "php4" | "php5" | "phps" | "inc") => Language::Php,
            Some("html" | "htm" | "xhtml") => Language::Html,
            Some("css") => Language::Css,
            Some("js" | "mjs" | "cjs" | "jsx") => Language::JavaScript,
            Some("ts" | "mts" | "cts") => Language::TypeScript,
            Some("tsx") => Language::Tsx,
            _ => Language::Plain,
        }
    }

    /// Clé stable (`"php"`, `"html"`…), pour le catalogue de textes et les réglages.
    pub fn key(self) -> &'static str {
        match self {
            Language::Plain => "plain",
            Language::Php => "php",
            Language::Html => "html",
            Language::Css => "css",
            Language::JavaScript => "javascript",
            Language::TypeScript => "typescript",
            Language::Tsx => "tsx",
        }
    }

    /// Préfixe de commentaire de ligne, s'il y en a un.
    pub fn line_comment(self) -> Option<&'static str> {
        match self {
            Language::Php | Language::JavaScript | Language::TypeScript | Language::Tsx => {
                Some("//")
            }
            Language::Plain | Language::Html | Language::Css => None,
        }
    }

    /// Délimiteurs de commentaire de bloc, s'il y en a.
    pub fn block_comment(self) -> Option<(&'static str, &'static str)> {
        match self {
            Language::Html => Some(("<!--", "-->")),
            Language::Plain => None,
            _ => Some(("/*", "*/")),
        }
    }

    fn grammar(self) -> Option<tree_sitter::Language> {
        Some(match self {
            Language::Plain => return None,
            Language::Php => tree_sitter_php::LANGUAGE_PHP.into(),
            Language::Html => tree_sitter_html::LANGUAGE.into(),
            Language::Css => tree_sitter_css::LANGUAGE.into(),
            Language::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
            Language::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Language::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
        })
    }

    fn query_source(self) -> String {
        let js = tree_sitter_javascript::HIGHLIGHT_QUERY;
        let jsx = tree_sitter_javascript::JSX_HIGHLIGHT_QUERY;
        let ts = tree_sitter_typescript::HIGHLIGHTS_QUERY;
        match self {
            Language::Plain => String::new(),
            Language::Php => tree_sitter_php::HIGHLIGHTS_QUERY.to_owned(),
            Language::Html => tree_sitter_html::HIGHLIGHTS_QUERY.to_owned(),
            Language::Css => include_str!("../queries/css.scm").to_owned(),
            Language::JavaScript => format!("{js}\n{jsx}"),
            Language::TypeScript => format!("{js}\n{ts}"),
            Language::Tsx => format!("{js}\n{jsx}\n{ts}"),
        }
    }
}

/// Catégorie d'une plage colorée. La couleur relève du thème, côté interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum HighlightKind {
    Keyword,
    String,
    Number,
    Comment,
    /// Variable PHP (`$nom`), variable CSS (`--nom`).
    Variable,
    /// Nom de fonction ou de méthode (déclaration ou appel).
    Function,
    /// Classe, interface, type, espace de noms.
    Type,
    /// Constante (`MAX_SIZE`, `PHP_EOL`).
    Constant,
    /// Balise HTML ou JSX.
    Tag,
    /// Attribut HTML ou JSX.
    Attribute,
    /// Propriété CSS, propriété d'objet.
    Property,
    /// Sélecteur CSS.
    Selector,
    Operator,
    Punctuation,
    /// Séquence d'échappement dans une chaîne.
    Escape,
}

impl HighlightKind {
    pub const ALL: [HighlightKind; 15] = [
        HighlightKind::Keyword,
        HighlightKind::String,
        HighlightKind::Number,
        HighlightKind::Comment,
        HighlightKind::Variable,
        HighlightKind::Function,
        HighlightKind::Type,
        HighlightKind::Constant,
        HighlightKind::Tag,
        HighlightKind::Attribute,
        HighlightKind::Property,
        HighlightKind::Selector,
        HighlightKind::Operator,
        HighlightKind::Punctuation,
        HighlightKind::Escape,
    ];
}

/// Une plage colorée **dans une ligne** : octets `start..end` du contenu de la
/// ligne (UTF-8), sans chevauchement, triées.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HighlightSpan {
    pub start: u32,
    pub end: u32,
    pub kind: HighlightKind,
}

/// Plages colorées de tout un texte, ligne par ligne.
pub type LineHighlights = Vec<Vec<HighlightSpan>>;

/// Un arbre d'une zone imbriquée (le HTML d'un fichier PHP, le JS d'un `<script>`).
#[derive(Debug, Clone)]
pub struct InjectedTree {
    pub language: Language,
    pub tree: Tree,
    /// Plages d'octets du document couvertes par cette zone.
    pub ranges: Vec<Range<usize>>,
}

/// Arbres d'une version du document, partageables (cloner un `Tree` est en temps
/// constant). C'est ce que `julie-index` pourra lire pour extraire les symboles
/// d'un fichier ouvert sans le réanalyser.
#[derive(Debug, Clone)]
pub struct SyntaxSnapshot {
    pub version: u64,
    pub language: Language,
    pub tree: Tree,
    pub injections: Vec<InjectedTree>,
}

struct Config {
    grammar: tree_sitter::Language,
    query: Query,
    kinds: Vec<Option<HighlightKind>>,
}

fn config(language: Language) -> Option<&'static Config> {
    static CONFIGS: OnceLock<HashMap<Language, Option<Config>>> = OnceLock::new();
    CONFIGS
        .get_or_init(|| {
            [
                Language::Php,
                Language::Html,
                Language::Css,
                Language::JavaScript,
                Language::TypeScript,
                Language::Tsx,
            ]
            .into_iter()
            .map(|language| (language, build_config(language)))
            .collect()
        })
        .get(&language)
        .and_then(Option::as_ref)
}

fn build_config(language: Language) -> Option<Config> {
    let grammar = language.grammar()?;
    let mut query = match Query::new(&grammar, &language.query_source()) {
        Ok(query) => query,
        Err(error) => {
            // Ne devrait jamais arriver (les requêtes sont vérifiées par les tests) :
            // on retombe sur le texte brut plutôt que de paniquer.
            debug_assert!(false, "requête {language:?} invalide : {error}");
            return None;
        }
    };
    if language == Language::Php {
        // Le « $ » d'une variable PHP fait partie de la variable (comme dans PhpStorm).
        query.disable_capture("operator");
    }
    let kinds = query
        .capture_names()
        .iter()
        .map(|name| kind_for_capture(name, language))
        .collect();
    Some(Config {
        grammar,
        query,
        kinds,
    })
}

/// Traduit un nom de capture des requêtes tree-sitter en catégorie.
fn kind_for_capture(name: &str, language: Language) -> Option<HighlightKind> {
    use HighlightKind as K;
    let mut parts = name.split('.');
    let head = parts.next().unwrap_or_default();
    let sub = parts.next();
    match (head, sub) {
        ("variable", Some("builtin")) => Some(K::Keyword),
        // En JS, toute identifiant est « variable » : PhpStorm ne les colore pas.
        ("variable", _) => matches!(language, Language::Php | Language::Css).then_some(K::Variable),
        ("constant", Some("builtin")) => Some(K::Keyword),
        ("constant", _) => Some(K::Constant),
        ("keyword", _) => Some(K::Keyword),
        ("string", _) => Some(K::String),
        ("number" | "float", _) => Some(K::Number),
        ("comment", _) => Some(K::Comment),
        ("function" | "method", _) => Some(K::Function),
        ("constructor" | "type" | "module" | "namespace", _) => Some(K::Type),
        // `<?php` et `?>` : marqueurs du langage, pas des balises HTML.
        ("tag", _) if language == Language::Php => Some(K::Keyword),
        ("tag", _) => Some(K::Tag),
        ("attribute", _) => Some(K::Attribute),
        ("property", _) => Some(K::Property),
        ("selector", _) => Some(K::Selector),
        ("operator", _) => Some(K::Operator),
        ("punctuation", Some("bracket")) if language == Language::Html => Some(K::Tag),
        ("punctuation", _) => Some(K::Punctuation),
        ("escape", _) => Some(K::Escape),
        _ => None,
    }
}

/// Une plage colorée en octets du document, avant découpage en lignes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Span {
    start: usize,
    end: usize,
    kind: Option<HighlightKind>,
}

/// Une zone d'une version à l'autre : son langage et son premier octet.
type LayerKey = (Language, usize);

/// Une zone imbriquée analysée. `depth` : 1 dans le document, 2 dans une zone
/// de profondeur 1 (le JS d'un `<script>` du HTML d'un fichier PHP), etc.
struct Layer {
    language: Language,
    tree: Tree,
    ranges: Vec<Range<usize>>,
    depth: usize,
}

/// Analyse et colore un document, en gardant ses arbres et ses plages d'une
/// version à l'autre : après une modification, l'analyse est incrémentale et
/// seules les lignes touchées, ou dont l'analyse a changé, sont recolorées.
pub struct Highlighter {
    language: Language,
    parsers: HashMap<Language, Parser>,
    tree: Option<Tree>,
    layers: Vec<Layer>,
    /// Plages de la version précédente, recalées sur les modifications.
    lines: Option<LineHighlights>,
    /// Octets modifiés depuis la dernière analyse (coordonnées actuelles).
    dirty: Vec<Range<usize>>,
}

impl Highlighter {
    pub fn new(language: Language) -> Self {
        Self {
            language,
            parsers: HashMap::new(),
            tree: None,
            layers: Vec::new(),
            lines: None,
            dirty: Vec::new(),
        }
    }

    pub fn language(&self) -> Language {
        self.language
    }

    /// Reporte des modifications sur les arbres et les plages gardés (avant
    /// [`Self::update`]).
    pub fn edit(&mut self, edits: &[ByteEdit]) {
        for edit in edits {
            let input = to_input_edit(edit);
            if let Some(tree) = &mut self.tree {
                tree.edit(&input);
            }
            for layer in &mut self.layers {
                layer.tree.edit(&input);
                for range in &mut layer.ranges {
                    *range = map_byte(range.start, edit)..map_byte(range.end, edit);
                }
            }
            for range in &mut self.dirty {
                *range = map_byte(range.start, edit)..map_byte(range.end, edit);
            }
            self.dirty.push(edit.start_byte..edit.new_end_byte);
            if let Some(lines) = &mut self.lines {
                crate::session::splice(lines, edit);
            }
        }
    }

    /// Oublie arbres et plages : la prochaine analyse repart de zéro.
    pub fn reset(&mut self) {
        self.tree = None;
        self.layers.clear();
        self.lines = None;
        self.dirty.clear();
    }

    /// Analyse `rope` (incrémentalement si des arbres sont gardés et ont reçu
    /// les modifications par [`Self::edit`]) et rend ses plages colorées.
    pub fn update(
        &mut self,
        rope: &Rope,
        version: u64,
    ) -> (LineHighlights, Option<SyntaxSnapshot>) {
        let Some(root_config) = config(self.language) else {
            self.reset();
            return (vec![Vec::new(); rope.len_lines()], None);
        };
        let old_tree = self.tree.take();
        let parser = self.parsers.entry(self.language).or_default();
        let Some(tree) = parse(parser, root_config, rope, old_tree.as_ref(), None) else {
            self.reset();
            return (vec![Vec::new(); rope.len_lines()], None);
        };
        let mut dirty = std::mem::take(&mut self.dirty);
        let full = old_tree.is_none()
            || self
                .lines
                .as_ref()
                .is_none_or(|lines| lines.len() != rope.len_lines());
        if let Some(old) = &old_tree {
            dirty.extend(old.changed_ranges(&tree).map(|r| r.start_byte..r.end_byte));
        }

        // Zones imbriquées : toutes réanalysées, incrémentalement quand la même
        // zone (même langage, même début) existait à la version précédente.
        let mut old_layers: HashMap<LayerKey, (Tree, Vec<Range<usize>>)> = self
            .layers
            .drain(..)
            .filter_map(|layer| {
                let start = layer.ranges.first()?.start;
                Some(((layer.language, start), (layer.tree, layer.ranges)))
            })
            .collect();
        let mut layers: Vec<Layer> = Vec::new();
        let mut queue: Vec<(Language, Vec<Range<usize>>, usize)> = find_injections(
            self.language,
            &tree,
            rope,
            std::slice::from_ref(&(0..rope.len_bytes())),
        )
        .into_iter()
        .map(|(language, ranges)| (language, ranges, 1))
        .collect();
        let mut index = 0;
        while index < queue.len() {
            let (language, ranges, depth) = queue[index].clone();
            index += 1;
            let Some(layer_config) = config(language) else {
                continue;
            };
            // Une zone dont l'arbre avait des erreurs est réanalysée de zéro : avec des
            // plages incluses, la reprise sur erreur incrémentale peut donner un autre
            // arbre que l'analyse complète (constaté sur du HTML dans du PHP).
            let old =
                old_layers
                    .remove(&(language, ranges[0].start))
                    .and_then(|(tree, old_ranges)| {
                        if tree.root_node().has_error() {
                            dirty.extend(old_ranges);
                            None
                        } else {
                            Some((tree, old_ranges))
                        }
                    });
            let ts_ranges = to_ts_ranges(rope, &ranges);
            let parser = self.parsers.entry(language).or_default();
            let Some(layer_tree) = parse(
                parser,
                layer_config,
                rope,
                old.as_ref().map(|(tree, _)| tree),
                Some(&ts_ranges),
            ) else {
                continue;
            };
            match &old {
                Some((old_tree, old_ranges)) => {
                    dirty.extend(
                        old_tree
                            .changed_ranges(&layer_tree)
                            .map(|r| r.start_byte..r.end_byte),
                    );
                    if *old_ranges != ranges {
                        dirty.extend(symmetric_difference(old_ranges, &ranges));
                    }
                }
                None => dirty.extend(ranges.iter().cloned()),
            }
            for (inner, inner_ranges) in find_injections(language, &layer_tree, rope, &ranges) {
                queue.push((inner, inner_ranges, depth + 1));
            }
            layers.push(Layer {
                language,
                tree: layer_tree,
                ranges,
                depth,
            });
        }
        // Zones disparues : leur place est à recolorer.
        for (_, ranges) in old_layers.into_values() {
            dirty.extend(ranges);
        }

        let mut lines = match (full, self.lines.take()) {
            (false, Some(lines)) => lines,
            _ => {
                dirty.clear();
                dirty.push(0..rope.len_bytes());
                vec![Vec::new(); rope.len_lines()]
            }
        };
        let max_depth = layers.iter().map(|l| l.depth).max().unwrap_or(0);
        for line_range in dirty_lines(rope, &dirty) {
            let start = rope.line_to_byte(line_range.start);
            let end = if line_range.end < rope.len_lines() {
                rope.line_to_byte(line_range.end)
            } else {
                rope.len_bytes()
            };
            let window = start..end;
            let mut spans = layer_spans(
                root_config,
                &tree,
                rope,
                std::slice::from_ref(&window),
                &window,
            );
            for depth in 1..=max_depth {
                let mut top = Vec::new();
                let mut covered = Vec::new();
                for layer in layers.iter().filter(|l| l.depth == depth) {
                    let ranges = clip_ranges(&layer.ranges, &window);
                    if ranges.is_empty() {
                        continue;
                    }
                    if let Some(layer_config) = config(layer.language) {
                        top.extend(layer_spans(
                            layer_config,
                            &layer.tree,
                            rope,
                            &ranges,
                            &window,
                        ));
                    }
                    covered.extend(ranges);
                }
                if covered.is_empty() {
                    continue;
                }
                top.sort_by_key(|s| s.start);
                covered.sort_by_key(|r| r.start);
                spans = overlay(&spans, &top, &covered);
            }
            fill_lines(rope, &spans, line_range, &mut lines);
        }

        let snapshot = SyntaxSnapshot {
            version,
            language: self.language,
            tree: tree.clone(),
            injections: layers
                .iter()
                .map(|layer| InjectedTree {
                    language: layer.language,
                    tree: layer.tree.clone(),
                    ranges: layer.ranges.clone(),
                })
                .collect(),
        };
        self.tree = Some(tree);
        self.layers = layers;
        self.lines = Some(lines.clone());
        (lines, Some(snapshot))
    }
}

/// Colore `text` d'un coup, sans fil de fond (tests, aperçus).
pub fn highlight_text(language: Language, text: &str) -> LineHighlights {
    let rope = Rope::from_str(text);
    Highlighter::new(language).update(&rope, 0).0
}

/// Où va l'octet `byte` (texte d'avant) après la modification.
fn map_byte(byte: usize, edit: &ByteEdit) -> usize {
    if byte <= edit.start_byte {
        byte
    } else if byte >= edit.old_end_byte {
        byte - edit.old_end_byte + edit.new_end_byte
    } else {
        edit.new_end_byte
    }
}

/// Ce qui est dans l'un des deux ensembles de plages et pas dans l'autre (à la
/// grosse : chaque plage qui n'a pas d'égale de l'autre côté).
fn symmetric_difference(a: &[Range<usize>], b: &[Range<usize>]) -> Vec<Range<usize>> {
    a.iter()
        .filter(|r| !b.contains(r))
        .chain(b.iter().filter(|r| !a.contains(r)))
        .cloned()
        .collect()
}

/// Les plages `ranges` (triées) bornées à `window`.
fn clip_ranges(ranges: &[Range<usize>], window: &Range<usize>) -> Vec<Range<usize>> {
    let (Some(first), Some(last)) = (ranges.first(), ranges.last()) else {
        return Vec::new();
    };
    if first.start >= window.end || last.end <= window.start {
        return Vec::new();
    }
    let from = ranges.partition_point(|r| r.end <= window.start);
    ranges[from..]
        .iter()
        .take_while(|r| r.start < window.end)
        .filter_map(|r| {
            let start = r.start.max(window.start);
            let end = r.end.min(window.end);
            (start < end).then_some(start..end)
        })
        .collect()
}

/// Lignes (début inclus, fin exclue) touchées par des plages d'octets, fusionnées.
fn dirty_lines(rope: &Rope, dirty: &[Range<usize>]) -> Vec<Range<usize>> {
    let len = rope.len_bytes();
    let line_count = rope.len_lines();
    let mut lines: Vec<Range<usize>> = dirty
        .iter()
        .map(|r| {
            let first = rope.byte_to_line(r.start.min(len));
            let last = rope.byte_to_line(r.end.min(len));
            first..(last + 1).min(line_count)
        })
        .collect();
    lines.sort_by_key(|r| r.start);
    let mut merged: Vec<Range<usize>> = Vec::with_capacity(lines.len());
    for range in lines {
        match merged.last_mut() {
            Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
            _ => merged.push(range),
        }
    }
    merged
}

fn to_input_edit(edit: &ByteEdit) -> InputEdit {
    let point = |p: crate::buffer::BytePoint| Point {
        row: p.row,
        column: p.column,
    };
    InputEdit {
        start_byte: edit.start_byte,
        old_end_byte: edit.old_end_byte,
        new_end_byte: edit.new_end_byte,
        start_position: point(edit.start_point),
        old_end_position: point(edit.old_end_point),
        new_end_position: point(edit.new_end_point),
    }
}

fn byte_point(rope: &Rope, byte: usize) -> Point {
    let row = rope.byte_to_line(byte);
    Point {
        row,
        column: byte - rope.line_to_byte(row),
    }
}

fn to_ts_ranges(rope: &Rope, ranges: &[Range<usize>]) -> Vec<TsRange> {
    ranges
        .iter()
        .map(|range| TsRange {
            start_byte: range.start,
            end_byte: range.end,
            start_point: byte_point(rope, range.start),
            end_point: byte_point(rope, range.end),
        })
        .collect()
}

fn parse(
    parser: &mut Parser,
    config: &Config,
    rope: &Rope,
    old: Option<&Tree>,
    ranges: Option<&[TsRange]>,
) -> Option<Tree> {
    parser.set_language(&config.grammar).ok()?;
    match ranges {
        Some(ranges) => parser.set_included_ranges(ranges).ok()?,
        None => parser.set_included_ranges(&[]).ok()?,
    }
    let len = rope.len_bytes();
    let mut read = |byte: usize, _: Point| -> &[u8] {
        if byte >= len {
            return &[];
        }
        let (chunk, chunk_start, _, _) = rope.chunk_at_byte(byte);
        &chunk.as_bytes()[byte - chunk_start..]
    };
    parser.parse_with_options(&mut read, old, None)
}

/// Plages colorées d'une couche, sans chevauchement, bornées à `ranges` ; seuls
/// les nœuds qui touchent `window` sont examinés.
fn layer_spans(
    config: &Config,
    tree: &Tree,
    rope: &Rope,
    ranges: &[Range<usize>],
    window: &Range<usize>,
) -> Vec<Span> {
    let mut raw: Vec<(usize, usize, u32, Option<HighlightKind>)> = Vec::new();
    let mut cursor = QueryCursor::new();
    cursor.set_byte_range(window.clone());
    let provider = |node: Node| {
        rope.byte_slice(node.byte_range())
            .chunks()
            .map(str::as_bytes)
    };
    let mut matches = cursor.matches(&config.query, tree.root_node(), provider);
    while let Some(found) = matches.next() {
        for capture in found.captures {
            let range = capture.node.byte_range();
            if range.is_empty() {
                continue;
            }
            raw.push((
                range.start,
                range.end,
                found.pattern_index as u32,
                config.kinds[capture.index as usize],
            ));
        }
    }
    let spans = resolve(raw);
    clip(&spans, ranges)
}

/// Rend des plages sans chevauchement à partir de captures imbriquées : pour une
/// même plage, le dernier motif de la requête l'emporte (convention des
/// requêtes fournies avec les grammaires) ; une capture intérieure l'emporte sur
/// celle qui l'englobe, même sans catégorie (un identifiant dans une chaîne
/// gabarit redevient du texte ordinaire).
fn resolve(mut raw: Vec<(usize, usize, u32, Option<HighlightKind>)>) -> Vec<Span> {
    // Début croissant, fin décroissante (l'englobante d'abord), motif décroissant.
    raw.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)).then(b.2.cmp(&a.2)));
    raw.dedup_by(|later, first| later.0 == first.0 && later.1 == first.1);

    let mut out: Vec<Span> = Vec::new();
    let mut emit = |start: usize, end: usize, kind: Option<HighlightKind>| {
        if start < end && kind.is_some() {
            match out.last_mut() {
                Some(last) if last.end == start && last.kind == kind => last.end = end,
                _ => out.push(Span { start, end, kind }),
            }
        }
    };
    // Pile des captures ouvertes : (fin, catégorie).
    let mut stack: Vec<(usize, Option<HighlightKind>)> = Vec::new();
    let mut position = 0usize;
    for (start, end, _, kind) in raw {
        while let Some(&(top_end, top_kind)) = stack.last() {
            if top_end <= start {
                emit(position, top_end, top_kind);
                position = position.max(top_end);
                stack.pop();
            } else {
                break;
            }
        }
        if let Some(&(_, top_kind)) = stack.last() {
            emit(position, start, top_kind);
        }
        position = position.max(start);
        // Chevauchement partiel (rare) : la nouvelle capture est rognée.
        let end = stack.last().map_or(end, |&(top_end, _)| end.min(top_end));
        stack.push((end, kind));
    }
    while let Some((top_end, top_kind)) = stack.pop() {
        emit(position, top_end, top_kind);
        position = position.max(top_end);
    }
    out
}

/// Garde la partie des plages qui tombe dans `ranges` (triées).
fn clip(spans: &[Span], ranges: &[Range<usize>]) -> Vec<Span> {
    if ranges.len() == 1
        && spans.first().is_none_or(|s| s.start >= ranges[0].start)
        && spans.last().is_none_or(|s| s.end <= ranges[0].end)
    {
        return spans.to_vec();
    }
    let mut out = Vec::with_capacity(spans.len());
    let mut r = 0;
    for span in spans {
        while r < ranges.len() && ranges[r].end <= span.start {
            r += 1;
        }
        let mut k = r;
        while k < ranges.len() && ranges[k].start < span.end {
            let start = span.start.max(ranges[k].start);
            let end = span.end.min(ranges[k].end);
            if start < end {
                out.push(Span {
                    start,
                    end,
                    kind: span.kind,
                });
            }
            k += 1;
        }
    }
    out
}

/// Pose les plages d'une zone imbriquée par-dessus celles de la couche hôte :
/// dans `ranges`, seules comptent celles de la zone.
fn overlay(base: &[Span], top: &[Span], ranges: &[Range<usize>]) -> Vec<Span> {
    // La base, privée de ce qui tombe dans les zones.
    let mut out: Vec<Span> = Vec::with_capacity(base.len() + top.len());
    let mut r = 0;
    for span in base {
        let mut start = span.start;
        while r < ranges.len() && ranges[r].end <= start {
            r += 1;
        }
        let mut k = r;
        while start < span.end {
            match ranges.get(k) {
                Some(range) if range.start < span.end => {
                    if start < range.start {
                        out.push(Span {
                            start,
                            end: range.start,
                            kind: span.kind,
                        });
                    }
                    start = start.max(range.end);
                    k += 1;
                }
                _ => {
                    out.push(Span {
                        start,
                        end: span.end,
                        kind: span.kind,
                    });
                    break;
                }
            }
        }
    }
    out.extend_from_slice(top);
    out.sort_by_key(|s| s.start);
    out
}

/// Zones imbriquées d'une couche : `(langage, plages)`, plages bornées à celles
/// de la couche.
fn find_injections(
    language: Language,
    tree: &Tree,
    rope: &Rope,
    layer_ranges: &[Range<usize>],
) -> Vec<(Language, Vec<Range<usize>>)> {
    let mut out = Vec::new();
    match language {
        Language::Php => {
            // Tout le HTML hors des balises PHP forme un seul document HTML.
            let mut ranges = Vec::new();
            collect_kind(tree.root_node(), "text", &mut ranges);
            if !ranges.is_empty() {
                out.push((Language::Html, ranges));
            }
        }
        Language::Html => {
            let mut stack = vec![tree.root_node()];
            while let Some(node) = stack.pop() {
                let target = match node.kind() {
                    "script_element" if script_is_javascript(node, rope) => {
                        Some(Language::JavaScript)
                    }
                    "style_element" => Some(Language::Css),
                    _ => None,
                };
                if let Some(target) = target {
                    let mut cursor = node.walk();
                    for child in node.named_children(&mut cursor) {
                        if child.kind() == "raw_text" {
                            let ranges = intersect(child.byte_range(), layer_ranges);
                            if !ranges.is_empty() {
                                out.push((target, ranges));
                            }
                        }
                    }
                    continue;
                }
                let mut cursor = node.walk();
                for child in node.named_children(&mut cursor) {
                    stack.push(child);
                }
            }
            out.sort_by_key(|(_, ranges)| ranges[0].start);
        }
        _ => {}
    }
    out
}

/// Plages des nœuds de type `kind` (sans descendre dedans).
fn collect_kind(node: Node, kind: &str, out: &mut Vec<Range<usize>>) {
    let mut cursor = node.walk();
    let mut stack = vec![node];
    while let Some(node) = stack.pop() {
        if node.kind() == kind {
            if !node.byte_range().is_empty() {
                out.push(node.byte_range());
            }
            continue;
        }
        // Enfants en ordre inverse pour dépiler dans l'ordre du texte.
        let children: Vec<Node> = node.named_children(&mut cursor).collect();
        stack.extend(children.into_iter().rev());
    }
}

fn intersect(range: Range<usize>, ranges: &[Range<usize>]) -> Vec<Range<usize>> {
    ranges
        .iter()
        .filter_map(|r| {
            let start = range.start.max(r.start);
            let end = range.end.min(r.end);
            (start < end).then_some(start..end)
        })
        .collect()
}

/// `<script>` sans `type`, ou d'un type JavaScript. Un `<script
/// type="text/ng-template">` (gabarit AngularJS) n'est pas du JavaScript.
fn script_is_javascript(script: Node, rope: &Rope) -> bool {
    let mut cursor = script.walk();
    let Some(start_tag) = script
        .named_children(&mut cursor)
        .find(|n| n.kind() == "start_tag")
    else {
        return true;
    };
    let mut cursor = start_tag.walk();
    for attribute in start_tag.named_children(&mut cursor) {
        if attribute.kind() != "attribute" {
            continue;
        }
        let mut inner = attribute.walk();
        let children: Vec<Node> = attribute.named_children(&mut inner).collect();
        let is_type = children.first().is_some_and(|name| {
            name.kind() == "attribute_name"
                && rope
                    .byte_slice(name.byte_range())
                    .to_string()
                    .eq_ignore_ascii_case("type")
        });
        if !is_type {
            continue;
        }
        let value = children
            .get(1)
            .map(|v| rope.byte_slice(v.byte_range()).to_string())
            .unwrap_or_default();
        let value = value
            .trim_matches(|c| c == '"' || c == '\'')
            .to_ascii_lowercase();
        return value.is_empty()
            || value.contains("javascript")
            || value.contains("ecmascript")
            || value == "module"
            || value.contains("babel")
            || value.contains("jsx");
    }
    true
}

/// Octets de la fin de ligne qui précède `next_start`.
fn terminator_len(rope: &Rope, line_start: usize, next_start: usize) -> usize {
    let len = next_start - line_start;
    if len == 0 {
        return 0;
    }
    match rope.byte(next_start - 1) {
        b'\n' if len >= 2 && rope.byte(next_start - 2) == b'\r' => 2,
        b'\n' | b'\r' => 1,
        _ => 0,
    }
}

/// Remplace les plages des lignes `range` par `spans` découpées par ligne (fin de
/// ligne exclue).
fn fill_lines(rope: &Rope, spans: &[Span], range: Range<usize>, lines: &mut LineHighlights) {
    let line_count = rope.len_lines().min(lines.len());
    for spans in lines
        .iter_mut()
        .take(range.end.min(line_count))
        .skip(range.start)
    {
        spans.clear();
    }
    let first_byte = rope.line_to_byte(range.start.min(line_count));
    let last_byte = if range.end < line_count {
        rope.line_to_byte(range.end)
    } else {
        rope.len_bytes()
    };
    for span in spans {
        if span.end <= first_byte || span.start >= last_byte {
            continue;
        }
        let span = Span {
            start: span.start.max(first_byte),
            end: span.end.min(last_byte),
            kind: span.kind,
        };
        let Some(kind) = span.kind else { continue };
        let end = span.end.min(rope.len_bytes());
        let mut start = span.start;
        if start >= end {
            continue;
        }
        let mut line = rope.byte_to_line(start);
        while start < end && line < line_count {
            let line_start = rope.line_to_byte(line);
            let next_start = if line + 1 < line_count {
                rope.line_to_byte(line + 1)
            } else {
                rope.len_bytes()
            };
            let piece_end = end.min(next_start - terminator_len(rope, line_start, next_start));
            if start < piece_end {
                lines[line].push(HighlightSpan {
                    start: (start - line_start) as u32,
                    end: (piece_end - line_start) as u32,
                    kind,
                });
            }
            start = next_start;
            line += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Texte des plages d'une ligne, avec leur catégorie.
    fn spans_of(text: &str, language: Language) -> Vec<(usize, String, HighlightKind)> {
        let lines = highlight_text(language, text);
        let source: Vec<&str> = text.split('\n').collect();
        let mut out = Vec::new();
        for (i, spans) in lines.iter().enumerate() {
            for span in spans {
                let line = source[i];
                let end = (span.end as usize).min(line.len());
                out.push((i, line[span.start as usize..end].to_owned(), span.kind));
            }
        }
        out
    }

    fn has(spans: &[(usize, String, HighlightKind)], text: &str, kind: HighlightKind) -> bool {
        spans.iter().any(|(_, t, k)| t.contains(text) && *k == kind)
    }

    #[test]
    fn every_query_compiles() {
        for language in [
            Language::Php,
            Language::Html,
            Language::Css,
            Language::JavaScript,
            Language::TypeScript,
            Language::Tsx,
        ] {
            assert!(config(language).is_some(), "{language:?}");
        }
    }

    #[test]
    fn php_with_html_script_and_style_shows_every_layer() {
        let text = "<html>\n<style>p { color: red; }</style>\n<?php\n// note\n$nom = strtoupper('été');\n?>\n<script>const x = 42;</script>\n</html>\n";
        let spans = spans_of(text, Language::Php);
        assert!(has(&spans, "html", HighlightKind::Tag), "{spans:?}");
        assert!(has(&spans, "p", HighlightKind::Selector), "{spans:?}");
        assert!(has(&spans, "color", HighlightKind::Property), "{spans:?}");
        assert!(has(&spans, "<?php", HighlightKind::Keyword), "{spans:?}");
        assert!(has(&spans, "// note", HighlightKind::Comment), "{spans:?}");
        assert!(has(&spans, "$nom", HighlightKind::Variable), "{spans:?}");
        assert!(
            has(&spans, "strtoupper", HighlightKind::Function),
            "{spans:?}"
        );
        assert!(has(&spans, "'été'", HighlightKind::String), "{spans:?}");
        assert!(has(&spans, "const", HighlightKind::Keyword), "{spans:?}");
        assert!(has(&spans, "42", HighlightKind::Number), "{spans:?}");
    }

    #[test]
    fn php_inside_an_html_attribute_is_not_painted_as_a_string() {
        let text = "<a href=\"<?= $url ?>\">x</a>\n";
        let spans = spans_of(text, Language::Php);
        assert!(has(&spans, "$url", HighlightKind::Variable), "{spans:?}");
        assert!(has(&spans, "a", HighlightKind::Tag), "{spans:?}");
    }

    #[test]
    fn javascript_and_jsx() {
        let text = "function f(a) { return <div className=\"x\">{a}</div>; }\n";
        let spans = spans_of(text, Language::JavaScript);
        assert!(has(&spans, "function", HighlightKind::Keyword), "{spans:?}");
        assert!(has(&spans, "f", HighlightKind::Function), "{spans:?}");
        assert!(has(&spans, "div", HighlightKind::Tag), "{spans:?}");
        assert!(
            has(&spans, "className", HighlightKind::Attribute),
            "{spans:?}"
        );
        // Les identifiants ordinaires ne sont pas colorés.
        assert!(!spans.iter().any(|(_, t, _)| t == "a"), "{spans:?}");
    }

    #[test]
    fn typescript_types() {
        let spans = spans_of("let n: number = 1;\ninterface A {}\n", Language::TypeScript);
        assert!(
            has(&spans, "interface", HighlightKind::Keyword),
            "{spans:?}"
        );
        assert!(has(&spans, "number", HighlightKind::Type), "{spans:?}");
        let spans = spans_of("const e = <B x={1} />;\n", Language::Tsx);
        assert!(has(&spans, "1", HighlightKind::Number), "{spans:?}");
    }

    #[test]
    fn a_template_script_is_not_javascript() {
        let text = "<script type=\"text/ng-template\">const x = 1;</script>\n";
        let spans = spans_of(text, Language::Html);
        assert!(!has(&spans, "const", HighlightKind::Keyword), "{spans:?}");
    }

    #[test]
    fn broken_code_is_still_highlighted() {
        let spans = spans_of("<?php\nfunction (\n$x = 'a\n", Language::Php);
        assert!(has(&spans, "function", HighlightKind::Keyword), "{spans:?}");
    }

    #[test]
    fn spans_spanning_lines_are_split_per_line() {
        let lines = highlight_text(Language::JavaScript, "/* un\ndeux */ x;\n");
        assert_eq!(
            lines[0],
            vec![HighlightSpan {
                start: 0,
                end: 5,
                kind: HighlightKind::Comment
            }]
        );
        assert_eq!(
            lines[1][0],
            HighlightSpan {
                start: 0,
                end: 7,
                kind: HighlightKind::Comment
            }
        );
    }

    #[test]
    fn incremental_update_matches_a_fresh_parse() {
        let mut buffer = crate::Buffer::new("<?php\n$a = 1;\n?>\n<p>x</p>\n");
        let mut highlighter = Highlighter::new(Language::Php);
        highlighter.update(buffer.rope(), 0);
        let selections = crate::Selections::default();
        buffer.apply(
            &[
                (6..6, "echo 'z';\n".into()),
                (19..19, " class=\"c\"".into()),
            ],
            crate::EditKind::Other,
            &selections,
            selections.clone(),
            std::time::Instant::now(),
        );
        highlighter.edit(&buffer.take_byte_edits());
        let (incremental, _) = highlighter.update(buffer.rope(), 1);
        assert_eq!(incremental, highlight_text(Language::Php, &buffer.text()));
    }

    /// Modifications au hasard (graine fixe) : après chacune, la coloration
    /// incrémentale doit être exactement celle d'une analyse complète.
    #[test]
    fn random_edits_keep_incremental_highlighting_exact() {
        let base = "<?php\n// début\n$a = 'x';\n?>\n<div class=\"c\">\n<script>\nconst x = `t${1}`;\n/* bloc */\n</script>\n<style>p { color: red; }</style>\n</div>\n<?php\nfunction f($b) {\n  return \"s\";\n}\n";
        let pieces = [
            "/*",
            "*/",
            "'",
            "\"",
            "\n",
            "<?php ",
            "?>",
            "<script>",
            "</script>",
            "{",
            "}",
            "x",
            "// c\n",
            "<!--",
            "-->",
            "`",
            "é",
            "<style>",
            "$v",
            "\n\n",
        ];
        for language in [
            Language::Php,
            Language::Html,
            Language::JavaScript,
            Language::Css,
        ] {
            let mut seed: u64 = 0x5eed_1234 ^ language as u64;
            let mut next = |bound: usize| {
                seed = seed
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                ((seed >> 33) as usize) % bound.max(1)
            };
            let mut buffer = crate::Buffer::new(base);
            let mut highlighter = Highlighter::new(language);
            highlighter.update(buffer.rope(), 0);
            let selections = crate::Selections::default();
            for step in 0..150 {
                let len = buffer.len_chars();
                let at = next(len + 1);
                let edit = if next(3) == 0 && len > 0 {
                    let end = (at + 1 + next(6)).min(len);
                    (at.min(end)..end, String::new())
                } else {
                    (at..at, pieces[next(pieces.len())].to_owned())
                };
                buffer.apply(
                    &[edit],
                    crate::EditKind::Other,
                    &selections,
                    selections.clone(),
                    std::time::Instant::now(),
                );
                highlighter.edit(&buffer.take_byte_edits());
                let (incremental, snapshot) = highlighter.update(buffer.rope(), buffer.version());
                let fresh = highlight_text(language, &buffer.text());
                if incremental != fresh {
                    let (_, fresh_snapshot) = Highlighter::new(language).update(buffer.rope(), 0);
                    let (a, b) = (snapshot.unwrap(), fresh_snapshot.unwrap());
                    eprintln!(
                        "racine identique : {}",
                        a.tree.root_node().to_sexp() == b.tree.root_node().to_sexp()
                    );
                    for (x, y) in a.injections.iter().zip(&b.injections) {
                        eprintln!(
                            "{:?} identique : {} ; plages {:?} / {:?}",
                            x.language,
                            x.tree.root_node().to_sexp() == y.tree.root_node().to_sexp(),
                            x.ranges == y.ranges,
                            x.ranges.len()
                        );
                    }
                }
                assert_eq!(
                    incremental,
                    fresh,
                    "{language:?}, étape {step}, texte {:?}",
                    buffer.text()
                );
            }
        }
    }

    #[test]
    fn resolve_keeps_inner_captures_and_the_last_pattern() {
        use HighlightKind as K;
        let spans = resolve(vec![
            (0, 10, 0, Some(K::String)),
            (2, 4, 1, Some(K::Escape)),
            (6, 8, 2, None),
            (0, 10, 3, Some(K::Comment)),
        ]);
        let kinds: Vec<_> = spans
            .iter()
            .map(|s| (s.start, s.end, s.kind.unwrap()))
            .collect();
        assert_eq!(
            kinds,
            vec![
                (0, 2, K::Comment),
                (2, 4, K::Escape),
                (4, 6, K::Comment),
                (8, 10, K::Comment)
            ]
        );
    }
}
