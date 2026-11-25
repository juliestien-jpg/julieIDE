# JulieStorm — IDE de développement web écrit en Rust

IDE de bureau pour le développement web : **PHP, HTML, CSS, JavaScript/TypeScript,
React (JSX/TSX), AngularJS**. L'inspiration de PhpStorm est assumée pour les
fonctionnalités et le ressenti (la personne qui dirige le projet en a la licence et
s'en sert tous les jours). En revanche le contenu est le nôtre : nom, code, icônes,
thème, textes. **On ne recopie rien de JetBrains** : ni code (pas de décompilation
des `.jar`), ni icônes, ni textes d'interface mot pour mot, ni fichiers de
configuration. On observe un comportement, on le décrit avec nos mots dans `docs/`,
on le réécrit.

Langue de l'interface, de la documentation et des commentaires : français.
Identifiants de code : anglais.

## Les trois piliers (par ordre de priorité)

1. **Git** — changements locaux, commit partiel, journal avec graphe, branches,
   étagère (mise de côté de changements), création et application de patchs, et
   surtout la **fusion à trois volets** : gauche = notre version, droite = la leur,
   centre = résultat éditable ; application automatique des changements non
   conflictuels, résolution automatique des conflits simples, acceptation bloc par
   bloc.
2. **PHPUnit** — découverte des tests, configurations d'exécution, arbre de
   résultats en direct, relance des échecs, saut à la ligne depuis une trace,
   comparaison attendu/obtenu.
3. **Indexation et recherche** — recherche partout (fichiers, classes, symboles,
   actions, texte), recherche dans les fichiers (regex, casse, mots, filtre de
   fichiers, portée), aller à la définition, trouver les usages.

S'y ajoute, demandé le 24/11/2026 par la personne qui dirige le projet : un
**terminal intégré** — une ou plusieurs fenêtres de terminal en onglets, au choix
**cmd** ou **Git Bash** (`C:\Program Files\Git\bin\bash.exe`), ouvertes dans le
dossier du projet.

Ce que l'on a observé dans PhpStorm est consigné dans `docs/observations-phpstorm.md` ;
les spécifications qui en découlent vivent dans `docs/specs/`.

## Choix techniques

- **Rust stable**, toolchain `x86_64-pc-windows-msvc`. Cargo est dans
  `%USERPROFILE%\.cargo\bin`, qui **n'est pas dans le PATH** : commencer chaque
  commande PowerShell par `$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"`.
- **Interface : egui / eframe** (Rust pur, mode immédiat). Choisi parce que le
  dessin libre (connecteurs courbes entre les volets de fusion, graphe du journal,
  gouttières) y est simple, et que l'état de l'interface reste dans nos structures.
  Version 0.37 : l'API a changé par rapport aux exemples courants (`App::ui` au
  lieu de `update`, `egui::Panel::left/bottom(...).show(ui, ...)` au lieu de
  `SidePanel`/`TopBottomPanel`) — lire les sources dans `~/.cargo/registry` plutôt
  que de se fier à sa mémoire. Rendu **forcé sur DirectX 12** dans `main.rs` : le
  pilote Vulkan Intel de cette machine plante à l'ouverture de la fenêtre.
- `rustfmt` et `clippy` sont installés : `cargo fmt` et `cargo clippy -- -D warnings` propres avant tout rapport.
- **Texte** : `ropey` (corde) pour les tampons ; **tree-sitter** pour la coloration
  et l'extraction des symboles (PHP, HTML, CSS, JS, TS, JSX/TSX).
- **Git : on pilote l'exécutable `git`** (sorties `--porcelain=v2`, `-z`), comme le
  fait PhpStorm : mêmes crochets, mêmes identifiants, même configuration que la ligne
  de commande. Pas de libgit2. Le moteur de diff et de fusion, lui, est **le nôtre**
  (`crates/diff`) et ne dépend pas de Git.
- **PHPUnit** : on lance `php` + `phpunit` avec la sortie `--teamcity` et on
  l'analyse en flux. PHP local : `C:\Program Files (x86)\AgileBio\LabCollector\php\php.exe`.
- **Recherche** : parcours respectant `.gitignore` (`ignore`), correspondance floue
  maison pour les noms, `regex` + `memchr` pour le texte, index de symboles en
  mémoire avec cache sur disque dans `.juliestorm/` du projet ouvert.
- Les seules dépendances téléchargées sans demander sont les crates de crates.io.
  Tout autre téléchargement (binaire, phar, police, icônes) se demande d'abord à la
  personne qui dirige le projet, avec nom, source et taille.

## Organisation du code

