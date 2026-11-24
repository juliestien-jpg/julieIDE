//! Traces d'échec et liens `fichier:ligne`.
//!
//! PHP numérote les lignes à partir de 1 ; [`julie_core::Location`] à partir de 0 :
//! la conversion est faite ici, une seule fois.

use std::ops::Range;
use std::path::{Path, PathBuf};

use julie_core::{Location, TextPosition};

/// Une ligne de trace, avec l'endroit où sauter quand elle en désigne un.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceEntry {
    /// La ligne telle que PHPUnit l'a donnée, sans espaces de bord.
    pub text: String,
    pub location: Option<Location>,
}

/// Un lien trouvé dans un texte libre (console).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextLink {
    /// Octets du texte de départ couverts par le lien (`chemin:ligne`).
    pub range: Range<usize>,
    pub location: Location,
}

/// Découpe la trace (`details` d'un message TeamCity) en lignes, chacune avec son
/// lien. Formes reconnues : `chemin:12` (PHPUnit), `#0 chemin(12): appel()`
/// (trace PHP complète). Un chemin relatif se lit depuis `root`.
pub fn parse_trace(details: &str, root: &Path) -> Vec<TraceEntry> {
    details
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| TraceEntry {
            text: line.to_owned(),
            location: location_of_trace_line(line, root),
        })
        .collect()
}

fn location_of_trace_line(line: &str, root: &Path) -> Option<Location> {
    let (path, number) = split_location(line).or_else(|| split_call_frame(line))?;
    Some(location(path, number, root))
}

/// `chemin:12` -> (`chemin`, 12). Tolère `chemin:12:5` (colonne ignorée). Le chemin
/// doit ressembler à un chemin (séparateur ou extension), pour écarter `Total:12`.
pub fn split_location(text: &str) -> Option<(&str, u32)> {
    let text = text.trim();
    let (head, last) = text.rsplit_once(':')?;
    let (path, number) = if is_number(last) {
        // `chemin:12:5` ? Alors `head` finit par `:12`.
        match head.rsplit_once(':') {
            Some((path, line)) if is_number(line) && looks_like_path(path) => (path, line),
            _ => (head, last),
        }
    } else {
        return None;
    };
    if !looks_like_path(path) {
        return None;
    }
    Some((path, number.parse().ok()?))
}

/// `#0 C:\a\b.php(12): Classe->methode()` -> (`C:\a\b.php`, 12).
fn split_call_frame(line: &str) -> Option<(&str, u32)> {
    let rest = line.strip_prefix('#')?;
    let rest = rest
        .trim_start_matches(|c: char| c.is_ascii_digit())
        .trim_start();
    // Première parenthèse qui encadre des chiffres seuls : `(x86)` n'en est pas une.
    let mut search_from = 0;
    while let Some(open) = rest[search_from..].find('(') {
        let open = search_from + open;
        let after = &rest[open + 1..];
        if let Some(close) = after.find(')') {
            if is_number(&after[..close]) {
                return Some((&rest[..open], after[..close].parse().ok()?));
            }
        }
        search_from = open + 1;
    }
    None
}

fn is_number(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit())
}

fn looks_like_path(path: &str) -> bool {
    path.len() >= 2 && (path.contains(['\\', '/']) || path.contains('.'))
}

fn location(path: &str, line: u32, root: &Path) -> Location {
    let path = Path::new(path);
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    Location {
        path,
        position: TextPosition::new(line.saturating_sub(1), 0),
    }
}

