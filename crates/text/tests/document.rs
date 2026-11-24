//! Commandes d'édition et aller-retour des fichiers.

use std::cell::Cell;
use std::fs;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use julie_core::file::{Encoding, LineEnding};
use julie_core::TextPosition;
use julie_text::*;

/// Document dont l'horloge avance à la main.
fn doc(text: &str) -> (Document, Arc<Mutex<Instant>>) {
    let mut document = Document::new(text);
    let now = Arc::new(Mutex::new(Instant::now()));
    let clock = now.clone();
    document.set_clock(move || *clock.lock().unwrap());
    (document, now)
}

fn at(document: &mut Document, line: u32, column: u32) {
    let offset = document
        .buffer()
        .position_to_offset(TextPosition::new(line, column));
    document.set_cursor(offset, false);
}

fn type_text(document: &mut Document, text: &str) {
    for c in text.chars() {
        document.insert_text(&c.to_string());
    }
}

#[test]
fn typing_abc_then_undo_removes_it_whole_and_redo_puts_it_back() {
    let (mut d, _) = doc("x\n");
    type_text(&mut d, "abc");
    assert_eq!(d.text(), "abcx\n");
    assert!(d.is_modified());
    assert!(d.undo());
    assert_eq!(d.text(), "x\n");
    assert!(!d.is_modified());
    assert!(d.redo());
    assert_eq!(d.text(), "abcx\n");
    assert_eq!(d.cursor_position(), TextPosition::new(0, 3));
}

#[test]
fn a_pause_a_cursor_move_or_a_newline_closes_the_step() {
    let (mut d, now) = doc("");
    type_text(&mut d, "ab");
    *now.lock().unwrap() += Duration::from_millis(1500);
    type_text(&mut d, "cd");
    d.move_cursor(Motion::Left, false);
    d.move_cursor(Motion::Right, false);
    type_text(&mut d, "ef");
    d.insert_newline();
    type_text(&mut d, "gh");
    assert_eq!(d.text(), "abcdef\ngh");
    let mut states = vec![];
    while d.undo() {
        states.push(d.text());
    }
    assert_eq!(states, ["abcdef\n", "abcdef", "abcd", "ab", ""]);
}

#[test]
fn undo_puts_the_cursor_back_where_the_change_was() {
    let (mut d, _) = doc("un\ndeux\ntrois\n");
    at(&mut d, 2, 5);
    type_text(&mut d, "!");
    at(&mut d, 0, 0);
    d.undo();
    assert_eq!(d.cursor_position(), TextPosition::new(2, 5));
}

#[test]
fn enter_keeps_the_indentation_and_opens_braces() {
    let (mut d, _) = doc("    if ($a) {}\n");
    at(&mut d, 0, 13);
    d.insert_newline();
    assert_eq!(d.text(), "    if ($a) {\n        \n    }\n");
    assert_eq!(d.cursor_position(), TextPosition::new(1, 8));
    type_text(&mut d, "x;");
    d.insert_newline();
    assert_eq!(d.cursor_position(), TextPosition::new(2, 8));
    // Un « } » tapé sur une ligne blanche retire un cran.
    type_text(&mut d, "}");
    assert_eq!(d.buffer().line(2).to_string(), "    }");
}

#[test]
fn tab_inserts_the_file_indentation() {
    let (mut d, _) = doc("<?php\n\tif (1) {\n\t\techo 1;\n\t}\n");
    assert_eq!(d.indent_style(), IndentStyle::Tabs);
    at(&mut d, 4, 0);
    d.indent();
    assert_eq!(d.buffer().line(4).to_string(), "\t");

    let (mut d, _) = doc("a\n  b\n    c\n");
    assert_eq!(d.indent_style(), IndentStyle::Spaces(2));
    at(&mut d, 0, 1);
    d.indent();
    assert_eq!(d.buffer().line(0).to_string(), "a ");

    let (d, _) = doc("x\n");
    assert_eq!(d.indent_style(), IndentStyle::Spaces(4));
}

#[test]
fn tab_and_shift_tab_on_a_selection_indent_the_lines() {
    let (mut d, _) = doc("a\nb\nc\n");
    d.set_selections(Selections::single(Selection::new(0, 3)));
    d.indent();
    assert_eq!(d.text(), "    a\n    b\nc\n");
    d.outdent();
    assert_eq!(d.text(), "a\nb\nc\n");
}

