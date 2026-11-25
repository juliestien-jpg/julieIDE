#!/usr/bin/env bash
# Décors de la contre-recette v1 (docs/recette-v1.md, section « Contre-recette »).
# Construit sur tests/fixtures/branches.sh (dépôt nu local, jamais de vrai distant).
#
# Usage : tests/fixtures/contre-recette.sh <dossier> <cas>
#   pick     cherry-pick de « Connexion : a.txt ligne 2 » (feature/login) sur main : conflit sur
#            a.txt ; a.txt (ligne 3), LISEZMOI.md modifiés et nv.txt non suivi => le cherry-pick
#            est refusé (a.txt écrasé), « Mettre de côté et reprendre le commit »
#   revert   main reçoit un commit qui change encore la ligne 2 de a.txt ; annuler « Main : a.txt
#            ligne 2 » est en conflit ; mêmes changements locaux
#   deux     comme maj-conflit (mise à jour en conflit avec mise de côté) ; sert aux opérations
#            successives (le scénario enchaîne ensuite un cherry-pick en conflit)
#   detachee-branche   HEAD détachée portant 1 commit, un changement local non suivi
set -euo pipefail
D="${1:?dossier}"
CASE="${2:?cas}"
HERE="$(cd "$(dirname "$0")" && pwd)"
export GIT_CONFIG_NOSYSTEM=1
export GIT_TERMINAL_PROMPT=0
bash "$HERE/branches.sh" "$D" >/dev/null
DD="$(cd "$D" && pwd -W 2>/dev/null || pwd)"
export GIT_CONFIG_GLOBAL="$DD/gitconfig-essai"
A="$D/A"
B="$D/B"
locals() {
  printf 'un\ndeux (côté main)\ntrois (local)\n' > "$A/a.txt"
  printf 'modif locale\n' >> "$A/LISEZMOI.md"
  printf 'nouveau\n' > "$A/nv.txt"
}
case "$CASE" in
  pick) locals ;;
  revert)
    printf 'un\ndeux (final)\ntrois\n' > "$A/a.txt"
    git -C "$A" commit -q -am "Main : ligne 2 finale"
    locals_a() { :; }
    printf 'un\ndeux (final)\ntrois (local)\n' > "$A/a.txt"
    printf 'modif locale\n' >> "$A/LISEZMOI.md"
    printf 'nouveau\n' > "$A/nv.txt"
    ;;
  deux)
    printf 'un\nDEUX (A)\ntrois\n' > "$A/a.txt"
    git -C "$A" commit -q -am "A : ligne 2"
    git -C "$B" pull -q
    printf 'un\nDEUX (B)\ntrois\n' > "$B/a.txt"
    git -C "$B" commit -q -am "B : ligne 2"
    git -C "$B" push -q
    printf 'modif locale\n' >> "$A/LISEZMOI.md"
    printf 'nouveau\n' > "$A/nv.txt"
    ;;
  detachee-branche)
    for n in 1 2 3; do printf 'c%s\n' "$n" > "$A/c$n.txt"; git -C "$A" add -A; git -C "$A" commit -q -m "C$n"; done
    ;;
  *) echo "cas inconnu : $CASE" >&2; exit 1 ;;
esac
echo "Décor « $CASE » prêt : $DD/A"
