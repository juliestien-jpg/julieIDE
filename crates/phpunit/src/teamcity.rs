//! Analyse en flux de la sortie `--teamcity` de PHPUnit.
//!
//! Une ligne `##teamcity[nom clé='valeur' ...]` est un message ; tout le reste est
//! de la sortie ordinaire (avertissements PHP, `echo`, résumé de PHPUnit), rendue
//! en [`TestEvent::Output`] pour qu'elle ne se perde jamais.
//!
//! Échappements de TeamCity dans les valeurs : `|n` retour à la ligne, `|r`,
//! `|'` apostrophe, `||` barre, `|[` et `|]` crochets, `|0xXXXX` caractère Unicode.
//! Un `|` suivi d'autre chose est gardé tel quel.
//!
//! Tolérances (PHPUnit 9 à 12 ne sont pas identiques) :
//! - le message peut ne pas être en début de ligne (sortie d'un test sans saut de
//!   ligne final) : ce qui précède est de la sortie ordinaire ;
//! - PHPUnit 9 écrit une ligne vide avant chaque message : une ligne vide juste
//!   avant un message est retirée (une seule), les autres sont conservées ;
//! - un message inconnu ou mal formé devient de la sortie ordinaire ;
//! - un attribut absent a une valeur par défaut vide.

use std::time::Duration;

use julie_core::process::Stream;

use crate::detect::parse_phpunit_version;
use crate::event::{Comparison, ReportedSummary, TestEvent};
use crate::model::TestState;

const PREFIX: &str = "##teamcity[";

/// Un message TeamCity décodé.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub name: String,
    /// Dans l'ordre d'apparition, valeurs déjà désechappées.
    pub attributes: Vec<(String, String)>,
}

impl Message {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
    }
}

/// Échappe une valeur comme TeamCity (pour construire des messages en test).
pub fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '|' => out.push_str("||"),
            '\'' => out.push_str("|'"),
            '\n' => out.push_str("|n"),
            '\r' => out.push_str("|r"),
            '[' => out.push_str("|["),
            ']' => out.push_str("|]"),
            other => out.push(other),
        }
    }
    out
}

/// Désechappe une valeur TeamCity.
pub fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.char_indices();
    while let Some((at, ch)) = chars.next() {
        if ch != '|' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some((_, 'n')) => out.push('\n'),
            Some((_, 'r')) => out.push('\r'),
            Some((_, '\'')) => out.push('\''),
            Some((_, '|')) => out.push('|'),
            Some((_, '[')) => out.push('['),
            Some((_, ']')) => out.push(']'),
            Some((_, '0')) if value[at..].starts_with("|0x") => {
                let hex = value.get(at + 3..at + 7).filter(|hex| {
                    hex.len() == 4 && hex.chars().all(|digit| digit.is_ascii_hexdigit())
                });
                match hex.and_then(|hex| u32::from_str_radix(hex, 16).ok()) {
                    Some(code) => {
                        out.push(char::from_u32(code).unwrap_or('\u{fffd}'));
                        // `|0x` + 4 chiffres : on a déjà lu `|0`, reste `x` + 4 chiffres.
                        for _ in 0..5 {
                            chars.next();
                        }
                    }
                    None => out.push_str("|0"),
                }
            }
            Some((_, other)) => {
                out.push('|');
                out.push(other);
            }
            None => out.push('|'),
        }
    }
    out
}

/// Cherche le premier message d'une ligne. Rend `(début, fin, message)` : octets de
/// `##teamcity[` jusqu'après le `]` fermant.
pub fn find_message(line: &str) -> Option<(usize, usize, Message)> {
    let start = line.find(PREFIX)?;
    let (message, length) = parse_message(&line[start + PREFIX.len()..])?;
    Some((start, start + PREFIX.len() + length, message))
}

