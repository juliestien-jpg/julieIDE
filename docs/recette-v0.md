# Recette de la v0

Faite le 24/11/2026 par la testeuse, sur `target\debug\juliestorm.exe` (build de développement, DirectX 12), Windows 11,
`git` 2.56 (le `core.autocrlf` du système est **true** : les clones jetables ont des fichiers de travail en CRLF).
Tout s'est joué par le mode scénario (`docs/essais.md`) dans des dossiers jetables de `%TEMP%`, supprimés à la fin.
Aucun commit, aucun push, aucune modification du code de l'application.

Outillage ajouté (dans `tests/` uniquement) : `tests/fixtures/fusion-corpus.py` (38 cas de fusion posés directement dans
l'index, plus `limites`), `tests/fixtures/fusion-reel.py` + `tests/fixtures/mergecli/` (mesure des conflits en trop sur du
vrai code), `tests/fixtures/recette-projets.py` (projets d'essai édition / recherche / terminal / robustesse) et seize
scénarios `tests/scenarios/recette-*.txt`.

## Verdict en une phrase

La v0 tient sa promesse sur les trois piliers et le terminal : **aucun bloquant, aucun conflit réel résolu en silence,
aucun octet altéré hors des blocs touchés, aucun processus orphelin.** Restent quelques écarts à la spécification et
une famille de faux conflits dans la fusion (côté sûr), listés ci-dessous.

## Chiffres tenus

| Contrôle | Résultat |
|---|---|
| `cargo test --workspace` | **432 verts, 1 rouge** : `julie-terminal` `sessions::ctrl_c_interrupts_a_long_command` (voir bug M2 : rouge uniquement quand la commande est lancée depuis le Git Bash de l'outil d'agent ; vert depuis PowerShell) |
| `cargo clippy --workspace --all-targets -- -D warnings` | propre |
| Corpus de fusion maison, 38 cas × 4 modes (appliquer + enregistrer ; puis accepter gauche / droite / les deux sur chaque conflit restant), jusqu'à l'enregistrement, fichier écrit comparé à `git merge-file` | 33 à 34 / 38 identiques à Git en octets selon le mode ; les 5 écarts expliqués ci-dessous (3 voulus par la spec : CRLF contre LF, saut final, BOM ; 2 faux conflits) ; **0 conflit réel appliqué en silence** |
| Conflits en trop sur du vrai code | **14 sur 2 209 fusions que Git fait proprement = 0,63 %** (détail en (d)) |
| Processus restants après chaque essai (juliestorm, php, bash, cmd, conhost, sleep, ping, git) | aucun (instantanés avant / après, y compris IDE quitté avec `sleep 100` actif et PHPUnit qui dort 30 s) |
| `labmate_paper` | rendu comme trouvé (voir « État de labmate_paper ») |

## (a) Vu fonctionner

### Fusion (le plus lourd)
- **Octet pour octet** : corpus de 38 conflits fabriqués (disjoints, même ligne, ajouts / modifs identiques, supprimé contre modifié, supprimé des deux côtés, CRLF contre LF, CRLF partout, fichier à fins mixtes, sans saut final (LF et CRLF), BOM des trois côtés / chez nous seul / chez eux seul, accents + CJK + emoji, 3 000 lignes avec ~50 modifs par côté et 3 conflits, lignes contiguës, insertions au même point, insertion contre modification, espaces seuls, espace final, ajouté des deux côtés, fichier vidé, lignes répétées, CR isolé, réindentation contre édition, ligne de 200 000 caractères, marqueurs `<<<<<<<` dans le texte, nom de fichier avec espaces et accent, suppressions disjointes / chevauchantes, PHP). Dans chaque mode, après `merge.save` : le fichier écrit est le texte attendu, `git ls-files -u` ne le liste plus, aucun octet hors blocs touché (CRLF / LF ligne par ligne, BOM de « notre » version, absence de saut final conservée).
- **Jamais de conflit réel résolu en silence** : sur les 38 cas, tout ce que Git déclare en conflit reste en conflit chez nous après « Appliquer tous » (le centre garde le texte de la base). Sur 2 209 fusions de vrai code (en (d)) : 0 cas où Git signale un conflit et où notre moteur fusionne proprement.
- Critères d'acceptation de `fusion.md` : 2 (nôtre à gauche, base au centre, leur à droite ; frappe refusée sur les deux côtés : essai par clic + frappe, résultat et « touched » inchangés), 3, 4 (« tous / de gauche / de droite » ne touchent aucun conflit), 5 (accepter, annuler, ignorer, ajouter à la suite dans l'ordre des clics, frappe puis Ctrl+Z, compteurs à jour), 7 (F7 / Maj+F7 : bloc 0 → 1 → 2 → 1), 8 (« Revenir » n'écrit rien), 9, 10 (fermeture après un geste : question, puis disque identique octet pour octet : SHA-256 avant / après identique, fichier toujours en conflit), 11 (binaire, non UTF-8, > 20 Mio, > 200 000 lignes : pas de dialogue, message « Choisissez un côté »), 12 (10 000 lignes / 400 blocs : ouvert en ~0,7 s, « appliquer tous » en ~0,2 s, en build debug), 13 (bandeau CRLF présent pour le cas 15, absent pour le 14).
- Fichier changé sur le disque pendant la fusion (`git-fusion-disque`) : demande écraser / revenir, rien de perdu.
- Fusion sur du vrai code (clone jetable de `labmate_paper`, branches `juliestorm-essai/nous` et `/eux`, `git-fusion-complete`) : appliquer les non conflictuels, résoudre les deux conflits, enregistrer, conclure : `git log --graph` montre un commit à deux parents `3d84b5f` + `e6288ef`, message `Merge branch 'juliestorm-essai/eux' into juliestorm-essai/nous` (celui de `MERGE_MSG`), contenu de `README.md` conforme aux choix. Avec `core.autocrlf=true` (fichier de travail CRLF, index LF) le fichier écrit reste CRLF et l'index LF : correct.
- Conflits Git : « Prendre la leur » : fichier égal à l'étape 3 (SHA identique) et plus dans `--diff-filter=U` ; « Garder la nôtre » sur un binaire : octets de l'étape 2 ; supprimé par eux : confirmation puis fichier supprimé et enregistré ; après le dernier conflit, `Valider` crée la fusion à deux parents (git.md 9, 10, 11).

### Git (hors fusion)
- Critère 1 (modifié, supprimé, ajouté à l'index, deux non suivis : groupes et nombres exacts, identiques à `git status --porcelain`), 3 (HEAD à gauche, travail à droite, F7, « 3 différences »), 4 (« Seules les fins de ligne diffèrent (LF → CRLF) »), 5 (2 fichiers cochés sur 3 : `git show --stat HEAD` liste exactement ces fichiers ; le staged non coché reste `A ` dans l'index, le supprimé reste ` D`), 6 (non suivi coché inclus), 7 (crochet `commit-msg` qui refuse : pas de commit, message conservé, sortie du crochet visible), 8 (journal dans l'ordre de `git log`, message + fichiers), 12 (la Console liste chaque commande : `status`, `add`, `commit -F - --only --`, `rev-parse`, `log`, `ls-tree`, `cat-file`…, avec durées).
- Dépôt sans commit : liste, case à cocher et premier commit fonctionnent (`git log` : un commit, 2 fichiers, chemin avec espace et accent). Dossier sans dépôt : « Ce dossier n'est pas un dépôt Git. ». Chemin `jsessai term (x86) é` : tout fonctionne.
- Ctrl+D, F4, Entrée dans la liste des changements ouvrent diff / fichier (après un clic qui donne le focus ; `ui git.select` seul ne le donne pas, limite du scénario).

### Éditeur
- Critères 1, 2 (deux ouvertures du même fichier = un onglet), 3 (HTML + PHP + `<style>` + `<script>` : trois colorations, vu sur capture), 4 (« abc » annulé d'un trait, rétabli par Ctrl+Maj+Z), 5 (Ctrl+Y), 6 (CRLF sans BOM, LF avec BOM, mixte, sans saut final LF et CRLF, vide, avec tabulation : après une frappe, `cmp` avec l'original : seule la ligne touchée change), 7 (`café déjà` en Windows-1252 : affiché, réenregistré avec `0xE9` et `0xE7` pour un « ç » tapé), 8, 9 (Échap sur la question : onglet gardé et modifié), 10 (fichier non modifié changé dehors : contenu mis à jour en ≤ 1,5 s), 11 (onglet modifié + changement externe : bandeau « Le fichier a changé sur le disque » avec Recharger / Garder les miennes, rien perdu), 12 (PHP de 10 000 lignes ouvert en < 0,5 s), 13 (lecture seule : message d'erreur, onglet reste modifié).

### Recherche
- Aller au fichier : `usct` → `src/Controller/UserController.php` seul ; champ vide → les 3 fichiers ouverts, le dernier en haut ; `index.php:12` → onglet à la ligne 12 ; `ctrl/user` ; nom accentué ; aucun résultat ; fichiers ignorés par `.gitignore` (dossier, motif `*.log`, `vendor/`) absents ; non suivis non ignorés présents.
- Rechercher dans les fichiers : `function` = 11 occurrences / 5 fichiers (= `git grep -o -i --untracked function | wc -l`, et 8 avec « mots entiers » = `git grep -o -w`) ; casse (`Foo` : 2) ; regex `\$\w+Id\b` (6, vérifié à la main) ; `foo$` avant un CRLF ; invalide `(a` : « Expression régulière invalide » sous le champ, pas d'erreur fenêtre ; masque `*.js` / `*.php` ; arrêt à 10 000 avec le message ; binaire et 12 Mio ignorés et comptés (« 1 fichier trop gros ignoré ») ; fichier Windows-1252 décodé ; texte tapé dans un onglet non enregistré trouvé (aperçu compris) ; requêtes enchaînées vite : seul le dernier résultat reste ; Entrée ouvre à l'occurrence sélectionnée.

### PHPUnit
- Sur la copie de `tests/fixtures/phpunit` : la commande exacte est en tête de la console ; compteurs **identiques à PHPUnit en ligne de commande** (13 tests : 3 échecs, 1 erreur, 1 ignoré, 1 incomplet, 1 risqué, 6 réussis) ; noeuds en direct (5 réussis déjà visibles à 0,4 s) ; jeu de données à 3 noms dont `l'apostrophe | la barre [et] les crochets` ; échec déplié, console avec message, attendu / obtenu et lien `CalculatorTest.php:25` qui ouvre le bon fichier à la ligne 25 (même ligne que PHPUnit).
- `exit(1)` : « Exécution interrompue (code de sortie 1) », erreur sur le test en cours, 3 annoncés / 2 démarrés, pas de test inventé, l'IDE vit. Arrêt pendant un test de 30 s : le processus `php` disparaît en moins de 0,4 s (échantillonnage toutes les 250 ms). Relance pendant une exécution : confirmation. Sans `vendor/bin/phpunit`, sans `phpunit.xml`, avec un chemin de PHP faux : message nommant le chemin, aucun processus lancé.

### Terminal
- Alt+F12 ouvre Git Bash par défaut dans le dossier du projet, y compris `…\jsessai term (x86) é` (MSYS : `/tmp/jsessai term (x86) é`) ; trois onglets (Git Bash, Git Bash, cmd) indépendants : une commande longue dans l'un n'empêche pas l'autre (`sleep 4; echo` terminé pendant qu'on tape ailleurs).
- Couleurs 16 / 256 / 24 bits, gras, souligné, inverse, fond : vus sur capture. `tput cols` = 156 ; `mode con` 12 → 23 lignes après `view.bottom_tall`. `echo é` : « é » dans Git Bash et dans cmd. Ctrl+C sans sélection interrompt `PING -t` (cmd lancé depuis PowerShell). Ctrl+Maj+V colle (`echo colle-ok > …` exécuté). `less` + `q`, `vim` (`i`, texte, Échap, `:wq`) : fichier écrit, écran propre ; Ctrl+R atteint le shell (historique) ; Échap va au shell. `exit 3` : onglet gardé, « Processus terminé (code 3) » + Relancer.
- Fermer l'onglet (Ctrl+F4) ou l'IDE (`file.quit`) pendant `sleep 100` : confirmation « Un programme tourne encore… » ; après `quit`, aucun `sleep.exe` / `bash.exe` / `cmd.exe` / `ping.exe` orphelin. Chemin de Git Bash faux (`C:\nope\bash.exe`) : démarrage sur cmd, entrée « Git Bash » grisée, pas de plantage.

### Raccourcis
- Joués et observés : Ctrl+Maj+N, Ctrl+Maj+F, Ctrl+Maj+A (palette), Ctrl+S, Ctrl+F4, Ctrl+Tab (retour à l'onglet précédent), Alt+Gauche / Droite (onglets), Alt+1 / Alt+9 / Alt+4 / Alt+F12 / Alt+0, Ctrl+K, Ctrl+D / F4 / Entrée (changements), F7 / Maj+F7, Échap, Ctrl+Z / Ctrl+Maj+Z / Ctrl+Y, Alt+C / Alt+W / Alt+X, Ctrl+Maj+T, Ctrl+Maj+V ; dans le terminal Échap et Ctrl+R vont au shell, Alt+F12 reste à l'IDE. Critère 3 (pas de doublon) : test unitaire `no_two_actions_share_a_shortcut` vert. Critère 4 : les menus Fichier, Affichage, Git affichent le raccourci en face de l'action.

### Robustesse
- Binaire (1 Mo aléatoire) : refusé avec message. Fichier de 150 Mo : « Fichier trop gros ». 1 million de lignes (12,9 Mo) : ouvert, saut à la ligne 500 000 immédiat. Ligne unique de 5 Mo : ouverte. Dépôt sans commit, dossier sans Git, projet sans `phpunit.xml`, chemin avec espaces, parenthèses et accents : sans panique. Aucune panique sur l'ensemble des ~60 lancements.

## (b) Bugs et écarts

Aucun bloquant. Aucun majeur de perte de données ou de faux résultat. Mineurs, classés par intérêt.

### M1 — Faux conflits de fusion sur du code courant (mineur, côté sûr ; fusion-diff)
- **Étapes** : `python tests\fixtures\fusion-corpus.py build %TEMP%\jsessai-corpus`, ouvrir le dossier, Git, `Fusionner…` sur `38-php.php`. Base :
  `<?php / class A { / public function a() { return 1; } / (ligne vide) / public function b() { return 2; } / }`. Nôtre : `a` → `return 10`. Leur : `b` → `return 20` **et** une méthode `c()` ajoutée à la fin de la classe.
- **Attendu** : `a` et `b` fusionnés, `c` ajoutée (c'est ce que fait `git merge-file`, sans conflit) ; ou au pire un conflit sur le seul bloc `b`/`c`.
- **Observé** : un seul bloc conflictuel qui couvre `a`, la ligne vide et `b` (`ours [2,5)`, `theirs [2,7)`) ; « Tous » n'applique rien ; le texte repris est celui de la base, le travail des deux côtés est à refaire à la main. Même chose, en plus discret, sur `25-lignes-repetees.txt` (changements aux lignes 3 et 5 d'un fichier de lignes identiques).
- **Cause probable** : l'alignement du diff (Myers) décale le changement de « leur » côté d'une ligne vers le haut (la ligne vide de la base est rapprochée de la ligne vide de la méthode ajoutée), il touche alors le changement de « notre » côté, et la règle « contigus = un seul bloc » (`fusion.md`, règle 2) en fait un conflit. Git (xdiff) a une heuristique de glissement des changements.
- **Preuve** : `etat-classify/38-ouvert.json` (bloc unique) ; `git merge-file` : code de sortie 0 ; mesure en (d). Rien n'est perdu ni écrit de travers : c'est le côté « dans le doute, conflit ».
- **Métier** : `dev-fusion-diff`.

### M2 — Ctrl+C n'interrompt plus rien quand l'IDE est lancé par un parent qui ignore Ctrl+C (mineur, dépend du lanceur ; terminal)
- **Étapes** : lancer `target\debug\juliestorm.exe --scenario tests\scenarios\recette-terminal-cmd.txt` depuis le Git Bash de l'outil d'agent (ou `cargo test -p julie-terminal --test sessions ctrl_c` depuis ce même shell), `PING -t 127.0.0.1` dans l'onglet cmd, Ctrl+C.
- **Attendu** : le ping s'arrête (critère terminal 5). **Observé** : il continue (capture), et `ctrl_c_interrupts_a_long_command` échoue 3 fois sur 3 (« ping devrait être interrompu »). Lancé depuis PowerShell : le ping s'arrête, le test passe en 0,13 s.
- **Cause probable** : l'état « ignorer Ctrl+C » est hérité par les enfants de la pseudo-console ; l'IDE ne le remet pas à zéro (`SetConsoleCtrlHandler(NULL, FALSE)` au démarrage ou à l'ouverture d'une session).
- **Métier** : `dev-terminal`. Gravité faible tant qu'on lance l'IDE depuis un shell normal, mais le test automatisé est rouge dans cet environnement.

### M3 — « Fusionner… » n'est jamais grisé (mineur, écart à `git.md` / `fusion.md`)
- **Étapes** : dépôt en conflit avec un binaire, un fichier Windows-1252, un fichier de 22 Mio (`fusion-corpus.py limites`). **Attendu** : bouton désactivé avec la raison en infobulle (git.md, « Liste des fichiers en conflit »). **Observé** : bouton actif pour les cinq fichiers ; le clic affiche seulement « Fusion de … impossible : fichier trop gros… » dans la barre d'état (message correct, mais discret). **Métier** : `dev-interface` / `dev-git`.

### M4 — Un fichier modifié hors de l'IDE apparaît dans Git en 1,8 à 2,3 s (mineur, git.md critère 2 : « moins de 2 s »)
- `external-write` d'un fichier suivi puis états toutes les 0,45 s : absent à +1,82 s, présent à +2,27 s. Le même délai pour un onglet ouvert est ≤ 1,5 s. **Métier** : `dev-git` (période de relecture).

### M5 — Recherche : « ( » seul n'affiche pas l'erreur d'expression (mineur, recherche.md critère 8)
- Regex activée, taper `(` : le message est « Saisissez au moins 2 caractères. » ; l'erreur n'apparaît qu'à `(a`. Le texte d'erreur est mi-anglais : « Expression régulière invalide : unclosed group » (message de la crate `regex` non traduit). **Métier** : `dev-indexation-recherche` / `redacteur`.

### M6 — Messages de la barre d'état qui ne s'effacent pas (cosmétique)
- « 1 commit : premier commit » reste affiché après l'ouverture d'un autre projet ; l'erreur rouge « …\bin.dat : Fichier binaire » reste après l'ouverture réussie de trois autres fichiers. **Métier** : `dev-interface`.

### M7 — La BOM de « leur » version est perdue sans le dire (mineur)
- Base et nôtre sans BOM, leur version avec BOM et une modification : résultat sans BOM, aucun bandeau (la spec dit que le résultat suit « notre » version, et ne prévoit un bandeau que pour les fins de ligne). Git, lui, déclare un conflit. Cas rare ; à décider (bandeau comme pour CRLF ?). **Métier** : `specialiste-produit`.

### Observations (rien à corriger forcément)
- Critère `recherche.md` 10 contradictoire avec la section « cas limites » : le compteur compte les **occurrences** (= `git grep -o`), pas les lignes (= `git grep -c`) ; l'écart n'existe que si une ligne contient deux occurrences. À trancher dans la spec.
- Défilement synchronisé de la fusion (`fusion.md` 6) : les trois volets défilent ensemble et sont reliés par des courbes, mais un même bloc n'est pas à la même ordonnée dans les trois quand les hauteurs diffèrent (capture `courbes/04`, 67 px d'écart sur un bloc supprimé de 4 lignes) : il n'y a pas de remplissage. Compatible avec « reliés par des courbes » ; pas avec une lecture stricte de « alignés ».
- Le menu « + » du terminal propose aussi **PowerShell** (hors v0 selon `terminal.md`) ; utile, mais à entériner dans la spec.
- Le dossier vide `dd` et `.cas` s'affichent grisés dans l'arbre (comme les dossiers ignorés) ; sans doute voulu pour les dossiers vides, non documenté.
- Les tests ignorés et incomplets ont le même point gris dans l'arbre PHPUnit.
- 720 px de haut : « agrandir la fenêtre du bas » ne laisse plus que ~4 lignes à l'éditeur ; la liste des conflits ne montre que 4 fichiers sans défiler. Confortable à 1080 px.
- Cette machine n'a **pas `C:\Windows\System32` dans son `PATH`** (ni utilisateur ni machine) : dans le cmd de l'IDE, `ping`, `mode`, `where` sont « non reconnus » ; ce n'est pas un défaut de l'IDE (même chose dans tout `cmd` ici), mais il faut les appeler par `%SystemRoot%\System32\…` pour les essais.

## (c) Non testé, et pourquoi

- **Souris** : sélection par glisser, double / triple clic, clic droit (menu Copier / Coller / Effacer du terminal), molette, infobulles : le mode scénario n'a que `click`. Donc terminal critère 5 (sélection puis Ctrl+C copie) non vérifié ; seul « sans sélection, Ctrl+C interrompt » l'est.
- **Ctrl+V nu dans le terminal** : le presse-papiers arrive par `Event::Paste` généré par la couche fenêtre à partir d'un vrai clavier ; le scénario n'envoie qu'un `Event::Key`, que le terminal traite comme Ctrl+V (octet 0x16, qui a mis bash en « quoted-insert »). Ctrl+Maj+V fonctionne. À vérifier à la main sur un vrai clavier.
- **Lettres via `key` dans le terminal** (`key q`) : même limite (pas d'`Event::Text`) ; j'ai utilisé `type`.
- Git critère 13 (interface libre pendant un `git status` de 3 s) : il faudrait un `git` ralenti ; je n'ai pas modifié la configuration de la machine. Recherche critère 13 (5 s de recherche) et fluidité à 5 000 tests / 5 000 fichiers non suivis / 50 000 fichiers : non mesurés.
- Fermeture par la croix de la fenêtre (le scénario passe par `file.quit` / `quit`) ; fermeture avec fichiers modifiés **et** terminal actif : j'ai vu la question des fichiers (Tout enregistrer / Ne pas enregistrer / Annuler) ; je n'ai pas vu si la question du terminal suit.
- Ctrl+Alt+Y et Ctrl+F5 : touches acceptées, effet non observable par l'état (rien à relancer / à relire de visible).
- Comparaison avec PhpStorm : non faite (aucun comportement douteux assez précis pour justifier d'ouvrir PhpStorm).
- Fusion : cas des liens symboliques et sous-modules (« choix de côté seulement ») ; rebase / cherry-pick (ligne d'aide) ; Windows-1252 dans le dialogue (refusé, comme prévu).
- `labmate_paper` en conflit sur de vraies branches : seules les deux branches d'essai existantes ont été rejouées (un seul fichier, `README.md`).

## (d) Conflits en trop sur du vrai code

Méthode : un petit exécutable de recette (`tests/fixtures/mergecli/`) appelle `julie_diff::merge` (celui du bouton « Appliquer tous ») sur des triplets base / nôtre / leur ; Git : `git merge-file -p ours base theirs` sur les mêmes trois fichiers. « Conflit en trop » = Git fusionne proprement (code 0) et notre moteur laisse au moins un conflit. Comparaison du texte fusionné quand les deux sont propres.

L'historique de `labmate_paper` n'a que **10 commits en ligne droite, aucune fusion** : on ne peut pas rejouer de commits de fusion. Faute de mieux : (1) des « cherry-picks » réels (le correctif du commit *j* rejoué sur l'état d'avant le commit *i* grâce à Git, base = parent de *i*, nôtre = *i*, leur = ce résultat), et (2) des éditions simulées sur du code réel (JavaScript de `labmate_paper/src` : 71 fichiers ; PHP : `Framework` de PHPUnit 9.6, copie en lecture seule), 1, 2, 3, 5 ou 8 modifications aléatoires de chaque côté (modifier une ligne, insérer 1 à 3 lignes, supprimer 1 à 3 lignes), ou, dans la variante « blocs », l'insertion d'un petit bloc de code (ligne vide, signature, accolades).

| Corpus | Fusions propres pour Git | Conflits en trop chez nous | Taux | Conflit réel fusionné chez nous seulement | Texte différent quand les deux sont propres |
|---|---|---|---|---|---|
| Vrais correctifs de l'historique (cherry-picks, `labmate_paper`) | 39 | 1 (`src/core/audio.js` : deux insertions de ~10 lignes à 6 lignes d'écart) | 2,6 % | 0 | 0 |
| JavaScript, éditions aléatoires | 645 | 0 | 0 % | 0 | 0 |
| JavaScript, insertions de blocs | 657 | 3 | 0,46 % | 0 | 0 (hors fins de ligne, voir ci-dessous) |
| PHP, éditions aléatoires | 591 | 5 | 0,85 % | 0 | 0 |
| PHP, insertions de blocs | 277 | 5 | 1,8 % | 0 | 0 |
| **Total** | **2 209** | **14** | **0,63 %** | **0** | 0 |

Corpus fait main (38 cas) : sur les 17 cas que Git fusionne proprement, 2 faux conflits (`25-lignes-repetees`, `38-php`) : 12 %, mais c'est un corpus choisi pour piéger.

À retenir : sur du code réel le surcoût est de l'ordre de **0,5 à 2 %** (le « ~6 % » de l'agent du moteur vaut pour des tirages aléatoires sur des lignes courtes et répétées, plus durs que du code) ; il est plus fort en PHP quand on ajoute une méthode près d'une autre modifiée (M1). Les 14 cas sont tous du côté sûr (on montre un conflit de trop, on ne perd rien). Limite de la mesure : 39 vrais couples seulement, donc l'intervalle de confiance du premier chiffre est très large ; les autres lignes sont des éditions simulées, pas des éditions humaines. Les écarts de texte du premier jet (451 sur le PHP) venaient de mon générateur (lignes insérées sans `\r` dans des fichiers CRLF) : notre moteur donne alors à la ligne le style dominant de « notre » version, comme le veut la spec, alors que Git recopie l'octet ; corrigé dans `fusion-reel.py`, 0 écart ensuite hors cet effet voulu.

Reproduire : `python tests\fixtures\fusion-reel.py histoire <clone> <dossier>` ou `aleatoire <dossier-source> <ext> <n> <dossier> [graine]` (variable `BLOCS=1` pour les insertions de blocs), puis `mergecli <dossier> > sortie.tsv` et `fusion-reel.py comparer <dossier> sortie.tsv`.

## (e) L'avis net : « en dix minutes, sentir les trois piliers »

**Oui, la v0 tient la promesse.** Un parcours de dix minutes sur un clone de `labmate_paper` a marché sans accroc majeur : ouvrir, déplier l'arbre, éditer, Ctrl+S, voir le fichier apparaître dans Git en moins de 3 s avec un diff propre, chercher dans les fichiers (l'occurrence s'ouvre à la bonne ligne), valider avec un message accentué (le `git show` en ligne de commande dit la même chose que l'IDE), résoudre une fusion à trois volets jusqu'au commit à deux parents, lancer PHPUnit et sauter de la trace à la ligne, ouvrir un Git Bash et un cmd côte à côte. Les fichiers écrits sont fidèles à l'octet, y compris CRLF / BOM / sans saut final / Windows-1252, et la fusion s'abstient quand elle doute : c'est le point le plus important, et il est tenu sur 2 200 fusions de vrai code.

Ce qui gâche encore l'impression, par ordre de gêne : (1) les faux conflits de la fusion sur « méthode ajoutée près d'une autre modifiée » en PHP (M1) : ils ne coûtent rien en sécurité mais c'est exactement le cas où l'on attend de l'outil qu'il fasse mieux que Git ; (2) le bouton « Fusionner… » toujours actif (M3) ; (3) le délai d'environ 2 s du rafraîchissement Git (M4) ; (4) deux messages d'état qui restent (M6). Rien de tout cela n'empêche de livrer la v0 à la personne qui dirige le projet ; M1 et M2 sont à planifier avant la v1.

## État de labmate_paper

Rendu comme trouvé, vérifié à la fin : branche `main` à `2bbd96a`, `README.md` modifié avec le **même SHA-256** qu'au début (`20ffe694…b4fb` : la modification non validée est intacte), `.idea/` non suivi inchangé, aucune étagère, aucun commit, aucun push. Les deux branches locales `juliestorm-essai/eux` et `juliestorm-essai/nous` **existaient avant ma recette** : je ne les ai ni créées ni supprimées, car `tests/fixtures/labmate-clone.sh` en dépend (il les `fetch`). `CLAUDE.md` demande de les supprimer en fin d'essai : à la personne qui dirige le projet de trancher (je les laisse). Tous mes essais ont eu lieu sur des clones dans `%TEMP%`, supprimés.

## Fichiers livrés

`tests/fixtures/fusion-corpus.py`, `tests/fixtures/fusion-reel.py`, `tests/fixtures/mergecli/` (Cargo.toml + src/main.rs), `tests/fixtures/recette-projets.py`, `tests/scenarios/recette-*.txt` (16), ce rapport.

## Suite donnée aux écarts (25/11/2026)

Les sept écarts ont été corrigés ; ils restent à rejouer par la testeuse pour être clos.

| Écart | Correctif | Preuve apportée par le développement |
|---|---|---|
| M1 faux conflits | `julie-diff` : lignes sûres comptées sur tout le fichier, alignements rangés comme ceux de Git | conflits en trop sur les mesures de la recette rejouées : 18 → 0 sur 2 038 ; 54 000 tirages contre `git merge-file` : 0 cas « nous propre, Git en conflit » ; cas `25` et `38` fusionnés |
| — faille trouvée en route | la v0 fusionnait en silence un cas que Git déclare en conflit (lignes répétées décalées contre une modification) | cas `repeat-shift-vs-edit-git-conflict` ajouté au corpus et en test unitaire |
| M2 Ctrl+C | `julie-terminal` efface le drapeau « ignorer Ctrl+C » hérité avant de lancer un shell | test `ctrl_c_works_when_the_ide_ignores_ctrl_c`, vert lancé depuis PowerShell et depuis Git Bash |
| M3 « Fusionner… » | vérification en tâche de fond, bouton grisé avec la raison en infobulle | scénario `ecarts-m3-fusionner-grise.txt` |
| M4 délai Git | relève Git toutes les 1,2 s et rafraîchissement dès qu'un fichier ouvert change | 1,11 s au plus sur 8 essais (`ecarts-m4-modif-externe.txt`) |
| M5 regex | erreur affichée dès un caractère, messages traduits | `ecarts-m5-regex.txt` |
| M6 barre d'état | durée de vie des messages, effacement au changement de projet | `ecarts-m6-messages.txt` |
| M7 BOM | le résultat suit toujours notre version ; bandeau quand les versions diffèrent sur le BOM | `ecarts-m7-bom.txt` |

Changement de comportement à connaître : deux cas que la v0 laissait volontairement en conflit (`js-blank-run-vs-neighbour`, `php-new-method-after-edited-statement`) fusionnent désormais, comme chez Git.
