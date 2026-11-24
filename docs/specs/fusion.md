# Fusion à trois volets (v0)

Rédigé le 24/11/2026 par `specialiste-produit`. Repères : **(observé)** = vu dans
PhpStorm et consigné dans `docs/observations-phpstorm.md` ; **(supposé)** = connu
d'usage, pas ré-observé sur un vrai conflit ; **(écart)** = on fait autrement que
PhpStorm, exprès.

## À quoi ça sert

Quand Git s'arrête sur un conflit, résoudre un fichier en voyant côte à côte ma
version, la leur et le résultat, sans jamais perdre une ligne de l'une ou de l'autre.

## D'où viennent les trois textes

- **Gauche** = « la nôtre » (étape 2 de l'index, `git show :2:<chemin>`), lecture seule.
- **Droite** = « la leur » (étape 3), lecture seule.
- **Centre** = résultat éditable, qui **part de la base** (étape 1) **(observé)**.
  S'il n'y a pas de base (ajout des deux côtés), le centre part vide.
- Chaque volet porte un titre : « Notre version », « Résultat », « Leur version »,
  suivi du nom de branche quand Git le donne (`MERGE_HEAD`, branche courante).
  Pendant un rebase ou un cherry-pick, une ligne d'aide sous les titres rappelle
  que Git appelle « nôtre » la branche sur laquelle on rejoue (supposé : PhpStorm
  ne l'explique pas toujours ; on préfère le dire).
- Le dialogue s'ouvre depuis la liste des fichiers en conflit (voir `git.md`).
  Il ne s'ouvre que pour un fichier texte des deux côtés (voir Cas limites).

## Classification des blocs (règle exacte, pour `dev-fusion-diff`)

1. Diff par lignes base→nôtre et base→leur. Les lignes se comparent **sans leur fin
   de ligne** (CR LF et LF sont la même ligne) ; l'absence de saut de ligne final
   fait partie de la dernière ligne (« b » sans saut ≠ « b » avec saut).
2. Chaque changement couvre un intervalle de la base `[début, fin)` (vide pour une
   insertion). Deux changements de côtés opposés **se touchent** si leurs
   intervalles se chevauchent **ou sont contigus** (fin de l'un = début de l'autre),
   deux insertions au même point comprises. Les changements qui se touchent, de
   proche en proche, forment **un bloc**.
3. Nature d'un bloc :
   - **changement de gauche** : seul notre côté a changé ;
   - **changement de droite** : seul leur côté a changé ;
   - **changement identique** : les deux côtés ont produit exactement le même texte
     sur le bloc → non conflictuel, appliqué une seule fois ;
   - **conflit** : tout le reste. Une différence d'espaces seule suffit à faire un
     conflit. **Dans le doute, conflit.**
4. Couleurs par nature (modifié, ajouté, supprimé, conflit), formes courbes reliant
   le bloc d'un volet à l'autre **(observé)**. Les couleurs exactes relèvent du thème.

## Le comportement

- À l'ouverture, **rien n'est appliqué** : le centre montre la base, tous les blocs
  sont « en attente » (supposé : c'est le réglage par défaut de PhpStorm).
- **Gouttières** entre les volets, par bloc et par côté : une flèche « accepter »
  et une croix « ignorer » **(observé)**.
  - *Accepter* un côté remplace le texte du bloc au centre par celui de ce côté.
  - *Ignorer* un côté le marque traité sans toucher au centre.
  - Sur un conflit, accepter un côté **résout le bloc** ; l'autre côté garde une
    flèche « ajouter à la suite » : l'utiliser insère son texte **après** celui déjà
    accepté. Accepter les deux = les deux textes l'un après l'autre, **dans l'ordre
    des clics** **(observé pour la mise à la suite, supposé pour l'ordre)**.
  - Ignorer un côté d'un conflit ne le résout pas : il faut aussi accepter ou
    ignorer l'autre. Les deux ignorés → le centre garde son texte actuel.
  - Un bloc non conflictuel identique n'a qu'une paire flèche/croix.
- **Barre d'outils** **(observé)** : bloc précédent, bloc suivant, et un bouton
  « appliquer les changements non conflictuels » avec trois choix :
  - *tous* : applique changements de gauche, de droite et identiques ;
  - *de gauche* : changements de gauche et identiques ;
  - *de droite* : changements de droite et identiques.
  Les conflits ne sont jamais touchés par ce bouton.
- **Compteur** en tête : « n changements et m conflits restants » (n = blocs non
  conflictuels en attente, m = conflits en attente). À 0 et 0 : « Tout est traité »
  et le bouton d'enregistrement est mis en avant **(observé)**.
- **Édition libre du centre** : on peut taper partout. Taper dans un bloc en attente
  ne le résout pas ; ses flèches restent actives et accepter remplace alors le texte
  tapé (annulable).
- **Annuler/rétablir** (Ctrl+Z / Ctrl+Maj+Z) au centre couvre la frappe **et** les
  clics accepter/ignorer/appliquer, compteur compris.
- **Défilement synchronisé** des trois volets **(observé)** : les blocs
  correspondants restent alignés.
- **Bloc courant** : celui où se trouve le curseur du centre, ou le dernier atteint
  par F7 / Maj+F7. Les raccourcis d'acceptation agissent sur lui.
- **Enregistrer et marquer résolu** (bouton « Appliquer ») :
  - s'il reste des blocs en attente, confirmation : « Il reste n changements et
    m conflits. Les blocs non traités garderont le texte actuel du centre. » —
    Enregistrer quand même / Revenir ;
  - écrit le centre dans le fichier de travail (écriture atomique), avec l'encodage,
    la marque BOM et les fins de ligne décrites plus bas ;
  - si le fichier a changé sur le disque depuis l'ouverture du dialogue : demande
    écraser / revenir ;
  - lance `git add -- <chemin>` ; le fichier quitte la liste des conflits ; le
    dialogue se ferme. Si `git add` échoue : message avec la sortie de Git, le
    fichier est écrit mais reste en conflit.
- **Fermer** (Échap, croix, « Annuler ») : si rien n'a été fait, fermeture directe ;
  sinon confirmation « Abandonner la fusion de ce fichier ? ». Dans les deux cas
  **rien n'est écrit** : le fichier reste en conflit, marqueurs `<<<<<<<` compris,
  exactement comme avant.

### Fins de ligne, encodage

- Le résultat suit **notre version** : encodage (UTF-8 avec ou sans BOM) et fins de
  ligne. Ligne par ligne : une ligne restée identique à notre version garde sa
  propre fin de ligne ; toute autre ligne (venue de la leur, de la base, ou tapée)
  prend le style dominant de notre version (le plus fréquent ; à égalité, celui de
  la première ligne). Ainsi un fichier mixte n'est pas normalisé en douce.
- Si les styles de fin de ligne diffèrent entre les trois versions, bandeau
  d'information : « Les fins de ligne diffèrent entre les versions ; le résultat
  utilisera celles de notre version (CRLF / LF). » (écart : PhpStorm le signale
  moins clairement ; une conversion de fin de ligne perdue doit se voir).
- Saut de ligne final : la dernière ligne du résultat a un saut de ligne final si
  et seulement si la version dont elle vient en avait un à cet endroit ; une ligne
  tapée au centre en dernière position en reçoit un si notre version en avait un
  (cas 18 à 20).

## Cas limites

- **Fichier binaire** (octet NUL dans les 8 premiers Kio de l'un des côtés) ou
  **non UTF-8** : pas de dialogue ; seuls « garder la nôtre / prendre la leur »
  dans la liste des conflits (écart : en v0 on ne fusionne que de l'UTF-8 ; les
  fichiers Windows-1252 se résolvent par choix de côté ou dans le terminal).
- **Supprimé d'un côté, modifié de l'autre** : pas de dialogue ; la liste propose
  « garder le fichier (modifié) » ou « le supprimer » (voir `git.md`).
- **Ajouté des deux côtés** : dialogue avec base vide (cas 22).
- **Fichier énorme** : au-delà de 20 Mio ou 200 000 lignes, pas de dialogue, choix
  de côté seulement, avec message.
- **Sous-module, lien symbolique** : choix de côté seulement.
- **Marqueurs déjà présents dans la base** (`<<<<<<<` dans le texte) : traités comme
  du texte ordinaire ; on ne relit jamais les marqueurs du fichier de travail, on
  part toujours des étapes 1/2/3.

## Raccourcis

F7 / Maj+F7 bloc suivant / précédent ; Alt+Gauche accepter le côté gauche du bloc
courant ; Alt+Droite accepter le côté droit ; Alt+Maj+Gauche / Alt+Maj+Droite
ignorer le côté gauche / droit ; Ctrl+Entrée enregistrer et marquer résolu ; Échap
fermer. Hors F7, ce sont **nos propositions** (supposé que PhpStorm n'a pas de
raccourci par défaut stable pour accepter un côté). Voir `raccourcis.md`.

## Cas d'essai (base / nôtre / leur → attendu)

Notation : `\n` = LF, `\r\n` = CRLF, `∅` = fichier vide. « G/D/I/C » = nombre de
blocs changement de gauche / de droite / identique / conflit. « Tous » = résultat
après « appliquer les changements non conflictuels → tous ».

| # | Base | Nôtre | Leur | Classement | Résultat attendu |
|---|---|---|---|---|---|
| 1 | `a\nb\nc\n` | `a\nB\nc\n` | = base | G1 | Tous → `a\nB\nc\n` |
| 2 | `a\nb\nc\n` | = base | `a\nb\nC\n` | D1 | Tous → `a\nb\nC\n` |
| 3 | `a\nb\nc\nd\ne\n` | b→`B` | d→`D` | G1 D1 | Tous → `a\nB\nc\nD\ne\n` ; « de gauche » seul → `a\nB\nc\nd\ne\n`, compteur « 1 changement » |
| 4 | `a\nb\nc\n` | b→`X` | b→`X` | I1 | Tous → `a\nX\nc\n` (X une seule fois) |
| 5 | `a\nb\nc\n` | b→`B1` | b→`B2` | C1 | Tous → inchangé (`a\nb\nc\n`), compteur « 1 conflit » |
| 6 | `a\nb\nc\nd\n` | b→`B` | c→`C` | C1 (contigus : **s'abstenir**) | Tous → inchangé |
| 7 | `a\nb\n` | `a\nX\nb\n` | `a\nY\nb\n` | C1 | accepter G puis ajouter D → `a\nX\nY\nb\n` ; accepter D puis ajouter G → `a\nY\nX\nb\n` |
| 8 | `a\nb\n` | `a\nX\nb\n` | `a\nX\nb\n` | I1 | Tous → `a\nX\nb\n` |
| 9 | `a\nb\nc\n` | `a\nc\n` | b→`B` | C1 (suppression contre modification : **s'abstenir**) | Tous → inchangé |
| 10 | `a\nb\nc\n` | `a\nc\n` | `a\nc\n` | I1 | Tous → `a\nc\n` |
| 11 | `a\nb\nc\n` | `a\nX\nb\nc\n` | b→`B` | C1 (insertion contiguë : **s'abstenir**) | Tous → inchangé |
| 12 | `  b\n` | `    b\n` | `  B\n` | C1 (espaces seuls d'un côté : **s'abstenir**) | Tous → inchangé |
| 13 | cas 1 | | | G1 | ignorer G → compteur 0 ; enregistrer → fichier = `a\nb\nc\n` |
| 14 | cas 3 en `\r\n` partout | | | G1 D1 | Tous → `a\r\nB\r\nc\r\nD\r\ne\r\n` |
| 15 | `a\nb\n` | `a\nB\n` | `a\r\nb\r\n` | G1 (leur conversion seule n'est pas un changement) | Tous → `a\nB\n` + bandeau fins de ligne |
| 16 | `a\r\nb\r\n` | = base | `a\nb\nc\n` | D1 | Tous → `a\r\nb\r\nc\r\n` |
| 17 | `a\r\nb\nc\r\n` | = base | `a\r\nb\nC\r\n` | D1 | Tous → `a\r\nb\nC\r\n` (b garde son LF) |
| 18 | `a\nb` | `a\nB` | = base | G1 | Tous → `a\nB` (pas de saut final) |
| 19 | `a\nb` | = base | `a\nb\n` | D1 | Tous → `a\nb\n` |
| 20 | `a\nb` | `a\nB` | `a\nb\n` | C1 (même dernière ligne : **s'abstenir**) | Tous → inchangé |
| 21 | `a\n` | `a\nX\n` | `a\nY\n` | C1 | Tous → inchangé |
| 22 | `∅` | `x\ny\n` | `x\nz\n` | C1 (un seul bloc, lignes communes comprises) | accepter G → `x\ny\n` |
| 23 | `a\n` | `∅` | `A\n` | C1 | Tous → inchangé |
| 24 | cas 5 | | | C1 | accepter G → `a\nB1\nc\n`, compteur 0 ; Ctrl+Z → `a\nb\nc\n`, compteur « 1 conflit » |
| 25 | BOM UTF-8 + cas 1 | idem avec BOM | = base | G1 | fichier écrit commence par EF BB BF |
| 26 | cas 4 et 6 dans le même fichier, séparés par 3 lignes | | | I1 C1 | Tous → seul le bloc identique appliqué |

## Critères d'acceptation

1. Chacun des cas 1 à 26 donne exactement le classement et le résultat indiqués
   (test automatisé dans `crates/diff`, octet pour octet).
2. Sur un dépôt d'essai en conflit, ouvrir le dialogue montre nôtre à gauche, base au
   centre, leur à droite ; gauche et droite refusent la frappe.
3. À l'ouverture, aucun bloc n'est appliqué et le compteur vaut le nombre de blocs.
4. « Appliquer non conflictuels → tous / de gauche / de droite » applique exactement
   les natures listées plus haut et ne modifie aucun conflit.
5. Accepter, ignorer, ajouter à la suite et l'édition libre mettent le compteur à
   jour immédiatement ; Ctrl+Z annule chacune de ces actions, compteur compris.
6. Les trois volets défilent ensemble ; un bloc reste aligné sur ses correspondants.
7. F7 / Maj+F7 vont au bloc en attente suivant / précédent et le rendent courant.
8. Enregistrer avec des blocs en attente demande confirmation ; « Revenir » n'écrit
   rien.
9. Après enregistrement, le fichier sur disque est le texte du centre, avec
   l'encodage, la BOM et les fins de ligne de notre version (règle ligne par ligne),
   `git status` ne le montre plus en conflit, et il a disparu de la liste.
10. Fermer après une action demande confirmation ; dans tous les cas de fermeture
    sans enregistrer, le fichier sur disque est identique octet pour octet à avant
    l'ouverture et reste en conflit.
11. Un fichier binaire, non UTF-8, supprimé d'un côté, ou de plus de 20 Mio n'ouvre
    pas le dialogue et ne propose que le choix de côté.
12. Un fichier de 10 000 lignes avec 200 blocs s'ouvre en moins de 2 s ;
    « appliquer tous » répond en moins de 0,5 s.
13. Le bandeau de fins de ligne apparaît pour le cas 15 et pas pour le cas 14.

## Hors périmètre v0

- Résolution automatique des conflits simples (« baguette magique »).
- Surlignage au mot près dans les blocs.
- Application automatique des non-conflictuels à l'ouverture (réglage).
- Ignorer les espaces ou les fins de ligne dans la comparaison (option).
- Fusion des fichiers non UTF-8, binaires, d'images.
- Diff entre la base et un côté (vue à deux volets depuis le dialogue).
