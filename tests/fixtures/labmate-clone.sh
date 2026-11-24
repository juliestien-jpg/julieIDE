#!/usr/bin/env bash
# Prépare un CLONE JETABLE de labmate_paper en cours de fusion (conflit sur README.md :
# changement à gauche seul, à droite seul, vrai conflit ligne 1, même ligne modifiée des
# deux côtés). N'écrit rien dans l'original.
# Usage : tests/fixtures/labmate-clone.sh [dossier-du-clone]   (défaut : $TEMP/jsessai-fusion)
set -euo pipefail
ORIG="C:/Program Files (x86)/AgileBio/LabCollector/www/labmate_paper"
DEST="${1:-${TEMP:-/tmp}/jsessai-fusion}"
rm -rf "$DEST"
git clone --no-hardlinks "$ORIG" "$DEST"
cd "$DEST"
git fetch "$ORIG" "refs/heads/juliestorm-essai/*:refs/heads/juliestorm-essai/*"
git checkout juliestorm-essai/nous
git merge juliestorm-essai/eux || true
git status --short