/// Analyse ce qui suit `##teamcity[` ; rend le message et le nombre d'octets
/// consommés (`]` compris). `None` si le message est mal formé ou inachevé.
fn parse_message(text: &str) -> Option<(Message, usize)> {
    let bytes = text.as_bytes();
    let mut at = 0;
    while at < bytes.len() && !bytes[at].is_ascii_whitespace() && bytes[at] != b']' {
        at += 1;
    }
    let name = text[..at].to_owned();
    if name.is_empty() {
        return None;
    }
    let mut attributes = Vec::new();
    loop {
        while at < bytes.len() && bytes[at].is_ascii_whitespace() {
            at += 1;
        }
        match *bytes.get(at)? {
            b']' => return Some((Message { name, attributes }, at + 1)),
            b'\'' => {
                // Valeur sans clé : `##teamcity[nom 'valeur']`.
                let (value, next) = read_value(text, at + 1)?;
                attributes.push((String::new(), value));
                at = next;
            }
            _ => {
                let key_start = at;
                while at < bytes.len()
                    && bytes[at] != b'='
                    && bytes[at] != b']'
                    && !bytes[at].is_ascii_whitespace()
                {
                    at += 1;
                }
                let key = text[key_start..at].to_owned();
                if bytes.get(at) == Some(&b'=') && bytes.get(at + 1) == Some(&b'\'') {
                    let (value, next) = read_value(text, at + 2)?;
                    attributes.push((key, value));
                    at = next;
                } else if bytes.get(at) == Some(&b'=') {
                    // `clé=valeur` sans apostrophes : jusqu'à l'espace ou au crochet.
                    let value_start = at + 1;
                    at = value_start;
                    while at < bytes.len() && !bytes[at].is_ascii_whitespace() && bytes[at] != b']'
                    {
                        at += 1;
                    }
                    attributes.push((key, unescape(&text[value_start..at])));
                } else if key.is_empty() {
                    return None;
                } else {
                    // Clé sans valeur : on la garde vide.
                    attributes.push((key, String::new()));
                }
            }
        }
    }
}

/// Lit une valeur entre apostrophes à partir de `from` (après l'apostrophe
/// ouvrante). Rend la valeur désechappée et l'indice après l'apostrophe fermante.
fn read_value(text: &str, from: usize) -> Option<(String, usize)> {
    let mut end = None;
    let mut chars = text[from..].char_indices();
    while let Some((at, ch)) = chars.next() {
        match ch {
            '|' => {
                chars.next();
            }
            '\'' => {
                end = Some(from + at);
                break;
            }
            _ => {}
        }
    }
    let end = end?;
    Some((unescape(&text[from..end]), end + 1))
}

/// Analyseur de flux : une ligne en entrée, zéro ou plusieurs événements en sortie.
#[derive(Debug, Default)]
pub struct TeamCityParser {
    pending_blank: usize,
    version_seen: bool,
}

impl TeamCityParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Analyse une ligne (sans fin de ligne). Les lignes d'erreur standard sont
    /// toujours de la sortie ordinaire.
    pub fn feed(&mut self, stream: Stream, line: &str) -> Vec<TestEvent> {
        let mut out = Vec::new();
        if stream == Stream::Stderr {
            out.push(TestEvent::Output {
                stream,
                text: line.to_owned(),
            });
            return out;
        }
        if line.is_empty() {
            self.pending_blank += 1;
            return out;
        }
        let mut rest = line;
        loop {
            match find_message(rest) {
                Some((start, end, message)) => {
                    let before = &rest[..start];
                    if before.is_empty() {
                        self.flush_blank(true, &mut out);
                    } else {
                        self.flush_blank(false, &mut out);
                        self.text(before, &mut out);
                    }
                    self.message(message, &rest[start..end], &mut out);
                    rest = &rest[end..];
                    if rest.is_empty() {
                        break;
                    }
                }
                None => {
                    self.flush_blank(false, &mut out);
                    self.text(rest, &mut out);
                    break;
                }
            }
        }
        out
    }

    /// À appeler à la fin du flux : rend les lignes vides encore retenues.
    pub fn finish(&mut self) -> Vec<TestEvent> {
        let mut out = Vec::new();
        self.flush_blank(false, &mut out);
        out
    }

    fn flush_blank(&mut self, drop_one: bool, out: &mut Vec<TestEvent>) {
        let mut count = std::mem::take(&mut self.pending_blank);
        if drop_one {
            count = count.saturating_sub(1);
        }
        for _ in 0..count {
            out.push(TestEvent::Output {
                stream: Stream::Stdout,
                text: String::new(),
            });
        }
    }

    fn text(&mut self, text: &str, out: &mut Vec<TestEvent>) {
        out.push(TestEvent::Output {
            stream: Stream::Stdout,
            text: text.to_owned(),
        });
        if !self.version_seen {
            if let Some(version) = parse_phpunit_version(text) {
                self.version_seen = true;
                out.push(TestEvent::Version(version));
            }
        }
        if let Some(summary) = parse_summary_line(text) {
            out.push(TestEvent::Summary(summary));
        }
    }

    fn message(&mut self, message: Message, raw: &str, out: &mut Vec<TestEvent>) {
        let get = |key: &str| message.get(key).unwrap_or_default().to_owned();
        let hint = || message.get("locationHint").map(str::to_owned);
        let duration = || message.get("duration").and_then(parse_duration);
        let event = match message.name.as_str() {
            "testCount" => TestEvent::TestCount(
                message
                    .get("count")
                    .and_then(|count| count.trim().parse().ok())
                    .unwrap_or(0),
            ),
            "testSuiteStarted" => TestEvent::SuiteStarted {
                name: get("name"),
                location_hint: hint(),
            },
            "testSuiteFinished" => TestEvent::SuiteFinished { name: get("name") },
            "testStarted" => TestEvent::TestStarted {
                name: get("name"),
                location_hint: hint(),
            },
            "testFinished" => TestEvent::TestFinished {
                name: get("name"),
                duration: duration(),
            },
            "testFailed" => {
                let text = get("message");
                let comparison = match (message.get("expected"), message.get("actual")) {
                    (Some(expected), Some(actual)) => Some(Comparison {
                        expected: expected.to_owned(),
                        actual: actual.to_owned(),
                    }),
                    _ => None,
                };
                TestEvent::TestFailed {
                    name: get("name"),
                    state: classify_failure(&text),
                    message: text,
                    details: get("details"),
                    duration: duration(),
                    comparison,
                }
            }
            "testIgnored" => {
                let text = get("message");
                TestEvent::TestIgnored {
                    name: get("name"),
                    state: if says_incomplete(&text) {
                        TestState::Incomplete
                    } else {
                        TestState::Skipped
                    },
                    message: text,
                    details: get("details"),
                    duration: duration(),
                }
            }
            "testStdOut" | "testStdErr" => {
                let stream = if message.name == "testStdErr" {
                    Stream::Stderr
                } else {
                    Stream::Stdout
                };
                for line in get("out").lines() {
                    out.push(TestEvent::Output {
                        stream,
                        text: line.to_owned(),
                    });
                }
                return;
            }
            _ => TestEvent::Output {
                stream: Stream::Stdout,
                text: raw.to_owned(),
            },
        };
        out.push(event);
    }
}

