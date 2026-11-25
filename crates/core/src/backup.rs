//! Copies de sécurité prises avant une opération destructive (« Annuler les
//! modifications », bascule forcée…).
//!
//! ```text
//! <racine>/.juliestorm/annulations/
//!     20261125-143012.toml          manifeste : écrit en dernier, il valide la copie
//!     20261125-143012/<chemin relatif au projet>
//! ```
//!
//! Règle d'usage : l'opération destructive n'a lieu **que si** [`create`] a
//! réussi. Une copie sans manifeste (création interrompue) est ignorée par
//! [`list`] et retirée par la purge.
//!
//! Tout ce module fait des entrées-sorties **bloquantes** : tâche de fond.

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::file::write_atomic;
use crate::project::Project;

/// Nom du dossier des copies dans `.juliestorm/`.
pub const BACKUP_DIR_NAME: &str = "annulations";

/// Durée de conservation des copies : sept jours.
pub const RETENTION: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// Raison de la copie que [`restore`] prend elle-même avant d'écraser.
pub const RESTORE_REASON: &str = "backup.restore";

const MANIFEST_VERSION: u32 = 1;

/// Une copie de sécurité.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Backup {
    /// Identifiant : l'horodatage local `AAAAMMJJ-HHMMSS` (suivi de `-2`, `-3`…
    /// si plusieurs copies sont prises dans la même seconde). C'est aussi le nom
    /// du dossier.
    pub id: String,
    /// Date de création, à la seconde.
    pub created: SystemTime,
    /// Clé stable de l'opération qui a demandé la copie (`"vcs.discard"`,
    /// `"vcs.checkout"`…) : l'interface en tire le libellé dans son catalogue.
    pub reason: String,
    pub files: Vec<BackupFile>,
}

/// Un fichier d'une copie.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupFile {
    /// Chemin **absolu** du fichier dans le projet.
    pub path: PathBuf,
    /// `false` : le fichier n'existait pas au moment de la copie (rien n'a été
    /// copié, et le rétablissement n'y touche pas).
    pub present: bool,
    /// Taille copiée, en octets.
    pub size: u64,
}

/// Ce qu'a fait [`restore`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RestoreReport {
    /// Fichiers remis dans l'état de la copie.
    pub restored: Vec<PathBuf>,
    /// Fichiers déjà identiques à la copie : non touchés.
    pub unchanged: Vec<PathBuf>,
    /// Fichiers qui n'ont pas pu être rétablis, avec la raison technique. Les
    /// autres l'ont été quand même.
    pub failed: Vec<(PathBuf, String)>,
    /// Copie prise par le rétablissement lui-même, des fichiers qu'il a écrasés.
    pub safety_backup: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct Manifest {
    version: u32,
    id: String,
    created_unix: u64,
    reason: String,
    #[serde(default)]
    files: Vec<ManifestFile>,
}

#[derive(Serialize, Deserialize)]
struct ManifestFile {
    /// Relatif à la racine du projet, séparateurs `/`.
    path: String,
    present: bool,
    size: u64,
}

/// Dossier des copies du projet, qu'il existe ou non.
pub fn backups_dir(project: &Project) -> PathBuf {
    project.data_dir().join(BACKUP_DIR_NAME)
}

/// Où se trouve, dans la copie `id`, la copie du fichier `path` du projet (pour
/// l'ouvrir ou la comparer). `None` si `path` est hors du projet.
pub fn copy_path(project: &Project, id: &str, path: &Path) -> Option<PathBuf> {
    let relative = relative_string(project, path).ok()?;
    is_valid_id(id).then(|| backups_dir(project).join(id).join(relative))
}

