# Recette de la v1

Faite le 25/11/2026 par la testeuse, sur `target\debug\juliestorm.exe` (build de développement, DirectX 12), sur le commit
`72809a9` (lot 5) avec l'arbre de travail du moment (le rédacteur travaillait en parallèle sur les textes ; une autre développeuse
avait des fichiers non validés sur la vue partagée, `split-*`, que je n'ai pas testés). Windows 11, `git` 2.56.
Tout s'est joué par le mode scénario (`docs/essais.md`) dans des dossiers jetables de `%TEMP%` (`jsqa\…`, `jsessai-…`), avec un
`JULIESTORM_STATE_DIR` neuf à chaque lancement (aucune écriture dans le vrai `%APPDATA%\JulieStorm`). Aucun commit, aucun push,
aucune modification du code de l'application. Seul distant : des dépôts nus locaux (`distant.git`) fabriqués par `branches.sh`.

Outillage ajouté (dans `tests/` uniquement) : `tests/fixtures/recette-v1-run.sh` (joue un scénario avec états neufs),
`recette-v1-nav.py` (32 Ctrl+clics + vérificateur), `recette-v1-nav-reel.py` (usages et sauts sur du vrai code, comparés à
`grep -w`), `fusion-enum.py` (énumération exhaustive de la fusion contre `git merge-file`) et 45 scénarios
`tests/scenarios/recette-v1-*.txt` (un par comportement essayé).

## Verdict en une phrase

La v1 tient ses promesses sur le chemin heureux (bascule, création, fetch, mise à jour, push refusé / accepté / bail, fusion et
rebase en conflit, cherry-pick, revert, reset, suppression et restauration, graphe, réglages PHPUnit pour backoffice, clic droit,
Ctrl+clic) et **la fusion n'a toujours résolu aucun vrai conflit en silence** ; en revanche **cinq majeurs** touchent au
travail de l'utilisatrice (surtout : des changements locaux mis de côté pendant une mise à jour qui tombe en conflit ne sont
jamais remis ni signalés) et un point des critères (HEAD détachée) n'est pas câblé. **Aucun bloquant** au sens strict : dans
chaque cas je retrouve le travail (stash, copie de sécurité, reflog), mais il faut savoir où chercher.

## Chiffres tenus

| Contrôle | Résultat |
|---|---|
| `cargo test --workspace` | **vert : ~665 tests, 0 rouge** (une seule exécution) |
| `cargo clippy --workspace --all-targets -- -D warnings` | propre |
| Fusion : énumération exhaustive + longs fichiers, comparée à `git merge-file` | **12 461 triplets : 0 « nous propre / Git en conflit »**, 0 texte différent entre deux fusions propres, 292 conflits en trop (2,3 %, côté sûr) |
| Navigation sur le projet d'essai (32 Ctrl+clics) | 30 conformes à la lettre, 2 écarts documentés (décision 9) ; usages 5 / 1, 1 / 1, 0 / 1, 7, 2, 2, 1, 6 exacts |
| Navigation sur 70 fichiers PHP de `lab` copiés | usages de fonctions et méthodes PHP : 0 manqué (1 faux manque = commentaire) ; 0 saut faux avéré sur 60 appels (voir (d)) |
| Scénarios de `tests/scenarios/` (hors `split-*`) | 74 joués : **72 réussis, 2 échecs** (`menus-arbre`, `branches-push-crochet`), 3 échecs passagers sous charge (rejoués réussis), **0 blocage** sauf un lancement resté 4 min sans écrire (non reproduit, voir m16) |
| Processus restants | aucun `juliestorm`, `php`, `bash` de ma part |
| `labmate_paper`, `backoffice` | rendus comme trouvés (voir plus bas) |

## (a) Vu fonctionner

