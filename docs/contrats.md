# Contrats entre crates

Tenu par `architecte-technique`. Ce document décrit ce que `julie-core` fournit et
les règles que toutes les crates suivent. Si quelque chose manque à `julie-core`,
on le demande à l'architecte dans son rapport : on ne modifie pas le noyau soi-même.

État : lot 1 de la v0 (24/11/2026).

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
   `Project::ensure_data_dir`, `Settings::load`, `TaskHandle::join`,
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
}
```

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
  (`.juliestorm/index/…`) et par `write_atomic`.
- `Settings::load(Some(&project))?` fusionne
  `%APPDATA%\JulieStorm\settings.toml` puis `<racine>/.juliestorm/settings.toml`
  (le projet l'emporte, clé par clé). Fichier absent = vide ; fichier mal formé =
  `Error::Settings`.
- Une table TOML par crate, du nom de la crate sans préfixe (`[vcs]`, `[index]`,
  `[phpunit]`, `[terminal]`, `[text]`, `[diff]`, `[app]`). La crate déclare sa
  structure et la lit :

```rust
#[derive(serde::Deserialize, Default)]
#[serde(default)]
pub struct PhpUnitSettings { pub php_path: Option<PathBuf>, pub phpunit_path: Option<PathBuf> }

let settings: PhpUnitSettings = settings.section("phpunit")?;   // section absente -> Default
```

- Lecture seule pour l'instant. `Settings` se clone et se passe aux tâches.

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
  code, stderr }`, `Unencodable`, `Settings`, `Invalid(String)`.
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
| `toml` | 0.9 | réglages (dans le noyau ; les crates n'en ont en principe pas besoin) |
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
- Liste des fichiers ouverts dans le modèle de projet (pour l'instant dans `app`).
- Écriture des réglages.
- Journal d'exécution (fichier de log) commun.
- UTF-16 et détection des fichiers binaires à la lecture.
- Plafond du nombre de tâches simultanées, si la mesure en montre le besoin.


## 8. `julie-vcs` (dev-git, lot 2 de la v0)

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
- **Plus prudent que Git, exprès** : un changement dont la place est ambiguë
  (ligne vide retirée d'une série, fonction insérée entre deux accolades, ligne
  déplacée) est testé sur toute sa zone d'ambiguïté. D'où parfois un conflit là
  où `git merge-file` fusionne, et parfois un seul bloc pour deux changements
  voisins du même côté. Jamais l'inverse sur le corpus et les tirages aléatoires.
- **Fins de ligne** : `\r\n` et `\n` sont la même ligne (cas 15) ; Git, lui, y
  voit un changement. Écarts listés dans `tests/fixtures/diff/merge/cases.txt`.
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

## 11. `julie-index` (dev-indexation-recherche, lot 3 de la v0)

Détail et exemple en tête de `crates/index/src/lib.rs`. Ne dépend que de
`julie-core` (plus `ignore`, `globset`, `regex`, `memchr`). Spécification :
`docs/specs/recherche.md`. Rien n'est écrit sur le disque (pas de cache en v0).

### Service (fil de l'interface)

`IndexService::new(tasks, project, settings)` (`settings: IndexSettings` =
section `[index]` : `excluded` (défaut `.git`, `.juliestorm`, `/target` ; un nom
seul vaut à toute profondeur, `/nom` à la racine seulement), `libraries` (défaut
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

### Mesures (24/11/2026, `lab`, lecture seule, compilation `--release`)

`cargo run -p julie-index --release --example mesure -- <projet>` : indexation
de 23 389 fichiers en 170–185 ms ; « aller au fichier » 1–5 ms (médiane) sur
`lab`, 10–24 ms sur 70 000 chemins ; recherche d'un texte absent sur 18 736
fichiers 0,7–0,9 s à chaud (`git grep` : 1,1–1,4 s), premier paquet en 20–30 ms.

### Prévu, pas livré (v1)

Index des symboles (attend la décision sur `tree-sitter` dans
`[workspace.dependencies]`, section 10), recherche partout à onglets,
remplacement, aller à la définition, usages, cache disque dans
`.juliestorm/index/`, proximité du fichier courant dans le classement.
