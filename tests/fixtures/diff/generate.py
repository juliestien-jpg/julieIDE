#!/usr/bin/env python3
"""Fabrique le corpus de fusion de julie-diff : un dossier par cas, avec trois
fichiers `base`, `ours` (la nôtre), `theirs` (la leur), écrits à l'octet près.

    python tests/fixtures/diff/generate.py

Le script réécrit `merge/<cas>/` ; il ne touche pas à `merge/cases.txt`, qui dit
pour chaque cas ce qu'on attend par rapport à `git merge-file` et pourquoi
(vérifié par `crates/diff/tests/git_compare.rs`).
"""
import os

ROOT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "merge")
CASES = {}


def case(name, base, ours, theirs):
    assert name not in CASES, name
    CASES[name] = (base, ours, theirs)


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


def main():
    for name, (base, ours, theirs) in sorted(CASES.items()):
        folder = os.path.join(ROOT, name)
        os.makedirs(folder, exist_ok=True)
        for filename, text in (("base", base), ("ours", ours), ("theirs", theirs)):
            with open(os.path.join(folder, filename), "wb") as handle:
                handle.write(text.encode("utf-8"))
    print(f"{len(CASES)} cas écrits dans {ROOT}")


if __name__ == "__main__":
    main()