/// Copie `files` (chemins absolus de **fichiers** du projet) avant une opération
/// destructive.
///
/// - Un fichier absent est noté comme tel dans le manifeste, sans erreur.
/// - Un dossier, un chemin hors du projet ou une liste vide : [`Error::Invalid`].
/// - En cas d'échec, la copie partielle est retirée et l'erreur rendue :
///   l'appelant **renonce** alors à l'opération destructive.
pub fn create(project: &Project, reason: &str, files: &[PathBuf]) -> Result<Backup> {
    if files.is_empty() {
        return Err(Error::Invalid(
            "copie de sécurité : aucun fichier".to_owned(),
        ));
    }
    // Tout est vérifié avant la première écriture.
    let mut entries: Vec<(PathBuf, String, bool)> = Vec::with_capacity(files.len());
    for path in files {
        let relative = relative_string(project, path)?;
        if entries.iter().any(|(_, known, _)| *known == relative) {
            continue;
        }
        let present = match fs::metadata(path) {
            Ok(meta) if meta.is_dir() => {
                return Err(Error::Invalid(format!(
                    "copie de sécurité : {} est un dossier",
                    path.display()
                )))
            }
            Ok(_) => true,
            Err(error) if error.kind() == io::ErrorKind::NotFound => false,
            Err(source) => return Err(Error::io(path, source)),
        };
        entries.push((path.clone(), relative, present));
    }

    project.ensure_data_dir()?;
    let root = backups_dir(project);
    fs::create_dir_all(&root).map_err(|source| Error::io(&root, source))?;
    let now = SystemTime::now();
    let (id, dir) = claim_directory(&root, &local_timestamp(now))?;

    let copy_all = || -> Result<Vec<BackupFile>> {
        let mut copied = Vec::with_capacity(entries.len());
        for (path, relative, present) in &entries {
            let mut size = 0;
            if *present {
                let bytes = fs::read(path).map_err(|source| Error::io(path, source))?;
                let target = dir.join(relative);
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent).map_err(|source| Error::io(parent, source))?;
                }
                write_atomic(&target, &bytes)?;
                size = bytes.len() as u64;
            }
            copied.push(BackupFile {
                path: path.clone(),
                present: *present,
                size,
            });
        }
        Ok(copied)
    };
    let created_unix = now
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let result = copy_all().and_then(|copied| {
        let manifest = Manifest {
            version: MANIFEST_VERSION,
            id: id.clone(),
            created_unix,
            reason: reason.to_owned(),
            files: entries
                .iter()
                .zip(&copied)
                .map(|((_, relative, _), file)| ManifestFile {
                    path: relative.clone(),
                    present: file.present,
                    size: file.size,
                })
                .collect(),
        };
        let text = toml::to_string(&manifest)
            .map_err(|error| Error::Invalid(format!("manifeste de copie : {error}")))?;
        write_atomic(manifest_path(&root, &id), text.as_bytes())?;
        Ok(copied)
    });

    match result {
        Ok(files) => Ok(Backup {
            id,
            created: UNIX_EPOCH + Duration::from_secs(created_unix),
            reason: reason.to_owned(),
            files,
        }),
        Err(error) => {
            let _ = fs::remove_dir_all(&dir);
            Err(error)
        }
    }
}

/// Les copies du projet, de la plus récente à la plus ancienne. Une copie sans
/// manifeste lisible est ignorée.
pub fn list(project: &Project) -> Result<Vec<Backup>> {
    let root = backups_dir(project);
    let mut backups: Vec<Backup> = manifest_ids(&root)?
        .into_iter()
        .filter_map(|id| read_manifest(project, &root, &id))
        .collect();
    backups.sort_by(|a, b| b.created.cmp(&a.created).then_with(|| b.id.cmp(&a.id)));
    Ok(backups)
}

