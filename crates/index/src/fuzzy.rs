//! « Aller au fichier » : correspondance floue des chemins et classement.
//!
//! Règles (docs/specs/recherche.md) : sans casse ; les caractères de la requête
//! apparaissent dans l'ordre dans le chemin relatif ; `/` et `\` font
//! correspondre les dossiers ; les espaces séparent des morceaux qui doivent tous
//! correspondre, dans l'ordre ; suffixe `:ligne` ou `:ligne:colonne`.
//!
//! Classement, du plus fort au plus faible : [`MatchKind`] (nom exact, nom qui
//! commence par la requête, correspondance dans le nom, correspondance qui
//! déborde sur le chemin) ; puis fichier récent (le plus récent d'abord) ; puis
//! fichier du projet avant bibliothèque ; puis qualité de la correspondance
//! (début de mot, initiales CamelCase et snake_case, morceaux consécutifs) ; puis
//! chemin le plus court ; puis ordre alphabétique.

use std::collections::HashMap;
use std::path::PathBuf;

use julie_core::{CancelToken, TextPosition};

use crate::error::Result;
use crate::files::{lower_char, FileEntry, FileIndex};

/// Nombre de fichiers récents montrés quand la requête est vide.
pub const RECENT_LIMIT: usize = 20;
/// Nombre de résultats rendus par défaut.
pub const DEFAULT_LIMIT: usize = 100;

/// Force d'une correspondance, de la plus forte à la plus faible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MatchKind {
    /// Requête vide : fichier récent.
    Recent,
    /// Le nom sans extension (ou le nom entier) est la requête.
    ExactName,
    /// Le nom commence par la requête.
    NamePrefix,
    /// Toute la requête correspond dans le nom.
    InName,
    /// La correspondance déborde sur les dossiers.
    InPath,
}

/// Un fichier trouvé.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileMatch {
    /// Chemin absolu (séparateurs du système).
    pub path: PathBuf,
    /// Chemin relatif à la racine, séparateurs `/`.
    pub relative: String,
    /// Début du nom dans `relative`, en caractères : `relative[..name_start]`
    /// est le dossier (à afficher en gris).
    pub name_start: usize,
    /// Caractères appariés, en indices de **caractères** dans `relative`,
    /// croissants. Ceux qui sont `>= name_start` tombent dans le nom.
    pub positions: Vec<usize>,
    pub kind: MatchKind,
    pub library: bool,
    /// Le fichier fait partie des récents fournis par l'appelant.
    pub recent: bool,
}

/// Résultat d'« aller au fichier ».
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileResults {
    /// La requête telle que tapée.
    pub query: String,
    /// Position demandée par un suffixe `:ligne[:colonne]`, **base zéro**
    /// (`index.php:12` donne la ligne 11, colonne 0).
    pub position: Option<TextPosition>,
    /// Les meilleurs résultats, dans l'ordre (au plus la limite demandée).
    pub matches: Vec<FileMatch>,
    /// Nombre total de fichiers qui correspondent (« et n autres » =
    /// `total - matches.len()`).
    pub total: usize,
    /// `false` tant que l'indexation initiale n'est pas finie : résultats partiels.
    pub index_complete: bool,
}

/// Sépare un suffixe `:ligne` ou `:ligne:colonne` (comptés à partir de 1 par la
/// personne) et rend la position en base zéro.
pub fn split_location(text: &str) -> (&str, Option<TextPosition>) {
    fn number(s: &str) -> Option<u32> {
        if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        s.parse().ok()
    }
    let trimmed = text.trim_end();
    let Some((head, last)) = trimmed.rsplit_once(':') else {
        return (text, None);
    };
    let Some(last) = number(last) else {
        return (text, None);
    };
    if let Some((head2, line)) = head.rsplit_once(':') {
        if let Some(line) = number(line) {
            return (
                head2,
                Some(TextPosition::new(
                    line.saturating_sub(1),
                    last.saturating_sub(1),
                )),
            );
        }
    }
    (head, Some(TextPosition::new(last.saturating_sub(1), 0)))
}

/// Requête préparée : minuscules, sans espaces, `\` changé en `/`.
struct Prepared {
    chars: Vec<char>,
    text: String,
    has_slash: bool,
    ascii: bool,
}