/// Repère dans un texte libre (la console d'un nœud) les `chemin:ligne` qui
/// désignent un fichier existant. `exists` dit si un chemin absolu est un fichier :
/// l'appelant décide d'où vient la réponse (disque, cache).
///
/// Les chemins avec espaces (`C:\Program Files (x86)\...`) sont reconnus quand ils
/// commencent par une lettre de lecteur ; un chemin relatif ne contient pas
/// d'espace.
pub fn find_links(text: &str, root: &Path, exists: impl Fn(&Path) -> bool) -> Vec<TextLink> {
    let bytes = text.as_bytes();
    let mut links: Vec<TextLink> = Vec::new();
    let mut previous_end = 0;
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b':' || !bytes.get(index + 1).is_some_and(u8::is_ascii_digit) {
            index += 1;
            continue;
        }
        let colon = index;
        let mut digits_end = colon + 1;
        while bytes.get(digits_end).is_some_and(u8::is_ascii_digit) {
            digits_end += 1;
        }
        // `12abc` n'est pas un numéro de ligne.
        if bytes
            .get(digits_end)
            .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')
        {
            index = digits_end;
            continue;
        }
        let Ok(number) = text[colon + 1..digits_end].parse::<u32>() else {
            index = digits_end;
            continue;
        };
        // Même ligne de texte seulement.
        let line_start = text[..colon].rfind('\n').map_or(0, |at| at + 1);
        let window_start = line_start.max(previous_end);
        if let Some((start, resolved)) = link_start(text, window_start, colon, root, &exists) {
            links.push(TextLink {
                range: start..digits_end,
                location: Location {
                    path: resolved,
                    position: TextPosition::new(number.saturating_sub(1), 0),
                },
            });
            previous_end = digits_end;
        }
        index = digits_end;
    }
    links
}

/// Cherche où commence le chemin qui finit en `colon`, en essayant d'abord les
/// départs par lettre de lecteur (du plus à gauche au plus à droite : les chemins
/// avec espaces), puis le mot qui précède.
fn link_start(
    text: &str,
    window_start: usize,
    colon: usize,
    root: &Path,
    exists: &impl Fn(&Path) -> bool,
) -> Option<(usize, PathBuf)> {
    let window = &text[window_start..colon];
    let bytes = window.as_bytes();
    let mut candidates: Vec<usize> = Vec::new();
    for at in 0..bytes.len().saturating_sub(2) {
        let is_drive = bytes[at].is_ascii_alphabetic()
            && bytes[at + 1] == b':'
            && matches!(bytes[at + 2], b'\\' | b'/')
            && (at == 0 || !bytes[at - 1].is_ascii_alphanumeric());
        if is_drive {
            candidates.push(window_start + at);
        }
    }
    // Le mot qui précède : après le dernier espace, guillemet ou parenthèse ouvrante.
    let word_start = window
        .rfind(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '(' | '[' | '=' | '<' | '>'))
        .map_or(0, |at| {
            at + window[at..].chars().next().map_or(1, char::len_utf8)
        });
    let word_start = window_start + word_start;
    if !candidates.contains(&word_start) {
        candidates.push(word_start);
    }
    for start in candidates {
        let raw = &text[start..colon];
        if raw.is_empty() {
            continue;
        }
        let path = Path::new(raw);
        let resolved = if path.is_absolute() {
            path.to_path_buf()
        } else {
            root.join(path)
        };
        if exists(&resolved) {
            return Some((start, resolved));
        }
    }
    None
}

