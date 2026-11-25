# Décisions de cadrage de la v1

Points soulevés par `specialiste-produit` le 25/11/2026 en écrivant `branches.md`,
`reglages.md`, `menus-contextuels.md` et `navigation.md`. Tranchés par
l'orchestrateur **par défaut, en attendant la confirmation de la personne qui dirige
le projet** : un choix contesté par elle l'emporte sur ce fichier, qui sera alors
corrigé. Les spécifications font foi pour tout le reste.

| # | Question | Décision par défaut | Pourquoi |
|---|---|---|---|
| 1 | Maj+F10 | **Lancer** (comme PhpStorm). Le menu contextuel s'ouvre au clic droit et avec la touche Menu. | La personne qui dirige le projet a les réflexes PhpStorm (même logique que Ctrl+Y). |
| 2 | Ctrl+Maj+` et Alt+` sur AZERTY | Garder ces raccourcis **et** en offrir un de remplacement sans accent grave, choisi par `specialiste-produit` dans `raccourcis.md`. | Accessible sans retirer le réflexe. |
| 3 | Copie de sécurité avant « Annuler les modifications » ou une bascule forcée | **Oui** : copie dans `.juliestorm/annulations/`, gardée 7 jours, avec « Rétablir ». | Un IDE qui se trompe détruit du travail ; le filet coûte peu. |
| 4 | Mise de côté automatique avant bascule ou mise à jour | **`git stash`** pour l'instant, avec un message reconnaissable ; l'étagère maison viendra avec le pilier « étagère » et reprendra ce rôle. | Livrer les branches sans attendre l'étagère. |
| 5 | Commit et index | Le commit garde la règle « contenu du fichier de travail des fichiers cochés ». « Valider l'index tel quel » attend le commit partiel. | Pas de changement de comportement en cours de route. |
| 6 | Partage des réglages de projet | **Locaux** (`.juliestorm/`, ignoré par Git). | Rien n'oblige encore à versionner. |
| 7 | Commande PHPUnit libre (`composer test`, ParaTest) | **Écartée pour l'instant** ; `php` + script + arguments + environnement suffisent pour backoffice. | Rester petit. |
| 8 | Essais sur backoffice | **La vraie suite de backoffice n'est jamais lancée par un agent** (des tests envoient des mails). Sur l'original, seule la détection est vérifiée, sans rien écrire (pas même `.juliestorm/`). Les essais d'exécution se font sur `tests/fixtures/phpunit`. | Effets de bord réels. |
| 9 | Ctrl+clic sur `X` dans `new X()` | **La classe** ; le constructeur est proposé dans la liste s'il existe. | Comportement le plus prévisible. |
| 10 | Usages dans les bibliothèques (`vendor/`, `node_modules/`) | **Exclus par défaut**, case pour les inclure. | Bruit et temps. |
| 11 | Ctrl+Alt+Gauche / Droite (navigation arrière / avant) | **Gardés**, plus les boutons précédent / suivant de la souris ; si le pilote Intel les capture sur cette machine, le signaler dans le rapport. | Réflexe PhpStorm. |
| 12 | Observation de PhpStorm | En suspens : les agents ne peuvent faire qu'un clic gauche dans PhpStorm. Le comportement « supposé » de `branches.md` est suivi ; la personne qui dirige le projet le corrigera à l'usage ou par des captures. | Pas de contournement des limites d'accès. |
