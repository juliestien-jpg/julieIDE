# Raccourcis clavier (v0)

Rédigé le 24/11/2026 par `specialiste-produit`. Table unique ; les autres specs y
renvoient. Colonne « PhpStorm » : **oui** = même raccourci que le schéma par défaut
de PhpStorm sous Windows (connu d'usage, **supposé** — non vérifié touche par
touche sur cette machine) ; **nôtre** = proposition JulieStorm, faute de
raccourci par défaut connu ou par choix.

## À quoi ça sert

Que les réflexes PhpStorm de la personne qui dirige le projet marchent tels quels.

## Table

| Contexte | Raccourci | Action | PhpStorm |
|---|---|---|---|
| Partout | Ctrl+Maj+N | Aller au fichier | oui |
| Partout | Ctrl+Maj+F | Rechercher dans les fichiers | oui |
| Partout | Ctrl+K | Ouvrir le panneau de commit | oui |
| Partout | Alt+0 | Montrer/replier le panneau de commit | oui |
| Partout | Alt+1 | Montrer/replier l'arbre du projet | oui |
| Partout | Alt+4 | Montrer/replier la fenêtre Exécution (tests) | oui |
| Partout | Alt+9 | Montrer/replier la fenêtre Git | oui |
| Partout | Alt+F12 | Montrer/replier le terminal | oui |
| Partout | Ctrl+Maj+F10 | Lancer la suite PHPUnit | oui (lancer le contexte) |
| Partout | Ctrl+F5 | Relancer la dernière exécution | oui |
| Partout | Ctrl+F2 | Arrêter l'exécution | oui |
| Partout | Ctrl+S | Tout enregistrer | oui |
| Partout | Ctrl+Alt+Y | Relire le disque (rafraîchir arbre, Git, onglets) | oui |
| Éditeur | Ctrl+Z / Ctrl+Maj+Z | Annuler / rétablir | oui |
| Éditeur | Ctrl+Y | Supprimer la ligne | oui (choix délibéré, pas « rétablir ») |
| Éditeur | Ctrl+C / Ctrl+X / Ctrl+V / Ctrl+A | Copier / couper / coller / tout sélectionner | oui |
| Éditeur | Ctrl+F4 | Fermer l'onglet | oui |
| Éditeur | Alt+Gauche / Alt+Droite | Onglet précédent / suivant | oui |
| Éditeur | Ctrl+Tab | Revenir à l'onglet précédemment actif | oui (version simplifiée) |
| Changements locaux | Ctrl+D ou Entrée | Ouvrir le diff du fichier | oui |
| Changements locaux | F4 | Ouvrir le fichier dans l'éditeur | oui |
| Panneau de commit | Ctrl+Entrée | Valider | oui |
| Diff | F7 / Maj+F7 | Différence suivante / précédente | oui |
| Diff, fusion | Échap | Fermer | oui |
| Fusion | F7 / Maj+F7 | Bloc en attente suivant / précédent | oui |
| Fusion | Alt+Gauche / Alt+Droite | Accepter le côté gauche / droit du bloc courant (ou l'ajouter à la suite) | nôtre |
| Fusion | Alt+Maj+Gauche / Alt+Maj+Droite | Ignorer le côté gauche / droit du bloc courant | nôtre |
| Fusion | Ctrl+Entrée | Enregistrer et marquer résolu | nôtre |
| Aller au fichier | Haut / Bas, Entrée, Échap | Parcourir, ouvrir, fermer | oui |
| Recherche dans les fichiers | Alt+C / Alt+W / Alt+X | Casse / mots entiers / expression régulière | nôtre (PhpStorm a des équivalents, non vérifiés) |
| Recherche dans les fichiers | Entrée, Échap | Ouvrir l'occurrence, fermer | oui |
| Terminal | Ctrl+Maj+T | Nouvel onglet (terminal par défaut) | oui |
| Terminal | Ctrl+F4 | Fermer l'onglet de terminal | oui |
| Terminal | Ctrl+C | Copier si sélection, sinon interrompre | oui |
| Terminal | Ctrl+V, Maj+Inser | Coller | oui |
| Terminal | Ctrl+Maj+C / Ctrl+Maj+V | Copier / coller toujours | nôtre |

### Ajouts du 25/11/2026 (`reglages.md`, `menus-contextuels.md`, `navigation.md`, `branches.md`)

| Contexte | Raccourci | Action | PhpStorm |
|---|---|---|---|
| Partout | Ctrl+Alt+S | Ouvrir les réglages | oui (supposé) |
| Partout (sauf terminal) | Touche Menu | Menu contextuel de la zone qui a le focus | oui (supposé) |
| Partout (sauf terminal) | Maj+F10 | Menu contextuel (convention Windows) | **écart** : dans PhpStorm, lance la configuration courante — à trancher |
| Partout | Ctrl+T | Mettre à jour le projet (Git) | oui (supposé) |
| Partout | Ctrl+Maj+K | Pousser… | oui (supposé) |
| Partout | Ctrl+Maj+` | Fenêtre surgissante des branches | oui (supposé ; voir la note clavier AZERTY) |
| Partout | Alt+` | Liste rapide des opérations Git | oui (supposé) |
| Éditeur | Ctrl+clic, Ctrl+B | Aller à la déclaration (sur une déclaration : liste des usages) | oui (supposé) |
| Éditeur | Alt+F7 | Trouver les usages (fenêtre Usages) | oui (supposé) |
| Éditeur | survol avec Ctrl | Souligner un nom qui a une déclaration | oui (supposé) |
| Partout (sauf terminal) | Ctrl+Alt+Gauche / Ctrl+Alt+Droite, boutons précédent / suivant de la souris | Position de navigation précédente / suivante | oui (supposé) |
| Fenêtre Usages, éditeur | Ctrl+Alt+Bas / Ctrl+Alt+Haut | Usage suivant / précédent | oui (supposé) |
| Arbre du projet, changements, onglets, éditeur | Ctrl+Alt+A | Git : ajouter à Git | oui (supposé) |
| Arbre du projet, changements, onglets, éditeur | Ctrl+Alt+Z | Git : annuler les modifications… | oui (supposé) |
| Arbre du projet, onglets, changements, résultats | Ctrl+Maj+C | Copier le chemin absolu | oui (supposé) |
| Arbre du projet | Alt+Inser | Nouveau fichier / dossier | oui (supposé) |
| Arbre du projet | Maj+F6 | Renommer… | oui (supposé) |
| Arbre du projet, changements (non suivis) | Suppr | Supprimer vers la corbeille… | oui (supposé) |
| Arbre du projet, onglets, changements | Alt+F1 | Afficher dans l'arbre du projet | à peu près (PhpStorm ouvre un choix de destinations) |
| Arbre du projet, éditeur, arbre des tests | Ctrl+Maj+F10 | Lancer les tests du contexte (fichier, dossier, nœud) ; ailleurs, la suite | oui |
| Arbre des tests | F4 | Aller au code source du test | oui (supposé) |
| Menu contextuel | Haut / Bas, Droite / Gauche, Entrée, Échap | Parcourir, sous-menu, exécuter, fermer | oui |
| Réglages | Échap / Entrée | Annuler / OK | oui |

Note clavier : sur un clavier AZERTY, l'accent grave est sur AltGr+7 ; Ctrl+Maj+`
et Alt+` y sont mal commodes ou inaccessibles **(supposé)**. Le widget de branche
reste cliquable ; un raccourci de remplacement est à trancher si la personne qui
dirige le projet utilise ces raccourcis.

Ctrl+Maj+C garde son sens « copier toujours » dans le terminal (contexte plus précis).

## Règles

- Dans le terminal, seuls Alt+F12, Alt+0 à Alt+9, Ctrl+Maj+N, Ctrl+Maj+F,
  Ctrl+Maj+T, Ctrl+Maj+C, Ctrl+Maj+V, Ctrl+F4, Ctrl+Alt+S et la touche Menu sont
  gardés par l'IDE ; tout le reste (Échap, Ctrl+T, Maj+F10, Alt+F7 compris) va au
  shell.
- Dans le dialogue de fusion, Alt+Gauche / Alt+Droite n'ont pas leur sens
  « changer d'onglet » : le dialogue est une fenêtre à part.
- Une touche n'a qu'un sens par contexte ; un raccourci d'un contexte plus précis
  l'emporte sur « Partout ».
- Pas de personnalisation des raccourcis en v0.

## Critères d'acceptation

1. Chaque ligne de la table, jouée dans son contexte, déclenche l'action indiquée.
2. Dans le terminal, Échap, Ctrl+R et les flèches arrivent au shell ; Alt+F12 et
   Ctrl+Maj+N restent à l'IDE.
3. Aucune combinaison de la table n'a deux actions dans le même contexte.
4. Chaque raccourci apparaît à côté de son action dans les menus.

## Hors périmètre v0

Recherche partout (double Maj), rechercher une action (Ctrl+Maj+A), Ctrl+E (récents),
Ctrl+G (aller à la ligne), Ctrl+F / Ctrl+R (rechercher/remplacer dans le fichier),
schémas de touches personnalisables. (Ctrl+B, Alt+F7, Ctrl+Alt+Z et Ctrl+T entrent
dans la table par les ajouts du 25/11/2026.)
