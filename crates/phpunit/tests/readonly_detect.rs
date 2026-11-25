//! Vérification **en lecture seule** de la détection sur un vrai projet, désigné par
//! la variable `JULIE_READONLY_DETECT_ROOT` (sans elle, l'essai passe sans rien
//! faire). Décision n°8 de `docs/specs/decisions-v1.md` : sur `backoffice`, seule la
//! détection est vérifiée ; la suite de tests n'est jamais lancée.
//!
//! Ce que fait l'essai : `is_file` sur les emplacements candidats, la commande qui
//! serait lancée (sans la lancer), puis `php <script> --version`. Rien n'est écrit :
//! ni `Project::ensure_data_dir`, ni cache, ni aucun fichier.

use std::path::{Path, PathBuf};

use julie_core::CancelToken;
use julie_phpunit::{
    detect_files, preview_command, probe_phpunit, PhpUnitSettings, ProbeSuccess, RunRequest,
    PROBE_TIMEOUT,
};

#[test]
fn detection_of_a_real_project_without_writing_anything() {
    let Some(root) = std::env::var_os("JULIE_READONLY_DETECT_ROOT").map(PathBuf::from) else {
        eprintln!("essai sauté : JULIE_READONLY_DETECT_ROOT n'est pas défini");
        return;
    };
    let settings = PhpUnitSettings::default();

    let detection = detect_files(&root, &settings);
    println!("php           : {:?}", detection.php);
    println!("phpunit       : {:?}", detection.phpunit);
    println!("configuration : {:?}", detection.configuration);
    let unit = detection.phpunit.as_ref().expect("PHPUnit détecté");
    assert!(unit.path.is_file());

    let preview = preview_command(&root, &settings, &RunRequest::all()).expect("commande");
    println!("COMMANDE : {}", preview.command_line);
    println!("dossier de travail : {}", preview.working_dir.display());

    // `php <script> --version` seulement.
    let probed = probe_phpunit(
        &root,
        None,
        Path::new(&unit.path),
        &CancelToken::new(),
        PROBE_TIMEOUT,
    )
    .unwrap();
    match probed {
        Ok(ProbeSuccess::PhpUnit { path, version }) => {
            println!("VERSION : PHPUnit {} ({})", version.text, path.display());
        }
        other => panic!("{other:?}"),
    }
}
