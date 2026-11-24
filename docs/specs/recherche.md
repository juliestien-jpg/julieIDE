# Recherche (v0) : aller au fichier, recherche dans les fichiers

Rédigé le 24/11/2026 par `specialiste-produit`. **(observé)** / **(supposé)** /
**(écart)** : voir `fusion.md`. La recherche dans les fichiers de PhpStorm reste à
observer ; ce qui la décrit ici est **supposé** d'usage.

## À quoi ça sert

Ouvrir n'importe quel fichier en tapant quelques lettres de son nom, et retrouver
un texte dans tout le projet en voyant tout de suite chaque occurrence dans son
contexte.

## Portée commune

- Fichiers du projet ouvert, en respectant `.gitignore`, `.git/info/exclude` et
  l'exclusion globale de Git ; `.git` et `.juliestorm` toujours exclus.
- L'index des noms se construit à l'ouverture du projet en tâche de fond et suit le
  disque (moins de 2 s). Pendant la construction, les résultats sont partiels et un
  indicateur « Indexation… » est visible.

## Aller au fichier

- Ctrl+Maj+N : fenêtre flottante centrée, un champ, une liste **(observé pour la
  recherche globale)**. Échap ferme.
- Champ vide : les fichiers récemment ouverts de la session, le plus récent en haut
  (20 au plus) **(observé)**.
- Correspondance floue, sans casse : les caractères de la requête doivent apparaître
  **dans l'ordre** dans le chemin relatif. Une requête avec `/` ou `\` fait
  correspondre les segments de dossier (`ctrl/user` trouve
  `src/Controller/UserController.php`). Les espaces séparent des morceaux qui
  doivent tous correspondre, dans l'ordre.
- Ordre des résultats, du plus fort au plus faible :
  1. nom exact (sans extension) ;
  2. nom qui commence par la requête ;
  3. correspondance dans le nom en morceaux consécutifs ou en initiales de mots
     (`uc` → `UserController`) ;
  4. correspondance qui déborde sur le chemin ;
  à égalité : fichier ouvert récemment, puis chemin le plus court, puis ordre
  alphabétique.
- Chaque ligne : nom avec les lettres correspondantes surlignées, chemin du dossier
  en gris **(observé)**. 100 résultats au plus, « et n autres » en bas.
- Suffixe `:42` (ou `:42:7`) : ouvre à la ligne 42 (colonne 7).
- Flèches haut/bas, Entrée ouvre et ferme la fenêtre ; la liste se met à jour à
  chaque frappe en moins de 50 ms sur 50 000 fichiers.
- Aucun résultat : « Aucun fichier ne correspond ».

## Recherche dans les fichiers

- Ctrl+Maj+F : fenêtre avec, en haut, le champ et les options ; au milieu, la liste
  des résultats ; en bas, l'aperçu. Si du texte est sélectionné dans l'éditeur
  (une ligne), il pré-remplit le champ.
- Options (cases, raccourcis dans la fenêtre) : respecter la casse (Alt+C), mots
  entiers (Alt+W), expression régulière (Alt+X). Filtre de fichiers : masques
  séparés par des virgules (`*.php, *.js`), vide = tous. Portée : projet entier ou
  un dossier (pré-rempli avec le dossier sélectionné dans l'arbre).
- Recherche lancée à chaque modification, 300 ms après la dernière frappe ; une
  recherche en cours est annulée par la suivante. Requête de moins de 2 caractères :
  rien n'est lancé.
- Résultats en flux, regroupés par fichier, une ligne par occurrence : numéro de
  ligne, texte de la ligne avec l'occurrence surlignée (lignes longues tronquées
  autour de l'occurrence). En tête : « n occurrences dans m fichiers », avec
  « recherche en cours » tant que ce n'est pas fini.
- Arrêt à 10 000 occurrences : « Plus de 10 000 résultats, précisez la recherche ».
- **Aperçu** : le fichier de l'occurrence sélectionnée, coloré, centré sur la ligne,
  occurrences surlignées, en lecture seule (**écart** : PhpStorm permet d'y éditer).
- Entrée ou double-clic : ouvre le fichier à l'occurrence, sélectionnée, et ferme
  la fenêtre. Échap ferme.
- Les onglets modifiés non enregistrés sont cherchés **dans leur contenu en
  mémoire**, pas sur le disque.
- Expression invalide : message sous le champ, pas de recherche, pas d'erreur
  fenêtre.

## Cas limites

Fichiers binaires (octet NUL) ignorés ; fichiers de plus de 10 Mio ignorés et
comptés (« 2 fichiers trop gros ignorés ») ; fichiers Windows-1252 décodés comme
dans l'éditeur ; CRLF : une regex `$` correspond avant le `\r` ; occurrence en
fin de fichier sans saut final ; projet sans `.gitignore` ; dépôt sans Git ;
`vendor` et `node_modules` (inclus seulement s'ils ne sont pas ignorés par Git) ;
noms de fichiers accentués ; plusieurs occurrences sur une même ligne (une ligne de
résultat, toutes surlignées, comptées chacune).

## Critères d'acceptation

1. Sur le projet d'essai, Ctrl+Maj+N puis `usct` propose `UserController.php` en
   premier si ce fichier existe.
2. Un fichier ignoré par `.gitignore` n'apparaît ni dans Aller au fichier ni dans la
   recherche dans les fichiers.
3. Champ vide : les fichiers ouverts dans la session apparaissent, le dernier en
   haut.
4. `index.php:12` ouvre `index.php` curseur en ligne 12.
5. Recherche de `function` avec « mots entiers » ne trouve pas `functions`.
6. Avec « casse », `Foo` ne trouve pas `foo`.
7. Regex `\$\w+Id\b` trouve les variables PHP finissant par `Id`.
8. Regex `(` invalide : message sous le champ, aucun plantage.
9. Masque `*.php` exclut les `.js`.
10. Le compteur « n occurrences dans m fichiers » est égal au résultat de
    `git grep -c` (mêmes options) sur le projet d'essai.
11. Un texte tapé dans un onglet non enregistré est trouvé.
12. Taper vite une requête puis une autre : seuls les résultats de la dernière restent.
13. L'interface reste utilisable pendant une recherche de 5 s.

## Hors périmètre v0

Remplacer dans les fichiers, recherche partout (double Maj), classes, symboles,
actions, recherche multiligne, portées nommées, historique des recherches, épingler
les résultats dans une fenêtre d'outils, édition dans l'aperçu, fichiers hors projet.