impl Prepared {
    fn new(pattern: &str) -> Self {
        let chars: Vec<char> = pattern
            .chars()
            .filter(|c| !c.is_whitespace())
            .map(|c| if c == '\\' { '/' } else { lower_char(c) })
            .collect();
        let start = chars.iter().position(|&c| c != '/').unwrap_or(chars.len());
        let chars = chars[start..].to_vec();
        let text: String = chars.iter().collect();
        Self {
            has_slash: chars.contains(&'/'),
            ascii: text.is_ascii(),
            text,
            chars,
        }
    }
}

impl FileIndex {
    /// Cherche les fichiers pour une requête d'« aller au fichier ».
    ///
    /// `recent` : fichiers récemment ouverts, le plus récent d'abord (chemins
    /// absolus ou relatifs à la racine). Requête vide : rend ces fichiers (ceux
    /// qui sont dans l'index, `RECENT_LIMIT` au plus). `limit` : nombre de
    /// résultats rendus (`DEFAULT_LIMIT` pour la fenêtre). Calcul en mémoire,
    /// parallèle sur les gros index ; annulable.
    pub fn find_files(
        &self,
        query: &str,
        recent: &[PathBuf],
        limit: usize,
        cancel: &CancelToken,
    ) -> Result<FileResults> {
        let (pattern, position) = split_location(query);
        let prepared = Prepared::new(pattern);
        let recent_rank: HashMap<String, u32> = recent
            .iter()
            .enumerate()
            .filter_map(|(rank, path)| Some((self.relative(path)?, rank as u32)))
            .rev() // le premier rang l'emporte en cas de doublon
            .collect();

        let mut results = FileResults {
            query: query.to_string(),
            position,
            matches: Vec::new(),
            total: 0,
            index_complete: true,
        };

        if prepared.chars.is_empty() {
            let mut seen = std::collections::HashSet::new();
            for path in recent {
                let Some(relative) = self.relative(path) else {
                    continue;
                };
                let Some(entry) = self.get(&relative) else {
                    continue;
                };
                if !seen.insert(relative) {
                    continue;
                }
                results.total += 1;
                if results.matches.len() < limit.min(RECENT_LIMIT) {
                    results
                        .matches
                        .push(self.to_match(entry, MatchKind::Recent, Vec::new(), true));
                }
            }
            return Ok(results);
        }

        let entries = self.entries();
        let threads = if entries.len() >= 8_000 {
            self.settings().thread_count().min(8)
        } else {
            1
        };
        let chunk = entries.len().div_ceil(threads).max(1);
        let mut candidates: Vec<Candidate> = if threads == 1 {
            collect_candidates(entries, 0, &prepared, &recent_rank, cancel)?
        } else {
            let parts: Vec<Result<Vec<Candidate>>> = std::thread::scope(|scope| {
                let handles: Vec<_> = entries
                    .chunks(chunk)
                    .enumerate()
                    .map(|(i, part)| {
                        let prepared = &prepared;
                        let recent_rank = &recent_rank;
                        scope.spawn(move || {
                            collect_candidates(part, i * chunk, prepared, recent_rank, cancel)
                        })
                    })
                    .collect();
                handles
                    .into_iter()
                    .map(|h| h.join().unwrap_or_else(|_| Ok(Vec::new())))
                    .collect()
            });
            let mut all = Vec::new();
            for part in parts {
                all.extend(part?);
            }
            all
        };
        cancel.check()?;

        results.total = candidates.len();
        let compare = |a: &Candidate, b: &Candidate| {
            a.key().cmp(&b.key()).then_with(|| {
                entries[a.index as usize]
                    .relative()
                    .cmp(entries[b.index as usize].relative())
            })
        };
        if candidates.len() > limit && limit > 0 {
            candidates.select_nth_unstable_by(limit - 1, compare);
            candidates.truncate(limit);
        }
        candidates.truncate(limit);
        candidates.sort_unstable_by(compare);

        let mut dp = Dp::default();
        for candidate in candidates {
            let entry = &entries[candidate.index as usize];
            let positions = positions_for(entry, candidate.kind, &prepared, &mut dp);
            results.matches.push(self.to_match(
                entry,
                candidate.kind,
                positions,
                candidate.recent != u32::MAX,
            ));
        }
        Ok(results)
    }