#[test]
fn ctrl_y_deletes_the_line() {
    let (mut d, _) = doc("un\ndeux\ntrois");
    at(&mut d, 1, 3);
    d.delete_lines();
    assert_eq!(d.text(), "un\ntrois");
    assert_eq!(d.cursor_position(), TextPosition::new(1, 3));
    d.delete_lines();
    assert_eq!(d.text(), "un");
    assert_eq!(d.cursor_position().line, 0);
    d.undo();
    d.undo();
    assert_eq!(d.text(), "un\ndeux\ntrois");
}

#[test]
fn duplicate_and_move_lines() {
    let (mut d, _) = doc("a\nb\nc\n");
    at(&mut d, 1, 1);
    d.duplicate();
    assert_eq!(d.text(), "a\nb\nb\nc\n");
    assert_eq!(d.cursor_position(), TextPosition::new(2, 1));
    d.move_lines(true);
    d.move_lines(true);
    assert_eq!(d.text(), "b\na\nb\nc\n");
    assert_eq!(d.cursor_position(), TextPosition::new(0, 1));
    assert!(!d.move_lines(true));
    // La dernière ligne sans fin de ligne reste sans fin de ligne.
    let (mut d, _) = doc("a\nb");
    at(&mut d, 1, 0);
    d.move_lines(true);
    assert_eq!(d.text(), "b\na");
}

#[test]
fn comment_toggling() {
    let (mut d, _) = doc("<?php\necho 1;\necho 2;\n");
    let mut d2 = Document::from_text_file(
        julie_core::file::decode(b"echo 1;\necho 2;\n".to_vec()),
        Some("a.php".into()),
    );
    d2.set_selections(Selections::single(Selection::new(0, 15)));
    d2.toggle_line_comment();
    assert_eq!(d2.text(), "//echo 1;\n//echo 2;\n");
    d2.toggle_line_comment();
    assert_eq!(d2.text(), "echo 1;\necho 2;\n");
    // Texte brut : rien.
    assert!(!d.toggle_line_comment());

    let mut css = Document::from_text_file(
        julie_core::file::decode(b"  p { }\n".to_vec()),
        Some("a.css".into()),
    );
    css.toggle_line_comment();
    assert_eq!(css.text(), "  /*p { }*/\n");
    // Le curseur est passé à la ligne suivante.
    assert_eq!(css.cursor_position().line, 1);
    at(&mut css, 0, 0);
    css.toggle_line_comment();
    assert_eq!(css.text(), "  p { }\n");
}

#[test]
fn copy_and_cut_take_the_whole_line_without_a_selection() {
    let (mut d, _) = doc("un\ndeux\n");
    at(&mut d, 1, 2);
    assert_eq!(d.copy_text().as_deref(), Some("deux\n"));
    assert_eq!(d.cut().as_deref(), Some("deux\n"));
    assert_eq!(d.text(), "un\n");
    d.set_selections(Selections::single(Selection::new(0, 2)));
    assert_eq!(d.copy_text().as_deref(), Some("un"));
}

#[test]
fn paste_is_one_step_and_line_endings_follow_the_file() {
    let (mut d, _) = doc("");
    type_text(&mut d, "a");
    d.paste("x\r\ny\rz");
    assert_eq!(d.text(), "ax\ny\nz");
    d.undo();
    assert_eq!(d.text(), "a");

    // Fichier mélangé à dominante CRLF : le collage prend des CRLF.
    let file = julie_core::file::decode(b"a\r\nb\r\nc\n".to_vec());
    assert_eq!(file.format.line_ending, LineEnding::Mixed);
    let mut d = Document::from_text_file(file, None);
    d.set_cursor(d.buffer().len_chars(), false);
    d.paste("x\ny");
    assert_eq!(d.text(), "a\r\nb\r\nc\nx\r\ny");
    d.insert_newline();
    assert!(d.text().ends_with("y\r\n"));
}

#[test]
fn backspace_crosses_crlf_and_graphemes_whole() {
    let file = julie_core::file::decode(b"ab\r\ncd\n".to_vec());
    let mut d = Document::from_text_file(file, None);
    at(&mut d, 1, 0);
    d.backspace();
    assert_eq!(d.text(), "abcd\n");

    let (mut d, _) = doc("e\u{301}x");
    at(&mut d, 0, 2);
    d.backspace();
    assert_eq!(d.text(), "x");
}

