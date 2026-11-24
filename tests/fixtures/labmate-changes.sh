#!/usr/bin/env bash
# Prépare un CLONE JETABLE de labmate_paper avec des changements locaux variés :
# modifié (src/main.js, README.md), supprimé (LICENSE), ajouté à l'index
# (docs/essai-ajoute.md), non suivis (notes-essai.txt, tools/brouillon.js), fins de ligne
# seules changées (package.json, LF -> CRLF), et un crochet commit-msg qui refuse tout
# message contenant REFUSE. N'écrit rien dans l'original.
# Usage : tests/fixtures/labmate-changes.sh [dossier-du-clone]   (défaut : $TEMP/jsessai-git)
set -euo pipefail
ORIG="C:/Program Files (x86)/AgileBio/LabCollector/www/labmate_paper"
DEST="${1:-${TEMP:-/tmp}/jsessai-git}"
rm -rf "$DEST"
git -c core.autocrlf=false clone --no-hardlinks "$ORIG" "$DEST"
cd "$DEST"
git config user.name "Essai JulieStorm"
git config user.email "essai@juliestorm.invalid"
git config core.autocrlf false
# Les crochets du clone, pas ceux de la configuration globale de la machine.
git config core.hooksPath .git/hooks
sed -i "s/#1e3440/#1e3a48/; s/antialias: true/antialias: false/" src/main.js
printf '\n// Essai : ligne ajoutée en fin de fichier.\n' >> src/main.js
sed -i '1s/^.*$/# LabosSystem Quest — essai de commit\r/' README.md
rm LICENSE
printf '# Essai\r\nUne note ajoutée à l index.\r\n' > docs/essai-ajoute.md
git add docs/essai-ajoute.md
printf 'note non suivie\n' > notes-essai.txt
printf 'console.log("brouillon");\n' > tools/brouillon.js
sed -i 's/$/\r/' package.json
cat > .git/hooks/commit-msg <<'HOOK'
#!/bin/sh
if grep -q REFUSE "$1"; then
  echo "commit-msg : message refusé par le crochet de l'essai (contient REFUSE)" >&2
  exit 1
fi
HOOK
chmod +x .git/hooks/commit-msg
git status --short
