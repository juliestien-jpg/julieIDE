//! Lecture et écriture des fichiers texte du projet.
//!
//! - [`write_atomic`] : toute écriture de l'IDE passe par là (fichier temporaire
//!   dans le même dossier, puis renommage).
//! - [`decode`] / [`encode`] : détection de l'encodage et des fins de ligne à la
//!   lecture, restitution à l'identique à l'écriture.
//!
//! Tout ce module fait des entrées-sorties **bloquantes** : jamais dans le fil de
//! l'interface.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use crate::error::{Error, Result};

const UTF8_BOM: &[u8] = &[0xEF, 0xBB, 0xBF];

/// Encodage d'un fichier texte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Encoding {
    #[default]
    Utf8,
    /// UTF-8 précédé de la marque d'ordre des octets (EF BB BF).
    Utf8Bom,
    /// Repli quand les octets ne sont pas de l'UTF-8 valide. Tout octet y a un
    /// caractère (les cinq octets non attribués deviennent les caractères de
    /// contrôle de même valeur), donc la relecture est toujours sans perte.
    Windows1252,
}

impl Encoding {
    pub fn name(self) -> &'static str {
        match self {
            Encoding::Utf8 => "UTF-8",
            Encoding::Utf8Bom => "UTF-8 avec BOM",
            Encoding::Windows1252 => "Windows-1252",
        }
    }
}

/// Fins de ligne d'un fichier texte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LineEnding {
    /// `\n`. Aussi le choix par défaut d'un fichier sans aucune fin de ligne.
    #[default]
    Lf,
    /// `\r\n`.
    CrLf,
    /// `\r` seul.
    Cr,
    /// Le fichier mélange plusieurs fins de ligne : le texte en mémoire les garde
    /// telles quelles et l'écriture n'y touche pas.
    Mixed,
}

impl LineEnding {
    pub fn as_str(self) -> Option<&'static str> {
        match self {
            LineEnding::Lf => Some("\n"),
            LineEnding::CrLf => Some("\r\n"),
            LineEnding::Cr => Some("\r"),
            LineEnding::Mixed => None,
        }
    }
}

/// Ce qu'il faut retenir d'un fichier pour le réécrire comme on l'a trouvé.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct TextFormat {
    pub encoding: Encoding,
    pub line_ending: LineEnding,
}

/// Un fichier texte décodé.
///
/// **Invariant** : si `format.line_ending` n'est pas `Mixed`, `text` ne contient
/// que des `\n` comme fins de ligne (aucun `\r`). S'il est `Mixed`, `text` est le
/// contenu exact du fichier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextFile {
    pub text: String,
    pub format: TextFormat,
}

/// Décode le contenu d'un fichier. Ne peut pas échouer, et
/// `encode(&file.text, file.format)` rend exactement les octets d'origine.
pub fn decode(bytes: Vec<u8>) -> TextFile {
    let (text, encoding) = match bytes.strip_prefix(UTF8_BOM).map(std::str::from_utf8) {
        Some(Ok(text)) => (text.to_owned(), Encoding::Utf8Bom),
        // Marque présente mais suite invalide : tout le fichier part en repli.
        Some(Err(_)) => (decode_windows_1252(&bytes), Encoding::Windows1252),
        None => match String::from_utf8(bytes) {
            Ok(text) => (text, Encoding::Utf8),
            Err(error) => (decode_windows_1252(error.as_bytes()), Encoding::Windows1252),
        },
    };

    let line_ending = detect_line_ending(&text);
    let text = match line_ending {
        LineEnding::CrLf => text.replace("\r\n", "\n"),
        LineEnding::Cr => text.replace('\r', "\n"),
        LineEnding::Lf | LineEnding::Mixed => text,
    };
    TextFile {
        text,
        format: TextFormat {
            encoding,
            line_ending,
        },
    }
}

