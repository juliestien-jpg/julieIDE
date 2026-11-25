# Branches Git (v1)

Rédigé le 25/11/2026 par `specialiste-produit`, d'après la demande du même jour
(« gérer les branches de la même manière que PhpStorm : basculer, créer, fusionner,
pull, fetch, push, rebase… »). Repères : **(observé)** / **(supposé)** / **(écart)** :
voir `fusion.md`.

**État de l'observation** : seul ce qui figure dans `observations-phpstorm.md` a été
vu (journal à trois colonnes, branches rangées par préfixe, étagère avec des entrées
automatiques « avant bascule / avant mise à jour »). L'ouverture du widget de
branche de PhpStorm a été tentée le 25/11/2026 puis interrompue (outil de contrôle
limité au clic simple dans un IDE, clic arrêté) : **tout ce qui décrit la fenêtre
surgissante et ses sous-menus est supposé d'usage** et devra être confirmé par une
observation (voir les points à trancher).

Complète `git.md` (v0) ; la liste des conflits et le dialogue de fusion existants
(`fusion.md`) servent tels quels quand une opération s'arrête sur un conflit.

## À quoi ça sert

Passer d'une branche à l'autre, en créer, récupérer le travail des autres et
envoyer le mien sans quitter l'IDE, sans jamais perdre un changement local, et en
comprenant ce que Git a fait quand il refuse.

## Principes

- Tout passe par l'exécutable `git` (`CLAUDE.md`). Chaque commande apparaît dans
  l'onglet Console. Aucune commande réseau n'est lancée sans geste de
  l'utilisatrice (pas de fetch automatique dans cette version).
- **Rien de local ne se perd sans confirmation nommée** : bascule forcée,
  réinitialisation, suppression de branche non fusionnée, force push. Toute action
  qui jette quelque chose dit quoi (fichiers, nombre de commits) et comment le
  récupérer.
- **Identifiants** : l'IDE ne demande, ne lit, ne stocke **jamais** de mot de passe
  ni de jeton. Il laisse faire le gestionnaire d'identifiants de Git (sa fenêtre
  peut apparaître pendant un fetch, pull ou push) et l'agent SSH (Pageant,
  ssh-agent). `GIT_TERMINAL_PROMPT=0` reste posé : si Git a besoin d'une saisie au
  clavier, l'opération échoue proprement avec un message qui renvoie au terminal
  intégré (« Git demande une authentification qu'il ne peut pas obtenir ici. Lancez
  la commande une fois dans le terminal pour enregistrer vos identifiants. »).
- Les opérations réseau ont une barre de progression **annulable** ; pas de délai
  maximal imposé (un gros fetch peut durer).
- Les crochets Git (`post-checkout`, `pre-rebase`, `pre-push`…) ne sont jamais
  contournés ; leur sortie est montrée quand ils échouent.
- Une opération à la fois sur le dépôt (les écritures sont déjà sérialisées par
  `julie-vcs`) ; une deuxième demande attend, avec « En attente de l'opération
  Git en cours ».

## Le widget de branche

- Dans la barre du haut, à côté du nom du projet **(observé)** : le nom de la
  branche courante, et, si elle a un amont, l'avance et le retard
  (« ↑2 ↓3 ») **(supposé)**. Cas particuliers : « HEAD détachée (a1b2c3d) » ;
  « Rebase en cours (3/7) », « Fusion en cours » ; « Pas de dépôt Git » (grisé) ;
  dépôt sans commit : le nom de la branche à naître, en italique.
