#!/usr/bin/env bash
# Dépôt jetable avec UN fichier de 10 000 lignes en conflit : 400 blocs non conflictuels
# (200 à gauche, 200 à droite) et 20 conflits. Sert à mesurer l'ouverture et « appliquer tous ».
# Usage : tests/fixtures/conflit-gros.sh [dossier]   (défaut : $TEMP/jsessai-gros)
set -euo pipefail
DEST="${1:-${TEMP:-/tmp}/jsessai-gros}"
rm -rf "$DEST"; mkdir -p "$DEST"; cd "$DEST"
git init -q -b base
git config user.name "Essai JulieStorm"; git config user.email "essai@juliestorm.invalid"
git config core.autocrlf false; git config core.hooksPath .git/hooks
seq 1 10000 | awk '{print "ligne " $1 " : du texte pour remplir la ligne"}' > gros.txt
git add -A && git commit -q -m base
git branch eux
git checkout -q -b nous
awk 'NR%50==0 && NR%1000!=0 {print "ligne " NR " : MODIFIÉE PAR NOUS"; next} NR%1000==0 {print "ligne " NR " : CONFLIT nous"; next} {print}' gros.txt > t && mv t gros.txt
git add -A && git commit -q -m nous
git checkout -q eux
awk 'NR%50==25 {print "ligne " NR " : MODIFIÉE PAR EUX"; next} NR%1000==0 {print "ligne " NR " : CONFLIT eux"; next} {print}' gros.txt > t && mv t gros.txt
git add -A && git commit -q -m eux
git checkout -q nous
git merge eux || true
git status --short