/// Remet les fichiers de la copie `id` dans l'état où elle les a pris.
///
/// - Un fichier déjà identique n'est pas touché ; un fichier absent lors de la
///   copie non plus (le rétablissement ne supprime jamais rien).
/// - Avant d'écraser quoi que ce soit, les fichiers concernés qui existent sont
///   eux-mêmes copiés (raison [`RESTORE_REASON`]) : rétablir se défait aussi. Si
///   cette copie échoue, rien n'est touché et l'erreur est rendue.
/// - Écriture atomique fichier par fichier ; un échec (lecture seule, fichier
///   verrouillé) est noté dans le rapport et n'arrête pas les autres.
/// - La copie reste en place : elle partira avec la purge.
pub fn restore(project: &Project, id: &str) -> Result<RestoreReport> {
    let root = backups_dir(project);
    let unknown = || Error::Invalid(format!("copie de sécurité inconnue : {id}"));
    if !is_valid_id(id) {
        return Err(unknown());
    }
    let backup = read_manifest(project, &root, id).ok_or_else(unknown)?;
    let dir = root.join(id);

    let mut report = RestoreReport::default();
    let mut to_write: Vec<(PathBuf, Vec<u8>)> = Vec::new();
    let mut overwritten: Vec<PathBuf> = Vec::new();
    for file in backup.files.iter().filter(|file| file.present) {
        let Some(copy) = project.relative(&file.path).map(|rel| dir.join(rel)) else {
            continue;
        };
        let saved = match fs::read(&copy) {
            Ok(bytes) => bytes,
            Err(error) => {
                report.failed.push((file.path.clone(), error.to_string()));
                continue;
            }
        };
        match fs::read(&file.path) {
            Ok(current) if current == saved => report.unchanged.push(file.path.clone()),
            Ok(_) => {
                overwritten.push(file.path.clone());
                to_write.push((file.path.clone(), saved));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                to_write.push((file.path.clone(), saved));
            }
            Err(error) => report.failed.push((file.path.clone(), error.to_string())),
        }
    }

    if !overwritten.is_empty() {
        report.safety_backup = Some(create(project, RESTORE_REASON, &overwritten)?.id);
    }
    for (path, bytes) in to_write {
        let written = match path.parent() {
            Some(parent) => fs::create_dir_all(parent).map_err(|source| Error::io(parent, source)),
            None => Ok(()),
        }
        .and_then(|()| write_atomic(&path, &bytes));
        match written {
            Ok(()) => report.restored.push(path),
            Err(error) => report.failed.push((path, error.to_string())),
        }
    }
    Ok(report)
}

/// Retire les copies de plus de sept jours ([`RETENTION`]). À appeler à
/// l'ouverture du projet. Rend le nombre de copies retirées.
pub fn purge_expired(project: &Project) -> Result<usize> {
    purge_older_than(project, RETENTION, SystemTime::now())
}

/// Retire les copies créées plus de `max_age` avant `now`, ainsi que les restes
/// de copies interrompues (dossier sans manifeste, manifeste illisible) du même
/// âge. Un élément qui résiste (fichier verrouillé) est laissé pour la fois
/// suivante.
pub fn purge_older_than(project: &Project, max_age: Duration, now: SystemTime) -> Result<usize> {
    let root = backups_dir(project);
    let expired = |created: SystemTime| now.duration_since(created).is_ok_and(|age| age > max_age);
    let modified = |path: &Path| fs::metadata(path).and_then(|meta| meta.modified()).ok();

    let mut removed = 0;
    let ids = manifest_ids(&root)?;
    for id in &ids {
        let manifest = manifest_path(&root, id);
        let created = match read_manifest(project, &root, id) {
            Some(backup) => Some(backup.created),
            None => modified(&manifest),
        };
        if created.is_some_and(expired) {
            // Le manifeste d'abord : la copie cesse d'être listée, puis disparaît.
            if fs::remove_file(&manifest).is_ok() {
                let _ = fs::remove_dir_all(root.join(id));
                removed += 1;
            }
        }
    }

    // Dossiers sans manifeste : copies interrompues.
    if let Ok(entries) = fs::read_dir(&root) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            let path = entry.path();
            if is_valid_id(name)
                && path.is_dir()
                && !ids.iter().any(|id| id == name)
                && modified(&path).is_some_and(expired)
            {
                let _ = fs::remove_dir_all(&path);
            }
        }
    }
    Ok(removed)
}

