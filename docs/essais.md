# Essais de l'application : le mode scénario

Les agents (et la testeuse) ne peuvent ni cliquer ni taper dans une fenêtre egui,
et un outil de capture d'écran ne la voit pas. Le **mode scénario** fait jouer à
l'application une liste d'étapes, dans sa vraie fenêtre, puis la fait photographier
et décrire son état. C'est le moyen d'essai de référence pour tout ce qui s'affiche.

## Lancer

```powershell
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
cargo build -p juliestorm
.\target\debug\juliestorm.exe --scenario tests\scenarios\ouvrir-et-chercher.txt
```

- `--scenario <fichier>` ou la variable d'environnement `JULIESTORM_SCENARIO`.
- La fenêtre s'ouvre en 1280×720 (taille intérieure, 100 %). L'application se ferme
  toute seule à la fin (`quit` ou fin du fichier) : **aucun processus à tuer**.
- Code de sortie : 0 si tout s'est joué, **2** si une étape a échoué (action
  inconnue, touche illisible, fichier non écrit, `wait-idle` au-delà de 60 s). Les
  échecs sont écrits sur la sortie d'erreur (`[scénario fichier:ligne] ÉCHEC : …`) ;
  en version de développement (`cargo build`, pas `--release`) la console reste
  attachée, donc `2>&1 | tail` les montre.
- Un scénario illisible (étape inconnue) arrête tout avant l'ouverture : message
  `ligne n : …` et code 2.
- Les chemins relatifs des étapes `screenshot` et `state` partent du dossier
  courant ; les dossiers manquants sont créés.
- Le scénario ne rouvre **pas** le dernier projet et n'écrit pas
  `%APPDATA%\JulieStorm\last_project.txt`. Pour essayer cette fonction :
  `JULIESTORM_STATE_DIR=<dossier jetable>` redirige le fichier de mémoire (et réactive
  la mémoire du dernier projet dans le scénario) ; voir l'exemple plus bas.

## Format

Une étape par ligne ; `#` commence un commentaire ; lignes vides ignorées. Le mot
d'étape est suivi d'un seul argument qui va jusqu'à la fin de la ligne (les
espaces des chemins sont donc permis).

