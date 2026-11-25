#!/usr/bin/env python3
"""Corpus de fusion de la recette v0 : fabrique un dépôt jetable en fusion dont chaque
fichier de `cas/` a ses trois étapes (base, nôtre, leur) posées DIRECTEMENT dans l'index
(git update-index --index-info), ce qui permet des combinaisons que Git fusionnerait seul.

Sous-commandes :
  build    <dossier>               fabrique le dépôt (écrase) et écrit cas.json (manifeste)
  scenario <dossier> <mode> <out>  écrit un scénario ; mode = classify | apply | left | right | both
  check    <dossier> <mode>        compare les fichiers écrits avec l'attendu (git merge-file)

Les modes de `scenario` : classify = ouvrir, état, appliquer tous, état, fermer sans écrire ;
apply = appliquer tous puis enregistrer (les conflits gardent le texte de la base) ;
left / right / both = en plus, accepter ce côté sur chaque conflit restant.
"""
import json, os, subprocess, sys, shutil, tempfile

LF, CRLF = b"\n", b"\r\n"


def lines(n, prefix=b"ligne ", eol=LF):
    return b"".join(prefix + str(i).encode() + eol for i in range(1, n + 1))


def sub(data, old, new):
    assert old in data, (old, data[:80])
    return data.replace(old, new, 1)


def big():
    base = lines(3000)
    ours, theirs = base, base
    for i in range(50, 3000, 60):          # modifs de notre côté, lignes impaires
        ours = sub(ours, b"ligne %d\n" % i, b"NOUS %d\n" % i)
    for i in range(80, 3000, 60):          # modifs de leur côté, ailleurs
        theirs = sub(theirs, b"ligne %d\n" % i, b"EUX %d\n" % i)
    for i in (500, 1500, 2500):            # 3 vrais conflits
        ours = sub(ours, b"ligne %d\n" % (i + 1), b"NOUS-conflit %d\n" % i)
        theirs = sub(theirs, b"ligne %d\n" % (i + 1), b"EUX-conflit %d\n" % i)
    return base, ours, theirs


def mixed(base_extra=b""):
    return b"un\r\ndeux\ntrois\r\nquatre\ncinq\r\nsix\nsept\r\n" + base_extra


CASES = {}


def case(name, base, ours, theirs, note="", expect=None):
    CASES[name] = dict(base=base, ours=ours, theirs=theirs, note=note, expect=expect)


b10 = lines(10)
case("01-disjoint-loin.txt", b10, sub(b10, b"ligne 2\n", b"DEUX\n"), sub(b10, b"ligne 8\n", b"HUIT\n"), "changements disjoints")
case("02-disjoint-plus-conflit.txt", b10,
     sub(sub(b10, b"ligne 2\n", b"DEUX\n"), b"ligne 5\n", b"CINQ-nous\n"),
     sub(sub(b10, b"ligne 9\n", b"NEUF\n"), b"ligne 5\n", b"CINQ-eux\n"), "disjoints + 1 vrai conflit")
case("03-meme-ligne.txt", b10, sub(b10, b"ligne 4\n", b"A\n"), sub(b10, b"ligne 4\n", b"B\n"), "même ligne")
case("04-ajout-identique.txt", b10, sub(b10, b"ligne 4\n", b"ligne 4\nNOUVEAU\n"), sub(b10, b"ligne 4\n", b"ligne 4\nNOUVEAU\n"), "ajouts identiques + conflit ailleurs")
case("05-modif-identique-plus-conflit.txt", b10,
     sub(sub(b10, b"ligne 2\n", b"X\n"), b"ligne 8\n", b"A\n"),
     sub(sub(b10, b"ligne 2\n", b"X\n"), b"ligne 8\n", b"B\n"), "modif identique + conflit")
case("06-suppr-contre-modif.txt", b10, sub(b10, b"ligne 5\n", b""), sub(b10, b"ligne 5\n", b"CINQ\n"), "supprimé contre modifié")
case("07-suppr-des-deux-cotes.txt", b10, sub(b10, b"ligne 5\n", b""), sub(sub(b10, b"ligne 5\n", b""), b"ligne 9\n", b"NEUF\n"), "suppression identique + modif")
case("08-crlf-nous-lf-eux.txt", lines(8, eol=CRLF), sub(lines(8, eol=CRLF), b"ligne 2\r\n", b"DEUX\r\n"),
     sub(lines(8, eol=LF), b"ligne 7\n", b"SEPT\n"), "nous CRLF, eux LF (base CRLF)")
