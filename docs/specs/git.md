# Git (v0) : changements locaux, diff, commit, journal, conflits

Rédigé le 24/11/2026 par `specialiste-produit`. **(observé)** / **(supposé)** /
**(écart)** : voir `fusion.md`. Tout passe par l'exécutable `git` (voir `CLAUDE.md`).

## À quoi ça sert

Voir ce que j'ai changé, le relire en diff, le valider, parcourir l'historique, et
savoir tout de suite que mon dépôt est en conflit et quels fichiers régler.

## Fenêtre Git

Fenêtre d'outils du bas (Alt+9), trois onglets : **Changements**, **Journal**,
**Console**. La console liste chaque commande `git` lancée par l'IDE, sa durée et sa
sortie **(observé)**. Si le dossier ouvert n'est pas dans un dépôt : « Ce dossier
n'est pas un dépôt Git. » et rien d'autre. Si `git` est introuvable : message
indiquant le réglage du chemin de `git`.

## Changements locaux

- Source : `git status --porcelain=v2 -z --untracked-files=all`.
- Deux groupes, avec leur nombre de fichiers : **Modifiés** (modifiés, ajoutés à
  l'index, supprimés, renommés) et **Non suivis**. Un troisième groupe **En
  conflit** apparaît en tête quand il y en a (voir plus bas).
- Par fichier : nom, dossier en gris, couleur selon l'état (modifié, ajouté,
  supprimé, renommé, non suivi). Un renommage montre « ancien → nouveau ». Les
  fichiers ignorés n'apparaissent pas.
