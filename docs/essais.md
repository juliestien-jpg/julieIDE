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
  celles que la barre cache (moins de 400 ms).

Le fichier ne contient **jamais** le contenu des fichiers, seulement la première ligne
de chaque onglet.

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

(L'IDE relit l'état des fichiers ouverts toutes les 2 s tant qu'il en a, et au retour du
focus ; en scénario la fenêtre est toujours considérée comme active.)

Dernier projet rouvert (état redirigé pour ne pas toucher à `%APPDATA%`) :

```powershell
$env:JULIESTORM_STATE_DIR = "$env:TEMP\jsessai-etat"
.\target\debug\juliestorm.exe --scenario un-projet.txt      # contient : project … / quit
.\target\debug\juliestorm.exe --scenario sans-projet.txt    # wait 800 / wait-idle / state …
```

## Ce que le mode scénario ne couvre pas

- Le **sélecteur de dossiers du système** (`file.open_folder`) : fenêtre native, hors
  de la fenêtre egui. Le reste du chemin (`project <dossier>`) est identique.
- Le glisser-déposer, le clic droit, la molette, les infobulles (survol) : pas
  d'étape correspondante. `click` couvre le clic gauche.
- Le rendu exact de la fenêtre dépend de la carte graphique (DirectX 12 imposé) ;
  les captures de la testeuse et de la développeuse peuvent différer d'un pixel.
