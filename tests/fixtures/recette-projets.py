#!/usr/bin/env python3
"""Projets jetables de la recette v0 (dans %TEMP%), pour les scénarios tests/scenarios/recette-*.txt.

  python recette-projets.py edit|search|term|robust|all

  edit    %TEMP%\\jsessai-edit     fichiers aux fins de ligne / encodages variés (CRLF, BOM, mixte, sans saut final,
                                   Windows-1252, vide, lecture seule), PHP de 10 000 lignes, dépôt Git
  search  %TEMP%\\jsessai-search   .gitignore, UserController.php, mots entiers, casse, CRLF, binaire, 12 Mo, 12 000 lignes
  term    %TEMP%\\jsessai term (x86) é   dossier sans Git, chemin avec espaces, parenthèses et accent
  robust  %TEMP%\\jsessai-robust   fichier de 150 Mo, binaire, ligne de 5 Mo, un million de lignes
Aucun de ces dossiers n'est un vrai projet ; les supprimer après l'essai.
"""
import os, subprocess, sys, shutil, stat

TEMP = os.environ.get("TEMP", "/tmp")


def fresh(name):
    p = os.path.join(TEMP, name)
    if os.path.exists(p):
        shutil.rmtree(p, onerror=lambda f, q, e: (os.chmod(q, stat.S_IWRITE), f(q)))
    os.makedirs(p)
    return p


def w(root, name, data):
    full = os.path.join(root, name)
    os.makedirs(os.path.dirname(full) or ".", exist_ok=True)
    open(full, "wb").write(data)


def git(root, *a):
    subprocess.run(["git", "-c", "user.name=Essai", "-c", "user.email=essai@juliestorm.invalid", *a], cwd=root, capture_output=True)


def edit():
    r = fresh("jsessai-edit")
    git(r, "init", "-q", "-b", "main")
    git(r, "config", "core.autocrlf", "false")
    w(r, "crlf.txt", b"un\r\ndeux\r\ntrois\r\n")
    w(r, "lf-bom.txt", b"\xef\xbb\xbfun\ndeux\ntrois\n")
    w(r, "mixed.txt", b"un\r\ndeux\ntrois\r\nquatre\ncinq\r\n")
    w(r, "nofinal.txt", b"un\ndeux\ntrois")
    w(r, "nofinal-crlf.txt", b"un\r\ndeux\r\ntrois")
    w(r, "latin1.txt", "café déjà\nligne 2 é\n".encode("cp1252"))
    w(r, "mix.php", b"<!DOCTYPE html>\n<html>\n<body>\n<h1 class=\"t\">Titre</h1>\n<?php\nclass A { public function f(int $x): string { return \"v=$x\"; } }\necho (new A)->f(3);\n?>\n<style>\n.a { color: #fff; margin: 0 auto; }\n</style>\n<script>\nconst x = 1; function g(a) { return a + 1; }\n</script>\n</body>\n</html>\n")
    w(r, "big.php", ("<?php\n" + "".join("function f%d(int $a): int { return $a + %d; } // ligne %d\n" % (i, i, i) for i in range(1, 10000))).encode())
    w(r, "dossier é/sous dossier/fichier (1).php", "<?php\n// accentué é\necho 'ok';\n".encode())
    w(r, "same.txt", b"un\ndeux\n")
    w(r, "ro.txt", b"lecture seule\n")
    w(r, "ext.txt", b"externe\nligne 2\n")
    w(r, "empty.txt", b"")
    w(r, "tab.js", b"function a() {\n\treturn 1;\n}\n")
    os.chmod(os.path.join(r, "ro.txt"), stat.S_IREAD)
    git(r, "add", "-A", "-f", ".")
    git(r, "commit", "-qm", "init")
    return r


def search():
    r = fresh("jsessai-search")
    git(r, "init", "-q", "-b", "main")
    w(r, ".gitignore", b"ignored/\n*.log\nvendor/\n")
    w(r, "src/Controller/UserController.php", b"<?php\nclass UserController {\n    public function show($userId) {\n        $orderId = 5;\n        $id = 1;\n        return function () use ($userId) { return $userId; };\n    }\n    public function functions() {}\n}\n")
    w(r, "src/Controller/OrderController.php", b"<?php\nclass OrderController { function index() { $customerId = 3; $x = $customerId + 1; } }\n")
    w(r, "src/Functions.php", b"<?php\n// functions helper\nfunction a() {}\nfunction b() {}\n$functions = 1;\n")
    w(r, "index.php", b"".join(b"line %d\n" % i for i in range(1, 11)) + b"<?php\n// ligne 12 function\necho 'x';\n")
    w(r, "app.js", b"const Foo = 1;\nconst foo = 2;\nconst FOO = 3;\nfoo(foo, Foo);\n")
    w(r, "ignored/secret.php", b"<?php function secret() {}\n")
    w(r, "vendor/x/lib.php", b"<?php function vendorlib() {}\n")
    w(r, "debug.log", b"function log\n")
    w(r, "new.php", b"<?php function untracked_but_visible() {}\n")
    w(r, "dossier é/été fichier.txt", "texte avec foo accentué é\n".encode())
    w(r, "crlf.txt", b"foo\r\nbar foo\r\nend foo")
    w(r, "bin.dat", b"foo\0\1\2foo\n")
    w(r, "latin1.txt", "foo café\n".encode("cp1252"))
    w(r, "big.txt", b"foo bar baz\n" * 1000000)
    w(r, "many.txt", b"".join(b"foo %d foo\n" % i for i in range(1, 12001)))
    git(r, "add", "-A", ".")
    git(r, "commit", "-qm", "init")
    w(r, "new2.php", b"untracked\n")
    return r


def term():
    r = fresh("jsessai term (x86) é")
    w(r, "a.txt", b"bonjour\n")
    return r


def robust():
    r = fresh("jsessai-robust")
    w(r, "huge.log", (b"x" * 79 + b"\n") * (150 * 1024 * 1024 // 80))
    w(r, "bin.dat", os.urandom(1_000_000))
    w(r, "longline.txt", b"abcdefghij" * 500000 + b"\n")
    w(r, "million.txt", b"".join(b"ligne %d\n" % i for i in range(1, 1_000_001)))
    w(r, "small.txt", b"ok\n")
    return r


if __name__ == "__main__":
    what = sys.argv[1] if len(sys.argv) > 1 else "all"
    for name, fn in (("edit", edit), ("search", search), ("term", term), ("robust", robust)):
        if what in (name, "all"):
            print(fn())