- L'arbre du projet reprend les mêmes couleurs **(observé)**.
- Rafraîchissement : automatique moins de 2 s après tout changement sur disque
  (fait par l'IDE ou ailleurs, `.git` compris), au retour du focus sur la fenêtre,
  et par un bouton.
- Double-clic ou Ctrl+D sur un fichier : diff côte à côte. Clic simple : aperçu du
  diff à droite de la liste **(observé)**.
- États : chargement (indicateur discret, l'ancienne liste reste affichée) ;
  dépôt propre (« Aucun changement ») ; erreur Git (message + lien vers la console).
- **Écart** : pas de listes de changements nommées en v0, une seule liste.

## Diff côte à côte

- Gauche : version de `HEAD` (lecture seule). Droite : fichier de travail (lecture
  seule en v0 — **écart**, PhpStorm permet d'y éditer ; on garde l'édition à
  l'éditeur pour la v0).
- Blocs colorés (ajouté, supprimé, modifié), reliés d'un volet à l'autre,
  défilement synchronisé, coloration syntaxique des deux volets, numéros de ligne.
- F7 / Maj+F7 : différence suivante / précédente. Compteur « n différences ».
- Non suivi ou ajouté : gauche vide. Supprimé : droite vide. Renommé : gauche =
  ancien chemin dans `HEAD`.
- Fins de ligne seules différentes : « Seules les fins de ligne diffèrent (CRLF →
  LF) » au lieu d'un diff plein rouge. Saut final ajouté/retiré : signalé en bas.
- Binaire : « Fichiers binaires — taille avant / après ». Au-delà de 20 Mio :
  « Fichier trop gros pour le diff ».
- Dépôt sans commit : gauche vide pour tous les fichiers.

## Commit

- Ctrl+K ouvre le panneau de commit (Alt+0 le montre) : la liste des changements
  avec une case par fichier (cochés par défaut : les modifiés ; décochés : les non
  suivis — **supposé** comme PhpStorm), le champ du message, le bouton « Valider ».
- Ctrl+Entrée dans le message = Valider. Message vide ou seulement des espaces :
  bouton désactivé. Aucun fichier coché : bouton désactivé.
- Ce qui est validé = **le contenu du fichier de travail** des fichiers cochés,
  quel que soit l'état de l'index (pas de notion d'index exposée, comme PhpStorm) ;
  les fichiers non cochés gardent leur état d'index intact. Concrètement :
  `git add -- <non suivis cochés>` puis `git commit -F <message> --only -- <cochés>`
  (à confirmer par `dev-git`).
- Après succès : message vidé, liste rafraîchie, notification « 1 commit : <sujet> »,
  le journal montre le commit en tête.
- Échec (crochet `pre-commit`, identité Git absente…) : rien n'est validé, le
  message est conservé, la sortie de Git s'affiche dans une fenêtre lisible.
- Le message en cours survit à la fermeture du panneau (pas au redémarrage en v0).
- Pendant une fusion, une fois tous les conflits résolus, le message est
  pré-rempli avec `.git/MERGE_MSG` et Valider termine la fusion.

## Journal

- Onglet Journal : liste des commits de `HEAD` (`git log`), plus récent en haut,
  colonnes : sujet, auteur, date (relative si moins de 7 jours, sinon `JJ/MM/AAAA
  HH:MM`), hash court. Chargement par lots de 500 en défilant.
- Les commits de fusion sont grisés **(observé)**.
- Sélection d'un commit : à droite, le message complet, l'auteur, la date exacte, le
  hash complet, la liste des fichiers changés ; double-clic sur un fichier = diff de
  ce fichier entre le parent et ce commit (premier parent pour une fusion).
- Dépôt sans commit : « Aucun commit pour l'instant. »
- Rafraîchi après chaque commit fait par l'IDE et au bouton.

## Dépôt en conflit

- Détection : `.git/MERGE_HEAD`, `.git/rebase-merge` ou `rebase-apply`,
  `.git/CHERRY_PICK_HEAD`, et entrées non fusionnées (lignes `u` de porcelain v2).
- Bandeau en tête de la fenêtre Git et pastille dans la barre d'état :
  « Fusion en cours — 3 fichiers en conflit » (ou rebase / cherry-pick), bouton
  « Résoudre ».
- **Liste des fichiers en conflit** (fenêtre) **(observé d'usage)** : par fichier,
  chemin, nature (modifié des deux côtés, ajouté des deux côtés, supprimé par nous,
  supprimé par eux), et trois actions :
  - **Garder la nôtre** : écrit l'étape 2 (ou supprime si supprimé par nous),
    puis `git add` / `git rm` ;
  - **Prendre la leur** : idem avec l'étape 3 ;
  - **Fusionner…** : ouvre le dialogue de `fusion.md` (désactivé, avec la raison en
    infobulle, pour les cas listés dans `fusion.md` → Cas limites).
  Sélection multiple permise pour garder / prendre. Toute action qui supprime un
  fichier demande confirmation en nommant le fichier.
- Un fichier résolu quitte la liste ; liste vide → « Tous les conflits sont résolus,
  vous pouvez valider. » et le panneau de commit s'ouvre.
- Si les conflits sont résolus hors de l'IDE (terminal), la liste se met à jour
  dans les 2 s.

## Cas limites

Dépôt sans commit (liste et commit fonctionnent ; premier commit possible) ; chemin
avec espaces, accents ou guillemets (via `-z`, jamais de découpe sur les espaces) ;
fichier supprimé puis recréé non suivi ; renommage non indexé (apparaît comme
supprimé + non suivi, comme Git) ; `core.autocrlf` actif (on affiche ce que Git
dit, on ne convertit rien) ; dépôt dans un sous-dossier du projet ouvert ou projet
dans un sous-dossier du dépôt (chemins relatifs au projet) ; 5 000 fichiers non
suivis (liste en moins de 2 s, défilement fluide) ; verrou `.git/index.lock`
présent (message « Une autre opération Git est en cours »).

## Critères d'acceptation

1. Sur un dépôt d'essai avec 1 fichier modifié, 1 supprimé, 1 ajouté à l'index et
   1 non suivi, les groupes et les nombres affichés sont exacts.
2. Modifier un fichier hors de l'IDE le fait apparaître dans la liste en moins de 2 s.
3. Le diff d'un fichier modifié montre `HEAD` à gauche, le travail à droite, F7 va
   de bloc en bloc.
4. Un fichier dont seules les fins de ligne changent affiche le message dédié.
5. Valider 2 fichiers cochés sur 3 : `git show --stat HEAD` liste exactement ces 2,
   le troisième reste modifié et son état d'index est inchangé.
6. Un non suivi coché est inclus dans le commit.
7. Un crochet `pre-commit` qui sort en erreur : pas de nouveau commit, message
   conservé, sortie visible.
8. Le journal liste les commits dans l'ordre de `git log` ; sélectionner un commit
   montre son message et ses fichiers.
9. Dépôt d'essai en fusion avec 2 fichiers en conflit : bandeau et liste les
   montrent ; « Prendre la leur » sur l'un rend son contenu égal à l'étape 3 et le
   retire de `git diff --name-only --diff-filter=U`.
10. Fichier supprimé par eux : « Fusionner… » désactivé ; « Prendre la leur »
    demande confirmation puis le supprime et l'enregistre comme supprimé.
11. Après résolution du dernier conflit, Valider crée un commit à deux parents avec
    le message de `MERGE_MSG` par défaut.
12. Toutes les commandes lancées apparaissent dans la Console.
13. Aucune opération Git ne gèle l'interface (le curseur de l'éditeur reste
    utilisable pendant un `git status` de 3 s).

## Hors périmètre v0

Listes de changements nommées, commit partiel par blocs, amend, annuler des
changements (rollback), graphe du journal, filtres du journal, branches,
récupérer/envoyer, étagère, patchs, abandon ou poursuite d'un rebase/fusion depuis
l'IDE (se fait dans le terminal), édition dans le diff, blame.
