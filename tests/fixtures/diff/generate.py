#!/usr/bin/env python3
"""Fabrique le corpus de fusion de julie-diff : un dossier par cas, avec trois
fichiers `base`, `ours` (la nôtre), `theirs` (la leur), écrits à l'octet près.

    python tests/fixtures/diff/generate.py

Le script réécrit `merge/<cas>/` ; il ne touche pas à `merge/cases.txt`, qui dit
pour chaque cas ce qu'on attend par rapport à `git merge-file` et pourquoi
(vérifié par `crates/diff/tests/git_compare.rs`).

Un cas peut porter un quatrième fichier, `expected` : le texte fusionné attendu,
écrit à la main. Le test exige alors une fusion sans conflit et ce texte-là à
l'octet près.
"""
import os

ROOT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "merge")
CASES = {}


def case(name, base, ours, theirs, expected=None):
    assert name not in CASES, name
    CASES[name] = (base, ours, theirs, expected)


def crlf(text):
    return text.replace("\n", "\r\n")


# --- Les cas de docs/specs/fusion.md (13 et 24 reprennent 1 et 5) -------------

case("spec-01", "a\nb\nc\n", "a\nB\nc\n", "a\nb\nc\n")
case("spec-02", "a\nb\nc\n", "a\nb\nc\n", "a\nb\nC\n")
case("spec-03", "a\nb\nc\nd\ne\n", "a\nB\nc\nd\ne\n", "a\nb\nc\nD\ne\n")
case("spec-04", "a\nb\nc\n", "a\nX\nc\n", "a\nX\nc\n")
case("spec-05", "a\nb\nc\n", "a\nB1\nc\n", "a\nB2\nc\n")
case("spec-06", "a\nb\nc\nd\n", "a\nB\nc\nd\n", "a\nb\nC\nd\n")
case("spec-07", "a\nb\n", "a\nX\nb\n", "a\nY\nb\n")
case("spec-08", "a\nb\n", "a\nX\nb\n", "a\nX\nb\n")
case("spec-09", "a\nb\nc\n", "a\nc\n", "a\nB\nc\n")
case("spec-10", "a\nb\nc\n", "a\nc\n", "a\nc\n")
case("spec-11", "a\nb\nc\n", "a\nX\nb\nc\n", "a\nB\nc\n")
case("spec-12", "  b\n", "    b\n", "  B\n")
case("spec-14", crlf("a\nb\nc\nd\ne\n"), crlf("a\nB\nc\nd\ne\n"), crlf("a\nb\nc\nD\ne\n"))
case("spec-15", "a\nb\n", "a\nB\n", "a\r\nb\r\n")
case("spec-16", "a\r\nb\r\n", "a\r\nb\r\n", "a\nb\nc\n")
case("spec-17", "a\r\nb\nc\r\n", "a\r\nb\nc\r\n", "a\r\nb\nC\r\n")
case("spec-18", "a\nb", "a\nB", "a\nb")
case("spec-19", "a\nb", "a\nb", "a\nb\n")
case("spec-20", "a\nb", "a\nB", "a\nb\n")
case("spec-21", "a\n", "a\nX\n", "a\nY\n")
case("spec-22", "", "x\ny\n", "x\nz\n")
case("spec-23", "a\n", "", "A\n")
case("spec-25", "﻿a\nb\nc\n", "﻿a\nB\nc\n", "﻿a\nb\nc\n")
case(
    "spec-26",
    "a\nb\nc\n1\n2\n3\na\nb\nc\nd\n",
    "a\nX\nc\n1\n2\n3\na\nB\nc\nd\n",
    "a\nX\nc\n1\n2\n3\na\nb\nC\nd\n",
)

# --- Du code qui ressemble à du vrai ------------------------------------------

PHP = """<?php

namespace App\\Stock;

use App\\Entity\\Sample;
use App\\Repository\\SampleRepository;

/**
 * Inventaire des échantillons d'un congélateur.
 */
class Inventory
{
    private SampleRepository $samples;
    private int $capacity;

    public function __construct(SampleRepository $samples, int $capacity)
    {
        $this->samples = $samples;
        $this->capacity = $capacity;
    }

    public function count(): int
    {
        return $this->samples->countAll();
    }

    public function isFull(): bool
    {
        return $this->count() >= $this->capacity;
    }

    public function add(Sample $sample): void
    {
        if ($this->isFull()) {
            throw new \\RuntimeException('Congélateur plein');
        }
        $this->samples->save($sample);
    }

    public function remove(Sample $sample): void
    {
        $this->samples->delete($sample);
    }
}
"""


