#!/usr/bin/env bash
# Dépôt jetable en cours de fusion avec des conflits de toutes natures :
#   texte.txt     modifié des deux côtés (fusionnable, 3 blocs)
#   crlf.txt      notre version en CRLF, la leur en LF (bandeau des fins de ligne)
#   ajoute.txt    ajouté des deux côtés (base vide)
#   binaire.bin   binaire modifié des deux côtés (pas de dialogue)
#   latin1.txt    Windows-1252 modifié des deux côtés (pas de dialogue)
#   supprime.txt  modifié par nous, supprimé par eux (pas de dialogue)
#   longs.txt     blocs de hauteurs différentes (connecteurs courbes) : insertion de 6 lignes à
#                 gauche, suppression de 4 lignes à droite, conflit 1 ligne contre 4 lignes
# Usage : tests/fixtures/conflits-varies.sh [dossier]   (défaut : $TEMP/jsessai-conflits)
set -euo pipefail
DEST="${1:-${TEMP:-/tmp}/jsessai-conflits}"
rm -rf "$DEST"
mkdir -p "$DEST"
cd "$DEST"
git init -q -b base
git config user.name "Essai JulieStorm"
git config user.email "essai@juliestorm.invalid"
git config core.autocrlf false
git config core.hooksPath .git/hooks
printf 'un\ndeux\ntrois\nquatre\ncinq\nsix\nsept\nhuit\nneuf\ndix\n' > texte.txt
printf 'a\nb\nc\nd\ne\n' > crlf.txt
printf 'BIN\0\1\2base\n' > binaire.bin
printf 'caf\xe9 de la base\nligne 2\n' > latin1.txt
printf 'sera supprimé\npar eux\n' > supprime.txt
seq 1 40 | sed 's/^/ligne /' > longs.txt
git add -A && git commit -q -m "base"
git branch eux
git checkout -q -b nous
sed -i 's/^deux$/DEUX (nous)/; s/^sept$/SEPT (nous)/; s/^dix$/DIX (nous)/' texte.txt
printf 'a\r\nB nous\r\nc\r\nd\r\ne\r\n' > crlf.txt
printf 'ajouté par nous\ncommun\n' > ajoute.txt
printf 'BIN\0\1\2nous\n' > binaire.bin
printf 'caf\xe9 de nous\nligne 2\n' > latin1.txt
printf 'sera supprimé\npar eux, modifié par nous\n' > supprime.txt
awk 'NR==5{print "ligne 5 (nous)"; next} {print} NR==10{for(i=1;i<=6;i++) print "nouvelle " i}' longs.txt > longs.tmp && mv longs.tmp longs.txt
git add -A && git commit -q -m "nous"
git checkout -q eux
sed -i 's/^quatre$/QUATRE (eux)/; s/^sept$/SEPT (eux)/; s/^dix$/DIX (nous)/' texte.txt
printf 'a\nb\nc\nd\nE eux\n' > crlf.txt
printf 'ajouté par eux\ncommun\n' > ajoute.txt
printf 'BIN\0\1\2eux\n' > binaire.bin
printf 'caf\xe9 de eux\nligne 2\n' > latin1.txt
git rm -q supprime.txt
awk 'NR==5{print "ligne 5 (eux, a)"; print "ligne 5 (eux, b)"; print "ligne 5 (eux, c)"; print "ligne 5 (eux, d)"; next} NR>=20 && NR<=23{next} {print}' longs.txt > longs.tmp && mv longs.tmp longs.txt
git add -A && git commit -q -m "eux"
git checkout -q nous
git merge eux || true
git status --short
