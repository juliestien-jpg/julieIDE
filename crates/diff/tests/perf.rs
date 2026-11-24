//! Mesures sur de gros fichiers (20 000 lignes et plus) : pas de quadratique caché.
//!
//! Les temps s'affichent avec `--nocapture`. Les bornes vérifiées sont larges
//! (machine partagée, compilation sans optimisation) : elles attrapent un
//! comportement quadratique, pas une régression de quelques pour cent. Pour des
//! chiffres représentatifs :
//!
//! ```text
//! cargo test -p julie-diff --release --test perf -- --nocapture --test-threads=1
//! ```

mod common;

use std::time::{Duration, Instant};

use common::Rng;
use julie_core::CancelToken;
use julie_diff::{diff_lines, ApplyScope, DiffOptions, MergeSession, Side};

/// Un fichier qui ressemble à du code : beaucoup de lignes uniques, mais aussi
/// des lignes vides, des accolades et des `return` répétés partout.
fn source(lines: usize) -> Vec<String> {
    let mut out = Vec::with_capacity(lines);
    let mut i = 0;
    while out.len() < lines {
        i += 1;
        out.push(format!("    public function method{i}(array $rows): int"));
        out.push("    {".to_string());
        out.push(format!("        $total = count($rows) + {i};"));
        out.push("        foreach ($rows as $row) {".to_string());
        out.push(format!("            $total += $row['value{}'];", i % 7));
        out.push("        }".to_string());
        out.push("        return $total;".to_string());
        out.push("    }".to_string());
        out.push(String::new());
    }
    out.truncate(lines);
    out
}