    fn to_match(
        &self,
        entry: &FileEntry,
        kind: MatchKind,
        positions: Vec<usize>,
        recent: bool,
    ) -> FileMatch {
        FileMatch {
            path: self.absolute(entry.relative()),
            relative: entry.relative().to_string(),
            name_start: entry.name_start(),
            positions,
            kind,
            library: entry.is_library(),
            recent,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Candidate {
    index: u32,
    kind: MatchKind,
    recent: u32,
    library: bool,
    score: i32,
    length: u32,
}

impl Candidate {
    fn key(&self) -> (MatchKind, u32, bool, i32, u32) {
        (
            self.kind,
            self.recent,
            self.library,
            -self.score,
            self.length,
        )
    }
}

fn collect_candidates(
    entries: &[FileEntry],
    offset: usize,
    prepared: &Prepared,
    recent_rank: &HashMap<String, u32>,
    cancel: &CancelToken,
) -> Result<Vec<Candidate>> {
    let mut dp = Dp::default();
    let mut out = Vec::new();
    for (i, entry) in entries.iter().enumerate() {
        if i % 4096 == 0 {
            cancel.check()?;
        }
        let Some((kind, score)) = classify(entry, prepared, &mut dp) else {
            continue;
        };
        let recent = if recent_rank.is_empty() {
            u32::MAX
        } else {
            recent_rank
                .get(entry.relative())
                .copied()
                .unwrap_or(u32::MAX)
        };
        out.push(Candidate {
            index: (offset + i) as u32,
            kind,
            recent,
            library: entry.is_library(),
            score,
            length: entry.relative().len() as u32,
        });
    }
    Ok(out)
}

/// Les caractères de `query` apparaissent-ils dans l'ordre dans `target` ?
fn is_subsequence(target: &str, query: &Prepared) -> bool {
    if query.ascii {
        let mut wanted = query.text.as_bytes().iter();
        let mut next = wanted.next();
        for &b in target.as_bytes() {
            match next {
                None => return true,
                Some(&w) if w == b => next = wanted.next(),
                Some(_) => {}
            }
        }
        next.is_none()
    } else {
        let mut wanted = query.chars.iter();
        let mut next = wanted.next();
        for c in target.chars() {
            match next {
                None => return true,
                Some(&w) if w == c => next = wanted.next(),
                Some(_) => {}
            }
        }
        next.is_none()
    }
}

fn split_name(entry: &FileEntry) -> (&str, &str) {
    let name = &entry.lower[entry.lower_name_byte as usize..];
    let stem = match name.rfind('.') {
        Some(dot) if dot > 0 => &name[..dot],
        _ => name,
    };
    (name, stem)
}

fn classify(entry: &FileEntry, query: &Prepared, dp: &mut Dp) -> Option<(MatchKind, i32)> {
    if !is_subsequence(&entry.lower, query) {
        return None;
    }
    if !query.has_slash {
        let (name, stem) = split_name(entry);
        if stem == query.text || name == query.text {
            return Some((MatchKind::ExactName, 0));
        }
        if name.starts_with(&query.text) {
            return Some((MatchKind::NamePrefix, 0));
        }
        if is_subsequence(name, query) {
            dp.load(entry, true);
            let score = dp.run(&query.chars, false).map_or(0, |(s, _)| s);
            return Some((MatchKind::InName, score));
        }
    }
    dp.load(entry, false);
    let score = dp.run(&query.chars, false).map_or(0, |(s, _)| s);
    Some((MatchKind::InPath, score))
}

fn positions_for(entry: &FileEntry, kind: MatchKind, query: &Prepared, dp: &mut Dp) -> Vec<usize> {
    let name_start = entry.name_start();
    match kind {
        MatchKind::Recent => Vec::new(),
        MatchKind::ExactName | MatchKind::NamePrefix => {
            (name_start..name_start + query.chars.len()).collect()
        }
        MatchKind::InName => {
            dp.load(entry, true);
            dp.run(&query.chars, true)
                .map(|(_, p)| p.into_iter().map(|i| i + name_start).collect())
                .unwrap_or_default()
        }
        MatchKind::InPath => {
            dp.load(entry, false);
            dp.run(&query.chars, true)
                .map(|(_, p)| p)
                .unwrap_or_default()
        }
    }
}

// Barème de la correspondance (inspiré des scores classiques des chercheurs flous).
const SCORE_MATCH: i32 = 16;
const GAP_START: i32 = 3;
const GAP_EXTENSION: i32 = 1;
const BONUS_SEPARATOR: i32 = 10;
const BONUS_BOUNDARY: i32 = 8;
const BONUS_CAMEL: i32 = 7;
const BONUS_DIGIT: i32 = 4;
const BONUS_CONSECUTIVE: i32 = 6;
const FIRST_CHAR_MULTIPLIER: i32 = 2;
const NEG: i32 = i32::MIN / 4;
const MAX_QUERY: usize = 64;
const MAX_TARGET: usize = 1024;

/// Programmation dynamique de la meilleure correspondance (tampons réutilisés).
#[derive(Default)]
struct Dp {
    original: Vec<char>,
    lower: Vec<char>,
    bonus: Vec<i32>,
    previous: Vec<i32>,
    current: Vec<i32>,
    back: Vec<u16>,
}

impl Dp {
    fn load(&mut self, entry: &FileEntry, name_only: bool) {
        let (original, lower) = if name_only {
            (entry.name(), &entry.lower[entry.lower_name_byte as usize..])
        } else {
            (entry.relative(), &*entry.lower)
        };
        self.original.clear();
        self.original.extend(original.chars());
        self.lower.clear();
        self.lower.extend(lower.chars());
        self.bonus.clear();
        let mut previous: Option<char> = None;
        for &c in &self.original {
            let bonus = match previous {
                None => BONUS_SEPARATOR,
                Some('/') | Some('\\') => BONUS_SEPARATOR,
                Some(p)
                    if matches!(p, '_' | '-' | '.' | ' ')
                        && !matches!(c, '_' | '-' | '.' | ' ') =>
                {
                    BONUS_BOUNDARY
                }
                Some(p) if (p.is_lowercase() || p.is_ascii_digit()) && c.is_uppercase() => {
                    BONUS_CAMEL
                }
                Some(p) if !p.is_ascii_digit() && c.is_ascii_digit() => BONUS_DIGIT,
                _ => 0,
            };
            self.bonus.push(bonus);
            previous = Some(c);
        }
    }

    /// Meilleur score et, sur demande, positions (indices de caractères).
    fn run(&mut self, query: &[char], with_positions: bool) -> Option<(i32, Vec<usize>)> {
        let n = self.lower.len();
        let m = query.len();
        if m == 0 || m > n {
            return None;
        }
        if m > MAX_QUERY || n > MAX_TARGET {
            return self.greedy(query);
        }
        self.previous.clear();
        self.previous.resize(n, NEG);
        self.current.clear();
        self.current.resize(n, NEG);
        if with_positions {
            self.back.clear();
            self.back.resize(m * n, 0);
        }
        for (i, &qc) in query.iter().enumerate() {
            let mut carry = NEG;
            let mut carry_at = 0usize;
            for j in 0..n {
                if i > 0 && j >= 2 {
                    carry -= GAP_EXTENSION;
                    let candidate = self.previous[j - 2] - GAP_START;
                    if candidate > carry {
                        carry = candidate;
                        carry_at = j - 2;
                    }
                }
                if self.lower[j] != qc || j < i {
                    self.current[j] = NEG;
                    continue;
                }
                let bonus = self.bonus[j];
                if i == 0 {
                    self.current[j] = SCORE_MATCH + bonus * FIRST_CHAR_MULTIPLIER;
                    continue;
                }
                let consecutive = if self.previous[j - 1] > NEG / 2 {
                    self.previous[j - 1] + BONUS_CONSECUTIVE
                } else {
                    NEG
                };
                let (best, from) = if consecutive >= carry {
                    (consecutive, j - 1)
                } else {
                    (carry, carry_at)
                };
                if best <= NEG / 2 {
                    self.current[j] = NEG;
                    continue;
                }
                self.current[j] = best + SCORE_MATCH + bonus;
                if with_positions {
                    self.back[i * n + j] = from as u16;
                }
            }
            std::mem::swap(&mut self.previous, &mut self.current);
        }
        let (end, &score) = self
            .previous
            .iter()
            .enumerate()
            .filter(|(_, &s)| s > NEG / 2)
            .max_by_key(|(j, &s)| (s, std::cmp::Reverse(*j)))?;
        if !with_positions {
            return Some((score, Vec::new()));
        }
        let mut positions = vec![0; m];
        let mut j = end;
        for i in (0..m).rev() {
            positions[i] = j;
            if i > 0 {
                j = self.back[i * n + j] as usize;
            }
        }
        Some((score, positions))
    }

    /// Repli pour les très longues entrées : première correspondance, score nul.
    fn greedy(&self, query: &[char]) -> Option<(i32, Vec<usize>)> {
        let mut positions = Vec::with_capacity(query.len());
        let mut j = 0;
        for &qc in query {
            while j < self.lower.len() && self.lower[j] != qc {
                j += 1;
            }
            if j == self.lower.len() {
                return None;
            }
            positions.push(j);
            j += 1;
        }
        Some((0, positions))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::IndexSettings;

    fn index(paths: &[&str]) -> FileIndex {
        let mut index = FileIndex::new(PathBuf::from("C:\\projet"), IndexSettings::default());
        index.insert_walked(
            paths
                .iter()
                .map(|p| crate::files::Walked::File(p.to_string()))
                .collect(),
        );
        index
    }

    fn find(index: &FileIndex, query: &str) -> Vec<String> {
        index
            .find_files(query, &[], 100, &CancelToken::new())
            .unwrap()
            .matches
            .into_iter()
            .map(|m| m.relative)
            .collect()
    }

    #[test]
    fn location_suffix_is_split_and_made_zero_based() {
        assert_eq!(
            split_location("index.php:12"),
            ("index.php", Some(TextPosition::new(11, 0)))
        );
        assert_eq!(
            split_location("index.php:12:7"),
            ("index.php", Some(TextPosition::new(11, 6)))
        );
        assert_eq!(split_location("index.php"), ("index.php", None));
        assert_eq!(split_location("a:b"), ("a:b", None));
        assert_eq!(split_location("index.php:"), ("index.php:", None));
    }

    #[test]
    fn initials_find_user_controller_first() {
        let index = index(&[
            "src/Controller/UserController.php",
            "src/Model/User.php",
            "lib/useless/cat.php",
            "tests/usecase/ctx.php",
            "src/Controller/UsersController.php",
            "docs/cats.txt",
        ]);
        let found = find(&index, "usct");
        assert_eq!(found[0], "src/Controller/UserController.php", "{found:?}");
        assert!(!found.contains(&"docs/cats.txt".to_string()));
    }

    #[test]
    fn ranking_tiers() {
        let index = index(&[
            "a/very/long/path/to/user/Thing.php",
            "src/UserService.php",
            "src/User.php",
            "src/AbstractUser.php",
            "src/Model/User.js",
            "user/x/Foo.php",
        ]);
        let results = index
            .find_files("user", &[], 100, &CancelToken::new())
            .unwrap();
        let kinds: Vec<_> = results
            .matches
            .iter()
            .map(|m| (m.relative.as_str(), m.kind))
            .collect();
        assert_eq!(kinds[0], ("src/User.php", MatchKind::ExactName));
        assert_eq!(kinds[1], ("src/Model/User.js", MatchKind::ExactName));
        assert_eq!(kinds[2], ("src/UserService.php", MatchKind::NamePrefix));
        assert_eq!(kinds[3], ("src/AbstractUser.php", MatchKind::InName));
        // Débordement sur le chemin : le plus court d'abord à score égal.
        assert_eq!(kinds[4].1, MatchKind::InPath);
        assert_eq!(kinds[5].1, MatchKind::InPath);
        assert_eq!(results.total, 6);
    }

    #[test]
    fn recent_files_win_ties_and_fill_an_empty_query() {
        let index = index(&["a/User.php", "b/User.php", "c/Other.php"]);
        let recent = vec![
            PathBuf::from("C:\\projet\\b\\User.php"),
            PathBuf::from("C:\\projet\\c\\Other.php"),
            PathBuf::from("C:\\ailleurs\\x.php"),
            PathBuf::from("C:\\projet\\deleted.php"),
        ];
        let results = index
            .find_files("user", &recent, 100, &CancelToken::new())
            .unwrap();
        assert_eq!(results.matches[0].relative, "b/User.php");
        assert!(results.matches[0].recent);

        let empty = index
            .find_files("", &recent, 100, &CancelToken::new())
            .unwrap();
        let names: Vec<_> = empty.matches.iter().map(|m| m.relative.as_str()).collect();
        assert_eq!(names, vec!["b/User.php", "c/Other.php"]);
        assert!(empty.matches.iter().all(|m| m.kind == MatchKind::Recent));
    }

    #[test]
    fn slash_matches_folder_segments_and_spaces_split_pieces() {
        let index = index(&[
            "src/Controller/UserController.php",
            "src/Model/User.php",
            "ctrl.txt",
        ]);
        assert_eq!(
            find(&index, "ctrl/user"),
            vec!["src/Controller/UserController.php"]
        );
        assert_eq!(
            find(&index, "ctrl\\user"),
            vec!["src/Controller/UserController.php"]
        );
        assert_eq!(find(&index, "model user"), vec!["src/Model/User.php"]);
        assert!(
            find(&index, "user model").is_empty(),
            "les morceaux sont dans l'ordre"
        );
    }

    #[test]
    fn positions_highlight_the_matched_characters() {
        let index = index(&["src/Controller/UserController.php", "docs/accentué/Été.md"]);
        let results = index
            .find_files("uc", &[], 100, &CancelToken::new())
            .unwrap();
        let m = &results.matches[0];
        assert_eq!(m.relative, "src/Controller/UserController.php");
        assert_eq!(m.kind, MatchKind::InName);
        let picked: String = m
            .positions
            .iter()
            .map(|&i| m.relative.chars().nth(i).unwrap())
            .collect();
        assert_eq!(picked, "UC", "initiales en CamelCase : {:?}", m.positions);

        let results = index
            .find_files("acc/été", &[], 100, &CancelToken::new())
            .unwrap();
        let m = &results.matches[0];
        let picked: String = m
            .positions
            .iter()
            .map(|&i| m.relative.chars().nth(i).unwrap())
            .collect();
        assert_eq!(picked, "acc/Été");

        let results = index
            .find_files("contr", &[], 100, &CancelToken::new())
            .unwrap();
        let m = &results.matches[0];
        assert_eq!(
            m.positions,
            (m.name_start + 4..m.name_start + 9).collect::<Vec<_>>()
        );
    }

    #[test]
    fn snake_case_initials_and_accents() {
        let index = index(&["lib/user_group_manager.php", "lib/ugly.php", "docs/Été.md"]);
        assert_eq!(find(&index, "ugm")[0], "lib/user_group_manager.php");
        assert_eq!(find(&index, "été"), vec!["docs/Été.md"]);
        assert_eq!(find(&index, "ÉTÉ"), vec!["docs/Été.md"]);
    }

    #[test]
    fn libraries_come_after_project_files() {
        let index = index(&["vendor/x/Request.php", "src/Http/Request.php"]);
        // `insert_walked` marque les bibliothèques d'après les réglages.
        assert_eq!(find(&index, "request")[0], "src/Http/Request.php");
    }

    #[test]
    fn limit_and_total() {
        let paths: Vec<String> = (0..500).map(|i| format!("d/file{i}.txt")).collect();
        let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
        let index = index(&refs);
        let results = index
            .find_files("file", &[], 100, &CancelToken::new())
            .unwrap();
        assert_eq!(results.total, 500);
        assert_eq!(results.matches.len(), 100);
        assert_eq!(results.matches[0].relative, "d/file0.txt");
    }

    #[test]
    fn location_query_still_finds_the_file() {
        let index = index(&["index.php", "src/index.php"]);
        let results = index
            .find_files("index.php:12", &[], 100, &CancelToken::new())
            .unwrap();
        assert_eq!(results.position, Some(TextPosition::new(11, 0)));
        assert_eq!(results.matches[0].relative, "index.php");
    }
}