fn manifest_path(root: &Path, id: &str) -> PathBuf {
    root.join(format!("{id}.toml"))
}

/// Identifiants des manifestes présents (dossier absent : aucun).
fn manifest_ids(root: &Path) -> Result<Vec<String>> {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => return Err(Error::io(root, source)),
    };
    let mut ids = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        if let Some(id) = name.to_str().and_then(|name| name.strip_suffix(".toml")) {
            if is_valid_id(id) {
                ids.push(id.to_owned());
            }
        }
    }
    Ok(ids)
}

/// Le manifeste de la copie `id`, s'il est lisible et cohérent.
fn read_manifest(project: &Project, root: &Path, id: &str) -> Option<Backup> {
    let text = fs::read_to_string(manifest_path(root, id)).ok()?;
    let manifest: Manifest = toml::from_str(&text).ok()?;
    if manifest.version != MANIFEST_VERSION || manifest.id != id {
        return None;
    }
    let mut files = Vec::with_capacity(manifest.files.len());
    for file in manifest.files {
        // Un manifeste retouché à la main ne doit pas faire écrire hors du projet.
        let relative = Path::new(&file.path);
        if !is_plain_relative(relative) {
            return None;
        }
        files.push(BackupFile {
            path: project.absolute(relative),
            present: file.present,
            size: file.size,
        });
    }
    Some(Backup {
        id: manifest.id,
        created: UNIX_EPOCH + Duration::from_secs(manifest.created_unix),
        reason: manifest.reason,
        files,
    })
}

/// Chemin relatif à la racine, séparateurs `/`, pour le manifeste.
fn relative_string(project: &Project, path: &Path) -> Result<String> {
    let invalid =
        |reason: &str| Error::Invalid(format!("copie de sécurité : {} {reason}", path.display()));
    let relative = project
        .relative(path)
        .ok_or_else(|| invalid("est hors du projet"))?;
    if !is_plain_relative(relative) {
        return Err(invalid("n'est pas un chemin de fichier du projet"));
    }
    let mut parts = Vec::new();
    for component in relative.components() {
        let part = component
            .as_os_str()
            .to_str()
            .ok_or_else(|| invalid("n'est pas de l'Unicode valide"))?;
        parts.push(part);
    }
    Ok(parts.join("/"))
}

/// Relatif, non vide, fait seulement de noms (ni `..`, ni racine, ni lecteur).
fn is_plain_relative(path: &Path) -> bool {
    let mut components = path.components().peekable();
    components.peek().is_some() && components.all(|part| matches!(part, Component::Normal(_)))
}

fn is_valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 40
        && id.starts_with(|c: char| c.is_ascii_digit())
        && id.chars().all(|c| c.is_ascii_digit() || c == '-')
}

