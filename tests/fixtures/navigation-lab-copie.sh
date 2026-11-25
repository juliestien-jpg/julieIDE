#!/usr/bin/env bash
# Copie jetable de quelques fichiers de `lab` (LECTURE SEULE sur lab : on ne fait que copier)
# pour essayer la navigation sur du vrai code. On n'ouvre jamais `lab` lui-même dans l'IDE :
# le cache des symboles (.juliestorm/) serait écrit dedans.
# Usage : bash tests/fixtures/navigation-lab-copie.sh [dossier]   (défaut : $TEMP/jsessai-lab/proj)
set -euo pipefail
lab="C:/Program Files (x86)/AgileBio/LabCollector/www/lab"
dest="${1:-${TEMP:-/tmp}/jsessai-lab/proj}"
rm -rf "$dest"
mkdir -p "$dest/classes"
cp -r "$lab/classes/General" "$dest/classes/General"
cp "$lab/classes/AlertMaintenanceController.php" "$lab/classes/Timestamp.php" "$dest/classes/"
echo "$dest"