case("09-crlf-partout.txt", lines(10, eol=CRLF), sub(lines(10, eol=CRLF), b"ligne 2\r\n", b"DEUX\r\n"),
     sub(lines(10, eol=CRLF), b"ligne 8\r\n", b"HUIT\r\n"), "CRLF partout, disjoint")
case("10-sans-final-tous.txt", b"a\nb\nc", b"a\nb\nC", b"A\nb\nc", "sans saut final, disjoint")
case("11-sans-final-conflit.txt", b"a\nb\nc", b"a\nb\nC", b"a\nb\nc\n", "dernière ligne: modifiée vs saut final ajouté")
case("12-bom-partout.txt", b"\xef\xbb\xbf" + b10, b"\xef\xbb\xbf" + sub(b10, b"ligne 2\n", b"DEUX\n"), b"\xef\xbb\xbf" + sub(b10, b"ligne 8\n", b"HUIT\n"), "BOM partout")
case("13-bom-nous-seul.txt", b10, b"\xef\xbb\xbf" + sub(b10, b"ligne 2\n", b"DEUX\n"), sub(b10, b"ligne 8\n", b"HUIT\n"), "BOM chez nous seulement")
case("14-bom-eux-seul.txt", b10, sub(b10, b"ligne 2\n", b"DEUX\n"), b"\xef\xbb\xbf" + sub(b10, b"ligne 8\n", b"HUIT\n"), "BOM chez eux seulement")
acc = "café déjà été\nnaïve œuvre Ünïcode\n日本語のテキスト\nemoji 😀 fin\nligne5 ñ\nligne6\n".encode()
case("15-accents.txt", acc, sub(acc, "café".encode(), "CAFÉ".encode()), sub(sub(acc, "日本語".encode(), "中文".encode()), "ligne5 ñ".encode(), "LIGNE5 Ñ".encode()), "accents/CJK/emoji")
bb, bo, bt = big()
case("16-gros-3000.txt", bb, bo, bt, "3000 lignes, ~50 modifs de chaque côté et 3 conflits")
case("17-fins-mixtes.txt", mixed(), sub(mixed(), b"deux\n", b"DEUX\n"), sub(mixed(), b"sept\r\n", b"SEPT\r\n"), "fichier à fins mixtes")
case("18-lignes-adjacentes.txt", b10, sub(b10, b"ligne 3\n", b"TROIS\n"), sub(b10, b"ligne 4\n", b"QUATRE\n"), "lignes contiguës")
case("19-insertion-meme-point.txt", b10, sub(b10, b"ligne 5\n", b"ligne 5\nX\n"), sub(b10, b"ligne 5\n", b"ligne 5\nY\n"), "insertions au même point")
case("20-insertion-contre-modif.txt", b10, sub(b10, b"ligne 5\n", b"X\nligne 5\n"), sub(b10, b"ligne 5\n", b"CINQ\n"), "insertion contiguë à modif")
case("21-espaces-seuls.txt", b"a\n  b\nc\n", b"a\n    b\nc\n", b"a\n  B\nc\n", "espaces d'un côté")
case("22-espaces-fin-de-ligne.txt", b10, sub(b10, b"ligne 3\n", b"ligne 3 \n"), sub(b10, b"ligne 6\n", b"SIX\n"), "espace final chez nous")
case("23-ajoute-des-deux-cotes.txt", None, b"x\ny\n", b"x\nz\n", "ajouté des deux côtés")
case("24-nous-vide.txt", b"a\nb\n", b"", sub(b"a\nb\n", b"b\n", b"B\n"), "nous: fichier vidé; eux: modifié")
case("25-lignes-repetees.txt", b"x\nx\nx\nx\nx\nx\n", b"x\nx\nY\nx\nx\nx\n", b"x\nx\nx\nx\nZ\nx\n", "lignes répétées")
case("26-cr-isole.txt", b"a\rb\nc\n", b"a\rB\nc\n", b"a\rb\nC\n", "CR isolé")
case("27-reindent-contre-edition.txt", b"if (a) {\n  x();\n  y();\n}\n", b"if (a) {\n    x();\n    y();\n}\n", b"if (a) {\n  x();\n  Y();\n}\n", "réindentation contre édition")
long_base = b"debut\n" + b"L" * 200000 + b"\nfin\n"
case("28-ligne-geante.txt", long_base, sub(long_base, b"debut\n", b"DEBUT\n"), sub(long_base, b"fin\n", b"FIN\n"), "ligne de 200 000 caractères")
mk = b"avant\n<<<<<<< HEAD\nxx\n=======\nyy\n>>>>>>> autre\napres\n"
case("29-marqueurs-dans-la-base.txt", mk, sub(mk, b"avant\n", b"AVANT\n"), sub(mk, b"apres\n", b"APRES\n"), "marqueurs littéraux dans le texte")
case("30 nom avec espaces é.txt", b10, sub(b10, b"ligne 2\n", b"DEUX\n"), sub(b10, b"ligne 2\n", b"deux\n"), "nom de fichier avec espaces/accents")
case("31-suppressions-disjointes.txt", b10, sub(b10, b"ligne 2\n", b""), sub(b10, b"ligne 8\n", b""), "suppressions disjointes")
case("32-suppressions-chevauchantes.txt", b10, sub(sub(b10, b"ligne 4\n", b""), b"ligne 5\n", b""), sub(sub(b10, b"ligne 5\n", b""), b"ligne 6\n", b""), "suppressions qui se chevauchent")
case("33-eux-convertit-crlf.txt", b"a\nb\nc\n", b"a\nB\nc\n", b"a\r\nb\r\nC\r\n", "eux CRLF + modif ; nous LF + modif")
case("34-nous-sans-final-eux-avec.txt", b"a\nb", b"a\nb", b"a\nb\nc\n", "eux ajoute une ligne et le saut final")
case("35-tout-pareil-sauf-eux.txt", b10, b10, sub(b10, b"ligne 5\n", b"CINQ\n"), "un seul côté (Git le résoudrait seul)")
case("36-bloc-vide-inserer-au-debut.txt", b10, b"PRE\n" + b10, sub(b10, b"ligne 1\n", b"UN\n"), "insertion en tête contre modif ligne 1")
case("37-insertion-en-fin.txt", b10, b10 + b"fin-nous\n", b10 + b"fin-eux\n", "ajouts en fin de fichier")
case("38-php.php", b"<?php\nclass A {\n    public function a() { return 1; }\n\n    public function b() { return 2; }\n}\n",
     b"<?php\nclass A {\n    public function a() { return 10; }\n\n    public function b() { return 2; }\n}\n",
     b"<?php\nclass A {\n    public function a() { return 1; }\n\n    public function b() { return 20; }\n\n    public function c() {}\n}\n", "PHP, méthodes disjointes")
