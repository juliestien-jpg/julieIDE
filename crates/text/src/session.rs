//! Coloration en fil de fond : un fil par document, qui garde ses arbres et
//! analyse de façon incrémentale.
//!
//! Le fil de l'interface n'attend jamais : à chaque modification il envoie un
//! instantané de la corde (copie en temps constant) et les modifications en
//! octets, et continue d'afficher les anciennes plages, recalées sur les lignes
//! insérées ou supprimées. Quand le fil de fond a fini, il appelle le réveil
//! (`request_repaint` côté interface) et [`SyntaxSession::poll`] prend le
//! résultat.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ropey::Rope;

use crate::buffer::ByteEdit;
use crate::syntax::{HighlightSpan, Highlighter, Language, LineHighlights, SyntaxSnapshot};

/// Rappel appelé (depuis le fil de fond) quand un résultat est prêt.
pub type Waker = Arc<dyn Fn() + Send + Sync>;

struct Request {
    version: u64,
    rope: Rope,
    edits: Vec<ByteEdit>,
}

struct Output {
    version: u64,
    lines: LineHighlights,
    snapshot: Option<SyntaxSnapshot>,
    elapsed: Duration,
}

/// La coloration d'un document.
pub struct SyntaxSession {
    language: Language,
    requests: Sender<Request>,
    results: Receiver<Output>,
    lines: LineHighlights,
    /// Version des plages affichées (avant recalage).
    shown_version: Option<u64>,
    /// Dernière version envoyée au fil de fond.
    requested_version: u64,
    /// Modifications postérieures à `shown_version`, pour recaler un résultat
    /// arrivé en retard.
    pending: Vec<(u64, ByteEdit)>,
    snapshot: Option<SyntaxSnapshot>,
    last_elapsed: Option<Duration>,
}

impl SyntaxSession {
    /// Lance le fil de fond et demande une première analyse de `rope`.
    pub fn new(language: Language, rope: &Rope, version: u64, waker: Option<Waker>) -> Self {
        let (requests, request_rx) = mpsc::channel::<Request>();
        let (result_tx, results) = mpsc::channel::<Output>();
        let spawned = std::thread::Builder::new()
            .name(format!("julie-syntax-{}", language.key()))
            .spawn(move || worker(language, request_rx, result_tx, waker));
        debug_assert!(spawned.is_ok(), "fil de coloration non lancé");
        let session = Self {
            language,
            requests,
            results,
            lines: vec![Vec::new(); rope.len_lines()],
            shown_version: None,
            requested_version: version,
            pending: Vec::new(),
            snapshot: None,
            last_elapsed: None,
        };
        let _ = session.requests.send(Request {
            version,
            rope: rope.clone(),
            edits: Vec::new(),
        });
        session
    }

    pub fn language(&self) -> Language {
        self.language
    }

    /// Signale des modifications : recale les plages affichées tout de suite et
    /// demande une nouvelle analyse de `rope` (version `version`).
    pub fn update(&mut self, rope: &Rope, version: u64, edits: Vec<ByteEdit>) {
        if edits.is_empty() && version == self.requested_version {
            return;
        }
        for edit in &edits {
            splice(&mut self.lines, edit);
            self.pending.push((version, *edit));
        }
        self.requested_version = version;
        let _ = self.requests.send(Request {
            version,
            rope: rope.clone(),
            edits,
        });
    }

    /// Prend le résultat le plus récent s'il y en a un. Rend `true` si les
    /// plages ont changé. À appeler une fois par image.
    pub fn poll(&mut self) -> bool {
        let mut latest = None;
        while let Ok(output) = self.results.try_recv() {
            latest = Some(output);
        }
        match latest {
            Some(output) => {
                self.accept(output);
                true
            }
            None => false,
        }
    }

    /// Attend (bloquant) que la dernière version demandée soit colorée. Pour les
    /// tests et les mesures ; jamais dans le fil de l'interface.
    pub fn wait(&mut self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while self.shown_version != Some(self.requested_version) {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.results.recv_timeout(left) {
                Ok(output) => self.accept(output),
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => return false,
            }
        }
        true
    }

    /// La coloration affichée est-elle celle de la dernière version ?
    pub fn is_current(&self) -> bool {
        self.shown_version == Some(self.requested_version)
    }

    /// Plages de la ligne `line` (vide hors du texte).
    pub fn line(&self, line: usize) -> &[HighlightSpan] {
        self.lines.get(line).map_or(&[], Vec::as_slice)
    }

    /// Les arbres de la dernière analyse terminée.
    pub fn snapshot(&self) -> Option<&SyntaxSnapshot> {
        self.snapshot.as_ref()
    }

    /// Durée de la dernière analyse (mesure).
    pub fn last_elapsed(&self) -> Option<Duration> {
        self.last_elapsed
    }

    fn accept(&mut self, output: Output) {
        let mut lines = output.lines;
        self.pending
            .retain(|(version, _)| *version > output.version);
        for (_, edit) in &self.pending {
            splice(&mut lines, edit);
        }
        self.lines = lines;
        self.shown_version = Some(output.version);
        self.snapshot = output.snapshot;
        self.last_elapsed = Some(output.elapsed);
    }
}

