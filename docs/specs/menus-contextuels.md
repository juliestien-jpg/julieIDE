# Menus contextuels

Rédigé le 25/11/2026 par `specialiste-produit`, d'après la demande du même jour
(« pouvoir faire `git add` sur un fichier par un clic droit ; il manque tous les
clics droits »). Repères : **(observé)** / **(supposé)** / **(écart)** : voir
`fusion.md`. Les menus contextuels de PhpStorm **n'ont pas pu être ré-observés**
(l'outil de contrôle de l'ordinateur n'autorise pas le clic droit dans un IDE) :
leur contenu est **supposé** d'usage. Les entrées sont décrites avec nos mots ; les
libellés définitifs sont ceux du catalogue de `redacteur`.

## À quoi ça sert

Faire l'action évidente sur ce que j'ai sous la souris (ajouter ce fichier à Git,
le renommer, relancer ce test…) sans chercher dans les menus du haut.

## Règles communes

- **Ouverture** : clic droit, **touche Menu** du clavier, ou **Maj+F10** (voir
  « Clavier » : écart avec PhpStorm à trancher). Le menu s'ouvre sous le pointeur ;
  au clavier, sous l'élément sélectionné (ligne d'une liste, position du curseur
  dans l'éditeur).
- **Sélection** : clic droit sur un élément hors de la sélection → la sélection
  devient cet élément ; sur un élément de la sélection → le menu agit sur **toute**
  la sélection. Les listes et arbres concernés (arbre du projet, changements,
  conflits, résultats de tests, résultats de recherche) acceptent la sélection
  multiple : Ctrl+clic, Maj+clic, Maj+flèches.
- **Une seule définition par action** : une entrée de menu contextuel est la même
  action que celle du menu du haut ou de la palette (même libellé, même raccourci
  affiché à droite, même règle de disponibilité). Pas de menu bâti à la main par
  zone.
- **Entrées grisées** : une action impossible **reste visible, grisée**, avec la
  raison en infobulle (« Ce dossier n'est pas dans un dépôt Git », « Aucun
  changement à annuler »…). Exception : un groupe entier qui n'a pas de sens dans la
  zone n'apparaît pas (pas de sous-menu Git dans le terminal). **(écart)** :
  PhpStorm masque souvent les entrées impossibles ; griser avec une raison
  explique mieux.
- **Sous-menus** : un niveau au plus (Nouveau ▸, Git ▸, Copier le chemin ▸).
  Séparateurs entre groupes. Ordre stable : on ne réordonne pas selon l'état.
- **Actions destructives** : toujours une confirmation qui **nomme** ce qui sera
  touché (jusqu'à 10 noms, puis « et n autres »). Bouton par défaut = l'action
  quand elle se défait (corbeille, filet de sécurité ci-dessous), **Annuler**
  sinon. Liste : annuler les modifications, supprimer, garder / prendre un côté
  d'un conflit quand le fichier a été retouché à la main, fermer des onglets
  modifiés (question déjà existante), fermer un terminal occupé (déjà existante).
- **Filet de sécurité de « Annuler les modifications »** (proposition, point à
  trancher) : avant d'écraser un fichier, l'IDE en garde une copie dans
  `.juliestorm/annulations/AAAAMMJJ-HHMMSS/<chemin>` ; la notification qui suit
  propose « Rétablir » pendant la session ; les copies de plus de 7 jours sont
  effacées à l'ouverture du projet. **(écart)** : PhpStorm a un historique local
  complet ; on fait le minimum qui empêche de perdre un travail par un clic.
- Après une action Git, l'état est rafraîchi (`Status` frais) et l'arbre du projet
  reprend les couleurs.
- Échap ferme le menu sans rien faire ; un clic ailleurs aussi.

## Clavier

- **Touche Menu** : ouvre le menu de la zone qui a le focus. Gardée par l'IDE même
  dans le terminal.
- **Maj+F10** : ouvre aussi le menu (convention Windows) — **(écart)** : dans
  PhpStorm, Maj+F10 lance la configuration d'exécution courante. **À trancher** par
  la personne qui dirige le projet (réflexe PhpStorm ou réflexe Windows). Dans le
  terminal, Maj+F10 va au shell.
- Dans le menu : Haut/Bas (en boucle, saute les séparateurs et les grisées),
  Droite ouvre un sous-menu, Gauche le referme, Entrée exécute, Échap ferme (un
  niveau à la fois). Pas de lettres soulignées.
- Les raccourcis affichés en face des entrées marchent aussi sans ouvrir le menu,
  dans la zone (voir `raccourcis.md`).

## Inventaire par zone

Colonnes : entrée ; quand elle est grisée (raison en infobulle) ; raccourci.
« Git ▸ » désigne le sous-menu Git commun décrit en premier.

### Sous-menu Git ▸ (sur un ou plusieurs fichiers ou dossiers)

Tout le sous-menu est grisé hors d'un dépôt (« Ce dossier n'est pas dans un dépôt
Git ») ou si Git est introuvable. Sur un dossier, chaque action porte sur les
fichiers concernés **qu'il contient**.

| Entrée | Effet | Grisée quand | Raccourci |
|---|---|---|---|
| Ajouter à Git | `git add -- <chemins>` : un non suivi devient ajouté, un modifié est indexé | aucun fichier non suivi ni modifié dans la sélection ; fichier ignoré par Git (« Fichier ignoré par .gitignore ») | Ctrl+Alt+A |
| Retirer de l'index | `git restore --staged -- <chemins>` (dépôt sans commit : `git rm --cached`) ; le fichier de travail n'est pas touché | rien d'indexé dans la sélection | — |
| Annuler les modifications… | ramène les fichiers à `HEAD` (index et travail) après confirmation ; détail plus bas | aucun changement suivi dans la sélection ; dépôt en fusion pour un fichier en conflit (« Résolvez le conflit ») | Ctrl+Alt+Z |
| Voir le diff | diff côte à côte `HEAD` / travail (existant) | fichier sans changement ; dossier | Ctrl+D |
| Historique du fichier | onglet Journal filtré sur ce chemin (`git log --follow`), filtre visible et retirable | non suivi ; dépôt sans commit ; plusieurs fichiers | — |
| Ajouter à .gitignore ▸ | ce fichier (`/chemin`), ce dossier (`/chemin/`), tous les `*.ext` : ajoute une ligne au `.gitignore` de la racine (créé s'il manque, fin de ligne du fichier respectée) | fichier déjà suivi (« Git suit déjà ce fichier : l'ignorer ne le retire pas ») | — |

**Annuler les modifications, en détail** :
- La confirmation liste les fichiers et dit ce qui arrivera à chacun : modifié ou
  supprimé → ramené à `HEAD` ; ajouté (absent de `HEAD`) → retiré de l'index, **le
  fichier reste sur le disque** sauf si la case « Supprimer aussi les fichiers
  ajoutés (vers la corbeille) » est cochée (décochée par défaut) ; renommé →
  l'ancien chemin revient, le nouveau suit la règle de l'ajouté.
- Un onglet ouvert non modifié est rechargé ; un onglet **modifié non enregistré**
  est signalé dans la confirmation (« les modifications non enregistrées de
  <nom> seront perdues ») et rechargé.
- Les non suivis ne sont jamais concernés (c'est « Supprimer »).
- Commande : `git restore --source=HEAD --staged --worktree -- <chemins>` ; dépôt
  sans commit : seulement retirer de l'index. Le filet de sécurité est pris avant.

### Arbre du projet

| Entrée | Effet | Grisée quand | Raccourci |
|---|---|---|---|
| Nouveau ▸ Fichier / Dossier | champ de nom dans une petite fenêtre ; créé dans le dossier sélectionné (ou celui du fichier sélectionné) ; un nom avec `/` ou `\` crée les dossiers intermédiaires ; le fichier créé (vide, écriture atomique) s'ouvre dans l'éditeur | sélection multiple | Alt+Inser |
| Ouvrir | ouvre le ou les fichiers | dossier | Entrée, F4 |
| Couper / Copier / Coller des fichiers | — | hors périmètre (pas affichées) | — |
| Copier le chemin ▸ | chemin absolu (Ctrl+Maj+C), chemin relatif au projet, nom seul ; plusieurs éléments = un par ligne | — | Ctrl+Maj+C |
| Renommer… | champ prérempli, nom sans l'extension sélectionné ; fichier suivi par Git → `git mv` (Git voit un renommage) ; sinon renommage simple ; les onglets ouverts suivent le nouveau chemin, contenu non enregistré compris | sélection multiple ; racine du projet | Maj+F6 |
| Supprimer… | vers la **corbeille** Windows après confirmation (dossier : « et ses n fichiers ») ; onglets non modifiés fermés, onglets modifiés gardés avec le bandeau « Supprimé du disque » | racine du projet | Suppr |
| Rechercher dans ce dossier… | recherche dans les fichiers, portée = ce dossier | fichier (prend son dossier : jamais grisée) | Ctrl+Maj+F |
| Ouvrir dans l'Explorateur | Explorateur Windows, l'élément sélectionné | — | — |
| Ouvrir un terminal ici | nouvel onglet du terminal par défaut, dossier de départ = ce dossier (ou celui du fichier) | — | — |
| Lancer les tests | `phpunit.xml` / `.dist` : la suite de cette configuration ; dossier : les tests qu'il contient ; fichier `*Test.php` : ce fichier (voir `phpunit.md`) | PHPUnit non détecté (« PHPUnit introuvable : réglez-le dans Réglages ») ; fichier qui n'est ni une configuration ni un `*Test.php` ; dossier hors des dossiers de tests de la configuration et sans `*Test.php` | Ctrl+Maj+F10 |
| Exclure de la recherche / Inclure dans la recherche | écrit `[index] excluded` en portée projet (voir `reglages.md`) | fichier ; `.git`, `.juliestorm` | — |
| Git ▸ | voir plus haut | | |
| Relire depuis le disque | existant | — | Ctrl+Alt+Y |

Cas limites : nom déjà pris (« Un fichier de ce nom existe déjà », rien
d'écrasé) ; caractères interdits par Windows (`<>:"|?*`), noms réservés (`CON`,
`NUL`, `COM1`…), point ou espace final : refus avec la raison, avant toute écriture ;
renommage qui ne change que la casse (`user.php` → `User.php`) : fait en deux temps
par un nom intermédiaire, et `git mv` de même ; corbeille indisponible (lecteur
réseau, élément trop gros) : seconde confirmation « Suppression définitive : ne
pourra pas être annulée », bouton par défaut Annuler ; fichier verrouillé par un
autre programme : message, rien de supprimé ; dossier non vide de 10 000 fichiers :
la suppression tourne en fond, l'arbre reste utilisable. Pas de mise à jour des
références dans le code après un renommage (**écart**, hors périmètre).

### Onglets d'éditeur

| Entrée | Grisée quand | Raccourci |
|---|---|---|
| Fermer | — | Ctrl+F4 |
| Fermer les autres onglets | un seul onglet | — |
| Fermer tous les onglets | — | — |
| Fermer les onglets non modifiés | aucun onglet non modifié | — |
| Copier le chemin ▸ | fichier jamais enregistré | Ctrl+Maj+C |
| Afficher dans l'arbre du projet | fichier hors projet | Alt+F1 |
| Ouvrir dans l'Explorateur | fichier jamais enregistré | — |
| Lancer les tests de ce fichier | comme dans l'arbre | Ctrl+Maj+F10 |
| Git ▸ (Voir le diff, Annuler les modifications…, Historique du fichier) | comme plus haut | |

Fermer plusieurs onglets dont certains modifiés : la question existante, **une
fois**, avec la liste.

### Éditeur (texte)

Le clic droit hors de la sélection place d'abord le curseur au point cliqué ;
dans la sélection, il la garde **(supposé comme PhpStorm)**.

| Entrée | Grisée quand | Raccourci |
|---|---|---|
| Aller à la déclaration | aucun identifiant sous le curseur ; langage non pris en charge (CSS, HTML, texte) | Ctrl+B, Ctrl+clic |
| Trouver les usages | idem | Alt+F7 |
| Couper / Copier / Coller | Couper et Coller : document en lecture seule ; Coller : presse-papiers vide. Sans sélection, Couper et Copier prennent la ligne (comportement existant) | Ctrl+X / Ctrl+C / Ctrl+V |
| Tout sélectionner | — | Ctrl+A |
| Commenter la ligne | langage sans commentaire de ligne ; lecture seule | Ctrl+/ |
| Rechercher dans les fichiers… | — (préremplie par la sélection d'une ligne) | Ctrl+Maj+F |
| Lancer les tests de ce fichier | comme dans l'arbre | Ctrl+Maj+F10 |
| Copier le chemin ▸ | fichier jamais enregistré | — |
| Git ▸ (Voir le diff, Annuler les modifications…, Historique du fichier) | comme plus haut | |

### Liste des changements Git

| Entrée | Grisée quand | Raccourci |
|---|---|---|
| Voir le diff | — | Ctrl+D, Entrée |
| Ouvrir le fichier | supprimé | F4 |
| Valider ces fichiers… | — : ouvre le panneau de commit avec **seulement** la sélection cochée | — |
| Ajouter à Git | rien de non suivi ni de modifié non indexé | Ctrl+Alt+A |
| Retirer de l'index | rien d'indexé | — |
| Annuler les modifications… | sélection de non suivis seulement | Ctrl+Alt+Z |
| Supprimer… | sélection sans non suivi (vers la corbeille ; seuls les non suivis sont supprimés) | Suppr |
| Ajouter à .gitignore ▸ | fichier suivi | — |
| Historique du fichier | non suivi, plusieurs fichiers | — |
| Afficher dans l'arbre du projet / Ouvrir dans l'Explorateur / Copier le chemin ▸ | supprimé (Explorateur) | Alt+F1 / — / Ctrl+Maj+C |

- Sur l'en-tête d'un groupe : les mêmes actions sur tout le groupe (« Ajouter les
  12 non suivis à Git », « Annuler toutes les modifications… »).
- Pour que « Retirer de l'index » ait un sens, la liste montre désormais l'état
  d'index : une marque discrète « indexé » ou « en partie indexé » après le nom
  (modifie `git.md`, qui n'exposait pas l'index). Le commit garde sa règle : il
  valide le **contenu du fichier de travail** des fichiers cochés (point à
  trancher : faut-il une option « valider l'index tel quel » ?).
- Groupe En conflit : les entrées de la liste des conflits (ci-dessous).

### Journal Git

Sur un commit :

| Entrée | Grisée quand |
|---|---|
| Copier le hash complet (Ctrl+C) / le hash court / le message | — |
| Afficher les fichiers changés | — (sélectionne, déjà fait au clic) |
| Actions de branche (créer une branche ici, basculer, cherry-pick, revert, réinitialiser) | voir `branches.md` |

Sur un fichier du détail d'un commit :

| Entrée | Grisée quand |
|---|---|
| Voir le diff avec le parent | — |
| Ouvrir la version de ce commit | binaire, supprimé dans ce commit (onglet en lecture seule, titre « nom @ hash court ») |
| Ouvrir le fichier actuel (F4) | absent du disque |
| Historique du fichier / Copier le chemin ▸ | — |

### Liste des fichiers en conflit

| Entrée | Grisée quand |
|---|---|
| Fusionner… | cas de `fusion.md` → Cas limites (raison en infobulle, déjà fait : M3) |
| Garder la nôtre / Prendre la leur | — ; confirmation si le côté supprime le fichier (existant) **ou** si le fichier de travail a été retouché depuis le début du conflit (« le contenu actuel sera remplacé ») |
| Ouvrir le fichier | supprimé du disque |
| Comparer notre version avec la leur | un côté absent ; binaire |
| Copier le chemin ▸ / Afficher dans l'arbre du projet | — |

### Arbre des résultats de tests

| Entrée | Grisée quand | Raccourci |
|---|---|---|
| Lancer ce test / cette classe / cette suite | nœud sans source connue ; exécution en cours (alors : confirmation d'arrêt existante, pas grisée) | Ctrl+Maj+F10 |
| Relancer les tests en échec | aucun échec ni erreur dans la dernière exécution | — |
| Aller au code source | nœud sans fichier | F4, double-clic |
| Comparer attendu et obtenu | pas de comparaison fournie par PHPUnit | — |
| Copier le nom (`Classe::méthode`, jeu de données compris) | — | Ctrl+C |
| Copier le message d'échec | test sans échec | — |

« Lancer ce test » sur un jeu de données lance la méthode avec ce seul jeu (filtre
PHPUnit, nom échappé : `'`, `|`, `[`, `"`, retours à la ligne).

### Résultats de recherche (recherche dans les fichiers, fenêtre Usages)

| Entrée | Grisée quand | Raccourci |
|---|---|---|
| Ouvrir à l'occurrence | ligne de fichier (ouvre la première) | Entrée, F4 |
| Copier la ligne / Copier le chemin ▸ | — | Ctrl+C (ligne) |
| Retirer des résultats (ligne ou fichier, pour cette recherche seulement) | — | Suppr |
| Rechercher dans ce dossier… | — | — |
| Afficher dans l'arbre du projet / Ouvrir dans l'Explorateur | — | Alt+F1 / — |

### Terminal

Le clic droit ouvre **toujours** notre menu (pas de collage direct au clic droit).

| Entrée | Grisée quand | Raccourci |
|---|---|---|
| Copier | aucune sélection | Ctrl+Maj+C (et Ctrl+C avec sélection) |
| Coller | presse-papiers sans texte | Ctrl+Maj+V, Ctrl+V |
| Tout sélectionner | — | — |
| Effacer (écran et historique) | — | — |
| Nouvel onglet ▸ cmd / Git Bash | Git Bash introuvable (existant) | Ctrl+Maj+T (défaut) |
| Renommer l'onglet… / Fermer l'onglet | — | — / Ctrl+F4 |

Sur un onglet de terminal : Renommer, Fermer, Fermer les autres, Nouvel onglet ▸.

## Critères d'acceptation

Sur des dépôts jetables dans `%TEMP%` (jamais un vrai dépôt).

1. Clic droit sur un fichier non suivi de l'arbre → Git ▸ Ajouter à Git : `git
   status --porcelain` passe de `?? f.php` à `A  f.php` ; la couleur change dans
   l'arbre et le fichier passe du groupe Non suivis à Modifiés en moins de 2 s.
2. Ctrl+Alt+A sur le même cas, sans menu : même résultat.
3. Retirer de l'index sur ce fichier : retour à `?? f.php`, le contenu du fichier
   est identique (SHA-256).
4. Dans un dossier hors dépôt, le sous-menu Git est grisé et son infobulle donne
   la raison.
5. Annuler les modifications sur 1 modifié, 1 supprimé et 1 ajouté : la
   confirmation nomme les 3 fichiers et ce qui arrivera à chacun ; Annuler (bouton)
   ne change rien (SHA-256 identiques) ; confirmer : le modifié et le supprimé
   égalent `HEAD`, l'ajouté n'est plus indexé et **existe toujours** sur le disque.
6. (si le filet est adopté) Après 5, « Rétablir » remet le modifié dans l'état
   d'avant, octet pour octet.
7. Annuler les modifications d'un fichier ouvert et modifié non enregistré : la
   confirmation le signale ; après confirmation l'onglet montre la version `HEAD`.
8. Nouveau ▸ Fichier `sous/dos/a.php` dans `src` : `src/sous/dos/a.php` existe,
   vide, s'ouvre dans un onglet. Nom `con.php` ou `a?.php` : refus, rien créé.
   Nom déjà pris : refus, l'existant intact.
9. Renommer un fichier suivi : `git status` montre `R  ancien -> nouveau` ; un
   onglet ouvert modifié de ce fichier garde son contenu et pointe le nouveau
   chemin. Renommer `user.php` en `User.php` : le nom sur le disque a la nouvelle
   casse.
10. Supprimer un fichier : il est dans la corbeille Windows (restaurable), plus sur
    le disque ; la confirmation le nommait.
11. Copier le chemin ▸ relatif sur `src/a.php` : le presse-papiers contient
    `src/a.php` ; absolu (Ctrl+Maj+C) : le chemin complet Windows.
12. Ouvrir un terminal ici sur `src` : `pwd` (Git Bash) ou `cd` (cmd) affiche `…\src`.
13. Lancer les tests sur un `*Test.php` du projet d'essai PHPUnit : seuls les tests
    de ce fichier apparaissent dans l'arbre, compteurs égaux à `php phpunit
    <fichier>` en ligne de commande. Sur un `.md` : entrée grisée avec raison.
14. Arbre des tests : « Lancer ce test » sur un jeu de données nommé
    `l'apostrophe | la barre [et] les crochets` lance exactement 1 test.
15. Journal : Copier le hash complet met les 40 caractères du commit dans le
    presse-papiers.
16. Terminal : sans sélection, Copier est grisé ; Coller colle le presse-papiers.
17. Touche Menu dans la liste des changements ouvre le menu de la ligne
    sélectionnée, sous elle ; Échap le ferme, rien n'a changé.
18. Toute entrée grisée affiche une infobulle non vide.
19. Chaque entrée qui a un raccourci l'affiche, et ce raccourci produit le même
    effet sans menu (vérifié pour Ctrl+Alt+A, Ctrl+Alt+Z, Maj+F6, Suppr, Alt+Inser,
    Ctrl+Maj+C).

## Hors périmètre

Couper / copier / coller de fichiers dans l'arbre, glisser-déposer, mise à jour des
références après renommage, annuler un bloc de changement dans l'éditeur (gouttière),
commit partiel, listes de changements nommées, menus personnalisables, historique
local complet, forcer l'ajout d'un fichier ignoré.
