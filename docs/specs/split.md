# Éditeurs fractionnés (split view)

Demande de la personne qui dirige le projet, le 25/11/2026 : « pouvoir afficher
plusieurs fichiers en simultané en mode split view ». Comportement inspiré de
PhpStorm (supposé, non observé sur cette machine), décrit avec nos mots.

## À quoi ça sert

Voir et éditer plusieurs fichiers côte à côte ou l'un au-dessus de l'autre — ou deux
endroits d'un même fichier — sans basculer d'onglet.

## Comportement

- La zone centrale contient un ou plusieurs **groupes d'éditeurs**. Chaque groupe a
  sa propre rangée d'onglets et son onglet actif. Un seul groupe est **actif** à la
  fois (celui qui a eu le dernier clic ou le focus) ; il est signalé discrètement.
- Les groupes sont disposés en **arbre de divisions** : chaque division est verticale
  (côte à côte) ou horizontale (l'un au-dessus de l'autre), avec un séparateur
  déplaçable. On peut imbriquer (par exemple deux colonnes, la droite coupée en deux).
- **Fractionner** (menu contextuel d'un onglet, menu Affichage, palette d'actions) :
  - « Fractionner à droite » / « Fractionner en bas » : crée un groupe à côté et y
    ouvre **le même fichier** (même document, deux vues : une frappe dans l'une se
    voit dans l'autre ; positions et défilement indépendants) ;
  - « Déplacer à droite » / « Déplacer en bas » : crée le groupe et y **déplace**
    l'onglet ;
  - « Déplacer vers l'autre groupe » quand il en existe déjà un.
- **Glisser un onglet** vers le bord d'un groupe (gauche, droite, haut, bas) crée une
  division ; vers une rangée d'onglets, le déplace dans ce groupe. Un aperçu montre
  où il atterrira. Si le glisser n'est pas faisable proprement avec egui, le menu
  suffit pour cette version ; le dire.
- **Ouvrir un fichier** (arbre, aller au fichier, recherche, navigation, trace de
  test) l'ouvre dans le **groupe actif**. Si le fichier y est déjà ouvert, on l'active ;
  s'il n'est ouvert que dans un autre groupe, on active cet onglet-là.
- **Fermer** le dernier onglet d'un groupe ferme le groupe ; la place est rendue à son
  voisin. « Défractionner » (« Tout regrouper ») ramène tous les onglets dans un seul
  groupe, sans doublon.
- Un fichier ouvert dans deux groupes est **un seul document** : marque de
  modification, enregistrement, rechargement après changement sur le disque et
  « Enregistrer ? » à la fermeture ne s'appliquent qu'une fois. Fermer l'une des deux
  vues d'un document modifié ne pose pas la question tant que l'autre reste ouverte.
- Le dialogue de fusion et le diff côte à côte ne sont pas concernés : ils ont leurs
  propres volets.
- La disposition (divisions, proportions, onglets de chaque groupe) est **retrouvée à
  la réouverture du projet**, dans `.juliestorm/`.

## Raccourcis proposés

| Action | Raccourci |
|---|---|
| Groupe suivant / précédent | Alt+Tab est pris par Windows : **Ctrl+Alt+Tab** non plus ; proposer Ctrl+Alt+Page suivante / précédente, à confirmer |
| Fractionner à droite | aucun par défaut (menu et palette) |
| Fermer l'onglet | Ctrl+F4 (inchangé, agit dans le groupe actif) |

## Critères d'acceptation

1. Sur un onglet `a.php`, « Fractionner à droite » donne deux groupes côte à côte
   montrant `a.php` ; une frappe dans l'un apparaît dans l'autre ; Ctrl+S
   n'enregistre qu'une fois et les deux vues perdent leur marque de modification.
2. « Déplacer en bas » sur `b.js` : deux groupes l'un au-dessus de l'autre, `b.js`
   n'est plus dans le premier.
3. Avec deux groupes, Ctrl+Maj+N puis Entrée ouvre le fichier dans le groupe actif.
4. Le séparateur se déplace à la souris et la proportion est gardée au
   redimensionnement de la fenêtre.
5. Trois niveaux d'imbrication restent utilisables en 1280×720.
6. Fermer le dernier onglet d'un groupe le fait disparaître sans perdre les autres.
7. « Tout regrouper » : un seul groupe, chaque fichier une seule fois.
8. Fermer puis rouvrir le projet redonne la même disposition et les mêmes onglets.
9. Fermer une vue d'un document modifié ouvert deux fois ne pose pas de question ;
   fermer la dernière la pose.
10. Aucune régression des scénarios existants qui supposent un seul groupe.
