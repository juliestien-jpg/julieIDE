#!/usr/bin/env bash
# Décors des corrections de la recette v1 (docs/recette-v1.md : M1 à M5, m1 à m13).
# Construit sur tests/fixtures/branches.sh (dépôt nu local `distant.git`, clone A à ouvrir,
# clone B = la collègue) ; jamais de vrai distant, rien en dehors du dossier donné.
#
# Usage : tests/fixtures/recette-v1-corrections.sh <dossier> <cas>
#   force         b04 : a.txt modifié et login.txt non suivi gênent la bascule vers feature/login ;
#                 n.txt (indexé) et u.txt (non suivi) n'y touchent pas
#   maj-conflit   b06 à b09 : A a un commit local sur a.txt ligne 2, B pousse la même ligne ;
#                 dans A, LISEZMOI.md modifié et nv.txt non suivi (mis de côté par la mise à jour)
#   reset-hard    b12 : main avec c1, c2, c3 ; a.txt et c3.txt modifiés, n.txt indexé, u.txt non suivi
#   detachee      b13 : main avec c1, c2, c3 ; orphelin.txt non suivi
#   arbre         b14 : main avec c1, c2, c3 (l'arbre doit suivre une bascule sur c1)
#   pick-present  m2 : feature/search déjà fusionnée dans main (cherry-pick d'un commit présent)
#   merge-sale    m1 : a.txt modifié (non validé), fusionner feature/login le refuserait
#   supprimer     m3 : bugfix/crash supprimée puis restaurée avec son amont
#   sans-amont    m5 : une branche locale `sans-amont` sans amont, courante
#   js-php        m11 : page.php avec une fonction JavaScript dans un <script>
#   ops-over      M1 (état OperationOver) : comme maj-conflit ; le scénario part, puis l'opération est
#                 conclue en ligne de commande (voir recette-v1-corr-m1-terminal-a.txt)
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

three_commits() {
  for n in 1 2 3; do
    printf 'c%s\n' "$n" > "$A/c$n.txt"
    git -C "$A" add -A
    git -C "$A" commit -q -m "C$n"
  done
}

case "$CASE" in
  force)
    printf 'n\n' > "$A/n.txt"; git -C "$A" add n.txt
    printf 'u\n' > "$A/u.txt"
    printf 'mon login non suivi\n' > "$A/login.txt"
    printf 'un\ndeux (changement local non validé)\ntrois\n' > "$A/a.txt"
    ;;
  maj-conflit|ops-over)
    printf 'un\nDEUX (A)\ntrois\n' > "$A/a.txt"
    git -C "$A" commit -q -am "A : ligne 2"
    git -C "$B" pull -q
    printf 'un\nDEUX (B)\ntrois\n' > "$B/a.txt"
    git -C "$B" commit -q -am "B : ligne 2"
    git -C "$B" push -q
    printf 'modif locale\n' >> "$A/LISEZMOI.md"
    printf 'nouveau\n' > "$A/nv.txt"
    ;;
  reset-hard)
    three_commits
    printf 'un\ndeux (local)\ntrois\n' > "$A/a.txt"
    printf 'c3 local\n' >> "$A/c3.txt"
    printf 'n\n' > "$A/n.txt"; git -C "$A" add n.txt
    printf 'u\n' > "$A/u.txt"
    ;;
  js-php)
    printf '<?php
function phpfn() { return 1; }
phpfn();
?>
<script>
function jsfn() { return 2; }
jsfn();
</script>
' > "$A/page.php"
    ;;
  detachee)
    three_commits
    printf 'orphelin\n' > "$A/orphelin.txt"
    ;;
  arbre)
    three_commits
    ;;
  pick-present)
    git -C "$A" merge -q --no-edit feature/search
    ;;
  merge-sale)
    printf 'un\ndeux (changement local non validé)\ntrois\n' > "$A/a.txt"
    ;;
  supprimer|sans-amont)
    if [ "$CASE" = "sans-amont" ]; then
      git -C "$A" switch -q -c sans-amont
      printf 'x\n' > "$A/sans.txt"; git -C "$A" add -A; git -C "$A" commit -q -m "Sans amont"
    fi
    ;;
  *) echo "cas inconnu : $CASE" >&2; exit 1 ;;
esac
echo "Décor « $CASE » prêt : $DD/A"