/// Encode un texte pour l'écrire dans le format donné.
///
/// Hors `Mixed`, toutes les fins de ligne du texte (y compris un `\r\n` arrivé par
/// collage) sont ramenées à celle du format. Échoue avec [`Error::Unencodable`] si
/// un caractère n'existe pas dans l'encodage : à l'interface de proposer une
/// conversion en UTF-8, jamais de remplacement silencieux.
pub fn encode(text: &str, format: TextFormat) -> Result<Vec<u8>> {
    let converted;
    let text = match format.line_ending.as_str() {
        None => text,
        Some(ending) => {
            let has_cr = text.contains('\r');
            if ending == "\n" && !has_cr {
                text
            } else {
                let normalized = if has_cr {
                    text.replace("\r\n", "\n").replace('\r', "\n")
                } else {
                    text.to_owned()
                };
                converted = if ending == "\n" {
                    normalized
                } else {
                    normalized.replace('\n', ending)
                };
                &converted
            }
        }
    };

    match format.encoding {
        Encoding::Utf8 => Ok(text.as_bytes().to_vec()),
        Encoding::Utf8Bom => {
            let mut bytes = Vec::with_capacity(UTF8_BOM.len() + text.len());
            bytes.extend_from_slice(UTF8_BOM);
            bytes.extend_from_slice(text.as_bytes());
            Ok(bytes)
        }
        Encoding::Windows1252 => encode_windows_1252(text),
    }
}

/// Lit et décode un fichier texte.
pub fn read_text(path: impl AsRef<Path>) -> Result<TextFile> {
    let path = path.as_ref();
    let bytes = fs::read(path).map_err(|source| Error::io(path, source))?;
    Ok(decode(bytes))
}

/// Encode puis écrit atomiquement un fichier texte.
pub fn write_text(path: impl AsRef<Path>, text: &str, format: TextFormat) -> Result<()> {
    write_atomic(path, &encode(text, format)?)
}

/// Écrit `bytes` dans `path` sans jamais laisser un fichier à moitié écrit : le
/// contenu va dans un fichier temporaire du même dossier, est poussé sur le
/// disque, puis prend la place de la cible par renommage.
///
/// - Un lien symbolique est suivi : c'est sa cible qui est remplacée.
/// - Une cible en lecture seule est refusée (`PermissionDenied`), jamais forcée.
/// - En cas d'échec, la cible est intacte et le fichier temporaire est supprimé.
pub fn write_atomic(path: impl AsRef<Path>, bytes: &[u8]) -> Result<()> {
    let path = path.as_ref();
    let io_error = |source| Error::io(path, source);

    let target: PathBuf = match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => fs::canonicalize(path).map_err(io_error)?,
        _ => path.to_path_buf(),
    };
    let existing = fs::metadata(&target).ok();
    if existing
        .as_ref()
        .is_some_and(|meta| meta.permissions().readonly())
    {
        return Err(io_error(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "fichier en lecture seule",
        )));
    }

    let file_name = target.file_name().ok_or_else(|| {
        io_error(io::Error::new(
            io::ErrorKind::InvalidInput,
            "pas un nom de fichier",
        ))
    })?;
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut temp_name = std::ffi::OsString::from(".");
    temp_name.push(file_name);
    temp_name.push(format!(
        ".{}-{}.julie-tmp",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let temp = target.with_file_name(temp_name);

    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        if let Some(meta) = &existing {
            // Droits d'origine (bit exécutable sous Unix) ; sans gravité si refusé.
            let _ = fs::set_permissions(&temp, meta.permissions());
        }
        rename_with_retry(&temp, &target)
    })();

    result.map_err(|source| {
        let _ = fs::remove_file(&temp);
        io_error(source)
    })
}

/// Sous Windows, le renommage échoue tant qu'un autre programme (antivirus,
/// indexeur, autre éditeur) tient la cible ouverte : c'est le plus souvent très
/// bref, donc on réessaie un court moment avant de rendre l'erreur.
fn rename_with_retry(from: &Path, to: &Path) -> io::Result<()> {
    const ATTEMPTS: u32 = 6;
    let mut attempt = 1;
    loop {
        match fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(error) if attempt < ATTEMPTS && error.kind() == io::ErrorKind::PermissionDenied => {
                std::thread::sleep(Duration::from_millis(20 * u64::from(attempt)));
                attempt += 1;
            }
            Err(error) => return Err(error),
        }
    }
}