fn worker(
    language: Language,
    requests: Receiver<Request>,
    results: Sender<Output>,
    waker: Option<Waker>,
) {
    let mut highlighter = Highlighter::new(language);
    while let Ok(first) = requests.recv() {
        // Les demandes arrivées entre-temps sont fusionnées : seule compte la
        // dernière corde, mais toutes les modifications vont aux arbres.
        let mut request = first;
        let mut edits = std::mem::take(&mut request.edits);
        while let Ok(next) = requests.try_recv() {
            edits.extend(next.edits);
            request.version = next.version;
            request.rope = next.rope;
        }
        let started = Instant::now();
        highlighter.edit(&edits);
        let (lines, snapshot) = highlighter.update(&request.rope, request.version);
        let output = Output {
            version: request.version,
            lines,
            snapshot,
            elapsed: started.elapsed(),
        };
        if results.send(output).is_err() {
            return;
        }
        if let Some(waker) = &waker {
            waker();
        }
    }
}

/// Recale des plages par ligne sur une modification, en attendant la nouvelle
/// analyse : les lignes qui suivent gardent leurs couleurs.
pub(crate) fn splice(lines: &mut LineHighlights, edit: &ByteEdit) {
    let row = edit.start_point.row;
    if row >= lines.len() {
        return;
    }
    let removed = edit.old_end_point.row - row;
    let added = edit.new_end_point.row.saturating_sub(row);
    let start = edit.start_point.column as u32;
    if removed == 0 && added == 0 {
        let old_end = edit.old_end_point.column as i64;
        let delta = edit.new_end_point.column as i64 - old_end;
        for span in &mut lines[row] {
            let shift = |x: u32| -> u32 {
                if (x as i64) >= old_end {
                    (x as i64 + delta).max(start as i64) as u32
                } else {
                    x.min(start)
                }
            };
            span.start = shift(span.start);
            span.end = shift(span.end);
        }
        lines[row].retain(|span| span.start < span.end);
        return;
    }
    lines[row].retain(|span| span.start < start);
    for span in &mut lines[row] {
        span.end = span.end.min(start);
    }
    let first_removed = (row + 1).min(lines.len());
    let last_removed = (row + 1 + removed).min(lines.len());
    lines.splice(
        first_removed..last_removed,
        std::iter::repeat_with(Vec::new).take(added),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Buffer, EditKind, Selections};

    #[test]
    fn the_session_follows_edits_and_ends_up_like_a_fresh_parse() {
        let mut buffer = Buffer::new("<?php\n$a = 1;\n");
        let mut session = SyntaxSession::new(Language::Php, buffer.rope(), buffer.version(), None);
        assert!(session.wait(Duration::from_secs(10)));
        let selections = Selections::default();
        for i in 0..20 {
            let at = buffer.len_chars();
            buffer.apply(
                &[(at..at, format!("$v{i} = 'x';\n"))],
                EditKind::Other,
                &selections,
                selections.clone(),
                Instant::now(),
            );
            let edits = buffer.take_byte_edits();
            session.update(buffer.rope(), buffer.version(), edits);
            // Recalage immédiat : autant de lignes que le texte.
            assert_eq!(session.lines.len(), buffer.len_lines());
        }
        assert!(session.wait(Duration::from_secs(10)));
        assert!(session.is_current());
        let expected = crate::syntax::highlight_text(Language::Php, &buffer.text());
        assert_eq!(session.lines, expected);
        assert!(session.snapshot().is_some());
    }

    #[test]
    fn splice_shifts_spans_on_the_edited_line() {
        use crate::buffer::BytePoint;
        use crate::HighlightKind as K;
        let span = |start, end| HighlightSpan {
            start,
            end,
            kind: K::Keyword,
        };
        let mut lines = vec![vec![span(0, 2), span(5, 8)], vec![span(0, 1)]];
        // Deux octets insérés en colonne 3 de la ligne 0.
        splice(
            &mut lines,
            &ByteEdit {
                start_byte: 3,
                old_end_byte: 3,
                new_end_byte: 5,
                start_point: BytePoint { row: 0, column: 3 },
                old_end_point: BytePoint { row: 0, column: 3 },
                new_end_point: BytePoint { row: 0, column: 5 },
            },
        );
        assert_eq!(lines[0], vec![span(0, 2), span(7, 10)]);
        // Un saut de ligne inséré en colonne 1 : une ligne de plus.
        splice(
            &mut lines,
            &ByteEdit {
                start_byte: 1,
                old_end_byte: 1,
                new_end_byte: 2,
                start_point: BytePoint { row: 0, column: 1 },
                old_end_point: BytePoint { row: 0, column: 1 },
                new_end_point: BytePoint { row: 1, column: 0 },
            },
        );
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], vec![span(0, 1)]);
        assert_eq!(lines[2], vec![span(0, 1)]);
    }
}