_l = lambda s: "".join(c + "\n" for c in s.split()).encode()
case("39-faille-lignes-repetees-decalees.txt", _l("g b a a b a b b b g"), _l("g a a b a a b a b b g"), _l("g b Z a b a b b b g"),
     "faille de la contre-recette : lignes répétées décalées contre une modification (Git : conflit)")


def git(cwd, *args, inp=None, check=True):
    r = subprocess.run(["git", *args], cwd=cwd, input=inp, capture_output=True)
    if check and r.returncode not in (0,):
        raise SystemExit("git %s : %s" % (args, r.stderr.decode(errors="replace")))
    return r


def build(dest):
    if os.path.exists(dest):
        shutil.rmtree(dest, onerror=lambda f, p, e: (os.chmod(p, 0o700), f(p)))
    os.makedirs(dest)
    git(dest, "init", "-q", "-b", "main")
    for k, v in (("user.name", "Essai"), ("user.email", "essai@juliestorm.invalid"), ("core.autocrlf", "false")):
        git(dest, "config", k, v)
    open(os.path.join(dest, "README.md"), "wb").write(b"corpus\n")
    git(dest, "add", "-A"); git(dest, "commit", "-q", "-m", "init")
    git(dest, "checkout", "-q", "-b", "eux")
    open(os.path.join(dest, "autre.txt"), "wb").write(b"autre\n")
    git(dest, "add", "-A"); git(dest, "commit", "-q", "-m", "eux")
    eux = git(dest, "rev-parse", "HEAD").stdout.decode().strip()
    git(dest, "checkout", "-q", "main")
    entries = []
    manifest = {}
    for name, c in CASES.items():
        sha = {}
        for stage, key in ((1, "base"), (2, "ours"), (3, "theirs")):
            if c[key] is None:
                continue
            sha[stage] = git(dest, "hash-object", "-w", "--stdin", inp=c[key]).stdout.decode().strip()
        for stage, s in sha.items():
            entries.append("100644 %s %d\t%s\n" % (s, stage, name))
        # fichier de travail : sortie de git merge-file avec marqueurs, comme après un vrai merge
        with tempfile.TemporaryDirectory() as t:
            for k in ("base", "ours", "theirs"):
                open(os.path.join(t, k), "wb").write(c[k] or b"")
            r = git(t, "merge-file", "-p", "ours", "base", "theirs", check=False)
            work = r.stdout
            manifest[name] = dict(git_conflicts=r.returncode, note=c["note"])
        open(os.path.join(dest, name), "wb").write(work)
    git(dest, "update-index", "--index-info", inp="".join(entries).encode())
    open(os.path.join(dest, ".git", "MERGE_HEAD"), "w").write(eux + "\n")
    open(os.path.join(dest, ".git", "MERGE_MSG"), "w").write("Merge branch 'eux'\n")
    open(os.path.join(dest, ".git", "MERGE_MODE"), "w").write("")
    os.makedirs(os.path.join(dest, ".cas"), exist_ok=True)
    for name, c in CASES.items():
        for k in ("base", "ours", "theirs"):
            if c[k] is not None:
                open(os.path.join(dest, ".git", "cas-" + name + "." + k), "wb").write(c[k])
    json.dump(manifest, open(os.path.join(dest, ".git", "cas.json"), "w"), ensure_ascii=False, indent=1)
    print(git(dest, "status", "--short").stdout.decode())


