//! Ce que l'extérieur demande d'afficher par-dessus le texte : fonds de lignes et
//! de plages (diff, fusion, recherche), marques de gouttière (Git, tests),
//! séparateurs. L'éditeur ne sait pas d'où elles viennent.

use julie_core::TextRange;

/// Sens d'une décoration ; la couleur vient du thème ([`super::EditorTheme`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tone {
    Added,
    Removed,
    Modified,
    Conflict,
    SearchMatch,
    CurrentSearchMatch,
    Occurrence,
    Warning,
    Error,
    Info,
}

impl Tone {
    pub const ALL: [Tone; 10] = [
        Tone::Added,
        Tone::Removed,
        Tone::Modified,
        Tone::Conflict,
        Tone::SearchMatch,
        Tone::CurrentSearchMatch,
        Tone::Occurrence,
        Tone::Warning,
        Tone::Error,
        Tone::Info,
    ];

    pub(crate) fn index(self) -> usize {
        Tone::ALL
            .iter()
            .position(|t| *t == self)
            .unwrap_or_default()
    }
}

/// Fond sur toute la largeur des lignes `lines` (base zéro, fin exclue).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineDecoration {
    pub lines: std::ops::Range<u32>,
    pub tone: Tone,
}

/// Fond sous une plage de texte (positions en caractères).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangeDecoration {
    pub range: TextRange,
    pub tone: Tone,
}

/// Forme d'une marque de gouttière (dessinée, pas d'icône à télécharger).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MarkShape {
    /// Barre verticale sur la hauteur de la ligne (changement Git).
    Bar,
    /// Petit triangle couché entre deux lignes (lignes supprimées).
    Wedge,
    /// Triangle « lancer » (test).
    Run,
    /// Pastille.
    Dot,
}

/// Une marque dans la gouttière. Un clic dessus rend
/// [`super::EditorIntent::GutterMarkClicked`] avec son `id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GutterMark {
    pub line: u32,
    pub shape: MarkShape,
    pub tone: Tone,
    /// Identifiant choisi par l'appelant.
    pub id: u64,
    /// Infobulle, déjà traduite par l'appelant (catalogue de textes).
    pub tooltip: Option<String>,
}

/// Toutes les décorations d'un éditeur pour une image. Chaque liste est
/// **triée par ligne de début** : l'éditeur n'en lit que la partie visible.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Decorations {
    pub lines: Vec<LineDecoration>,
    pub ranges: Vec<RangeDecoration>,
    pub gutter: Vec<GutterMark>,
    /// Trait horizontal au-dessus de la ligne (bloc supprimé dans un diff ; une
    /// ligne égale au nombre de lignes trace le trait sous la dernière).
    pub separators: Vec<(u32, Tone)>,
    /// Mot navigable sous le pointeur (Ctrl enfoncé) : souligné, et le pointeur
    /// devient une main au-dessus. L'éditeur ne sait pas d'où il vient.
    pub link: Option<TextRange>,
}

impl Decorations {
    pub fn is_empty(&self) -> bool {
        self.link.is_none()
            && self.lines.is_empty()
            && self.ranges.is_empty()
            && self.gutter.is_empty()
            && self.separators.is_empty()
    }

    /// Décorations de ligne qui touchent `visible`.
    pub(crate) fn lines_in(
        &self,
        visible: std::ops::Range<u32>,
    ) -> impl Iterator<Item = &LineDecoration> {
        let end = self.lines.partition_point(|d| d.lines.start < visible.end);
        self.lines[..end]
            .iter()
            .filter(move |d| d.lines.end > visible.start)
    }

    pub(crate) fn ranges_in(
        &self,
        visible: std::ops::Range<u32>,
    ) -> impl Iterator<Item = &RangeDecoration> {
        let end = self
            .ranges
            .partition_point(|d| d.range.start.line < visible.end);
        self.ranges[..end]
            .iter()
            .filter(move |d| d.range.end.line >= visible.start)
    }

    pub(crate) fn gutter_in(
        &self,
        visible: std::ops::Range<u32>,
    ) -> impl Iterator<Item = &GutterMark> {
        let start = self.gutter.partition_point(|m| m.line < visible.start);
        let end = self.gutter.partition_point(|m| m.line < visible.end);
        self.gutter[start..end.max(start)].iter()
    }
}
