#!/usr/bin/env bash
# Copie jetable du projet d'essai tests/fixtures/phpunit, avec le réglage qui désigne
# le PHPUnit 9.6 de lab (le projet d'essai n'a pas de vendor/). Lecture seule sur lab.
# Usage : tests/fixtures/phpunit-copy.sh [dossier]   (défaut : $TEMP/jsessai-phpunit)
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
DEST="${1:-${TEMP:-/tmp}/jsessai-phpunit}"
rm -rf "$DEST"
mkdir -p "$DEST/.juliestorm"
cp -r "$HERE/phpunit/phpunit.xml" "$HERE/phpunit/tests" "$DEST/"
cat > "$DEST/.juliestorm/settings.toml" <<'TOML'
[phpunit]
phpunit_path = 'C:\Program Files (x86)\AgileBio\LabCollector\www\lab\vendor\bin\phpunit'
TOML
echo "$DEST"