#[test]
fn vertical_moves_keep_the_goal_column() {
    let (mut d, _) = doc("abcdef\nab\nabcdef\n\tx\n");
    at(&mut d, 0, 5);
    d.move_cursor(Motion::Down, false);
    assert_eq!(d.cursor_position(), TextPosition::new(1, 2));
    d.move_cursor(Motion::Down, false);
    assert_eq!(d.cursor_position(), TextPosition::new(2, 5));
    // Tabulation : colonne visuelle 5 = après « \tx ».
    d.move_cursor(Motion::Down, false);
    assert_eq!(d.cursor_position(), TextPosition::new(3, 2));
    d.move_cursor(Motion::Up, true);
    assert_eq!(d.selections().primary().range().len(), 4);
}

#[test]
fn smart_home() {
    let (mut d, _) = doc("    echo;");
    at(&mut d, 0, 7);
    d.move_cursor(Motion::LineStart, false);
    assert_eq!(d.cursor_position().column, 4);
    d.move_cursor(Motion::LineStart, false);
    assert_eq!(d.cursor_position().column, 0);
    d.move_cursor(Motion::LineStart, false);
    assert_eq!(d.cursor_position().column, 4);
}

#[test]
fn multiple_cursors_type_together_and_undo_together() {
    let (mut d, _) = doc("a\nb\nc\n");
    d.set_selections(Selections::from_vec(
        vec![
            Selection::cursor(1),
            Selection::cursor(3),
            Selection::cursor(5),
        ],
        0,
    ));
    type_text(&mut d, ";;");
    assert_eq!(d.text(), "a;;\nb;;\nc;;\n");
    d.backspace();
    assert_eq!(d.text(), "a;\nb;\nc;\n");
    assert_eq!(d.selections().len(), 3);
    d.undo();
    assert_eq!(d.text(), "a;;\nb;;\nc;;\n");
}

#[test]
fn matching_brackets() {
    let (d, _) = doc("f(a[1], {b})");
    assert_eq!(d.matching_bracket(2), Some((1, 11)));
    assert_eq!(d.matching_bracket(12), Some((11, 1)));
    assert_eq!(d.matching_bracket(4), Some((3, 5)));
}

#[test]
fn read_only_documents_refuse_edits() {
    let (mut d, _) = doc("abc");
    d.set_read_only(true);
    assert!(!d.insert_text("x"));
    assert!(!d.delete_lines());
    assert!(d.cut().is_none());
    assert_eq!(d.text(), "abc");
    // On peut toujours sélectionner et copier.
    d.select_all();
    assert_eq!(d.copy_text().as_deref(), Some("abc"));
}

// ----- Fichiers --------------------------------------------------------------

/// Ouvre, modifie la ligne `line`, enregistre : en dehors de cette ligne les octets
/// sont ceux d'origine.
fn edit_line_and_save(original: &[u8], line: u32) -> Vec<u8> {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("f.php");
    fs::write(&path, original).unwrap();
    let mut d = Document::open(&path).unwrap();
    assert!(!d.is_modified());
    at(&mut d, line, 0);
    d.insert_text("X");
    assert!(d.is_modified());
    d.save().unwrap();
    assert!(!d.is_modified());
    fs::read(&path).unwrap()
}

#[test]
fn saving_preserves_bytes_outside_the_edited_lines() {
    assert_eq!(
        edit_line_and_save(b"a\r\nb\r\nc\r\n", 1),
        b"a\r\nXb\r\nc\r\n"
    );
    assert_eq!(
        edit_line_and_save(b"\xEF\xBB\xBFa\nb\n", 0),
        b"\xEF\xBB\xBFXa\nb\n"
    );
    assert_eq!(edit_line_and_save(b"a\r\nb\nc\rd", 2), b"a\r\nb\nXc\rd");
    assert_eq!(edit_line_and_save(b"a\nb", 1), b"a\nXb");
    assert_eq!(edit_line_and_save(b"", 0), b"X");
}

#[test]
fn windows_1252_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ancien.php");
    fs::write(&path, b"caf\xE9\r\n").unwrap();
    let mut d = Document::open(&path).unwrap();
    assert_eq!(d.text(), "café\n");
    assert_eq!(d.format().encoding, Encoding::Windows1252);
    at(&mut d, 0, 4);
    d.insert_text("é");
    d.save().unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"caf\xE9\xE9\r\n");

    // Un caractère hors de Windows-1252 : refus, document toujours modifié.
    d.insert_text("→");
    let error = d.save().unwrap_err();
    assert!(matches!(
        error,
        TextError::Core(julie_core::Error::Unencodable {
            character: '→', ..
        })
    ));
    assert!(d.is_modified());
    assert_eq!(fs::read(&path).unwrap(), b"caf\xE9\xE9\r\n");
}

