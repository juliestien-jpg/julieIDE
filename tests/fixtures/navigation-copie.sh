#!/usr/bin/env bash
# Copie jetable du projet d'essai de la navigation (sans Git) : on y écrit (onglet modifié,
# cache .juliestorm/), jamais dans tests/fixtures/navigation lui-même.
# Usage : bash tests/fixtures/navigation-copie.sh [dossier]   (défaut : $TEMP/jsessai-nav/proj)
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
dest="${1:-${TEMP:-/tmp}/jsessai-nav/proj}"
rm -rf "$dest"
mkdir -p "$dest"
cp -r "$here/navigation/." "$dest/"
echo "$dest"
