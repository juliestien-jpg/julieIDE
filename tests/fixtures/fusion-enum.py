#!/usr/bin/env python3
"""Énumération exhaustive autour de la faille de fusion (recette v1).

Étend `fusion-attaque.py` : bases de TROIS lettres (a, b, g) au lieu de deux, énumération de toutes les
paires (opération chez nous, opération chez eux), et fichiers plus longs. Chaque triplet base / nôtre / leur
est passé à `mergecli` (julie_diff::merge) et à `git merge-file -p`.

Classes : nous propre / Git en conflit (INTERDIT, doit valoir 0), Git propre / nous en conflit (conflit en
trop, côté sûr), deux propres au texte différent (à expliquer), deux en conflit.

Usage : python fusion-enum.py <dossier> <mergecli.exe> <famille> <n> [graine]
Familles :
  enum3    bases de 3 lettres de 8 lignes ; n bases ; toutes les paires (1 opération chez nous) x (1 chez eux)
  enum3b   bases de 3 lettres cadrées par g ; n bases ; nôtre = 1 suppression + 1 insertion (décalage), leur = 1 remplacement
  enum3c   bases de 3 lettres de 8 lignes ; n bases ; nôtre = 2 opérations (toutes), leur = 1 opération (toutes), échantillonné
  long     n triplets aléatoires de 20 à 40 lignes (alphabet de 2 ou 3 lettres), 2 à 5 opérations de chaque côté
"""
import os, random, shutil, subprocess, sys
from multiprocessing import Pool

LET = "abg"


def ops(n, alphabet):
    """Toutes les opérations simples sur un tableau de n lignes."""
    out = []
    for p in range(n):
        out.append(("repZ", p, "Z"))
        out.append(("del", p, None))
    for p in range(n + 1):
        for c in alphabet:
            out.append(("ins", p, c))
    return out


def apply(ls, op):
    ls = list(ls)
    k, p, c = op
    if k == "repZ":
        ls[p] = c
    elif k == "del":
        del ls[p]
    else:
        ls.insert(p, c)
    return ls


def join(ls, eol="\n"):
    return (eol.join(ls) + eol).encode()


def triplets(fam, n, rng):
    if fam == "enum3":
        for _ in range(n):
            base = [rng.choice(LET) for _ in range(8)]
            o_ops = ops(len(base), LET)
            for oo in o_ops:
                for to in o_ops:
                    yield base, apply(base, oo), apply(base, to)
    elif fam == "enum3b":
        for _ in range(n):
            base = ["g"] + [rng.choice(LET) for _ in range(rng.randint(6, 9))] + ["g"]
            N = len(base)
            for dp in range(1, N - 1):
                after = apply(base, ("del", dp, None))
                for ip in range(1, len(after)):
                    for c in LET:
                        o = apply(after, ("ins", ip, c))
                        for tp in range(1, N - 1):
                            yield base, o, apply(base, ("repZ", tp, "Z"))
    elif fam == "enum3c":
        for _ in range(n):
            base = [rng.choice(LET) for _ in range(8)]
            all_ops = ops(len(base), LET)
            for _ in range(900):
                o = apply(apply(base, rng.choice(all_ops)), ("ins", 0, "x")) if False else base
                o = base
                for _k in range(2):
                    o_ops = ops(len(o), LET)
                    o = apply(o, rng.choice(o_ops))
                t = apply(base, rng.choice(all_ops))
                yield base, o, t
    elif fam == "long":
        for _ in range(n):
            al = list(LET[: rng.randint(2, 3)])
            base = [rng.choice(al) for _ in range(rng.randint(20, 40))]
            def mut():
                l = list(base)
                for _k in range(rng.randint(2, 5)):
                    l = apply(l, rng.choice(ops(len(l), al)))
                return l
            yield base, mut(), mut()


def git_merge(d):
    r = subprocess.run(["git", "merge-file", "-p", "ours", "base", "theirs"], cwd=d, capture_output=True,
                       env=dict(os.environ, GIT_CONFIG_NOSYSTEM="1"))
    return os.path.basename(d), r.returncode, r.stdout


def main():
    root, cli, fam, n = sys.argv[1], sys.argv[2], sys.argv[3], int(sys.argv[4])
    seed = int(sys.argv[5]) if len(sys.argv) > 5 else 1
    rng = random.Random(seed)
    shutil.rmtree(root, ignore_errors=True)
    os.makedirs(root)
    seen = set()
    i = 0
    for b, o, t in triplets(fam, n, rng):
        if b == o or b == t:
            continue
        key = (tuple(b), tuple(o), tuple(t))
        if key in seen:
            continue
        seen.add(key)
        d = os.path.join(root, "%s-%06d" % (fam, i))
        os.makedirs(d)
        for k, v in (("base", b), ("ours", o), ("theirs", t)):
            open(os.path.join(d, k), "wb").write(join(v))
        i += 1
    print("triplets écrits :", i)
    out = subprocess.run([cli, root], capture_output=True).stdout.decode()
    ours = dict(line.split("\t") for line in out.splitlines())
    dirs = [os.path.join(root, k) for k in sorted(ours)]
    with Pool(4) as p:
        res = p.map(git_merge, dirs, chunksize=100)
    st = dict(total=0, both_clean_same=0, both_clean_diff=0, false_conflict=0, silent=0, both_conflict=0, skip=0)
    bad = []
    for name, grc, gtxt in res:
        v = ours[name]
        if v == "SKIP":
            st["skip"] += 1
            continue
        st["total"] += 1
        oc = int(v)
        if grc == 0 and oc == 0:
            if open(os.path.join(root, name, "out"), "rb").read() == gtxt:
                st["both_clean_same"] += 1
            else:
                st["both_clean_diff"] += 1
                bad.append(("DIFF-TEXTE", name))
        elif grc == 0:
            st["false_conflict"] += 1
            bad.append(("CONFLIT-EN-TROP", name))
        elif oc == 0:
            st["silent"] += 1
            bad.append(("NOUS-PROPRE/GIT-CONFLIT", name))
        else:
            st["both_conflict"] += 1
    print(fam, st)
    open(os.path.join(root, "_ecarts.txt"), "w").write("\n".join("%s\t%s" % x for x in bad))


if __name__ == "__main__":
    main()