/// Réserve un dossier neuf pour la copie : `base`, sinon `base-2`, `base-3`…
fn claim_directory(root: &Path, base: &str) -> Result<(String, PathBuf)> {
    for attempt in 1..1000u32 {
        let id = if attempt == 1 {
            base.to_owned()
        } else {
            format!("{base}-{attempt}")
        };
        let dir = root.join(&id);
        if manifest_path(root, &id).exists() {
            continue;
        }
        match fs::create_dir(&dir) {
            Ok(()) => return Ok((id, dir)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(source) => return Err(Error::io(&dir, source)),
        }
    }
    Err(Error::Invalid(format!(
        "copie de sécurité : trop de copies à {base}"
    )))
}

/// `AAAAMMJJ-HHMMSS`, heure locale sous Windows.
#[cfg(windows)]
fn local_timestamp(_now: SystemTime) -> String {
    use windows_sys::Win32::Foundation::SYSTEMTIME;
    use windows_sys::Win32::System::SystemInformation::GetLocalTime;
    let mut time = SYSTEMTIME {
        wYear: 0,
        wMonth: 0,
        wDayOfWeek: 0,
        wDay: 0,
        wHour: 0,
        wMinute: 0,
        wSecond: 0,
        wMilliseconds: 0,
    };
    // SAFETY: `GetLocalTime` remplit la structure fournie et n'échoue pas.
    unsafe { GetLocalTime(&mut time) };
    format!(
        "{:04}{:02}{:02}-{:02}{:02}{:02}",
        time.wYear, time.wMonth, time.wDay, time.wHour, time.wMinute, time.wSecond
    )
}

/// `AAAAMMJJ-HHMMSS`, en temps universel hors de Windows.
#[cfg(not(windows))]
fn local_timestamp(now: SystemTime) -> String {
    let seconds = now
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let (days, rest) = (seconds / 86_400, seconds % 86_400);
    // Date civile à partir du nombre de jours (algorithme de Howard Hinnant).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}{month:02}{day:02}-{:02}{:02}{:02}",
        rest / 3_600,
        rest % 3_600 / 60,
        rest % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Un projet jetable dont le chemin a des espaces, des parenthèses et des
    /// accents.
    fn project() -> (tempfile::TempDir, Project) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("Projet d'été (x86)");
        fs::create_dir_all(root.join("src").join("Contrôleur vidéo")).unwrap();
        (dir, Project::open(&root).unwrap())
    }

    fn write(project: &Project, relative: &str, bytes: &[u8]) -> PathBuf {
        let path = project.absolute(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn a_backup_copies_the_exact_bytes_and_lists_itself() {
        let (_dir, project) = project();
        // Windows-1252, CRLF : la copie est octet pour octet.
        let legacy = b"<?php\r\n// cha\xEEne \x93cit\xE9e\x94\r\n";
        let a = write(&project, "src/Contrôleur vidéo/Mon fichier (1).php", legacy);
        let b = write(&project, "à la racine.txt", "été\n".as_bytes());
        let absent = project.absolute("src/pas encore là.php");

        let backup = create(
            &project,
            "vcs.discard",
            &[a.clone(), b.clone(), absent.clone(), a.clone()],
        )
        .unwrap();

        assert_eq!(backup.reason, "vcs.discard");
        assert_eq!(backup.id.len(), "AAAAMMJJ-HHMMSS".len());
        assert!(is_valid_id(&backup.id));
        assert_eq!(
            backup.files,
            vec![
                BackupFile {
                    path: a.clone(),
                    present: true,
                    size: legacy.len() as u64
                },
                BackupFile {
                    path: b.clone(),
                    present: true,
                    size: "été\n".len() as u64
                },
                BackupFile {
                    path: absent.clone(),
                    present: false,
                    size: 0
                },
            ]
        );

        let dir = project
            .root()
            .join(".juliestorm")
            .join("annulations")
            .join(&backup.id);
        let copy = dir
            .join("src")
            .join("Contrôleur vidéo")
            .join("Mon fichier (1).php");
        assert_eq!(fs::read(&copy).unwrap(), legacy);
        assert_eq!(copy_path(&project, &backup.id, &a), Some(copy));
        assert_eq!(
            fs::read(dir.join("à la racine.txt")).unwrap(),
            "été\n".as_bytes()
        );
        assert!(!dir.join("src").join("pas encore là.php").exists());
        // .juliestorm/ est ignoré par Git : les copies n'apparaissent pas dans les
        // changements locaux.
        assert!(project.data_dir().join(".gitignore").exists());

        assert_eq!(list(&project).unwrap(), vec![backup]);
    }

    #[test]
    fn restore_puts_files_back_and_can_itself_be_undone() {
        let (_dir, project) = project();
        let a = write(&project, "src/a.php", b"version d'origine de a\r\n");
        let b = write(&project, "src/sous dossier/b.php", b"origine de b");
        let c = write(&project, "c.txt", b"inchang\xE9");
        let backup = create(&project, "vcs.discard", &[a.clone(), b.clone(), c.clone()]).unwrap();

        // L'opération destructive : a écrasé, b et son dossier supprimés.
        fs::write(&a, b"contenu de HEAD").unwrap();
        fs::remove_dir_all(b.parent().unwrap()).unwrap();

        let report = restore(&project, &backup.id).unwrap();
        assert_eq!(report.restored, vec![a.clone(), b.clone()]);
        assert_eq!(report.unchanged, vec![c.clone()]);
        assert!(report.failed.is_empty());
        assert_eq!(fs::read(&a).unwrap(), b"version d'origine de a\r\n");
        assert_eq!(fs::read(&b).unwrap(), b"origine de b");
        assert_eq!(fs::read(&c).unwrap(), b"inchang\xE9");

        // Ce que le rétablissement a écrasé est lui-même copié.
        let safety = report.safety_backup.expect("copie avant rétablissement");
        let backups = list(&project).unwrap();
        assert_eq!(backups.len(), 2);
        let safety_backup = backups.iter().find(|backup| backup.id == safety).unwrap();
        assert_eq!(safety_backup.reason, RESTORE_REASON);
        assert_eq!(safety_backup.files.len(), 1);
        assert_eq!(
            fs::read(copy_path(&project, &safety, &a).unwrap()).unwrap(),
            b"contenu de HEAD"
        );

        // Rétablir deux fois ne change plus rien et ne crée pas de copie.
        let again = restore(&project, &backup.id).unwrap();
        assert!(again.restored.is_empty() && again.safety_backup.is_none());
        assert_eq!(again.unchanged.len(), 3);
        assert_eq!(list(&project).unwrap().len(), 2);

        // Défaire le rétablissement.
        restore(&project, &safety).unwrap();
        assert_eq!(fs::read(&a).unwrap(), b"contenu de HEAD");
    }

    #[test]
    fn restore_reports_what_it_could_not_write_and_does_the_rest() {
        let (_dir, project) = project();
        let a = write(&project, "a.txt", b"a d'origine");
        let b = write(&project, "b.txt", b"b d'origine");
        let backup = create(&project, "vcs.checkout", &[a.clone(), b.clone()]).unwrap();
        fs::write(&a, b"a modifi\xE9").unwrap();
        fs::write(&b, b"b modifi\xE9").unwrap();
        let mut permissions = fs::metadata(&a).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&a, permissions.clone()).unwrap();

        let report = restore(&project, &backup.id).unwrap();
        assert_eq!(report.restored, vec![b.clone()]);
        assert_eq!(report.failed.len(), 1);
        assert_eq!(report.failed[0].0, a);
        assert_eq!(fs::read(&a).unwrap(), b"a modifi\xE9", "jamais forcé");
        assert_eq!(fs::read(&b).unwrap(), b"b d'origine");

        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        fs::set_permissions(&a, permissions).unwrap();

        assert!(matches!(
            restore(&project, "20000101-000000"),
            Err(Error::Invalid(_))
        ));
        assert!(matches!(
            restore(&project, "../ailleurs"),
            Err(Error::Invalid(_))
        ));
    }

    #[test]
    fn invalid_requests_are_refused_and_leave_nothing_behind() {
        let (dir, project) = project();
        let inside = write(&project, "src/a.php", b"a");
        let outside = dir.path().join("hors du projet.txt");
        fs::write(&outside, "x").unwrap();

        assert!(matches!(
            create(&project, "essai", &[]),
            Err(Error::Invalid(_))
        ));
        assert!(matches!(
            create(&project, "essai", &[inside.clone(), outside]),
            Err(Error::Invalid(_))
        ));
        assert!(matches!(
            create(&project, "essai", &[project.absolute("src")]),
            Err(Error::Invalid(_))
        ));
        assert!(matches!(
            create(
                &project,
                "essai",
                &[project.root().join("src").join("..").join("x")]
            ),
            Err(Error::Invalid(_))
        ));
        assert!(matches!(
            create(&project, "essai", &[PathBuf::from("relatif.txt")]),
            Err(Error::Invalid(_))
        ));
        assert!(!project.data_dir().exists(), "rien n'est créé sur un refus");
        assert!(list(&project).unwrap().is_empty());
        assert_eq!(purge_expired(&project).unwrap(), 0);
    }

    #[test]
    fn several_backups_in_the_same_second_get_distinct_ids() {
        let (_dir, project) = project();
        let a = write(&project, "a.txt", b"1");
        let ids: Vec<String> = (0..3)
            .map(|round| {
                fs::write(&a, format!("version {round}")).unwrap();
                create(&project, "essai", std::slice::from_ref(&a))
                    .unwrap()
                    .id
            })
            .collect();
        let mut unique = ids.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), 3, "{ids:?}");

        let listed = list(&project).unwrap();
        assert_eq!(listed.len(), 3);
        for (round, id) in ids.iter().enumerate() {
            assert_eq!(
                fs::read_to_string(copy_path(&project, id, &a).unwrap()).unwrap(),
                format!("version {round}")
            );
        }
    }

    #[test]
    fn purge_removes_only_what_is_older_than_the_retention() {
        let (_dir, project) = project();
        let a = write(&project, "a.txt", b"a");
        let old = create(&project, "essai", std::slice::from_ref(&a)).unwrap();
        let recent = create(&project, "essai", std::slice::from_ref(&a)).unwrap();
        let root = backups_dir(&project);

        // Vieillit la première copie de huit jours en retouchant son manifeste.
        let manifest = manifest_path(&root, &old.id);
        let created_unix = old.created.duration_since(UNIX_EPOCH).unwrap().as_secs();
        let text = fs::read_to_string(&manifest).unwrap();
        let aged = text.replace(
            &format!("created_unix = {created_unix}"),
            &format!("created_unix = {}", created_unix - 8 * 24 * 60 * 60),
        );
        assert_ne!(text, aged);
        fs::write(&manifest, aged).unwrap();
        // Un reste de copie interrompue, et un manifeste illisible : récents, gardés.
        fs::create_dir(root.join("20200101-000000")).unwrap();
        fs::write(root.join("20200102-000000.toml"), "pas un manifeste").unwrap();

        assert_eq!(purge_expired(&project).unwrap(), 1);
        assert!(!manifest.exists());
        assert!(!root.join(&old.id).exists());
        assert!(root.join("20200101-000000").exists());
        assert!(root.join("20200102-000000.toml").exists());
        let listed = list(&project).unwrap();
        assert_eq!(listed, vec![recent.clone()]);

        // Vu de dans huit jours, tout part — restes compris.
        let later = SystemTime::now() + Duration::from_secs(8 * 24 * 60 * 60);
        assert_eq!(purge_older_than(&project, RETENTION, later).unwrap(), 2);
        assert!(list(&project).unwrap().is_empty());
        let left: Vec<_> = fs::read_dir(&root).unwrap().flatten().collect();
        assert!(left.is_empty(), "{left:?}");
        assert_eq!(fs::read(&a).unwrap(), b"a", "le projet n'est pas touché");
    }

    #[test]
    fn a_tampered_manifest_cannot_write_outside_the_project() {
        let (dir, project) = project();
        let a = write(&project, "a.txt", b"a");
        let backup = create(&project, "essai", std::slice::from_ref(&a)).unwrap();
        let manifest = manifest_path(&backups_dir(&project), &backup.id);
        let text = fs::read_to_string(&manifest).unwrap();
        fs::write(&manifest, text.replace("\"a.txt\"", "\"../évasion.txt\"")).unwrap();

        assert!(matches!(
            restore(&project, &backup.id),
            Err(Error::Invalid(_))
        ));
        assert!(list(&project).unwrap().is_empty());
        assert!(!dir.path().join("évasion.txt").exists());
    }
}