def scenario(dest, mode, out):
    dest_s = dest.replace("\\", "/")
    ev = os.path.join(os.path.dirname(out), "etat-" + mode).replace("\\", "/")
    s = ["# Corpus de fusion (généré par tests/fixtures/fusion-corpus.py, mode %s)" % mode,
         "project " + dest_s, "action view.git", "action view.bottom_tall", "wait-idle"]
    for i, name in enumerate(CASES):
        tag = "%02d" % (i + 1)
        s += ["ui merge.open " + name, "wait-idle", "state %s/%s-ouvert.json" % (ev, tag)]
        if mode == "classify":
            s += ["ui merge.apply all", "state %s/%s-apres.json" % (ev, tag), "ui merge.close", "ui confirm"]
            continue
        s += ["ui merge.apply all"]
        if mode in ("left", "right", "both"):
            # blocs conflictuels connus par la passe « classify »
            kinds = json.load(open("%s/%s-ouvert.json" % (ev.replace("etat-" + mode, "etat-classify"), tag), encoding="utf-8")) if os.path.exists("%s/%s-ouvert.json" % (ev.replace("etat-" + mode, "etat-classify"), tag)) else None
            if kinds and kinds.get("merge"):
                for b in kinds["merge"]["blocks"]:
                    if b["kind"].lower().startswith("conf"):
                        if mode == "both":
                            s.append("ui merge.both %d left" % b["id"])
                        else:
                            s.append("ui merge.accept %d %s" % (b["id"], mode))
        s += ["state %s/%s-avant-save.json" % (ev, tag), "ui merge.save", "ui confirm", "wait-idle", "state %s/%s-apres.json" % (ev, tag)]
    s += ["quit"]
    open(out, "w", encoding="utf-8").write("\n".join(s) + "\n")


