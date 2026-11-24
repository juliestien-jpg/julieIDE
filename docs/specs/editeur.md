# Projet et éditeur (v0)

Rédigé le 24/11/2026 par `specialiste-produit`. **(observé)** / **(supposé)** /
**(écart)** : voir `fusion.md`.

## À quoi ça sert

Ouvrir mon projet, naviguer dans ses fichiers, les modifier avec une coloration
lisible et les enregistrer sans jamais abîmer encodage ni fins de ligne.

## Ouvrir un dossier, arbre du projet

- Menu Fichier → « Ouvrir un dossier… » (sélecteur de dossier Windows). Le dossier
  devient le projet ; titre de la fenêtre = nom du dossier. Au démarrage suivant,
  le dernier projet est rouvert.
- Fenêtre d'outils Projet à gauche (Alt+1) : arbre des dossiers puis fichiers, par
  ordre alphabétique sans casse, dossiers d'abord. `.git` masqué ; les fichiers
  ignorés par Git sont affichés en grisé (supposé comme PhpStorm).
- Double-clic ou Entrée : ouvre le fichier. L'arbre suit les créations, suppressions
  et renommages faits hors de l'IDE en moins de 2 s.

## Onglets

- Un onglet par fichier ; rouvrir un fichier déjà ouvert active son onglet.
- Titre = nom du fichier ; infobulle = chemin complet ; deux fichiers de même nom
  ouverts → le dossier parent est ajouté au titre.
- Fichier modifié non enregistré : marque visible sur l'onglet (astérisque).
- Fermer : Ctrl+F4, clic milieu, croix de l'onglet. Onglet modifié → « Enregistrer
  les modifications de <nom> ? » Enregistrer / Ne pas enregistrer / Annuler.
- Fermer l'IDE avec des onglets modifiés : même question, une fois, avec la liste.
- Alt+Gauche / Alt+Droite : onglet précédent / suivant. Ctrl+Tab : bascule vers
  l'onglet précédemment actif (écart : PhpStorm ouvre un sélecteur ; v0 simple).
- Trop d'onglets : ils défilent horizontalement, l'onglet actif est toujours visible.

## Coloration

- Par extension : `.php`, `.phtml` → PHP (avec le HTML hors des balises `<?php ?>`) ;
  `.html`, `.htm` → HTML ; `.css` → CSS ; `.js`, `.mjs`, `.cjs` → JavaScript. Le
  contenu de `<script>` et `<style>` dans le HTML (et dans le HTML d'un fichier PHP)
  est coloré en JS et CSS. Toute autre extension : texte brut.
- Catégories minimales : mot-clé, chaîne, nombre, commentaire, variable PHP, nom de
  fonction, balise HTML, attribut, propriété CSS, sélecteur CSS. Les couleurs
  relèvent du thème.
- Un fichier syntaxiquement faux reste coloré au mieux, sans message.
- La coloration suit la frappe sans délai perceptible (re-analyse incrémentale).

## Édition, annuler, rétablir

- Frappe, sélection (souris, Maj+flèches), copier/couper/coller (Ctrl+C/X/V), tout
  sélectionner (Ctrl+A), Tab insère l'indentation du fichier (détectée ; à défaut
  4 espaces), Entrée reprend l'indentation de la ligne.
- Ctrl+Z annule, Ctrl+Maj+Z rétablit. **Ctrl+Y supprime la ligne**, comme PhpStorm
  (choix délibéré : la personne qui dirige le projet a ce réflexe).
- Groupement : une frappe continue forme une seule étape ; l'étape se clôt à un
  déplacement du curseur (flèches, clic), un saut de ligne, un collage, une pause
  de plus d'une seconde. Un collage = une étape. Annuler replace le curseur là où
  était le changement.
- L'historique survit à l'enregistrement ; il est perdu à la fermeture de l'onglet.
  Annuler jusqu'à l'état enregistré fait disparaître la marque de modification.

## Enregistrer

- Ctrl+S enregistre **tous** les onglets modifiés (comme PhpStorm). Pas
  d'enregistrement automatique en v0 (**écart** : PhpStorm enregistre à la perte de
  focus ; ici on veut que chaque écriture soit voulue).
