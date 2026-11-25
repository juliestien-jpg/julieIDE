# Ce que l'on a observé dans PhpStorm

Observations faites le 24/11/2026 sur le PhpStorm de cette machine (2025.3.4,
projet `lab` ouvert), en lecture seule. Décrites avec nos mots : ce document dit
**ce que ça fait**, pas comment c'est dessiné. Tenu par `specialiste-produit`.

## Disposition générale

- Barre du haut : nom du projet, branche courante (cliquable), configuration
  d'exécution avec lancer / déboguer, loupe de recherche globale, réglages.
- Bande d'icônes à gauche et à droite : chaque icône ouvre ou replie une fenêtre
  d'outils. Projet en haut à gauche ; exécution, terminal, problèmes, Git en bas.
- Centre : éditeurs à onglets. Bas : la fenêtre d'outils active, sur toute la largeur.
- Barre d'état : fil d'Ariane du fichier, version de PHP, position du curseur, branche.
- L'arbre du projet colore les dossiers et fichiers selon leur état.

## Fenêtre Git

Des onglets : changements locaux, étagère, journal, console, et un onglet par
résultat de mise à jour.

- **Changements locaux** : un arbre de **listes de changements nommées** (une par
  sujet en cours, avec le nombre de fichiers) et un groupe des fichiers non suivis.
  À droite, l'aperçu du diff du fichier sélectionné. Barre verticale d'actions :
  rafraîchir, valider, annuler les changements, mettre de côté, voir le diff, grouper.
- **Étagère** : une liste d'entrées datées. Beaucoup sont créées **automatiquement**
  (« changements non validés avant bascule de branche / avant mise à jour ») avec
  la liste de changements d'origine et le nombre de fichiers ; une corbeille des
  entrées récemment supprimées. À retenir : l'IDE met de côté tout seul avant une
  opération risquée, et sait restaurer.
- **Journal**, trois colonnes :
  - à gauche, l'arbre des branches (HEAD, locales rangées en dossiers par préfixe
    `bugfix/`, `feature/`…, distantes), avec recherche et favoris ;
  - au centre, la liste des commits avec **graphe en couloirs colorés**,
    étiquettes de branches, date relative, auteur ; filtres en tête : texte ou
    hash (regex, casse), branche, auteur, date, chemins ; les commits de fusion
    sont grisés ;
  - à droite, les fichiers du commit sélectionné en arbre, puis le message complet.
- **Console** : toutes les commandes Git lancées par l'IDE et leur sortie.

## Fusion à trois volets (connu d'usage, à ré-observer sur un dépôt d'essai)

- D'abord une liste des fichiers en conflit avec, par fichier : accepter la nôtre,
  accepter la leur, fusionner.
- Le dialogue : gauche = notre version (lecture seule), droite = la leur (lecture
  seule), **centre = résultat éditable** qui part de la base commune.
- Chaque bloc est coloré selon sa nature (modifié, ajouté, supprimé, conflit) et
  relié d'un volet à l'autre par une forme courbe.
- Dans les gouttières entre volets, par bloc : une flèche pour l'accepter, une
  croix pour l'ignorer. Accepter les deux côtés d'un conflit les met à la suite.
- Barre d'outils : bloc précédent/suivant, **appliquer tous les changements non
  conflictuels** (des deux côtés, ou d'un seul), **résoudre les conflits simples**
  (la « baguette magique » : même ligne modifiée à des endroits différents).
- Un compteur dit ce qu'il reste : n changements, m conflits. Quand tout est
  traité, proposition d'enregistrer et de marquer le fichier résolu.
- Défilement synchronisé des trois volets ; surlignage au mot près dans les blocs.

## Recherche globale

Une fenêtre flottante au centre : un champ, et des onglets — tout, classes,
fichiers, symboles, actions, texte — plus une case « inclure hors projet » et un
filtre par type. Avant toute frappe, la liste montre les **fichiers récents**.
Chaque résultat : icône du type, nom, chemin abrégé en gris. Tout au clavier.

## Exécution des tests

- À gauche, l'**arbre des résultats** : racine → suite → classe → méthode → jeu
  de données (« with data set "…" »), chacun avec son état et sa durée.
- En tête : « 5 395 tests réussis, 5 ignorés sur 5 400, 4 min 8 s » et une barre
  de progression colorée.
- À droite, la **console** du nœud sélectionné : message, et un lien
  `fichier:ligne` cliquable vers le test.
- Barre d'outils : relancer, relancer les échecs, arrêter, afficher ou non les
  réussis et les ignorés, trier, déplier, historique des exécutions.
- Par défaut seuls les tests à problème sont dépliés ; les réussis sont masqués.

## Configuration PHPUnit du projet backoffice (25/11/2026, fichiers lus, rien modifié)

Regardé dans les fichiers de projet de PhpStorm de `backoffice` (`.idea`), en
lecture seule, pour comprendre le besoin ; rien n'en est recopié.

- PhpStorm 2025.3.4 est ouvert sur `backoffice`, branche `master` (barre du haut :
  nom du projet, puis widget de branche, puis à droite la configuration
  d'exécution « backoffice » avec lancer / déboguer).
- Le projet a **deux** configurations d'exécution PHPUnit nommées : une pour la
  racine (`phpunit.xml`) et une pour un sous-projet imbriqué qui a son propre
  `phpunit.xml.dist` et son propre `vendor` (`api/licenses_v2/plugins/mustache_php`).
  La configuration courante est retenue par projet.
- Les réglages PHPUnit de PhpStorm sont **par projet** (un chargeur automatique,
  un fichier de configuration par défaut, plusieurs dossiers de tests déclarés).
- Sur le disque : `tools/phpunit/phpunit.phar` (PHPUnit 9.6.39, vérifié par
  `php tools\phpunit\phpunit.phar --version`), un **dossier** `tools/phpunit/`, et
  dans `vendor/bin/` un `phpunit.rar` mais pas de `phpunit` : une détection par nom
  approchant se tromperait.

## Widget de branche (25/11/2026, tentative)

Clic sur le widget de branche tenté puis interrompu : rien d'observé au-delà de sa
place dans la barre du haut (à droite du nom du projet, il affiche le nom de la
branche courante, `master` sur `backoffice`). La fenêtre surgissante, ses sous-menus,
le dialogue de push et le choix fusion / rebase de la mise à jour restent à
observer. Les menus contextuels et la fenêtre de réglages n'ont pas pu être
observés : l'outil de contrôle de l'ordinateur n'autorise dans un IDE que le clic
gauche simple (ni clic droit, ni Ctrl+clic, ni raccourci, ni Échap).

## Ce qu'il reste à observer

Le dialogue de fusion sur un vrai conflit (dépôt d'essai), la création et
l'application d'un patch, le commit partiel par blocs, la recherche dans les
fichiers, les icônes de lancement de test dans la gouttière, le diff
attendu/obtenu d'un test en échec ; la fenêtre surgissante des branches et ses
sous-menus, le dialogue de push, la mise à jour du projet (choix fusion / rebase) ;
les menus contextuels de l'arbre, de l'éditeur et des changements ; la fenêtre de
réglages ; Ctrl+clic sur un usage et sur une déclaration (ces derniers demandent un
accès plus large que le clic simple, ou que la personne qui dirige le projet les
montre).
