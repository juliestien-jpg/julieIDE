# PHPUnit (v0)

Rédigé le 24/11/2026 par `specialiste-produit`. **(observé)** / **(supposé)** /
**(écart)** : voir `fusion.md`.

## À quoi ça sert

Lancer toute la suite de mon projet en un raccourci, voir les résultats arriver en
direct et sauter d'un échec à la ligne fautive.

## Préalables

- **PHP** : réglage « chemin de php » ; par défaut
  `C:\Program Files (x86)\AgileBio\LabCollector\php\php.exe` s'il existe, sinon
  `php` du PATH.
- **PHPUnit** : `vendor/bin/phpunit` du projet (on lance le script PHP sous-jacent
  avec `php`, pas le `.bat`). Absent → message « PHPUnit introuvable dans
  vendor/bin » et rien n'est lancé.
- **Configuration** : `phpunit.xml`, sinon `phpunit.xml.dist`, à la racine du
  projet. Absente → message « Aucun phpunit.xml à la racine du projet ». En v0, une
  seule configuration, implicite (pas de fenêtre de configurations).
- Commande : `php <phpunit> --configuration <xml> --teamcity`, dossier de travail =
  racine du projet. Elle est affichée en tête de la console.

## Le comportement

- Ctrl+Maj+F10 (ou bouton Lancer de la barre du haut, ou clic droit sur
  `phpunit.xml` → Lancer) : la fenêtre d'outils Exécution (Alt+4) s'ouvre en bas.
- **Barre en tête** **(observé)** : barre de progression (verte tant que tout passe,
  rouge dès un échec ou une erreur), compteurs en direct « n réussis, n échoués,
  n en erreur, n ignorés sur N » et durée écoulée. N vient du décompte annoncé par
  PHPUnit ; tant qu'il est inconnu, « sur ? ».
- **Arbre à gauche** **(observé)** : racine → suite(s) → classe → méthode → jeu de
  données (nom du jeu tel que PHPUnit le donne). Chaque nœud : état (en cours,
  réussi, échoué, erreur, ignoré/incomplet/risqué), durée. Un parent prend l'état le
  plus grave de ses enfants. Les nœuds apparaissent au fil de l'exécution.
- Par défaut, les nœuds en échec ou en erreur sont dépliés, les autres repliés
  **(observé)**. Les réussis restent visibles (**écart** : PhpStorm les masque par
  défaut ; le filtre est hors v0).
- **Console à droite** : sortie du nœud sélectionné (racine = toute la sortie :
  commande, sortie standard, erreurs). Pour un échec : message, attendu / obtenu en
  texte s'ils sont fournis, pile d'appels.
- **Liens** : tout `chemin:ligne` de la console qui désigne un fichier existant est
  souligné et cliquable → ouvre le fichier à la ligne **(observé)**. Double-clic sur
  un test dans l'arbre → ouvre son fichier à la méthode (ligne donnée par
  `locationHint`, à défaut début du fichier).
- **Arrêter** (Ctrl+F2) : tue le processus et ses enfants ; les tests non terminés
  passent à « interrompu » ; le résumé dit « Arrêté ».
- **Relancer** (Ctrl+F5) : relance toute la suite, l'arbre précédent est remplacé.
- Fin : résumé « Tests réussis : n sur N — 12,4 s » ou « n échecs, n erreurs »,
  notification si la fenêtre n'est pas visible.

## Cas limites

- **PHP plante** (erreur fatale, mémoire épuisée, `exit()` dans un test) : le test
  en cours passe en erreur avec la fin de la sortie standard d'erreur comme
  message ; les tests jamais commencés ne sont pas inventés ; résumé « Exécution
  interrompue (code de sortie n) ».
- Aucun test trouvé : « Aucun test exécuté » (pas de barre rouge).
- Code de sortie non nul sans échec (avertissements) : vert, avec la mention.
- Sortie non TeamCity mêlée (echo, var_dump) : rattachée au test en cours, sinon à
  la racine.
- Chemins Windows avec espaces et lettres de lecteur dans les traces ; chemins
  relatifs (résolus depuis la racine du projet).
- Noms de jeux de données contenant `'`, `|`, `[` ou des retours à la ligne
  (échappements TeamCity).
- 5 000 tests : l'arbre reste fluide, mise à jour au moins toutes les 200 ms.
- Lancer pendant une exécution : la première est arrêtée après confirmation.

## Critères d'acceptation

1. Sur le projet PHP d'essai de `tests/fixtures/`, Ctrl+Maj+F10 lance la suite ;
   la commande exacte apparaît en tête de la console.
2. Les nœuds apparaissent pendant l'exécution, avant la fin (vérifiable avec un
   test qui dort 2 s).
3. Les compteurs finaux égalent ceux du résumé texte de PHPUnit pour la même
   commande sans `--teamcity`.
4. Un test avec fournisseur de données à 3 jeux donne 3 nœuds enfants nommés.
5. Un test en échec est déplié ; sa console montre le message et un lien
   `fichier:ligne` qui ouvre le bon fichier à la bonne ligne.
6. Un test qui appelle `exit(1)` : résumé « Exécution interrompue », le test en
   cours est en erreur, l'IDE ne plante pas.
7. Arrêter pendant un test qui dort 30 s : plus aucun processus `php` lancé par
   l'IDE dans le gestionnaire des tâches en moins de 2 s.
8. Sans `phpunit.xml` ni `phpunit.xml.dist` : message dédié, aucun processus lancé.
9. Avec un chemin de php invalide : message nommant le chemin, aucun plantage.
10. L'interface reste utilisable pendant toute l'exécution.

## Hors périmètre v0

Découverte des tests et icônes dans la gouttière, lancer une classe / méthode / jeu
de données, plusieurs configurations d'exécution, relance des échecs, comparaison
attendu/obtenu en diff, masquer les réussis, tri, historique des exécutions,
couverture, débogage, interpréteurs distants, Pest/ParaTest.
