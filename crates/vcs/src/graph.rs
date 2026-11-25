//! Graphe du journal : calcul des couloirs (« lanes ») à partir des parents.
//! Calcul pur, sans Git ni disque, incrémental : [`GraphBuilder`] garde son
//! état d'une page à l'autre, si bien que les pages successives donnent
//! **exactement** les mêmes lignes que le calcul d'un seul tenant.
//!
//! Règles :
//! - les commits arrivent dans l'ordre d'affichage, enfants avant parents
//!   (`LogOrder::Date` ou `Topo`, celui de [`crate::LogQuery`] par défaut) ;
//! - le premier parent reprend le couloir de son enfant : la ligne de premier
//!   parent reste droite ; un couloir libéré est réutilisé, aucun couloir ne
//!   se décale (pas de compaction : une ligne qui passe reste verticale) ;
//! - un parent déjà attendu dans un autre couloir y est relié (les lignes se
//!   rejoignent) au lieu d'ouvrir un couloir de plus ;
//! - la couleur d'un couloir naît avec lui et le suit : dérivée du nom de la
//!   branche locale posée sur le commit qui l'ouvre, sinon de son hash — donc
//!   stable d'une session à l'autre. L'interface la réduit à sa palette
//!   (`color % palette.len()`).
//!
//! Dessin d'une ligne `r` : chaque [`GraphEdge`] relie soit le haut de la ligne
//! au milieu ([`EdgeHalf::Top`]), soit le milieu au bas ([`EdgeHalf::Bottom`]),
//! d'un couloir `from` à un couloir `to`. Le point du commit est au milieu du
//! couloir `lane`. Le bas de la ligne `r` est le haut de la ligne `r + 1`.

use crate::history::{CommitSummary, RefKind};

/// Moitié de ligne parcourue par un segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EdgeHalf {
    /// Du haut de la ligne (couloir `from`) au milieu (couloir `to`).
    Top,
    /// Du milieu (couloir `from`) au bas de la ligne (couloir `to`).
    Bottom,
}

/// Un segment à dessiner dans une ligne.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GraphEdge {
    pub from: usize,
    pub to: usize,
    pub color: u32,
    pub half: EdgeHalf,
}

/// Une ligne du graphe (un commit).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphRow {
    pub commit: String,
    /// Couloir du point.
    pub lane: usize,
    /// Couleur du point.
    pub color: u32,
    /// Segments de la ligne : couloirs qui passent, lignes qui arrivent au
    /// point (enfants), lignes qui en partent (parents).
    pub edges: Vec<GraphEdge>,
    /// Nombre de couloirs à réserver pour cette ligne.
    pub width: usize,
}

impl GraphRow {
    /// Les segments qui partent du point vers ses parents.
    pub fn parent_edges(&self) -> impl Iterator<Item = &GraphEdge> {
        // Aucun couloir ne passe par celui du point : un segment bas qui en
        // part va forcément vers un parent.
        self.edges
            .iter()
            .filter(move |e| e.half == EdgeHalf::Bottom && e.from == self.lane)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Lane {
    /// Le commit attendu dans ce couloir.
    commit: String,
    color: u32,
}

/// Calcul incrémental des couloirs. Garder la même instance pour toutes les
/// pages d'un même journal ; en créer une neuve quand le journal recommence
/// (autre filtre, rafraîchissement).
#[derive(Debug, Clone, Default)]
pub struct GraphBuilder {
    lanes: Vec<Option<Lane>>,
}

impl GraphBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Les lignes d'une page de commits (dans l'ordre du journal).
    pub fn push_page(&mut self, commits: &[CommitSummary]) -> Vec<GraphRow> {
        commits
            .iter()
            .map(|commit| {
                let branch = commit
                    .refs
                    .iter()
                    .find(|r| r.kind == RefKind::LocalBranch)
                    .or_else(|| commit.refs.iter().find(|r| r.kind == RefKind::RemoteBranch))
                    .map(|r| r.name.as_str());
                self.push(&commit.id, &commit.parents, branch)
            })
            .collect()
    }

