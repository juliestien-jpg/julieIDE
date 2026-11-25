#!/usr/bin/env bash
# Recette v1 : joue un scénario sur un décor, avec un JULIESTORM_STATE_DIR neuf.
# Usage : recette-v1-run.sh <dossier-décor> <scénario> [délai-max-s]
# Dans le scénario : @A@ = <décor>/A, @D@ = <décor>, @OUT@ = <décor>/out (créé).
set -uo pipefail
D="$1"; SC="$2"; MAX="${3:-240}"
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
DW="$(cd "$D" && pwd -W 2>/dev/null || pwd)"
mkdir -p "$D/out"
STATE="$(mktemp -d "${TEMP:-/tmp}/jsetat-XXXXXX")"
TMPSC="$D/out/$(basename "$SC")"
sed -e "s#@A@#$DW/A#g" -e "s#@D@#$DW#g" -e "s#@OUT@#$DW/out#g" "$SC" > "$TMPSC"
cd "$ROOT"
JULIESTORM_STATE_DIR="$(cygpath -w "$STATE")" timeout "$MAX" ./target/debug/juliestorm.exe --scenario "$TMPSC" > "$D/out/$(basename "$SC").log" 2>&1
RC=$?
echo "rc=$RC ($(basename "$SC"))"
grep -E 'ÉCHEC|panick|paniqu' "$D/out/$(basename "$SC").log" | head -10
rm -rf "$STATE"
