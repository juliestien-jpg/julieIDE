#!/usr/bin/env bash
# Décor jetable pour les branches (docs/specs/branches.md, critères 1 à 28) :
#   distant.git   dépôt NU local, le seul « distant » (jamais de vrai distant)
#   A/            le clone à ouvrir dans JulieStorm
#   B/            la collègue, en ligne de commande (git -C "$DEST/B" …)
# Branches de départ, poussées, avec amont dans A :
#   main, feature/login, feature/search, bugfix/crash, juliestorm-essai/a
# Dans A, feature/login a trois commits dont le deuxième change la même ligne
# de a.txt que main (fusion et rebase en conflit, critères 16 et 17).
#
# Usage : tests/fixtures/branches.sh [dossier] [scénario…]
#   dossier par défaut : $TEMP/jsessai-branches (effacé puis recréé)
#   scénarios (cumulables, dans cet ordre de préférence) :
#     retard           B pousse 2 commits sur main (A est en retard : ↓2 après Récupérer)  (1, 9, 13)
#     avance           A fait 1 commit local sur main (↑1)                               (1, 13)
#     distante-seule   B pousse feature/remote-only, inconnue de A                     (4)
#     amont-supprime   B supprime feature/search sur le distant                         (25)
#     non-fusionnee    bugfix/crash porte un commit non poussé, non fusionné            (18)
#     stash-conflit    a.txt modifié dans A (non validé), en conflit avec feature/login (6, 7)
#     crochet          crochet pre-push de A qui refuse (« refusé par le crochet »)     (15)
#     lent             distant « lent » de A dont le ssh factice dort 30 s (annulation) (27)
#     tout             tous les scénarios ci-dessus sauf crochet et lent
# La configuration Git de la machine n'est pas lue par ce script
# (GIT_CONFIG_GLOBAL et GIT_CONFIG_NOSYSTEM) ; chaque dépôt reçoit sa propre
# identité, core.autocrlf=false et core.hooksPath=.git/hooks.
set -euo pipefail
DEST="${1:-${TEMP:-/tmp}/jsessai-branches}"
shift || true
SCENARIOS=("$@")
if [ "${#SCENARIOS[@]}" -eq 1 ] && [ "${SCENARIOS[0]}" = "tout" ]; then
  SCENARIOS=(retard avance distante-seule amont-supprime non-fusionnee stash-conflit)
fi

case "$DEST" in
  ""|"/"|"$HOME"|"$HOME/") echo "dossier refusé : $DEST" >&2; exit 1 ;;
esac
rm -rf "$DEST"
mkdir -p "$DEST"
cd "$DEST"
# Chemin Windows (C:/…) sous Git Bash, pour l'ouvrir dans l'IDE.
DEST="$(pwd -W 2>/dev/null || pwd)"

export GIT_CONFIG_NOSYSTEM=1
export GIT_CONFIG_GLOBAL="$DEST/gitconfig-essai"
export GIT_TERMINAL_PROMPT=0
printf '[init]\n\tdefaultBranch = main\n[protocol "file"]\n\tallow = always\n' > "$GIT_CONFIG_GLOBAL"

configure() {
  git -C "$1" config user.name "$2"
  git -C "$1" config user.email "$3"
  git -C "$1" config core.autocrlf false
  git -C "$1" config core.hooksPath .git/hooks
  git -C "$1" config commit.gpgsign false
}

git init -q --bare -b main distant.git
git clone -q distant.git A 2>/dev/null
configure A "Essai A" "a@juliestorm.invalid"

# --- main
printf 'un\ndeux\ntrois\n' > A/a.txt
printf '# Projet d essai\n' > A/LISEZMOI.md
git -C A add -A && git -C A commit -q -m "Premier commit"
git -C A push -q -u origin main

# --- branches de départ
make_branch() { # nom fichier sujet
  git -C A switch -q -c "$1" main
  printf '%s\n' "$1" > "A/$2"
  git -C A add -A && git -C A commit -q -m "$3"
}
make_branch feature/login login.txt "Connexion : écran"
printf 'un\ndeux (côté login)\ntrois\n' > A/a.txt
git -C A commit -q -am "Connexion : a.txt ligne 2"
printf 'ok\n' > A/login-fin.txt
git -C A add -A && git -C A commit -q -m "Connexion : fin"
make_branch feature/search search.txt "Recherche"
make_branch bugfix/crash crash.txt "Correctif du plantage"
make_branch juliestorm-essai/a essai.txt "Essai A"
for b in feature/login feature/search bugfix/crash juliestorm-essai/a; do
  git -C A push -q -u origin "$b"
done
git -C A switch -q main
printf 'un\ndeux (côté main)\ntrois\n' > A/a.txt
git -C A commit -q -am "Main : a.txt ligne 2"
git -C A push -q

# --- la collègue
git clone -q distant.git B
configure B "Collègue B" "b@juliestorm.invalid"

for scenario in "${SCENARIOS[@]}"; do
  case "$scenario" in
    retard)
      printf 'b1\n' > B/b1.txt; git -C B add -A; git -C B commit -q -m "B : premier"
      printf 'b2\n' > B/b2.txt; git -C B add -A; git -C B commit -q -m "B : second"
      git -C B push -q
      ;;
    avance)
      printf 'local\n' > A/local.txt; git -C A add -A; git -C A commit -q -m "A : commit local"
      ;;
    distante-seule)
      git -C B switch -q -c feature/remote-only
      printf 'distante\n' > B/remote.txt; git -C B add -A; git -C B commit -q -m "B : branche distante seule"
      git -C B push -q -u origin feature/remote-only
      git -C B switch -q main
      ;;
    amont-supprime)
      git -C B push -q origin --delete feature/search
      ;;
    non-fusionnee)
      git -C A switch -q bugfix/crash
      printf 'non poussé\n' > A/crash2.txt; git -C A add -A; git -C A commit -q -m "Correctif non poussé"
      git -C A switch -q main
      ;;
    stash-conflit)
      printf 'un\ndeux (changement local non validé)\ntrois\n' > A/a.txt
      ;;
    crochet)
      printf '#!/bin/sh\necho "refusé par le crochet" >&2\nexit 1\n' > A/.git/hooks/pre-push
      chmod +x A/.git/hooks/pre-push
      ;;
    lent)
      printf '#!/bin/sh\nsleep 30\n' > "$DEST/ssh-lent.sh"
      chmod +x "$DEST/ssh-lent.sh"
      git -C A config core.sshCommand "\"$DEST/ssh-lent.sh\""
      git -C A remote add lent ssh://git@example.invalid/lent.git
      ;;
    *)
      echo "scénario inconnu : $scenario" >&2; exit 1 ;;
  esac
done

echo "Décor prêt dans $DEST"
echo "  distant : $DEST/distant.git"
echo "  ouvrir  : $DEST/A"
echo "  B       : git -C \"$DEST/B\" …"
echo "Scénarios : ${SCENARIOS[*]:-(aucun)}"
git -C A branch -vv
