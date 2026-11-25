# Contrats entre crates

Tenu par `architecte-technique`. Ce document décrit ce que `julie-core` fournit et
les règles que toutes les crates suivent. Si quelque chose manque à `julie-core`,
on le demande à l'architecte dans son rapport : on ne modifie pas le noyau soi-même.

État : lot 1 de la v0 (24/11/2026) ; réglages en écriture, corbeille et copies de
sécurité : v1 (25/11/2026).

## 1. Qui dépend de qui

```
app (juliestorm) ──> text, diff, vcs, index, phpunit, terminal ──> julie-core
```

- Une crate métier ne dépend que de `julie-core` : ni d'egui, ni d'une autre crate
  métier. Si `vcs` a besoin d'un diff, c'est `app` qui appelle `diff` avec ce que
  `vcs` a rendu.
- `julie-core` ne connaît aucun type métier.
- Tout le métier se teste sans fenêtre.

## 2. Règles de fil d'exécution

1. **Le fil de l'interface ne fait ni accès disque, ni lancement de processus, ni
   calcul de plus de quelques millisecondes.** Tout cela passe par `Tasks::spawn`.
2. Les fonctions de `julie-core` marquées « bloquant » ou « accès disque » dans
   leur documentation (`process::Command::run*`, `file::*`, `Project::open`,
   `Project::ensure_data_dir`, `Settings::load`, tout `SettingsFiles` sauf
   `path`, `trash::move_to_trash`, tout `backup::*`, `TaskHandle::join`,
   `Bus::recv_timeout`) ne s'appellent **jamais** depuis le fil de l'interface.
3. Une tâche de fond ne touche pas à l'état de l'interface : elle **publie** ses
   résultats sur le bus. L'interface vide le bus une fois par image
   (`Bus::drain`) et distribue aux vues.
4. Chaque publication réveille l'interface (rappel installé par `app` avec
   `Bus::set_waker`, branché sur `egui::Context::request_repaint`). Une tâche n'a
   donc rien d'autre à faire pour que son résultat s'affiche.
5. **Débit** : un événement coûte une image redessinée. On publie par lots (un
   paquet de résultats de recherche, un bloc de sortie de terminal), pas par
   élément. `TaskContext::progress` peut en revanche s'appeler à chaque élément :
   il s'espace tout seul (50 ms au moins entre deux événements, le dernier toujours
   publié).
6. **Annulation coopérative** : une tâche consulte son jeton dans ses boucles
   (`ctx.check_cancelled()?`) et le passe aux appels annulables
   (`Command::run(ctx.cancel_token())`).
7. Un fil par tâche, sans plafond. Une crate qui veut du parallélisme massif
   (indexation) gère ses propres fils **à l'intérieur** d'une tâche ; `TaskContext`
   est partageable entre fils (`&TaskContext` est `Sync`).

## 3. Ce que fournit `julie-core`

Tout est réexporté à la racine pour les types courants (`julie_core::Tasks`,
`julie_core::Event`…). Les signatures exactes sont dans la documentation du code.

### 3.1 Types partagés (`julie_core`)

| Type | Rôle |
|---|---|
| `TextPosition { line, column }` | Base zéro ; colonne en **caractères** (valeurs scalaires Unicode), pas en octets. |
| `TextRange { start, end }` | `start` inclus, `end` exclu ; `TextRange::new` remet les bornes dans l'ordre. |
| `Location { path, position }` | Un endroit où sauter. Chemin **absolu**. |
| `TaskId` | Identifiant d'une tâche de fond. |
| `Domain` | `Text`, `Diff`, `Vcs`, `Index`, `PhpUnit`, `Terminal`. |

