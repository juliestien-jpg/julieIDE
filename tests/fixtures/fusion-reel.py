#!/usr/bin/env python3
"""Mesure des « conflits en trop » du moteur de fusion sur du vrai code (recette v0).

Le moteur est appelé par un petit exécutable de recette (mergecli, voir docs/recette-v0.md)
qui lit des dossiers <cas>/{base,ours,theirs} et écrit <cas>/out + « cas<TAB>nb_conflits ».
Comparaison avec `git merge-file -p` sur les mêmes trois fichiers.

  python fusion-reel.py histoire <clone> <dossier-de-cas>     paires de commits d'un vrai historique
  python fusion-reel.py aleatoire <dossier-source> <ext> <n-essais-par-fichier> <dossier-de-cas> [graine]
  python fusion-reel.py comparer <dossier-de-cas> <mergecli-sortie.tsv>
"""
import os, random, subprocess, sys, tempfile, shutil


def git(cwd, *a, inp=None):
    return subprocess.run(["git", *a], cwd=cwd, input=inp, capture_output=True)


def show(repo, rev, path):
    r = git(repo, "show", "%s:%s" % (rev, path))
    return r.stdout if r.returncode == 0 else None


def merge_file(base, ours, theirs):
    with tempfile.TemporaryDirectory() as t:
        for k, v in (("base", base), ("ours", ours), ("theirs", theirs)):
            open(os.path.join(t, k), "wb").write(v)
        r = git(t, "merge-file", "-p", "ours", "base", "theirs")
        return r.stdout, r.returncode


def put(dest, name, base, ours, theirs):
    d = os.path.join(dest, name)
    os.makedirs(d, exist_ok=True)
    for k, v in (("base", base), ("ours", ours), ("theirs", theirs)):
        open(os.path.join(d, k), "wb").write(v)


def histoire(repo, dest):
    shutil.rmtree(dest, ignore_errors=True)
    os.makedirs(dest)
    revs = git(repo, "rev-list", "--first-parent", "--reverse", "HEAD").stdout.decode().split()
    n = 0
    files = {}
    for r in revs:
        for f in git(repo, "diff-tree", "--no-commit-id", "--name-only", "-r", "--root", r).stdout.decode().splitlines():
            files.setdefault(f, []).append(r)
    for f, rs in files.items():
        for a in range(len(rs)):
            for b in range(a + 1, len(rs)):
                i, j = rs[a], rs[b]
                base = show(repo, i + "^", f)
                ours = show(repo, i, f)
                bj, tj = show(repo, j + "^", f), show(repo, j, f)
                if None in (base, ours, bj, tj):
                    continue
                # « leur » = le correctif j rejoué sur l'état d'avant i (comme un cherry-pick)
                theirs, rc = merge_file(bj, base, tj) if False else merge_file(bj, base, tj)
                # merge-file ours=base base=bj theirs=tj  -> reporte j sur base
                if rc != 0:
                    continue
                n += 1
                put(dest, "h%04d-%s-%s-%s" % (n, i[:7], j[:7], f.replace("/", "_")), base, ours, theirs)
    print("cas d'historique :", n)


def mutate(rng, data, k):
    lines = data.split(b"\n")
    for _ in range(k):
        if len(lines) < 5:
            break
        p = rng.randrange(len(lines))
        t = rng.random()
        cr = b"\r" if lines[p].endswith(b"\r") else b""   # respecte les fins de ligne du fichier
        if t < 0.5:
            lines[p] = lines[p][:len(lines[p]) - len(cr)] + b" /*m%d*/" % rng.randrange(1000) + cr
        elif t < 0.8:
            if os.environ.get("BLOCS"):   # insertion d'un « petit bloc de code » : lignes vides et accolades comme dans du vrai code
                n = rng.randrange(1000)
                lines[p:p] = [x + cr for x in (b"", b"    public function m%d()" % n, b"    {", b"        return %d;" % n, b"    }")]
            else:
                lines[p:p] = [b"// ajout %d" % rng.randrange(1000) + cr] * rng.randint(1, 3)
        else:
            del lines[p:p + rng.randint(1, 3)]
    return b"\n".join(lines)


def aleatoire(src, ext, trials, dest, seed=1):
    rng = random.Random(int(seed))
    shutil.rmtree(dest, ignore_errors=True)
    os.makedirs(dest)
    n = 0
    for root, _, fs in os.walk(src):
        if any(x in root for x in ("node_modules", ".git")):
            continue
        for f in fs:
            if not f.endswith(ext):
                continue
            data = open(os.path.join(root, f), "rb").read()
            if len(data) < 800 or len(data) > 400000 or b"\0" in data:
                continue
            for _ in range(int(trials)):
                n += 1
                k = rng.choice([1, 2, 3, 5, 8])
                put(dest, "r%05d-%s" % (n, f), data, mutate(rng, data, k), mutate(rng, data, k))
    print("cas aléatoires :", n)


def comparer(dest, tsv):
    stats = dict(total=0, skip=0, both_clean_same=0, both_clean_diff=0, false_conflict=0, silent=0, both_conflict=0)
    bad = []
    for line in open(tsv, encoding="utf-8"):
        name, res = line.rstrip("\n").split("\t")
        if res == "SKIP":
            stats["skip"] += 1
            continue
        d = os.path.join(dest, name)
        rd = lambda k: open(os.path.join(d, k), "rb").read()
        gtxt, grc = merge_file(rd("base"), rd("ours"), rd("theirs"))
        stats["total"] += 1
        oc = int(res)
        if grc == 0 and oc == 0:
            if open(os.path.join(d, "out"), "rb").read() == gtxt:
                stats["both_clean_same"] += 1
            else:
                stats["both_clean_diff"] += 1
                bad.append(("DIFF-TEXTE", name))
        elif grc == 0 and oc > 0:
            stats["false_conflict"] += 1
            bad.append(("CONFLIT-EN-TROP", name))
        elif grc > 0 and oc == 0:
            stats["silent"] += 1
            bad.append(("GIT-CONFLIT/NOUS-PROPRE", name))
        else:
            stats["both_conflict"] += 1
    print(stats)
    gc = stats["both_clean_same"] + stats["both_clean_diff"] + stats["false_conflict"]
    if gc:
        print("fusions propres pour Git : %d ; conflit en trop chez nous : %d = %.2f %%" % (gc, stats["false_conflict"], 100.0 * stats["false_conflict"] / gc))
    out = os.path.join(dest, "_ecarts.txt")
    open(out, "w", encoding="utf-8").write("\n".join("%s\t%s" % x for x in bad))


if __name__ == "__main__":
    c = sys.argv[1]
    if c == "histoire":
        histoire(sys.argv[2], sys.argv[3])
    elif c == "aleatoire":
        aleatoire(*sys.argv[2:])
    elif c == "comparer":
        comparer(sys.argv[2], sys.argv[3])