/// Rend le texte d'une valeur « exportée » par PHPUnit dans `expected` / `actual` :
/// une chaîne y est entre apostrophes, avec ses sauts de ligne écrits `\n` suivis
/// du vrai saut de ligne. Le résultat est le texte d'origine, bon pour un diff.
/// Toute autre valeur (nombre, tableau, objet) est rendue telle quelle.
pub fn unexport(value: &str) -> String {
    let Some(inner) = value
        .strip_prefix('\'')
        .and_then(|rest| rest.strip_suffix('\''))
    else {
        return value.to_owned();
    };
    let mut result = String::with_capacity(inner.len());
    let mut rest = inner;
    while !rest.is_empty() {
        // Dans l'ordre de l'exportateur : `\r\n`, `\n\r`, `\r`, `\n`, chacun suivi
        // d'un vrai saut de ligne.
        let mut matched = false;
        for (escaped, real) in [
            ("\\r\\n\n", "\r\n"),
            ("\\n\\r\n", "\n\r"),
            ("\\r\n", "\r"),
            ("\\n\n", "\n"),
        ] {
            if let Some(after) = rest.strip_prefix(escaped) {
                result.push_str(real);
                rest = after;
                matched = true;
                break;
            }
        }
        if !matched {
            let ch = rest.chars().next().unwrap_or_default();
            result.push(ch);
            rest = &rest[ch.len_utf8()..];
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT: &str = r"C:\proj";

    #[test]
    fn split_location_handles_windows_paths_and_rejects_non_paths() {
        assert_eq!(
            split_location(r"C:\Program Files (x86)\x\Test.php:25"),
            Some((r"C:\Program Files (x86)\x\Test.php", 25))
        );
        assert_eq!(
            split_location("tests/FooTest.php:7:3"),
            Some(("tests/FooTest.php", 7))
        );
        assert_eq!(split_location("Total:12"), None);
        assert_eq!(split_location("pas de ligne"), None);
        assert_eq!(split_location("a.php:"), None);
        assert_eq!(split_location(":12"), None);
    }

    #[test]
    fn trace_lines_become_zero_based_locations() {
        let details = " C:\\Program Files (x86)\\x\\Test.php:25\n \n";
        let trace = parse_trace(details, Path::new(ROOT));
        assert_eq!(trace.len(), 1);
        let location = trace[0].location.as_ref().unwrap();
        assert_eq!(
            location.path,
            PathBuf::from(r"C:\Program Files (x86)\x\Test.php")
        );
        assert_eq!(location.position, TextPosition::new(24, 0));
    }

    #[test]
    fn relative_paths_are_read_from_the_project_root() {
        let trace = parse_trace("tests/FooTest.php:3", Path::new(ROOT));
        let location = trace[0].location.as_ref().unwrap();
        assert_eq!(location.path, Path::new(ROOT).join("tests/FooTest.php"));
        assert_eq!(location.position.line, 2);
    }

    #[test]
    fn full_php_traces_are_understood() {
        let details = "#0 C:\\a (x86)\\b.php(12): Foo->bar()\n#1 {main}\n";
        let trace = parse_trace(details, Path::new(ROOT));
        assert_eq!(trace.len(), 2);
        let location = trace[0].location.as_ref().unwrap();
        assert_eq!(location.path, PathBuf::from(r"C:\a (x86)\b.php"));
        assert_eq!(location.position.line, 11);
        assert!(trace[1].location.is_none());
        assert_eq!(trace[1].text, "#1 {main}");
    }

    fn exists_in(files: &'static [&'static str]) -> impl Fn(&Path) -> bool {
        move |path| files.iter().any(|file| Path::new(file) == path)
    }

    #[test]
    fn links_are_found_in_free_text_only_for_existing_files() {
        let text = "Erreur dans C:\\Program Files (x86)\\p\\a.php:12 puis tests/b.php:5 et c.php:9";
        let links = find_links(
            text,
            Path::new(ROOT),
            exists_in(&[r"C:\Program Files (x86)\p\a.php", r"C:\proj\tests/b.php"]),
        );
        assert_eq!(links.len(), 2);
        assert_eq!(
            &text[links[0].range.clone()],
            "C:\\Program Files (x86)\\p\\a.php:12"
        );
        assert_eq!(links[0].location.position.line, 11);
        assert_eq!(&text[links[1].range.clone()], "tests/b.php:5");
        assert_eq!(links[1].location.path, Path::new(ROOT).join("tests/b.php"));
    }

    #[test]
    fn two_drive_paths_on_one_line_do_not_swallow_each_other() {
        let text = r"C:\a.php:1 et C:\b.php:2";
        let links = find_links(
            text,
            Path::new(ROOT),
            exists_in(&[r"C:\a.php", r"C:\b.php"]),
        );
        assert_eq!(links.len(), 2);
        assert_eq!(&text[links[0].range.clone()], r"C:\a.php:1");
        assert_eq!(&text[links[1].range.clone()], r"C:\b.php:2");
    }

    #[test]
    fn a_time_or_a_word_followed_by_digits_is_not_a_link() {
        let links = find_links(
            "à 12:30 le fichier:12abc",
            Path::new(ROOT),
            exists_in(&[r"C:\proj\fichier"]),
        );
        assert!(links.is_empty(), "{links:?}");
    }

    #[test]
    fn unexport_restores_strings_and_leaves_other_values() {
        assert_eq!(unexport("'attendu'"), "attendu");
        assert_eq!(
            unexport("'ligne 1\\n\nligne 2\\n\nligne 3'"),
            "ligne 1\nligne 2\nligne 3"
        );
        assert_eq!(unexport("'a\\r\\n\nb'"), "a\r\nb");
        assert_eq!(unexport("'un \\n littéral'"), "un \\n littéral");
        assert_eq!(unexport("4"), "4");
        assert_eq!(
            unexport("Array &0 (\n    0 => 1\n)"),
            "Array &0 (\n    0 => 1\n)"
        );
        assert_eq!(unexport("'"), "'");
        assert_eq!(unexport("''"), "");
    }
}