def sub(text, old, new, count=1):
    assert text.count(old) >= 1, old
    return text.replace(old, new, count)


# Deux méthodes différentes : fusion propre.
case(
    "php-disjoint-methods",
    PHP,
    sub(PHP, "return $this->samples->countAll();", "return $this->samples->countActive();"),
    sub(PHP, "'Congélateur plein'", "'Plus de place dans le congélateur'"),
)

# Deux lignes voisines de la même méthode : conflit pour tout le monde.
case(
    "php-adjacent-lines",
    PHP,
    sub(PHP, "$this->samples = $samples;", "$this->samples = clone $samples;"),
    sub(PHP, "$this->capacity = $capacity;", "$this->capacity = max(0, $capacity);"),
)

# La même ligne changée différemment.
case(
    "php-same-line",
    PHP,
    sub(PHP, ">= $this->capacity", "> $this->capacity"),
    sub(PHP, ">= $this->capacity", ">= $this->capacity - 1"),
)

# Les deux côtés font la même correction.
case(
    "php-identical-fix",
    PHP,
    sub(PHP, ">= $this->capacity", "> $this->capacity"),
    sub(PHP, ">= $this->capacity", "> $this->capacity"),
)

NEW_METHOD = """
    public function clear(): void
    {
        $this->samples->deleteAll();
    }
"""

# Nouvelle méthode ajoutée après remove() ; l'autre côté modifie la dernière
# instruction de remove(). L'insertion peut se lire « après l'accolade » ou
# « avant l'accolade » : place ambiguë, à une ligne du changement d'en face.
case(
    "php-new-method-after-edited-statement",
    PHP,
    sub(PHP, "        $this->samples->delete($sample);\n    }\n", "        $this->samples->delete($sample);\n    }\n" + NEW_METHOD),
    sub(PHP, "$this->samples->delete($sample);", "$this->samples->delete($sample, true);"),
)

# Nouvelle méthode ajoutée d'un côté, changement loin de là de l'autre.
case(
    "php-new-method-far-from-edit",
    PHP,
    sub(PHP, "        $this->samples->delete($sample);\n    }\n", "        $this->samples->delete($sample);\n    }\n" + NEW_METHOD),
    sub(PHP, "use App\\Entity\\Sample;\n", "use App\\Entity\\Sample;\nuse App\\Entity\\Freezer;\n"),
)

# Chacun ajoute sa méthode au même endroit.
case(
    "php-both-add-method-same-place",
    PHP,
    sub(PHP, "        $this->samples->delete($sample);\n    }\n", "        $this->samples->delete($sample);\n    }\n" + NEW_METHOD),
    sub(
        PHP,
        "        $this->samples->delete($sample);\n    }\n",
        "        $this->samples->delete($sample);\n    }\n" + NEW_METHOD.replace("clear", "purge"),
    ),
)

# Suppression d'une méthode d'un côté, modification dedans de l'autre.
case(
    "php-delete-vs-modify",
    PHP,
    sub(PHP, "    public function isFull(): bool\n    {\n        return $this->count() >= $this->capacity;\n    }\n\n", ""),
    sub(PHP, ">= $this->capacity", "> $this->capacity"),
)

# Réindentation (espaces seuls) d'un côté, vrai changement ailleurs de l'autre.
case(
    "php-reindent-vs-far-edit",
    PHP,
    sub(PHP, "        $this->samples->delete($sample);", "\t\t$this->samples->delete($sample);"),
    sub(PHP, "return $this->samples->countAll();", "return $this->samples->countActive();"),
)

# Réindentation d'une ligne d'un côté, changement de la même ligne de l'autre.
case(
    "php-reindent-vs-same-line",
    PHP,
    sub(PHP, "        $this->samples->delete($sample);", "\t\t$this->samples->delete($sample);"),
    sub(PHP, "$this->samples->delete($sample);", "$this->samples->delete($sample, true);"),
)

# Fichier Windows (CRLF partout) : mêmes changements disjoints.
case(
    "php-crlf-disjoint",
    crlf(PHP),
    crlf(sub(PHP, "return $this->samples->countAll();", "return $this->samples->countActive();")),
    crlf(sub(PHP, "'Congélateur plein'", "'Plus de place dans le congélateur'")),
)