fn join(lines: &[String]) -> String {
    let mut out = String::with_capacity(lines.len() * 32);
    for line in lines {
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// `count` modifications réparties dans le fichier, décalées de `phase` lignes.
fn edited(lines: &[String], count: usize, phase: usize, tag: &str) -> Vec<String> {
    let mut out = lines.to_vec();
    let step = lines.len() / count;
    // De la fin vers le début : les indices restent valides.
    for k in (0..count).rev() {
        let at = k * step + phase;
        match k % 3 {
            0 => out[at] = format!("{} // {tag} {k}", out[at]),
            1 => out.insert(at, format!("        // ajout {tag} {k}")),
            _ => {
                out.remove(at);
            }
        }
    }
    out
}

fn timed<T>(label: &str, limit: Duration, work: impl FnOnce() -> T) -> (T, Duration) {
    let start = Instant::now();
    let value = work();
    let elapsed = start.elapsed();
    println!("{label:<58} {:>9.2} ms", elapsed.as_secs_f64() * 1000.0);
    assert!(
        elapsed < limit,
        "{label} : {elapsed:?}, au-delà de la borne {limit:?}"
    );
    (value, elapsed)
}

const GENEROUS: Duration = Duration::from_secs(20);

#[test]
fn diff_of_20_000_lines() {
    let cancel = CancelToken::new();
    let options = DiffOptions::default();
    let lines = source(20_000);
    let left = join(&lines);

    let right = join(&edited(&lines, 200, 3, "édition"));
    let (diff, _) = timed(
        "diff 20 000 lignes, 200 changements épars",
        GENEROUS,
        || diff_lines(&left, &right, &options, &cancel).unwrap(),
    );
    assert!(diff.hunks.len() >= 190, "{} blocs", diff.hunks.len());

    let (diff, _) = timed("diff 20 000 lignes, fichiers identiques", GENEROUS, || {
        diff_lines(&left, &left, &options, &cancel).unwrap()
    });
    assert!(diff.identical);

    // Rien en commun : tout est remplacé.
    let other: String = (0..20_000).map(|i| format!("autre ligne {i}\n")).collect();
    let (diff, _) = timed("diff 20 000 lignes, rien en commun", GENEROUS, || {
        diff_lines(&left, &other, &options, &cancel).unwrap()
    });
    assert_eq!(diff.hunks.len(), 1);

    // Seules les lignes répétées (accolades, lignes vides) sont communes.
    let similar = join(
        &source(20_000)
            .into_iter()
            .map(|line| line.replace("method", "handler").replace("$total", "$sum"))
            .collect::<Vec<_>>(),
    );
    timed(
        "diff 20 000 lignes, seules les lignes banales communes",
        GENEROUS,
        || diff_lines(&left, &similar, &options, &cancel).unwrap(),
    );

    // Le pire pour Myers : les mêmes lignes dans un autre ordre.
    let mut shuffled = lines.clone();
    let mut rng = Rng(7);
    for i in (1..shuffled.len()).rev() {
        shuffled.swap(i, rng.below(i + 1));
    }
    let shuffled = join(&shuffled);
    timed(
        "diff 20 000 lignes, mêmes lignes mélangées",
        GENEROUS,
        || diff_lines(&left, &shuffled, &options, &cancel).unwrap(),
    );

    // Deux moitiés échangées : un grand déplacement.
    let mut swapped = lines[10_000..].to_vec();
    swapped.extend_from_slice(&lines[..10_000]);
    let swapped = join(&swapped);
    timed(
        "diff 20 000 lignes, deux moitiés échangées",
        GENEROUS,
        || diff_lines(&left, &swapped, &options, &cancel).unwrap(),
    );

    // Une seule ligne répétée 20 000 fois, 500 en moins.
    let blank = "\n".repeat(20_000);
    let fewer = "\n".repeat(19_500);
    timed("diff 20 000 lignes vides contre 19 500", GENEROUS, || {
        diff_lines(&blank, &fewer, &options, &cancel).unwrap()
    });

    // Une ligne très longue modifiée (affinage au mot compris).
    let long_left = format!("{}\n", "mot ".repeat(50_000));
    let long_right = format!("{}fin\n", "mot ".repeat(50_000));
    timed("diff d'une ligne de 200 000 caractères", GENEROUS, || {
        diff_lines(&long_left, &long_right, &options, &cancel).unwrap()
    });
}

/// Doubler la taille ne doit pas quadrupler le temps, même dans le pire cas.
#[test]
fn diff_time_grows_about_linearly() {
    let cancel = CancelToken::new();
    let options = DiffOptions::default();
    let mut times = Vec::new();
    for size in [10_000usize, 20_000, 40_000, 80_000] {
        let lines = source(size);
        let left = join(&lines);
        let mut shuffled = lines.clone();
        let mut rng = Rng(11);
        for i in (1..shuffled.len()).rev() {
            shuffled.swap(i, rng.below(i + 1));
        }
        let right = join(&shuffled);
        let label = format!("diff mélangé, {size} lignes");
        let (_, elapsed) = timed(&label, Duration::from_secs(60), || {
            diff_lines(&left, &right, &options, &cancel).unwrap()
        });
        times.push(elapsed.as_secs_f64());
    }
    let ratio = times[3] / times[1].max(1e-9);
    println!("rapport 80 000 / 20 000 lignes : x{ratio:.1} (linéaire = 4, quadratique = 16)");
    assert!(ratio < 10.0, "croissance trop rapide : x{ratio:.1}");
}

#[test]
fn merge_of_20_000_lines() {
    let cancel = CancelToken::new();
    let lines = source(20_000);
    let base = join(&lines);
    let ours = join(&edited(&lines, 200, 2, "nôtre"));
    // Une fois sur quatre, leur changement tombe sur le nôtre : des conflits.
    let theirs_lines = edited(&lines, 200, 40, "leur");
    let theirs_lines = edited(&theirs_lines, 50, 2, "leur aussi");
    let theirs = join(&theirs_lines);

    let (mut session, _) = timed(
        "fusion 20 000 lignes : ouverture de la session",
        GENEROUS,
        || MergeSession::new(&base, &ours, &theirs, &cancel).unwrap(),
    );
    let counts = session.counts();
    println!(
        "    {} changements, {} conflits",
        counts.changes, counts.conflicts
    );
    assert!(counts.changes > 200 && counts.conflicts > 10);

    timed("fusion 20 000 lignes : appliquer tous", GENEROUS, || {
        session.apply_non_conflicting(ApplyScope::All)
    });
    timed(
        "fusion 20 000 lignes : blocs + correspondance",
        GENEROUS,
        || (session.blocks(), session.alignment()),
    );
    timed(
        "fusion 20 000 lignes : accepter chaque conflit",
        GENEROUS,
        || {
            for block in session.blocks() {
                if !block.resolved {
                    session.accept(block.id, Side::Left).unwrap();
                }
            }
        },
    );
    assert!(session.is_all_resolved());
    timed("fusion 20 000 lignes : 500 frappes", GENEROUS, || {
        for i in 0..500 {
            session
                .replace_lines(i * 30..i * 30 + 1, &["ligne tapée"])
                .unwrap();
        }
    });
    let (text, _) = timed(
        "fusion 20 000 lignes : texte du résultat",
        GENEROUS,
        || session.result_text(),
    );
    assert!(text.len() > 400_000);
    timed("fusion 20 000 lignes : tout annuler", GENEROUS, || {
        while session.undo() {}
    });
    assert_eq!(session.result_text(), base);
}

/// Critère d'acceptation 12 de `docs/specs/fusion.md` : 10 000 lignes et 200
/// blocs, ouverture en moins de 2 s, « appliquer tous » en moins de 0,5 s.
#[test]
fn acceptance_criterion_12() {
    let cancel = CancelToken::new();
    let lines = source(10_000);
    let base = join(&lines);
    let ours = join(&edited(&lines, 100, 2, "nôtre"));
    let theirs = join(&edited(&lines, 100, 30, "leur"));
    let (mut session, _) = timed(
        "critère 12 : ouverture, 10 000 lignes et 200 blocs",
        Duration::from_secs(2),
        || MergeSession::new(&base, &ours, &theirs, &cancel).unwrap(),
    );
    assert_eq!(session.block_count(), 200);
    timed(
        "critère 12 : appliquer tous",
        Duration::from_millis(500),
        || session.apply_non_conflicting(ApplyScope::All),
    );
    assert!(session.is_all_resolved());
}

/// Zones d'ambiguïté : un fichier fait d'une seule ligne répétée, où chaque
/// changement pourrait glisser sur tout le fichier.
#[test]
fn merge_of_repetitive_files() {
    let cancel = CancelToken::new();
    let unit = "}\n";
    let base = unit.repeat(20_000);
    let ours = unit.repeat(19_000);
    let mut theirs = String::new();
    for i in 0..20_000 {
        theirs.push_str(if i % 100 == 50 { "x\n" } else { unit });
    }
    let (session, _) = timed("fusion de 20 000 lignes identiques", GENEROUS, || {
        MergeSession::new(&base, &ours, &theirs, &cancel).unwrap()
    });
    // Tout est ambigu : un seul conflit, et c'est voulu.
    assert_eq!(session.counts().conflicts, 1);
    assert_eq!(session.counts().changes, 0);

    // Motif alterné « a b a b… » précédé d'un en-tête : 2 000 changements de
    // notre côté dans le motif, qui se soudent en une seule longue zone derrière
    // un premier changement resté à part. Le leur ne touche que l'en-tête.
    let mut lines: Vec<String> = (0..20).map(|i| format!("en-tête {i}")).collect();
    lines.extend((0..20_000).map(|i| if i % 2 == 0 { "a" } else { "b" }.to_string()));
    let base = join(&lines);
    let mut ours_lines = lines.clone();
    ours_lines[5] = "en-tête modifié".to_string();
    for i in (20..20_020).step_by(10) {
        ours_lines[i] = "b".to_string();
    }
    let ours = join(&ours_lines);
    let mut theirs_lines = lines.clone();
    theirs_lines[12] = "en-tête modifié en face".to_string();
    let theirs = join(&theirs_lines);
    timed(
        "diff d'un motif alterné, 2 000 changements",
        GENEROUS,
        || diff_lines(&base, &ours, &DiffOptions::default(), &cancel).unwrap(),
    );
    let (session, _) = timed(
        "fusion d'un motif alterné, 2 000 changements",
        GENEROUS,
        || MergeSession::new(&base, &ours, &theirs, &cancel).unwrap(),
    );
    assert_eq!(session.counts().conflicts, 0);
    let (mut merged, _) = timed("fusion d'un motif alterné : résultat", GENEROUS, || {
        let mut session = session.clone();
        session.apply_non_conflicting(ApplyScope::All);
        session
    });
    assert!(merged.is_all_resolved());
    assert_eq!(merged.result_text().matches("modifié").count(), 2);
    assert!(merged.undo());
}

#[test]
fn cancellation_stops_a_long_diff() {
    let cancel = CancelToken::new();
    cancel.cancel();
    let lines = source(20_000);
    let left = join(&lines);
    let right = join(&edited(&lines, 200, 3, "x"));
    let error = diff_lines(&left, &right, &DiffOptions::default(), &cancel).unwrap_err();
    assert!(error.is_cancelled());
    let error = MergeSession::new(&left, &right, &left, &cancel).unwrap_err();
    assert!(error.is_cancelled());
}