```
Cargo.toml             espace de travail
crates/
  core/      julie-core      types communs, bus d'événements, projet, réglages, tâches de fond
  text/      julie-text      tampon (corde), édition, annulation, syntaxe tree-sitter
  diff/      julie-diff      diff, fusion à trois volets, patchs (création, application)
  vcs/       julie-vcs       Git par la ligne de commande : état, commit, journal, branches, étagère
  index/     julie-index     index des fichiers et des symboles, recherche
  phpunit/   julie-phpunit   découverte, exécution, analyse de la sortie TeamCity
  terminal/  julie-terminal  pseudo-console Windows (ConPTY), émulation VT, sessions cmd / Git Bash
  app/       juliestorm      l'application egui : fenêtres d'outils, éditeur, dialogues
docs/        observations, spécifications, contrats entre crates
tests/fixtures/   dépôts Git et projets PHP d'essai, générés par script
```

Règles communes :

- **Les crates métier (`text`, `diff`, `vcs`, `index`, `phpunit`) ne dépendent pas
  d'egui** et ne se connaissent pas entre elles : elles ne dépendent que de
  `julie-core`. Seule `app` assemble. Tout le métier doit donc se tester sans fenêtre.
- **Rien ne bloque le fil de l'interface** : Git, PHPUnit, indexation et recherche
  tournent en tâches de fond et publient leurs résultats sur le bus de `julie-core`.
- Les contrats entre crates (types publics, événements) sont dans `docs/contrats.md`,
  tenu par `architecte-technique`.
- Chaque agent reste dans ses dossiers ; pour toucher ceux d'un autre, il le signale
  dans son rapport plutôt que de le faire.
- Aucun texte visible codé en dur dans les composants : tout passe par le catalogue
  de textes tenu par `redacteur`.
- `cargo fmt` et `cargo clippy -- -D warnings` propres avant tout rapport.

## Sécurité des données — règles impératives

Un IDE qui se trompe détruit du travail. Donc :

- **Jamais d'essai sur un vrai dépôt de cette machine** (`lab`, `camper`, etc.).
  Toute opération Git d'écriture se teste sur un dépôt jetable créé dans le dossier
  temporaire à partir de `tests/fixtures/`.
- **Exception accordée le 24/11/2026 : `C:\Program Files (x86)\AgileBio\LabCollector\www\labmate_paper`**
  (projet JavaScript, branche `main`, distant GitHub) peut servir à tout essayer,
  dans PhpStorm comme dans JulieStorm, conflits de fusion compris. On s'y tient à ceci :
  - les essais automatisés travaillent sur un **clone local** fait dans le dossier
    temporaire (`git clone <labmate_paper> %TEMP%\...`), pas sur l'original ;
  - sur l'original, seulement des branches locales préfixées `juliestorm-essai/` ;
    **jamais de `push`, jamais de commit sur `main`, jamais de réécriture de
    l'historique existant** ;
  - on le rend comme on l'a trouvé : sur `main`, arbre propre, branches d'essai
    supprimées — et on le dit dans son rapport.
- PHPUnit 9.6 est disponible dans `lab\vendor\bin\phpunit` : on peut **l'exécuter**
  (lecture seule) sur le petit projet PHP de `tests/fixtures/`, jamais sur `lab`.
- La fusion ne doit **jamais** résoudre en silence un conflit douteux : dans le
  doute, c'est un conflit, montré à la personne. Chaque règle de résolution
  automatique a ses tests, y compris les cas où elle doit s'abstenir.
- Toute écriture de fichier par l'IDE est atomique (fichier temporaire puis
  renommage) et respecte les fins de ligne et l'encodage d'origine.