# Leur côté a converti tout le fichier en CRLF et changé une ligne ; le nôtre a
# changé une autre ligne.
case(
    "php-their-crlf-conversion",
    PHP,
    sub(PHP, "return $this->samples->countAll();", "return $this->samples->countActive();"),
    crlf(sub(PHP, "'Congélateur plein'", "'Plus de place dans le congélateur'")),
)

JS = """import { fetchSamples } from './api';

const PAGE_SIZE = 50;

export function listSamples(page) {
  const offset = page * PAGE_SIZE;
  return fetchSamples({ offset, limit: PAGE_SIZE });
}

export function formatSample(sample) {
  const name = sample.name.trim();
  const date = new Date(sample.createdAt);

  return `${name} (${date.getFullYear()})`;
}

export function sortSamples(samples) {
  return [...samples].sort((a, b) => a.name.localeCompare(b.name));
}
"""

# Une ligne déplacée de deux lignes vers le bas d'un côté ; l'autre côté modifie
# une ligne juste après. Un déplacement se lit de deux façons (c'est la ligne qui
# descend, ou ce sont ses voisines qui montent).
case(
    "js-moved-line-near-edit",
    JS,
    sub(
        JS,
        "  const name = sample.name.trim();\n  const date = new Date(sample.createdAt);\n\n",
        "  const date = new Date(sample.createdAt);\n\n  const name = sample.name.trim();\n",
    ),
    sub(JS, "return `${name} (${date.getFullYear()})`;", "return `${name} — ${date.getFullYear()}`;"),
)

# Trois changements d'un côté, deux de l'autre, tous éloignés.
case(
    "js-many-disjoint",
    JS,
    sub(sub(sub(JS, "const PAGE_SIZE = 50;", "const PAGE_SIZE = 100;"), "sample.name.trim()", "sample.name.trim().toUpperCase()"), "a.name.localeCompare(b.name)", "a.name.localeCompare(b.name, 'fr')"),
    sub(sub(JS, "import { fetchSamples } from './api';", "import { fetchSamples } from './api/samples';"), "  return fetchSamples({ offset, limit: PAGE_SIZE });", "  return fetchSamples({ offset, limit: PAGE_SIZE, order: 'name' });"),
)

# Une ligne vide retirée d'une suite de lignes vides : laquelle ? L'autre côté
# modifie la ligne juste avant la suite.
BLANKS = "function a() {}\n\n\n\nfunction b() {}\n"
case(
    "js-blank-run-vs-neighbour",
    BLANKS,
    "function a() {}\n\n\nfunction b() {}\n",
    "function a() { return 1; }\n\n\n\nfunction b() {}\n",
)

CSS = """.sample {
  color: #333;
  padding: 4px;
}

.sample--expired {
  color: #a00;
}
"""

# Chacun ajoute une règle différente à la fin du fichier.
case(
    "css-both-append",
    CSS,
    CSS + "\n.sample--new {\n  color: #060;\n}\n",
    CSS + "\n.sample--lost {\n  color: #999;\n}\n",
)

# Chacun ajoute la même règle à la fin.
case(
    "css-both-append-same",
    CSS,
    CSS + "\n.sample--new {\n  color: #060;\n}\n",
    CSS + "\n.sample--new {\n  color: #060;\n}\n",
)

# Propriétés différentes de la même règle, séparées par une ligne.
case(
    "css-one-line-apart",
    ".a {\n  color: red;\n  margin: 0;\n  padding: 0;\n}\n",
    ".a {\n  color: blue;\n  margin: 0;\n  padding: 0;\n}\n",
    ".a {\n  color: red;\n  margin: 0;\n  padding: 2px;\n}\n",
)

# --- Cas limites ---------------------------------------------------------------

# Aucun des trois n'a de saut de ligne final ; changements disjoints.
case("no-final-newline-all", "a\nb\nc\nd\ne", "A\nb\nc\nd\ne", "a\nb\nc\nd\nE")

# Le nôtre retire le saut de ligne final, le leur change le début.
case("final-newline-removed-by-us", "a\nb\nc\nd\n", "a\nb\nc\nd", "A\nb\nc\nd\n")

# Une ligne de 60 000 caractères, modifiée d'un côté ; l'autre change ailleurs.
LONG = "x" * 60000
case("long-line", "début\n" + LONG + "\nmilieu\nfin\n", "début\n" + LONG + "y\nmilieu\nfin\n", "début\n" + LONG + "\nmilieu\nFIN\n")