fn parse_duration(text: &str) -> Option<Duration> {
    let text = text.trim();
    if let Ok(millis) = text.parse::<u64>() {
        return Some(Duration::from_millis(millis));
    }
    text.parse::<f64>()
        .ok()
        .filter(|millis| millis.is_finite() && *millis >= 0.0)
        .map(|millis| Duration::from_secs_f64(millis / 1000.0))
}

fn says_incomplete(message: &str) -> bool {
    message
        .trim_start()
        .get(..10)
        .is_some_and(|start| start.eq_ignore_ascii_case("incomplete"))
}

/// Range un `testFailed` : échec d'assertion, erreur (exception) ou test risqué.
///
/// PHPUnit 9 émet les trois sous le même message `testFailed`. Ce qui les
/// distingue :
/// - une exception se présente comme `Classe : message` (la classe finit par
///   `Exception` ou `Error`, ou porte un espace de noms) ;
/// - un test risqué a un message qui commence par `This test ...` (« This test did
///   not perform any assertions », « This test printed output »...) ou
///   `Test code or tested code did not...`.
///
/// C'est une heuristique, vérifiée contre PHPUnit 9.6 seulement.
pub fn classify_failure(message: &str) -> TestState {
    let text = message.trim_start();
    if text.starts_with("This test ")
        || text.starts_with("Test code or tested code did not")
        || text.starts_with("Risky:")
    {
        return TestState::Risky;
    }
    if let Some((class, _)) = text.split_once(" : ") {
        let looks_like_class = !class.is_empty()
            && class
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '\\')
            && (class.ends_with("Exception") || class.ends_with("Error") || class.contains('\\'));
        if looks_like_class {
            return TestState::Error;
        }
    }
    TestState::Failed
}

/// Reconnaît les lignes de résumé de PHPUnit : `Tests: 13, Assertions: 10, ...` et
/// `OK (5 tests, 5 assertions)`.
pub fn parse_summary_line(line: &str) -> Option<ReportedSummary> {
    let line = line.trim();
    if let Some(rest) = line.strip_prefix("OK (").and_then(|r| r.strip_suffix(')')) {
        let mut summary = ReportedSummary {
            ok: true,
            ..Default::default()
        };
        let mut parts = rest.split(", ");
        summary.tests = leading_number(parts.next()?, &["tests", "test"])?;
        summary.assertions = leading_number(parts.next()?, &["assertions", "assertion"])?;
        return Some(summary);
    }
    let rest = line.strip_prefix("Tests: ")?.strip_suffix('.')?;
    let mut summary = ReportedSummary::default();
    let mut fields = rest.split(", ");
    summary.tests = fields.next()?.parse().ok()?;
    let mut saw_assertions = false;
    for field in fields {
        let (name, count) = field.split_once(": ")?;
        let count: u32 = count.parse().ok()?;
        match name {
            "Assertions" => {
                summary.assertions = count;
                saw_assertions = true;
            }
            "Errors" => summary.errors = count,
            "Failures" => summary.failures = count,
            "Warnings" => summary.warnings = count,
            "Skipped" => summary.skipped = count,
            "Incomplete" => summary.incomplete = count,
            "Risky" => summary.risky = count,
            "Deprecations" | "PHPUnit Deprecations" => summary.deprecations += count,
            "Notices" | "PHPUnit Notices" => summary.notices += count,
            _ => return None,
        }
    }
    saw_assertions.then_some(summary)
}