Chemins : `PathBuf` / `&Path` partout, **absolus** dans les événements et les API
publiques. Pour un chemin relatif à la racine : `Project::relative` /
`Project::absolute`. On ne canonise pas (pas de préfixe `\\?\`).

### 3.2 Bus (`julie_core::bus`, `julie_core::event`)

```rust
pub enum Event {
    ProjectOpened(PathBuf),
    FilesChangedOnDisk(Vec<PathBuf>),
    Task(TaskEvent),        // cycle de vie d'une tâche de fond
    Domain(DomainEvent),    // résultat d'une crate métier
    SettingsChanged(SettingsChanged),   // réglages lus ou relus (v1, voir 3.6)
}
```

`Event::SettingsChanged` est nouveau en v1 : un `match` exhaustif sur `Event` doit
gagner une branche (dans ce dépôt : `crates/app/src/app.rs` seulement).

- `Bus::new()`, `bus.publisher()`, `bus.drain()`, `bus.set_waker(f)`,
  `bus.recv_timeout(d)` (tests).
- `Publisher` (clonable, tout fil) : `publish(Event)`, `publish_domain(domain, charge)`.
- `DomainEvent { domain, task: Option<TaskId>, .. }` : `get::<T>() -> Option<&T>`,
  `take::<T>() -> Result<T, DomainEvent>` (possession sans copie), `type_name()`.

**Le choix : une variante `Domain` à charge utile opaque.** Chaque crate métier
définit *son* type d'événement public (une énumération, par convention
`<Crate>Event`) et le publie tel quel ; le noyau le transporte dans un
`Arc<dyn Any + Send + Sync>` sans le connaître. `app`, qui connaît toutes les
crates, le retrouve par son type. Écarté : des variantes typées dans `Event`
(obligerait le noyau à connaître les types métier, et chaque développeuse à
modifier `julie-core`) ; des canaux séparés par domaine (plusieurs files à vider,
ordre entre domaines perdu, réveil à dupliquer).

Règles :

- Une crate ne publie que sous **son** `Domain`, et de préférence par
  `ctx.publish(charge)` dans une tâche : le domaine et le `TaskId` sont alors
  posés d'office. Le `TaskId` permet à l'interface d'écarter les résultats d'une
  demande périmée.
- La charge utile est `Send + Sync + 'static`. Pas besoin de `Clone`.
- Les types d'événements font partie de l'API publique de la crate : les décrire
  dans sa documentation.

Côté `app`, une fois par image :

```rust
for event in bus.drain() {
    match event {
        Event::Task(task) => status_bar.on_task(task),
        Event::Domain(event) => match event.domain {
            Domain::Vcs => if let Some(e) = event.get::<julie_vcs::VcsEvent>() { git_view.on_event(e) },
            Domain::Index => match event.take::<julie_index::IndexEvent>() { /* possession */ },
            // …
        },
        // …
    }
}
```

### 3.3 Tâches de fond (`julie_core::task`)

```rust
let tasks = Tasks::new(bus.publisher());            // créé par app, clonable
let handle = tasks.spawn(Domain::Vcs, "vcs.status", move |ctx| {
    ctx.check_cancelled()?;                          // Err(Error::Cancelled) si annulé
    ctx.progress(done, total);                       // total == 0 : indéterminé
    ctx.publish(VcsEvent::Status(status));           // résultat
    Ok(())
});
handle.cancel();    // ou tasks.cancel(id), tasks.cancel_all()
```

- Le travail rend `TaskResult` = `Result<(), Box<dyn Error + Send + Sync>>` : `?`
  convertit l'erreur de n'importe quelle crate.
- Événements publiés : `TaskEvent { id, domain, kind, state }` avec `state` =
  `Started` (avant le retour de `spawn`), `Progress { done, total }`,
  `Finished(Done | Cancelled | Failed(String))`. Une panique devient `Failed`.
  Une erreur rendue alors que l'annulation était demandée devient `Cancelled`.
- `kind` est une clé stable `domaine.action` (`"index.scan"`, `"phpunit.run"`) :
  l'interface en tire le libellé dans le catalogue de textes.
- Lâcher le `TaskHandle` n'arrête pas la tâche. `handle.join()` est bloquant
  (tests).
- Le **résultat** d'une tâche est un événement, pas une valeur de retour. L'échec
  détaillé à montrer (sortie d'erreur de Git…) se publie aussi comme événement de
  la crate ; `Failed(String)` n'est que le message technique.

### 3.4 Processus externes (`julie_core::process`)

```rust
let command = Command::new("git")
    .args(["status", "--porcelain=v2", "-z"])
    .current_dir(project.root())
    .env("GIT_OPTIONAL_LOCKS", "0");

let output = command.run(cancel)?;                   // en bloc : status, stdout, stderr (octets)
let output = output.into_success(&command)?;         // code non nul -> Error::ProcessFailed

command.run_lines(cancel, |stream, line| { … })?;    // ligne à ligne, sans \n ni \r\n
command.run_split(cancel, 0, |stream, record| { … })?; // enregistrements séparés par NUL
command.run_chunks(cancel, |stream, chunk| { … })?;  // morceaux bruts
```

- Bloquant ; à appeler dans une tâche avec `ctx.cancel_token()`.
- Sous Windows : aucune fenêtre console (`CREATE_NO_WINDOW`).
- **Annulation** : le processus *et ses descendants* sont tués (objet « job »
  Windows) ; l'appel rend `Error::Cancelled` (le jeton est consulté toutes les 20 ms).
- Entrée standard fermée par défaut (un programme qui attend une saisie reçoit une
  fin de fichier) ; `.stdin(octets)` pour en fournir.
- Environnement hérité de l'IDE ; `.env`, `.env_remove`, `.env_clear`. Le noyau
  n'impose aucune variable : à `vcs` de poser `GIT_TERMINAL_PROMPT=0`, la langue,
  etc.
- Sorties en **octets**. Git et PHP écrivent de l'UTF-8 ; un programme console
  Windows ordinaire écrit dans la page de code OEM.
- Un code de sortie non nul n'est pas une erreur en soi (`git diff --quiet`) :
  c'est l'appelant qui décide, avec `into_success` s'il le veut.
- Programme introuvable : `Error::ProcessSpawn`.
- Arguments : passés un par un, jamais une ligne de commande à découper. Éviter de
  passer par `cmd /C` avec des guillemets dans les arguments.
- Le terminal intégré n'utilise pas ce module (pseudo-console).

### 3.5 Fichiers texte (`julie_core::file`)

```rust
let file = file::read_text(&path)?;       // TextFile { text, format: TextFormat { encoding, line_ending } }
file::write_text(&path, &text, file.format)?;   // encode + écriture atomique
file::write_atomic(&path, &bytes)?;       // toute autre écriture de l'IDE
// sans disque : file::decode(Vec<u8>) -> TextFile, file::encode(&str, TextFormat) -> Result<Vec<u8>>
```

- **Toute écriture de fichier par l'IDE passe par `write_atomic` ou `write_text`**
  (fichier temporaire dans le même dossier, `sync`, renommage). En cas d'échec la
  cible est intacte. Cible en lecture seule : refusée, jamais forcée. Lien
  symbolique : suivi.
- Encodages : `Utf8`, `Utf8Bom`, `Windows1252` (repli dès que les octets ne sont
  pas de l'UTF-8 valide). Le décodage n'échoue jamais et
  `encode(decode(octets))` rend **exactement** les octets d'origine, pour
  n'importe quel contenu.
- **Fins de ligne — invariant du texte en mémoire** : si le fichier est homogène
  (`Lf`, `CrLf` ou `Cr`), `text` ne contient que des `\n` et l'écriture remet la
  fin de ligne d'origine partout (un `\r\n` arrivé par collage est ramené au
  format du fichier). Si le fichier mélange les fins de ligne (`Mixed`), `text`
  est le contenu exact et l'écriture n'y touche pas. Un fichier sans fin de ligne
  est `Lf`.
- Un caractère absent de Windows-1252 fait échouer l'encodage
  (`Error::Unencodable { character, line, .. }`) : l'interface propose de
  convertir le fichier en UTF-8 ; jamais de remplacement silencieux.
- Le `TextFormat` accompagne le tampon de sa lecture à son enregistrement :
  `text` le garde, `diff` et `index` n'en ont pas besoin.

### 3.6 Projet et réglages (`julie_core::project`, `julie_core::settings`)

- `Project::open(racine)?` (accès disque), `root()`, `name()`, `relative(&path)`,
  `absolute(rel)`, `data_dir()` (= `<racine>/.juliestorm`, peut ne pas exister),
  `ensure_data_dir()?` (le crée, avec un `.gitignore` qui ignore tout son
  contenu), `settings_path()`. `Project` se clone et se passe par valeur aux tâches.
- Dans `.juliestorm/`, chaque crate écrit dans **son** sous-dossier
  (`.juliestorm/index/…`) et par `write_atomic`. Le noyau y tient `settings.toml`
  et `annulations/` (3.8).
- `project::ALWAYS_EXCLUDED` = `[".git", ".juliestorm"]` : noms que la recherche
  et l'indexation excluent **toujours**, à toute profondeur, quoi que dise
  `[index] excluded`. `julie-index` les applique en plus de la liste réglée ;
  l'interface les affiche en tête de la liste, grisés, non retirables, et ne les
  écrit jamais dans le fichier.

#### Lire

- Deux portées (`settings::Scope`) : `User` = `%APPDATA%\JulieStorm\settings.toml`
  (« Tous mes projets »), `Project` = `<racine>\.juliestorm\settings.toml` (« Ce
  projet », local à la machine : décision n° 6). Priorité **clé par clé** :
  défaut < utilisateur < projet. Les tables fusionnent en profondeur
  (`[phpunit.env]` fusionne par nom de variable) ; toute autre valeur, tableaux
  compris, est remplacée en bloc (`extra_args`, `excluded`).
- Une table TOML par crate, du nom de la crate sans préfixe (`[vcs]`, `[index]`,
  `[phpunit]`, `[terminal]`, `[text]`, `[diff]`, `[app]`). La crate déclare sa
  structure et la lit :

```rust
#[derive(serde::Deserialize, Default)]
#[serde(default)]
pub struct PhpUnitSettings { pub php_path: Option<PathBuf>, pub phpunit_path: Option<PathBuf> }

let settings: PhpUnitSettings = settings.section("phpunit")?;   // section absente -> Default
```

- `Settings` (immuable, se clone, se passe aux tâches) :

| Appel | Rend |
|---|---|
| `section::<T>("phpunit")?` | la section fusionnée ; clé de mauvais type : `Error::Settings` pour **cette** section, les autres restent lisibles |
| `get(&["phpunit", "php_path"])` | la valeur effective (`Option<&settings::Value>`, le `toml::Value` réexporté), `None` si aucun fichier ne la règle |
| `get_in(Scope::Project, chemin)` | la valeur écrite **à cette portée**, sans héritage |
| `origin(chemin)` | `Origin::Project`, `User` ou `Default` (aucun fichier : valeur par défaut **ou** détection automatique, la crate propriétaire sait laquelle) |
| `inherited(Scope::Project)` | les réglages tels qu'ils seraient si le fichier de cette portée était vide (pour le projet : ceux de l'utilisateur ; pour l'utilisateur : rien) |

  Un chemin de clé commence par la section : `["phpunit", "env", "APP_ENV"]`.
- `Settings::load(Some(&project))?` et `Settings::load_files` restent (lecture
  stricte : fichier mal formé = `Error::Settings`). Marque d'ordre des octets et
  CRLF acceptés ; un fichier qui n'est pas de l'UTF-8 est refusé.

Ce que la fenêtre de réglages fait d'un champ (page PHPUnit, portée « Ce projet ») :

```rust
let chemin = ["phpunit", "php_path"];
let effectif = PhpUnitSettings::from_settings(&settings)?;            // valeur effective
let origine  = settings.origin(&chemin);                              // « réglé pour ce projet »…
let regle_ici = settings.get_in(Scope::Project, &chemin).is_some();   // bouton « Revenir à la valeur héritée »
let herite = PhpUnitSettings::from_settings(&settings.inherited(Scope::Project))?;  // texte grisé
// Portée « Tous mes projets » : `settings.origin(&chemin) == Origin::Project`
// => « ce projet utilise sa propre valeur ».
```

#### Écrire (`settings::SettingsFiles`)

```rust
let files = SettingsFiles::locate(Some(&project));        // fichiers réels (APPDATA + projet)
// Tests : SettingsFiles::new(Some(dossier_jetable.join("settings.toml")), Some(project))
//         — jamais `locate` dans un test, qui écrirait dans le vrai %APPDATA%.

// « Appliquer » : toutes les clés modifiées d'une portée en UNE écriture atomique.
files.apply(Scope::Project, &[
    SettingsEdit::set(&["phpunit", "phpunit_path"], SettingValue::Path("outils\\phpunit.phar".into())),
    SettingsEdit::set(&["phpunit", "extra_args"], SettingValue::List(vec!["--group".into(), "très lent".into()])),
    SettingsEdit::set(&["phpunit", "env", "APP_ENV"], SettingValue::Text("test".into())),
    SettingsEdit::unset(&["phpunit", "php_path"]),          // « Revenir à la valeur héritée »
])?;
files.set(Scope::User, &["app", "theme"], SettingValue::Text("clair".into()))?;   // une clé
files.unset(Scope::Project, &["vcs", "update_method"])?;
let chemin = files.ensure_file(Scope::Project)?;           // « Ouvrir le fichier de réglages »
watcher.check_now();                                       // effet immédiat (voir plus bas)
```

- `SettingValue` : `Text`, `Path`, `Bool`, `Integer`, `List(Vec<String>)`. Un
  `Path` s'écrit en chaîne littérale (`'C:\Program Files (x86)\…'`, barres non
  doublées) sauf s'il contient une apostrophe. Un tableau de plus de 80 colonnes
  s'écrit un élément par ligne.
- **Seules les clés demandées changent.** Commentaires (en tête, en fin de ligne,
  au-dessus d'une clé), ordre, espaces, clés et tables inconnues, BOM, fins de
  ligne et absence de fin de ligne finale sont rendus octet pour octet. Une valeur
  déjà égale n'est pas réécrite (la façon dont elle a été tapée est gardée) ; sans
  changement réel, le fichier n'est pas touché du tout.
- Le fichier est **relu juste avant** l'écriture : les clés qu'une autre instance
  vient d'écrire sont conservées (il reste une fenêtre de quelques millisecondes
  entre lecture et renommage ; pas de verrou).
- Retirer une clé la **supprime** (pas de valeur vide). Le commentaire placé
  au-dessus d'elle reste à sa place. Une table devenue vide est retirée, **sauf si
  elle porte un commentaire** (on ne sait pas distinguer une table créée par l'IDE
  d'une table tapée à la main : le commentaire tranche). Retirer une clé absente,
  ou d'un fichier absent, ne fait rien et ne crée rien.
- Fichier absent : créé à la première clé écrite, avec son dossier
  (`%APPDATA%\JulieStorm\`, ou `.juliestorm/` et son `.gitignore`), une ligne de
  commentaire en tête, UTF-8 sans BOM, CRLF. Le texte de cette ligne est dans
  `settings/files.rs` (`USER_HEADER`, `PROJECT_HEADER`) : à relire par `redacteur`.
- **Refus** (rien n'est écrit, le fichier ne change pas d'un octet) :
  - fichier mal formé ou pas en UTF-8 : `Error::SettingsUnreadable { path, line,
    message }` (`line` à partir de un) ;
  - fichier en lecture seule, dossier sans droit : `Error::Io { path, .. }` ;
  - chemin de clé de moins de deux éléments, clé parente qui n'est pas une table,
    valeur écrite là où se trouve une table, pas de projet ouvert pour
    `Scope::Project` : `Error::Invalid`.
- `files.load()?` : lecture stricte. `files.load_lenient()` : `(Settings,
  Vec<SettingsProblem>)`, un fichier illisible compte pour vide et est signalé.

#### Relire à chaud (`settings::SettingsWatcher`, `Event::SettingsChanged`)

```rust
// app, à l'ouverture d'un projet (et une fois au démarrage, sans projet) :
let watcher = SettingsWatcher::start(SettingsFiles::locate(Some(&project)), bus.publisher());
// Lâcher `watcher` arrête la surveillance : un par projet ouvert.

// app, en vidant le bus :
Event::SettingsChanged(change) => {
    self.settings = change.settings.clone();                // derniers réglages valides
    for problem in &change.problems { /* notification « Réglages illisibles : <fichier>, ligne n : <raison> » */ }
    if change.concerns("phpunit") { /* nouveau PhpUnitSettings pour la prochaine exécution */ }
    if change.concerns("vcs")     { /* nouveau VcsSettings, rouvrir le dépôt */ }
    if change.concerns("index")   { /* relancer l'indexation */ }
    if change.concerns("terminal"){ /* les prochains onglets */ }
    if change.concerns("app")     { /* thème */ }
}
```

- Un fil dédié regarde taille et date des deux fichiers toutes les 500 ms
  (`POLL_INTERVAL`) et ne relit que ce qui a changé : une modification à la main,
  dans l'IDE ou ailleurs, est prise en compte en moins d'une seconde.
  `watcher.check_now()` (non bloquant) force une relecture immédiate après
  `apply`. `watcher.current()` rend les derniers réglages publiés sans accès disque.
- `SettingsChanged { settings, changed_sections, problems }` :
  - le **premier** événement est toujours publié (même sans aucun fichier) : c'est
    lui qui donne les réglages initiaux ; `changed_sections` y liste toutes les
    sections présentes ;
  - ensuite, un événement seulement si les réglages effectifs ou la liste des
    problèmes changent (un commentaire ajouté ne publie rien) ;
  - `changed_sections` : sections dont le contenu **effectif** a changé, triées ;
  - `problems: Vec<SettingsProblem { scope, path, line: Option<u32>, message }>` :
    fichiers actuellement illisibles. Tant qu'un fichier est illisible, **sa
    dernière version valide reste en vigueur** (vide s'il n'a jamais été lisible)
    et `apply` refuse d'y écrire. `problems` vide après un problème = réparé.
- Chaque service reçoit ses nouveaux réglages **de `app`** (les crates métier ne
  s'abonnent pas au bus). Une exécution, une session de terminal ou une commande
  Git **en cours** n'est pas touchée : la suivante prend les nouveaux réglages.

#### Clés connues

Le noyau ne valide pas le contenu des sections ; cette table est l'inventaire.

| Section | Clés | Propriétaire |
|---|---|---|
| `[app]` | `theme` = `"sombre"` (défaut) ou `"clair"` ; portée utilisateur | dev-interface |
| `[vcs]` | `git_path`, `update_method` = `"fusion"` ou `"rebase"` (absent : demander ; portée projet), `protected_branches` (défaut `["main", "master"]`, motif `préfixe*`) | dev-git (§ 8) |
| `[index]` | `excluded` (motifs en plus d'`ALWAYS_EXCLUDED`, qui s'applique toujours), `libraries`, `max_file_size`, `threads` | dev-indexation-recherche (§ 11) |
| `[phpunit]` | `php_path`, `phpunit_path`, `configuration`, `extra_args` (tableau, remplacé en bloc par la portée la plus forte), `timeout_seconds` ; `[phpunit.env]` (table nom = valeur, fusionnée par nom) | dev-phpunit (§ 12) |
| `[terminal]` | `default_profile`, `git_bash_path`, `cmd_path`, `scrollback` | dev-terminal (§ 13) |

### 3.7 Corbeille (`julie_core::trash`)

```rust
// Dans une tâche de fond. Chemins absolus ; fichiers ou dossiers (avec leur contenu).
match julie_core::trash::move_to_trash(&paths) {
    Ok(()) => { /* tout est dans la corbeille Windows, restaurable */ }
    Err(Error::TrashUnavailable { remaining }) => {
        // Lecteur réseau, corbeille désactivée… RIEN n'a été supprimé définitivement.
        // Seconde confirmation « Suppression définitive : ne pourra pas être annulée »
        // (bouton par défaut : Annuler), puis app supprime elle-même `remaining`
        // (std::fs, dans une tâche).
    }
    Err(Error::TrashFailed { remaining, code }) => {
        // Fichier verrouillé par un autre programme, droits… : message, `remaining`
        // est toujours sur le disque. `code` (HRESULT) va au journal.
    }
    Err(other) => { /* Error::Io : un chemin n'existe pas — rien n'a été supprimé */ }
}
```

- Sert à « Supprimer… » (arbre du projet, non suivis des changements) et aux
  fichiers ajoutés que « Annuler les modifications » retire quand la case
  « Supprimer aussi les fichiers ajoutés (vers la corbeille) » est cochée.
- **Jamais de suppression définitive** : avant chaque élément, Windows annonce
  s'il ira à la corbeille ; sinon l'élément est refusé et rien n'est supprimé.
- Aucune fenêtre de Windows (ni confirmation, ni progression, ni erreur) : les
  confirmations sont celles de l'IDE, **avant** l'appel.
- Tous les chemins sont vérifiés avant la première suppression (inexistant :
  `Error::Io` ; relatif : `Error::Invalid`). L'opération s'arrête au premier
  échec : ce qui précède est déjà à la corbeille, `remaining` dit ce qui reste.
- Bloquant, sans annulation ni progression. Hors Windows : `TrashUnavailable`.
- Réalisation : `IFileOperation` du shell, dans un fil à lui. `SHFileOperationW`
  est écartée : sans fenêtre, elle supprime définitivement ce que la corbeille ne
  peut pas recevoir.
- Essais réels (ils remplissent la vraie corbeille, donc à la demande
  seulement) : `cargo test -p julie-core --lib -- --ignored corbeille`. Essayés
  le 25/11/2026 : fichier et dossier (espaces, accents) ; fichier verrouillé ;
  partage réseau sans corbeille (`\\localhost\C$`). **Non essayé** : élément plus
  gros que la corbeille, corbeille désactivée dans les propriétés de Windows.

### 3.8 Copies de sécurité (`julie_core::backup`)

Décision n° 3 de la v1 : avant « Annuler les modifications » ou une bascule forcée,
les fichiers qui vont être écrasés sont copiés ; « Rétablir » les remet.

```
<racine>/.juliestorm/annulations/
    20261125-143012.toml               manifeste (écrit en dernier : il valide la copie)
    20261125-143012/src/Vue/Liste.php  les fichiers, à leur chemin relatif au projet
```

```rust
// Dans la tâche qui va détruire, AVANT de détruire :
let copie = julie_core::backup::create(&project, "vcs.discard", &fichiers)?;   // Err => on renonce
git_restore(&fichiers)?;
ctx.publish(VcsEvent::Discarded { backup: Some(copie.id.clone()), .. });       // nom indicatif

// « Rétablir » (notification, pendant la session) :
let rapport = julie_core::backup::restore(&project, &copie.id)?;
// rapport.restored, rapport.unchanged, rapport.failed (chemin, raison), rapport.safety_backup

// À l'ouverture du projet :
julie_core::backup::purge_expired(&project)?;            // copies de plus de 7 jours
// Lister :
for copie in julie_core::backup::list(&project)? { /* id, created, reason, files */ }
```

- `create(&project, raison, &fichiers)` : chemins **absolus de fichiers du
  projet**. Copie octet pour octet (aucun décodage), écriture atomique. Un fichier
  absent est noté `present: false` sans erreur. Dossier, chemin hors du projet,
  `..`, liste vide : `Error::Invalid`, rien n'est créé. En cas d'échec en cours de
  route, la copie partielle est retirée. **Règle : pas de copie, pas d'opération
  destructive.**
- `raison` : clé stable (`"vcs.discard"`, `"vcs.checkout"`…), jamais un texte ;
  l'interface en tire le libellé.
- `Backup { id, created, reason, files: Vec<BackupFile { path, present, size }> }`.
  `id` = horodatage **local** `AAAAMMJJ-HHMMSS` (`-2`, `-3`… dans la même
  seconde) ; c'est aussi le nom du dossier. `backup::copy_path(&project, id,
  &fichier)` rend le chemin de la copie d'un fichier (pour l'ouvrir ou la comparer).
- `restore(&project, id)` : remet chaque fichier copié dans l'état de la copie.
  Un fichier déjà identique n'est pas touché ; un fichier `present: false` non
  plus (**rétablir ne supprime jamais**). Avant d'écraser, les fichiers concernés
  sont eux-mêmes copiés (raison `backup::RESTORE_REASON`, identifiant dans
  `safety_backup`) : rétablir se défait aussi. Un fichier en lecture seule ou
  verrouillé va dans `failed`, les autres sont rétablis. La copie reste en place.
- `list(&project)` : de la plus récente à la plus ancienne ; une copie sans
  manifeste lisible est ignorée. `purge_expired` / `purge_older_than(&project,
  durée, maintenant)` : retire ce qui a plus de `backup::RETENTION` (7 jours),
  restes de copies interrompues compris ; rend le nombre de copies retirées.
- Chemins avec espaces, parenthèses, apostrophes et accents : essayés.
- Un manifeste retouché à la main ne peut pas faire écrire hors du projet (chemin
  relatif sans `..` exigé, sinon la copie est ignorée).
- `.juliestorm/` étant ignoré par Git, les copies n'apparaissent pas dans les
  changements locaux ; `ALWAYS_EXCLUDED` les tient hors de la recherche.
- Pas d'événement : ce sont des fonctions synchrones, à appeler dans la tâche de
  la crate qui détruit, laquelle publie l'identifiant de la copie dans **son**
  événement.

## 4. Comment une crate métier s'expose à `app`

1. **Du calcul pur quand c'est possible.** Fonctions sans effet de bord, testables
   seules (`julie_diff::diff_lines(a, b)`, l'analyse d'une ligne TeamCity, le
   découpage d'une sortie `--porcelain`). `app` les appelle directement si c'est
   court, dans une tâche sinon.
2. **Un service pour ce qui tourne en fond.** Une structure construite par `app`
   avec ce dont elle a besoin (`Tasks`, `Project`, ses réglages), dont les
   méthodes rendent la main tout de suite en lançant une tâche, et rendent le
   `TaskHandle` (ou le `TaskId`) pour l'annulation et le rapprochement :

```rust
pub struct Vcs { tasks: Tasks, project: Project, settings: VcsSettings }
impl Vcs {
    pub fn new(tasks: Tasks, project: Project, settings: VcsSettings) -> Self;
    pub fn refresh_status(&self) -> TaskHandle;      // publie VcsEvent::Status(..)
}
```

3. **Un type d'événement public** (`VcsEvent`) qui porte les résultats, données
   complètes comprises : l'interface ne rappelle pas la crate pour « aller chercher »
   le résultat.
4. Derrière chaque méthode de service, une fonction **synchrone** publique ou
   `pub(crate)` qui fait le travail (`status(&project, &cancel) -> Result<Status>`) :
   c'est elle que les tests appellent, sans bus.
5. Aucun texte destiné à l'utilisatrice dans une crate métier : des données, des
   variantes d'énumération, des clés. Les libellés sont dans le catalogue.

### Un exemple par crate

Les noms de types ci-dessous sont indicatifs : chaque propriétaire fixe les siens.

**`julie-text`** — presque tout est synchrone (le tampon vit dans le fil de
l'interface ; une frappe ne passe jamais par le bus). Le bus sert au chargement, à
l'enregistrement et à la coloration d'un gros fichier.

```rust
pub fn load(tasks: &Tasks, path: PathBuf) -> TaskHandle {
    tasks.spawn(Domain::Text, "text.load", move |ctx| {
        let file = julie_core::file::read_text(&path)?;
        ctx.publish(TextEvent::Loaded { path, text: file.text, format: file.format });
        Ok(())
    })
}
// app : event.take::<TextEvent>() pour prendre le texte sans le copier.
```

**`julie-diff`** — bibliothèque pure, sans fichiers ni bus. `app` l'appelle
directement, ou dans une tâche qu'elle lance elle-même pour un gros fichier :

```rust
let handle = tasks.spawn(Domain::Diff, "diff.compute", move |ctx| {
    let hunks = julie_diff::diff_lines(&left, &right, ctx.cancel_token())?;
    ctx.publish(DiffReady { request, hunks });       // type défini par app ou par diff
    Ok(())
});
```

**`julie-vcs`**

```rust
tasks.spawn(Domain::Vcs, "vcs.status", move |ctx| {
    let command = git(&settings).args(["status", "--porcelain=v2", "-z"]).current_dir(project.root());
    let output = command.run(ctx.cancel_token())?.into_success(&command)?;
    ctx.publish(VcsEvent::Status(parse_status(&output.stdout)?));
    Ok(())
})
```

**`julie-index`**

```rust
tasks.spawn(Domain::Index, "index.scan", move |ctx| {
    let mut batch = Vec::new();
    for (done, entry) in walk(project.root()).enumerate() {
        ctx.check_cancelled()?;
        batch.push(entry?);
        ctx.progress(done as u64, 0);                // espacé d'office
        if batch.len() == 2_000 { ctx.publish(IndexEvent::Files(std::mem::take(&mut batch))); }
    }
    ctx.publish(IndexEvent::Files(batch));
    ctx.publish(IndexEvent::ScanFinished);
    Ok(())
})
// Recherche au fil de la frappe : annuler la tâche précédente, lancer la nouvelle,
// et côté app ignorer tout DomainEvent dont `task` n'est pas la tâche courante.
```

**`julie-phpunit`**

```rust
tasks.spawn(Domain::PhpUnit, "phpunit.run", move |ctx| {
    let command = Command::new(&settings.php_path).arg(&phpunit).arg("--teamcity").current_dir(project.root());
    let mut parser = TeamCityParser::new();
    let status = command.run_lines(ctx.cancel_token(), |stream, line| {
        if let Some(update) = parser.feed(stream, line) { ctx.publish(TestEvent::Update(update)); }
    })?;
    ctx.publish(TestEvent::RunFinished { exit_code: status.code });
    Ok(())
})
```

**`julie-terminal`** — n'utilise pas `process::Command` (ConPTY). Chaque session a
son fil de lecture, lancé comme une tâche pour hériter de l'annulation et du
réveil ; la sortie est publiée par blocs, l'entrée clavier passe par un canal que
la crate possède.

```rust
tasks.spawn(Domain::Terminal, "terminal.session", move |ctx| {
    while let Some(bytes) = pty.read_block()? {       // lecture bloquante de la pseudo-console
        ctx.publish(TerminalEvent::Output { session, bytes });
    }
    ctx.publish(TerminalEvent::Exited { session });
    Ok(())
})
// Fermer l'onglet : fermer la pseudo-console (ce qui débloque la lecture) puis handle.cancel().
```

## 5. Erreurs

- Le noyau a un type : `julie_core::Error` (et `julie_core::Result<T>`), variantes
  `Io { path, source }`, `Cancelled`, `ProcessSpawn`, `ProcessFailed { program,
  code, stderr }`, `Unencodable`, `Settings`, `SettingsUnreadable { path, line,
  message }`, `TrashUnavailable { remaining }`, `TrashFailed { remaining, code }`,
  `Invalid(String)`. Des variantes peuvent s'ajouter : pas de `match` exhaustif
  sur `julie_core::Error` hors du noyau.
- **Chaque crate métier a sa propre énumération d'erreurs**, écrite avec
  `thiserror`, qui enveloppe celle du noyau :

```rust
#[derive(Debug, thiserror::Error)]
pub enum VcsError {
    #[error(transparent)]
    Core(#[from] julie_core::Error),
    #[error("pas un dépôt Git : {0}")]
    NotARepository(PathBuf),
}
```

- Pas de crate d'erreurs séparée, pas d'`anyhow` dans les crates métier : l'appelant
  doit pouvoir distinguer les cas.
- `Cancelled` n'est pas un échec : on ne l'affiche pas, on ne le journalise pas
  comme une erreur.
- Le texte de `Display` sert aux journaux. Ce que l'utilisatrice lit vient du
  catalogue de textes, choisi par `app` d'après la variante.
- Pas de `unwrap`/`expect` sur ce qui vient du disque, d'un processus ou de
  l'utilisatrice. Une panique dans une tâche est rattrapée (`Failed`), mais c'est
  un bogue.
- Dans le doute sur des données (fusion, encodage, écriture), on **refuse** avec
  une erreur plutôt que de deviner.

## 6. Dépendances

Versions fixées dans `[workspace.dependencies]` du `Cargo.toml` racine ; une crate
écrit `nom.workspace = true` (plus ses `features` si besoin).

| Crate | Version | Usage |
|---|---|---|
| `thiserror` | 2 | énumérations d'erreurs de toutes les crates |
| `serde` (avec `derive`) | 1 | sections de réglages, caches sur disque |
| `toml` | 0.9 | réglages (dans le noyau ; les crates n'en ont en principe pas besoin : `julie_core::settings::Value` est réexporté) |
| `toml_edit` | 0.25 | écriture des réglages sans perdre commentaires ni mise en forme ; **noyau seul**, déclarée dans `crates/core/Cargo.toml` |
| `tempfile` | 3 | **tests uniquement** (`[dev-dependencies]`) : dossiers et dépôts jetables |
| `windows-sys` | 0.61 | appels Win32 ; chaque crate active ses `features` |
| `eframe` | 0.37 | `app` seulement |

- Une dépendance utilisée par **une seule** crate (`ropey`, `tree-sitter`,
  `ignore`, `regex`, `memchr`…) se déclare, avec sa version, dans le `Cargo.toml`
  de cette crate : pas besoin de toucher au fichier racine.
- Dès qu'une deuxième crate en a besoin, le signaler à l'architecte, qui la monte
  dans `[workspace.dependencies]`.
- Seules les crates de crates.io se téléchargent sans demander (voir `CLAUDE.md`).

## 7. Prévu, pas encore livré

- Surveillance du disque (`Event::FilesChangedOnDisk` existe, rien ne le publie
  encore) et détection d'un fichier modifié hors de l'IDE avant enregistrement.
  Seuls les deux fichiers de réglages sont surveillés (3.6).
- Liste des fichiers ouverts dans le modèle de projet (pour l'instant dans `app`).
- Réglages : verrou entre deux instances de l'IDE qui écrivent le même fichier
  au même instant.
- Corbeille : progression et annulation d'une très grosse suppression ; « annuler
  la suppression » depuis l'IDE (aujourd'hui : restaurer depuis la corbeille).
- Copies de sécurité : plafond de taille, copie de dossiers entiers.
- Journal d'exécution (fichier de log) commun.
- UTF-16 et détection des fichiers binaires à la lecture.
- Plafond du nombre de tâches simultanées, si la mesure en montre le besoin.


## 8. `julie-vcs` (dev-git, lot 2 de la v0 ; lot C de la v1)

Détail et exemples en tête de `crates/vcs/src/lib.rs`. Ne dépend que de
`julie-core` ; ne connaît pas `julie-diff` (c'est `app` qui passe les versions
d'un conflit au moteur de fusion et rend le résultat).

### Service (fil de l'interface)

`Vcs::new(tasks, &settings)` (`settings: VcsSettings` = section `[vcs]`,
`git_path: Option<PathBuf>`). Chaque méthode lance une tâche `Domain::Vcs` et rend
un `TaskHandle` ; le résultat arrive en `VcsEvent` (`event.get::<julie_vcs::VcsEvent>()`).

| Méthode | Clé de tâche | Événements publiés |
|---|---|---|
| `open(dir)` | `vcs.open` | `Opened(Repository)` puis `Status`, ou `NoRepository { dir }` |
| `refresh_status(&repo)` | `vcs.status` | `Status(Status)` |
| `notify_disk_change(&repo, &paths)` | `vcs.status` | `Status`, 300 ms après le dernier changement pertinent ; une tâche à la fois (`None` si déjà prévue) |
| `load_diff(&repo, DiffRequest)` | `vcs.diff` | `Diff(DiffSides)` |
| `commit(&repo, CommitRequest)` | `vcs.commit` | `Committed(CommitOutcome)` puis `Status` |
| `load_log(&repo, LogQuery)` | `vcs.log` | `Log(LogPage)` |
| `load_commit(&repo, id)` | `vcs.commit_details` | `CommitDetails(CommitDetails)` |
| `load_conflict(&repo, path)` | `vcs.conflict` | `Conflict(ConflictVersions)` |
| `resolve_conflict(&repo, path, octets, attendu)` | `vcs.resolve` | `Resolved { path }` puis `Status` |
| `accept_side(&repo, paths, ConflictSide)` | `vcs.accept_side` | `SideAccepted { side, files }` puis `Status` |
| `continue_operation(&repo, message)` | `vcs.continue` | `OperationContinued(ContinueOutcome)` puis `Status` |
| `abort_operation(&repo)` | `vcs.abort` | `OperationAborted(OperationKind)` puis `Status` |

- Toute commande Git lancée publie `VcsEvent::Command(CommandRecord)` deux fois
  (début, `result: None` ; fin, avec durée, code, sortie tronquée à 16 Kio), même
  `id` : c'est l'onglet Console. `vcs.command_log()` en rend une copie (500
  dernières) sans accès disque.
- Échec : `VcsEvent::Failed(Failure { action, error })` **et** tâche `Failed`.
  La sortie de Git (crochets compris) est dans `VcsError::GitFailed { stdout,
  stderr, .. }`. Annulation : rien d'autre que `Cancelled`.
- Après toute écriture, réussie ou non, un `Status` frais suit. Les écritures sont
  sérialisées entre elles.

### Types principaux

- `Repository` : `work_tree()` (écrit comme le dossier donné à `open`, non
  canonisé), `git_dir()`, `to_repo_path(&Path)`, `to_absolute(&str)`,
  `operation()`.
- `Status { head: Head, upstream: Option<Upstream>, operation: Option<Operation>, files }` ;
  `Head::Branch { name, commit: Option<_> }` (`None` : dépôt sans commit) ou
  `Head::Detached { commit }` ; `Upstream { name, ahead_behind }` ;
  `Operation { kind: Merge | Rebase | CherryPick | Revert, message (MERGE_MSG), their_commit, rebase_branch }`.
- `FileStatus { path, repo_path, original_path, original_repo_path, kind, staged, unstaged, conflict, submodule }` ;
  `kind` : `Modified | Added | Deleted | Renamed | Copied | TypeChanged | Untracked | Conflicted` ;
  `group()` : `Conflicted | Changed | Untracked` ; `conflict` : `ConflictKind`
  (`BothModified`, `BothAdded`, `BothDeleted`, `AddedByUs`, `AddedByThem`,
  `DeletedByUs`, `DeletedByThem`, avec `involves_deletion()`).
- `Content` : `Missing | Blob { bytes, mode } | TooLarge { size } | Submodule { commit } | Directory` ;
  `is_binary()` (NUL dans les 8 premiers Kio), `size()`, `into_bytes()`.
  Les versions tirées de Git passent par `git cat-file --filters` : octets tels
  que Git les écrirait sur le disque (`autocrlf` compris). Au-delà de 20 Mio :
  `TooLarge`.
- `Revision` : `Absent | Head | Index | Stage(Base | Ours | Theirs) | Commit(id) | WorkTree`.
  `DiffRequest::local_change(&FileStatus)` (HEAD de l'ancien chemin → disque),
  `DiffRequest::commit_file(id, &CommitFile)` (premier parent → commit).
- `CommitRequest { paths, message, amend }` ; `CommitOutcome { commit, subject, amended, output }`.
- `LogQuery { revision: Option<String>, skip, limit }` (500 par défaut,
  `next_page()`) ; `LogPage { commits, has_more }` ; `CommitSummary { id, short_id,
  parents, author, committer, subject, refs }` (`is_merge()`) ; `Person { name,
  email, time (Unix), offset_minutes }` ; `RefLabel { kind, name, current }`.
- `CommitDetails { summary, message, files: Vec<CommitFile> }` ; `CommitFile {
  path, repo_path, original_*, change, parent }`.
- `ConflictVersions { path, repo_path, kind, operation, base, ours, theirs, work_tree }`.
  Pendant un rebase, `ours` est la branche sur laquelle on rejoue (sens de Git).
- `ContinueOutcome { kind, finished, head, conflicts, output }` (`finished:
  false` : le rebase s'est arrêté sur un nouveau conflit, ce n'est pas un échec).

### Commandes retenues

- **État** : `git status --porcelain=v2 -z --branch --untracked-files=all` ;
  opération lue dans le dossier Git (`rebase-merge`, `rebase-apply`, `MERGE_HEAD`,
  `CHERRY_PICK_HEAD`, `REVERT_HEAD`).
- **Commit** (confirme la proposition de `git.md`, précisée) :
  `git add -- <non suivis choisis>` puis
  `git commit -F - --only [--amend] -- <choisis>` (message sur l'entrée standard ;
  un renommage choisi entraîne son ancien chemin). Si le commit échoue, l'ajout
  des non suivis est défait (`git rm --cached --quiet -- …`) : index inchangé.
  `--amend` sans fichier = message seul (l'index n'est pas validé). Pendant une
  fusion ou un rebase, le commit de fichiers choisis est refusé
  (`OperationInProgress`) : Git interdit le commit partiel, on passe par
  `continue_operation` (`git commit --no-edit` ou `-F -` pour une fusion,
  `git <op> --continue` sinon).
- **Conflits** : étages par `git ls-files -s -z`, contenu par `git cat-file
  --filters --path=<chemin> <oid>` ; enregistrer = écriture atomique
  (`file::write_atomic`) puis `git add --all -- <chemin>` ; garder un côté =
  `git checkout --ours|--theirs -- …` + `git add`, ou `git rm` si ce côté avait
  supprimé le fichier ; abandon = `git <op> --abort`.
- **Journal** : `git log -z --date=raw --decorate=full --format=… --skip --max-count
  --end-of-options <rév> --` ; fichiers : `git diff-tree -r -z -M --name-status`
  contre le premier parent (`--root` pour un commit racine).

### Environnement de chaque appel

`--no-pager -c color.ui=false -c core.quotepath=false -c log.showSignature=false` ;
`LC_ALL=C`, `GIT_TERMINAL_PROMPT=0`, `GIT_EDITOR=:`, `GIT_SEQUENCE_EDITOR=:`,
`GIT_MERGE_AUTOEDIT=no`, `GIT_LITERAL_PATHSPECS=1` (un nom avec `[` n'est pas un
motif), `GIT_OPTIONAL_LOCKS=0` pour les lectures ; `GIT_DIR`, `GIT_WORK_TREE`,
`GIT_INDEX_FILE` hérités retirés. Crochets et signature jamais contournés.
Destructif seulement sur demande nommée : `accept_side` (écrase ou supprime le
fichier) et `abort_operation`.

### v1, lot C : branches, distant, opérations, menus des fichiers (25/11/2026)

Exemples complets en tête de `crates/vcs/src/lib.rs`. Rien de changé pour les
appels v0 ; `LogQuery` gagne des champs (construire avec `..LogQuery::default()`),
`VcsSettings` aussi (`..VcsSettings::default()`), `VcsEvent` et `Action` gagnent
des variantes (garder un `_ =>`).

**Réglages `[vcs]`** : `git_path`, `protected_branches: Vec<String>` (défaut
`["main", "master"]`, motif `préfixe*` admis), `update_method: Option<IntegrationMethod>`
(`"fusion"` / `"rebase"` ; `None` : demander). `Vcs::new` les garde ;
`Vcs::with_settings` les remplace.

**Règle commune des écritures v1** : résultat, puis `Status` **et** `Branches`
relus, réussite ou non ; `Waiting { action }` publié si une autre écriture tient
le verrou. Réseau : `Progress(RemoteProgress { phase, from_remote, percent, done,
total })` espacé de 100 ms, et `ctx.progress`. Aucune saisie d'identifiants
(`GIT_TERMINAL_PROMPT=0`) : besoin d'identifiants ou refus →
`VcsError::AuthenticationFailed { stderr }` (l'interface renvoie au terminal / au
gestionnaire d'identifiants). Annulation : Git et ses descendants tués.

| Méthode | Clé | Événement | Erreurs propres |
|---|---|---|---|
| `load_branches(&repo)` | `vcs.branches` | `Branches(BranchList)` | — |
| `check_branch_name(&repo, nom, ignore)` | `vcs.branch_name` | `BranchNameChecked { name, problem }` | — |
| `create_branch(&repo, CreateBranchRequest)` | `vcs.branch_create` | `BranchCreated(CreateBranchOutcome)` | `InvalidBranchName`, `WouldOverwrite`, `NoCommits` |
| `rename_branch(&repo, old, new)` | `vcs.branch_rename` | `BranchRenamed { old, new, upstream }` | `InvalidBranchName`, `BranchNotFound` |
| `delete_branch(&repo, nom)` | `vcs.branch_delete` | `BranchDeleted { name, tip, upstream }` | `BranchNotMerged { commits, subjects, tip }`, `CurrentBranch` |
| `force_delete_branch` (**destructif**) | `vcs.branch_delete` | `BranchDeleted` | `CurrentBranch` |
| `restore_branch(&repo, nom, tip)` | `vcs.branch_restore` | `BranchRestored` | `InvalidBranchName` |
| `delete_remote_branch(&repo, distant, nom)` (**destructif**) | `vcs.remote_branch_delete` | `RemoteBranchDeleted` | `ProtectedBranch`, `PushRejected` |
| `switch_branch(&repo, SwitchRequest)` | `vcs.switch` | `Switched(SwitchOutcome)` | `WouldOverwrite(OverwriteConflict)`, `OperationInProgress`, `BranchNotFound` |
| `switch_discarding_changes(&repo, cible, conflit)` (**destructif**) | `vcs.switch` | `Switched` | — |
| `check_detached_commits(&repo)` | `vcs.detached_commits` | `DetachedCommits { count }` | — |
| `fetch(&repo, FetchRequest { remote, prune })` | `vcs.fetch` | `Fetched(FetchOutcome)` | `AuthenticationFailed`, `GitFailed` |
| `fast_forward_branch(&repo, nom)` | `vcs.branch_update` | `BranchFastForwarded { branch, fetch }` | `Diverged`, `NoUpstream`, `UpstreamGone`, `CurrentBranch` |
| `update_project(&repo, UpdateRequest { method, local_changes })` | `vcs.update` | `Updated(OperationOutcome)` | `NoUpstream` (sans aucune commande réseau ni fusion), `UpstreamGone`, `DetachedHead` |
| `pull(&repo, PullRequest { remote, branch, mode, local_changes })` | `vcs.pull` | `Pulled(OperationOutcome)` | idem |
| `load_push_preview(&repo, branche, cible)` | `vcs.push_preview` | `PushPreview { target, new_branch, outgoing, remote_tip, remote_only, protected, sets_upstream }` | `DetachedHead` |
| `push(&repo, PushRequest { target, set_upstream, follow_tags, force })` | `vcs.push` | `Pushed { kind, pushed, upstream_set }` | `PushRejected { rejection, output }`, `ProtectedBranch`, `GitFailed` (crochet `pre-push`) |
| `merge(&repo, MergeRequest)` | `vcs.merge` | `OperationRan(OperationOutcome)` | `WouldOverwrite`, `OperationInProgress` |
| `rebase(&repo, RebaseRequest { onto, branch, local_changes })` | `vcs.rebase` | `OperationRan` | idem |
| `cherry_pick(&repo, commits, lc)` / `revert_commits` | `vcs.cherry_pick` / `vcs.revert` | `OperationRan` | `MergeCommitNotSupported` |
| `skip_operation(&repo)` (**abandonne le commit**) | `vcs.skip` | `OperationSkipped(ContinueOutcome)` | `NoOperationInProgress` |
| `load_reset_preview(&repo, cible)` | `vcs.reset_preview` | `ResetPreview { commits_leaving, unreachable, pushed, local_changes }` | — |
| `reset(&repo, cible, Soft/Mixed)` / `reset_hard(&repo, cible, stash_first)` (**destructif**) | `vcs.reset` | `ResetDone` | — |
| `load_stashes(&repo)` / `restore_auto_stash(&repo, commit)` | `vcs.stashes` / `vcs.stash_restore` | `Stashes(Vec<StashEntry>)` / `StashRestored(AutoStash)` | — |
| `stage` / `unstage(&repo, chemins)` | `vcs.stage` / `vcs.unstage` | `Staged` / `Unstaged` | `IgnoredPaths` |
| `discard_changes(&repo, chemins)` (**destructif**) | `vcs.discard` | `Discarded { restored, unstaged_added }` | `Conflicted` |
| `move_path(&repo, de, vers)` | `vcs.move` | `Moved` | `NotTracked`, `DestinationExists` |
| `add_to_gitignore(&repo, IgnoreRule)` | `vcs.ignore` | `IgnoreAdded { file, line, already_present }` | `AlreadyTracked` |

**Types** :

- `BranchList { current, detached_at, unborn, local, remote, remotes, recent }` ;
  `Branch { name, full_ref, kind, remote, short_name, current, tracking:
  Option<Tracking { upstream, ahead, behind, gone }>, last_commit: BranchCommit
  { id, subject, author, time } }`. `origin/HEAD` exclu ; `recent` : 5 au plus,
  tirées du reflog de HEAD (`checkout: moving from A to B`), courante exclue.
  `branch_tree(&noms) -> Vec<BranchNode>` (pur) : dossiers par préfixe, un dossier
  d'un seul élément replié en une ligne (`bugfix/crash`).
- `BranchNameProblem` : `Empty`, `Invalid { suggestion }` (espaces → tirets),
  `AlreadyExists`, `SameNameDifferentCase`, `FolderOfExisting`, `InsideExisting`,
  `FolderCaseMismatch` (casse Windows). `suggest_branch_name`,
  `branch_name_collision` purs.
- `SwitchTarget::{Branch, Remote("origin/x"), Detach(rév)}` ; `LocalChanges::{Keep,
  Stash}` ; `SwitchOutcome { branch, head, created, stash, hook_output,
  behind_upstream, output }` (`hook_output` : bascule faite, `post-checkout` en
  échec). `OverwriteConflict { tracked, untracked, output }`.
- `AutoStash { message, commit, restore: StashRestore }` ; `StashRestore::{Restored,
  Conflicts(chemins), Pending, Kept, Failed(texte)}` : l'entrée n'est supprimée que
  si `Restored`. Message : `JulieStorm : avant bascule de C vers X — AAAA-MM-JJ
  HH:MM UTC` (`AUTO_STASH_PREFIX`) ; aussi « avant mise à jour de », « avant rebase
  de … sur … », « avant fusion de … dans … », « avant cherry-pick sur », « avant
  revert sur », « avant réinitialisation de ».
- `OperationOutcome { kind, finished, up_to_date, fast_forward, before, head,
  received, changed_files, conflicts, stash, fetch, output }` : `finished: false` =
  arrêtée sur un conflit (liste des conflits et dialogue de fusion existants, puis
  `continue_operation` / `skip_operation` / `abort_operation`).
  `Operation` (dans `Status`) gagne `step: Option<(n, total)>` (« commit 2 sur 3 »)
  et `onto`.
- `PushTarget { branch, remote, remote_branch }` (`push_target` : l'amont, sinon
  `origin` et le même nom) ; `PushForce::{No, WithLease { expected }}` (jamais de
  `--force` nu) ; `PushRejection::{NonFastForward { known_behind }, StaleLease,
  RemoteRejected { reason }, Other { reason }}` ; `PushedKind::{New, FastForward,
  Forced, UpToDate}`.
- `FetchOutcome { updates: Vec<RefUpdate { kind, old, new, local_ref }>, output }`,
  `updated()`, `pruned()`, `is_empty()`.
- `FileStatus::index_state() -> IndexState::{NotStaged, Staged, PartiallyStaged,
  Conflicted}` (marque « indexé » / « en partie indexé »).
- `LogQuery` gagne `all_refs`, `author`, `text` (chaînes fixes, sans casse),
  `since`, `until` (Unix), `paths`, `follow`, `order` (`LogOrder::Date` par défaut,
  enfants avant parents) ; `LogQuery::file_history(chemin)`. Avec `paths`, les
  parents rendus sont ceux de l'historique simplifié.
- **Graphe** (pur, `GraphBuilder::push_page(&commits) -> Vec<GraphRow>`) :
  `GraphRow { commit, lane, color, edges: Vec<GraphEdge { from, to, color, half:
  Top|Bottom }>, width }`. Garder le même `GraphBuilder` pour toutes les pages d'un
  journal : les pages donnent les mêmes lignes qu'un calcul d'un tenant. Premier
  parent dans le couloir de l'enfant ; aucun couloir ne se décale ; couleur stable
  (hachage du nom de branche posé sur le commit qui ouvre le couloir, sinon du
  hash), à réduire par `color % palette.len()`. 10 000 commits : bien moins de 1 s
  en debug.

**Commandes retenues** : branches `git for-each-ref --format=…%00…` (avance/retard
par `%(upstream:track,nobracket)`, jamais de réseau), récentes `git log -g
--grep-reflog="checkout: moving from "`, nom `git check-ref-format --branch`,
créer `git switch -c` / `git branch [--track]` (dépôt sans commit : `git
symbolic-ref HEAD refs/heads/<nom>`, l'index n'est pas touché), basculer `git switch
--no-guess` / `-c X --track origin/X` / `--detach`, supprimer `git branch -d|-D`
(commits orphelins comptés par `git rev-list --count --stdin`, branches passées
sur l'entrée standard), fetch `git fetch --porcelain --progress [--prune]
(--all|<distant>)`, mise à jour `fetch <distant>` puis `git merge --no-edit
<amont>` ou `git rebase <amont>`, pull `git pull --progress
--no-rebase|--rebase|--ff-only`, avance d'une autre branche `git fetch <distant>
<réf>:refs/heads/<b>`, push `git push --porcelain --progress [-u] [--follow-tags]
[--force-with-lease=refs/heads/<n>:<attendu>] <distant> refs/heads/<b>:refs/heads/<n>`,
suppression distante `git push --porcelain <distant> --delete refs/heads/<n>`,
fusion `git merge --ff|--no-ff|--ff-only [--squash] [--no-commit] (-m|--no-edit)`,
`git rebase <onto> [<branche>]`, `git cherry-pick`, `git revert --no-edit`, `git
<op> --skip`, `git reset -q --soft|--mixed|--hard <commit>`, mise de côté `git
stash push --include-untracked -m` / `git stash apply <commit>` / `git stash drop
stash@{n}` (cherchée par son commit) — **sans** `GIT_LITERAL_PATHSPECS` pour ces
deux commandes (sous chemins littéraux, `stash push -u` laisse les non suivis sur
le disque). Menus : `git add --`, `git restore --staged --` (dépôt sans commit :
`git rm --cached -r --force --ignore-unmatch`), `git restore --source=HEAD --staged
--worktree --` pour annuler, `git mv --` (casse seule : nom intermédiaire),
`.gitignore` par `file::write_text` (fin de ligne et encodage du fichier gardés).

**Copie de sécurité** (`decisions-v1.md` n° 3) : pas dans `julie-vcs`. Avant
`switch_discarding_changes`, `reset_hard`, `discard_changes`, `accept_side`,
l'appelant copie les fichiers qu'il a fait confirmer (`OverwriteConflict::files()`,
`ResetPreview::local_changes`, sa sélection) dans `.juliestorm/annulations/`.

## 9. `julie-diff` (dev-fusion-diff, lot 2 de la v0)

Bibliothèque **pure** : ni Git, ni fichiers, ni bus, ni service. Elle reçoit des
`&str`, elle rend des structures ; `app` l'appelle directement, ou dans une tâche
qu'elle lance elle-même pour un gros fichier (`Domain::Diff`). Spécification :
`docs/specs/fusion.md` (les 26 cas sont des tests) et `docs/specs/git.md` (diff
côte à côte). Documentation complète et exemples exécutés : tête de
`crates/diff/src/lib.rs`.

### Conventions

- **Lignes** numérotées à partir de zéro selon `TextLines` : fin de ligne = `\n`
  ou `\r\n` ; un `\r` isolé reste dans la ligne (comme Git) ; texte vide = zéro
  ligne. Plages `début..fin`, fin exclue ; plage vide = point d'insertion.
  `TextLines::new(texte)` donne `len()`, `content(i)`, `ending(i)`,
  `byte_range(i)`, `has_bom()`, `has_final_newline()`, `style()`.
- **Entrées = contenu exact des fichiers**, fins de ligne comprises. Pour la
  fusion, partir des octets de `julie_vcs::ConflictVersions` (`String::from_utf8` ;
  un échec = fichier non UTF-8 = pas de dialogue) et **non** du texte normalisé en
  `\n` de `julie_core::file::read_text`, sinon les fins de ligne d'origine sont
  perdues. Une BOM en tête est acceptée, ignorée dans les comparaisons, et remise
  en tête du résultat si notre version en avait une : `result_text().into_bytes()`
  se passe tel quel à `julie_vcs::resolve_conflict`.
- Colonnes : `InlineSpan::range` est en **octets** dans le contenu de la ligne
  (frontières de caractères garanties) ; `MergeSession::edit` prend des
  `julie_core::TextPosition` (colonnes en caractères).
- Erreurs : `DiffError` (en pratique l'annulation seule, `is_cancelled()`) pour
  les calculs ; `SessionError` pour un geste refusé (`UnknownBlock`,
  `SideNotApplicable`, `AlreadyAccepted`, `AlreadyIgnored`, `NotAConflict`,
  `OutOfRange`) — un refus ne change rien et ne laisse rien dans l'historique.

### Diff à deux côtés

```rust
let diff = julie_diff::diff_lines(&left, &right, &DiffOptions::default(), cancel)?;
```

| Type | Contenu |
|---|---|
| `DiffOptions { whitespace, line_endings_matter, final_newline_matters, inline }` | Défaut : espaces exacts, style de fin de ligne et saut final **hors blocs** (rapportés à part), affinage calculé. `DiffOptions::exact()` : à l'octet près. `Whitespace` : `Exact`, `IgnoreTrailing`, `IgnoreAmount`, `IgnoreAll`. |
| `Diff { hunks, left_line_count, right_line_count, left_style, right_style, final_newline, identical }` | `only_line_endings_differ()` (« Seules les fins de ligne diffèrent »), `final_newline` : `Same`, `Added`, `Removed` (à signaler en bas), `changed_line_counts()`. Compteur « n différences » = `hunks.len()`. |
| `Hunk { kind, left, right, inline }` | `HunkKind` : `Added` (gauche vide), `Removed` (droite vide), `Modified`. Blocs dans l'ordre, séparés par au moins une ligne identique. |
| `InlineDiff { left, right }`, `InlineSpan { line, range }` | Portions modifiées au mot, au caractère entre deux mots proches ; `None` au-delà de 400 lignes ou 100 Ko par côté. |
| `diff.alignment() -> Vec<AlignedRange { left, right, hunk }>` | Couvre les deux textes sans trou : connecteurs et défilement synchronisé. |
| `diff.left_to_right(ligne)`, `right_to_left(ligne) -> LineMatch` | `Same(ligne)` ou `Changed { hunk, other }`. |

### Fusion à trois volets

```rust
// base, notre version (gauche), leur version (droite) ; pas d'ancêtre : base = ""
let mut session = MergeSession::new(&base, &ours, &theirs, cancel)?;   // rien n'est appliqué
session.apply_non_conflicting(ApplyScope::All);        // ou Left, Right ; rend le nombre appliqué
session.accept(id, Side::Left)?;                       // flèche ; sur un conflit dont l'autre côté
                                                       // est accepté : « ajouter à la suite »
session.ignore(id, Side::Right)?;                      // croix
session.accept_both(id, Side::Left)?;                  // les deux, une seule étape annulable
session.edit(start, end, "texte")?;                    // frappe (TextPosition)
session.replace_lines(3..4, &["ligne"])?;              // frappe par lignes entières
session.undo(); session.redo();                        // frappe et clics, compteur compris
let MergeCounts { changes, conflicts } = session.counts();
if session.is_all_resolved() { /* « Tout est traité » */ }
let texte = session.result_text();                     // à écrire tel quel
```

| Méthode | Sert à |
|---|---|
| `blocks() -> Vec<Block>`, `block(id)` | `Block { id, kind, base, ours, theirs, result, left, right, resolved }` ; `kind` : `Left`, `Right`, `Identical`, `Conflict` ; `left`/`right` : `SideState` (`NotApplicable`, `Pending`, `Accepted`, `Ignored`) ; `result` = place **actuelle** au centre. `can_accept(side)`, `accept_appends(side)`, `can_ignore(side)` disent quelles commandes dessiner. `shape(side) -> HunkKind` donne la couleur (ajout, suppression, modification). `id` = indice stable. |
| `alignment() -> Vec<AlignedSegment { left, result, right, block }>` | Correspondance des trois volets, sans trou. |
| `result_lines()`, `result_line(i)`, `result_line_count()` | Le centre, ligne par ligne, sans fins de ligne. |
| `counts()`, `is_all_resolved()`, `is_touched()` | Compteur ; état « tout est traité » ; « quelque chose a été fait » (confirmation à la fermeture). |
| `next_pending(courant)`, `previous_pending(courant)`, `block_at_result_line(ligne)` | F7 / Maj+F7, bloc courant. |
| `line_endings() -> LineEndingReport { base, ours, theirs, result, differ }` | `differ` : afficher le bandeau. |
| `bom() -> BomReport { base, ours, theirs, result, differ }` | Présence de la BOM dans chaque version (`bool`) et dans le résultat (`result` = notre version, toujours). `differ` : les versions ne sont pas d'accord, **afficher un bandeau** comme pour les fins de ligne (une version vide — pas d'ancêtre, fichier vidé — n'a pas d'avis). `report.their_change_is_dropped()` : leur côté a ajouté ou retiré la BOM et le résultat ne le suit pas (cas où le bandeau doit le dire nettement ; Git y verrait un conflit sur la première ligne). Ne dépend pas des gestes : à lire une fois à l'ouverture. |
| `merge_last_two_steps()` | Regrouper une rafale de frappe en une seule étape annulable. |
| `ours()`, `theirs()`, `base()`, `chunks()` | Les textes des volets et le découpage d'origine. |

Aussi : `merge_chunks(&base, &ours, &theirs, cancel) -> Vec<Chunk>` (découpage
classé seul : `ChunkKind` = `Unchanged`, `Left`, `Right`, `Identical`, `Conflict`)
et `merge(base, ours, theirs, cancel) -> MergeOutcome { text, conflicts }`
(fusion d'un trait, sans interface).

### Décisions à connaître

- **Accepter les deux côtés d'un conflit** (question ouverte de la
  spécification) : **ordre des clics**. Le premier côté accepté remplace le texte
  du bloc ; le second s'ajoute à la fin du bloc.
- **Frappe et blocs** : taper dans un bloc ne le résout pas ; le bloc grandit ou
  rétrécit avec le texte. Une ligne insérée pile entre un bloc et sa voisine reste
  hors du bloc, sauf si la frappe est partie d'une ligne du bloc (Entrée en fin
  de dernière ligne).
- **Saut de ligne final** : il ne s'édite pas, il suit la règle de la
  spécification (dernière ligne venue d'une version : comme dans cette version ;
  dernière ligne tapée : comme notre version). `edit` accepte la position
  `(nombre de lignes, 0)` pour « la fin du texte ».
- **Changements qui peuvent glisser : jugés là où Git les range** (écart M1 de
  la recette de la v0). Une méthode ajoutée après une accolade, une ligne vide
  retirée d'une série, une ligne banale commune à la méthode modifiée et à la
  méthode ajoutée : ces lectures ne diffèrent que par un glissement, et Git
  range toujours le changement à la même place (le plus bas possible, ou en
  face d'un changement de l'autre fichier). Le moteur range de même son diff et
  tous les alignements aussi courts avant de décider du contact entre les deux
  côtés (`crates/diff/src/canonical.rs`). Les blocs sont donc placés comme dans
  `git diff`.
- **Plus prudent que Git, exprès** — ce qui reste un conflit alors que
  `git merge-file` fusionne :
  - un changement qui a **plusieurs lectures qu'aucun glissement ne ramène
    l'une à l'autre** (ligne déplacée, motif répété décalé d'un cran entre deux
    changements) est testé sur toute sa zone d'ambiguïté ; deux changements
    voisins du même côté peuvent alors former un seul bloc ;
  - une **ligne retirée d'une série de lignes identiques** quand l'autre côté
    touche à une ligne de la série (laquelle a été retirée ?) ;
  - un côté dont le diff remue **plus de 400 lignes banales** (lignes qui
    existent aussi de l'autre côté) : le diff de Git n'y est plus forcément le
    plus court, on en reste à l'analyse stricte.
  Jamais l'inverse sur le corpus ni sur les tirages aléatoires contre Git
  (`tests/git_compare.rs`, version longue `--ignored`). L'analyse compte les
  alignements sur le fichier entier (`crates/diff/src/band.rs`) ; au-delà de
  son budget (très gros fichier et très gros diff de lignes banales), elle se
  rabat sur une analyse par fenêtres, approximation documentée en tête de
  `merge.rs`.
- **Fins de ligne** : `\r\n` et `\n` sont la même ligne (cas 15) ; Git, lui, y
  voit un changement. Écarts listés dans `tests/fixtures/diff/merge/cases.txt`.
- **BOM** : elle n'entre pas dans les comparaisons et le résultat porte celle
  de notre version, quoi qu'ait fait l'autre côté (Git, lui, y voit une première
  ligne modifiée). Ce n'est plus silencieux : `session.bom().differ` demande un
  bandeau (écart M7 de la recette).
- Les limites de taille (20 Mio, 200 000 lignes), la détection du binaire et du
  non-UTF-8 sont à la charge de l'appelant (`julie-vcs` les donne) : la crate ne
  refuse rien.

### Prévu, pas livré

Modules `julie_diff::resolve` (résolution automatique des conflits simples) et
`julie_diff::patch` (diff unifié) : emplacements réservés, vides. Options
« ignorer les espaces » dans la fusion. Affinage au mot dans les blocs de fusion
(appeler `diff_lines` sur les textes d'un bloc en attendant).

## 10. `julie-text` et le composant éditeur (dev-editeur, lot 3 de la v0)

Détail en tête de `crates/text/src/lib.rs` et de `crates/app/src/editor/mod.rs`.
`julie-text` ne dépend que de `julie-core` (plus `ropey`, `unicode-segmentation`,
`tree-sitter` 0.25 et les grammaires PHP 0.24, HTML/CSS/JS/TS 0.23).

### Tampon et document (fil de l'interface, synchrone)

- `Buffer` : corde `ropey`, positions en **caractères** ; lignes séparées par
  `\n`, `\r\n` ou `\r` (`line(n)` sans la fin de ligne). `apply(remplacements,
  EditKind, avant, après, instant)` ; `undo`/`redo` rendent les sélections à
  rétablir ; `state_id`/`is_modified`/`mark_saved` ; `version()` augmente à
  chaque modification ; ancres : `map_offset_since(position, version, Bias)`
  (journal des 4096 dernières modifications).
- `Selections` / `Selection { anchor, head, goal_column }` : curseurs multiples
  (modèle complet ; la v0 n'en montre qu'un, Alt+clic en ajoute).
- `Document` = tampon + sélections + chemin + `TextFormat` + `DiskStamp` +
  langage + coloration. Commandes : `insert_text` (frappe groupée), `paste`,
  `insert_newline` (indentation reprise, cran de plus après `{[(`), `backspace`,
  `delete_forward`, `delete_word_*`, `indent`/`outdent`, `delete_lines` (Ctrl+Y),
  `duplicate`, `move_lines`, `toggle_line_comment`, `copy_text`/`cut` (ligne
  entière sans sélection), `undo`/`redo`, `move_cursor(Motion, étendre)`,
  `set_cursor`, `select_word_at`, `select_line`, `select_all`,
  `matching_bracket`. Lecture seule : `set_read_only` ou fichier Windows-1252 à
  octet non attribué (`ReadOnlyReason`).
- Groupement de l'annulation : frappe continue = une étape ; close par un
  déplacement, un saut de ligne, un collage, une pause > 1 s, un enregistrement.

### Fichiers (accès disque : tâches `Domain::Text`)

| Fonction | Clé de tâche | Événements (`TextEvent`) |
|---|---|---|
| `spawn_load(tasks, chemin)` | `text.load` | `Loaded(LoadedFile)` ou `LoadFailed { path, error }` |
| `spawn_save(tasks, document.save_request()?)` | `text.save` | `Saved(SaveOutcome)` → `document.mark_saved(&outcome)`, ou `SaveFailed { path, error }` |
| `spawn_check_disk(tasks, [(chemin, stamp)])` | `text.check_disk` | `DiskChecked(Vec<(PathBuf, DiskState)>)` (`Unchanged`, `Changed`, `Deleted`) |

- Versions synchrones pour les tests et les tâches d'autres crates :
  `load_file`, `Document::open`, `SaveRequest::execute`, `Document::save`,
  `check_disk`.
- Refus à l'ouverture : binaire (NUL dans les 8 premiers Kio,
  `TextError::Binary`), plus de 50 Mio (`TooLarge`). Au-delà de 5 Mio : texte
  brut, sans coloration.
- Encodage et fins de ligne : ceux de `julie_core::file`, rendus à l'identique ;
  en fichier `Mixed`, les lignes nouvelles prennent la fin de ligne dominante.
  Refus d'encodage : `TextError::Core(Error::Unencodable { .. })`, le document
  reste modifié ; `Document::set_encoding(Utf8)` pour convertir.
- Recharger après un changement sur le disque : `Document::reload(LoadedFile)`
  (historique vidé, curseur sur la même ligne). Fichier supprimé :
  `mark_deleted_on_disk()`.

### Coloration

- `Language::from_path` (`.php .phtml .inc`, `.html .htm`, `.css`,
  `.js .mjs .cjs .jsx`, `.ts`, `.tsx`). Zones imbriquées : HTML hors des
  balises PHP (un seul document HTML sur toutes les zones), JS des `<script>`
  (sauf `type` non JavaScript, par ex. gabarit AngularJS), CSS des `<style>`.
- `Document::enable_syntax(Some(waker))` lance **un fil par document** (pas une
  tâche du bus : l'analyse incrémentale garde ses arbres d'une frappe à l'autre,
  et le résultat ne concerne que ce document). Le fil appelle `waker` (→
  `request_repaint`) quand un résultat est prêt ; `poll_syntax()` le prend
  (l'éditeur le fait à chaque image). En attendant, les plages sont recalées sur
  les lignes ajoutées ou retirées.
- Résultat : `highlights(ligne) -> &[HighlightSpan { start, end, kind }]`, en
  **octets** du contenu de la ligne ; `HighlightKind` : `Keyword`, `String`,
  `Number`, `Comment`, `Variable`, `Function`, `Type`, `Constant`, `Tag`,
  `Attribute`, `Property`, `Selector`, `Operator`, `Punctuation`, `Escape`. Les
  couleurs sont dans le thème de l'interface.
- **Arbres partagés** (pour `dev-indexation-recherche`) :
  `document.syntax()?.snapshot() -> &SyntaxSnapshot { version, language, tree,
  injections: Vec<InjectedTree { language, tree, ranges }> }` ; cloner un
  `tree_sitter::Tree` est en temps constant. `julie-text` réexporte
  `tree_sitter`. **À trancher par l'architecte** : pour que `julie-index` lise ces
  arbres sans dépendre de `julie-text`, monter `tree-sitter = "0.25"` (et les
  grammaires) dans `[workspace.dependencies]` ; `app` passera alors le
  `SyntaxSnapshot` (ou ses `Tree`) à l'index pour les fichiers ouverts.
- `Highlighter` (synchrone) et `highlight_text(langage, texte)` pour colorer
  sans fil.

### Composant éditeur (`crates/app/src/editor`)

- `EditorView::new(id)` garde l'état d'affichage ; `show(ui, &mut document,
  &decorations)` ou `show_in_rect(ui, rect, …)` rend `EditorOutput { response,
  changed, selection_changed, scrolled, intents, layout }`.
- Entrées de l'hôte : `Decorations { lines: Vec<LineDecoration { lines, tone }>,
  ranges: Vec<RangeDecoration { range: TextRange, tone }>, gutter:
  Vec<GutterMark { line, shape, tone, id, tooltip }>, separators: Vec<(ligne,
  Tone)> }`, chaque liste triée par ligne ; `Tone` : `Added`, `Removed`,
  `Modified`, `Conflict`, `SearchMatch`, `CurrentSearchMatch`, `Occurrence`,
  `Warning`, `Error`, `Info` ; `EditorOptions { read_only, … }` ;
  `EditorTheme` (palette claire/sombre par défaut, remplaçable par
  `view.theme`).
- Sorties : `EditorIntent::GutterMarkClicked { line, id }`.
- Volets synchronisés : `split_panes(rect, n, largeur_gouttière) -> (volets,
  gouttières)`, `sync_scroll(&mut [vues], &[sorties], |de, vers, ligne_haut|
  …)`, `set_top_line`, `set_scroll_y`, `center_line`, et
  `EditorLayout { text_rect, gutter_rect, line_height, scroll_y, visible_lines,
  line_count }` avec `line_top(l)`, `line_y_range(l)`, `lines_y_range(a..b)`,
  `line_at(y)` pour dessiner les connecteurs.
- Raccourcis pris par l'éditeur (focus) : ceux de la ligne « Éditeur » de
  `docs/specs/raccourcis.md` sauf Ctrl+F4, Alt+Gauche/Droite et Ctrl+Tab
  (onglets : à l'hôte), plus Ctrl+D, Ctrl+/, Alt+Maj+Haut/Bas. Ctrl+S est à
  l'hôte.

## 11. `julie-index` (dev-indexation-recherche, lot 3 de la v0 ; symboles et navigation : v1, 25/11/2026)

Détail et exemple en tête de `crates/index/src/lib.rs`. Ne dépend que de
`julie-core` (plus `ignore`, `globset`, `regex`, `memchr`, et `tree-sitter` avec
les grammaires PHP, JavaScript, TypeScript de `[workspace.dependencies]`, les
mêmes que `julie-text` ; pas de dépendance vers `julie-text` : chaque crate
analyse elle-même). Spécifications : `docs/specs/recherche.md`,
`docs/specs/navigation.md`. Seule écriture disque : le cache des symboles,
`<projet>/.juliestorm/index/symbols.bin` (jamais hors du projet ouvert).

### Service (fil de l'interface)

`IndexService::new(tasks, project, settings)` (`settings: IndexSettings` =
section `[index]` : `excluded` (défaut `/target` ; un nom seul vaut à toute
profondeur, `/nom` à la racine seulement ; `project::ALWAYS_EXCLUDED` = `.git`,
`.juliestorm` s'applique **toujours** en plus, même avec une liste vide), `libraries` (défaut
`vendor`, `node_modules`), `max_file_size` (10 Mio), `threads` (4)). Chaque
méthode lance une tâche `Domain::Index` et rend la main ; résultats en
`IndexEvent` (`event.get::<julie_index::IndexEvent>()`, ou `take` pour les gros
paquets).

| Méthode | Clé de tâche | Événements publiés |
|---|---|---|
| `scan()` | `index.scan` | progression de tâche `Progress { done: fichiers, total: 0 }`, puis `ScanFinished(ScanSummary { files, library_files, elapsed })`. Annule un `scan` en cours. |
| `update(chemins)` | `index.update` | `Updated(UpdateSummary { added, removed, files, full_rescan })`. Chemins absolus de `Event::FilesChangedOnDisk` (fichiers ou dossiers, créés, modifiés, supprimés ; un renommage = les deux chemins). Attend la fin d'un `scan`. |
| `find_files(requête, récents)` | `index.find_files` | `Files(FileResults)`, un seul. Annule la demande précédente. |
| `search_text(TextQuery, UnsavedFiles) -> Result<TaskHandle>` | `index.search_text` | `TextMatches(Vec<FileTextMatches>)` en flux (premier paquet dès la première occurrence, puis toutes les 50 ms au plus), puis `TextSearchFinished(TextSearchSummary)`. Annule la recherche précédente ; attend la fin d'un `scan`. |
| `cancel_search()` | — | annule la recherche en cours (fenêtre fermée). |
| `preview(chemin, ligne, contexte, Option<TextQuery>, Option<Arc<str>>) -> Result<TaskHandle>` | `index.preview` | `Preview(Preview)`. Annule l'aperçu précédent. |
| `set_settings(IndexSettings) -> Option<TaskHandle>` | `index.scan` | Nouveaux réglages (`Event::SettingsChanged` qui `concerns("index")`) : si différents, relance tout (fichiers puis symboles) et rend la tâche ; sinon `None`. Le service n'est pas reconstruit. |
| (fin de `scan`) / `build_symbols()` | `index.symbols` | progression `Progress { done, total }` (fichiers PHP/JS/TS), puis `SymbolsReady(SymbolBuildSummary { files, parsed, from_cache, skipped_too_large, skipped_binary, unreadable, declarations, cache_loaded, cache_written, elapsed })`. Lancée d'office à la fin de chaque `scan`. |
| `update(chemins)` (suite) | `index.update` | après `Updated` : `SymbolsUpdated { files }` si des fichiers de code ont été réindexés. |
| `set_unsaved(chemin, Option<Arc<str>>)` | `index.unsaved` | `SymbolsUpdated { files: 1 }`. `Some` : contenu d'un onglet modifié (après une pause de frappe) ; `None` : enregistré, fermé, annulé (retour au disque). Sert à l'index, à la navigation et aux usages. |
| `navigate(NavigationRequest { path, text, position })` | `index.navigate` | `Navigation(NavigationResult)`, un seul. Annule la demande précédente. |
| `find_usages(UsagesQuery { symbol, include_libraries })` | `index.usages` | `Usages(Vec<Usage>)` en flux (premier fichier trouvé tout de suite, puis toutes les 50 ms au plus), puis `UsagesFinished(UsagesSummary)`. Annule la recherche précédente ; annulée : pas de `UsagesFinished`. |
| `cancel_usages()` | — | onglet Usages ou liste surgissante fermés. |
| `save_symbol_cache()` | `index.symbols_cache` | aucun ; à la fermeture du projet (la construction l'écrit déjà). |

- `is_ready()` : l'indexation initiale est finie (sinon afficher « Indexation… »).
  `with_index(|index| …)` peut bloquer quelques millisecondes : tests et tâches.
- **Annulation** : chaque nouvelle demande `find_files`, `search_text` ou
  `preview` annule la précédente du même genre. Côté `app`, ignorer tout
  `DomainEvent` dont `task` n'est pas la demande courante (critère 12).
  Une recherche annulée ne publie pas `TextSearchFinished`.
- **Erreurs rendues tout de suite, sans tâche** : `IndexError::InvalidPattern {
  message }` (regex invalide : message sous le champ), `InvalidMask { mask,
  message }`, `OutsideProject(dossier)`. Le reste passe par `Failed` de la tâche.
- Requête de moins de 2 caractères, délai de 300 ms après la frappe : à la
  charge de l'interface (la crate cherche ce qu'on lui donne).

### Aller au fichier

- `FileResults { query, position: Option<TextPosition>, matches: Vec<FileMatch>,
  total, index_complete }` : 100 résultats au plus (`DEFAULT_LIMIT`),
  « et n autres » = `total - matches.len()` ; `index_complete == false` pendant
  l'indexation (résultats partiels). `position` : suffixe `:12` ou `:12:7`,
  **base zéro** (ligne 11, colonne 6).
- `FileMatch { path (absolu), relative ('/'), name_start, positions, kind,
  library, recent }` : `positions` = indices de **caractères** appariés dans
  `relative`, croissants ; `relative[..name_start]` est le dossier (gris).
- Requête vide : les `récents` fournis par l'appelant (chemins absolus, le plus
  récent d'abord), présents dans l'index, 20 au plus (`kind == Recent`).
- Classement : `MatchKind` (`ExactName` < `NamePrefix` < `InName` < `InPath`),
  puis récent, puis projet avant bibliothèque, puis qualité (débuts de mots,
  initiales CamelCase/snake_case, morceaux consécutifs), puis chemin le plus
  court, puis alphabétique. `/` et `\` font correspondre les dossiers ; les
  espaces séparent des morceaux à trouver dans l'ordre.
- Synchrone : `FileIndex::find_files(requête, récents, limite, cancel)`,
  `split_location(texte)`.

### Recherche dans les fichiers

- `TextQuery { pattern, regex, case_sensitive, whole_word, file_mask, scope:
  SearchScope::Project | Directory(absolu), max_matches (10 000) }` ;
  `TextQuery::new(motif)` : littéral, sans casse, tous fichiers, projet.
- Regex : syntaxe du crate `regex` (pas de références arrière ni de
  regards en arrière), ligne à ligne (pas de multiligne), `^`/`$` aux bornes de
  la ligne, `$` avant un `\r`. Mot entier : ni lettre, ni chiffre, ni `_` de part
  et d'autre (règle de `git grep -w`, `$id` compris). Correspondances vides
  écartées.
- Masque : `*.php, *.js`, sans casse ; sans `/` sur le nom, avec `/` sur le
  chemin relatif ; `!*.min.js` exclut.
- `UnsavedFiles = HashMap<PathBuf (absolu), Arc<str>>` : contenu des onglets
  modifiés, cherché **à la place** du disque ; un fichier non enregistré absent
  de l'index (nouveau) est cherché aussi s'il est dans la portée.
- `FileTextMatches { path, relative, lines: Vec<LineMatch>, unsaved }`, un
  fichier en une fois, fichiers dans un ordre quelconque (l'interface trie ou
  groupe). `LineMatch { line, matches: Vec<TextRange>, snippet, snippet_start,
  line_length }` : `matches` en colonnes de **caractères** de la ligne entière
  (à passer telles quelles à l'éditeur pour sélectionner) ; une ligne de plus de
  400 caractères est rendue en extrait (`snippet`) qui commence 80 caractères
  avant la première occurrence (`snippet_start`).
- `TextSearchSummary { matches, lines, files_with_matches, files_searched,
  skipped_binary, skipped_too_large, unreadable, limit_reached, elapsed }` :
  « n occurrences dans m fichiers » = `matches` / `files_with_matches` ;
  « 2 fichiers trop gros ignorés » = `skipped_too_large` ; « Plus de 10 000
  résultats » = `limit_reached`.
- Binaire : octet NUL dans les 8 000 premiers octets (comme Git). Au-delà de
  `max_file_size` : ignoré et compté. Encodage : décodé comme l'éditeur
  (`julie_core::file::decode` : UTF-8, BOM, repli Windows-1252) ; colonnes sans
  la BOM.
- Aperçu : `Preview { path, line, lines: Vec<PreviewLine { line, text, matches }>,
  line_count }`. Synchrone : `preview_file(chemin, non_enregistré, ligne, avant,
  après, Option<&Matcher>)`, `preview_text(…)`.
- Synchrone : `Matcher::new(&query)?`, `FileMask::parse(masque)?`,
  `files_to_search(&index, &query, &mask, &unsaved)?`,
  `search_files(&fichiers, &matcher, &SearchOptions::new(&query, &settings),
  cancel, &mut |paquet| …)?`.

### Index des fichiers

- Parcours parallèle (`ignore`, 4 fils) : `.gitignore` à toute profondeur
  (appliqué aussi hors dépôt Git), `.git/info/exclude`, exclusion globale de
  Git ; fichiers cachés inclus (`.htaccess`) ; liens symboliques ignorés.
  `vendor`, `node_modules` : indexés seulement s'ils ne sont pas ignorés par Git,
  marqués `is_library()`.
- `FileIndex` : `scan(racine, réglages, cancel)`, `update(&chemins, cancel)`,
  `entries()`, `get(relatif)`, `contains`, `absolute`, `relative`, `len`,
  `from_relative_paths(…)` (sans disque). `FileEntry` : `relative()`, `name()`,
  `directory()`, `name_start()`, `is_library()`.
- Mise à jour : un `.gitignore` changé fait revoir son dossier ; celui de la
  racine, tout le projet (`full_rescan`). `.git/info/exclude` ou l'exclusion
  globale changés : relancer `scan()`.

### Navigation (v1) — pour dev-editeur et dev-interface

**Nouvelles variantes de `IndexEvent`** : `SymbolsReady`, `SymbolsUpdated`,
`Navigation`, `Usages`, `UsagesFinished`. Le `match` exhaustif de
`crates/app/src/app.rs` (`on_index`) doit gagner ces branches.

- `symbols_progress() -> Option<u8>` : `None` = index des symboles prêt, sinon
  pourcentage (« Indexation des symboles… n % » ; « Indexation en cours (n %),
  réessayez dans un instant » quand rien n'est trouvé).
- `NavigationRequest { path (absolu), text: Arc<str> (texte de l'éditeur),
  position: TextPosition (colonne en caractères) }`. Le même appel sert au
  Ctrl+clic, à Ctrl+B et au survol avec Ctrl : l'analyse du fichier est gardée
  tant que le texte ne change pas (survol répété : quelques ms, voir mesures).
- `NavigationResult { path, position, word: Option<TextRange>, navigation,
  indexing: Option<u8> }`. `word` : le mot navigable à souligner (`None` : pas de
  soulignement, pas de main).
- `Navigation` :
  - `Targets(Targets { targets, guess, constructor })` : `targets` jamais vide,
    dans l'ordre de la spec (fichier courant, même espace de noms / même dossier,
    projet, bibliothèques ; puis nom qualifié). Une cible et `guess == None` :
    saut direct ; sinon liste surgissante. `guess: Some(Guess::UnknownReceiver
    { receiver: "$other", member, kind })` → titre « Type de $other inconnu :
    méthodes nommées getName » ; `Guess::GlobalName { name }` (JS sans modules).
    `constructor` : sur `new X(…)`, le constructeur (propre ou hérité), à
    proposer **en plus** (décision n° 9) ; il ne change pas le saut direct.
  - `Declaration(Declaration)` : le curseur est sur une déclaration → lancer
    `find_usages(UsagesQuery::new(d))` (liste surgissante des usages ; un seul
    usage : saut direct).
  - `OpenFile(chemin)` : chaîne d'un `import`/`require`.
  - `Nothing(NoTarget)` : `NotASymbol` (rien : commentaire, chaîne, espace),
    `NotInProject { name }` (« Aucune déclaration dans le projet »),
    `NotFound { name }` (« Aucune déclaration trouvée »), `DynamicName`
    (« Nom calculé à l'exécution » ; pas encore émis : sur `$obj->$nom()` le
    Ctrl+clic vise la variable `$nom`), `ExternalModule { specifier }` (« Module
    externe : non résolu »). Le curseur ne bouge pas.
- `Declaration { kind: SymbolKind, language, name, qualified_name
  ("App\Model\User::getName", "Api.get"), container, path, relative, range
  (nom déclaré : y placer le curseur), line_text (sans indentation, 300 car.),
  library }`. `SymbolKind` : `Namespace`, `Class`, `Interface`, `Trait`,
  `Enum`, `EnumCase`, `Function`, `Method`, `Property`, `ClassConstant`,
  `Constant`, `Variable`, `Parameter`, `Import`, `TypeAlias`.
- `Usage { certainty: Sure | Possible, path, relative, range (colonnes de la
  ligne entière, à sélectionner telles quelles), line_text, line_text_start
  (colonne où commence l'extrait), library, unsaved }`.
- `UsagesSummary { symbol, sure, possible, files_with_sure,
  files_with_possible, files_searched, library_files_skipped, unreadable,
  dynamic_not_searched (membres : ligne d'aide du pied), index_complete,
  elapsed }`. Titre : « 5 usages de User::getName, 1 appel possible » ; le compte
  des appels possibles toujours affiché ; jamais « tous ».
- Synchrone (tests, mesures) : `SymbolIndex::build(&FileIndex, Option<&Path>
  cache, &UnsavedFiles, &CancelToken)`, `update`, `set_text`, `save_cache`,
  `navigate(&SymbolIndex, &NavigationRequest, &UnsavedFiles)`,
  `find_usages(&SymbolIndex, &UsagesQuery, &UnsavedFiles, &IndexSettings,
  index_complete, &CancelToken, &mut sink)`.
- Mesures et essais sur un projet où l'on n'écrit rien :
  `IndexService::with_symbol_cache(None | Some(dossier temporaire))`.

**Ce qui est résolu** (détail dans `navigation.md`) — PHP : noms de classes,
fonctions et constantes par l'espace de noms et les `use` (alias, groupés,
`use function`/`use const`, repli global) ; `self`, `static`, `parent`,
`X::class` ; membres par le type du receveur : `$this` (fermetures comprises),
`X::`, paramètres typés (unions), propriétés typées et promues, `new` dans la
même fonction (toutes les affectations du même type), `@var`/`@param`/`@return`,
types de retour déclarés (`self`, `static`) sur une chaîne de 4 maillons ; membre
le plus proche : classe, traits, parents, interfaces ; héritage cyclique arrêté.
JS/TS : portées (bloc, fonction, module), imports nommés/défaut/espace de noms,
réexports (`export {…} from`, `export * from`, 5 niveaux), chemins relatifs
(`x`, `.js .ts .tsx .jsx .mjs .cjs`, `x/index.*`), `this.m()`, `x.m()` si
`x = new C()`, `ns.a`, JSX `<Composant />`, globales de scripts sans modules
(devinées).

**Ce qui reste « par le nom » (deviné ou absent)** : toute valeur de type
inconnu (retour non typé, tableau, `$obj->$nom`, variable réaffectée de types
différents, exception attrapée `catch (E $e)`, variable d'un `foreach`),
propriétés et méthodes JS sur autre chose que `this`/`new`/imports,
`__call`/`__get` (rien), `define()` à nom calculé, JS des `<script>` d'un HTML
ou d'un PHP (non indexé), alias de `tsconfig`, modules de `node_modules`.
Usages : seuls les fichiers qui **mentionnent** le nom (filtre par empreintes
dans l'index) sont lus ; les appels dynamiques, chaînes et commentaires (hors
types de documentation PHP) ne sont pas cherchés.

**Limites connues** : fichiers de plus de 1 Mio (`SYMBOL_MAX_FILE_SIZE` : tables
générées, polices) et `*.min.js` hors de l'index des symboles (la recherche de
texte les lit) ; une analyse neuve d'un fichier de 25 000 lignes coûte ~0,4 s
(la première demande sur ce fichier) ; pendant une recherche d'usages,
l'écriture de l'index est différée (la navigation résout alors avec l'index tel
qu'il est). Lignes séparées par `\n` seulement (un fichier à `\r` seuls aurait
des positions fausses).

### Mesures (24-25/11/2026, `lab`, lecture seule, compilation `--release`)

`cargo run -p julie-index --release --example mesure` (fichiers, texte) et
`--example mesure_symboles -- <projet>` (cache dans `%TEMP%`, effacé) :
indexation de 23 389 fichiers en 160–250 ms ; « aller au fichier » 1–5 ms
(médiane) ; recherche d'un texte absent 0,7–0,9 s à chaud, premier paquet
20–30 ms. Symboles : 15 469 fichiers, 120 858 déclarations, **26,6–35 s** sans
cache selon la charge de la machine partagée (cible 30 s : tenue une fois sur
trois ; l'analyse tree-sitter seule coûte 44 s de processeur), **0,4–1,5 s**
avec cache (une mesure à 9,9 s, disque chargé) ; mémoire estimée 33 Mio, cache
13 Mio. Aller à la déclaration par le service, analyse gardée : 3,8 ms
(médiane), 4,3 ms (max) sur 20 essais. Usages : classe instanciée 168 fois →
171 usages dans 17 fichiers, premier lot 5 ms, fin 30 ms ; `save` → 1 sûr, 24
possibles, premier lot 30–40 ms, fin 0,2 s.

### Prévu, pas livré

Recherche partout à onglets (classes, symboles), remplacement, proximité du
fichier courant dans le classement d'« aller au fichier », AngularJS, CSS et
HTML dans l'index des symboles, JS des `<script>`.
## 12. `julie-phpunit` (dev-phpunit, lot 4 de la v0 ; réglages, sondes et parties de la suite : v1, 25/11/2026)

Détail et exemple d'assemblage en tête de `crates/phpunit/src/lib.rs`. Ne dépend que
de `julie-core` (plus `serde`, `thiserror`). Spécification : `docs/specs/phpunit.md`.

### Service (fil de l'interface)

`PhpUnit::new(tasks, project, settings)` (`settings: PhpUnitSettings` = section
`[phpunit]` : `php_path`, `phpunit_path`, `configuration`, `extra_args`, `env`,
`timeout_seconds`, tous facultatifs ; `PhpUnitSettings::from_settings(&Settings)`). Chaque méthode rend un
`TaskHandle` tout de suite ; le résultat arrive en `PhpUnitEvent`
(`event.get::<julie_phpunit::PhpUnitEvent>()`, domaine `Domain::PhpUnit`, `event.task`
= identifiant de la tâche lancée).

| Méthode | Clé de tâche | Événements publiés |
|---|---|---|
| `detect()` | `phpunit.detect` | `Detected(Box<Detection>)` (versions sondées : deux processus) |
| `run(RunRequest)` | `phpunit.run` | `RunStarted(Box<RunStart>)`, des `Events(Vec<TestEvent>)`, `RunFinished(Box<RunSummary>)` ; ou `LaunchFailed(Problem)` seul |
| `stop(TaskId)` | | PHP et ses descendants tués ; `RunFinished` (`Stopped`) puis tâche `Cancelled` |
| `probe(ProbeRequest)` (v1) | `phpunit.probe` | `ProbeEvent { request, result }` (autre type que `PhpUnitEvent`, même domaine) ; rien si annulée |

- `RunRequest::all()`, `test_suite(nom)`, `filter(motif brut)`, `extra_args`, `env`,
  `working_dir`, `timeout` ; fichier, classe, méthode, jeu de données, nœud, échecs :
  voir « Lancer une partie de la suite (v1) » plus bas.
- Commande : `php <phpunit> --configuration <xml> --teamcity [--testsuite|--filter ..]
  [extra_args]`, dossier de travail = racine du projet. `RunStart::command_line` la
  donne telle qu'elle s'écrit en tête de la console (aussi dans le premier lot, en
  `TestEvent::Command`).
- `RunFinished` est le dernier événement de toute exécution lancée. `RunSummary {
  outcome, exit_code, counts, duration, reported }` ; `outcome` : `Completed` (même
  avec des échecs), `Aborted { exit_code }` (PHP s'est arrêté avant la fin du
  rapport : `exit()`, erreur fatale, config illisible), `Stopped`, `TimedOut`
  (réglage `timeout_seconds`, tâche `Done`). `is_green()` ; « Aucun test exécuté » =
  `counts.started == 0` ; un code de sortie non nul n'est pas un échec en soi.
- `LaunchFailed(Problem)` : `PhpNotFound`, `PhpPathInvalid(chemin)`,
  `PhpNotRunnable`, `PhpUnitNotFound`, `PhpUnitPathInvalid`, `NoConfiguration`,
  `ConfigurationPathInvalid`. Aucun test lancé ; la tâche finit `Failed`. Les
  textes viennent du catalogue d'après la variante.
- **Lots** : premier lot immédiat, puis toutes les 100 ms ou à 500 événements,
  expédiés par un fil de surveillance même pendant un long test (un nœud apparaît
  dès `testStarted`). Annulation, délai et lots en retard : un seul fil.
- Version synchrone sans bus, pour les tests : `julie_phpunit::execute(&project,
  &settings, &request, &cancel, &|event| ..)`.

### Côté `app` : l'arbre se remplit en direct

```rust
PhpUnitEvent::RunStarted(start) => view = RunView { tree: ResultTree::new(project.root()), start, summary: None },
PhpUnitEvent::Events(events)    => view.tree.apply_all(events),   // un lot par image au plus
PhpUnitEvent::RunFinished(sum)  => view.summary = Some(sum),
PhpUnitEvent::LaunchFailed(p)   => show_problem(p),
PhpUnitEvent::Detected(d)       => self.detection = d,
```

Écarter tout événement dont `event.task` n'est pas la dernière exécution lancée.

### Modèle de résultats (`ResultTree`)

- Arbre à plat, `NodeId` stable ; `tree.root()`, `node(id)`, `children(id)`,
  `path_of(id)`, `iter()`, `counts()`, `command()`, `phpunit_version()`,
  `reported_summary()`, `version()` (change à chaque événement : clé de cache),
  `is_ended()`.
- `TestNode` : `kind` (`Root`, `Suite`, `Class`, `Method`, `DataSet` ; une méthode
  à fournisseur de données est un `Method` dont les enfants sont des `DataSet`),
  `name`, `label` (nom à afficher), `state` (`TestState` : `Pending`, `Running`,
  `Passed`, `Skipped`, `Incomplete`, `Risky`, `Interrupted`, `Failed`, `Error` ;
  gravité croissante dans cet ordre sauf `Pending`/`Running`), `worst`, `duration`
  (somme pour un conteneur), `console`, `diagnostic`, `source`, `is_test`,
  `finished`. Un conteneur est `Running` jusqu'à sa fin puis prend l'état le plus
  grave de ses enfants.
- `Counts { total: Option<usize>, started, running, passed, failed, errors,
  skipped, incomplete, risky, interrupted }` + `finished()`, `ignored()`,
  `failures()`. `total == None` : afficher « sur ? ».
- **Sortie** : la sortie non reconnue (avertissement PHP, `echo`, messages inconnus)
  est rattachée au test en cours (`node.console`), sinon à la racine ; la console
  de la racine contient **toute** la sortie non TeamCity, dans l'ordre. Une ligne
  vide juste avant un message TeamCity est du cadrage de PHPUnit 9 et n'est pas
  gardée. `console_text()` ; `console_truncated` au-delà de 1 Mio par nœud.
- **Échec** : `diagnostic { message, trace: Vec<TraceEntry { text, location:
  Option<julie_core::Location> }>, comparison: Option<Comparison>, cause }`. Les
  lignes PHP (1-based) sont converties en `TextPosition` 0-based. `cause` =
  `ProcessExited { exit_code }` pour le test emporté par un arrêt brutal (message
  = fin de la sortie d'erreur, éventuellement vide ; l'interface écrit le texte).
  `Comparison::expected_text()` / `actual_text()` : texte prêt pour
  `julie_diff::diff_lines` (apostrophes et `\n` d'export retirés pour une chaîne).
- **Liens dans la console** : `julie_phpunit::find_links(texte, racine, |p| existe)`
  rend les plages `chemin:ligne` à souligner (chemins avec espaces et lettre de
  lecteur compris ; `exists` est au choix de l'appelant, avec cache).
- Double-clic sur un test : `node.jump_location()` (début du fichier : le
  `locationHint` de PHPUnit ne donne pas de ligne) ; `node.source.method` permet
  d'affiner avec l'index des symboles.
- Exécution interrompue : le test en cours passe en `Error` (arrêt brutal) ou
  `Interrupted` (arrêt demandé, délai) ; les tests jamais commencés ne sont pas
  inventés ; la racine prend `Error` / `Interrupted` même sans test en cours.

### Détection

`julie_phpunit::detect(root, &settings, probe_versions, &cancel)` -> `Detection {
php, phpunit, configuration }`, chacun un `Result<_, Problem>` : PHP = réglage, sinon
`php` du `PATH`, sinon `C:\Program Files (x86)\AgileBio\LabCollector\php\php.exe`
(un réglage invalide n'est jamais contourné) ; configuration = réglage, `phpunit.xml`,
`phpunit.xml.dist`, `phpunit.dist.xml`. `Detection::plan()` -> `LaunchPlan` ou le
premier `Problem`. `detect_files(root, &settings)` : la même détection **sans
processus** (accès disque seulement, versions `None`) : sûre sur le fil de l'interface
pour un projet local.

**PHPUnit** (v1, 25/11/2026, `reglages.md`) = réglage `phpunit_path`, sinon, dans
l'ordre (`julie_phpunit::PHPUNIT_CANDIDATES`) : `vendor/bin/phpunit`,
`vendor/phpunit/phpunit/phpunit`, `vendor/bin/phpunit.phar`, `phpunit.phar`,
`tools/phpunit.phar`, `tools/phpunit/phpunit.phar` (source `ToolsPhar` : cas de
`backoffice`), `tools/phpunit` quand c'est un fichier (source `ToolsFile`, supposé :
Phive). Règles : chaque candidat doit être un **fichier** (un dossier du même nom est
sauté : `backoffice` a un dossier `tools/phpunit/`) ; **nom exact**, jamais approchant
(`vendor/bin/phpunit.rar` n'est pas pris) ; on lance toujours `php <script>`, jamais un
`.bat` (un `.bat` réglé à la main est remplacé par le script du même nom sans
extension, `PhpUnitPathInvalid` s'il n'existe pas) ; un dossier réglé à la main est
`PhpUnitPathInvalid`. Échec : `Problem::PhpUnitNotFound { searched }` liste **tous** les
emplacements essayés, dans l'ordre. Les chemins trouvés ont des séparateurs homogènes
(`…\tools\phpunit\phpunit.phar`).

### Réglages `[phpunit]` (v1)

`PhpUnitSettings` : `php_path`, `phpunit_path`, `configuration`, `timeout_seconds`
et, nouveaux :

| Clé | Type | Rôle |
|---|---|---|
| `extra_args` | `Vec<String>` (tableau TOML) | options ajoutées à chaque exécution, **avant** celles de la demande, après `--teamcity` (et après `--testsuite`/`--filter`) |
| `env` (`[phpunit.env]`) | `BTreeMap<String, String>` | variables ajoutées à l'environnement de l'IDE, elles l'emportent sur lui ; une demande (`RunRequest::env`) l'emporte sur elles. Valeurs TOML numériques ou booléennes acceptées |

- `PhpUnitSettings::issues() -> Vec<SettingsIssue>` : `ReservedArgument(arg)`
  (`--teamcity`, `--configuration[=…]`, `-c`, `--printer`, `--log-teamcity`,
  `--no-configuration`), `EmptyArgument`, `InvalidEnvName(nom)` (vide, `=` ou NUL :
  la variable est écartée au lancement). Avertissements : n'empêchent pas
  d'enregistrer. Les arguments réservés **restent** dans la commande (c'est ce qu'on a
  écrit qui est montré).
- `split_arguments(&str) -> Vec<String>` / `join_arguments(&[String]) -> String` : la
  ligne de la fenêtre (aux espaces, les guillemets doubles groupent, `\"` littéral).
  Aller-retour garanti.
- Fusion des couches (`julie-core`) : `extra_args` est **remplacé en bloc** par la
  couche projet ; `[phpunit.env]` fusionne par nom de variable.
- `RunRequest::with_settings(&settings)` rend la demande **effective** (options et
  variables du réglage d'abord). `execute` et `preview_command` l'appliquent : ne
  jamais la ré-appliquer.

**La commande exacte, pour l'afficher** :

```rust
let preview = julie_phpunit::preview_command(project.root(), &draft_settings, &RunRequest::all())?;
// preview.command_line : "C:\…\php.exe" "C:\…\tools\phpunit\phpunit.phar" --configuration "C:\…\phpunit.xml" --teamcity --stop-on-failure
// preview.args, preview.env, preview.working_dir, preview.plan
```

`Err(Problem)` = le premier problème de la détection (rien à lancer). Aucun
processus, aucun fichier écrit. `preview_with(&detection, …)` si l'on a déjà une
détection. C'est la même chaîne que `RunStart::command_line` (testé).

### Sondes de validation (`julie_phpunit::probe`)

Pour la fenêtre de réglages, en tâche de fond, à chaque frappe :

```rust
let handle = phpunit.probe(ProbeRequest::Php(path));            // ou julie_phpunit::probe::spawn(&tasks, root, request) sans projet
// ProbeRequest::{ Php(chemin), PhpUnit { php: Option<chemin>, script }, Configuration(chemin) }
// -> bus Domain::PhpUnit : event.get::<julie_phpunit::ProbeEvent>() = ProbeEvent { request, result }
```

- Tâche `phpunit.probe`. Annuler la précédente avec `Tasks::cancel(id)` : une sonde
  annulée ne publie **rien** et tue son processus. Écarter tout `ProbeEvent` dont
  `event.task` n'est pas la dernière sonde lancée.
- `result: Result<ProbeSuccess, ProbeFailure>` :
  - `Php { path, version }` (« PHP 8.2.12 »), `PhpUnit { path, version: PhpUnitVersion }`
    (« PHPUnit 9.6.39 »), `Configuration { path }` (fichier lisible, contient `<phpunit`) ;
  - `NotFound(chemin)` (« Ce chemin n'existe pas »), `IsDirectory(chemin)`,
    `TimedOut(5 s)` (« Pas de réponse en 5 s »), `NotThis { kind, first_line }`
    (« Ce n'est pas PHP / PHPUnit / une configuration PHPUnit : <ligne> »),
    `NotRunnable { path, reason }`, `Unreadable { path, reason }`,
    `PhpUnavailable` (sonde de PHPUnit sans PHP utilisable).
- Un nom nu (`php`) est cherché dans le `PATH` ; un chemin relatif se lit depuis la
  racine ; un `.bat` donné comme script PHPUnit est remplacé par son script.
- Versions synchrones (bloquantes, à ne pas appeler sur le fil de l'interface) :
  `probe(root, &request, &cancel)`, `probe_php`, `probe_phpunit`,
  `probe_configuration` (sans processus). La sonde ne lance que `php --version` et
  `php <script> --version` : aucun test, aucune écriture.

### Lancer une partie de la suite (v1)

`RunScope` : `All`, `TestSuite(nom)`, `Filter(motif brut)` et, nouveau,
`Selection(Selection { base: Base, filter: Option<String> })` où `Base` =
`Default` | `TestSuite(nom)` | `Path(fichier ou dossier)`. Commande :
`php <phpunit> --configuration <xml> --teamcity [--testsuite S] [--filter F] [options] [chemin]` —
le chemin vient **en dernier** (PHPUnit cesse de lire les options au premier argument
qui n'en est pas une) et remplace les suites de `phpunit.xml` : on peut donc lancer
un test qui n'est pas dans `defaultTestSuite`.

| Constructeur | Lance |
|---|---|
| `RunRequest::path(fichier_ou_dossier)` | ce fichier ou ce dossier |
| `RunRequest::class(fichier, "Ns\\Classe")` | tous les tests de la classe |
| `RunRequest::method(fichier, classe, méthode)` | la méthode et tous ses jeux de données |
| `RunRequest::data_set(fichier, classe, méthode, &DataSetKey::Name(..) \| Index(n))` | un seul jeu |
| `RunRequest::for_node(&tree, id, previous: Option<&RunRequest>) -> Option<RunRequest>` | un nœud de l'arbre : racine = relancer `previous` ; suite de premier niveau = `--testsuite` ; classe, méthode, jeu = filtre + fichier. `None` si le nœud n'a pas de source (entrée de menu grisée) |
| `RunRequest::rerun_failures(&tree, previous) -> Option<RunRequest>` | les tests `Failed` et `Error` de l'arbre ; `None` s'il n'y en a aucun (entrée grisée) |

`previous` (la demande de l'arbre affiché) fournit options, variables, dossier, délai
et la base (suite ou chemin) quand il n'y a qu'un filtre. **L'interface garde donc la
`RunRequest` de la dernière exécution à côté de son arbre.** Filtres produits :
expressions régulières **délimitées et ancrées** (`/^Ns\\Classe::méthode(?: with data
set .*)?$/s`), jamais réinterprétées par PHPUnit (qui lit autrement `#3` et `@nom`
dans un motif non délimité) ; tout nom de test passe par `quote_regex` (`'`, `|`, `[`,
`]`, `"`, `\`, `/`, `#`, retours à la ligne, NUL, accents). Détails dans `select.rs`.

- Le chemin n'est passé que si le nom du fichier est celui de la classe (PHPUnit
  retrouve la classe d'après le nom du fichier) ; sinon base de `previous`.
- « Relancer les échecs » sur plusieurs fichiers : pas de chemin, base de `previous` ;
  un seul fichier : ce fichier. Un échec sans source connue : on reprend la portée
  précédente sans filtre (on relance plus, jamais moins). Filtre de plus de
  `MAX_FILTER_LEN` (20 000) caractères : un motif par classe.
- Jeu de données dont le nom n'est pas lisible : toute la méthode (plus, jamais moins).
- Vérifié contre PHPUnit 9.6 sur `tests/fixtures/phpunit` (suites `tricky` et `env`
  comprises : guillemets, `|`, `[]`, `/`, `\`, `#`, `@`, regex, retour à la ligne,
  accents, japonais, `.*`). PHPUnit 10 à 12 non essayés (format de nom supposé
  identique : `Classe::méthode with data set "x"` / `#3`).

### Limites connues (à lire avant de s'en servir)

- PHPUnit 9 émet erreur, échec et test risqué sous le même `testFailed`, et ignoré /
  incomplet sous le même `testIgnored` : la crate les distingue par le message
  (`Classe : message` = erreur ; `This test ...` = risqué) et par la ligne de code
  de la trace (`markTestIncomplete`). Heuristique, vérifiée sur PHPUnit 9.6
  seulement ; en cas de doute un test est « échoué » ou « ignoré ».
- PHPUnit 10, 11 et 12 n'ont pas été exécutés (absents de la machine) : l'analyseur
  est tolérant et testé sur des sorties écrites de mémoire, pas enregistrées.
- Pas encore : découverte (gouttière), configurations nommées, historique des
  exécutions. Commande libre (`composer test`, ParaTest, Pest) écartée
  (`decisions-v1.md` n°7).

## 13. `julie-terminal` et le composant terminal (dev-terminal, lot 4 de la v0)

Détail et exemple en tête de `crates/terminal/src/lib.rs` et de
`crates/app/src/terminal/mod.rs`. Spécification : `docs/specs/terminal.md`.
Ne dépend que de `julie-core` (plus `alacritty_terminal` 0.26, sans ses
fonctionnalités par défaut, pour l'émulation, et `windows-sys`). La pseudo-console
est la nôtre (pas le module `tty` d'alacritty) : on tient l'arbre de processus
dans un objet « job ».

### Service et sessions (fil de l'interface, rien de bloquant)

`Terminals::new(tasks, settings)` (`settings: TerminalSettings` = section
`[terminal]` : `default_profile` (`"cmd"`, `"git-bash"`, `"powershell"`),
`git_bash_path`, `cmd_path`, `scrollback` (10 000)).

| Méthode | Clé de tâche | Événements (`TerminalEvent`, sous `Domain::Terminal`) |
|---|---|---|
| `detect_profiles()` | `terminal.profiles` | `Profiles(Profiles)` |
| `open(SessionOptions { profile, cwd, columns, lines }) -> Session` | `terminal.session` (dure autant que la session) | `Started { session, pid }`, `Output { session }` (coalescé), `TitleChanged { session, title }`, `Bell`, `Clipboard { text }` (OSC 52), puis `Exited { session, code }` ou `Closed { session }`, ou `Failed { session, error }` |

- `Session` (possédée par l'onglet ; la lâcher la ferme) : `id()`, `status()`
  (`Starting`, `Running { pid }`, `Exited { code }`, `Closed`, `Failed`),
  `title()`, `generation()`, `snapshot(&mut Screen)`, `resize(colonnes, lignes,
  largeur_px, hauteur_px)`, `send_text`, `send_key(Key, Modifiers)`, `paste`,
  `write`, `scroll(ScrollAction)`, `wheel(lignes)`, sélection
  (`start_selection`, `extend_selection`, `clear_selection`, `selection_text`,
  `has_selection`), `clear_history`, `running_programs()`, `pid()`,
  `process_ids()`, `close()` ; **bloquants** (tests, fermeture de l'IDE) :
  `wait(délai)`, `close_and_wait(délai)`.
- **Débit** : la sortie est analysée dans le fil de lecture, sous verrou ; un
  seul `Output` est en attente par session tant que l'écran n'a pas été relu
  (`snapshot` réarme). L'interface ne redessine donc qu'une fois par paquet,
  au plus une fois par image, et un onglet caché ne coûte rien.
- La saisie part dans un canal ; un fil de contrôle par session écrit, applique
  les redimensionnements (émulateur puis `ResizePseudoConsole`), surveille la
  fin de l'interpréteur. La saisie faite avant le démarrage est gardée.
- `Screen` : `columns`, `lines`, `cells: Vec<ScreenCell { ch, zerowidth, fg,
  bg, flags }>`, `cursor: Option<ScreenCursor { line, column, shape }>`,
  `display_offset`, `history_size`, `modes` (`app_cursor`, `bracketed_paste`,
  `alt_screen`, `alternate_scroll`, `mouse`, `show_cursor`…). `Color` :
  `Foreground`, `Background`, `Indexed(0..=255)`, `Rgb` — palette 0–15 et
  défauts dans le thème de l'interface, `xterm_rgb` pour 16–255 ; une couleur
  redéfinie par le programme (OSC 4/10/11) arrive en `Rgb`. `CellFlags` :
  `BOLD`, `ITALIC`, `UNDERLINE`, `INVERSE`, `DIM`, `STRIKEOUT`, `HIDDEN`, `WIDE`,
  `WIDE_SPACER`, `SELECTED`.
- Pur et testable : `encode_key(Key, Modifiers, &Modes)`, `encode_paste(texte,
  encadré)`, `control_byte`, `Emulator` (octets → écran, sans processus),
  `detect_profiles` (accès disque), `profile(kind, &réglages)` (sans disque).
- Erreurs : `TerminalError` (`Core`, `ShellNotFound(chemin)`,
  `DirectoryNotFound`, `Os { call, code }`, `Unsupported`).

### Processus

- Lancement : `CreatePseudoConsole` + `CreateProcessW` avec l'attribut
  `PROC_THREAD_ATTRIBUTE_JOB_LIST` : l'interpréteur **naît** dans un job
  `KILL_ON_JOB_CLOSE` créé juste avant (pas un instant hors du job, même si
  l'IDE meurt pendant le lancement) ; aucun descendant n'échappe au job.
  Entrées-sorties standard nulles dans `STARTUPINFO` (sinon un IDE lancé depuis
  une console y attacherait l'enfant). Environnement hérité + variables du
  profil ; Git Bash : `--login -i`, `TERM=xterm-256color`, `COLORTERM=truecolor`,
  `CHERE_INVOKING=1`. Dossier de départ donné tel quel (espaces et parenthèses
  testés).
- Fermeture (`close`, `Drop`) : `TerminateJobObject` tout de suite (non
  bloquant), puis le fil de contrôle ferme la pseudo-console pendant que la
  lecture vide encore la sortie (sinon `ClosePseudoConsole` peut attendre
  conhost). Fin de l'interpréteur par lui-même : on attend 150 ms de silence
  (2 s au plus), on tue ce qui reste dans le job, on ferme ; `Exited { code }`
  arrive après toute la sortie.
- IDE tué brutalement : Windows ferme le job (arbre tué) et conhost s'arrête
  (vérifié par `host_killed_leaves_no_orphan`). IDE qui se ferme : rien
  n'attend (`TerminalPanel::close_all` / `Drop`) ; `close()` sur chaque session
  tue son job tout de suite, la fermeture des pseudo-consoles finit dans les
  fils de session ou, si le processus se termine avant, Windows s'en charge
  (vérifié par `closing_without_waiting_then_exiting_leaves_no_orphan`, sessions
  occupées et session fermée avant d'avoir démarré). Une session fermée avant
  son démarrage ne lance rien (`Closed` directement).
- **Ctrl+C et parent qui l'ignore** (écart M2) : l'état « ignorer Ctrl+C »
  (bit 0 de `RTL_USER_PROCESS_PARAMETERS.ConsoleFlags`) s'hérite à la création
  de chaque processus, pseudo-console comprise, et aucun drapeau de création ne
  l'évite. Un IDE lancé par un parent qui ignore Ctrl+C (Git Bash de l'outil
  d'agent, `CREATE_NEW_PROCESS_GROUP`) le transmettait à cmd puis à ping, qui ne
  s'interrompaient plus. Avant le premier lancement, si le processus ignore
  Ctrl+C, `julie-terminal` installe un gestionnaire qui n'avale que
  `CTRL_C_EVENT` (l'IDE garde donc son comportement) puis remet le drapeau à
  zéro (`SetConsoleCtrlHandler(NULL, FALSE)`), comme Windows Terminal. Une fois
  par processus ; rien n'est touché si Ctrl+C n'était pas ignoré. Essai :
  `ctrl_c_works_when_the_ide_ignores_ctrl_c` (hôte lancé avec
  `CREATE_NEW_PROCESS_GROUP`, cmd + ping, Git Bash + sleep, et l'hôte survit à
  un Ctrl+C envoyé à sa propre console).
- `TerminalPanel::shutdown(délai)` ne bloque plus (délai ignoré) : préférer
  `close_all()`.

### Composant (`crates/app/src/terminal`)

- `TerminalView::new(id)`, `show(ui, &session)` / `show_in_rect` →
  `TerminalOutput { response, intents, columns, lines }` ;
  `TerminalIntent::OpenFile { path, line, column }` (Ctrl+clic sur
  `chemin:ligne[:colonne]`, chemin tel qu'affiché : `resolve_path(cwd, path)`
  le rend absolu, chemins MSYS `/c/…` compris). `TerminalTheme` (sombre, clair).
- Raccourcis gardés par l'IDE quand le terminal a le focus : Alt+F12, Alt+0–9,
  Ctrl+Maj+N/F/T, Ctrl+F4 ; tout le reste va au shell (Échap, Tab, flèches,
  Ctrl+C/D/R/S…). La coquille ne consomme plus Ctrl+S quand le terminal a le
  focus (`TerminalPanel::has_focus`).
- Branchement (`panel.rs`, à rhabiller par dev-interface) : `TerminalPanel::new`,
  `on_event`, `ui_global` (à chaque image : confirmation à la fermeture de l'IDE
  si un programme tourne, presse-papiers OSC 52), `show` (onglet Terminal),
  `has_focus`, `focus` (Alt+F12). Variable d'essai `JULIESTORM_TERMINAL=1` :
  l'IDE s'ouvre sur l'onglet Terminal.

### À savoir, pas fait

- La tâche `terminal.session` reste « en cours » toute la vie de la session :
  la barre d'état ne doit pas la compter comme un travail en attente.
- Pas de projet dans la coquille : les shells partent du dossier courant de
  l'IDE. À brancher sur `Project::root()` quand il existe.
- Ctrl+clic ouvre le fichier mais ne saute pas encore à la ligne (la coquille
  n'a pas de « aller à la ligne » après chargement).
- Souris envoyée aux programmes (vim `mouse=a`) : non gérée, la souris reste à
  la sélection. Pas de clignotement du curseur (pas d'image redessinée sans
  raison). Pas de réagencement au redimensionnement côté conhost (hors v0).
- Gras : tracé doublé (egui n'a pas de monospace gras par défaut).
- Textes de l'interface regroupés dans des modules `texts` (panel.rs, view.rs)
  en attendant le catalogue de `redacteur`.