# Des marqueurs de conflit dans la base : du texte comme un autre.
MARKERS = "titre\n<<<<<<< HEAD\nun\n=======\ndeux\n>>>>>>> autre\nfin\n"
case("markers-in-base", MARKERS, sub(MARKERS, "titre", "Titre"), sub(MARKERS, "fin\n", "fin.\n"))

# Fichier vidé d'un côté, inchangé de l'autre.
case("emptied-by-them", "a\nb\n", "a\nb\n", "")

# Les trois identiques.
case("nothing-changed", "a\nb\n", "a\nb\n", "a\nb\n")

# Fins de ligne mêlées dans les trois versions, changements disjoints.
case("mixed-endings-disjoint", "a\r\nb\nc\r\nd\ne\r\n", "A\r\nb\nc\r\nd\ne\r\n", "a\r\nb\nc\r\nd\nE\r\n")

# Un retour chariot isolé au milieu d'une ligne n'est pas une fin de ligne.
case("lone-cr", "a\rb\nc\nd\n", "a\rB\nc\nd\n", "a\rb\nc\nD\n")

# Lignes répétées : notre côté retire un « x », le leur en ajoute un plus loin.
case("repeated-lines", "x\nx\nk\nx\nx\n", "x\nk\nx\nx\n", "x\nx\nk\nx\nx\nx\n")

# --- Écart M1 de la recette de la v0 : faux conflits sur du code courant -------
#
# Une ligne banale (ligne vide, accolade) se répète autour des changements. Un
# alignement brut peut l'apparier « de travers » et faire remonter le changement
# d'un côté jusqu'à toucher celui d'en face. Git range ses alignements (chaque
# changement glissé à une place déterminée) ; nous aussi désormais. Chaque cas
# propre porte son texte attendu.

# Le cas de la recette (38-php.php) : la nôtre change a(), la leur change b() et
# ajoute c() après une ligne vide, en fin de classe.
M1_PHP = """<?php
class A {
    public function a() { return 1; }

    public function b() { return 2; }
}
"""
M1_PHP_THEIRS = """<?php
class A {
    public function a() { return 1; }

    public function b() { return 20; }

    public function c() {}
}
"""
case(
    "m1-php-method-added-after-edited-neighbour",
    M1_PHP,
    sub(M1_PHP, "return 1;", "return 10;"),
    M1_PHP_THEIRS,
    expected=sub(M1_PHP_THEIRS, "return 1;", "return 10;"),
)

# Le même motif au milieu d'une classe, les deux côtés ayant aussi changé plus
# haut et plus bas : rien ne tient au début ou à la fin du fichier.
M1_MID = """<?php

final class Freezer
{
    private int $capacity = 10;

    public function name(): string
    {
        return 'A';
    }

    public function count(): int
    {
        return 1;
    }

    public function isFull(): bool
    {
        return false;
    }

    public function open(): void
    {
        $this->log('open');
    }
}
"""
M1_MID_OURS = sub(sub(M1_MID, "return 1;", "return $this->samples->count();"), "$capacity = 10", "$capacity = 20")
M1_MID_THEIRS = sub(
    sub(
        sub(M1_MID, "return 'A';", "return 'Congélateur A';"),
        "        return false;\n    }\n",
        "        return $this->count() >= $this->capacity;\n    }\n\n    public function isEmpty(): bool\n    {\n        return $this->count() === 0;\n    }\n",
    ),
    "$this->log('open');",
    "$this->log('ouvert');",
)
case(
    "m1-php-method-added-mid-class",
    M1_MID,
    M1_MID_OURS,
    M1_MID_THEIRS,
    expected=sub(sub(M1_MID_THEIRS, "return 1;", "return $this->samples->count();"), "$capacity = 10", "$capacity = 20"),
)

# Le cas « lignes répétées » de la recette (25) : chacun remplace une ligne d'un
# fichier de lignes identiques, à une ligne d'écart.
case(
    "m1-repeated-lines-two-edits",
    "x\nx\nx\nx\nx\nx\n",
    "x\nx\nY\nx\nx\nx\n",
    "x\nx\nx\nx\nZ\nx\n",
    expected="x\nx\nY\nx\nZ\nx\n",
)

