#!/usr/bin/env python
"""Recette v1, navigation : génère un scénario de Ctrl+clic (un cas par état JSON) pour la copie
de tests/fixtures/navigation, puis vérifie les états contre navigation.md (critères 1 à 11, 18, 19).
Usage : recette-v1-nav.py gen <dossier-copie> <sortie.txt> <dossier-états>
        recette-v1-nav.py check <dossier-états>"""
import json, os, sys

# (id, fichier, motif de la ligne, mot à cliquer, attendu)
# attendu : ('jump', fichier, ligne|None) | ('list', [noms]) | ('none',)
CASES = [
 ('c01', 'src/Service/UserService.php', '$user->getName()', 'getName', ('jump', 'src/Model/User.php', 17)),
 ('c02a', 'src/Service/UserService.php', '$this->current->getName()', 'getName', ('jump', 'src/Model/User.php', 17)),
 ('c02b', 'src/Service/UserService.php', '$fresh->getName()', 'getName', ('jump', 'src/Model/User.php', 17)),
 ('c02c', 'src/Service/UserService.php', '$boss->getName()', 'getName', ('jump', 'src/Model/Admin.php', 6)),
 ('c03', 'src/Service/UserService.php', '$other->getName()', 'getName', ('list', ['App\\Legacy\\Order::getName', 'App\\Model\\Admin::getName', 'App\\Model\\User::getName'])),
 ('c04a', 'src/Service/UserService.php', '$user->save()', 'save', ('jump', 'src/Model/Entity.php', None)),
 ('c04b', 'src/Service/UserService.php', '$user->touch()', 'touch', ('jump', 'src/Model/Timestamps.php', None)),
 ('c04c', 'src/Model/User.php', "'id' => $this->getId()", 'getId', ('jump', 'src/Model/Entity.php', None)),
 ('c04d', 'src/Model/Admin.php', 'parent::getName()', 'getName', ('jump', 'src/Model/User.php', 17)),
 ('c05a', 'src/Service/UserService.php', 'use App\\Model\\User;', 'User', ('jump', 'src/Model/User.php', 4)),
 ('c05b', 'src/Service/UserService.php', "new Boss('b')", 'Boss', ('jump', 'src/Model/Admin.php', 4)),
 ('c05c', 'src/Model/User.php', 'new static($name)', 'static', ('jump', 'src/Model/User.php', 4)),
 ('c06a', 'src/Service/UserService.php', "User::create('root')", 'create', ('jump', 'src/Model/User.php', 22)),
 ('c06b', 'src/Service/UserService.php', 'echo User::ROLE_ADMIN', 'ROLE_ADMIN', ('jump', 'src/Model/User.php', 8)),
 ('c06c', 'src/Service/UserService.php', 'self::check()', 'check', ('jump', 'src/Service/UserService.php', 40)),
 ('c06d', 'src/Service/UserService.php', 'static::check()', 'check', ('jump', 'src/Service/UserService.php', 40)),
 ('c07a', 'src/Service/UserService.php', 'return format_name(', 'format_name', ('jump', 'src/functions.php', 6)),
 ('c07b', 'src/Service/UserService.php', '$this->logger->log(VERSION)', 'VERSION', ('jump', 'src/functions.php', 4)),
 ('c07c', 'src/Service/UserService.php', 'legacy_helper();', 'legacy_helper', ('jump', 'src/global.php', None)),
 ('c08', 'src/Service/UserService.php', '->log(VERSION)', 'log', ('jump', 'vendor/acme/logger/src/Logger.php', None)),
 ('c09', 'src/Service/UserService.php', 'strlen($label)', '$label', ('jump', 'src/Service/UserService.php', 22)),
 ('c10', 'src/Service/UserService.php', 'strlen($label)', 'strlen', ('none',)),
 ('c11a', 'src/Service/UserService.php', '// getName dans un commentaire', 'getName', ('none',)),
 ('c11b', 'src/Service/UserService.php', "$label = 'getName';", 'getName', ('none',)),
 ('c18a', 'web/js/app.js', 'fd(new Date());', 'fd', ('jump', 'web/js/util.js', 1)),
 ('c18b', 'web/js/app.js', 'formatDate(new Date());', 'formatDate', ('jump', 'web/js/util.js', 1)),
 ('c18c', 'web/js/app.js', 'new Api()', 'Api', ('jump', 'web/js/util.js', 3)),
 ('c18d', 'web/js/app.js', "api.get('/x')", 'get', ('jump', 'web/js/util.js', 3)),
 ('c18e', 'web/js/app.js', 'local();', 'local', ('jump', 'web/js/app.js', 8)),
 ('c18f', 'web/js/types.ts', 'formatDate(new Date())', 'formatDate', ('jump', 'web/js/util.js', 1)),
 ('c19a', 'web/js/app.js', "from './util'", './util', ('jump', 'web/js/util.js', 1)),
 ('c19b', 'web/js/util.js', 'return fetch(url)', 'fetch', ('none',)),
]


def locate(root, f, pat, word):
    lines = open(os.path.join(root, f), encoding='utf-8').read().split('\n')
    for i, l in enumerate(lines):
        p = l.find(pat)
        if p >= 0:
            w = l.find(word, p)
            assert w >= 0, (f, pat, word)
            return i + 1, w + 1 + min(1, len(word) - 1)  # 2e caractère du mot
    raise SystemExit('motif introuvable : %s %s' % (f, pat))


def gen(root, out, states):
    s = ['# Généré par tests/fixtures/recette-v1-nav.py', 'project ' + root]
    for cid, f, pat, word, exp in CASES:
        L, C = locate(root, f, pat, word)
        s += ['file %s:%d:%d' % (f, L, C), 'ui nav.ctrl-click %d:%d' % (L, C), 'wait-idle', 'wait 300',
              'state %s/%s.json' % (states, cid)]
        if exp[0] == 'list':
            s += ['key Escape']
    s.append('quit')
    open(out, 'w', encoding='utf-8', newline='\n').write('\n'.join(s) + '\n')


def check(states):
    ok = bad = 0
    for cid, f, pat, word, exp in CASES:
        p = os.path.join(states, cid + '.json')
        if not os.path.exists(p):
            print('MANQUE', cid)
            bad += 1
            continue
        st = json.load(open(p, encoding='utf-8'))
        act = [t for t in st['tabs'] if t.get('active')]
        act = act[0] if act else None
        pop = (st.get('nav') or {}).get('popup')
        msg = (st.get('status') or {}).get('message')
        apath = (act or {}).get('path', '').replace('\\', '/')
        if exp[0] == 'jump':
            good = act and apath.endswith(exp[1]) and (exp[2] is None or act['line'] == exp[2]) and not pop
        elif exp[0] == 'list':
            names = [r.split(' | ')[1] if ' | ' in r else r for r in (pop or {}).get('rows', [])]
            good = bool(pop) and names == exp[1]
        else:
            good = (not pop) and apath.endswith(f)
        if good:
            ok += 1
        else:
            bad += 1
        print('OK ' if good else 'FAUX', cid, '|', f, '|', word, '| attendu', exp, '| obtenu', apath[-40:],
              (act or {}).get('line'), (act or {}).get('column'), '| popup', pop and pop.get('rows'), '| msg', msg)
    print('TOTAL ok', ok, 'faux', bad)


if __name__ == '__main__':
    if sys.argv[1] == 'gen':
        gen(sys.argv[2], sys.argv[3], sys.argv[4])
    else:
        check(sys.argv[2])