def read_case(dest, name):
    d = {}
    for k in ("base", "ours", "theirs"):
        p = os.path.join(dest, ".git", "cas-" + name + "." + k)
        d[k] = open(p, "rb").read() if os.path.exists(p) else None
    return d


def reference(c, mode):
    """Attendu d'après git merge-file --diff3 : conflits remplacés par base / nôtre / leur."""
    with tempfile.TemporaryDirectory() as t:
        for k in ("base", "ours", "theirs"):
            open(os.path.join(t, k), "wb").write(c[k] or b"")
        r = git(t, "merge-file", "-p", "--diff3", "-L", "O", "-L", "B", "-L", "T", "ours", "base", "theirs", check=False)
    out, state, buf = [], 0, {"O": [], "B": [], "T": []}
    for ln in r.stdout.splitlines(keepends=True):
        if state == 0 and ln.startswith(b"<<<<<<< O"):
            state, buf = "O", {"O": [], "B": [], "T": []}
        elif state == "O" and ln.startswith(b"||||||| B"):
            state = "B"
        elif state == "B" and ln.startswith(b"=======") and len(ln.strip()) == 7:
            state = "T"
        elif state == "T" and ln.startswith(b">>>>>>> T"):
            pick = {"apply": "B", "left": "O", "right": "T"}.get(mode, "B")
            out += buf[pick] if mode != "both" else buf["O"] + buf["T"]
            state = 0
        elif state in ("O", "B", "T"):
            buf[state].append(ln)
        else:
            out.append(ln)
    return b"".join(out), r.returncode


def check(dest, mode):
    ok = bad = 0
    for name in CASES:
        c = read_case(dest, name)
        got = open(os.path.join(dest, name), "rb").read() if os.path.exists(os.path.join(dest, name)) else None
        stages = git(dest, "ls-files", "-u", "--", name).stdout.decode().strip()
        exp, n = reference(c, mode)
        status = "RESOLU" if not stages else "EN-CONFLIT"
        same = got == exp
        print("%-40s git-conflits=%-2s %-10s %s" % (name, n, status, "= git" if same else "DIFF"))
        if not same:
            bad += 1
            print("    attendu (git) : %r" % exp[:300])
            print("    obtenu        : %r" % (got[:300] if got is not None else None))
        else:
            ok += 1
    print("identiques à git : %d / %d" % (ok, ok + bad))


def limites(dest):
    """Dépôt en fusion avec les cas limites de taille : > 20 Mio, > 200 000 lignes, binaire, Windows-1252.
    Aucun de ces fichiers ne doit ouvrir le dialogue (choix de côté seulement) ; normal.txt est le témoin."""
    global CASES
    saved = CASES
    mb = 1024 * 1024
    big = lambda n, edit: b"".join((b"EDIT\n" if i == edit else b"ligne normale numero %d\n" % i) for i in range(n))
    CASES = {}
    n_big = (22 * mb) // 25
    case("enorme-22Mio.txt", big(n_big, -1), big(n_big, 10), big(n_big, n_big - 10), "plus de 20 Mio")
    case("beaucoup-de-lignes-250k.txt", b"a\n" * 250000, b"b\n" + b"a\n" * 249999, b"a\n" * 249999 + b"c\n", "plus de 200 000 lignes")
    case("binaire.bin", b"BIN\0\1base\n", b"BIN\0\1nous\n", b"BIN\0\1eux\n", "NUL")
    case("latin1.txt", "café base\n".encode("cp1252"), "café nous\n".encode("cp1252"), "café eux\n".encode("cp1252"), "non UTF-8")
    case("normal.txt", b"a\nb\nc\n", b"A\nb\nc\n", b"a\nb\nC\n", "témoin")
    build(dest)
    CASES = saved


if __name__ == "__main__":
    cmd = sys.argv[1]
    if cmd == "build":
        build(sys.argv[2])
    elif cmd == "limites":
        limites(sys.argv[2])
    elif cmd == "limites":
        limites(sys.argv[2])
    elif cmd == "scenario":
        scenario(sys.argv[2], sys.argv[3], sys.argv[4])
    elif cmd == "check":
        check(sys.argv[2], sys.argv[3])
