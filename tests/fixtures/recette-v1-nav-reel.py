#!/usr/bin/env python
"""Recette v1, navigation sur du vrai code : compare « Trouver les usages » à une recherche de mots (grep -w).
Usage : recette-v1-nav-reel.py gen <copie> <scenario.txt> <dossier-etats> [nombre]
        recette-v1-nav-reel.py check <copie> <dossier-etats>
La copie est un dossier de fichiers PHP (copie de lab, jamais lab lui-même)."""
import json, os, re, sys, random

DECL = re.compile(r'^\s*(?:(?:public|private|protected|static|final|abstract)\s+)*function\s+&?(\w+)\s*\(')


def php_files(root):
    for d, _, fs in os.walk(root):
        if '.juliestorm' in d or '.git' in d:
            continue
        for f in fs:
            if f.endswith('.php'):
                yield os.path.relpath(os.path.join(d, f), root).replace('\\', '/')


def read(root, f):
    return open(os.path.join(root, f), encoding='utf-8', errors='replace').read().split('\n')


def symbols(root):
    decls = {}
    for f in php_files(root):
        for i, l in enumerate(read(root, f)):
            m = DECL.match(l)
            if m:
                decls.setdefault(m.group(1), []).append((f, i + 1, l.find(m.group(1)) + 2))
    return {n: v[0] for n, v in decls.items() if len(v) == 1 and len(n) > 3}


def grep_calls(root, name):
    """Lignes (fichier:ligne) qui contiennent le mot suivi d'une parenthèse, hors lignes de déclaration."""
    pat = re.compile(r'\b' + re.escape(name) + r'\s*\(')
    out = set()
    for f in php_files(root):
        for i, l in enumerate(read(root, f)):
            if pat.search(l) and not DECL.match(l):
                s = l.strip()
                if s.startswith('//') or s.startswith('*') or s.startswith('#'):
                    continue
                out.add('%s:%d' % (f, i + 1))
    return out


def gen(root, out, states, n):
    syms = symbols(root)
    cands = [(k, v) for k, v in sorted(syms.items()) if grep_calls(root, k)]
    random.Random(7).shuffle(cands)
    cands = cands[:n]
    s = ['# Généré par tests/fixtures/recette-v1-nav-reel.py', 'project ' + root]
    meta = []
    for k, (f, l, c) in cands:
        s += ['file %s:%d:%d' % (f, l, c), 'key Alt+F7', 'wait-idle', 'wait 400', 'state %s/%s.json' % (states, k)]
        meta.append(k)
    s.append('quit')
    open(out, 'w', encoding='utf-8', newline='\n').write('\n'.join(s) + '\n')
    json.dump(meta, open(os.path.join(os.path.dirname(out), 'symbols.json'), 'w'))


def check(root, states):
    meta = json.load(open(os.path.join(states, '..', 'symbols.json')))
    miss_total = extra_total = 0
    nowin = []
    for k in meta:
        p = os.path.join(states, k + '.json')
        if not os.path.exists(p):
            print('MANQUE', k)
            continue
        st = json.load(open(p, encoding='utf-8'))
        tabs = (st.get('usages') or {}).get('tabs') or []
        if not tabs:
            print(k, 'aucune fenêtre usages ; actif', [t['path'][-30:] for t in st['tabs'] if t['active']], st['status'].get('message'))
            continue
        t = tabs[-1]
        if k not in t['headline']:
            print('%-32s PAS de fenêtre d usages pour ce symbole (onglet précédent resté) ; message : %s' % (k, st['status'].get('message')))
            nowin.append(k)
            continue
        found = set()
        cur = None
        for r in t['rows']:
            if r.startswith('#'):
                continue
            m = re.match(r'^(\S.*) \((\d+)\)$', r)
            if m and not r.startswith(' '):
                cur = m.group(1).replace('\\', '/')
                continue
            m = re.match(r'^\s+(\d+): ', r)
            if m and cur:
                found.add('%s:%s' % (cur, m.group(1)))
        grep = grep_calls(root, k)
        missed = sorted(grep - found)
        extra = sorted(found - grep)
        miss_total += len(missed)
        extra_total += len(extra)
        print('%-32s outil %2d (sûrs %s, possibles %s) | grep %2d | manqués %d | en plus %d | %s' % (
            k, len(found), t['sure'], t['possible'], len(grep), len(missed), len(extra), t['headline'][:60]))
        for m in missed[:4]:
            print('      manqué :', m, '|', read(root, m.split(':')[0])[int(m.split(':')[1]) - 1].strip()[:100])
        for m in extra[:3]:
            print('      en plus :', m, '|', read(root, m.split(':')[0])[int(m.split(':')[1]) - 1].strip()[:100])
    print('TOTAL manqués', miss_total, 'en plus', extra_total, '| sans fenêtre', len(nowin), nowin)