fn detect_line_ending(text: &str) -> LineEnding {
    let bytes = text.as_bytes();
    let (mut lf, mut crlf, mut cr) = (false, false, false);
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\n' => lf = true,
            b'\r' if bytes.get(index + 1) == Some(&b'\n') => {
                crlf = true;
                index += 1;
            }
            b'\r' => cr = true,
            _ => {}
        }
        index += 1;
    }
    match (lf, crlf, cr) {
        (_, false, false) => LineEnding::Lf,
        (false, true, false) => LineEnding::CrLf,
        (false, false, true) => LineEnding::Cr,
        _ => LineEnding::Mixed,
    }
}

/// Caractères des octets 0x80 à 0x9F en Windows-1252. Les cinq octets non
/// attribués (81, 8D, 8F, 90, 9D) valent le caractère de contrôle de même numéro.
const WINDOWS_1252_HIGH: [char; 32] = [
    '\u{20AC}', '\u{0081}', '\u{201A}', '\u{0192}', '\u{201E}', '\u{2026}', '\u{2020}', '\u{2021}',
    '\u{02C6}', '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{008D}', '\u{017D}', '\u{008F}',
    '\u{0090}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}', '\u{2013}', '\u{2014}',
    '\u{02DC}', '\u{2122}', '\u{0161}', '\u{203A}', '\u{0153}', '\u{009D}', '\u{017E}', '\u{0178}',
];

fn decode_windows_1252(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|&byte| match byte {
            0x80..=0x9F => WINDOWS_1252_HIGH[usize::from(byte - 0x80)],
            _ => char::from(byte),
        })
        .collect()
}

