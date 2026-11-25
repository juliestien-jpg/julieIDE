# Réglages

Rédigé le 25/11/2026 par `specialiste-produit`, d'après la demande du même jour
(« pouvoir configurer quelques petites choses, comme la façon d'exécuter
PHPUnit »). Repères : **(observé)** / **(supposé)** / **(écart)** : voir `fusion.md`.
La fenêtre de réglages de PhpStorm n'a pas été ré-observée pour cette spec : ce qui
la concerne est **supposé** d'usage. La configuration PHPUnit de `backoffice` a été
regardée dans ses fichiers, en lecture seule (voir `observations-phpstorm.md`).

## À quoi ça sert

Dire une fois pour toutes à l'IDE comment lancer les tests de mon projet (et deux
ou trois autres choses), sans éditer de fichier à la main — tout en pouvant le
faire si je préfère.

## Où c'est écrit, qui l'emporte

- Trois niveaux, du plus faible au plus fort : **valeur par défaut** (ou détection
  automatique) < **réglage utilisateur** (`%APPDATA%\JulieStorm\settings.toml`,
  vaut pour tous les projets) < **réglage de projet**
  (`<racine>\.juliestorm\settings.toml`). Le plus fort l'emporte, **clé par clé**
  (c'est déjà la règle de lecture de `julie-core`).
- Le réglage de projet reste **sur cette machine** : `.juliestorm/` est ignoré par
  Git (son `.gitignore` interne). **(écart)** : PhpStorm range une partie de ses
  réglages de projet dans `.idea/`, souvent versionné. Partager des réglages par
  Git est hors périmètre (point à trancher).
- Les fichiers sont du TOML, une table par sujet (`[phpunit]`, `[vcs]`,
  `[terminal]`, `[index]`, `[app]`). Ils restent **lisibles et éditables à la
  main** :
  - l'IDE ne réécrit que les clés qu'on a changées ; **commentaires, ordre des
    clés, clés et tables inconnues sont conservés** octet pour octet ;
  - un chemin est écrit en chaîne littérale TOML (`'C:\…'`, sans doubler les
    barres obliques inverses) sauf s'il contient une apostrophe ;
  - un fichier créé par l'IDE commence par une ligne de commentaire qui dit ce
    qu'il est et qu'on peut l'éditer ;
  - écriture atomique (`file::write_atomic`), fins de ligne et encodage du fichier
    existant conservés ; un fichier neuf est en UTF-8 sans BOM, fins de ligne CRLF.
- Revenir à la valeur héritée **supprime la clé** du fichier (pas de valeur vide
  écrite). Une table devenue vide que l'IDE avait créée est retirée.
- Fichier modifié à la main (dans l'IDE ou ailleurs) : relu en moins de 2 s, les
  nouveaux réglages s'appliquent comme après « Appliquer ». Fichier mal formé :
  notification « Réglages illisibles : <fichier>, ligne n : <raison> » avec un lien
  qui ouvre le fichier à la ligne ; l'IDE garde les derniers réglages valides et
  **refuse d'écrire** dans ce fichier tant qu'il est illisible (jamais d'écrasement
  d'un fichier qu'on ne comprend pas).

## La fenêtre

- **Ctrl+Alt+S** (comme PhpStorm), menu Fichier → Réglages…, ou palette d'actions.
  Fenêtre modale, redimensionnable, taille et page retenues pour la session.
- À gauche : un champ de filtre (le texte filtre les pages **et** les libellés des
  réglages ; une page sans correspondance disparaît, la correspondance est
  surlignée) puis la liste des pages : **PHPUnit**, **Git**, **Terminal**,
  **Recherche**, **Apparence**.
- À droite : la page. En tête des pages PHPUnit, Git, Terminal et Recherche, un
  choix de **portée** : « Ce projet » ou « Tous mes projets ». Par défaut : Ce
  projet pour PHPUnit et Recherche, Tous mes projets pour Git et Terminal.
  Apparence est toujours « Tous mes projets ». Sans projet ouvert, seule la portée
  utilisateur existe.
- Chaque champ montre la **valeur effective** et, en petit dessous, d'où elle
  vient : « détection automatique », « par défaut », « réglé pour tous mes
  projets », « réglé pour ce projet ». Un champ non réglé à la portée choisie
  montre la valeur héritée en grisé (texte d'aide), pas en valeur saisie. Un
  bouton « Revenir à la valeur héritée » apparaît sur un champ réglé à cette
  portée.
- Une valeur réglée pour ce projet masque celle de « Tous mes projets » : sur la
  page en portée « Tous mes projets », le champ le signale (« ce projet utilise sa
  propre valeur »).
- Un champ modifié et non appliqué est marqué ; la page aussi, dans la liste.
- En bas : « Ouvrir le fichier de réglages » (ouvre dans l'éditeur le fichier de la
  portée affichée, le crée s'il n'existe pas — avec seulement la ligne de
  commentaire), puis **OK** (applique et ferme), **Annuler** (ferme sans rien
  écrire ; s'il y a des changements, pas de question **(supposé comme PhpStorm)**),
  **Appliquer** (écrit, reste ouvert ; grisé sans changement).
- Échap = Annuler. Entrée dans un champ d'une ligne = OK.
- Un fichier de réglages modifié sur le disque alors que la fenêtre a des
  changements en attente sur la même portée : bandeau « Le fichier de réglages a
  changé sur le disque » avec Recharger (perd les changements en attente) / Garder
  les miens (ils écraseront **seulement les clés modifiées**).

### Validation immédiate

- 300 ms après la dernière frappe, chaque chemin est vérifié **en tâche de fond**
  (jamais sur le fil de l'interface : un lecteur réseau absent ne gèle rien) :
  - chemin vide → valeur héritée ;
  - **n'existe pas** → avertissement orange « Ce chemin n'existe pas » ;
  - c'est un dossier alors qu'on attend un fichier (ou l'inverse) → avertissement ;
  - exécutable valable → **version détectée** affichée en vert (« PHP 8.2.12 »,
    « PHPUnit 9.6.39 », « git version 2.56.0 ») ; sonde lancée avec un délai de 5 s,
    au-delà « Pas de réponse en 5 s » ;
  - le programme répond mais pas comme attendu → « Ce n'est pas <PHP / PHPUnit /
    Git> : <première ligne de la sortie> ».
- Un avertissement **n'empêche pas** d'enregistrer (un lecteur réseau peut être
  absent aujourd'hui). Une **erreur** (rouge) l'empêche : OK et Appliquer grisés
  avec la raison en infobulle. Erreurs : guillemets non fermés dans les arguments,
  nom de variable vide ou en double, motif d'exclusion vide.
- Les chemins relatifs sont relatifs à la racine du projet (portée projet) ; en
  portée utilisateur, un chemin relatif est une erreur, sauf un nom d'exécutable
  seul (`php`, `git`) cherché dans le PATH (le message le dit : « cherché dans le
  PATH : C:\…\php.exe »).
- Bouton « Parcourir… » à côté de chaque chemin (sélecteur de fichier Windows) ;
  pour un fichier dans le projet, le chemin est écrit relatif.

## Les réglages proposés

Liste courte, volontairement. Les autres clés existantes (`[phpunit]
timeout_seconds`, `[terminal] cmd_path`, `scrollback`, `[index] libraries`,
`max_file_size`) restent réglables **à la main** dans le fichier.

### PHPUnit (`[phpunit]`)

| Champ | Clé | Valeur |
|---|---|---|
| Interpréteur PHP | `php_path` | chemin de `php.exe`. Hérité : détection (voir `phpunit.md`). |
| Script PHPUnit | `phpunit_path` | chemin d'un script PHP lancé par `php` : `vendor/bin/phpunit`, un `.phar`… Hérité : détection. |
| Fichier de configuration | `configuration` | chemin d'un `phpunit.xml`. Hérité : `phpunit.xml`, puis `phpunit.xml.dist` à la racine. |
| Arguments supplémentaires | `extra_args` | tableau TOML de chaînes. Dans la fenêtre, une ligne, découpée aux espaces, les guillemets doubles groupent ; sous le champ, l'aperçu « 3 arguments : `--group` `lent` `--stop-on-failure` ». |
| Variables d'environnement | `[phpunit.env]` | table nom = valeur. Dans la fenêtre, un tableau à deux colonnes, lignes ajoutables et retirables. Ajoutées à l'environnement hérité de l'IDE, elles l'emportent sur lui. |

- Sous les champs, en lecture seule : **la commande qui sera lancée**, exactement
  celle qui s'écrira en tête de la console d'exécution, mise à jour en direct
  (`"C:\…\php.exe" "tools\phpunit\phpunit.phar" --configuration "phpunit.xml"
  --teamcity --stop-on-failure`).
- `extra_args` contenant `--teamcity`, `--configuration`/`-c`, `--printer`,
  `--log-teamcity` ou `--no-configuration` : avertissement « L'IDE pose déjà cette
  option / cette option empêche de lire les résultats ». On n'empêche pas.
- Les variables sont fusionnées clé par clé entre portées, comme le reste.
- Les deux niveaux ne concatènent jamais `extra_args` : la portée la plus forte
  remplace la liste entière (règle de fusion par clé).
- Une exécution en cours n'est pas touchée ; la suivante prend les nouveaux réglages.

**Détection automatique du script PHPUnit** (modifie `phpunit.md`) : dans l'ordre,
`vendor/bin/phpunit`, `vendor/phpunit/phpunit/phpunit`, `phpunit.phar`,
`tools/phpunit.phar`, **`tools/phpunit/phpunit.phar`** (nouveau : cas de
`backoffice`), **`tools/phpunit`** quand c'est un fichier (nouveau, **supposé** :
dossier d'outils installés par Phive). Chaque candidat doit être un **fichier** ; un
dossier du même nom est sauté (`backoffice` a un dossier `tools/phpunit/`). Jamais
un `.bat`, jamais un nom approchant (`backoffice` a un `vendor/bin/phpunit.rar` :
ce n'est pas un PHPUnit). Le message d'absence liste les emplacements essayés et
propose « Choisir le script PHPUnit… » qui ouvre cette page.

### Git (`[vcs]`)

| Champ | Clé | Valeur |
|---|---|---|
| Exécutable Git | `git_path` | chemin de `git.exe`. Hérité : `git` du PATH. Version affichée. |

Appliquer relance l'ouverture du dépôt (état rafraîchi) avec le nouvel exécutable.

### Terminal (`[terminal]`)

| Champ | Clé | Valeur |
|---|---|---|
| Terminal par défaut | `default_profile` | Git Bash ou cmd (liste). Hérité : Git Bash s'il est installé, sinon cmd. |
| Git Bash | `git_bash_path` | chemin de `bash.exe`. Hérité : `C:\Program Files\Git\bin\bash.exe`. |

Les onglets ouverts ne changent pas ; les nouveaux suivent le réglage.

### Recherche (`[index]`)

- **Dossiers et fichiers exclus** (`excluded`) : une liste éditable de motifs, un
  par ligne. Rappel de la règle sous la liste : « `nom` : à toute profondeur ;
  `/nom` : à la racine seulement ». Défaut : `/target`.
- `.git` et `.juliestorm` sont **toujours** exclus : affichés en tête de liste,
  grisés, non retirables (modifie le contrat de `julie-index`, où ils font partie
  de la valeur par défaut et disparaîtraient si on la remplaçait).
- Les exclus ne sont ni dans « Aller au fichier », ni dans la recherche dans les
  fichiers, ni dans l'index des symboles ; ils restent **visibles dans l'arbre du
  projet, grisés** **(supposé comme PhpStorm)**. Appliquer relance l'indexation.
- Le menu contextuel de l'arbre propose aussi « Exclure de la recherche » /
  « Inclure dans la recherche » sur un dossier (voir `menus-contextuels.md`) : il
  écrit dans ce même réglage, portée projet.

### Apparence (`[app]`)

| Champ | Clé | Valeur |
|---|---|---|
| Thème | `theme` | `"sombre"` ou `"clair"`. Défaut : sombre. |

Appliqué tout de suite après OK/Appliquer, sans redémarrer. L'action existante
« basculer le thème » écrit désormais ce réglage (portée utilisateur) : le thème
survit au redémarrage.

## Cas limites

- `%APPDATA%\JulieStorm\` absent : créé à la première écriture.
- Fichier de réglages en lecture seule ou dossier sans droit d'écriture : message
  qui nomme le fichier ; la fenêtre reste ouverte avec les changements.
- Fichier de réglages qui contient une clé de type inattendu (`timeout_seconds =
  "trente"`) : la section concernée est signalée comme illisible (même message que
  le fichier mal formé), les autres sections s'appliquent.
- Fichier en CRLF, avec BOM, avec commentaires en fin de ligne : conservés.
- Chemin avec espaces, parenthèses, accents (`C:\Program Files (x86)\…`) : écrit,
  relu et lancé correctement.
- Le projet ouvert change pendant que la fenêtre est ouverte : impossible (fenêtre
  modale).
- Deux instances de l'IDE qui écrivent le même fichier utilisateur : la dernière
  écriture gagne pour les clés qu'elle a changées, les autres clés sont relues
  juste avant l'écriture (pas de perte des clés de l'autre instance).
- Projet imbriqué : `backoffice` contient un second projet PHPUnit
  (`api/licenses_v2/plugins/mustache_php/phpunit.xml.dist`). On n'utilise que la
  configuration de la racine ou celle réglée ; plusieurs configurations nommées
  sont hors périmètre.

## Raccourcis

Ctrl+Alt+S ouvre la fenêtre (aussi depuis le terminal, qui garde ce raccourci pour
l'IDE). Dans la fenêtre : Échap, Entrée, Tab. Voir `raccourcis.md`.

## Critères d'acceptation

Les essais d'écriture se font sur des copies dans `%TEMP%` et avec
`%APPDATA%` redirigé vers un dossier jetable (variable `APPDATA` de l'IDE lancé pour
l'essai), jamais dans les réglages réels de la machine.

1. Ctrl+Alt+S ouvre la fenêtre sur la page PHPUnit ; Échap la ferme sans qu'aucun
   fichier soit créé ni modifié (horodatage et SHA-256 identiques).
2. **backoffice sans réglage** : sur une copie jetable qui reproduit
   l'organisation de `backoffice` (`phpunit.xml` à la racine, le
   `tools/phpunit/phpunit.phar` de backoffice copié, un dossier `tools/phpunit/`,
   un `vendor/bin/phpunit.rar` leurre, pas de `vendor/bin/phpunit`), la page
   PHPUnit affiche « détection automatique : tools\phpunit\phpunit.phar —
   PHPUnit 9.6.39 » et la commande affichée est
   `"<php>" "<copie>\tools\phpunit\phpunit.phar" --configuration
   "<copie>\phpunit.xml" --teamcity`. Ctrl+Maj+F10 lance cette commande (même
   texte en tête de la console).
3. Sur le vrai `backoffice`, en lecture seule (ouvrir le projet et la fenêtre, ne
   rien appliquer, ne rien lancer) : même détection et même commande affichées ;
   aucun fichier de `backoffice` créé ni modifié (`git status` identique avant et
   après).
4. **backoffice avec réglage** : sur un projet de copie où la détection ne trouve
   rien (phar déplacé dans `outils/phpunit.phar`), régler « Script PHPUnit » à
   `outils\phpunit.phar` en portée Ce projet, OK : `.juliestorm\settings.toml`
   contient la table `[phpunit]` avec `phpunit_path = 'outils\phpunit.phar'` ;
   Ctrl+Maj+F10 lance `php outils\phpunit.phar …` et les compteurs égalent ceux de
   la même commande sans `--teamcity` en ligne de commande.
5. Taper `C:\nexiste\pas\php.exe` dans Interpréteur PHP : « Ce chemin n'existe
   pas » apparaît en moins de 1 s ; OK reste actif. Taper le vrai chemin de PHP :
   la version s'affiche en moins de 2 s.
6. Un fichier `settings.toml` de projet écrit à la main avec un commentaire en tête,
   un commentaire en fin de ligne, une table inconnue `[perso]` et des fins de ligne
   CRLF : après modification d'une seule clé par la fenêtre, `git diff --no-index`
   entre avant et après ne montre que la ligne de cette clé.
7. « Revenir à la valeur héritée » puis OK : la clé n'est plus dans le fichier ; la
   valeur utilisateur (ou la détection) s'applique.
8. Même clé réglée côté utilisateur et côté projet : la commande affichée utilise
   la valeur du projet ; retirer celle du projet fait apparaître celle de
   l'utilisateur.
9. Arguments `--group "très lent"` : aperçu « 2 arguments » ; la commande lancée
   reçoit exactement deux arguments (vérifié par un test PHP qui affiche `$argv`,
   ou dans la console). Arguments `--group "lent` : erreur, OK grisé.
10. Variable `APP_ENV = test` en portée projet : un test qui écrit `getenv('APP_ENV')`
    affiche `test`.
11. Éditer à la main le fichier de réglages de projet (dans l'IDE, Ctrl+S) pour
    changer `phpunit_path` : la page PHPUnit (rouverte) et la commande suivante
    prennent la nouvelle valeur sans redémarrer.
12. Rendre le fichier mal formé (`[phpunit` sans crochet fermant) : notification
    avec le numéro de ligne ; ouvrir la fenêtre, changer un réglage de cette
    portée, OK : message de refus, le fichier n'a pas changé d'un octet.
13. Thème réglé sur clair, OK, quitter, relancer : l'IDE démarre en clair.
14. Ajouter `docs` aux exclus en portée projet : un fichier `docs/a.php` disparaît
    d'Aller au fichier et de la recherche dans les fichiers en moins de 5 s ; il
    reste visible, grisé, dans l'arbre. `.git` et `.juliestorm` ne peuvent pas être
    retirés de la liste.
15. Chemin de Git invalide, OK : la fenêtre Git dit que `git` est introuvable en
    nommant le réglage ; remettre la valeur héritée rétablit la fenêtre Git sans
    redémarrer.
16. Un fichier de réglages en lecture seule : OK affiche un message qui nomme le
    fichier, la fenêtre reste ouverte avec les changements.

## Hors périmètre

Réglages de l'éditeur (police, taille, tabulations), raccourcis personnalisables,
plusieurs configurations d'exécution nommées, commande PHPUnit libre (`composer
test`, ParaTest, Pest : point à trancher), interpréteurs distants ou Docker,
partage des réglages de projet par Git, import des réglages de PhpStorm, thème qui
suit Windows, réinitialiser tous les réglages.