- PhpStorm est ouvert sur les vrais projets de la personne : on y **regarde**, on n'y
  modifie rien (pas de commit, pas de checkout, pas de frappe dans l'éditeur).

## Ressources de la machine — règles impératives

Ce PC a 8 cœurs et il est partagé. La compilation Rust peut le saturer.

- `.cargo/config.toml` limite Cargo à **4 tâches** ; ne pas passer outre.
- **Une seule commande Cargo à la fois** sur toute la machine ; pas de
  `cargo watch`, pas de processus laissé en arrière-plan.
- Préférer `cargo check -p <crate>` et `cargo test -p <crate>` à une compilation
  complète ; ne compiler `app` que lorsqu'on a besoin de la voir tourner.
- L'application lancée pour un essai est fermée avant de rendre son rapport.
- Côté orchestration : **deux agents au plus en même temps, dont un seul qui compile.**

## Dépôt

Distant `origin` : https://github.com/juliestien-jpg/julieIDE.git. Les agents ne
font **ni commit ni push**. Décision du 24/11/2026 de la personne qui dirige le
projet : **l'orchestrateur commite et pousse sur `main` après chaque lot vérifié**
(tests verts, application qui se lance), un commit par lot, message en français.

## Lancer

```powershell
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
cargo run -p juliestorm            # l'IDE
cargo test --workspace             # tous les tests
```

## Périmètre de la v0 (livrée le 25/11/2026 ; recette dans `docs/recette-v0.md`)

But : en dix minutes d'usage, sentir les trois piliers. Rien d'autre.

- Ouvrir un dossier ; arbre du projet ; éditeur à onglets avec coloration
  PHP/HTML/CSS/JS, annuler/rétablir, enregistrer.
- **Git** : changements locaux (modifiés, non suivis), diff côte à côte, commit ;
  journal simple en liste ; quand le dépôt est en conflit, le **dialogue de fusion à
  trois volets** avec « appliquer les changements non conflictuels » et acceptation
  bloc par bloc.
- **Recherche** : aller au fichier (flou) et recherche dans les fichiers avec aperçu.
- **PHPUnit** : lancer la suite d'un `phpunit.xml`, arbre des résultats en direct,
  clic sur une trace → fichier et ligne.
- **Terminal** : onglets de terminal cmd et Git Bash dans la fenêtre d'outils du bas,
  plusieurs à la fois, couleurs ANSI, redimensionnement, copier-coller.
- Hors v0 : graphe du journal, branches, étagère, patchs, commit partiel, index des
  symboles, aller à la définition, usages, relance des échecs, couverture,
  complétion, débogueur.

## Périmètre de la v1 (en cours depuis le 25/11/2026)

Les trois piliers complets tels que décrits en haut, plus JSX/TSX et les gabarits
AngularJS dans la coloration et l'index. Complétion et inspections de code
viendront ensuite (probablement par des serveurs de langage).

Demandes de la personne qui dirige le projet, par priorité :

1. **Branches, « de la même manière que PhpStorm »** : widget de branche, basculer,
   créer, fusionner, rebaser, fetch, pull, push, mise à jour du projet, mise de
   côté automatique avant bascule, graphe du journal et actions sur un commit
   (`docs/specs/branches.md`).
2. **Réglages** : fenêtre de réglages, dont la façon d'exécuter PHPUnit (cas
   `backoffice` : `php tools\phpunit\phpunit.phar`) (`docs/specs/reglages.md`).
3. **Menus contextuels** partout, dont `git add` par clic droit
   (`docs/specs/menus-contextuels.md`).
4. **Navigation** : Ctrl+clic vers la déclaration et les usages, index des
   symboles (`docs/specs/navigation.md`).
5. Le reste des piliers : étagère, patchs, commit partiel, résolution automatique
   des conflits simples, relance des échecs, configurations de tests, recherche
   partout à onglets.

Règles propres aux opérations distantes : **les essais de fetch, pull et push se
font uniquement contre un dépôt nu local créé dans le dossier temporaire**, jamais
contre un vrai distant (GitHub ou autre), y compris celui de `labmate_paper`.
JulieStorm ne demande, ne stocke ni ne saisit jamais de mot de passe ou de jeton :
l'authentification reste au gestionnaire d'identifiants de Git. Un push forcé n'est
proposé que sur demande explicite, et toujours avec `--force-with-lease`.

## L'équipe d'agents (`.claude/agents/`)

| Agent | Métier | Modèle |
|---|---|---|
| `specialiste-produit` | observe PhpStorm, écrit les spécifications et les critères d'acceptation | opus |
| `architecte-technique` | architecture, `julie-core`, contrats, performance, revue transverse | fable |
| `dev-fusion-diff` | diff, fusion à trois volets, résolution automatique, patchs | fable |
| `dev-git` | Git : état, commit, journal, branches, étagère, conflits | opus |
| `dev-editeur` | tampon de texte, composant éditeur, coloration tree-sitter | opus |
| `dev-indexation-recherche` | index fichiers et symboles, recherche partout, recherche dans les fichiers | opus |
| `dev-phpunit` | découverte, exécution, arbre de résultats PHPUnit | sonnet |
| `dev-terminal` | terminal intégré : ConPTY, émulation VT, onglets cmd / Git Bash | opus |
| `dev-interface` | coquille egui : fenêtres d'outils, arbre du projet, menus, raccourcis, thème | sonnet |
| `testeur-qa` | tests automatisés, dépôts d'essai, essais de l'application, rapports de bugs | sonnet |
| `redacteur` | textes de l'interface, aide, documentation utilisateur | haiku |

Pourquoi ces modèles : `fable` là où une erreur coûte le plus cher (une fondation
que tout le monde utilise, une fusion qui abîme du code) ; `opus` pour les métiers
à algorithmes et à cas limites nombreux ; `sonnet` pour les métiers bien cadrés par
un contrat ou une spécification ; `haiku` pour la rédaction.
