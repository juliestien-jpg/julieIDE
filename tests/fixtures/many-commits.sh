#!/usr/bin/env bash
# Dépôt jetable de 1 200 commits (journal paginé par lots de 500). Rapide : git fast-import.
# Usage : tests/fixtures/many-commits.sh [dossier]   (défaut : $TEMP/jsessai-journal)
set -euo pipefail
DEST="${1:-${TEMP:-/tmp}/jsessai-journal}"
rm -rf "$DEST"; mkdir -p "$DEST"; cd "$DEST"
git init -q -b main
git config user.name "Essai JulieStorm"; git config user.email "essai@juliestorm.invalid"
awk 'BEGIN {
  t = 1700000000;
  for (i = 1; i <= 1200; i++) {
    msg = "Commit numéro " i;
    printf "commit refs/heads/main\ncommitter Essai <essai@juliestorm.invalid> %d +0000\ndata %d\n%s\n", t + i * 3600, length(msg), msg;
    printf "M 644 inline fichier.txt\ndata %d\ncontenu %d\n\n", length("contenu " i) + 1, i;
  }
}' | git fast-import --quiet
git reset -q --hard
git log --oneline | wc -l