| Étape | Effet |
|---|---|
| `project <dossier>` | Ouvre le projet, puis attend la fin des tâches de fond (index prêt, Git lu, arbre listé). |
| `project-nowait <dossier>` | Idem sans attendre (pour photographier l'indexation en cours). |
| `file <chemin>[:ligne[:colonne]]` | Ouvre un fichier (relatif à la racine du projet) à la ligne donnée (base 1), puis attend. `file-nowait` : idem sans attendre la fin des tâches de fond (navigation pendant la construction de l'index des symboles). |
| `action <identifiant>` | Déclenche une action du registre (voir la liste ci-dessous). Échec si elle est indisponible dans l'état courant (grisée). |
| `type <texte>` | Saisit le texte dans le champ qui a le focus (éditeur, champ d'une fenêtre flottante, terminal). Le texte est celui de la ligne, après le premier espace. |
| `key <combinaison>` | Appuie sur une touche : `Enter`, `Escape`, `Tab`, `Down`/`Up`/`Left`/`Right` (ou `Bas`, `Haut`, `Gauche`, `Droite`), `F4`…, une lettre, un chiffre ; avec `Ctrl+`, `Alt+`, `Shift+` (ou `Maj+`). Ex. `key Ctrl+Maj+N`, `key Alt+Right`, `key Ctrl+A`. |
| `click <x> <y>` | Clic gauche aux coordonnées (points, origine en haut à gauche de la fenêtre). |
| `rclick <x> <y>` | Clic droit aux coordonnées : ouvre le menu contextuel de ce qui est dessous (arbre, changements, onglets, journal). C'est le vrai chemin de la souris. |
| `drag <x1> <y1> <x2> <y2> [hold]` | Glisse le bouton gauche d'un point à l'autre (appui, huit déplacements, relâchement) : séparateur entre deux groupes d'éditeurs, onglet vers le bord ou la rangée d'onglets d'un groupe. Avec `hold`, le bouton reste enfoncé (photographier l'aperçu d'un glisser). |
| `release <x> <y>` | Relâche le bouton gauche (après `drag … hold`). Échap entre les deux annule le glisser d'un onglet. |
| `wait <ms>` | Attend. |
| `wait-idle` | Attend que plus rien ne travaille : aucune tâche de fond (sauf sessions de terminal), index prêt, état Git connu, résultats de recherche et d'« aller au fichier » arrivés, fichiers chargés, dossiers de l'arbre listés. Quatre images calmes de suite. |
| `screenshot <fichier.png>` | Capture la fenêtre (PNG, taille réelle en pixels : 1280×720 à 100 %). |
| `state <fichier.json>` | Écrit l'état de l'IDE en JSON (voir plus bas). |
| `resize <largeur> <hauteur>` | Redimensionne la fenêtre (taille intérieure, en points). |
| `fullscreen on\|off` | Plein écran. |
| `scale <facteur>` | Mise à l'échelle de l'interface : `1.0` = 100 %, `1.5` = 150 %. (Les captures gardent la taille de la fenêtre en pixels : à 150 % on voit une interface plus grosse dans le même cadre.) |
| `theme dark\|light` | Thème sombre ou clair. |
| `tree-expand <chemin relatif>` | Déplie l'arbre jusqu'à ce dossier et le sélectionne. |
| `external-write <chemin> <texte>` | Réécrit un fichier **hors de l'IDE** (la séquence `\n` fait un saut de ligne) pour simuler un changement externe. **Refusé hors du dossier temporaire du système** : sert à tester le bandeau « changé sur le disque » sur une copie. |
| `log <texte>` | Écrit une ligne sur la sortie d'erreur. |
| `ui <commande> [argument]` | Pilote les vues Git, le dialogue de fusion et la vue des tests sans coordonnées (liste plus bas). Échec (code 2) si la commande est inconnue ou impossible dans l'état courant. |
| `quit` | Ferme l'application (sans confirmation, terminaux compris). |

Entre deux étapes l'application laisse passer quelques images pour que l'interface
se mette à jour (davantage après `click`, `resize`, `scale`, `fullscreen`). Une étape
ne démarre jamais avant que la précédente soit prise en compte, mais **pour tout ce
qui travaille en fond, mettre `wait-idle`** (ou `wait`) avant de capturer.

Les saisies passent par la vraie boucle d'événements d'egui : un raccourci
`key Ctrl+S` est donc traité exactement comme au clavier, y compris la règle « le
terminal garde ses touches quand il a le focus ».

### Actions (identifiants de `action`)

`file.open_folder` (ouvre le sélecteur de dossiers du système : à éviter en
scénario), `file.save_all`, `file.close_tab`, `file.reload_from_disk`, `file.quit`,
`nav.goto_file`, `nav.find_in_files`, `nav.action_palette`, `nav.previous_tab`,
`nav.next_tab`, `nav.last_tab`, `view.project`, `view.git`, `view.tests`,
`view.search`, `view.terminal`, `view.toggle_theme`. La liste à jour est dans
`crates/app/src/actions.rs` (chaque action y a son raccourci).

### Commandes `ui` (Git, fusion, tests)

Les fichiers Git se désignent par leur **chemin dans le dépôt** (`README.md`,
`src/main.js`), les blocs de fusion par leur **numéro** (`id` de `merge.blocks` dans
l'état, à partir de 0), les côtés par `left` / `right` (ou `gauche` / `droite`).

| Commande | Effet |
|---|---|
| `git.tab changes\|log\|console` | Ouvre la fenêtre Git sur cet onglet. |
| `git.select <fichier>` | Sélectionne le fichier dans la liste des changements (l'aperçu du diff se charge). |
| `git.check <fichier>` / `git.uncheck <fichier>` | Coche / décoche la case du fichier. |
| `git.message <texte>` | Pose le message de validation (`\n` = saut de ligne). |
| `git.amend on\|off` | Case « Amender le dernier commit ». |
| `git.commit` | Valider (ou conclure la fusion quand il n'y a plus de conflit) : comme le bouton. Échec si le bouton serait grisé. |
| `git.diff` | Ouvre le diff du fichier sélectionné en grande fenêtre (Ctrl+D). |
| `git.refresh` | Relit l'état Git. |
| `git.keep ours\|theirs\|yours\|incoming [fichier]` | Garder un côté (ouvre la confirmation ; sans fichier : tous les conflits). `ours` / `theirs` désignent les **étages de Git** (pendant un rebase, `ours` est l'amont) ; `yours` / `incoming` suivent les boutons « Garder votre version » / « Prendre <nom> » et sont les mêmes quelle que soit l'opération. |
| `git.abort` | « Abandonner » la fusion (ouvre la confirmation). |
| `git.log-select <n>` | Sélectionne le n-ième commit du journal (base 1) et lui donne le focus clavier (flèches, Début, Fin, pages). |
| `git.commit-file <n>` | Diff du n-ième fichier du commit sélectionné, en grande fenêtre. |
| `diff.close` | Ferme la grande fenêtre de diff. |
| `merge.open <fichier>` | « Fusionner… » : charge les trois versions et ouvre le dialogue (ou affiche le refus dans la barre d'état). |
| `merge.accept <n> left\|right` | Flèche « accepter » (ou « ajouter à la suite » sur un conflit déjà résolu d'un côté). |
| `merge.ignore <n> left\|right` | Croix « ignorer ». |
| `merge.both <n> left\|right` | Accepter les deux côtés d'un conflit (le côté donné d'abord), une seule étape annulable. |
| `merge.apply all\|left\|right` | « Appliquer les changements non conflictuels ». |
| `merge.next` / `merge.prev` | F7 / Maj+F7. |
| `merge.undo` / `merge.redo` | Ctrl+Z / Ctrl+Maj+Z. |
| `merge.edit <ligne> <texte>` | Remplace la ligne (base 1) du résultat, comme une frappe. |
| `merge.focus [ligne]` | Donne le focus au volet du centre, curseur au début de la ligne (base 1) : le `type` et les `key` suivants y vont. |
| `merge.save` | « Appliquer » (Ctrl+Entrée) : enregistre et marque résolu, après la question s'il reste des blocs. |
| `merge.close` | Fermer (Échap) : pose la question d'abandon si des gestes ont été faits. |
| `confirm` / `cancel` | Répond oui / non à la question ouverte (fusion, confirmation Git, relance des tests ; sinon ferme la fenêtre d'échec de validation). |
| `tests.run` / `tests.stop` | Lancer la suite (Ctrl+Maj+F10) / arrêter (Ctrl+F2). |
| `tests.select <texte\|racine>` | Sélectionne le premier nœud dont le libellé contient le texte et déplie ses parents. |
| `tests.toggle` | Plie / déplie le nœud sélectionné. |
| `tests.passed on\|off` | Filtre « Afficher les réussis ». |
| `tests.compare` | Ouvre attendu / obtenu du nœud en diff côte à côte. |
| `tests.link <n>` | Clique le n-ième lien `fichier:ligne` de la console affichée (saut dans l'éditeur). |

### Commandes `ui` des branches, des menus contextuels et des dialogues (v1)

Elles s'ajoutent aux précédentes. Les branches se désignent par leur nom complet
(`feature/login`, `origin/feature/login`), les actions par leur **identifiant de
registre** (`vcs.add`, `branch.checkout`, `log.reset`…, la liste est dans
`crates/app/src/actions.rs`).

| Commande | Effet |
|---|---|
| `branches.open` / `branches.close` | Ouvre la fenêtre surgissante des branches (clic sur le widget de la barre du haut) / la ferme (ferme aussi le sous-menu). |
| `branches.query <texte>` | Tape dans le champ de recherche (correspondance floue : `fe/se`). Sans texte : efface. |
| `branches.select <branche>` | Choisit la ligne de cette branche (visible dans la liste filtrée). |
| `branches.menu [branche]` | Ouvre le sous-menu de la branche choisie (Droite / Entrée au clavier). |
| `menu.open <zone> <élément>` | Ouvre un menu contextuel : `tree <chemin relatif ou .>`, `changes <fichier>[,<fichier>…]` (chemins Git), `tab <numéro ou fragment de chemin>`, `log <n>` (commit n du journal, base 1), `branch <nom>`. Échec si aucune entrée n'a de sens. |
| `menu.expand <clé>` | Ouvre un sous-menu (`sub.git`, `sub.copy`, `sub.ignore`). |
| `menu.choose <clé d'action>` | Choisit l'entrée (sous-menus compris) comme un clic. **Échec avec la raison du grisé** si l'entrée est grisée. |
| `menu.close` | Ferme le menu (Échap). |
| `dialog.set <champ> <valeur>` | Règle un champ du dialogue Git ouvert : `name` (créer, renommer), `switch`, `remember`, `remote`, `branch`, `follow_tags`, `commit`, `also_remote`, `stash_first`, `mode` (`soft`, `mixed`, `hard`), `upstream` (dialogue « Choisir l'amont » : `origin/x`), `file` (liste « diff avec l'arbre de travail » : chemin relatif). |
| `dialog.choose <clé>` | Presse un bouton : `create`, `rename`, `stash` (« Mettre de côté et basculer / fusionner / rebaser / reprendre le commit / annuler le commit », selon l'opération refusée), `force`, `force_confirm`, `merge`, `rebase`, `push`, `merge_push`, `rebase_push`, `delete`, `force_delete`, `delete_remote`, `reset`, `discard`, `checkout`, `use_suggestion`, `terminal`, `push_new`, `back`, `create_first` / `anyway` (HEAD détachée : « Créer une branche d'abord » / « Basculer quand même »), `choose_upstream` / `remove_upstream` (message « pas d'amont » ou « amont supprimé »), `set_upstream` (dialogue « Choisir l'amont »), `open_diff` (liste « diff avec l'arbre de travail »), `cancel`, `close`. Échec si le bouton n'existe pas ou est grisé. |
| `toast.action` | Presse le bouton de la notification : « Restaurer » une branche supprimée (avec son amont), « Rétablir » une copie de sécurité (après « Annuler les modifications », une bascule forcée ou « tout jeter »), « Créer une branche » sur un commit laissé sur aucune branche. |
| `op.continue` / `op.skip` | « Continuer » (rebase, cherry-pick, revert ; échoue s'il reste des conflits) / « Passer ce commit » (ouvre la confirmation). `git.abort` abandonne. |
| `net.cancel` | La croix d'annulation de la barre d'état : annule l'opération réseau en cours. |
| `git.banner stash\|branch` | Presse le bouton d'un bandeau de la fenêtre Git : `stash` = « Remettre mes changements » (ou « Remettre » pour une mise de côté gardée), `branch` = « Créer une branche ici » (HEAD détachée avec des commits sur aucune branche). Échec si le bandeau n'a pas ce bouton. |
| `git.file-history <chemin relatif>` | « Historique du fichier » : journal filtré sur ce chemin. |
| `git.log-filter <champ> <valeur>` | Filtre du journal : `text`, `author`, `since`, `until` (`AAAA-MM-JJ`), `branch`, `clear`. |

Raccourcis réels jouables par `key` : `Ctrl+K` (valider), `Alt+0`, `Ctrl+Maj+F10`,
`Ctrl+F2`, `Ctrl+F5`, et dans la fusion `F7`, `Maj+F7`, `Alt+Left`, `Alt+Right`,
`Alt+Shift+Left`, `Alt+Shift+Right`, `Ctrl+Enter`, `Ctrl+Z`, `Escape`. Actions de
registre ajoutées : `vcs.commit`, `vcs.refresh`, `view.commit`, `view.bottom_tall`
(agrandit la fenêtre du bas : 62 % de la hauteur, utile pour photographier un diff),
`tests.run`, `tests.stop`, `tests.rerun`.

## L'état JSON (`state`)

Clés principales (toutes les listes sont en ordre d'affichage) :

- `title`, `dark`, `pixels_per_point`, `window` (taille en points), `focus`
  (`editor`, `tree`, `terminal`, `none`).
- `project` : `root`, `name`, `index_ready`, `git` (`unknown`, `no_repository`, ou
  `{branch, changed_files, upstream}`).
- `tabs` : par onglet `title`, `path`, `active`, `modified`, `line`, `column`
  (base 1), `notice` (`Changed` ou `Deleted` : bandeau affiché), `read_only`,
  `language`, `lines`, `first_line`. `recent` : fichiers récents (le plus récent
  d'abord).
- `tree` : `visible` (lignes affichées, indentées, `/` après un dossier), `expanded`,
  `selected`.
- `tools` : `project` (arbre visible), `bottom_open`, `bottom` (`git`, `tests`,
  `search`, `terminal`).
- `goto` (nul si fermée) : `query`, `results` (chemins relatifs), `selected`,
  `total`, `index_complete`, `position` (`ligne:colonne` du suffixe `:12`).
- `find` : `open`, `details` (`pattern`, options, `error`, `summary` avec les
  compteurs, `text` = la ligne de résumé affichée, `files`, `first_lines` = les 60
  premiers résultats `chemin:ligne: texte`, `selected`, `preview` = lignes de
  l'aperçu).
- `palette` (nulle si fermée) : `query`, `matches` (identifiants d'actions).
- `dialog` : `close_tab:<chemin>` ou `quit:<n>` quand une question est posée.
- `status` : ce que dit la barre d'état (`file`, `modified`, `position`,
  `encoding`, `line_ending`, `branch`, `ahead_behind`, `tasks` = tâches de fond
  affichées, `message`). `tasks_busy` : clés des tâches de fond en cours, même
  celles que la barre cache (moins de 400 ms). `status.operation` : la pastille
  « Fusion en cours — n fichiers en conflit » quand une opération Git est ouverte.

- `git` : `tab`, `files` (par fichier : `path`, `kind`, `group`, `conflict`, `checked`,
  `selected`, `merge_check` = vérification d'avance de « Fusionner… » : `Loading`, `Fine`,
  `Refused(Binary|NotUtf8|TooLarge|…)` ou `Unknown` ; `merge_disabled` = la raison affichée en
  infobulle quand le bouton est grisé), `operation` (`Merge`…), `message`, `amend`, `commit_pending`,
  `commit_error` (sortie de Git telle quelle), `confirm`, `preview` (état du diff côte à
  côte : `title`, `hunks`, `current`, `kinds`, `notice` pour binaire / identique / fins de
  ligne, `left_title`, `right_title`), `log` (`loaded`, `has_more`, `loading`, `selected`,
  `subjects`, `files`, `message`), `console` (`commands`, `last`).
- `merge` (nul si aucun dialogue) : `file`, `counts` (`changes`, `conflicts`),
  `all_resolved`, `touched`, `current`, `question` (`SavePending`, `Discard`,
  `Overwrite`), `saving`, `error`, `eol_banner`, `help`, `titles`, `result_lines`,
  `result_text` (les 4 000 premiers caractères du résultat du centre), `blocks` (par
  bloc : `id`, `kind`, `left`, `right`, `resolved`, `ours`, `theirs`, `result` en
  [début, fin[ base 0).
- `diff_window` (nul si fermée) : même forme que `git.preview`.
- `tests` : `service`, `detection` (`ready`, `php`, `phpunit`, `configuration`, versions),
  `problem` (message d'un lancement refusé), `show_passed`, `confirm_restart`, `run`
  (nul avant la première exécution : `running`, `finished`, `outcome`, `green`,
  `command`, `counts`, `summary_text`, `progress_color`, `nodes` = lignes visibles de
  l'arbre avec `depth`, `label`, `state`, `duration_ms`, `selected`, et `selected` = le
  nœud choisi avec `message`, `has_comparison`, `console_head`, `links`).
- `tools.bottom_tall` : fenêtre du bas agrandie.

Ajouts de la v1 :

- `branch_widget` : ce qu'affiche le widget de la barre du haut (`text`, `ahead`, `behind`,
  `kind` : `Normal`, `Unborn`, `Detached`, `Operation`, `NoRepository`).
- `branches` : `current`, `detached_at`, `unborn`, `local` et `remote` (par branche : `name`,
  `current`, `upstream`, `ahead`, `behind`, `gone`, `subject`), `recent`, `remotes`.
- `branch_popup` (nul si fermée) : `query`, `selected` et `rows` (libellés dans l'ordre : `# Actions`,
  `Mettre à jour le projet…`, `# Récentes`, `feature/` pour un dossier, `main *` pour la courante,
  `[origin/x ↑1 ↓2]` pour l'amont).
- `context_menu` (nul si fermé) : `zone`, `target`, `open_sub`, `entries` (par entrée : `key`,
  `label`, `shortcut`, `enabled`, `reason` = la raison du grisé, `destructive`, `children`).
- `git_dialog` (nul si aucun) : `kind`, `title`, `fields` (propres au dialogue : `name`, `problem`,
  `suggestion`, `files`, `outgoing`, `remote_only`, `commits_leaving`…), `buttons` (`key`, `label`,
  `enabled`, `primary`, `danger`, `tip`).
- `toast` : la notification de la barre d'état (`text`, `error`, `action`).
- `update_method_session` (choix fusion / rebase mémorisé pour la session), `safety_copy` (dossier
  de la dernière copie de sécurité), `net_progress`.
- Corrections de la recette v1 : `git.pending_stashes` (mises de côté de l'IDE en attente : `message`, `commit`,
  `state` = `AwaitingOperation(Merge)`, `OperationOver`, `Kept`), `git.detached` (`head`, `orphan_commits`,
  `subjects` ; nul sur une branche), `git.banners` (`stash`, `stash_button`, `detached`, `detached_button` :
  textes et boutons des bandeaux), `git.conflict_labels` (`yours` = l'étage qui porte votre travail, `ours` /
  `theirs` = rôle, nom, commit, sujet et titre de chaque étage ; `keep_buttons` = ce que disent les boutons de
  chaque fichier en conflit), `git_dialog.fields` des dialogues `detached_left`, `set_upstream`, `diff_worktree`,
  et `toast.action` = `restore_branch:<nom>`, `restore_copy:<id>` ou `new_branch_at:<commit>`.
- `git.log.filters` et `git.log.graph` (`lanes`, `rows`, `merge_rows`, `first` : couloir du point et
  nombre de segments des 8 premières lignes) ; `status.progress` (ligne de progression réseau) et
  `status.message_action` (libellé du bouton de la notification).

Le fichier ne contient **jamais** le contenu des fichiers, seulement la première ligne
de chaque onglet ; exceptions pour la vérification des fusions : `merge.result_text`
(tronqué) et les premières lignes de la console des tests.

## Règles d'essai

- **Un projet réel (labmate_paper) s'ouvre en lecture seule** : n'y tapez ni
  n'enregistrez rien. Pour tout essai qui écrit (`file.save_all`, `type` suivi de
  `Ctrl+S`, `external-write`), travaillez sur une copie dans le dossier temporaire :

  ```powershell
  $t = "$env:TEMP\jsessai"; New-Item -ItemType Directory -Force "$t\proj" | Out-Null
  Copy-Item "C:\Program Files (x86)\AgileBio\LabCollector\www\labmate_paper\src" "$t\proj\src" -Recurse
  git -C "$t\proj" init -b main
  ```
- Ne laissez pas de `juliestorm.exe` ouvert : un scénario se termine par `quit`.
  Vérifier : `Get-Process juliestorm` ne doit rien rendre.
- Un scénario est un fichier texte : il se versionne et se rejoue (voir
  `tests\scenarios\`).

## Exemples

Ouvrir, aller au fichier, chercher, sauter à la ligne (`tests\scenarios\ouvrir-et-chercher.txt`) :

```
project C:/Program Files (x86)/AgileBio/LabCollector/www/labmate_paper
action nav.goto_file
type main
wait-idle
screenshot out/aller-au-fichier.png
key Enter
action nav.find_in_files
type import
wait-idle
key Down
key Enter
state out/apres-recherche.json
quit
```

Bandeau « changé sur le disque » (sur une copie jetable) :

```
project C:/Users/moi/AppData/Local/Temp/jsessai/proj
file src/main.js:12
type XYZ
external-write src/main.js autre contenu\nsur deux lignes\n
wait 2600
screenshot out/bandeau.png
state out/bandeau.json
quit
```

(L'IDE relit l'état des fichiers ouverts toutes les secondes tant qu'il en a, et au retour du
focus ; il relit l'état Git toutes les 1,2 s, et tout de suite quand la relève d'un fichier ouvert
détecte un changement ; en scénario la fenêtre est toujours considérée comme active.)

Dernier projet rouvert (état redirigé pour ne pas toucher à `%APPDATA%`) :

```powershell
$env:JULIESTORM_STATE_DIR = "$env:TEMP\jsessai-etat"
.\target\debug\juliestorm.exe --scenario un-projet.txt      # contient : project … / quit
.\target\debug\juliestorm.exe --scenario sans-projet.txt    # wait 800 / wait-idle / state …
```

## Dépôts et projets d'essai (Git, fusion, tests)

Rien de ceci n'écrit dans `labmate_paper` ni dans `lab` : tout se fait sur des copies
dans `%TEMP%`, que l'on supprime après l'essai. Les scripts sont des scripts bash
(Git Bash) ; ils écrasent leur dossier de destination.

| Script (`tests/fixtures/`) | Fabrique | Dossier par défaut |
|---|---|---|
| `labmate-clone.sh` | clone de `labmate_paper` en cours de fusion : conflit sur `README.md` avec quatre cas (gauche seul, droite seul, vrai conflit ligne 1, même ligne modifiée des deux côtés) | `%TEMP%\jsessai-fusion` |
| `labmate-changes.sh` | clone avec changements locaux variés (modifiés, supprimé, ajouté à l'index, non suivis, fins de ligne seules) et un crochet `commit-msg` qui refuse « REFUSE » | `%TEMP%\jsessai-git` |
| `conflits-varies.sh` | petit dépôt en fusion : texte fusionnable, fins de ligne différentes, ajouté des deux côtés, binaire, Windows-1252, supprimé par eux, blocs de hauteurs différentes | `%TEMP%\jsessai-conflits` |
| `conflit-gros.sh` | un fichier de 10 000 lignes, 400 blocs non conflictuels et 10 conflits | `%TEMP%\jsessai-gros` |
| `many-commits.sh` | 1 200 commits (journal paginé par 500) | `%TEMP%\jsessai-journal` |
| `phpunit-copy.sh` | copie de `tests/fixtures/phpunit` avec le réglage `phpunit_path` vers le PHPUnit 9.6 de `lab` | `%TEMP%\jsessai-phpunit` |
| `branches.sh <dossier> [options]` | dépôt nu **local** `distant.git` (le seul distant), clone `A` à ouvrir dans l'IDE, clone `B` (la collègue) ; options : `retard`, `avance`, `distante-seule`, `amont-supprime`, `non-fusionnee`, `stash-conflit`, `crochet`, `lent`, `tout` (voir l'en-tête du script) | `%TEMP%\jsessai-branches` |

Les scénarios écrivent leurs captures et états dans `target\essais\<nom>\` (dossier ignoré
par Git) et le dossier de projet est écrit en dur dans la première ligne `project` :
l'adapter si le nom d'utilisateur diffère. Chaque scénario de `tests\scenarios\` dit en
tête de fichier quel script lancer avant.

| Scénario | Ce qu'il joue |
|---|---|
| `git-fusion-complete.txt` | fusion complète sur le clone de labmate_paper : ouvrir, appliquer les non conflictuels, résoudre les deux conflits, enregistrer, conclure ; journal |
| `git-fusion-clavier.txt` | la fusion au clavier : F7, Alt+flèches, Alt+Maj+flèches, Ctrl+Z, frappe, Ctrl+Entrée, Échap |
| `git-fusion-courbes.txt` | blocs de hauteurs différentes, connecteurs courbes, défilement synchronisé |
| `git-fusion-disque.txt` | le fichier change sur le disque pendant la fusion : « écraser / revenir » ; fermeture sans rien écrire |
| `git-fusion-ecrans.txt` | le dialogue en 1280×720 et 1920×1080, thèmes sombre et clair, 100 % et 150 % |
| `git-fusion-gros.txt` | 10 000 lignes, 400 blocs |
| `git-conflits-varies.txt` | binaire, non UTF-8, supprimé par eux, fins de ligne, ajouté des deux côtés, confirmations |
| `git-conflit-cotes.txt` | garder la nôtre / prendre la leur (confirmation), abandonner la fusion |
| `git-changements.txt` | liste des changements, aperçu et grande fenêtre de diff, F7, fins de ligne seules, commit refusé par un crochet puis réussi, journal et diff d'un fichier du commit, console |
| `git-commit-clavier.txt` | Ctrl+K, message tapé, Ctrl+Entrée, Alt+0 |
| `git-amender.txt` | amender le dernier commit |
| `git-journal-pagine.txt` | journal de 1 200 commits, page suivante au défilement |
| `tests-phpunit.txt` | lancer, arbre en direct, échec sélectionné, comparaison attendu / obtenu, saut dans le fichier |
| `tests-prerequis.txt` | PHP, PHPUnit ou `phpunit.xml` manquants |
| `tests-arret.txt` | arrêt d'un test qui dort, PHP qui plante (`exit(1)`) |
| `ecrans-git-tests.txt` | Git et Tests en 1280×720 / 1920×1080 / plein écran, 100 % et 150 %, thème clair |
| `ecarts-m3-fusionner-grise.txt` | « Fusionner… » grisé d'avance : binaire, Windows-1252, 22 Mio, 250 000 lignes (dépôt `fusion-corpus.py limites`) ; le refus après clic reste un filet |
| `ecarts-m4-modif-externe.txt` | un fichier réécrit hors de l'IDE : états toutes les ~0,17 s ; l'heure du premier état où il figure se mesure avec la date de modification des fichiers `target/essais/ecarts/m4-*.json` |
| `ecarts-m5-regex.txt` | regex invalide dès un caractère (`(`, `)`, `[`, `*a`, `\q`), message en français ; `a` et `(` sans regex restent « au moins 2 caractères » |
| `ecarts-m6-messages.txt` | messages de la barre d'état : expiration (6 s / 12 s), effacés à l'ouverture d'un autre projet ou d'un fichier réussie |
| `ecarts-arbre-dossier-vide.txt` | un dossier vide n'est pas grisé, un dossier ignoré l'est |

### Scénarios de la v1 (branches, menus contextuels)

Tous sur le décor `branches.sh` ; chaque scénario dit en tête quelles options lui donner. Le dossier
de projet est écrit en dur (`C:/Users/WEIDIAN/AppData/Local/Temp/jsessai-ui-branches/A`) : le
script se lance avec `bash tests/fixtures/branches.sh "$TEMP/jsessai-ui-branches" <options>`, et se
**relance entre deux scénarios** (ils écrivent dans `A` et dans `distant.git`).

| Scénario | Options du décor | Ce qu'il joue |
|---|---|---|
| `branches-widget.txt` | `tout` | widget `main ↑1`, fenêtre surgissante (actions, récentes, locales, dossiers, amont) |
| `branches-sous-menu.txt` | `tout` | Récupérer (↓2, « amont supprimé »), recherche floue `fe/se`, sous-menu d'une branche |
| `branches-basculer.txt` | `tout` | bascule refusée par un changement local : dialogue, « Forcer » (confirmation, Annuler), « Mettre de côté et basculer » avec remise en conflit |
| `branches-basculer-distante.txt` | `distante-seule` | bascule sur une distante seule (locale créée avec amont), changement local qui suit |
| `branches-creer.txt` | (aucune) | nouvelle branche : nom vérifié à la frappe (espaces, dossier d'une branche, casse), création |
| `branches-mise-a-jour-push.txt` | `retard avance` | Ctrl+T (fusion ou rebase, choix mémorisé pour la session), Ctrl+Maj+K, journal linéaire |
| `branches-push-refuse.txt` | `retard avance` | push refusé (le distant a des commits), « Rebaser puis pousser » |
| `branches-push-crochet.txt` | `avance crochet` | crochet `pre-push` : sa sortie est montrée |
| `branches-push-nouvelle-force.txt` | `avance` | nouvelle branche poussée (amont posé), réinitialisation, « Forcer avec bail » |
| `branches-fusion-conflit.txt` | (aucune) | fusion en conflit, bandeau, validation (commit à deux parents) |
| `branches-rebase-continuer.txt` | (aucune) | « Basculer et rebaser » : conflit au commit 2 sur 3, résoudre, Continuer |
| `branches-rebase-abandonner.txt` | (aucune) | même rebase, « Abandonner le rebase » : la branche revient à son sommet |
| `branches-supprimer.txt` | `non-fusionnee` | suppression non fusionnée (commits perdus), Restaurer, branche distante (confirmation forte) |
| `branches-reseau-annulation.txt` | `lent` | fetch ralenti : progression dans la barre d'état, croix d'annulation |
| `journal-graphe.txt` | `tout` | journal : arbre des branches, graphe en couloirs avec fusions, filtres, historique d'un fichier |
| `menus-arbre.txt` | (aucune) | clic droit dans l'arbre sur un non suivi : Git ▸ Ajouter à Git, Retirer de l'index, entrées grisées avec raison |
| `menus-changements.txt` | (aucune) | clic droit dans la liste des changements : ajouter, annuler (confirmation, copie de sécurité, Restaurer), .gitignore |
| `menus-onglets-journal.txt` | `tout` | clic droit sur un onglet et sur un commit (cherry-pick grisé sur une fusion, copier le hash) |
| `branches-ecrans.txt` | `tout` | widget, fenêtre et dialogues en 1280×720, 1920×1080, 100 % et 150 %, thème clair |

### Corrections de la recette v1 (M1 à M5, m1 à m13)

Décors : `bash tests/fixtures/recette-v1-corrections.sh <dossier> <cas>` (construit sur `branches.sh`, dépôt nu
local, jamais de vrai distant ; cas `force`, `maj-conflit`, `ops-over`, `reset-hard`, `detachee`, `arbre`,
`pick-present`, `merge-sale`, `supprimer`, `sans-amont`, `js-php`) puis
`bash tests/fixtures/recette-v1-run.sh <dossier> <scénario>` : joue le scénario avec un `JULIESTORM_STATE_DIR` neuf
(`@A@`, `@D@`, `@OUT@` remplacés ; états et captures dans `<dossier>/out`). Chaque scénario dit son cas en tête.

| Scénario | Cas | Écart | Ce qu'il joue et ce qu'il faut voir |
|---|---|---|---|
| `recette-v1-b06-maj-conflit.txt` | `maj-conflit` | M1 | Ctrl+T, fusion en conflit : bandeau « Vos changements locaux sont mis de côté… », notification qui le dit ; `git.keep yours`, Valider : « Fusion terminée ; vos changements locaux ont été remis », `LISEZMOI.md` et `nv.txt` de retour |
| `recette-v1-b07-maj-abandon.txt` | `maj-conflit` | M1, m12 | même chose puis « Abandonner la fusion » : « Fusion abandonnée ; vos changements locaux ont été remis » (le message d'arrêt est remplacé) |
| `recette-v1-b08-maj-rebase-conflit.txt` | `maj-conflit` | M1, M5 | rebase : boutons « Garder votre version » (= votre commit) / « Prendre origin/main » ; Continuer, remise |
| `recette-v1-b09-rebase-titres.txt` | `maj-conflit` | M5 | dialogue de fusion pendant un rebase : à gauche « Votre commit — main : … », à droite « Amont — origin/main : … », aide en bandeau |
| `recette-v1-corr-m1-terminal-a.txt` puis `-b.txt` | `ops-over` | M1 | a) conflit puis fermeture ; ligne de commande : `git checkout --ours a.txt && git add a.txt && git commit --no-edit` ; b) rouvrir : bandeau « Remettre mes changements » (`OperationOver`), `git.banner stash` |
| `recette-v1-b04-force.txt` | `force` | M2 | bascule forcée : « Jetés : a.txt, login.txt (copie gardée, « Rétablir » les remet). Ont suivi : n.txt, u.txt. », `toast.action` rétablit |
| `recette-v1-b12-reset-hard.txt` | `reset-hard` | M2 | « tout jeter » sans mise de côté : fichiers jetés nommés, « Rétablir » |
| `recette-v1-b13-detachee.txt` | `detachee` | M3 | HEAD détachée : bandeau « Vos commits ne sont sur aucune branche » après un commit ; sortie vers `main` refusée par le dialogue (3 boutons) |
| `recette-v1-corr-detachee-branche.txt` | `detachee` | M3 | « Créer une branche d'abord » puis la bascule reprend |
| `recette-v1-corr-detachee-quand-meme.txt` | `detachee` | M3 | « Basculer quand même » : notification et « Créer une branche » sur le commit laissé |
| `recette-v1-b14-arbre-perime.txt` | `arbre` | M4 | l'arbre suit la bascule en moins de 2 s |
| `recette-v1-corr-merge-sale.txt` | `merge-sale` | m1 | fusion refusée : dialogue « Mettre de côté et fusionner » (plus de texte brut) |
| `recette-v1-corr-pick-present.txt` | `pick-present` | m2 | « Cherry-pick » grisé : « Déjà sur la branche courante » |
| `recette-v1-corr-supprimer.txt` | `supprimer` | m3 | « Restaurer » remet aussi l'amont (`git branch -vv`) |
| `recette-v1-corr-sans-amont.txt` | `sans-amont` | m5, amont | Ctrl+T : « pas de branche distante associée » avant toute question, « Choisir l'amont… », « Retirer l'amont » |
| `recette-v1-corr-diff-arbre.txt` | `arbre` | m4 | « Voir le diff avec l'arbre de travail » (liste puis diff) ; pas de « Basculer et rebaser » sur une distante |
| `recette-v1-corr-onglet-bascule.txt` | `arbre` | M4 | un onglet ouvert dont le fichier disparaît dans la bascule : bandeau `Deleted`, arbre à jour |
| `recette-v1-corr-reset-stash.txt` | `reset-hard` | M1 | « tout jeter » avec mise de côté : bandeau « Mise de côté gardée » + « Remettre » (ici remise en conflit, l'entrée est gardée) |
| `recette-v1-corr-ecrans-a.txt` / `-ecrans-b.txt` | `maj-conflit` / `detachee` | M1, M3, M5 | bandeaux, boutons de conflit, dialogue de fusion pendant un rebase, dialogue de la HEAD détachée : 1280×720 et plein écran, 100 % et 150 %, thème clair |
| `recette-v1-corr-nav-js-php.txt` | `js-php` | m11 | Ctrl+clic / Alt+F7 sur du JavaScript d'un `<script>` : message ; une fonction PHP saute |

Remarques de métier côté interface : `Ctrl+Maj+B` ouvre la fenêtre des branches (remplacement de Ctrl+Maj+accent grave,
action `vcs.branches_alt`, m6) ; la liste « diff avec l'arbre de travail » lit `git diff --name-status` **depuis
l'interface** (`project::spawn_diff_worktree`), faute d'appel dans `julie-vcs`. Les scénarios d'un push attendent
l'aperçu des commits sortants par `wait-idle` (0,3 à 1,6 s) et non par un délai fixe ; `menus-arbre.txt` ouvre ses
menus par `ui menu.open tree <chemin>` (plus de coordonnée codée en dur).

Copie de sécurité : « Annuler les modifications » et la bascule forcée copient d'abord les fichiers
touchés dans `<projet>/.juliestorm/annulations/AAAAMMJJ-HHMMSS/` (7 jours) ; « Restaurer » les
remet. **Solution provisoire côté `app`** (`crates/app/src/safety.rs`) : la fonction viendra dans
`julie-core`.

Les couleurs des blocs et des connecteurs viennent du thème de l'éditeur
(`theme::editor_theme`) : ajouter / supprimer / modifier / conflit ; un bloc traité
s'estompe. Les mises en couleur syntaxique arrivent d'un fil à part : mettre un `wait 300`
avant une capture où elles comptent.

### Réglages, menus contextuels restants, multi-sélection (v1 bis)

- **Redirection des réglages** : `JULIESTORM_STATE_DIR=<dossier jetable>` redirige aussi
  `settings.toml` (« Tous mes projets ») vers `<dossier>\settings.toml`. **Sans** cette variable,
  le mode scénario n'a **aucun** fichier utilisateur (ni lecture ni écriture de `%APPDATA%`) :
  la portée « Tous mes projets » est alors indisponible dans la fenêtre.
- `click <x> <y> [ctrl|shift]` et `rclick <x> <y> [ctrl|shift]` : clic avec modificateur.
- Réglages (`ui settings.*`) : `settings.open [phpunit|git|terminal|search|appearance]`,
  `settings.page <p>`, `settings.scope user|project`, `settings.filter <texte>`,
  `settings.set <section.clé> <valeur>` (portée affichée ; `
` = saut de ligne),
  `settings.unset <section.clé>`, `settings.env NOM=valeur` / `settings.env -NOM`,
  `settings.apply`, `settings.ok`, `settings.cancel`, `settings.open-file`. L'état JSON a une clé
  `settings` (`dialog` : page, portée, champs avec sondes, `command`, `blocking`, `message` ;
  `files`, `problems`, `effective`).
- Menus : `menu.open` accepte aussi `editor`, `test <libellé>`, `hit <n>`, `terminal [tab]` ;
  `tree a,b` et `git.multi a,b` font une sélection multiple. Fenêtres de l'arbre (nouveau,
  renommer, supprimer, suppression définitive) : `dialog.set name <nom>`,
  `dialog.choose create|rename|trash|forever|cancel` ; état `fs_dialog`. Case de l'annulation :
  `dialog.set delete_added on`.

| Scénario | Décor | Ce qu'il joue |
|---|---|---|
| `reglages-backoffice.txt` | `backoffice-copie.sh` | Ctrl+Alt+S : commande et versions détectées, Échap sans rien écrire |
| `reglages-ecrire.txt` | `backoffice-copie.sh` | arguments et variable écrits, guillemet non fermé refusé, retour à l'héritage, thème clair écrit |
| `menus-clics-droits.txt` | `phpunit-copy.sh` + `git init` + un non suivi `nouveau.txt` | menus de l'éditeur, de l'arbre (lancer les tests), des tests, du terminal, suppression vers la corbeille |

### Navigation dans le code (v1 ter)

Les étapes `ui nav.*` jouent de **vrais** événements de souris et de clavier, aux coordonnées d'un
caractère du fichier **actif** (`ligne:colonne` à partir de 1, comme `file`) : le fichier doit être
ouvert et la ligne visible (faire `file chemin:L:C` d'abord, qui centre la ligne).

| Commande | Effet |
|---|---|
| `nav.ctrl-click L:C` | Ctrl+clic sur le caractère : aller à la déclaration (ou usages si c'est une déclaration). |
| `nav.click L:C` / `nav.rclick L:C` | Clic gauche / droit (le clic droit place d'abord le curseur puis ouvre le menu : **attendre 1,5 s**, `wait-idle` ne couvre pas l'ouverture différée du menu). |
| `nav.hover L:C` | Pointeur sur le caractère, **Ctrl maintenu** (soulignement et main si le mot a une cible). Laisser passer 300 à 600 ms (anti-rebond 60 ms, infobulle à 500 ms). |
| `nav.hover-off` | Relâche Ctrl et sort le pointeur. |
| `nav.mouse back\|forward` | Boutons « précédent » / « suivant » de la souris. |
| `nav.choose <n\|texte>` | Choisit la n-ième ligne (base 1) de la liste surgissante, ou la première dont le nom qualifié contient le texte, puis presse Entrée. |
| `usages.select <n>` / `usages.open` | Sélectionne le n-ième usage (base 1) de l'onglet affiché / l'ouvre. |
| `usages.include on\|off` | Case « Inclure les bibliothèques » (relance la recherche). |
| `usages.tab <n>` / `usages.close` / `usages.stop` | Onglet n (base 1) ; ferme l'onglet affiché ; arrête la recherche en cours. |

Touches utiles par `key` : `Ctrl+B`, `Alt+F7`, `Ctrl+Alt+Left` / `Ctrl+Alt+Right`, `Ctrl+Alt+Down` /
`Ctrl+Alt+Up` (usage suivant / précédent), et, pendant la liste surgissante, `Up`, `Down`, `Enter`,
`Escape`, `type <filtre>`. `action view.bottom_tall` agrandit la fenêtre du bas pour photographier
l'arbre des usages.

État JSON : `nav` (`link` : mot souligné `{path, range}` ; `hover_info` : ligne de l'infobulle ;
`popup` : `title`, `guessed`, `filter`, `selected`, `rows` = « genre | nom qualifié | fichier:ligne |
code », `libraries` ; `history` : `back` / `forward` ; `indexing` : pourcentage de l'index des
symboles ou nul ; `unsaved_sent` : onglets modifiés dont le texte est connu de l'index) et `usages`
(`tabs`, chacun : `headline`, `running`, `sure`, `possible`, `files_with_sure`, `selected`, `summary`,
`footer`, `rows` = l'arbre affiché : `# groupe`, `fichier (n)`, `ligne: extrait`). Le message d'une
navigation sans cible est dans `status.message`.

Décors : `bash tests/fixtures/navigation-copie.sh` (copie du projet d'essai dans
`%TEMP%\jsessai-nav\proj`, à refaire entre deux scénarios qui écrivent) et
`bash tests/fixtures/navigation-lab-copie.sh` (quelques fichiers de `lab`, **copiés** dans
`%TEMP%\jsessai-lab\proj` : on n'ouvre jamais `lab` lui-même, le cache `.juliestorm\` y serait écrit).
`python tests/fixtures/navigation-gros.py` génère 2 000 petites classes (index froid : supprimer
leur `.juliestorm\`) pour `nav-indexation*.txt` ; sur 6 000 fichiers la construction dépasse 60 s en
version de développement et `wait-idle` rend 2. Lancer les scénarios avec un `JULIESTORM_STATE_DIR`
vide et neuf : un dossier qui garde un `last_project.txt` fait rouvrir ce projet en plus.

| Scénario | Ce qu'il joue |
|---|---|
| `nav-saut-direct.txt` | survol avec Ctrl (soulignement, infobulle, `strlen` sans lien), Ctrl+clic sur `$user->getName()` (saut direct), Ctrl+Alt+Gauche / Droite, bouton précédent de la souris, `strlen` (message) |
| `nav-liste.txt` | `$other->getName()` : liste « type inconnu » (ordre, filtre, Échap, choix au clavier), `new User('a')` : classe et constructeur |
| `nav-usages.txt` | Ctrl+clic sur une déclaration → fenêtre Usages (sûrs / possibles), Ctrl+Alt+Bas, Alt+F7 sur classe / méthode, un seul usage dans `vendor` : saut direct, case « Inclure les bibliothèques » |
| `nav-onglet-modifie.txt` | onglet modifié non enregistré pris en compte (5 puis 6 usages), annulation, enregistrement (**écrit dans la copie**) |
| `nav-js.txt` | `fd`, `formatDate` (réexport), `Api`, `api.get`, chemin `'./util'`, usages de `formatDate` |
| `nav-menu-contextuel.txt` | clic droit dans l'éditeur (curseur d'abord), « Aller à la déclaration », « Trouver les usages », Ctrl+B, `Broken.php` |
| `nav-ecrans.txt` | liste et usages à 150 %, plein écran, thème clair, survol |
| `nav-vrai-code.txt` | copie de quelques fichiers de `lab` : propriété sans type, `new`, fonction native, usages |
| `nav-indexation.txt`, `nav-indexation-message.txt` | progression « Indexation des symboles… n % » dans la barre d'état, message « Indexation en cours (n %) » |

### Éditeurs fractionnés (split view)

Spécification : `docs/specs/split.md`. Les actions du registre :

| Action | Effet | Raccourci |
|---|---|---|
| `editor.split_right` / `editor.split_down` | « Fractionner à droite / en bas » : nouveau groupe, même document, deux vues | aucun |
| `editor.move_right` / `editor.move_down` | « Déplacer à droite / en bas » : l'onglet actif change de groupe (grisé s'il est seul dans son groupe) | aucun |
| `editor.move_other` | « Déplacer vers l'autre groupe » (le suivant dans l'ordre d'affichage) | aucun |
| `editor.unsplit_all` | « Tout regrouper » : un seul groupe, chaque fichier une fois | aucun |
| `editor.next_group` / `editor.prev_group` | groupe d'éditeurs suivant / précédent | Ctrl+Alt+Page suivante / précédente |

Commandes `ui` :

| Commande | Effet |
|---|---|
| `group.activate <n>` | Le groupe n (base 1, **ordre d'affichage** : gauche à droite, haut en bas) devient actif, son éditeur a le focus. |
| `group.tab <groupe> <onglet>` | Active l'onglet (base 1) d'un groupe. |
| `layout.persist on\|off` | Sauvegarde et restauration de la disposition dans `<projet>\.juliestorm\layout.json` (**désactivées par défaut en mode scénario**, comme la mémoire du dernier projet ; à placer **avant** `project`). Hors scénario elles sont toujours actives. |

`menu.open tab <numéro>` désigne l'onglet **du groupe actif** ; `menu.open tab <fragment>` le
premier onglet dont le chemin contient le fragment, en commençant par le groupe actif (il
devient actif s'il est ailleurs, comme un vrai clic droit). Le menu d'un onglet contient les
actions ci-dessus ; la raison du grisé est dans `context_menu.entries[].reason`.

État JSON : `tabs` reste la liste des onglets **du groupe actif** (un seul groupe : comme
avant). `groups` : `count`, `active` (n du groupe actif), `depth` (niveaux d'imbrication),
`layout` (l'arbre : `{split: vertical|horizontal, ratio, a, b}` ou `{group: n}`), `restoring`,
`list` (dans l'ordre d'affichage : `index`, `active`, `rect` = `[x, y, largeur, hauteur]` en points,
`tabs` : `title`, `path`, `active`, `modified`, `views` = nombre de vues du document,
`line` / `column` **de cette vue**, `scroll_y`). Un séparateur se déplace avec `drag` : il se
repère au bord entre les `rect` de deux groupes voisins.

| Scénario | Décor | Ce qu'il joue |
|---|---|---|
| `split-base.txt` | clone jetable de `labmate_paper` (`git clone` vers `%TEMP%\jsessai-split\proj`) | critère 1 : fractionner à droite, frappe dans une vue visible dans l'autre, Ctrl+S une seule fois |
| `split-groupes.txt` | idem | critères 2, 3, 6, 7, 9 : déplacer en bas, ouvrir dans le groupe actif, fermer le dernier onglet d'un groupe, tout regrouper, fermer une vue d'un document modifié |
| `split-imbrication.txt` | idem | critères 4 et 5 : séparateur glissé à la souris, proportion gardée au redimensionnement, trois niveaux en 1280×720 et 1920×1080, 100 % et 150 % |
| `split-glisser.txt` | idem | glisser un onglet vers un bord (nouvelle division) et vers la rangée d'onglets d'un autre groupe |
| `split-persistance-a.txt`, `split-persistance-b.txt` | idem | critère 8 : construire une disposition, quitter, rouvrir le projet |
| `split-ouvertures.txt` | `bash tests/fixtures/navigation-copie.sh` | ouverture dans le groupe actif depuis la recherche dans les fichiers, la navigation (Ctrl+clic : un fichier ouvert dans l'autre groupe y est activé) et l'arbre du projet |
| `split-menus.txt` | clone jetable | menu d'un onglet (entrées grisées avec leur raison), palette (`fraction`), menu Affichage, Ctrl+Alt+Page suivante / précédente |
| `split-disque.txt` | clone jetable | fichier réécrit hors de l'IDE et montré dans deux groupes : un seul rechargement ; modifié : le bandeau est dans les deux vues |

## Ce que le mode scénario ne couvre pas

- Le **sélecteur de dossiers du système** (`file.open_folder`) : fenêtre native, hors
  de la fenêtre egui. Le reste du chemin (`project <dossier>`) est identique.
- Une **vraie** frappe du clavier ou clic de la souris passant par le pilote graphique : les étapes
  injectent les événements dans egui, après le système. Ce qu'un pilote ferait des touches
  (Ctrl+Alt+flèches de certains pilotes Intel) ne s'y voit donc pas.
- La molette et les infobulles (survol) : pas d'étape correspondante ; le glisser-déposer
  n'a que `drag` / `release` (séparateurs, onglets), pas de dépôt de fichiers venant du système.
  `click` couvre le clic gauche, `rclick` le clic droit (les raisons des entrées
  grisées, normalement en infobulle, se lisent dans `context_menu.entries[].reason`). Le défilement des listes se
  joue au clavier (Fin, Début, pages dans le journal) et celui de la fusion par
  `merge.focus <ligne>` ou F7.
- Les clics sur les boutons des gouttières de la fusion, les cases à cocher et les
  boutons de la fenêtre Git passent par les mêmes fonctions que les commandes `ui`
  (une commande `ui` et un clic font la même chose) ; seul le **dessin** de ces
  boutons ne se vérifie que sur capture.
- Le rendu exact de la fenêtre dépend de la carte graphique (DirectX 12 imposé) ;
  les captures de la testeuse et de la développeuse peuvent différer d'un pixel.
