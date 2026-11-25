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
| `file <chemin>[:ligne[:colonne]]` | Ouvre un fichier (relatif à la racine du projet) à la ligne donnée (base 1), puis attend. |
| `action <identifiant>` | Déclenche une action du registre (voir la liste ci-dessous). Échec si elle est indisponible dans l'état courant (grisée). |
| `type <texte>` | Saisit le texte dans le champ qui a le focus (éditeur, champ d'une fenêtre flottante, terminal). Le texte est celui de la ligne, après le premier espace. |
| `key <combinaison>` | Appuie sur une touche : `Enter`, `Escape`, `Tab`, `Down`/`Up`/`Left`/`Right` (ou `Bas`, `Haut`, `Gauche`, `Droite`), `F4`…, une lettre, un chiffre ; avec `Ctrl+`, `Alt+`, `Shift+` (ou `Maj+`). Ex. `key Ctrl+Maj+N`, `key Alt+Right`, `key Ctrl+A`. |
| `click <x> <y>` | Clic gauche aux coordonnées (points, origine en haut à gauche de la fenêtre). |
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
| `git.keep ours\|theirs [fichier]` | « Garder la nôtre » / « Prendre la leur » (ouvre la confirmation ; sans fichier : tous les conflits). |
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

Les couleurs des blocs et des connecteurs viennent du thème de l'éditeur
(`theme::editor_theme`) : ajouter / supprimer / modifier / conflit ; un bloc traité
s'estompe. Les mises en couleur syntaxique arrivent d'un fil à part : mettre un `wait 300`
avant une capture où elles comptent.

## Ce que le mode scénario ne couvre pas

- Le **sélecteur de dossiers du système** (`file.open_folder`) : fenêtre native, hors
  de la fenêtre egui. Le reste du chemin (`project <dossier>`) est identique.
- Le glisser-déposer, le clic droit, la molette, les infobulles (survol) : pas
  d'étape correspondante. `click` couvre le clic gauche. Le défilement des listes se
  joue au clavier (Fin, Début, pages dans le journal) et celui de la fusion par
  `merge.focus <ligne>` ou F7.
- Les clics sur les boutons des gouttières de la fusion, les cases à cocher et les
  boutons de la fenêtre Git passent par les mêmes fonctions que les commandes `ui`
  (une commande `ui` et un clic font la même chose) ; seul le **dessin** de ces
  boutons ne se vérifie que sur capture.
- Le rendu exact de la fenêtre dépend de la carte graphique (DirectX 12 imposé) ;
  les captures de la testeuse et de la développeuse peuvent différer d'un pixel.
