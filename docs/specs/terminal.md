# Terminal intégré (v0)

Rédigé le 24/11/2026 par `specialiste-produit`, d'après la demande du 24/11/2026
(voir `CLAUDE.md`). **(observé)** / **(supposé)** / **(écart)** : voir `fusion.md`.

## À quoi ça sert

Taper mes commandes (git, composer, php…) sans quitter l'IDE, dans cmd ou Git
Bash, plusieurs à la fois, déjà placée dans le dossier du projet.

## Le comportement

- Alt+F12 ouvre la fenêtre d'outils Terminal en bas, focus dans l'onglet actif ;
  appuyé de nouveau quand le terminal a le focus, la replie et rend le focus à
  l'éditeur (supposé comme PhpStorm).
- Première ouverture : un onglet avec le terminal par défaut. Réglage « terminal par
  défaut » : **Git Bash** s'il est installé, sinon cmd (**écart** : PhpStorm part sur
  PowerShell ; ici l'usage quotidien est Git Bash).
- Bouton « + » : nouvel onglet du terminal par défaut ; sa flèche propose **cmd** ou
  **Git Bash**. Ctrl+Maj+T : nouvel onglet par défaut.
- Shells : cmd = `%COMSPEC%` ; Git Bash = `C:\Program Files\Git\bin\bash.exe
  --login -i` (réglage modifiable). Git Bash absent → entrée grisée, infobulle
  « Git Bash introuvable à <chemin> ».
- Dossier de départ : racine du projet. Variables : celles de l'IDE, plus
  `TERM=xterm-256color` pour Git Bash.
- Titre d'onglet : « cmd », « Git Bash », numérotés s'il y en a plusieurs
  (« Git Bash (2) ») ; remplacé par le titre que fixe le shell (séquence OSC) s'il
  en fixe un. Double-clic sur l'onglet : renommer.
- Onglets en nombre illimité, chacun avec sa session indépendante qui continue de
  tourner quand on regarde un autre onglet.
- **Affichage** : couleurs ANSI 16, 256 et 24 bits ; gras, souligné, inverse ;
  curseur ; écran alternatif (less, vim, `git log` paginé) ; Unicode et accents
  (`echo é` affiche « é » dans les deux shells) ; police à chasse fixe du thème.
- **Défilement** : 10 000 lignes d'historique, molette ; une nouvelle sortie ramène
  en bas seulement si on y était déjà.
- **Redimensionnement** : changer la taille de la fenêtre d'outils transmet la
  nouvelle taille au shell (ConPTY) ; pas de réagencement des anciennes lignes en v0.
- **Copier-coller** : sélection à la souris (double-clic = mot, triple-clic = ligne).
  Ctrl+C copie s'il y a une sélection, sinon envoie l'interruption. Ctrl+V et
  Maj+Inser collent ; Ctrl+Maj+C / Ctrl+Maj+V copient / collent toujours. Collage
  multiligne : envoyé tel quel (en mode collage encadré si le shell l'a demandé).
  Clic droit : menu Copier / Coller / Effacer.
- **Clavier** : tout va au shell (flèches, Tab, Échap, Ctrl+R, Ctrl+D…), sauf la
  courte liste gardée par l'IDE (voir `raccourcis.md` → Règles : Alt+F12, Alt+0 à
  Alt+9, Ctrl+Maj+N/F/T/C/V, Ctrl+F4). **Échap va au shell** (**écart** :
  dans PhpStorm Échap rend souvent le focus à l'éditeur, ce qui gêne vim et less).
- **Fin du shell** (`exit`) : code 0 → l'onglet se ferme ; autre code → l'onglet
  reste avec « Processus terminé (code n) » et un bouton « Relancer ».
- **Fermer un onglet** (Ctrl+F4 dans le terminal, croix) : si le shell a des
  processus enfants en cours, confirmation « Un programme tourne encore dans ce
  terminal (git, php…). Le fermer ? » ; sinon fermeture directe. Fermer = terminer
  le shell et tous ses descendants.
- **Fermer l'IDE** : même confirmation, une seule fois, si un terminal a des
  enfants en cours.

## Cas limites

Commande qui produit beaucoup de sortie (`git log` sans pagination sur un gros
dépôt, `type` d'un fichier de 50 Mio) : l'interface reste utilisable ; programme
plein écran (vim, less, `git rebase -i` avec vim) ; séquences VT inconnues
ignorées sans afficher de déchets ; chemin du projet avec espaces et parenthèses
(`Program Files (x86)`) ; caractères larges (CJK, emoji) occupant deux cellules ;
shell qui ne démarre pas (chemin faux) → message dans l'onglet, pas de plantage.

## Critères d'acceptation

1. Alt+F12 ouvre un terminal dont `pwd` (Git Bash) ou `cd` (cmd) affiche la racine
   du projet.
2. On peut avoir en même temps un onglet cmd et deux onglets Git Bash, chacun
   indépendant (une commande longue dans l'un n'empêche pas de taper dans l'autre).
3. `ls --color` dans Git Bash et une sortie 256 couleurs (`for i in $(seq 0 255)…`)
   s'affichent en couleurs.
4. Après agrandissement de la fenêtre d'outils, `tput cols` (Git Bash) ou
   `mode con` (cmd) affiche la nouvelle largeur.
5. Sélectionner du texte puis Ctrl+C le place dans le presse-papiers ; sans
   sélection, Ctrl+C interrompt `ping -t localhost` (cmd) ou `sleep 100` (Git Bash).
6. Ctrl+V colle le presse-papiers dans le shell.
7. `less` puis `q`, et `vim` puis `:q` fonctionnent et rendent un écran propre.
8. `echo é` affiche « é » dans cmd et dans Git Bash.
9. `exit` ferme l'onglet ; `exit 3` le laisse ouvert avec le code 3.
10. Fermer un onglet pendant `sleep 100` demande confirmation ; après fermeture,
    aucun `sleep.exe` ni `bash.exe` de cet onglet ne reste dans le gestionnaire des
    tâches.
11. Avec un chemin de Git Bash invalide, l'entrée Git Bash est grisée et l'IDE ne
    plante pas.

## Hors périmètre v0

PowerShell, WSL et autres shells, liens cliquables dans la sortie, recherche dans
le terminal, réagencement des lignes au redimensionnement, sessions conservées au
redémarrage, terminal en volet séparé (côte à côte), lancement de commandes depuis
l'éditeur, glisser-déposer de fichiers.