### Branches (critères de `branches.md`, un par un)
Conforme, preuve en sortie de `git` après l'opération :
- **1** widget `main ↑1`, après Récupérer `↑1 ↓2` (`git branch -vv` identique) ; « 2 branches mises à jour, 1 supprimée ».
- **2** `feature/` replié avec `login` et `search`, `bugfix/crash` seul, `fe/se` ne laisse que `feature/search` ; branche « amont supprimé » signalée.
- **3** Récentes dans l'ordre du reflog, courante exclue (la bascule faite au terminal : **non essayée**).
- **4** bascule sur `origin/feature/remote-only` après Récupérer : locale créée, suit l'amont, courante.
- **5** noms refusés avec message : `ma branche` (+ proposition `ma-branche`), `feature` (dossier), `Main` (casse), `feature/login/x`, `bad..name`, `x.lock`, `a~b` ; `feature/x` depuis `bugfix/crash` : `rev-parse HEAD` = sommet de `bugfix/crash`.
- **6** bascule refusée par `a.txt` : le dialogue nomme `a.txt` ; « Mettre de côté et basculer » avec une modification éloignée : on est sur la cible, `a.txt` a le changement, `git stash list` vide.
- **7** restauration en conflit : liste des conflits ouverte, l'entrée « JulieStorm : avant bascule… » reste dans `git stash list` (et n.txt indexé, u.txt non suivi sont revenus).
- **8** « Forcer » : deux confirmations nomment les fichiers ; Annuler à la seconde : SHA-256 de `a.txt` et `u.txt`, branche et HEAD identiques, aucune copie créée.
- **9** `update_method = "rebase"` écrit dans `.juliestorm/settings.toml` après « Ne plus demander » ; journal linéaire.
- **10** Ctrl+T avec une modification locale éloignée : fusion **et** rebase, modification là, stash vide.
- **12** nouvelle branche poussée (cible « nouvelle », amont posé) ; **13** push refusé → « Rebaser puis pousser » : `distant.git` a le commit de A au-dessus de ceux de B ; **14** « Forcer avec bail » grisé sur `main` avec la raison, **bail périmé** (B a poussé entre-temps) : refusé (`stale info`), `distant.git` inchangé (même SHA), rien d'écrasé ; **15** le crochet `pre-push` : « refusé par le crochet » affiché, distant inchangé.
- **16** fusion en conflit : bandeau, « Abandonner » : `git status` propre et HEAD identique ; refaire, résoudre, Valider : commit à **deux parents**.
- **17** rebase en conflit (2/3 puis Continuer, et Abandonner : existants, rejoués sans échec) ; **18** suppression non fusionnée : seconde confirmation (1 commit, sujet), Restaurer recrée au même SHA ; la case « aussi sur le distant » ne supprime **rien** sans la confirmation forte (jamais lancée par moi : le distant a gardé la branche) ; **19** idem confirmation forte.
- **20** nouvelle branche depuis le journal ; **21** cherry-pick d'un commit de `feature/search` (même diff) ; **22** revert (diff inverse) ; **23** reset : soft (index), mixed (non suivis), **hard avec stash** (« 2 commits » annoncés, seconde confirmation nommant les fichiers, a/c3/n.txt/u.txt retrouvés dans `git stash show --include-untracked`) ; hard sans stash : copie de sécurité faite.
- **25** `feature/login`/`search` « amont supprimé » ; **26** distant injoignable : message de Git, rien de changé, aucun verrou ; la mise à jour fait le `fetch` **avant** de mettre de côté (rien n'est stashé si le réseau échoue) ; **27** fetch lent annulé (scénario rejoué sans échec ; délai < 2 s non chronométré) ; **28** journal de 9 970 commits dont 30 fusions : 500 premières lignes, `merge_rows` = `git log --merges` (2 sur 500, 3 sur 1 000), Fin / Début sans accroc.
- Hors critères : mise à jour d'une autre locale par avance rapide (`feature/login` avancée), refusée si divergée (`feature/search`), « Tirer » par fusion depuis une distante, renommer (amont conservé, comme annoncé), supprimer la courante grisée avec la raison, mise à jour qui tombe en conflit (fusion et rebase) jusqu'à Valider / Continuer.

### Navigation (`navigation.md`)
- Critères **1 à 4, 6, 7, 8, 9, 10, 11, 18, 19** (30 Ctrl+clics sur 32, 2 écarts ci-dessous) ; **3** liste « type inconnu » dans l'ordre `Order`, `Admin`, `User` ; **12 à 16, 20** usages exacts (5 sûrs + 1 possible, 1 + 1, 0 + 1, 7 dans 2 fichiers, 2, 2, 1, saut direct pour `Logger::log`, 6 dans 2 fichiers) ; **17** 5 → 6 usages avec une ligne tapée non enregistrée ; **21** survol souligné et infobulle, rien sur `strlen` ; **22** Ctrl+Alt+Gauche revient ; **24** `Order.php` réécrit hors de l'IDE : 2 candidats en moins de 2,5 s ; **25** projet rouvert : index prêt dès le premier état.
- Sur du vrai code (70 fichiers de `lab`, copiés) : les usages de fonctions et de méthodes PHP sont complets (voir (d)).

### Réglages (`reglages.md`)
- **2** copie de backoffice : « détection automatique : tools\phpunit\phpunit.phar » + « PHPUnit 9.6.39 », commande `php phar --configuration phpunit.xml --teamcity` ; **4** phar relatif `outils\phpunit.phar` réglé par la fenêtre → `[phpunit] phpunit_path = 'outils\phpunit.phar'`, la commande lancée sur la suite d'essai donne 13 tests (1 erreur, 3 échecs, 1 ignoré, 1 incomplet, 1 risqué, 6 réussis) **comme la ligne de commande** ; **5** `C:\nexiste\pas\php.exe` : « Ce chemin n'existe pas », OK actif ; le vrai chemin : « PHP 8.5.13 » ; **6** fichier écrit à la main (commentaire de tête, commentaire de fin de ligne, `[perso]`, CRLF) : après modification puis retrait d'une clé, **seule la ligne de cette clé a changé**, octet pour octet ; **7** retour à l'héritage supprime la clé ; **8** même clé côté utilisateur et projet : le projet l'emporte, le retirer fait revenir la valeur utilisateur ; **9** guillemet non fermé refusé (« Guillemet non fermé »), `--group "très lent"` accepté ; **10** `APP_ENV=test` visible dans la sortie du test ; **12** fichier mal formé : notification avec la ligne 2, changement refusé, **SHA-256 du fichier inchangé** ; **13** thème clair écrit puis relancé : l'IDE démarre clair ; **14** `docs` ajouté aux exclus : disparaît d'Aller au fichier et de la recherche en moins de 5 s ; **15** chemin de Git invalide : « git introuvable : « C:\invalide\git.exe » », retour à l'héritage rétablit Git sans redémarrer ; **16** fichier en lecture seule : message qui le nomme, fenêtre ouverte.

### Menus contextuels (`menus-contextuels.md`)
- Zones dumpées avec toutes leurs entrées (arbre sur fichier suivi / modifié / non suivi / dossier / racine, sélection multiple et mixte, changements 1 / 3 / non suivi, onglet, éditeur, terminal, commit, branche courante et autre) : **toute entrée grisée a une raison non vide** (critère 18) ; « Nouveau », « Renommer » grisés en sélection multiple, « Renommer » / « Supprimer » grisés sur la racine, sous-menu Git grisé hors dépôt.
- **1, 2, 3** Ajouter à Git (menu **et** Ctrl+Alt+A) : `?? → A `, Retirer de l'index : retour à `??`, SHA-256 identique. **5, 6, 7** Annuler les modifications sur modifié + supprimé + ajouté : la confirmation nomme les trois et ce qui arrive à chacun et signale l'onglet non enregistré ; Annuler : rien ne change ; confirmer : modifié et supprimé = `HEAD`, l'ajouté **reste sur le disque** ; « Rétablir » remet le modifié octet pour octet. **8** `sous/dos/a.php` créé et ouvert, `con.php`, `a?.php`, nom pris refusés. **9** renommer `user.php` → `User.php` : `R  src/user.php -> src/User.php`, casse changée sur le disque, onglet suit. **10** supprimer : le fichier est dans la **corbeille Windows** (vérifié avec l'API Shell : nom et dossier d'origine). **11** absolu : presse-papiers = chemin complet. **13** « Lancer les tests de ce fichier » : 13 tests, compteurs égaux à `php phpunit.phar fichier` ; entrée grisée sur un `.md` avec la raison ; « Relancer les échecs » = exactement les 4 tests en échec ou en erreur (filtre exact).

### Non-régression
- 74 scénarios existants rejoués (détail en (d)). `git-fusion-complete` (commit de fusion), `git-fusion-gros`, `tests-phpunit`, `recette-terminal-*` (v0), `nav-*`, `branches-*` : réussis.

## (b) Bugs et écarts

**Aucun bloquant.** Cinq majeurs, seize mineurs. Chaque étape se rejoue avec le scénario nommé.

### Majeurs

#### M1 — Mise à jour arrêtée sur un conflit : le travail local mis de côté n'est jamais remis, et rien ne le dit (majeur ; dev-git + app)
- **Étapes** : `bash tests/fixtures/branches.sh D avance` ; dans A : `a.txt` ligne 2 changée et commitée localement, B change la même ligne et pousse ; dans A, modifier `LISEZMOI.md` et créer `nv.txt` (non suivi). Ouvrir A, Ctrl+T, « Fusion » (ou « Rebase »). Scénarios `recette-v1-b06-maj-conflit.txt`, `b07-maj-abandon.txt`, `b08-maj-rebase-conflit.txt`.
- **Attendu** (`branches.md`, « Mettre à jour le projet ») : les changements mis de côté sont **restaurés** après l'opération ; au minimum, l'utilisatrice est informée qu'une entrée « avant mise à jour » garde son travail.
- **Observé** : la fusion s'arrête (« 1 fichier en conflit à résoudre », **sans un mot sur le stash**). Après « Valider » ou « Continuer » : « Fusion terminée. » ; **après « Abandonner la fusion »** : HEAD revenue, bandeau parti. Dans les trois cas `LISEZMOI.md` a perdu sa modification, `nv.txt` a disparu du disque, et `git stash list` contient « JulieStorm : avant mise à jour de main ». Rien dans l'interface n'y renvoie.
- **Preuve** : `git status -sb` propre ; `git stash list` → `stash@{0}: On main: JulieStorm : avant mise à jour de main — … UTC` ; `cat LISEZMOI.md` sans la ligne locale. Cause : `StashRestore::Pending` est documenté « à restaurer une fois l'opération conclue, par `restore_auto_stash` » (`crates/vcs/src/stash.rs`) mais **`app` n'appelle jamais `restore_auto_stash`**.
- **Pourquoi majeur et pas bloquant** : le travail est entier dans le stash. Mais il s'agit du cas « mise à jour avec du travail en cours qui tombe en conflit », le plus courant en équipe : l'utilisatrice croit avoir perdu ses fichiers.
- **Métier** : `dev-git` (contrat) et `dev-interface` (appel à `restore_auto_stash` après `Valider` / `Continuer` / `Abandonner` + notification).

#### M2 — Bascule forcée et « Réinitialiser… tout jeter » : copie de sécurité invisible, message faux (majeur ; dev-interface)
- **Étapes** : décor `branches.sh D stash-conflit`, ajouter `u.txt` (non suivi) et un `login.txt` non suivi qui existe sur `feature/login` ; basculer sur `feature/login`, « Forcer la bascule… », confirmer. Scénario `recette-v1-b04-force.txt`. Pour le reset : `recette-v1-b12-reset-hard.txt` (case « mettre de côté » décochée).
- **Attendu** (décision 3, critère 8, `menus-contextuels.md` 6) : copie dans `.juliestorm/annulations/` **et « Rétablir »** proposé comme après « Annuler les modifications ».
- **Observé** : la copie est bien faite (`.juliestorm/annulations/<date>/a.txt`, `login.txt`, SHA-256 identiques aux originaux), **mais** la notification dit « Basculé sur feature/login ; **vos changements locaux ont suivi.** » (faux : `a.txt` et `login.txt` ont été jetés ; la phrase vient du fait que `n.txt` et `u.txt` sont restés), **sans bouton « Rétablir »** ; même chose après un reset hard (« Branche réinitialisée sur 8ad1db3. », aucune mention des copies de `a.txt`, `c3.txt`, `n.txt`). L'état JSON expose `safety_copy` mais aucune action n'y mène.
- **Métier** : `dev-interface` (`after_switch` / `ResetDone` ne passent pas `ToastAction::RestoreCopy`) ; `redacteur` pour le texte.

#### M3 — HEAD détachée : ni bandeau, ni avertissement en la quittant (critère 24 non conforme ; majeur à la limite du bloquant)
- **Étapes** : journal, clic droit sur un ancien commit → « Basculer sur ce commit » (confirmation puis OK) ; faire un commit ; basculer sur `main`. Scénario `recette-v1-b13-detachee.txt`.
- **Attendu** : bandeau « Vos commits ne sont sur aucune branche » avec « Créer une branche ici » ; à la sortie, avertissement « 1 commit sur aucune branche » avec « Créer une branche d'abord ».
- **Observé** : le widget dit « HEAD détachée (b9bd908) » et Pousser est refusé (« HEAD est détachée : créez d'abord une branche »), **mais** aucun bandeau après le commit, et la bascule sur `main` se fait sans aucun avertissement : « Basculé sur main. ». Le commit `b9bd908` n'est plus que dans le reflog (`git reflog` : `commit: Commit orphelin`).
- **Preuve** : `git log --oneline --all` ne le montre plus. L'API existe (`julie-vcs`, `service.rs` « Avertissement avant de quitter une HEAD détachée ») mais n'est appelée nulle part dans `app` (`grep -rn orphan crates/app` : rien).
- **Gravité** : un commit sans branche, sans avertissement. Je ne le classe pas bloquant parce que le reflog le garde 90 jours et que `git switch` imprime lui-même l'avertissement sur la sortie d'erreur que l'IDE avale, mais **c'est le cas le plus proche du bloquant** et la spécification le prévoyait explicitement.
- **Métier** : `dev-interface` / `dev-git`.

#### M4 — L'arbre du projet n'est jamais rafraîchi après une opération Git qui ajoute ou retire des fichiers (majeur ; dev-interface)
- **Étapes** : dépôt avec `c1.txt`, `c2.txt`, `c3.txt` ; journal → « Basculer sur ce commit » `c1`. Scénario `recette-v1-b14-arbre-perime.txt`.
- **Observé** : sur le disque il ne reste que `c1.txt` ; l'arbre affiche `c1`, **`c2`, `c3`** pendant au moins 12 s (4 états). Même défaut après une mise de côté (un non suivi disparu reste affiché pendant la fusion en conflit) et pendant l'abandon de la fusion. Après la bascule sur `main` l'arbre est à nouveau juste (hasard : les fichiers existent).
- **Attendu** : arbre à jour en moins de 2 s, comme la liste des changements.
- **Impact** : fichiers fantômes qu'on peut ouvrir (erreur), fichiers arrivés par un pull absents de l'arbre et d'Aller au fichier tant qu'on ne ferme pas le dossier.
- **Métier** : `dev-interface` (`tree.refresh()` après chaque événement VCS qui touche le disque, ou relève périodique).

#### M5 — Pendant un rebase, « Notre version » / « Garder la nôtre » désignent l'amont, pas son propre commit (majeur ; piège ; dev-fusion-diff + dev-interface)
- **Étapes** : A et B modifient la même ligne de `a.txt` (A en commit local, B poussé) ; Ctrl+T → « Rebase » ; dans la liste des conflits : « Fusionner… » (titres) puis « Garder la nôtre ». Scénarios `recette-v1-b08-maj-rebase-conflit.txt`, `b09-rebase-titres.txt`.
- **Observé** : volet de gauche « **Notre version** — (dd88520) » = le commit de B (amont) ; volet de droite « **Leur version** » = **le commit de l'utilisatrice**. « Garder la nôtre » donne `DEUX (B)` et jette le changement de `a.txt` de son propre commit (qui reste ensuite dans l'historique sans ce changement : `Continuer` réussit sans le moindre avertissement). C'est la sémantique de Git (`--ours` = la base du rebase) reprise telle quelle.
- **Attendu** : pendant un rebase, nommer les côtés par ce qu'ils sont (« Votre commit » / « Amont (origin/main) »), comme PhpStorm qui les intervertit, ou au moins afficher les noms de branche ; sinon on perd son propre travail en croyant garder « le sien ».
- **Gravité** : le geste est volontaire (aucune perte silencieuse), mais c'est un piège classique de perte de travail propre au rebase, que la v1 vient d'ouvrir au bouton. Majeur.
- **Métier** : `dev-fusion-diff` (étiquettes du dialogue), `dev-interface` (libellés « Garder la nôtre / Prendre la leur » en contexte de rebase), `redacteur`.

### Mineurs

- **m1 — Texte technique brut dans la barre d'état** : fusionner, cherry-pick ou rebaser quand un fichier local modifié serait écrasé (`branches.sh D stash-conflit`, Fusionner `feature/login`, ou cherry-pick sur `a.txt` modifié) : « Git : des changements locaux seraient écrasés : `OverwriteConflict { tracked: ["C:\\Users\\…\\a.txt"], untracked: [],…` » (représentation `Debug` de Rust). Rien n'est écrit (bon) mais aucun dialogue « mettre de côté ? » comme pour la bascule. Scénarios `b15` (première passe), `b19`. Métier : `dev-interface` / `redacteur`.
- **m2 — Cherry-pick d'un commit déjà dans la branche** : l'entrée n'est pas grisée (le critère 21 le demande) ; cliquer sur le commit du sommet laisse « Cherry-pick en cours — 0 fichier en conflit » (cherry-pick vide) avec Continuer / Passer / Abandonner. Scénario `b23`. Métier : `dev-interface` (griser « déjà sur la branche courante »).
- **m3 — « Restaurer » une branche supprimée perd son amont** : `bugfix/crash` restaurée n'a plus `[origin/bugfix/crash]` dans `git branch -vv`. Idem la suppression distante n'est pas rejouée (normal). Scénario `b16`. Métier : `dev-git`.
- **m4 — Sous-menu de branche** : manque « Voir le diff avec l'arbre de travail » (tableau de la spec) ; sur une branche distante « Basculer et rebaser sur C » est proposé alors que la spec met « — ». Métier : `dev-interface`.
- **m5 — Branche sans amont / amont supprimé (critères 11 et 25)** : le dialogue Fusion / Rebase est posé **avant** de dire qu'il n'y a pas d'amont ; le message existe (« Aucune fusion ni rebase n'a été lancé », bien) mais le bouton « Choisir l'amont… » n'existe pas (seulement « Pousser… »). Scénario `b17`. Métier : `dev-interface`.
- **m6 — Ctrl+Maj+` sans remplacement AZERTY** : la décision 2 de `decisions-v1.md` promet « ce raccourci **et** un remplacement » ; `actions.rs` n'en a qu'un (le raccourci marche : joué). La directrice utilise un clavier français. Métier : `specialiste-produit` / `dev-interface`.
- **m7 — Dialogue Pousser lent à devenir utilisable** : 0,9 à 1,6 s pour lister **un** commit sortant, boutons grisés pendant ce temps (états à +0,1 / +0,3 / +0,5 / +0,8 s : `loaded: false` ; +1,3 s : vrai). Cause probable : une dizaine d'appels `git` séquentiels sous Windows. Conséquence : `branches-push-crochet.txt` (attend 600 ms) échoue systématiquement (« bouton « push » grisé »), `branches-push-nouvelle-force` et `branches-ecrans` échouent sous charge. Métier : `dev-git` (performance), `testeur-qa` (attentes des scénarios).
- **m8 — `menus-arbre.txt` est périmé** : il clique `rclick 120 149` ; depuis que l'index écrit `.juliestorm/` dans le projet, une ligne de plus s'affiche dans l'arbre, le clic tombe sur `LISEZMOI.md` : « vcs.add grisée : Rien à ajouter ». Le menu est juste, le scénario non. Métier : `testeur-qa`.
- **m9 — Textes** : la raison d'un fichier de réglages illisible est en anglais (« unclosed table, expected `]` ») ; « git introuvable » ne **nomme pas le réglage** (critère 15) ; « 1 commit du distant **seront** remplacés ». Métier : `redacteur`.
- **m10 — Critère 5 de `navigation.md` contredit la décision 9** : `new Boss('b')` et `new static($name)` ouvrent une **liste** (classe puis constructeur), pas un saut direct vers la classe. Le code suit la décision 9 ; la spec n'a pas été corrigée. Métier : `specialiste-produit` (corriger le critère).
- **m11 — Alt+F7 / Ctrl+clic sur une fonction JavaScript écrite dans un `<script>` d'un `.php`** : rien ne se passe, **aucun message** (22 des 30 symboles tirés dans `lab` étaient dans ce cas). Hors périmètre de la spec, mais l'absence de message laisse croire à un plantage. Métier : `dev-indexation-recherche`.
- **m12 — Messages d'état périmés** : « La fusion s'est arrêtée : 1 fichier en conflit à résoudre. » reste affiché après « Abandonner la fusion » et après l'abandon du cherry-pick. Métier : `dev-interface`.
- **m13 — Un fichier non suivi écrasé lors de la remise** : `login.txt` (non suivi, mon contenu) remplacé par la version suivie de `feature/login` lors de « Mettre de côté et basculer » ; le contenu reste dans `stash@{0}^3` seulement et la notification ne le mentionne pas (seulement « Remise en conflit… »). Rien de perdu, à signaler. Métier : `dev-interface`.
- **m14 — Ouvrir un projet écrit `.juliestorm/`** (cache des symboles et `.gitignore` interne `*`) : la décision 8 demande « sans rien écrire, pas même `.juliestorm/` » sur le vrai `backoffice`. Ce n'est pas tenable avec l'index sur disque : **ouvrir le vrai `backoffice` dans JulieStorm créera `backoffice\.juliestorm\`** (ignoré par Git grâce à son propre `.gitignore`). C'est pourquoi je n'ai pas ouvert le vrai `backoffice` (critère 3 de `reglages.md` non vérifié). Métier : `specialiste-produit` (corriger la décision).
- **m15 — Rétablir** remet le contenu mais pas l'état d'index : un ajouté (`A `) rétabli redevient `??`. Cosmétique, conforme au critère 6 (contenu octet pour octet).
- **m16 — Un lancement bloqué 4 minutes, non reproduit** : la première exécution de `recette-v1-b10-force-bail.txt` (dépôt avec une branche divergée du distant) n'a rien écrit (log vide, aucun état) pendant les 240 s avant d'être tuée par `timeout` ; deux rejeux immédiats sur le même dépôt : 10 s. Je ne sais pas si c'est l'IDE ou l'outil de lancement (la commande avait été renvoyée en arrière-plan par l'outil au même moment). À surveiller.

## (c) Non testé, et pourquoi

- **Critère 3** (une bascule faite au terminal apparaît dans Récentes) : la lecture du reflog est prouvée par les branches du décor, pas par le terminal.
- **Raccourcis Alt+`** (liste rapide) et la touche Menu (critère 17 de `menus-contextuels.md`), Maj+F6, Suppr, Alt+Inser, Ctrl+Maj+C (critère 19, vérifié seulement pour Ctrl+Alt+A) ; « Ouvrir un terminal ici » (12), copier le chemin **relatif** (la valeur du presse-papiers n'a été lue que pour l'absolu), copier le hash (15), jeu de données piégé lancé par clic droit (14), Coller du terminal (16) : les entrées existent et les scénarios existants se jouent sans échec, mais je n'ai pas relu les valeurs.
- **Réglages** : critère 3 (vrai `backoffice`, volontairement non ouvert, voir m14), critère 11 (édition à la main dans l'éditeur + Ctrl+S, prise en compte en moins de 2 s), critère 9 « exactement deux arguments » (je n'ai vu que la commande affichée et le refus du guillemet), `.git` et `.juliestorm` non retirables de la liste d'exclusion, « grisé dans l'arbre ».
- **Navigation** : critères 26 et 27 (performances sur `lab` : jamais ouvert, règle de sécurité) ; qualité des sauts sur appels de méthodes d'objets dans du vrai code (mes cibles étaient surtout des fonctions).
- **Branches** : chronométrage des critères 27 (< 2 s) et 28 (< 1 s), 2 000 branches distantes (< 300 ms), authentification, `git worktree`, verrou `index.lock`, branche protégée côté serveur, signature des commits ; **comparaison avec PhpStorm** (rien n'y a été touché).
- **Fusion en vrai code** (`fusion-reel.py` sur `labmate_paper` et PHPUnit) : non rejouée, j'ai étendu l'énumération comme demandé ; les 8 scénarios `split-*` (travail en cours d'une autre développeuse, hors v1).
- `tests-prerequis.txt` : décor reconstitué à la main (le chemin de PHP faux a été mal écrit par mon script : le scénario se joue, mais ce cas précis n'a pas été vérifié).

## (d) Chiffres de la fusion et de la navigation

### Fusion : « nous propre, Git en conflit » = **0** sur 12 461 triplets

Le moteur `julie_diff::merge` (via `mergecli` compilé hors de l'espace de travail) contre `git merge-file -p`. Familles de `tests/fixtures/fusion-enum.py` (la contre-recette v0 recommandait bases de trois lettres et fichiers plus longs) :

| Famille | Triplets | Deux propres, même texte | Deux propres, texte différent | **Nous propre / Git en conflit** | Conflit en trop (Git propre) | Deux en conflit |
|---|---|---|---|---|---|---|
| Essai de calibrage `enum3` (1 base) | 1 089 | 891 | 0 | **0** | 6 | 192 |
| `enum3` : bases de 3 lettres (a, b, g), 8 lignes, **toutes** les paires (une opération chez nous × une chez eux : remplacement, suppression, insertion de a, b ou g à chaque position), 4 bases | 4 557 | 3 739 | 0 | **0** | 0 | 818 |
| `enum3b` : la forme de la faille (base cadrée par `g`, nôtre = une suppression + une insertion de a, b ou g, leur = un remplacement par `Z`), énumération complète, 2 bases | 971 | 463 | 0 | **0** | 37 | 471 |
| `enum3c` : 2 opérations chez nous × 1 chez eux, 3 lettres, 8 lignes, 4 bases × 900 tirages | 3 360 | 2 352 | 0 | **0** | 45 | 963 |
| `long` : 20 à 40 lignes, alphabet de 2 ou 3 lettres, 2 à 5 opérations de chaque côté | 2 484 | 1 253 | 0 | **0** | 204 | 1 027 |
| **Total** | **12 461** | 8 698 | **0** | **0** | 292 (2,3 %) | 3 471 |

Les conflits en trop sont tous du côté sûr (lignes répétées : on garde un conflit plus large que celui de Git ; exemple relu : insertions de `a` et `b` dans une suite de `a` chez nous, remplacement + insertion dans la même suite chez eux). La faille de la v0 (un seul cas sur 1 608 en énumération sur 2 lettres) n'est **pas revenue** avec les bases de trois lettres, ni avec les fichiers plus longs. Réserve : l'énumération est exhaustive en positions mais pas en bases ; le tirage aléatoire seul ne la voyait pas en v0.

### Navigation
- **Projet d'essai** : 32 Ctrl+clics (`recette-v1-nav.py`) : **30 conformes**, 2 « liste au lieu de saut » (m10). Usages : 12 (5 + 1), 13 (1 + 1 ; 0 + 1), 14 (7 / 2 fichiers), 15 (2, 2, 1), 16 (saut direct), 20 (6 / 2 fichiers) : exacts. Aucun saut faux.
- **70 fichiers PHP réels copiés de `lab`** (`recette-v1-nav-reel.py`, copie dans le dossier temporaire, `lab` jamais ouvert) :
  - *Usages* : 30 symboles tirés (fonctions et méthodes à déclaration unique avec au moins un appel dans la copie) ; sur les **8 déclarations PHP** (plus une, `copyLicense`, qui est du JavaScript), toutes les lignes d'appel que `grep -w` trouve dans du code non dynamique sont trouvées (**0 manqué** ; 1 faux manque, un appel dans un commentaire) et rien en plus ; sur les **22 déclarées en JavaScript dans un `<script>`**, aucun usage et aucun message (m11) ; exemples : `lcNiimbotLookup` 6 / 6, `loadManifest` 1 / 1.
  - *Sauts* : 60 appels de fonctions tirés : **13 sauts exacts à la déclaration**, 45 sans saut (JavaScript dans PHP), **0 saut faux avéré** (deux cas « FAUX » du vérificateur sont des clics tombés ailleurs sur des lignes de 48 tabulations : le saut obtenu est celui de la variable réellement cliquée, vérifié à la main).
  - *Lenteurs* : 30 Alt+F7 enchaînés, index compris, en 23 s au total (build de développement) ; la construction de l'index de 70 fichiers et la réouverture avec cache sont instantanées (`index_ready` vrai dès le premier état).

### Non-régression (scénarios)
- Existants joués : **74** (les 8 `split-*` sont le chantier d'une autre développeuse). **Réussis : 72. Échecs : 2** (`menus-arbre` : m8, `branches-push-crochet` : m7). Échecs passagers sous charge (enumération de la fusion en parallèle), **rejoués seuls et réussis** : `branches-ecrans`, `branches-push-nouvelle-force`. **Blocages : 0** (sauf m16, hors scénarios existants). **Paniques : 0** sur environ 170 lancements de l'IDE.
- Les 45 scénarios `recette-v1-*.txt` ajoutés rejouent mes essais.

## (e) L'avis net

- **« Les branches de la même manière que PhpStorm » : en grande partie oui sur les gestes, pas encore sur la confiance.** Tout ce qu'on fait de cette fenêtre (bascule, création avec nom vérifié, fetch, mise à jour par fusion ou rebase, push refusé / forcé avec bail / protégé / crochet, fusion et rebase en conflit jusqu'au bout, cherry-pick, revert, reset, suppression-restauration, graphe) donne le même état que `git status`, `git log --graph --all`, `git branch -vv` et le dépôt nu, et refuse proprement quand Git refuse (aucun `--force` nu, aucune écriture distante sans confirmation, aucun verrou forcé). Mais PhpStorm, lui, **remet vos changements après une mise à jour** (M1), **vous dit où est passé votre travail** après un « forcer » (M2) et **vous prévient quand vous quittez une HEAD détachée** (M3). Ces trois-là doivent être corrigés avant de dire « comme PhpStorm » à quelqu'un qui travaille toute la journée dedans. M5 (rebase : « nôtre » inversé) et M4 (arbre périmé) se verront dès la première semaine.
- **Réglages PHPUnit pour backoffice : oui.** La détection du phar (9.6.39), la commande, l'écriture sans bruit dans un fichier TOML édité à la main (une seule ligne change, CRLF et commentaires intacts), le refus d'un fichier cassé (octet pour octet), la priorité projet / utilisateur, l'environnement, les arguments entre guillemets et l'exécution partielle ou « relancer les échecs » égalent PHPUnit en ligne de commande. Réserve m14 : ouvrir le vrai `backoffice` créera un dossier `.juliestorm/` ignoré par Git (contraire à la décision 8, qu'il faut corriger plutôt que le code).
- **Clics droits : oui.** Toutes les zones ont leur menu, les raisons des grisés sont toutes renseignées, la sélection multiple se comporte, la corbeille est la vraie corbeille, « Annuler les modifications » nomme ce qu'il fait et se rétablit. À reprendre : M2 (le Rétablir des opérations Git), m1 (message brut), m2 (cherry-pick déjà présent), m4.
- **Ctrl+clic : oui, avec une honnêteté sur la mesure.** Sur le projet d'essai tout est conforme (usages exacts, saut direct, liste honnête quand le type est inconnu), et sur 70 fichiers de vrai code PHP je n'ai trouvé ni saut faux ni usage manqué (hors code dynamique). Je n'ai pas pu mesurer sur `lab` en entier et la moitié des symboles tirés étaient du JavaScript dans du PHP, que l'index ne lit pas (m11 : au moins un message).
- **Fusion : toujours la propriété intouchable.** 0 résolution silencieuse sur 12 461 triplets nouveaux (bases de trois lettres, fichiers de 40 lignes) en plus des 31 600 de la v0.

## État de `labmate_paper` et de `backoffice`

- **`labmate_paper`** : jamais ouvert dans l'IDE (un clone local jetable a servi à `ouvrir-et-chercher` et aux scripts `labmate-*.sh`) ; `main`, `README.md` modifié non validé (le travail de la directrice) et `.idea/` non suivi comme au début, les deux branches `juliestorm-essai/eux` et `/nous` d'avant, aucun `.juliestorm/`.
- **`backoffice`** : seulement **lu** (copie de `phpunit.xml`, du phar et du leurre) ; jamais ouvert dans l'IDE, aucune commande Git n'y a été lancée par moi, la suite de tests n'y a jamais été lancée (ni sur les copies : j'ai exécuté le phar sur la suite d'essai de `tests/fixtures/phpunit`, qui ne contient aucun test réseau).
- **Corbeille Windows** : mes essais y ont laissé **3 fichiers de test** (`essai-corbeille.txt` du dossier `jsqa\mn3` et deux `nouveau.txt` du dossier `jsessai-phpunit`, dont un de `menus-clics-droits.txt`) : à vider si on veut ; ce sont mes artefacts, rien d'autre.
- Dossiers `%TEMP%\jsqa` et `%TEMP%\jsessai-*` créés par moi : supprimés à la fin (sauf `jsessai-split*`, qui sont à une autre développeuse).

## Fichiers livrés (dans `tests/`)

`tests/fixtures/recette-v1-run.sh`, `recette-v1-nav.py`, `recette-v1-nav-reel.py`, `fusion-enum.py`, et `tests/scenarios/recette-v1-*.txt` (branches `b01` à `b26`, réglages `reg1` à `reg8`, menus `menu1` à `menu4` et `menu-tests*`, navigation `nav-*`).

---

# Contre-recette

Faite le 25/11/2026 par la testeuse sur `target\debug\juliestorm.exe` reconstruit au commit `c27b431` (arbre propre au départ : les
corrections de la recette et le fractionnement des éditeurs sont dedans). Mêmes règles que plus haut : mode scénario, dossiers
jetables de `%TEMP%` (`jsqa2\…`, `jsessai-*`), `JULIESTORM_STATE_DIR` neuf à chaque lancement, dépôts nus locaux comme seuls distants,
aucun commit ni push, aucune modification du code de l'application. `labmate_paper` n'a servi qu'à un `git clone` (jamais ouvert dans
l'IDE) ; `backoffice` n'a été que lu (copie de `phpunit.xml` et du phar par `backoffice-copie.sh`).

Ajouts dans `tests/` : `tests/fixtures/contre-recette.sh` (cas `pick`, `revert`, `deux`, `detachee-branche`) et 26
scénarios `tests/scenarios/contre-*.txt` (`contre-m1-*`, `contre-m3-*`, `contre-m4-pull`, `contre-m5-*`, `contre-m6-m13`,
`contre-split-git-a` à `d`).

## Verdict de la contre-recette

**Les cinq majeurs sont clos, y compris sur les variantes que la développeuse n'avait pas jouées, et je n'ai trouvé aucune perte de
travail : ni bloquant, ni majeur nouveau.** Douze mineurs sont clos, un l'est en partie (m13), deux ne le sont pas (m10, m15), m16
reste inexpliqué. Le fractionnement des éditeurs tient ses 10 critères et ses interactions avec Git. Il reste six mineurs nouveaux,
tous cosmétiques ou de texte, sauf un comportement de Ctrl+S à trancher (N4). **La v1 peut être déclarée terminée**, sous la réserve
que les scénarios devenus périmés par les corrections elles-mêmes (b17, b19) soient réécrits et que m10 (une phrase de spécification)
soit corrigé.

## Chiffres tenus

| Contrôle | Résultat |
|---|---|
| `cargo test --workspace` | **700 réussis, 0 échec, 14 ignorés** (33 suites, une seule exécution) |
| `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check` | propres |
| Scénarios non-split lancés en lot (110 sur 133) | 97 réussis, 13 échecs tous expliqués (décor non reconstitué ou scénario périmé par une correction), 0 régression du produit |
| Scénarios `split-*` | **8 sur 8 réussis** |
| Scénarios de cette contre-recette | 26, états relus un à un |
| Processus restants | aucun `juliestorm`, `cargo`, `rustc` de ma part |

## Tableau des écarts

| Écart | Verdict | Preuve |
|---|---|---|
| **M1** mise de côté jamais remise (mise à jour en conflit) | **Clos** | `recette-v1-b06` : après « Résoudre » puis « Valider », `git status` = `M LISEZMOI.md`, `?? nv.txt`, `git stash list` vide, `a.txt` fusionné, notification « Fusion terminée ; vos changements locaux ont été remis. ». `b07` (abandon) : « Fusion abandonnée ; vos changements locaux ont été remis. » (m12 aussi). `b08` (rebase, « Garder votre version ») : « Rebase terminé ; vos changements locaux ont été remis. », `a.txt` = `DEUX (A)`. Pendant le conflit, le bandeau et la notification disent que le travail est mis de côté et reviendra (capture regardée) |
| M1, variante **cherry-pick refusé puis mise de côté** (`contre-m1-pick*.txt`) | **Clos** | dialogue « Mettre de côté et reprendre le commit » ; conflit : bandeau « reviendront à la fin du cherry-pick » ; résolu côté « Prendre » : la remise tombe en conflit, `UU a.txt` avec les deux versions dans le fichier, l'entrée reste dans `git stash list` (état `Kept`), notification « Remise de vos changements en conflit… reste mise de côté » : rien perdu. Cherry-pick devenu vide (« Garder votre version ») : « La validation a échoué » (voir N6), puis « Passer ce commit » : « …vos changements locaux ont été remis », `a.txt` et `nv.txt` revenus |
| M1, variante **revert en conflit** (`contre-m1-revert.txt`) | **Clos** | refus, mise de côté, `AwaitingOperation(Revert)`, « Abandonner » : « Revert abandonné ; vos changements locaux ont été remis », `a.txt` = `deux (final)` + `trois (local)`, `LISEZMOI.md` modifié, `nv.txt` présent, `stash list` vide |
| M1, variante **mise à jour en conflit, fermeture de l'IDE, réouverture** (`contre-m1-deux-a` à `d`) | **Clos** | le lien est dans `.git/JULIESTORM_PENDING_STASH` ; rouvert en pleine fusion : bandeau encore là (`AwaitingOperation(Merge)`) ; résoudre + valider : remise ; abandonner après redémarrage : remise ; `git merge --abort` fait au terminal : bandeau « Remettre mes changements » (`OperationOver`), un clic remet tout |
| M1, **deux opérations successives** (`contre-m1-succ-a`, `-b`) | **Clos ; la limite annoncée existe mais sans perte** | cherry-pick en conflit avec mise de côté n°1, IDE fermé, `git cherry-pick --abort` au terminal ; nouveaux changements locaux ; rouvert : bandeau `OperationOver` pour n°1 ; deuxième cherry-pick avec mise de côté n°2 ; « Abandonner » : n°2 remise (`a.txt` = `trois (local2)`, `nv2.txt` présent) ; n°1 reste dans `git stash list`, son bandeau passe de `OperationOver` à `Kept` (voir N5) ; « Remettre » la n°1 ne s'applique pas sur des fichiers déjà modifiés : « n'ont pas pu être remis… reste mise de côté », entrée gardée. Aucune n'est perdue ni mélangée |
| **M2** bascule forcée / reset dur sans copie visible | **Clos** | `b04` : « Basculé sur feature/login. Jetés : a.txt, login.txt (copie gardée, « Rétablir » les remet). Ont suivi : n.txt, u.txt. », `toast.action = restore_copy:…`, « Rétablir » : « 2 fichiers rétablis », contenus identiques aux originaux. `b12` : « Branche réinitialisée sur c72432e. Jetés : a.txt, c3.txt, n.txt (copie gardée… ) », « 3 fichiers rétablis ». `corr-reset-stash` : bandeau « Mise de côté gardée » + « Remettre » |
| **M3** HEAD détachée sans avertissement | **Clos** | `b13` : après le commit, bandeau « Vos commits ne sont sur aucune branche (1 commit) » + « Créer une branche ici » ; sortie vers `main` : dialogue « 1 commit n'est sur aucune branche » (sujet nommé), 3 boutons ; Annuler : HEAD toujours sur le commit. `corr-detachee-branche` : « Créer une branche d'abord » puis bascule, la branche `sauvetage/orphelin` existe. `quand-meme` : « 1 commit laissé sur aucune branche, récupérable : « Créer une branche » sur e9918dd », le bouton ouvre « Nouvelle branche » sur ce commit |
| M3, variante **Nouvelle branche avec bascule depuis une HEAD détachée qui porte un commit** (`contre-m3-nouvelle-bascule`) | **Clos (rien n'est quitté en silence)** | avec « basculer » coché et le départ `bugfix/crash` : dialogue « 1 commit n'est sur aucune branche », **sans « Basculer quand même »** (`can_anyway: false`), HEAD reste sur le commit orphelin, la branche demandée n'existe pas. Texte du dialogue : voir N1 |
| M3, variantes **bascule sur une distante, mise à jour, reset dur** | **Clos** | `origin/main` : même dialogue, HEAD inchangée ; Ctrl+T : « HEAD est détachée : créez d'abord une branche. » ; reset dur vers un ancien commit : le dialogue annonce `commits_leaving: 1, unreachable: 1` avant la seconde confirmation (le commit reste ensuite dans le reflog) |
| **M4** arbre du projet périmé | **Clos** | `b14` : l'arbre est juste dès le premier état après la bascule (`c1`, sans `c2` ni `c3`) ; `corr-onglet-bascule` : onglet de `c3.txt` en `notice: Deleted`, arbre à jour ; mise à jour qui apporte `b1.txt`, `b2.txt` : l'arbre les montre aussitôt (`contre-m4-pull`) |
| **M5** « notre / leur » inversé pendant un rebase | **Clos** | rebase (`b09`, capture regardée) : volets « **Votre commit** — main : A : ligne 2 », « Résultat », « **Amont** — origin/main : B : ligne 2 », aide « … à gauche votre commit, à droite l'amont (Git les nomme à l'envers) » ; « Garder votre version » donne `DEUX (A)` (vérifié dans le fichier). **Cherry-pick** : « Votre version — main : … » / « Commit repris — 5ad6777 : Connexion : a.txt ligne 2 », boutons « Garder votre version » / « Prendre 5ad6777 ». **Revert** : « Annulation du commit — 79bb19b : Main : a.txt ligne 2 ». **Remise d'une mise de côté en conflit** : « Vos changements mis de côté » / « Branche mise à jour — main… », « Garder votre version » = le contenu de la mise de côté (vérifié : `trois (local)` conservé) |
| **m1** texte `Debug` dans la barre d'état | **Clos** | fusion avec `a.txt` modifié : dialogue « Des changements locaux gênent la fusion de feature/login » (fichier nommé), « Mettre de côté et fusionner » (idem cherry-pick et revert) |
| **m2** cherry-pick d'un commit déjà présent | **Clos** | `pick-present` : « Cherry-pick » grisé, raison « Déjà sur la branche courante (ce commit ou le même changement). » |
| **m3** « Restaurer » perd l'amont | **Clos** | `git branch -vv` : `bugfix/crash … [origin/bugfix/crash]` après Restaurer |
| **m4** sous-menu de branche | **Clos** | « Voir le diff avec l'arbre de travail » présent (liste « M a.txt, A c1.txt, D login.txt… » puis diff) ; aucune entrée « Basculer et rebaser » sur `origin/feature/login` |
| **m5** branche sans amont | **Clos** | Ctrl+T : « sans-amont n'a pas de branche distante associée… » **avant** toute question ; boutons « Choisir l'amont… », « Pousser pour la créer… » ; choisir `origin/main` : « sans-amont suit maintenant origin/main » ; « Retirer l'amont » : « n'a plus d'amont » |
| **m6** pas de remplacement AZERTY | **Clos** | `Ctrl+Maj+B` ouvre la fenêtre des branches (`vcs.branches_alt`) |
| **m7** dialogue Pousser lent | **Clos** | états toutes les 0,15 s : boutons grisés à +0,1 s, **utilisable avec le commit sortant listé à +0,3 s** (avant : 0,9 à 1,6 s) ; `branches-push-crochet` réussit |
| **m8** `menus-arbre.txt` périmé | **Clos** | réécrit (`ui menu.open tree`), joué : réussi |
| **m9** textes | **Clos** | fichier de réglages illisible : « table non fermée (« ] » attendu) » ; « Git est introuvable : « … ». Vérifiez le réglage « Exécutable Git » (Réglages, page Git). » ; « 1 commit du distant sera remplacé… » / « n commits… seront remplacés » (lus dans `texts.rs`, non rejoués à l'écran) |
| **m10** critère 5 de `navigation.md` | **Pas clos** | `docs/specs/navigation.md` ligne 356 dit toujours « `Boss` dans `new Boss('b')` → classe `Admin` » (saut direct) alors que le code ouvre une liste (décision 9) ; `specialiste-produit` |
| **m11** JavaScript d'un `<script>` | **Clos** | Ctrl+clic et Alt+F7 : « Le JavaScript d'un <script> écrit dans un fichier PHP n'est pas encore indexé : pas de déclaration à trouver. » ; une fonction PHP du même fichier saute (ligne 2) |
| **m12** message d'état périmé | **Clos** | après « Abandonner » le message est remplacé (« Fusion abandonnée ; vos changements locaux ont été remis. », « Cherry-pick abandonné… », « Revert abandonné… ») |
| **m13** non suivi écrasé lors de la remise | **Partiel** | rien n'est perdu (la version de l'utilisatrice reste dans `stash@{0}^3`, l'entrée est gardée, le bandeau « Mise de côté gardée… Remettre » reste), mais **la notification ne nomme pas le fichier** : `login.txt` non suivi + bascule vers `feature/login` : « Vos changements n'ont pas pu être remis : « avant bascule… » reste mise de côté. » (sans conflit) ou « Remise… en conflit » (avec conflit). `untracked_note` n'est appelé que quand la remise réussit. Dans les deux cas `login.txt` vaut la version de `feature/login` sur le disque |
| **m14** `.juliestorm/` à l'ouverture | **Clos** | la décision 8 de `decisions-v1.md` dit maintenant qu'ouvrir un projet crée `.juliestorm/` (ignoré par Git) |
| **m15** « Rétablir » ne remet pas l'index | **Pas clos** (cosmétique) | inchangé : un fichier ajouté (`A `) rétabli redevient non suivi (`??`) ; le contenu est exact |
| **m16** lancement bloqué | **Non reproduit** | un lancement (`contre-m3-reset`) est resté plus de 4 min avec un seul état écrit, **pendant que je faisais tourner un second IDE en parallèle** (erreur de ma part) ; trois rejeux seuls : 12 s chacun. Même signature que le blocage d'origine (premier lancement, rien d'écrit). Hypothèse : deux IDE simultanés peuvent s'étouffer ; à ne pas écarter |

## Fractionnement des éditeurs (`docs/specs/split.md`)

Les 8 scénarios `split-*` réussissent (rc 0, 0 échec) ; états relus un à un (`target\essais\split\`).

| Critère | Verdict | Preuve |
|---|---|---|
| 1 fractionner à droite, frappe visible des deux côtés, Ctrl+S une seule fois | Conforme | deux groupes de 482 px, `IntroScene.js` à `views: 2`, `modified: true` dans les deux vues après la frappe, `false` partout après Ctrl+S |
| 2 déplacer en bas | Conforme | `LevelScene.js` seul dans le groupe du bas, absent du haut |
| 3 Ctrl+Maj+N + Entrée dans le groupe actif | Conforme | `PauseScene.js` ouvert dans le groupe 1 actif |
| 4 séparateur à la souris, proportion gardée | Conforme | `drag` : 482 → 206 px ; 1280×720 → 1920×1080 : proportions gardées |
| 5 trois niveaux en 1280×720 | Conforme | `depth: 4` utilisable à 100 % et 150 %, thème clair, plein écran |
| 6 fermer le dernier onglet d'un groupe | Conforme | le groupe disparaît, les autres restent |
| 7 tout regrouper | Conforme | un groupe, chaque fichier une fois |
| 8 persistance | Conforme | trois groupes avant fermeture, les mêmes (rectangles et onglets) après réouverture |
| 9 fermer une vue d'un document modifié | Conforme | aucune question ; la dernière pose `close_tab:…/config.js` |
| 10 aucune régression | Conforme | voir la section suivante |

Interactions avec Git (scénarios `contre-split-git-a` à `d`, plus essais au clic) :
- **Fichier ouvert dans deux groupes puis supprimé par une bascule** (`c3.txt`, via le journal) : un seul document, le bandeau « Supprimé du disque » est dans les **deux** vues (`notice: Deleted`), l'arbre est à jour ; retour sur `main` : `notice: Changed` dans les deux vues, rien perdu.
- **Modifié par une bascule** : non modifié, les deux vues se rechargent. Modifié non enregistré : bandeau « Le fichier a changé sur le disque » dans les **deux** vues (capture regardée), le texte tapé est gardé. Voir N4 pour Ctrl+S.
- **Dialogue de fusion à trois volets ouvert pendant que le fichier est ouvert en split** (`a.txt` avec marqueurs) : accepter le bloc et enregistrer : les **deux vues se rechargent** (8 lignes → 4, sans marqueurs), disque = `DEUX (B)`. Avec du texte non enregistré dans un groupe : bandeau `Changed` dans les deux vues, le texte tapé est conservé, le disque porte la fusion : aucun écrasement.
- **Annuler les modifications** sur un fichier ouvert dans deux groupes avec une frappe non enregistrée : la confirmation nomme l'onglet non enregistré ; après confirmation les deux vues montrent `HEAD` ; « Rétablir » rend le contenu.
- **Ctrl+clic dans un groupe inactif** (clic aux coordonnées relevées sur capture) : le groupe devient actif et le saut a lieu dans le groupe cliqué. Saut dans le même fichier : ligne 40 dans le groupe cliqué, l'autre groupe garde sa position (33:37). Saut vers `User.php` : nouvel onglet dans le groupe cliqué, pas dans l'autre.

## Non-régression des scénarios

Lot de 133 scénarios non-split (hors `contre-*` et `split-*`), chacun sur son décor reconstitué, un seul IDE à la fois.

- **110 lancés en lot : 97 réussis, 13 échecs.** Réussis : `git-*`, `nav-*`, `ecarts-*`, `recette-*` (v0, terminal compris), `branches-*` (15 sur 15 après relance de `branches-ecrans`), `journal-graphe`, `menus-arbre`, `menus-clics-droits`, `tests-*`, `reglages-*`, `recette-v1-reg1` à `reg8`, `recette-v1-nav-*`, `recette-v1-b01` à `b05`, `b11`, `b22` à `b26`.
- **Les 13 échecs, tous expliqués, aucun n'est une régression du produit** :
  - `branches-ecrans` : 1 échec sur 4 passes (« aucun dialogue Git ouvert » après 500 ms) ; 3 relances seules réussies. Fragile sous charge, comme avant.
  - `journal-graphe` : décor trompé (je lui avais donné `tout`) ; sans option : réussi.
  - **`recette-v1-b17-amont` et `recette-v1-b19-merge-sale` : périmés par les corrections** (m5 : le message « pas d'amont » arrive avant le dialogue Fusion / Rebase ; m1 : fusion sale = dialogue « Mettre de côté… » au lieu du bouton « close »). À réécrire ; `corr-sans-amont` et `corr-merge-sale` les remplacent.
  - `b10`, `b15` (avec un mauvais décor), `b16`, `b18`, `b21`, `menu-tests2`, `menu1`, `menu2`, `menu4` : **décors de ma recette précédente (dépôts divergés, fichiers `added.txt`, `src/nouveau.php`, test « l'apostrophe… », distant cassé à la main) que je n'ai pas su reconstituer**. `b15` rejoué sur `branches.sh` sans option : réussi. Les autres sont **non revérifiés**.
- **Hors lot, rejoués sur le bon décor et réussis** : `recette-v1-b04`, `b06` à `b09`, `b12` à `b14`, les 14 `recette-v1-corr-*` (dont `corr-ecrans-a` / `-b` et `corr-m1-terminal-a` / `-b`, à 100 et 150 %, thème clair, plein écran).
- **Non rejoué** : `recette-v1-b20-graphe-gros` (dépôt de ~10 000 commits).
- **Blocages : 1** (m16, causé par deux IDE simultanés). **Paniques : 0** sur plusieurs centaines de lancements. Erreur de ma part : deux pilotes de lot ont tourné ensemble quelques minutes au départ ; je les ai arrêtés, vidé les résultats et recommencé : les chiffres viennent de la seconde exécution, seule.

## Nouveaux constats

Aucun bloquant, aucun majeur.

- **N1 (mineur) — texte du dialogue de HEAD détachée pour « Nouvelle branche avec bascule »** : « « Créer une branche d'abord » en crée une sur ce commit, puis la bascule reprend. » Pour une *création* de branche avec bascule (`contre-m3-nouvelle-creer-d-abord`), rien ne reprend : on crée `sauve` sur le commit orphelin et on est dessus ; la branche demandée au départ (`essai-nouvelle` depuis `bugfix/crash`) n'est pas créée. Rien n'est perdu ; le texte promet plus que l'action. `redacteur` / `dev-interface`.
- **N2 (mineur) — titre du dialogue de réinitialisation en HEAD détachée** : « Réinitialiser  sur c460bab » (deux espaces : le nom de branche est vide). `redacteur`.
- **N3 (mineur) — bandeau de conflit à 150 %** : « Rebase en cours — commit 1 sur 1 — 1 fichier en c » est coupé par le bouton « Résoudre » (capture `ecrans-a-1280-150.png`) et le message de la barre d'état est tronqué. `dev-interface`.
- **N4 (mineur, à trancher) — Ctrl+S pendant le bandeau « changé sur le disque »** (`contre-split-git-b`) : après une bascule qui réécrit `a.txt`, un onglet modifié non enregistré montre le bandeau (Recharger / Garder les miennes) dans les deux vues ; **Ctrl+S sans avoir cliqué** écrit le contenu de l'éditeur (`XYZun / deux (côté login) / trois`) par-dessus la version de `main` (`git status` : ` M a.txt`). La spec dit « Garder les miennes (le prochain Ctrl+S écrase) », ce qui laisse penser que c'est le clic qui arme l'écrasement. Récupérable ici par `git checkout` (fichier suivi), mais la bascule de branche rend ce cas facile à rencontrer. `dev-editeur` / `specialiste-produit`.
- **N5 (mineur) — état d'une mise de côté après une autre opération** : la n°1 passe de `OperationOver` à `Kept` ; le bouton passe de « Remettre mes changements » à « Remettre ». Cohérent avec la limite annoncée, sans perte ; seulement surprenant.
- **N6 (mineur) — « La validation a échoué »** : « Continuer » sur un cherry-pick devenu vide (on a gardé « votre version » partout) répond par ce titre générique ; la sortie de Git (« The previous cherry-pick is now empty… ») n'est que dans `commit_error`. La sortie réelle passe par « Passer ce commit », mais le message ne le dit pas. `redacteur` / `dev-interface`.

## Non vérifié

- `recette-v1-b20` et les décors perdus de `b10`, `b16`, `b18`, `b21`, `menu-tests2`, `menu1`, `menu2`, `menu4` : leurs critères ne sont pas revus par ce biais.
- Les textes de m9 ne sont lus que dans `texts.rs`, pas photographiés.
- Le glisser d'un onglet vers un autre groupe pendant une opération Git ; `split-glisser` ne fait que `drag` au pointeur synthétique.
- Concurrence : deux IDE sur le même dépôt, ou un IDE et la ligne de commande qui écrivent en même temps (hors `merge --abort` et `cherry-pick --abort` faits entre deux lancements).
- Comparaison avec PhpStorm (rien n'y a été touché) ; authentification, `git worktree`, distant réel.

## État laissé

`labmate_paper` : seulement cloné (copie jetable), jamais ouvert dans l'IDE, aucune branche créée dessus. `backoffice` : lu. Dossiers `%TEMP%\jsqa2`, `jsessai-*`, `jsetat-*` créés par moi : supprimés. Aucun commit, aucun push.
