#!/usr/bin/env python3
"""Générateur d'attaque du moteur de fusion (contre-recette du 25/11/2026).

Fabrique des triplets base / nôtre / leur de familles « difficiles » (lignes répétées décalées,
séries de lignes identiques, accolades et lignes vides de code, alphabets de 2 à 4 lignes), les passe à
`mergecli` (qui appelle julie_diff::merge) et à `git merge-file -p`, puis classe :

  nous propre / Git en conflit   -> la propriété intouchable est violée (compté, cas conservés)
  Git propre / nous en conflit   -> conflit en trop (côté sûr)
  deux propres, texte différent  -> écart de texte à expliquer
  deux en conflit                -> rien à dire

Usage : python fusion-attaque.py <dossier> <mergecli.exe> <famille|toutes> <n> [graine]
Familles : aleatoire, repetees, series, code, faille, decalage
"""
import os, random, shutil, subprocess, sys, tempfile
from multiprocessing import Pool

ALPHA = "abg"


def lines_join(ls, eol="\n", final=True):
    return (eol.join(ls) + (eol if final else "")).encode()


def edit(rng, ls, alphabet, k):
    ls = list(ls)
    for _ in range(k):
        t = rng.random()
        p = rng.randrange(len(ls) + 1)
        if t < 0.35 and ls:
            ls[min(p, len(ls) - 1)] = rng.choice(alphabet + ["Z"])
        elif t < 0.7:
            ls[p:p] = [rng.choice(alphabet) for _ in range(rng.randint(1, 3))]
        elif ls:
            del ls[min(p, len(ls) - 1):min(p, len(ls) - 1) + rng.randint(1, 3)]
    return ls


def fam_aleatoire(rng):
    al = list(ALPHA[: rng.randint(2, 3)] + "g")
    base = [rng.choice(al) for _ in range(rng.randint(4, 14))]
    return base, edit(rng, base, al, rng.randint(1, 3)), edit(rng, base, al, rng.randint(1, 3))


def fam_repetees(rng):
    # lignes répétées décalées : base a/b répétés ; nôtre = insertion/suppression d'une ou deux lignes au début ; leur = remplacement
    al = ["a", "b"]
    base = [rng.choice(al) for _ in range(rng.randint(6, 16))]
    o = list(base)
    s = rng.randint(0, 3)
    for _ in range(rng.randint(1, 3)):
        p = rng.randint(0, min(len(o), 2 + s))
        if rng.random() < 0.5:
            o[p:p] = [rng.choice(al)] * rng.randint(1, 2)
        elif o:
            del o[min(p, len(o) - 1)]
    t = list(base)
    p = rng.randrange(len(t))
    t[p] = "Z"
    if rng.random() < 0.4:
        p = rng.randrange(len(t))
        t[p] = "Y"
    return base, o, t


def fam_series(rng):
    n = rng.randint(5, 14)
    x = rng.choice(["a", "}", ""])
    base = [x] * n
    def mut():
        l = list(base)
        for _ in range(rng.randint(1, 3)):
            t = rng.random()
            p = rng.randrange(len(l) + 1)
            if t < 0.4:
                l[p:p] = [x] * rng.randint(1, 3)
            elif t < 0.7 and l:
                del l[min(p, len(l) - 1):min(p, len(l) - 1) + rng.randint(1, 3)]
            elif l:
                l[min(p, len(l) - 1)] = rng.choice(["Z", "Q", x + x])
        return l
    return base, mut(), mut()


CODE_LINES = ["", "{", "}", "    }", "    {", "return;", "x();", "    return 1;", "// c", "    public function f()", "else {", "    $a = 1;", "    $b = 2;"]


def fam_code(rng):
    # « méthodes » : blocs  [ "", "public function m() {", "  return n;", "}" ]
    def meth(i):
        return ["", "    public function m%d()" % i, "    {", "        return %d;" % i, "    }"]
    k = rng.randint(2, 5)
    base = ["<?php", "class A", "{"]
    for i in range(k):
        base += meth(i)
    base += ["}"]
    def mut():
        l = list(base)
        for _ in range(rng.randint(1, 2)):
            t = rng.random()
            if t < 0.45:
                p = rng.randrange(3, len(l))
                l[p:p] = meth(100 + rng.randrange(900))
            elif t < 0.75:
                idx = [i for i, s in enumerate(l) if "return" in s]
                l[rng.choice(idx)] += " /*x%d*/" % rng.randrange(99)
            else:
                p = rng.randrange(3, len(l) - 1)
                del l[p:p + rng.randint(1, 5)]
        return l
    return base, mut(), mut()