if __name__ == '__main__' and sys.argv[1] in ('gen', 'check'):
    if sys.argv[1] == 'gen':
        gen(sys.argv[2], sys.argv[3], sys.argv[4], int(sys.argv[5]) if len(sys.argv) > 5 else 25)
    else:
        check(sys.argv[2], sys.argv[3])


# ---- sauts : Ctrl+clic sur un appel de fonction, on attend la déclaration unique (ou rien) ----
def calls_sites(root, name):
    pat = re.compile(r'(?<![\w>:$])' + re.escape(name) + r'\s*\(')
    for f in php_files(root):
        for i, l in enumerate(read(root, f)):
            if DECL.match(l):
                continue
            s = l.strip()
            if s.startswith('//') or s.startswith('*') or s.startswith('#'):
                continue
            m = pat.search(l)
            if m:
                return f, i + 1, m.start() + 2
    return None


def gen_jumps(root, out, states, n):
    syms = symbols(root)
    cands = []
    for k, v in sorted(syms.items()):
        site = calls_sites(root, k)
        if site:
            cands.append((k, v, site))
    random.Random(11).shuffle(cands)
    cands = cands[:n]
    s = ['# sauts : généré par recette-v1-nav-reel.py', 'project ' + root]
    meta = []
    for k, (df, dl, dc), (f, l, c) in cands:
        s += ['file %s:%d:%d' % (f, l, c), 'ui nav.ctrl-click %d:%d' % (l, c), 'wait-idle', 'wait 300',
              'state %s/j-%s.json' % (states, k)]
        meta.append({'k': k, 'decl': [df, dl], 'site': [f, l]})
    s.append('quit')
    open(out, 'w', encoding='utf-8', newline='\n').write('\n'.join(s) + '\n')
    json.dump(meta, open(os.path.join(os.path.dirname(out), 'jumps.json'), 'w'))


def check_jumps(root, states):
    meta = json.load(open(os.path.join(states, '..', 'jumps.json')))
    good = wrong = none = 0
    for m in meta:
        p = os.path.join(states, 'j-%s.json' % m['k'])
        st = json.load(open(p, encoding='utf-8'))
        act = [t for t in st['tabs'] if t['active']][0]
        apath = act['path'].replace(chr(92), '/')
        pop = st['nav']['popup']
        at_decl = apath.endswith(m['decl'][0]) and act['line'] == m['decl'][1]
        at_site = apath.endswith(m['site'][0]) and act['line'] == m['site'][1]
        if pop:
            verdict = 'LISTE'
        elif at_decl:
            verdict = 'OK'
            good += 1
        elif at_site:
            verdict = 'RIEN'
            none += 1
        else:
            verdict = 'FAUX'
            wrong += 1
        print('%-5s %-30s site %s:%s | attendu %s:%s | obtenu %s:%s | %s' % (verdict, m['k'], m['site'][0], m['site'][1],
              m['decl'][0], m['decl'][1], apath[-45:], act['line'], st['status'].get('message') if verdict == 'RIEN' else ''))
    print('SAUTS bons', good, 'faux', wrong, 'sans saut', none)


if __name__ == '__main__' and sys.argv[1] == 'genj':
    gen_jumps(sys.argv[2], sys.argv[3], sys.argv[4], int(sys.argv[5]))
if __name__ == '__main__' and sys.argv[1] == 'checkj':
    check_jumps(sys.argv[2], sys.argv[3])