#[test]
fn undefined_windows_1252_bytes_open_read_only() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bizarre.txt");
    fs::write(&path, b"a\x81b").unwrap();
    let d = Document::open(&path).unwrap();
    assert_eq!(d.read_only(), Some(ReadOnlyReason::UndefinedBytes));
}

#[test]
fn binary_and_huge_files_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("image.png");
    fs::write(&path, b"\x89PNG\x00\x00").unwrap();
    assert!(matches!(
        Document::open(&path),
        Err(TextError::Binary { .. })
    ));
    assert!(matches!(
        Document::open(&dir.path().join("absent.php")),
        Err(TextError::Core(julie_core::Error::Io { .. }))
    ));
}

#[test]
fn read_only_file_on_disk_keeps_the_document_modified() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("protégé.php");
    fs::write(&path, b"a").unwrap();
    let mut d = Document::open(&path).unwrap();
    d.insert_text("b");
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&path, permissions.clone()).unwrap();
    assert!(d.save().is_err());
    assert!(d.is_modified());
    #[allow(clippy::permissions_set_readonly_false)]
    permissions.set_readonly(false);
    fs::set_permissions(&path, permissions).unwrap();
}

#[test]
fn disk_changes_are_detected_and_reload_keeps_the_line() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("f.js");
    fs::write(&path, b"un\ndeux\ntrois\n").unwrap();
    let mut d = Document::open(&path).unwrap();
    assert_eq!(check_disk(&path, d.disk_stamp()), DiskState::Unchanged);
    at(&mut d, 2, 3);
    fs::write(&path, b"un\ndeux\n").unwrap();
    assert_eq!(check_disk(&path, d.disk_stamp()), DiskState::Changed);
    d.reload(load_file(&path).unwrap());
    assert_eq!(d.text(), "un\ndeux\n");
    assert_eq!(d.cursor_position(), TextPosition::new(2, 0));
    assert!(!d.is_modified());
    assert!(!d.undo());
    assert_eq!(check_disk(&path, d.disk_stamp()), DiskState::Unchanged);
    // Après notre propre enregistrement, pas de faux « changé ».
    d.insert_text("x");
    d.save().unwrap();
    assert_eq!(check_disk(&path, d.disk_stamp()), DiskState::Unchanged);
    fs::remove_file(&path).unwrap();
    assert_eq!(check_disk(&path, d.disk_stamp()), DiskState::Deleted);
}

#[test]
fn background_load_and_save_publish_events() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("f.css");
    fs::write(&path, b"p {}\r\n").unwrap();
    let bus = julie_core::Bus::new();
    let tasks = julie_core::Tasks::new(bus.publisher());
    spawn_load(&tasks, path.clone()).join();
    let mut document = None;
    for event in bus.drain() {
        if let julie_core::Event::Domain(event) = event {
            if let Ok(TextEvent::Loaded(loaded)) = event.take::<TextEvent>() {
                document = Some(Document::from_loaded(loaded));
            }
        }
    }
    let mut document = document.expect("Loaded publié");
    assert_eq!(document.language(), Language::Css);
    document.insert_text("a ");
    spawn_save(&tasks, document.save_request().unwrap()).join();
    let saved = Cell::new(false);
    for event in bus.drain() {
        if let julie_core::Event::Domain(event) = event {
            if let Ok(TextEvent::Saved(outcome)) = event.take::<TextEvent>() {
                document.mark_saved(&outcome);
                saved.set(true);
            }
        }
    }
    assert!(saved.get());
    assert!(!document.is_modified());
    assert_eq!(fs::read(&path).unwrap(), b"a p {}\r\n");
}

#[test]
fn syntax_follows_the_document() {
    let mut d = Document::from_text_file(
        julie_core::file::decode(b"<?php\n$a = 1;\n".to_vec()),
        Some("x.php".into()),
    );
    d.enable_syntax(None);
    at(&mut d, 2, 0);
    type_text(&mut d, "echo 'x';");
    let session = d.syntax_mut().unwrap();
    assert!(session.wait(Duration::from_secs(10)));
    let kinds: Vec<_> = d.highlights(2).iter().map(|s| s.kind).collect();
    assert!(kinds.contains(&HighlightKind::Keyword), "{kinds:?}");
    assert!(kinds.contains(&HighlightKind::String), "{kinds:?}");
}
