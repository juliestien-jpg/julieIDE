#!/usr/bin/env bash
# Copie de backoffice pour les essais de réglages : seulement phpunit.xml, le phar de
# tools/phpunit/ et le leurre vendor/bin/phpunit.rar (rien d'autre, ni .git ni vendor).
# L'original n'est que LU. Ne lancez JAMAIS la suite de backoffice (des tests envoient des mails).
set -euo pipefail
SRC="${1:-/c/Program Files (x86)/AgileBio/LabCollector/www/backoffice}"
DEST="${2:-${TEMP:-/tmp}/jsessai-backoffice}"
rm -rf "$DEST"
mkdir -p "$DEST/tools/phpunit" "$DEST/vendor/bin" "$DEST/tests"
cp "$SRC/phpunit.xml" "$DEST/phpunit.xml"
cp "$SRC/tools/phpunit/phpunit.phar" "$DEST/tools/phpunit/phpunit.phar"
cp "$SRC/vendor/bin/phpunit.rar" "$DEST/vendor/bin/phpunit.rar"
echo '<?php // fichier d essai' > "$DEST/index.php"
echo "Copie prête : $DEST"