- Écriture atomique (temporaire puis renommage) ; l'attribut lecture seule ou un
  refus d'accès → message, contenu conservé dans l'onglet, marque maintenue.
- Encodage : UTF-8 avec ou sans BOM, conservé. Fichier non UTF-8 valide → ouvert en
  Windows-1252 et réenregistré en Windows-1252 ; un caractère non représentable
  bloque l'enregistrement de ce fichier avec un message. Si le fichier contient un
  octet non défini en Windows-1252 (0x81, 0x8D, 0x8F, 0x90, 0x9D), il s'ouvre en
  lecture seule.
- Fins de ligne : chaque ligne existante garde la sienne ; une ligne nouvelle prend
  le style dominant du fichier (LF si fichier vide ou nouveau). Le saut final est
  conservé tel quel (ni ajouté ni retiré).
- Barre d'état : ligne:colonne, encodage, fins de ligne (CRLF / LF / mixte).

## Fichier modifié hors de l'IDE

- Détection par surveillance du disque (moins de 2 s) et au retour du focus.
- Onglet **non modifié** : rechargé sans question, curseur gardé sur la même ligne
  (bornée à la fin du fichier) ; l'historique d'annulation est vidé.
- Onglet **modifié** : bandeau « Le fichier a changé sur le disque » avec
  Recharger (perd mes modifications, après confirmation) / Garder les miennes (le
  prochain Ctrl+S écrase). Jamais de fusion silencieuse.
- Fichier **supprimé** hors de l'IDE : l'onglet reste, bandeau « Supprimé du
  disque » ; Ctrl+S le recrée.
- Fichier **renommé** hors de l'IDE : traité comme supprimé (supposé ; suivre un
  renommage est hors v0).

## Cas limites

Fichier binaire (octet NUL dans les 8 premiers Kio) : non ouvert, message « Fichier
binaire ». Fichier de 5 à 50 Mio : ouvert sans coloration avec bandeau ; au-delà de
50 Mio : non ouvert. Ligne de 100 000 caractères : affichée sans gel. Fichier vide.
Fichier sans saut final. Fins de ligne mêlées. BOM UTF-8. Caractères accentués et
emoji. Tabulations (largeur 4).

## Critères d'acceptation

1. Ouvrir un dossier affiche son arbre ; `.git` n'y est pas.
2. Ouvrir deux fois le même fichier ne crée qu'un onglet.
3. Un `.php` mêlant HTML, PHP et `<script>` montre les trois colorations.
4. Taper « abc » d'un trait puis Ctrl+Z retire « abc » entier ; Ctrl+Maj+Z le remet.
5. Ctrl+Y supprime la ligne du curseur.
6. Ouvrir, modifier puis enregistrer un fichier CRLF sans BOM, un LF avec BOM, un
   fichier mixte et un fichier sans saut final : en dehors des lignes touchées, le
   fichier est identique octet pour octet à l'original.
7. Un fichier Windows-1252 contenant « é » est affiché correctement et réenregistré
   avec l'octet 0xE9.
8. Ctrl+S enregistre tous les onglets modifiés et retire leurs marques.
9. Fermer un onglet modifié pose la question ; « Annuler » le garde ouvert et modifié.
10. Modifier un fichier non modifié dans l'IDE depuis un autre programme : l'onglet
    montre le nouveau contenu en moins de 2 s.
11. Même chose avec un onglet modifié : bandeau, aucun contenu perdu sans clic.
12. Un PHP de 10 000 lignes s'ouvre coloré en moins de 1 s ; la frappe n'y montre
    aucun retard visible.
13. Un fichier en lecture seule : Ctrl+S affiche une erreur, l'onglet reste modifié.

## Hors périmètre v0

Rechercher/remplacer dans le fichier, aller à la ligne (Ctrl+G), curseurs multiples,
pliage, complétion, inspections, formatage, JSX/TSX/TypeScript, gabarits AngularJS,
onglet d'aperçu, écran partagé, restauration des onglets au démarrage, choix
manuel de l'encodage, suivi des renommages externes, création/suppression de
fichiers depuis l'arbre.
