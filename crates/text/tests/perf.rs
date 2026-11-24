//! Mesures (pas des tests de régression) : `cargo test -p julie-text --release --test perf -- --ignored --nocapture`.

use std::time::{Duration, Instant};

use julie_core::TextPosition;
use julie_text::*;

fn php_file(lines: usize) -> String {
    let mut text = String::from("<?php\nnamespace App\\Mesure;\n\n");
    let mut n = 0;
    while text.lines().count() < lines {
        text.push_str(&format!(
            "/**\n * Classe {n}.\n */\nclass Classe{n} extends Base implements Contrat\n{{\n    private array $valeurs = [];\n    public const MAX_{n} = {n};\n\n    public function calcule(int $a, string $b = 'été'): ?string\n    {{\n        // commentaire {n}\n        if ($a > self::MAX_{n} && strlen($b) !== 0) {{\n            return sprintf(\"%s-%d\", $b, $a * 2.5);\n        }}\n        foreach ($this->valeurs as $cle => $valeur) {{\n            $this->valeurs[$cle] = $valeur . \"\\n\";\n        }}\n        return null;\n    }}\n}}\n?>\n<div class=\"bloc-{n}\"><?= htmlspecialchars($titre) ?></div>\n<script>const x{n} = {{ a: {n}, b: 'z' }};</script>\n<?php\n\n"
        ));
        n += 1;
    }
    text
}

fn stats(label: &str, mut samples: Vec<Duration>) {
    samples.sort();
    let total: Duration = samples.iter().sum();
    let mean = total / samples.len().max(1) as u32;
    let p95 = samples[(samples.len() * 95 / 100).min(samples.len() - 1)];
    let max = *samples.last().unwrap();
    eprintln!(
        "{label:<55} moyenne {mean:>10.2?}  p95 {p95:>10.2?}  max {max:>10.2?}  (n={})",
        samples.len()
    );
}

#[test]
#[ignore]
fn measure_20000_lines_php() {
    let text = php_file(20_000);
    let lines = text.lines().count();
    eprintln!("fichier : {lines} lignes, {} octets", text.len());

    let started = Instant::now();
    let mut document = Document::from_text_file(
        julie_core::file::decode(text.clone().into_bytes()),
        Some("mesure.php".into()),
    );
    eprintln!(
        "Document::from_text_file                                {:?}",
        started.elapsed()
    );

    let started = Instant::now();
    let mut highlighter = Highlighter::new(Language::Php);
    let (highlights, _) = highlighter.update(document.buffer().rope(), 0);
    eprintln!(
        "coloration complète (fil de fond)                       {:?}",
        started.elapsed()
    );
    assert_eq!(highlights.len(), document.buffer().len_lines());

    // Frappe au milieu du fichier : coût dans le fil de l'interface.
    document.enable_syntax(None);
    document.syntax_mut().unwrap().wait(Duration::from_secs(30));
    let middle = (lines / 2) as u32;
    let offset = document
        .buffer()
        .position_to_offset(TextPosition::new(middle, 8));
    document.set_cursor(offset, false);
    let mut typing = Vec::new();
    for c in "$variable = strtoupper('abc');".chars().cycle().take(300) {
        let started = Instant::now();
        document.insert_text(&c.to_string());
        typing.push(started.elapsed());
    }
    stats(
        "frappe d'un caractère (insert_text + envoi à la coloration)",
        typing,
    );

    let mut newline = Vec::new();
    for _ in 0..50 {
        let started = Instant::now();
        document.insert_newline();
        newline.push(started.elapsed());
    }
    stats("Entrée (saut de ligne + indentation)", newline);

    let started = Instant::now();
    assert!(document.syntax_mut().unwrap().wait(Duration::from_secs(30)));
    eprintln!(
        "attente de la coloration à jour après la frappe          {:?}",
        started.elapsed()
    );

    // Analyse incrémentale seule (fil de fond), une frappe à la fois.
    let mut buffer = Buffer::new(&text);
    let mut highlighter = Highlighter::new(Language::Php);
    highlighter.update(buffer.rope(), 0);
    let selections = Selections::default();
    let at = buffer.position_to_offset(TextPosition::new(middle, 8));
    let mut incremental = Vec::new();
    for i in 0..50 {
        buffer.apply(
            &[(at + i..at + i, "x".into())],
            EditKind::Typing,
            &selections,
            selections.clone(),
            Instant::now(),
        );
        let started = Instant::now();
        highlighter.edit(&buffer.take_byte_edits());
        highlighter.update(buffer.rope(), buffer.version());
        incremental.push(started.elapsed());
    }
    stats(
        "coloration incrémentale après une frappe (fil de fond)",
        incremental,
    );

    // Lecture des lignes visibles, comme l'éditeur à chaque image.
    let mut frame = Vec::new();
    for top in (0..lines - 60).step_by(997) {
        let started = Instant::now();
        let mut total = 0usize;
        for line in top..top + 60 {
            total += document.buffer().line(line).to_string().len();
            total += document.highlights(line).len();
        }
        assert!(total > 0);
        frame.push(started.elapsed());
    }
    stats("lecture de 60 lignes visibles + leurs couleurs", frame);

    let mut undo = Vec::new();
    for _ in 0..20 {
        let started = Instant::now();
        document.undo();
        undo.push(started.elapsed());
    }
    stats("annuler une étape", undo);
}

#[test]
#[ignore]
fn measure_a_100000_character_line() {
    let mut line = String::new();
    let mut i = 0;
    while line.len() < 100_000 {
        line.push_str(&format!("var a{i}=function(b){{return b+{i}}};"));
        i += 1;
    }
    let text = format!("{line}\nconsole.log(1);\n");
    let mut document = Document::from_text_file(
        julie_core::file::decode(text.clone().into_bytes()),
        Some("min.js".into()),
    );
    let started = Instant::now();
    let highlights = highlight_text(Language::JavaScript, &text);
    eprintln!(
        "coloration d'une ligne de {} caractères        {:?} ({} plages)",
        line.len(),
        started.elapsed(),
        highlights[0].len()
    );

    document.enable_syntax(None);
    assert!(document.syntax_mut().unwrap().wait(Duration::from_secs(30)));
    document.set_cursor(50_000, false);
    let mut typing = Vec::new();
    for _ in 0..200 {
        let started = Instant::now();
        document.insert_text("x");
        typing.push(started.elapsed());
    }
    stats("frappe au milieu de la ligne", typing);
    let mut moves = Vec::new();
    for motion in [
        Motion::Right,
        Motion::WordRight,
        Motion::Left,
        Motion::Down,
        Motion::Up,
        Motion::LineEnd,
        Motion::LineStart,
    ] {
        for _ in 0..20 {
            let started = Instant::now();
            document.move_cursor(motion, false);
            moves.push(started.elapsed());
        }
    }
    stats("déplacements du curseur dans la ligne", moves);

    // Ce que l'éditeur extrait pour une image : une fenêtre de ~600 caractères.
    let mut window = Vec::new();
    for start in (0..99_000).step_by(9_000) {
        let started = Instant::now();
        let content = document.buffer().line(0);
        let s = content.slice(start..start + 600).to_string();
        let byte = content.char_to_byte(start);
        let spans = document.highlights(0);
        let first = spans.partition_point(|sp| (sp.end as usize) <= byte);
        assert!(!s.is_empty() && first <= spans.len());
        window.push(started.elapsed());
    }
    stats("extraction d'une fenêtre visible de 600 caractères", window);
}