    /// La ligne d'un commit. `branch` : nom d'une branche posée dessus (sert à
    /// la couleur d'un couloir qui s'ouvre ici).
    pub fn push(&mut self, id: &str, parents: &[String], branch: Option<&str>) -> GraphRow {
        let mut edges = Vec::new();
        let matching: Vec<usize> = self
            .lanes
            .iter()
            .enumerate()
            .filter(|(_, lane)| lane.as_ref().is_some_and(|l| l.commit == id))
            .map(|(index, _)| index)
            .collect();
        let (lane, color) = match matching.first() {
            Some(&index) => (
                index,
                self.lanes[index]
                    .as_ref()
                    .map(|l| l.color)
                    .unwrap_or_default(),
            ),
            None => (self.free_slot(None), color_of(branch.unwrap_or(id))),
        };

        // Moitié haute : ce qui passe, et ce qui arrive au point.
        let before: Vec<Option<Lane>> = self.lanes.clone();
        for (index, slot) in before.iter().enumerate() {
            let Some(existing) = slot else { continue };
            let to = if matching.contains(&index) {
                lane
            } else {
                index
            };
            edges.push(GraphEdge {
                from: index,
                to,
                color: existing.color,
                half: EdgeHalf::Top,
            });
        }
        for &index in &matching {
            self.lanes[index] = None;
        }
        if lane == self.lanes.len() {
            self.lanes.push(None);
        }

        // Parents : le premier reprend le couloir du point, sauf s'il est déjà
        // attendu ailleurs (on s'y relie).
        let mut node_targets: Vec<(usize, u32)> = Vec::new();
        for (position, parent) in parents.iter().enumerate() {
            if let Some(index) = self.position_of(parent) {
                let parent_color = self.lanes[index].as_ref().map(|l| l.color).unwrap_or(color);
                node_targets.push((index, parent_color));
                continue;
            }
            let (index, parent_color) = if position == 0 && self.lanes[lane].is_none() {
                (lane, color)
            } else {
                (self.free_slot(Some(lane)), color_of(parent))
            };
            if index == self.lanes.len() {
                self.lanes.push(None);
            }
            self.lanes[index] = Some(Lane {
                commit: parent.clone(),
                color: parent_color,
            });
            node_targets.push((index, parent_color));
        }

        // Moitié basse : ce qui passe toujours, puis les lignes vers les parents.
        for (index, slot) in before.iter().enumerate() {
            if slot.is_some() && !matching.contains(&index) {
                if let Some(still) = &self.lanes[index] {
                    edges.push(GraphEdge {
                        from: index,
                        to: index,
                        color: still.color,
                        half: EdgeHalf::Bottom,
                    });
                }
            }
        }
        for (index, parent_color) in node_targets {
            edges.push(GraphEdge {
                from: lane,
                to: index,
                color: parent_color,
                half: EdgeHalf::Bottom,
            });
        }

        while matches!(self.lanes.last(), Some(None)) {
            self.lanes.pop();
        }
        let width = edges
            .iter()
            .map(|e| e.from.max(e.to))
            .chain(std::iter::once(lane))
            .max()
            .unwrap_or(0)
            + 1;
        GraphRow {
            commit: id.to_owned(),
            lane,
            color,
            edges,
            width,
        }
    }

    fn position_of(&self, commit: &str) -> Option<usize> {
        self.lanes
            .iter()
            .position(|lane| lane.as_ref().is_some_and(|l| l.commit == commit))
    }

    /// Premier couloir libre (sauf `avoid`), ou un nouveau à droite.
    fn free_slot(&self, avoid: Option<usize>) -> usize {
        self.lanes
            .iter()
            .enumerate()
            .position(|(index, lane)| lane.is_none() && Some(index) != avoid)
            .unwrap_or_else(|| self.lanes.len().max(avoid.map_or(0, |a| a + 1)))
    }
}

/// Toutes les lignes d'un coup (même résultat que page par page).
pub fn graph_rows(commits: &[CommitSummary]) -> Vec<GraphRow> {
    GraphBuilder::new().push_page(commits)
}