# Famille « bloc inséré » de la mesure sur du vrai code : une fonction entière,
# ligne vide comprise, ajoutée juste après l'accolade d'une fonction dont
# l'autre côté a changé la dernière instruction.
M1_JS = """function load(id) {
  const sample = find(id);
  return sample;
}

function save(sample) {
  store(sample);
}
"""
M1_JS_BLOCK = "\nfunction remove(id) {\n  drop(id);\n}\n"
case(
    "m1-js-block-inserted-after-edited-function",
    M1_JS,
    sub(M1_JS, "  return sample;\n}\n", "  return sample;\n}\n" + M1_JS_BLOCK),
    sub(M1_JS, "return sample;", "return sample ?? null;"),
    expected=sub(M1_JS, "  return sample;\n}\n", "  return sample ?? null;\n}\n" + M1_JS_BLOCK),
)

# Garde-fous de la même famille : ce qui doit rester un conflit.
# La méthode ajoutée colle à la méthode que l'autre côté a modifiée (aucune
# ligne entre les deux) : conflit, pour Git comme pour nous.
case(
    "m1-php-method-added-against-edited-line",
    M1_PHP,
    sub(M1_PHP, "return 2;", "return 22;"),
    sub(M1_PHP, "    public function b() { return 2; }\n", "    public function b() { return 2; }\n    public function c() {}\n"),
)
# Une ligne retirée d'une série de lignes identiques, une autre ligne de la même
# série modifiée en face : laquelle a été retirée ? Git range la suppression en
# bas et fusionne ; nous gardons le conflit.
case(
    "m1-removed-line-of-a-run-vs-edit-in-the-run",
    "a\nx\nx\nx\nb\n",
    "a\nx\nx\nb\n",
    "a\nX\nx\nx\nb\n",
)


# Des lignes qui se répètent d'une méthode à l'autre (« { », « return; ») et un
# côté qui en recopie : plusieurs lectures aussi courtes, qu'aucun glissement ne
# ramène l'une à l'autre. Trouvé par le tirage de faux code. Git en retient une
# et fusionne... en rangeant nos deux instructions dans la méthode n0 que leur
# côté vient d'ajouter, après *son* « return; ». Nous montrons le conflit.
M1_DUP = """<?php
class A
{
    public function m0()
    {
        return;
    }
}
"""
case(
    "m1-repeated-lines-two-readings",
    M1_DUP,
    sub(M1_DUP, "        return;\n", "        return;\n        $x = 1;\n        $x = 1;\n"),
    """<?php
class A
{
    public function m0()
    public function m0()
    {
        return;

    public function n0()
    {
        return;
    }
    }
}
""",
)


# --- Motif répété décalé d'un cran entre deux changements éloignés -------------
#
# Notre côté se lit « deux lignes a insérées en haut, un b supprimé sept lignes
# plus bas » ou « le b du haut supprimé, deux a insérés plus bas » : tout le
# milieu se décale d'un cran d'une lecture à l'autre. Trouvé par le tirage
# contre Git ; l'analyse par fenêtres de la v0 ne le voyait pas et fusionnait le
# premier cas, que Git déclare en conflit.
SHIFT = "g\nb\na\na\nb\na\nb\nb\nb\ng\n"
SHIFT_OURS = "g\na\na\nb\na\na\nb\na\nb\nb\ng\n"
# Leur côté modifie la ligne que la lecture de Git touche : conflit pour Git.
case("repeat-shift-vs-edit-git-conflict", SHIFT, SHIFT_OURS, "g\nb\nZ\na\nb\na\nb\nb\nb\ng\n")
# Une ligne plus bas, Git fusionne ; pour nous aucune ligne du milieu n'est sûre.
case("repeat-shift-vs-edit-git-clean", SHIFT, SHIFT_OURS, "g\nb\na\nZ\nb\na\nb\nb\nb\ng\n")


def main():
    for name, (base, ours, theirs, expected) in sorted(CASES.items()):
        folder = os.path.join(ROOT, name)
        os.makedirs(folder, exist_ok=True)
        files = [("base", base), ("ours", ours), ("theirs", theirs)]
        if expected is not None:
            files.append(("expected", expected))
        for filename, text in files:
            with open(os.path.join(folder, filename), "wb") as handle:
                handle.write(text.encode("utf-8"))
    print(f"{len(CASES)} cas écrits dans {ROOT}")


if __name__ == "__main__":
    main()
