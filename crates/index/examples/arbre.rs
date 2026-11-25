//! Affiche l'arbre tree-sitter d'un fichier (outil de mise au point de
//! l'extraction des symboles).
//!
//! ```text
//! cargo run -p julie-index --example arbre -- chemin\du\fichier.php
//! ```

fn main() {
    let path = std::path::PathBuf::from(std::env::args().nth(1).expect("chemin du fichier"));
    let text = std::fs::read_to_string(&path).expect("lecture");
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let language: tree_sitter::Language = match ext.as_str() {
        "php" => tree_sitter_php::LANGUAGE_PHP.into(),
        "ts" => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        "tsx" => tree_sitter_typescript::LANGUAGE_TSX.into(),
        _ => tree_sitter_javascript::LANGUAGE.into(),
    };
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&language).expect("grammaire");
    let tree = parser.parse(&text, None).expect("analyse");
    print(tree.root_node(), &text, 0, None);
}

fn print(node: tree_sitter::Node, text: &str, depth: usize, field: Option<&str>) {
    let label = match field {
        Some(f) => format!("{f}: {}", node.kind()),
        None => node.kind().to_string(),
    };
    let snippet = if node.child_count() == 0 || node.kind().ends_with("name") {
        format!(
            " {:?}",
            text[node.byte_range()].chars().take(40).collect::<String>()
        )
    } else {
        String::new()
    };
    if node.is_named() {
        println!("{}{label}{snippet}", "  ".repeat(depth));
    }
    let mut cursor = node.walk();
    for (i, child) in node.children(&mut cursor).enumerate() {
        let f = node.field_name_for_child(i as u32);
        print(child, text, depth + 1, f);
    }
}