def fam_faille(rng):
    # variantes de la faille connue : base g b a a b a b b b g ; nôtre = décalage ; leur = une ligne remplacée par Z
    base = ["g"] + [rng.choice("ab") for _ in range(rng.randint(6, 12))] + ["g"]
    o = list(base)
    for _ in range(rng.randint(1, 3)):
        p = rng.randint(1, len(o) - 1)
        if rng.random() < 0.5:
            o.insert(p, rng.choice("ab"))
        elif len(o) > 4:
            del o[p]
    t = list(base)
    t[rng.randint(1, len(t) - 2)] = "Z"
    if rng.random() < 0.3:
        t[rng.randint(1, len(t) - 2)] = "Y"
    return base, o, t


def fam_decalage(rng):
    # famille exacte de la faille : base cadrée par g, lettres a/b ; nôtre = une suppression + une insertion (décalage) ; leur = un remplacement par Z
    base = ["g"] + [rng.choice("ab") for _ in range(rng.randint(6, 10))] + ["g"]
    o = list(base)
    for _ in range(rng.randint(0, 2)):
        del o[rng.randint(1, len(o) - 2)]
    for _ in range(rng.randint(1, 3)):
        o.insert(rng.randint(1, len(o) - 1), rng.choice("ab"))
    t = list(base)
    t[rng.randint(1, len(t) - 2)] = "Z"
    return base, o, t


FAM = dict(decalage=fam_decalage, aleatoire=fam_aleatoire, repetees=fam_repetees, series=fam_series, code=fam_code, faille=fam_faille)


def put(root, name, b, o, t):
    d = os.path.join(root, name)
    os.makedirs(d, exist_ok=True)
    for k, v in (("base", b), ("ours", o), ("theirs", t)):
        open(os.path.join(d, k), "wb").write(v)


def git_merge(d):
    r = subprocess.run(["git", "merge-file", "-p", "ours", "base", "theirs"], cwd=d, capture_output=True,
                       env=dict(os.environ, GIT_CONFIG_NOSYSTEM="1"))
    # copie des fichiers : git merge-file -p lit ours/base/theirs dans cwd
    return os.path.basename(d), r.returncode, r.stdout


def main():
    root, cli, fam, n = sys.argv[1], sys.argv[2], sys.argv[3], int(sys.argv[4])
    seed = int(sys.argv[5]) if len(sys.argv) > 5 else 1
    rng = random.Random(seed)
    shutil.rmtree(root, ignore_errors=True)
    os.makedirs(root)
    names = list(FAM) if fam == "toutes" else [fam]
    i = 0
    seen = set()
    while i < n:
        f = names[i % len(names)]
        base, o, t = FAM[f](rng)
        eol = "\r\n" if rng.random() < 0.1 else "\n"
        final = rng.random() < 0.9
        trip = (lines_join(base, eol, final), lines_join(o, eol, final), lines_join(t, eol, final))
        if trip in seen or trip[0] == trip[1] or trip[0] == trip[2]:
            continue
        seen.add(trip)
        put(root, "%s-%06d" % (f, i), *trip)
        i += 1
    out = subprocess.run([cli, root], capture_output=True).stdout.decode()
    ours = {}
    for line in out.splitlines():
        k, v = line.split("\t")
        ours[k] = v
    dirs = [os.path.join(root, k) for k in sorted(ours)]
    with Pool(4) as p:
        res = p.map(git_merge, dirs, chunksize=50)
    stats = dict(total=0, both_clean_same=0, both_clean_diff=0, false_conflict=0, silent=0, both_conflict=0, skip=0)
    bad = []
    perfam = {}
    for name, grc, gtxt in res:
        v = ours[name]
        if v == "SKIP":
            stats["skip"] += 1
            continue
        stats["total"] += 1
        oc = int(v)
        fam_ = name.split("-")[0]
        pf = perfam.setdefault(fam_, dict(total=0, silent=0, false_conflict=0, diff=0))
        pf["total"] += 1
        if grc == 0 and oc == 0:
            if open(os.path.join(root, name, "out"), "rb").read() == gtxt:
                stats["both_clean_same"] += 1
            else:
                stats["both_clean_diff"] += 1
                pf["diff"] += 1
                bad.append(("DIFF-TEXTE", name))
        elif grc == 0:
            stats["false_conflict"] += 1
            pf["false_conflict"] += 1
            bad.append(("CONFLIT-EN-TROP", name))
        elif oc == 0:
            stats["silent"] += 1
            pf["silent"] += 1
            bad.append(("NOUS-PROPRE/GIT-CONFLIT", name))
        else:
            stats["both_conflict"] += 1
    print(stats)
    for k, v in sorted(perfam.items()):
        print(k, v)
    open(os.path.join(root, "_ecarts.txt"), "w").write("\n".join("%s\t%s" % x for x in bad))


if __name__ == "__main__":
    main()