/// Couleur stable (FNV-1a sur 32 bits).
fn color_of(text: &str) -> u32 {
    let mut hash: u32 = 0x811c_9dc5;
    for byte in text.bytes() {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    fn builder_rows(commits: &[(&str, &[&str])]) -> Vec<GraphRow> {
        let mut builder = GraphBuilder::new();
        commits
            .iter()
            .map(|(id, parents)| {
                let parents: Vec<String> = parents.iter().map(|p| (*p).to_owned()).collect();
                builder.push(id, &parents, None)
            })
            .collect()
    }

    #[test]
    fn a_straight_history_stays_in_lane_zero() {
        let rows = builder_rows(&[("c", &["b"]), ("b", &["a"]), ("a", &[])]);
        assert!(rows.iter().all(|row| row.lane == 0 && row.width == 1));
        assert_eq!(rows[0].color, rows[2].color, "même couleur tout du long");
        assert!(rows[2].parent_edges().next().is_none());
    }

    #[test]
    fn a_merge_links_both_parents_and_the_branch_rejoins() {
        // m = fusion de (b2) dans (a2) ; b1 part de a1.
        let rows = builder_rows(&[
            ("m", &["a2", "b2"]),
            ("a2", &["a1"]),
            ("b2", &["b1"]),
            ("b1", &["a1"]),
            ("a1", &[]),
        ]);
        let merge = &rows[0];
        assert_eq!(merge.lane, 0);
        let targets: Vec<usize> = merge.parent_edges().map(|e| e.to).collect();
        assert_eq!(
            targets,
            vec![0, 1],
            "premier parent tout droit, second à droite"
        );
        assert_eq!(rows[1].lane, 0);
        assert_eq!(rows[2].lane, 1);
        assert_eq!(rows[3].lane, 1);
        // b1 rejoint a1, déjà attendu dans le couloir 0.
        let b1_targets: Vec<usize> = rows[3].parent_edges().map(|e| e.to).collect();
        assert_eq!(b1_targets, vec![0]);
        assert_eq!(rows[4].lane, 0);
        assert_eq!(rows[4].width, 1, "couloir 1 libéré");
        // Le couloir 0 passe à côté de b2 sans bouger.
        assert!(rows[2]
            .edges
            .iter()
            .any(|e| e.from == 0 && e.to == 0 && e.half == EdgeHalf::Top));
    }

    #[test]
    fn two_tips_converge_on_their_common_parent() {
        let rows = builder_rows(&[("x", &["base"]), ("y", &["base"]), ("base", &[])]);
        assert_eq!(rows[0].lane, 0);
        assert_eq!(rows[1].lane, 1);
        assert_eq!(
            rows[1].parent_edges().map(|e| e.to).collect::<Vec<_>>(),
            vec![0]
        );
        let base = &rows[2];
        assert_eq!(base.lane, 0);
        assert!(base
            .edges
            .iter()
            .all(|e| e.half == EdgeHalf::Top && e.to == 0));
    }

    #[test]
    fn freed_lanes_are_reused_without_shifting_others() {
        // Trois branches ; celle du milieu s'arrête tôt.
        let rows = builder_rows(&[
            ("a3", &["a2"]),
            ("b1", &["root"]),
            ("c2", &["c1"]),
            ("a2", &["a1"]),
            ("c1", &["root"]),
            ("d1", &["a1"]),
            ("a1", &["root"]),
            ("root", &[]),
        ]);
        assert_eq!(rows[0].lane, 0);
        assert_eq!(rows[1].lane, 1);
        assert_eq!(rows[2].lane, 2);
        // c1 rejoint root, déjà attendu dans le couloir 1 : le couloir 2 se libère…
        assert_eq!(rows[4].lane, 2);
        // …et d1, nouvelle pointe, le reprend sans décaler les autres.
        assert_eq!(rows[5].lane, 2);
        assert!(rows[5]
            .edges
            .iter()
            .filter(|e| e.half == EdgeHalf::Top)
            .all(|e| e.from == e.to));
    }

    #[test]
    fn pages_give_the_same_rows_as_a_single_pass() {
        let mut history: Vec<(String, Vec<String>)> = Vec::new();
        // Une histoire avec fusions régulières.
        for i in (0..60u32).rev() {
            let id = format!("m{i}");
            let mut parents = Vec::new();
            if i > 0 {
                parents.push(format!("m{}", i - 1));
            }
            if i % 7 == 3 {
                parents.push(format!("side{i}"));
            }
            history.push((id, parents));
            if i % 7 == 3 {
                history.push((
                    format!("side{i}"),
                    vec![format!("m{}", i.saturating_sub(2))],
                ));
            }
        }
        let summaries: Vec<CommitSummary> = history
            .iter()
            .map(|(id, parents)| CommitSummary {
                id: id.clone(),
                short_id: id.clone(),
                parents: parents.clone(),
                author: person(),
                committer: person(),
                subject: String::new(),
                refs: Vec::new(),
            })
            .collect();
        let whole = graph_rows(&summaries);
        let mut builder = GraphBuilder::new();
        let mut paged = Vec::new();
        for page in summaries.chunks(13) {
            paged.extend(builder.push_page(page));
        }
        assert_eq!(whole, paged);
        // Chaque fusion relie ses deux parents.
        for (row, summary) in whole.iter().zip(&summaries) {
            assert_eq!(
                row.parent_edges().count(),
                summary.parents.len(),
                "{}",
                summary.id
            );
        }
    }

    #[test]
    fn ten_thousand_commits_with_thirty_merges_are_quick() {
        // Critère 28, côté calcul : 10 000 commits, 30 fusions.
        let mut builder = GraphBuilder::new();
        let started = std::time::Instant::now();
        let mut merges = 0;
        for i in (0..10_000u32).rev() {
            let mut parents = Vec::new();
            if i > 0 {
                parents.push(format!("{}", i - 1));
            }
            if i % 300 == 150 && merges < 30 {
                merges += 1;
                parents.push(format!("{}", i - 100));
            }
            let row = builder.push(&i.to_string(), &parents, None);
            assert_eq!(row.parent_edges().count(), parents.len());
        }
        assert_eq!(merges, 30);
        assert!(
            started.elapsed() < std::time::Duration::from_secs(1),
            "{:?}",
            started.elapsed()
        );
    }

    #[test]
    fn colors_are_stable_and_follow_the_branch_name() {
        assert_eq!(color_of("feature/login"), color_of("feature/login"));
        assert_ne!(color_of("feature/login"), color_of("main"));
        let mut a = GraphBuilder::new();
        let mut b = GraphBuilder::new();
        let row_a = a.push("1111", &[], Some("main"));
        let row_b = b.push("2222", &[], Some("main"));
        assert_eq!(row_a.color, row_b.color);
    }

    fn person() -> crate::history::Person {
        crate::history::Person {
            name: String::new(),
            email: String::new(),
            time: 0,
            offset_minutes: 0,
        }
    }
}