fn encode_windows_1252(text: &str) -> Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(text.len());
    let mut line = 0u32;
    for character in text.chars() {
        let code = u32::from(character);
        let byte = match code {
            0x00..=0x7F | 0xA0..=0xFF => code as u8,
            _ => match WINDOWS_1252_HIGH.iter().position(|&high| high == character) {
                Some(index) => 0x80 + index as u8,
                None => {
                    return Err(Error::Unencodable {
                        character,
                        line,
                        encoding: Encoding::Windows1252.name(),
                    })
                }
            },
        };
        if character == '\n' {
            line += 1;
        }
        bytes.push(byte);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(bytes: &[u8]) -> TextFile {
        let file = decode(bytes.to_vec());
        assert_eq!(
            encode(&file.text, file.format).unwrap(),
            bytes,
            "restitution différente pour {:?}",
            file.format
        );
        file
    }

    fn format(encoding: Encoding, line_ending: LineEnding) -> TextFormat {
        TextFormat {
            encoding,
            line_ending,
        }
    }

    #[test]
    fn utf8_with_and_without_bom() {
        let file = round_trip("<?php\necho 'été';\n".as_bytes());
        assert_eq!(file.format, format(Encoding::Utf8, LineEnding::Lf));
        assert_eq!(file.text, "<?php\necho 'été';\n");

        let mut bytes = UTF8_BOM.to_vec();
        bytes.extend_from_slice("été\r\n".as_bytes());
        let file = round_trip(&bytes);
        assert_eq!(file.format, format(Encoding::Utf8Bom, LineEnding::CrLf));
        assert_eq!(file.text, "été\n");
    }

    #[test]
    fn invalid_utf8_falls_back_to_windows_1252() {
        // « été » puis « 10 € » en Windows-1252.
        let file = round_trip(b"\xE9t\xE9\r\n10 \x80\r\n");
        assert_eq!(file.format, format(Encoding::Windows1252, LineEnding::CrLf));
        assert_eq!(file.text, "été\n10 €\n");
    }

    #[test]
    fn every_byte_value_survives_a_round_trip() {
        // Tous les octets, y compris les cinq non attribués de Windows-1252 :
        // même un fichier binaire ouvert par erreur est réécrit à l'identique.
        let bytes: Vec<u8> = (0..=255u8).rev().collect();
        let file = round_trip(&bytes);
        assert_eq!(file.format.encoding, Encoding::Windows1252);

        let mut with_bom = UTF8_BOM.to_vec();
        with_bom.extend_from_slice(b"\xFF\xFE");
        assert_eq!(round_trip(&with_bom).format.encoding, Encoding::Windows1252);
    }

    #[test]
    fn line_endings_are_detected_and_normalized_in_memory() {
        assert_eq!(round_trip(b"").format.line_ending, LineEnding::Lf);
        assert_eq!(round_trip(b"une ligne").format.line_ending, LineEnding::Lf);

        let file = round_trip(b"a\r\nb\r\n\r\nc");
        assert_eq!(file.format.line_ending, LineEnding::CrLf);
        assert_eq!(file.text, "a\nb\n\nc");

        let file = round_trip(b"a\rb\r");
        assert_eq!(file.format.line_ending, LineEnding::Cr);
        assert_eq!(file.text, "a\nb\n");
    }

    #[test]
    fn mixed_line_endings_are_kept_verbatim() {
        for bytes in [&b"a\r\nb\nc\r\n"[..], b"a\rb\n", b"a\r\r\nb"] {
            let file = round_trip(bytes);
            assert_eq!(file.format.line_ending, LineEnding::Mixed);
            assert_eq!(file.text.as_bytes(), bytes);
        }
    }

    #[test]
    fn encoding_forces_the_line_ending_of_the_format() {
        let crlf = format(Encoding::Utf8, LineEnding::CrLf);
        assert_eq!(encode("a\nb\r\nc\rd", crlf).unwrap(), b"a\r\nb\r\nc\r\nd");
        let lf = format(Encoding::Utf8, LineEnding::Lf);
        assert_eq!(encode("a\nb\r\nc\rd", lf).unwrap(), b"a\nb\nc\nd");
    }

    #[test]
    fn a_character_outside_windows_1252_is_refused_not_replaced() {
        let target = format(Encoding::Windows1252, LineEnding::Lf);
        assert_eq!(encode("prix : 10 €", target).unwrap(), b"prix : 10 \x80");
        let error = encode("un\ndeux →", target).unwrap_err();
        assert!(matches!(
            error,
            Error::Unencodable {
                character: '→',
                line: 1,
                ..
            }
        ));
        // U+0080 n'est pas l'octet 0x80 (qui est « € ») : refusé aussi.
        assert!(encode("\u{0080}", target).is_err());
    }

    #[test]
    fn write_atomic_creates_and_replaces_without_leaving_temporaries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fichier.php");
        write_atomic(&path, b"un").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"un");
        write_atomic(&path, b"deux").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"deux");
        write_atomic(&path, b"").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn a_failed_write_leaves_the_target_intact() {
        let dir = tempfile::tempdir().unwrap();

        // Cible en lecture seule : refusée, contenu intact, pas de temporaire.
        let path = dir.path().join("protégé.txt");
        fs::write(&path, b"original").unwrap();
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&path, permissions.clone()).unwrap();
        let error = write_atomic(&path, b"nouveau").unwrap_err();
        assert!(matches!(
            error,
            Error::Io { ref source, .. } if source.kind() == io::ErrorKind::PermissionDenied
        ));
        assert_eq!(fs::read(&path).unwrap(), b"original");
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        fs::set_permissions(&path, permissions).unwrap();

        // Dossier inexistant : erreur, rien de créé.
        let missing = dir.path().join("absent").join("fichier.txt");
        assert!(write_atomic(&missing, b"x").is_err());

        // La cible est un dossier : le renommage échoue, le temporaire est retiré.
        let folder = dir.path().join("dossier");
        fs::create_dir(&folder).unwrap();
        assert!(write_atomic(&folder, b"x").is_err());
        assert!(folder.is_dir());

        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
    }

    #[test]
    fn read_then_write_text_restores_the_exact_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ancien.php");
        let original = b"<?php\r\n// cha\xEEne \x93cit\xE9e\x94\r\n";
        fs::write(&path, original).unwrap();

        let file = read_text(&path).unwrap();
        assert_eq!(file.text, "<?php\n// chaîne “citée”\n");
        write_text(&path, &file.text, file.format).unwrap();
        assert_eq!(fs::read(&path).unwrap(), original);

        assert!(matches!(
            read_text(dir.path().join("absent.php")),
            Err(Error::Io { .. })
        ));
    }
}
