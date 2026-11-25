# Lexique de JulieStorm

Ce lexique fait foi pour l'interface, l'aide et les messages. Un terme par notion :
on écrit le terme de la colonne « Retenu », jamais un synonyme. Si un besoin n'est
pas couvert, on l'ajoute ici avant de l'écrire dans l'application.

## Termes Git gardés tels quels

Ces noms sont ceux que tout le monde utilise ; on ne les traduit pas.

| Retenu | Usage dans l'interface | À éviter |
|---|---|---|
| commit (nom) | « 1 commit », « Commit de fusion », « Commits sortants » | « révision », « validation » employé seul pour le nom |
| valider (verbe) | « Valider », « Valider les changements… » : on valide un commit | « committer », « commiter » |
| amender | « Amender », « Amender le dernier commit » | |
| push (nom) | « Push refusé » | |
| pousser (verbe) | « Pousser… », « Pousser aussi les étiquettes » | « push » employé comme verbe |
| pull (nom) | « Tirer dans … par fusion » ; tâche « Récupération et intégration (pull) » | |
| tirer (verbe) | « Tirer dans la courante par fusion / par rebase » | « puller » |
| fetch (nom) | « Récupérer (fetch) », « Récupéré : … » | « fetcher » |
| récupérer (verbe) | « Récupérer (fetch) » : première mention avec le nom Git entre parenthèses | « rapatrier », « télécharger » |
| rebase (nom et verbe) | « Rebase », « Rebaser la courante sur celle-ci », « Rebase en cours » | « rebaser » sous une autre forme, comme « faire une rebase » |
| cherry-pick | « Cherry-pick » | |
| revert | « Revert », « Revert en cours », « commit inverse (revert) » dans les actions | « annulation de commit » |
| diff | « Voir le diff », « Diff côte à côte », « Calcul du diff » | « différentiel » |
| HEAD | « HEAD détachée » | |
| reflog | « reflog de Git » dans un message d'information | |
| bail (forçage avec bail) | « Forcer avec bail », explication dans l'infobulle | « bail nu », « lease » |

## Termes traduits

| Retenu | Sens | À éviter |
|---|---|---|
| branche | branch | « branch » |
| branche distante | référence distante, ex. `origin/main` | « branche remote » |
| distant | le dépôt distant (`origin`) | « remote », « serveur » |
| amont | branche distante suivie par une branche locale | « upstream », « tracking » |
| étagère | les changements rangés par JulieStorm sous forme de patchs nommés (comme le « shelf » de PhpStorm) ; à ne pas confondre avec la mise de côté | « stash » |
| mettre de côté, mise de côté | ranger ses changements avec `git stash` ; on peut citer `git stash` entre parenthèses pour qui connaît Git | « stasher », « étagère » |
| index (Git) | zone de préparation : « indexé », « à l'index », « Retirer de l'index » | « staging », « stage » |
| indexation | le calcul de l'index de recherche et des symboles | « index » pour parler de la recherche |
| crochet Git | un hook | « hook » |
| étiquette | un tag Git | « tag » |
| journal | la liste des commits (log) | « log » dans l'interface |
| fusion | merge, Git ou fenêtre à trois volets | « merge » |
| fusion à trois volets | le dialogue qui compare notre version, leur version et le résultat | |
| conflit | fichier ou bloc en désaccord entre deux versions | |
| résoudre / résolu | marquer un conflit traité | « régler » |
| notre version / leur version | côtés d'une fusion (nôtre = la branche courante) | « ours », « theirs » |
| bloc | un morceau de différence dans le dialogue de fusion | « hunk », « chunk » |
| garder la nôtre / prendre la leur | choix de côté pour un fichier en conflit | |
| basculer (sur) | changer de branche ou de commit | « checkout », « switch » |
| copie de travail | les fichiers tels qu'ils sont sur le disque | « répertoire de travail », « working tree » |
| annuler les modifications | ramener un fichier à HEAD, avec copie de sécurité | « rollback », « discard » |
| jeter | perte des changements locaux, sans retour | « écraser » sans préciser ce qui disparaît |
| écraser | remplacer un fichier sur le disque | |
| restaurer | remettre une copie de sécurité ou une branche supprimée | |
| réinitialiser | déplacer la branche sur un autre commit (`reset`) | |
| annuler (verbe seul) | fermer sans rien faire ; à ne pas confondre avec « Annuler les modifications » | |
| rétablir | refaire une action annulée (redo) | « refaire » |
| réglages | les options de l'application et des projets | « paramètres », « préférences », « options » |
| portée | « Ce projet » ou « Tous mes projets » | |
| dossier | un répertoire | « répertoire », « folder » |
| usage | une référence à un nom, trouvée par la recherche | « utilisation », « référence » |
| usage sûr / appel possible | usage dont le type est connu / appel dont le type est inconnu | « usage incertain » |
| déclaration | l'endroit où un nom est défini | « définition » |
| symbole | fonction, classe, méthode, variable… indexés | |
| recherche dans les fichiers | recherche de texte dans tout le projet | « grep » |
| occurrence | une correspondance de recherche | « résultat » pour une seule ligne |
| arbre (des résultats) | liste hiérarchique des tests ou des résultats | |
| fenêtre d'outils | Projet, Git, Tests, Recherche, Terminal, Usages (le panneau du bas et de côté) | « panneau » sauf pour le panneau de validation |
| volet | une partie du dialogue de fusion | « pane » |
| onglet | un fichier ouvert dans l'éditeur, ou un terminal | |
| terminal | une session cmd ou Git Bash | « console » |
| tâche de fond | un calcul qui tourne sans bloquer l'interface | « job » |
| enregistrer | écrire le fichier sur le disque | « sauvegarder », « save » |
| recharger | relire le fichier du disque | « relire » |
| supprimer | effacer un élément | « effacer » (réservé au vidage d'écran du terminal) |
| corbeille | la corbeille de Windows | « poubelle » |
| sonde / vérification | le test d'un chemin dans les réglages | « probe » |

## Tests PHPUnit

| Retenu | Sens |
|---|---|
| réussi | test passé |
| échoué | assertion fausse (failure) |
| en erreur | exception ou erreur PHP pendant le test (error) |
| ignoré | test sauté (skipped) |
| incomplet | test marqué incomplet |
| risqué | test sans assertion ou sans effet vérifié (risky) |
| interrompu | arrêt pendant le test |
| attendu / obtenu | valeurs d'une comparaison |
| trace | pile d'appels d'une erreur |
| suite | la liste de tests d'une configuration |

## Verbes des boutons

- Un bouton commence par un verbe à l'infinitif, ou porte le nom de l'action
  (« Fusionner… », « Basculer »). Les points de suspension (…) indiquent qu'un
  dialogue va s'ouvrir.
- « OK » n'est pas utilisé : on écrit ce que fait le bouton (« Enregistrer et fermer »).
- « Annuler » sert uniquement à fermer un dialogue sans agir.
- Une action qui perd des données dit le verbe exact : « Supprimer », « Jeter »,
  « Abandonner », « Écraser », « Supprimer définitivement ».

## Écarts et choix à confirmer

- Les verbes « pousser » et « tirer » remplacent push et pull sur les boutons, par
  accord avec les spécifications (`docs/specs/branches.md`). Le nom Git reste dans
  les titres (« Push refusé », « pull »). À confirmer avec la personne qui dirige le
  projet si elle préfère « Push… » et « Pull… » sur les boutons.
- « diff » garde son nom : les spécifications l'utilisent partout.
- « index » désigne la zone de préparation de Git ; « indexation » désigne l'index
  de recherche. Ne jamais écrire « index » pour la recherche.
