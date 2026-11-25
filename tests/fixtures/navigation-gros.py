#!/usr/bin/env python3
"""Genere un projet PHP synthetique (par defaut 2 000 petites classes) pour voir l'index des
symboles se construire : progression dans la barre d'etat, message « Indexation en cours (n %) ».

Usage : python tests/fixtures/navigation-gros.py [dossier] [nombre de dossiers de 100 classes]
Defaut : %TEMP%/jsessai-synth/proj, 20 dossiers. Efface le dossier de destination d'abord.
"""
import os
import shutil
import sys

dest = sys.argv[1] if len(sys.argv) > 1 else os.path.join(os.environ.get("TEMP", "/tmp"), "jsessai-synth", "proj")
folders = int(sys.argv[2]) if len(sys.argv) > 2 else 20
shutil.rmtree(dest, ignore_errors=True)
count = 0
for d in range(folders):
    path = os.path.join(dest, "src", f"d{d}")
    os.makedirs(path, exist_ok=True)
    for f in range(100):
        count += 1
        methods = "".join(
            f"    public function method{m}(int $a, string $b): string "
            f"{{ return $this->method{(m + 1) % 20}($a, $b) . 'x'; }}\n"
            for m in range(20)
        )
        with open(os.path.join(path, f"Class{d}_{f}.php"), "w", encoding="utf-8", newline="\r\n") as out:
            out.write(f"<?php\nnamespace App\\D{d};\n\nclass Class{d}_{f}\n{{\n    private int $id = {count};\n{methods}}}\n")
print(f"{count} fichiers dans {dest}")