- Infobulle : branche, amont, avance/retard, dernier fetch (« jamais » si inconnu).
- Clic, ou **Ctrl+Maj+`** **(supposé)** : la fenêtre surgissante des branches.

## La fenêtre surgissante des branches

De haut en bas **(supposé)** :

1. **Champ de recherche**, focus d'emblée : la frappe filtre toutes les branches
   par correspondance floue sur le nom complet (`fe/lo` trouve `feature/login`),
   les dossiers se déplient sur les correspondances. Entrée ouvre le sous-menu de la
   première branche trouvée.
2. **Actions** : Mettre à jour le projet… (Ctrl+T), Valider… (Ctrl+K), Pousser…
   (Ctrl+Maj+K), Nouvelle branche…, Basculer vers une révision… (hash, étiquette ou
   nom), et un bouton Récupérer (fetch) dans la barre du titre.
3. **Récentes** : les 5 dernières branches où l'on a basculé, la courante exclue,
   tirées du reflog (`checkout: moving from A to B`), donc aussi celles basculées en
   ligne de commande.
4. **Locales** : regroupées en dossiers par préfixe (`feature/`, `bugfix/`,
   `juliestorm-essai/`…) **(observé dans le journal)** ; un dossier à un seul
   élément est replié en une ligne `feature/login` (**écart** possible : choix de
   lisibilité). Favoris en tête de leur groupe, marqués d'une étoile cliquable ;
   `main` et `master` sont favoris par défaut **(supposé)**.
5. **Distantes** : par distant (`origin`, …), puis par préfixe. `origin/HEAD` n'est
   pas listé.

Chaque ligne : nom ; marque de la branche courante ; amont en gris
(`origin/feature/login`) ; avance/retard sur l'amont (« ↑1 », « ↓4 », rien si à
jour) ; « amont supprimé » si l'amont a disparu (`[gone]`). Les étiquettes sont
hors périmètre de cette fenêtre.

Clavier : Haut/Bas, Droite ou Entrée ouvre le sous-menu, Gauche le ferme, Échap
ferme un niveau. Survol d'une branche à la souris : le sous-menu s'ouvre à droite.

## Sous-menu d'une branche

`C` = branche courante, `X` = la branche du sous-menu. Les entrées impossibles sont
grisées avec la raison (règle de `menus-contextuels.md`).

| Entrée | Locale `X` | Distante `origin/X` | Courante | Commande et remarques |
|---|---|---|---|---|
| Basculer | oui | oui | — | `git switch X`. Distante : crée la locale `X` qui la suit (`git switch -c X --track origin/X`) ; si une locale `X` existe déjà, on bascule sur elle et on propose de la mettre à jour si elle est en retard. |
| Nouvelle branche depuis X… | oui | oui | oui | dialogue de création, base = X |
| Basculer et rebaser sur C | oui | — | — | `git rebase C X` : X est rejouée au-dessus de C puis devient la courante |
| Comparer avec C | oui | oui | — | onglet « X ↔ C » : commits de X absents de C, commits de C absents de X (`git log C..X`, `X..C`), et la liste des fichiers qui diffèrent, diff au double-clic |
| Voir le diff avec l'arbre de travail | oui | oui | oui | fichiers qui diffèrent entre X et le disque (`git diff X`), diff au double-clic |
| Rebaser C sur X | oui | oui | — | `git rebase X` |
| Fusionner X dans C | oui | oui | — | `git merge X` |
| Mettre à jour | oui (a un amont) | — | oui | courante = Mettre à jour le projet ; autre locale = avance rapide sur son amont sans basculer (`git fetch <distant> X:X`), refusée si elle a divergé |
| Tirer dans C par fusion / par rebase | — | oui | — | `git pull --no-rebase origin X` / `--rebase` |
| Pousser… | oui | — | oui | dialogue de push, sur cette branche |
| Renommer… | oui | — | oui | `git branch -m X Y` ; si X a un amont, avertit que la branche distante garde son nom |
| Supprimer… | oui | oui | — (grisée : « branche courante ») | voir « Supprimer une branche » |

## Créer une branche

- Dialogue : **nom**, **depuis** (préremplie : HEAD, ou la branche, le commit ou la
  branche distante d'où l'on vient), case **Basculer sur la nouvelle branche**
  (cochée).
- Nom vérifié à la frappe avec `git check-ref-format --branch` : vide, espaces,
  `..`, `~ ^ : ? * [ \`, fin en `.lock` ou `/` → erreur, bouton grisé. Les espaces
  peuvent être remplacés par des tirets d'un clic (proposition sous le champ).
- Nom déjà pris (y compris à la casse près : Windows ne distingue pas
  `Feature/x` de `feature/x` sur le disque) → erreur. Un nom qui serait un dossier
  d'une branche existante (`feature` quand `feature/login` existe) → erreur
  expliquée.
- Commandes : `git switch -c Y <depuis>` (cochée) ou `git branch Y <depuis>`.
  Depuis une branche distante : la nouvelle locale la suit (`--track`).
- Avec des changements locaux et la case cochée : ils suivent la nouvelle branche
  (Git le permet toujours depuis HEAD) ; depuis une autre base, règle de la bascule.

## Basculer avec des changements locaux

1. L'IDE lance `git switch X`. Si Git accepte, les changements suivent (comme en
   ligne de commande) ; notification « Basculé sur X ; vos changements locaux ont
   suivi ».
2. Si Git refuse parce que des fichiers modifiés seraient écrasés, un dialogue
   liste ces fichiers et propose :
   - **Mettre de côté et basculer** (bouton par défaut) : les changements locaux
     (non suivis compris) sont mis de côté sous le nom « JulieStorm : avant bascule
     de C vers X — <date> », on bascule, puis on les **restaure**. Si la restauration
     entre en conflit, la liste des conflits s'ouvre, et l'entrée mise de côté est
     **gardée** tant que ce n'est pas résolu (notification avec le nom de l'entrée)
     **(observé : entrées automatiques « avant bascule » dans l'étagère)** ;
   - **Forcer la bascule** : jette les changements des fichiers listés ; seconde
     confirmation qui les nomme et dit qu'ils seront perdus ; le filet de sécurité
     de `menus-contextuels.md` (copie dans `.juliestorm/annulations/`) s'applique ;
   - **Annuler**.
3. Fichier non suivi qui serait écrasé par un fichier suivi de X : même dialogue.
4. Pendant une fusion ou un rebase en cours : Basculer est grisé (« Terminez ou
   abandonnez la fusion en cours »).

Rangement des changements mis de côté : **`git stash`** (lisible en ligne de
commande, `git stash list`), en attendant l'étagère de la v1, qui devra les montrer
(**écart** : PhpStorm peut utiliser sa propre étagère ; point à trancher).

## Récupérer, mettre à jour, tirer

- **Récupérer (fetch)** : `git fetch --all --prune` ; notification « Récupéré :
  3 branches mises à jour, 1 supprimée sur le distant » ou « Rien de nouveau ».
  Les avances/retards se recalculent.
- **Mettre à jour le projet (Ctrl+T)** : la première fois, un dialogue demande
  **Fusion** ou **Rebase**, avec la case « Ne plus demander » qui mémorise le choix
  (réglage `[vcs] update_method`, `"fusion"` ou `"rebase"`, portée projet,
  modifiable dans la page Git des réglages) **(supposé comme PhpStorm)**. Puis :
  changements locaux mis de côté (« avant mise à jour ») → `git fetch` du distant de
  l'amont → `git merge @{u}` ou `git rebase @{u}` → restauration. Résultat :
  « 4 commits reçus, 12 fichiers changés » avec un lien vers le journal filtré sur
  ces commits ; « Déjà à jour » sinon.
  - Pas d'amont : message « C n'a pas de branche distante associée » avec
    « Choisir l'amont… » (liste des branches distantes) ; jamais de devinette.
  - Amont supprimé : « La branche distante origin/X n'existe plus » ; proposer
    « Pousser pour la recréer » ou « Retirer l'amont ».
- **Tirer… (Pull)** : menu Git → Tirer… : dialogue distant, branche (par défaut
  l'amont), mode fusion ou rebase, option « Avance rapide seulement »
  (`--ff-only`). Mêmes mises de côté et mêmes issues que la mise à jour.

## Pousser (Ctrl+Maj+K)

- Dialogue **(supposé)** : à gauche, pour la branche courante (ou celle du
  sous-menu), la ligne « C → origin : C » ; la cible est modifiable (distant et
  nom), marquée « nouvelle » si elle n'existe pas sur le distant. En dessous, les
  **commits sortants** (`git log <amont>..C`, ou pour une nouvelle branche les
  commits absents de tout distant : `git log C --not --remotes`) ; sélectionner un
  commit montre ses fichiers et son message à droite.
- Pousser une nouvelle branche pose l'amont (`git push -u`). Option « Pousser aussi
  les étiquettes » (décochée).
- Bouton **Pousser** (`git push --porcelain`). La flèche du bouton propose
  **Forcer avec bail** seulement (`--force-with-lease=C:<sha distant connu>`),
  jamais `--force` nu ; elle exige une confirmation qui dit combien de commits du
  distant seront remplacés. Grisée pour les branches protégées (`main`, `master`
  par défaut, réglage `[vcs] protected_branches`) avec la raison.
- Aucun commit sortant : bouton grisé, « Rien à pousser ».
- **Refus non avance rapide** : dialogue « Le distant a des commits que vous n'avez
  pas encore (n). Récupérez-les avant de pousser. » avec **Fusionner puis
  pousser**, **Rebaser puis pousser**, Annuler. Jamais de force proposée ici.
- Refus par un crochet `pre-push` ou par le serveur (branche protégée côté
  serveur) : la sortie de Git, lisible, dans une fenêtre ; rien n'est réessayé.
- Succès : « 3 commits poussés vers origin/C » ; l'avance retombe à zéro.

## Fusion et rebase

- Fusionner X dans C : `git merge X` (avance rapide si possible). Menu Git →
  Fusionner… ouvre un dialogue avec les options : sans avance rapide (`--no-ff`),
  avance rapide seulement, écraser en un commit (`--squash`), sans valider
  (`--no-commit`), message.
- Rebaser : `git rebase X` (avec `--autostash` quand il y a des changements
  locaux : même nom d'entrée « avant rebase »).
- **Conflit** : bandeau existant « Fusion en cours — n fichiers en conflit » (ou
  « Rebase en cours — commit 3 sur 7 : <sujet> »), liste des conflits et dialogue de
  fusion de `fusion.md`. Le bandeau gagne les boutons :
  - fusion : **Valider** (quand tout est résolu, message de `MERGE_MSG`) et
    **Abandonner la fusion** (confirmation : « vos résolutions seront perdues ») ;
  - rebase : **Continuer**, **Passer ce commit** (confirmation qui nomme le commit),
    **Abandonner le rebase** (confirmation ; la branche revient à son état d'avant).
  (Modifie `git.md`, qui laissait ces gestes au terminal ; le contrat de `julie-vcs`
  a déjà `continue_operation` et `abort_operation`.)
- Rebase interactif : hors périmètre.

## Journal : branches et graphe

- **Arbre des branches à gauche** **(observé)** : HEAD, Locales (dossiers par
  préfixe), Distantes ; champ de filtre ; favoris. Clic : le journal montre cette
  branche (filtre « branche » en tête, retirable) ; clic droit : le sous-menu de la
  fenêtre surgissante.
- **Graphe en couloirs colorés** **(observé)** : un couloir par lignée, la ligne de
  premier parent bien droite, une couleur stable par branche d'une session à
  l'autre, commits de fusion grisés (déjà fait), étiquettes de branches et
  d'étiquettes sur les commits, HEAD marqué. Le graphe se calcule par pages de 500
  commits, sans gel ; 10 000 commits défilent sans saccade.
- **Clic droit sur un commit** (en plus des copies de `menus-contextuels.md`) :

| Entrée | Commande | Confirmation / grisée |
|---|---|---|
| Nouvelle branche ici… | dialogue de création, base = ce commit | — |
| Basculer sur ce commit | `git switch --detach <sha>` | confirmation qui explique la HEAD détachée ; règles de la bascule pour les changements locaux |
| Cherry-pick | `git cherry-pick <sha>` (plusieurs commits sélectionnés : du plus ancien au plus récent) | grisé pour un commit de fusion (« commit de fusion : non pris en charge ») ou déjà sur C |
| Annuler par un commit inverse (revert) | `git revert --no-edit <sha>` | grisé pour un commit de fusion |
| Réinitialiser C ici… | `git reset --soft|--mixed|--hard <sha>` | dialogue ci-dessous |

- **Réinitialiser** : dialogue avec les trois modes expliqués en une ligne chacun —
  garder tout indexé ; garder les changements dans les fichiers (par défaut) ;
  **tout jeter** (fichiers compris). Il dit combien de commits ne seront plus sur
  aucune branche (« récupérables par le reflog de Git pendant 90 jours ») et, si la
  branche était poussée, qu'il faudra forcer le prochain push. Le mode « tout
  jeter » avec des changements locaux : seconde confirmation qui nomme les fichiers,
  case « Mettre de côté mes changements locaux avant » cochée par défaut.
- Cherry-pick et revert qui s'arrêtent sur un conflit : bandeau, liste des
  conflits, Continuer / Abandonner, comme le rebase.

## Supprimer une branche

- Locale : confirmation « Supprimer la branche X ? ». Si `git branch -d` refuse
  (commits non fusionnés dans C ni dans son amont), seconde confirmation qui donne
  le nombre de commits et les 5 premiers sujets, puis `git branch -D`. Après
  suppression, la notification propose **Restaurer** (recrée X sur le même commit)
  pendant la session.
- Case « Supprimer aussi origin/X sur le distant » (décochée, présente si X a un
  amont).
- Distante : `git push origin --delete X`, confirmation forte (« sur le distant,
  pour tout le monde »), bouton par défaut Annuler ; grisée pour les branches
  protégées.

## Cas d'erreur et cas limites

- **HEAD détachée** : le widget le dit ; Valider marche, avec un bandeau « Vos
  commits ne sont sur aucune branche » et le bouton « Créer une branche ici » ;
  Pousser et Mettre à jour grisés. Quitter une HEAD détachée qui porte des commits
  absents de toute branche : avertissement avec leur nombre et « Créer une branche
  d'abord ».
- **Branche distante supprimée** (`[gone]`) : marquée dans la fenêtre surgissante ;
  voir Mettre à jour et Supprimer.
- **Authentification** : voir Principes. Délai dépassé / réseau absent : message
  de Git, rien de changé localement.
- **Crochets** qui échouent (`post-checkout` qui sort en erreur après une bascule
  réussie) : la bascule est faite, l'avertissement montre la sortie du crochet.
- **Verrou** `.git/index.lock` : « Une autre opération Git est en cours », rien
  n'est forcé, aucun verrou n'est supprimé par l'IDE.
- Branche déjà extraite dans un autre arbre de travail (`git worktree`) : refus de
  Git expliqué.
- Nom de branche identique à un nom de fichier : commandes avec `--` ou
  `git switch` (sans ambiguïté).
- Dépôt sans commit : seules Nouvelle branche (renomme la branche à naître) et
  Valider ont un sens ; le reste grisé.
- Sous-modules : leurs branches ne sont pas gérées (hors périmètre).
- 2 000 branches distantes : la fenêtre surgissante s'ouvre en moins de 300 ms
  (liste lue par `git for-each-ref` en tâche de fond, rafraîchie après chaque
  opération et chaque fetch), la recherche suit la frappe.
- Avance/retard : calculés par `git for-each-ref --format=%(upstream:track)` ;
  jamais de commande réseau pour les obtenir.

## Raccourcis

| Raccourci | Action | PhpStorm |
|---|---|---|
| Ctrl+Maj+` | Fenêtre surgissante des branches | supposé oui |
| Ctrl+T | Mettre à jour le projet | supposé oui |
| Ctrl+Maj+K | Pousser… | supposé oui |
| Alt+` | Petite liste des opérations Git (valider, pousser, mettre à jour, branches, récupérer) | supposé oui |
| Échap, Entrée, flèches | dans la fenêtre surgissante et les dialogues | oui |

## Critères d'acceptation

Tous sur des dépôts **jetables** créés par un script de `tests/fixtures/`
(`branches.sh`, à écrire par `testeur-qa`) dans `%TEMP%` : un dépôt nu
`distant.git` en guise de distant, un clone **A** ouvert dans l'IDE, un clone **B**
qui joue la collègue en ligne de commande. Jamais de vrai distant : la Console ne
doit montrer aucune URL autre que le chemin de `distant.git`. Branches de départ :
`main`, `feature/login`, `feature/search`, `bugfix/crash`, `juliestorm-essai/a`.

1. Le widget affiche `main`. Après 2 commits poussés par B sur `main` et un
   Récupérer dans A, la ligne `main` montre « ↓2 » ; après un commit local dans A,
   « ↑1 ↓2 ».
2. La fenêtre surgissante range `feature/login` et `feature/search` sous un dossier
   `feature`, `bugfix/crash` seul en une ligne ; taper `fe/se` ne laisse que
   `feature/search`.
3. Basculer sur `feature/login` puis `bugfix/crash` puis `main` : Récentes montre
   `bugfix/crash`, `feature/login` dans cet ordre ; une bascule faite dans le
   terminal (`git switch feature/search`) y apparaît aussi.
4. Basculer sur `origin/feature/remote-only` (poussée par B) : une locale
   `feature/remote-only` est créée, suit l'amont, devient courante.
5. Nouvelle branche `ma branche` : erreur et proposition `ma-branche` ; `feature` :
   erreur (dossier existant) ; `Main` : erreur (casse) ; `feature/x` depuis
   `bugfix/crash`, case cochée : `git rev-parse HEAD` = sommet de `bugfix/crash`,
   branche courante `feature/x`.
6. Changement local sur `a.txt` qui serait écrasé par la bascule : dialogue qui
   nomme `a.txt` ; « Mettre de côté et basculer » : on est sur la cible, `a.txt` a
   retrouvé son changement, `git stash list` est vide.
7. Même cas avec une restauration en conflit : liste des conflits ouverte, l'entrée
   « JulieStorm : avant bascule… » est toujours dans `git stash list`.
8. « Forcer la bascule » : deux confirmations nomment `a.txt` ; Annuler à la
   seconde ne change rien (SHA-256 identique, toujours sur la branche d'origine).
9. Ctrl+T la première fois : question fusion / rebase ; « Rebase » + « Ne plus
   demander » : `.juliestorm/settings.toml` contient `update_method = "rebase"` ;
   le Ctrl+T suivant ne demande rien. Après la mise à jour, `git log --oneline` de A
   est linéaire et contient les commits de B.
10. Ctrl+T avec un changement local non conflictuel : le changement est là après
    la mise à jour (même SHA-256), `git stash list` vide.
11. Ctrl+T sur une branche sans amont : message et « Choisir l'amont… » ; aucune
    commande `merge`/`rebase` lancée (Console).
12. Pousser une nouvelle branche `feature/x` : la cible est marquée « nouvelle »,
    les commits sortants listés sont exactement `git log feature/x --not --remotes`
    ; après Pousser, `git -C distant.git branch` contient `feature/x` et
    `git rev-parse --abbrev-ref feature/x@{u}` = `origin/feature/x`.
13. B pousse sur `main`, A a un commit local et pousse : dialogue de refus avec
    Fusionner / Rebaser puis pousser ; « Rebaser puis pousser » : `distant.git` a le
    commit de A au-dessus de celui de B.
14. Forcer avec bail sur `main` : grisé (« branche protégée »). Sur `feature/x`
    après un `reset` local : confirmation qui donne le nombre de commits remplacés ;
    si B a poussé entre-temps sur `feature/x`, le push est refusé (bail) et rien
    n'est écrasé.
15. Crochet `pre-push` qui sort en erreur avec « refusé par le crochet » : le texte
    apparaît, `distant.git` n'a pas bougé.
16. Fusionner `feature/login` (qui modifie la même ligne que `main`) dans `main` :
    bandeau « Fusion en cours — 1 fichier en conflit » ; « Abandonner la fusion »,
    confirmé : `git status` propre, HEAD inchangée. Refaire, résoudre dans le
    dialogue de fusion, Valider : commit à deux parents.
17. Rebaser `feature/login` sur `main` avec conflit sur 1 commit sur 3 : bandeau
    « commit 2 sur 3 » ; résoudre puis Continuer : rebase terminé, 3 commits
    au-dessus de `main`. Refaire et « Abandonner le rebase » : sommet de
    `feature/login` égal à celui d'avant.
18. Supprimer `bugfix/crash` non fusionnée : seconde confirmation avec le nombre de
    commits ; après suppression, Restaurer la recrée sur le même SHA.
19. Supprimer `origin/feature/search` : confirmation forte, bouton par défaut
    Annuler ; confirmé : absente de `git -C distant.git branch`.
20. Journal, clic droit sur un commit ancien → Nouvelle branche ici `essai` (sans
    basculer) : `git rev-parse essai` = ce commit, la branche courante n'a pas
    changé.
21. Cherry-pick d'un commit de `feature/search` sur `main` : un nouveau commit au
    sommet de `main`, même diff. Sur un commit de fusion : entrée grisée.
22. Revert d'un commit : nouveau commit qui inverse son diff (`git diff <sha>^
    <sha>` inverse de `git show HEAD`).
23. Réinitialiser `main` 2 commits en arrière, mode « garder les changements dans
    les fichiers » : HEAD recule de 2, les changements des 2 commits sont dans les
    fichiers, non indexés ; le dialogue annonçait « 2 commits ». Mode « tout jeter »
    avec un fichier modifié : seconde confirmation qui le nomme ; case « mettre de
    côté » laissée cochée : le changement est dans `git stash list`.
24. Basculer sur un commit (HEAD détachée) : widget « HEAD détachée (<7 car.>) » ;
    Pousser grisé ; un commit fait là puis bascule sur `main` : avertissement
    « 1 commit sur aucune branche ».
25. B supprime `feature/login` sur le distant ; Récupérer dans A : la ligne
    `feature/login` dit « amont supprimé » ; Ctrl+T sur elle : message dédié.
26. Remplacer l'URL de `origin` par un chemin inexistant : Récupérer échoue avec le
    message de Git, l'IDE reste utilisable, rien n'a changé dans A.
27. Pendant un fetch ralenti (crochet de serveur `sleep 10` sur un push, ou dépôt
    volumineux), l'éditeur reste utilisable et Annuler arrête l'opération en moins
    de 2 s.
28. Graphe : sur un dépôt d'essai de 10 000 commits avec 30 fusions, le journal
    s'ouvre en moins de 1 s et défile sans saccade ; chaque commit de fusion relie
    ses deux parents.

## Hors périmètre

Rebase interactif, étiquettes (création, poussée sélective), sous-modules,
`git worktree`, fetch automatique périodique, plusieurs distants dans le dialogue de
push au-delà du choix du distant, gestion des distants (ajouter, renommer), branches
protégées configurées côté serveur (on affiche seulement le refus), comparaison de
branches en graphe, résolution « accepter la nôtre / la leur » automatique dans la
mise à jour, signature des commits (respectée si configurée, jamais demandée),
pull request.