fn leading_number(part: &str, units: &[&str]) -> Option<u32> {
    let (number, unit) = part.split_once(' ')?;
    units.contains(&unit).then(|| number.parse().ok())?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn out(text: &str) -> TestEvent {
        TestEvent::Output {
            stream: Stream::Stdout,
            text: text.to_owned(),
        }
    }

    #[test]
    fn escapes_round_trip() {
        let value = "il y a des | barres, 'apostrophes', [crochets]\net un retour\r";
        assert_eq!(unescape(&escape(value)), value);
        assert_eq!(escape("a|b"), "a||b");
        assert_eq!(unescape("|0x00e9t|0x00E9"), "été");
        // Un échappement inconnu n'est pas perdu.
        assert_eq!(unescape("a|zb|"), "a|zb|");
        assert_eq!(unescape("|0xZZ"), "|0xZZ");
    }

    #[test]
    fn a_message_is_found_with_its_attributes() {
        let line =
            "##teamcity[testFailed name='t|'1' message='a|nb || c |[x|]' duration='12' flowId='7']";
        let (start, end, message) = find_message(line).unwrap();
        assert_eq!((start, end), (0, line.len()));
        assert_eq!(message.name, "testFailed");
        assert_eq!(message.get("name"), Some("t'1"));
        assert_eq!(message.get("message"), Some("a\nb | c [x]"));
        assert_eq!(message.get("duration"), Some("12"));
        assert_eq!(message.get("absent"), None);
    }

    #[test]
    fn malformed_messages_are_not_messages() {
        assert!(find_message("##teamcity[testStarted name='x").is_none());
        assert!(find_message("##teamcity[]").is_none());
        assert!(find_message("rien").is_none());
    }

    #[test]
    fn a_closing_bracket_inside_a_value_does_not_end_the_message() {
        let (_, _, message) = find_message("##teamcity[x a='|]' b='[]']").unwrap();
        assert_eq!(message.get("a"), Some("]"));
        assert_eq!(message.get("b"), Some("[]"));
    }

    #[test]
    fn one_blank_line_before_a_message_is_framing_the_others_are_kept() {
        let mut parser = TeamCityParser::new();
        let mut events = Vec::new();
        for line in [
            "PHPUnit 9.6.34 by Sebastian Bergmann and contributors.",
            "",
            "",
            "##teamcity[testCount count='2' flowId='1']",
            "",
            "##teamcity[testStarted name='a' flowId='1']",
            "ma sortie",
            "",
            "",
            "##teamcity[testFinished name='a' duration='3' flowId='1']",
            "fin",
            "",
        ] {
            events.extend(parser.feed(Stream::Stdout, line));
        }
        events.extend(parser.finish());
        assert_eq!(
            events,
            vec![
                out("PHPUnit 9.6.34 by Sebastian Bergmann and contributors."),
                TestEvent::Version(parse_phpunit_version("PHPUnit 9.6.34").unwrap()),
                out(""),
                TestEvent::TestCount(2),
                TestEvent::TestStarted {
                    name: "a".into(),
                    location_hint: None
                },
                out("ma sortie"),
                out(""),
                TestEvent::TestFinished {
                    name: "a".into(),
                    duration: Some(Duration::from_millis(3))
                },
                out("fin"),
                out(""),
            ]
        );
    }

    #[test]
    fn output_glued_before_a_message_is_kept() {
        let mut parser = TeamCityParser::new();
        let events = parser.feed(
            Stream::Stdout,
            "texte sans saut##teamcity[testFinished name='a' duration='1']reste",
        );
        assert_eq!(events.len(), 3);
        assert_eq!(events[0], out("texte sans saut"));
        assert!(matches!(&events[1], TestEvent::TestFinished { name, .. } if name == "a"));
        assert_eq!(events[2], out("reste"));
    }

    #[test]
    fn unknown_and_broken_messages_stay_in_the_output() {
        let mut parser = TeamCityParser::new();
        let events = parser.feed(Stream::Stdout, "##teamcity[buildProblem description='x']");
        assert_eq!(
            events,
            vec![out("##teamcity[buildProblem description='x']")]
        );
        let events = parser.feed(Stream::Stdout, "##teamcity[testStarted name='coupé");
        assert_eq!(events, vec![out("##teamcity[testStarted name='coupé")]);
    }

    #[test]
    fn stderr_is_never_interpreted() {
        let mut parser = TeamCityParser::new();
        let events = parser.feed(Stream::Stderr, "##teamcity[testCount count='3']");
        assert_eq!(
            events,
            vec![TestEvent::Output {
                stream: Stream::Stderr,
                text: "##teamcity[testCount count='3']".into()
            }]
        );
    }

    #[test]
    fn failures_are_classified() {
        assert_eq!(
            classify_failure("Failed asserting that two strings are equal."),
            TestState::Failed
        );
        assert_eq!(
            classify_failure("RuntimeException : boum"),
            TestState::Error
        );
        assert_eq!(classify_failure("App\\Boom : x"), TestState::Error);
        assert_eq!(classify_failure("TypeError : x"), TestState::Error);
        assert_eq!(
            classify_failure("This test did not perform any assertions\n\nC:\\x.php:48"),
            TestState::Risky
        );
        // Un message de test qui ressemble à `Mot : texte` reste un échec.
        assert_eq!(classify_failure("Attendu : 3"), TestState::Failed);
    }

    #[test]
    fn a_failure_carries_expected_and_actual() {
        let mut parser = TeamCityParser::new();
        let events = parser.feed(
            Stream::Stdout,
            "##teamcity[testFailed name='t' message='m' details=' a.php:2|n ' duration='5' type='comparisonFailure' actual='|'obtenu|'' expected='|'attendu|'']",
        );
        let TestEvent::TestFailed {
            comparison,
            details,
            duration,
            ..
        } = &events[0]
        else {
            panic!("{events:?}");
        };
        let comparison = comparison.as_ref().unwrap();
        assert_eq!(comparison.expected, "'attendu'");
        assert_eq!(comparison.actual_text(), "obtenu");
        assert_eq!(details, " a.php:2\n ");
        assert_eq!(*duration, Some(Duration::from_millis(5)));
    }

    #[test]
    fn incomplete_is_told_from_skipped_by_its_message() {
        let mut parser = TeamCityParser::new();
        let events = parser.feed(
            Stream::Stdout,
            "##teamcity[testIgnored name='t' message='Incomplete: à finir']",
        );
        assert!(matches!(
            &events[0],
            TestEvent::TestIgnored {
                state: TestState::Incomplete,
                ..
            }
        ));
        let events = parser.feed(
            Stream::Stdout,
            "##teamcity[testIgnored name='t' message='à finir']",
        );
        assert!(matches!(
            &events[0],
            TestEvent::TestIgnored {
                state: TestState::Skipped,
                ..
            }
        ));
    }

    #[test]
    fn fractional_durations_are_accepted() {
        assert_eq!(parse_duration("12"), Some(Duration::from_millis(12)));
        assert_eq!(parse_duration("1.5"), Some(Duration::from_micros(1500)));
        assert_eq!(parse_duration("abc"), None);
        assert_eq!(parse_duration("-3"), None);
    }

    #[test]
    fn summary_lines_are_understood() {
        let summary = parse_summary_line(
            "Tests: 13, Assertions: 10, Errors: 1, Failures: 3, Skipped: 1, Incomplete: 1, Risky: 1.",
        )
        .unwrap();
        assert_eq!(
            (
                summary.tests,
                summary.assertions,
                summary.errors,
                summary.failures
            ),
            (13, 10, 1, 3)
        );
        assert_eq!(
            (summary.skipped, summary.incomplete, summary.risky),
            (1, 1, 1)
        );
        assert!(!summary.ok);

        let ok = parse_summary_line("OK (5 tests, 7 assertions)").unwrap();
        assert!(ok.ok);
        assert_eq!((ok.tests, ok.assertions), (5, 7));
        let single = parse_summary_line("OK (1 test, 1 assertion)").unwrap();
        assert_eq!(single.tests, 1);

        assert!(parse_summary_line("Tests: ce que je veux.").is_none());
        assert!(parse_summary_line("Time: 00:01.189, Memory: 6.00 MB").is_none());
        assert!(parse_summary_line("Tests: 3, Assertions: 3, Inconnu: 1.").is_none());
    }
}
