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
