# Navigation dans le code : aller à la déclaration, trouver les usages

Rédigé le 25/11/2026 par `specialiste-produit`, d'après la demande du même jour
(« accéder aux références d'une méthode ou fonction en faisant Ctrl+clic dessus »).
Repères : **(observé)** / **(supposé)** / **(écart)** : voir `fusion.md`. Rien n'a
été ré-observé dans PhpStorm pour cette spec (le contrôle de l'ordinateur n'y permet
ni Ctrl+clic ni raccourci) : le comportement de référence est **supposé** d'usage.

## À quoi ça sert

D'un Ctrl+clic, aller là où une fonction, une méthode ou une classe est déclarée,
et, depuis sa déclaration, voir où elle est utilisée — en sachant quand l'IDE est
sûr de lui et quand il devine.

## Les gestes

- **Ctrl+clic sur un usage** (appel, `new X`, type, `use`…) → **aller à la
  déclaration**. Une seule cible : l'éditeur l'ouvre (ou active son onglet),
  curseur sur le nom déclaré, ligne centrée. Plusieurs cibles : une **liste
  surgissante** sous le mot (voir plus bas). Aucune : rien ne bouge (le Ctrl+clic
  ne place pas non plus le curseur), message dans la barre d'état « Aucune
  déclaration trouvée pour « strlen » » **(supposé comme PhpStorm)**.
- **Ctrl+clic sur la déclaration elle-même** (le nom dans `function getName`,
  `class User`…) → **liste surgissante des usages**. Un seul usage : on y va
  directement ; aucun : « Aucun usage trouvé » **(supposé comme PhpStorm)**.
- **Ctrl+B** : la même chose que Ctrl+clic, au curseur.
- **Alt+F7** : **trouver les usages** du symbole sous le curseur, résultat dans la
  **fenêtre d'outils Usages** (en bas).
- **Survol avec Ctrl enfoncé** : si le mot sous le pointeur a au moins une cible,
  il est souligné et le pointeur devient une main ; après 500 ms immobile, une
  infobulle montre la ligne de déclaration et son conteneur (« public function
  getName(): string — App\Model\User »). Sans cible, ni soulignement ni main. Le
  soulignement suit le pointeur et disparaît au relâchement de Ctrl. Il n'apparaît
  que si la réponse arrive en moins de 100 ms ; sinon rien (pas d'indicateur
  d'attente sous le pointeur).
- **Retour / avance** : Ctrl+Alt+Gauche / Ctrl+Alt+Droite, et les boutons
  « précédent / suivant » de la souris, reviennent aux positions d'avant un saut
  (50 positions, la session seulement). **(supposé)** : certains pilotes graphiques
  Intel prennent Ctrl+Alt+flèches pour faire pivoter l'écran ; à vérifier sur cette
  machine, les boutons de souris restent alors le recours.
- Menu contextuel de l'éditeur : « Aller à la déclaration », « Trouver les usages »
  (voir `menus-contextuels.md`).

## La liste surgissante

- Sous le mot, au plus 15 lignes visibles, défilante. Une ligne par cible : genre
  (classe, interface, trait, fonction, méthode, propriété, constante, variable,
  import), nom qualifié (`App\Model\User::getName`), chemin relatif et ligne en
  gris, la ligne de code de la déclaration ou de l'usage. Les bibliothèques
  (`vendor`, `node_modules`) sont marquées.
- **Candidats devinés** : quand la cible a été trouvée par le seul nom (type
  inconnu), la liste a un titre qui le dit : « Type de $other inconnu : méthodes
  nommées getName » ; jamais présentée comme sûre.
- Liste d'usages : titre « 5 usages de User::getName, 1 appel possible » ; les
  usages sûrs d'abord, puis, après un séparateur, les appels possibles. Un lien
  « Ouvrir dans la fenêtre Usages » (ou Alt+F7) transfère la liste.
- Haut/Bas, Entrée ouvre, Échap ferme, clic ouvre. Taper du texte filtre la liste
  par nom de fichier ou de conteneur.
- Ordre : fichier courant, puis même espace de noms (ou même dossier en JS), puis
  le projet, puis les bibliothèques ; à égalité, ordre alphabétique du nom qualifié.

## La fenêtre Usages

- Fenêtre d'outils du bas, un onglet par recherche (10 au plus, le plus ancien est
  remplacé), titre = symbole.
- En tête : « App\Model\User::getName — 5 usages sûrs dans 3 fichiers, 1 appel
  possible », « recherche en cours » tant que ce n'est pas fini, et une case
  « Inclure les bibliothèques » (décochée par défaut).
- Arbre : **Usages** puis **Appels possibles (type inconnu)** → fichier (nombre) →
  ligne (numéro, texte avec l'occurrence surlignée). Une ligne d'aide permanente en
  pied pour les membres (méthodes, propriétés, constantes de classe) : « Les appels
  sur des valeurs de type inconnu sont listés à part. Les appels dynamiques
  (`$obj->$nom()`, `call_user_func`, noms dans des chaînes) ne sont pas cherchés. »
- Entrée, F4 ou double-clic : ouvre à l'occurrence, sélectionnée. Ctrl+Alt+Bas /
  Ctrl+Alt+Haut : occurrence suivante / précédente, même quand l'éditeur a le focus
  **(supposé comme PhpStorm)**. Bouton Relancer (refait la recherche), bouton
  Fermer l'onglet.
- Les résultats arrivent en flux ; la recherche s'annule si on ferme l'onglet.
- **Honnêteté** : le mot « tous » n'apparaît jamais. Le compte des appels possibles
  est toujours affiché, même à 0 (« 0 appel possible »). Pendant l'indexation,
  bandeau « Indexation en cours (45 %) : résultats incomplets ».
- Ce qui est cherché : le code. Les commentaires et les chaînes de caractères ne
  sont pas des usages (sauf les types des commentaires de documentation PHP,
  `@param User`, `@var`, `@return`, `@throws`, qui en sont).

## Ce que l'IDE sait résoudre : PHP

**Déclarations indexées** : espaces de noms, classes, interfaces, traits, énumérations,
fonctions, méthodes, propriétés (y compris celles promues par le constructeur),
constantes de classe, constantes globales (`const X =` et `define('X', …)` avec un
nom littéral), cas d'énumération ; et, dans une fonction, ses paramètres et
variables locales.

**Noms de classes** (règles de PHP) :
- nom complet `\App\Model\User` ; nom importé par `use` (avec alias `as`, `use`
  groupé `use App\Model\{User, Admin}`) ; nom relatif `Model\User` (premier segment
  importé, sinon espace de noms courant) ; nom simple non importé → espace de noms
  courant.
- `self`, `static` → classe englobante ; `parent` → classe étendue par la classe
  englobante ; `X::class` → X.
- Dans `new X(…)`, Ctrl+clic sur `X` → la **classe** (**supposé** : PhpStorm
  pourrait viser le constructeur ; à trancher si gênant).

**Fonctions et constantes globales** : nom importé par `use function` / `use const`,
sinon espace de noms courant **puis** espace global (règle de repli de PHP). Une
fonction de PHP lui-même (`strlen`, `array_map`) n'a pas de déclaration dans le
projet : « Aucune déclaration dans le projet (fonction de PHP ?) ». **(écart)** :
PhpStorm a des fichiers de déclarations des fonctions de PHP ; on n'en a pas (ni de
copie des siens).

**Membres** (`->m()`, `->p`, `::m()`, `::$p`, `::C`) : le type du receveur est
cherché sans analyse complète, par ces seules règles :
- `$this` (y compris dans une fonction anonyme écrite dans une classe) → classe
  englobante ; `self::`, `static::`, `parent::`, `X::` → comme plus haut ;
- paramètre typé (`User $user`, `?User`, `User|Admin` → les deux) ; propriété typée
  (`private User $current` pour `$this->current->…`) ; propriété promue ;
- variable affectée par `new X(…)` dans la même fonction, si **toutes** ses
  affectations dans la fonction donnent le même type ;
- annotation `@var X $v`, `@param X $v`, `@var X` sur une propriété ;
- type de retour déclaré d'une méthode ou d'une fonction (`: X`, `: self`,
  `: static`, `?X`), sur une chaîne de trois appels au plus
  (`$this->repo()->find()->getName()`).

Le membre est ensuite cherché dans la classe, puis ses traits, puis ses parents
(chaîne `extends`), puis ses interfaces ; la cible est la déclaration **la plus
proche** (une redéfinition dans une sous-classe l'emporte sur la méthode parente).

**Type inconnu** (toute autre variable, retour non typé, tableau, `$obj->$nom`…) :
les candidats sont **toutes les déclarations du même nom et du même genre** (méthode,
propriété) dans le projet et les bibliothèques, présentés comme devinés. Aucun
candidat : « Aucune déclaration trouvée ». Nom dynamique (`$obj->$nom()`,
`new $classe`) : rien (« Nom calculé à l'exécution »).

**Usages d'un symbole PHP** :
- Classe, interface, trait : `use`, `extends`, `implements`, `use Trait;`, `new`,
  `X::`, `instanceof`, `catch`, types de paramètres, de retour et de propriétés,
  `X::class`, types des commentaires de documentation. `self`, `static` et `parent`
  ne sont pas comptés. Résolus par les règles de noms : **sûrs**.
- Fonction, constante globale : appels et `use function` / `use const`, résolus par
  les règles de noms (repli global compris) : **sûrs**.
- Méthode, propriété, constante de classe : **sûrs** = accès dont le receveur est
  résolu et dont la cible (règle « la plus proche ») est ce membre ; **appels
  possibles** = accès de même nom dont le receveur est de type inconnu. Un accès
  résolu vers une autre classe sans lien n'apparaît pas.
- Variable locale, paramètre : ses lectures et écritures dans la fonction.

## Ce que l'IDE sait résoudre : JavaScript et TypeScript

`.js .mjs .cjs .jsx .ts .tsx`, et le JavaScript des `<script>` d'un fichier HTML ou
PHP pour les seuls symboles locaux à ce bloc.

- **Déclarations** : fonctions, classes et leurs méthodes, `const`/`let`/`var`
  (y compris celles qui reçoivent une fonction fléchée), paramètres, imports,
  exports ; en TS en plus : interfaces, alias de types, énumérations.
- **Portée** : un identifiant est résolu vers la déclaration de la portée la plus
  proche (bloc pour `let`/`const`/`class`, fonction pour `var` et `function`,
  module).
- **Imports** : `import { a as b } from './x'`, `import d from './x'`,
  `import * as ns from './x'` (`ns.a` → `a` de x). Chemin relatif résolu en
  essayant `x`, `x.js`, `.ts`, `.tsx`, `.jsx`, `.mjs`, `.cjs`, puis `x/index.*`.
  L'export est suivi à travers `export { a } from`, `export * from` (5 niveaux au
  plus). `require('./x')` : vers le fichier. Ctrl+clic sur la chaîne du chemin
  d'un import → ouvre le fichier.
- Module nu (`'react'`, `'angular'`) : « Module externe : non résolu » (hors
  périmètre, comme les alias de `tsconfig.json`).
- Composant JSX `<Bouton />` : résolu comme l'identifiant `Bouton`.
- Membres : `this.m()` dans une classe → méthode de la classe (puis `extends`) ;
  `x.m()` avec `const x = new C()` dans la même portée → `C.m` ; sinon candidats
  par le nom, comme en PHP.
- **Usages** : d'une déclaration de module, les références dans le fichier et dans
  les fichiers qui l'importent (spécificateurs d'import compris) ; des membres : la
  même distinction sûrs / possibles qu'en PHP.

## Index des symboles

- Construit en tâche de fond à l'ouverture du projet, après l'index des fichiers ;
  barre d'état « Indexation des symboles… n % ». Les fichiers exclus
  (`reglages.md`) et ignorés par Git n'y sont pas ; `vendor`, `node_modules` y sont
  s'ils ne sont pas ignorés par Git (bibliothèques : déclarations oui, usages
  seulement si la case est cochée).
- Pendant la construction, Ctrl+clic marche avec ce qui est déjà indexé ; si rien
  n'est trouvé, le message le dit (« Indexation en cours (45 %), réessayez dans un
  instant »).
- Suit le disque : un fichier enregistré, créé ou supprimé (dans l'IDE ou dehors)
  est réindexé en moins de 1 s. Un onglet **modifié non enregistré** est indexé sur
  son contenu en mémoire (comme la recherche dans les fichiers).
- Cache sur disque dans `.juliestorm/index/` : à la réouverture, seuls les fichiers
  changés sont relus. Cache illisible ou d'une autre version : jeté et reconstruit,
  sans message d'erreur.
- Fichier syntaxiquement faux : indexé au mieux (ce que l'analyse récupère), jamais
  d'erreur affichée. Fichier de plus de `max_file_size` (10 Mio), binaire : non
  indexé.

## Cas limites

Deux classes de même nom complet dans deux fichiers (projet copié, ancien code) :
les deux sont proposées, sans en choisir une. Fichier PHP sans espace de noms.
Fichier avec plusieurs `namespace` (blocs ou déclarations successives). Majuscules :
classes et fonctions PHP insensibles à la casse (`new user()` trouve `User`),
méthodes aussi ; variables et constantes sensibles ; JS sensible partout.
`__construct`, `__call`, `__get` : pas de résolution magique (un appel à une méthode
absente d'une classe qui a `__call` est « Aucune déclaration trouvée »). Héritage
cyclique (code faux) : arrêt sans boucle. Trait qui utilise un trait. Méthode
déclarée dans une interface et implémentée : la cible d'un appel sur le type
interface est la déclaration de l'interface. Ctrl+clic dans un commentaire, une
chaîne, un gabarit HTML : rien (sauf les types des commentaires de documentation et
le chemin d'un import). Ctrl+clic sur un mot qui s'étend sur une sélection : la
sélection est gardée. Fichier de 10 000 lignes : survol avec Ctrl sans gel.
Ligne de 100 000 caractères. Projet de 23 000 fichiers.

## Raccourcis

| Raccourci | Action |
|---|---|
| Ctrl+clic, Ctrl+B | Aller à la déclaration (ou liste des usages sur une déclaration) |
| Alt+F7 | Trouver les usages (fenêtre Usages) |
| Ctrl+Alt+Gauche / Droite, boutons précédent / suivant de la souris | Position précédente / suivante |
| Ctrl+Alt+Bas / Haut | Usage suivant / précédent (fenêtre Usages) |

Voir `raccourcis.md`.

## Performances

Mesurées en compilation `--release`, sur cette machine, 4 fils (limite de
`.cargo/config.toml`), sur `lab` **en lecture seule** : par l'exemple de mesure de
`julie-index` avec le cache dirigé vers `%TEMP%`, jamais en ouvrant `lab` dans l'IDE
(qui écrirait `lab\.juliestorm\`).

| Mesure (lab, ~23 000 fichiers) | Cible |
|---|---|
| Index des symboles complet, sans cache | ≤ 30 s, l'interface reste fluide |
| Prêt à la réouverture, avec cache | ≤ 3 s |
| Aller à la déclaration, index prêt (médiane / 95e centile) | ≤ 30 ms / ≤ 100 ms |
| Soulignement au survol avec Ctrl | ≤ 50 ms |
| Usages d'une classe : premier résultat / fin | ≤ 300 ms / ≤ 3 s |
| Usages d'une méthode au nom courant (`getName`, `save`) : premier résultat / fin | ≤ 500 ms / ≤ 5 s |
| Réindexation d'un fichier enregistré | ≤ 1 s |
| Mémoire de l'index | ≤ 300 Mio |

## Projet d'essai `tests/fixtures/navigation/`

Généré par script comme les autres projets d'essai ; le voici en entier, car les
critères en comptent les résultats. Pas de `.gitignore` (donc `vendor` est indexé,
comme bibliothèque). Chaque fichier commence par `<?php` ; on montre l'essentiel.

- `src/Model/Entity.php` — `namespace App\Model;` `abstract class Entity` :
  `protected int $id = 0;`, `public function getId(): int { return $this->id; }`,
  `public function save(): void {}`.
- `src/Model/Arrayable.php` — `namespace App\Model;` `interface Arrayable` :
  `public function toArray(): array;`.
- `src/Model/Timestamps.php` — `namespace App\Model;` `trait Timestamps` :
  `public function touch(): void {}`.
- `src/Model/User.php` — `namespace App\Model;`
  `class User extends Entity implements Arrayable` : `use Timestamps;`,
  `public const ROLE_ADMIN = 'admin';`, `private string $name;`,
  `public function __construct(string $name) { $this->name = $name; }`,
  `public function getName(): string { return $this->name; }`,
  `public static function create(string $name): self { return new static($name); }`,
  `public function toArray(): array { return ['id' => $this->getId(), 'name' => $this->getName()]; }`.
- `src/Model/Admin.php` — `namespace App\Model;` `class Admin extends User` :
  `public function getName(): string { return 'admin:' . parent::getName(); }`.
- `src/Legacy/Order.php` — `namespace App\Legacy;` `class Order` :
  `public function getName(): string { return 'order'; }`.
- `src/functions.php` — `namespace App;` `const VERSION = '1.0';`
  `function format_name(string $name): string { return ucfirst($name); }`.
- `src/global.php` — sans espace de noms : `function legacy_helper(): void {}`.
- `src/Broken.php` — `<?php function broken( {` (syntaxe fausse, sans autre
  symbole).
- `vendor/acme/logger/src/Logger.php` — `namespace Acme;` `class Logger` :
  `public function log(string $message): void {}`.
- `src/Service/UserService.php` :

```php
<?php
namespace App\Service;

use Acme\Logger;
use App\Model\User;
use App\Model\Admin as Boss;
use function App\format_name;
use const App\VERSION;

class UserService
{
    private User $current;

    public function __construct(private Logger $logger)
    {
        $this->current = User::create('root');
    }

    public function rename(User $user, $other): string
    {
        // getName dans un commentaire
        $label = 'getName';
        $user->touch();
        $user->save();
        $fresh = new User('a');
        $boss = new Boss('b');
        $this->logger->log(VERSION);
        legacy_helper();
        strlen($label);
        self::check();
        static::check();
        echo User::ROLE_ADMIN;
        return format_name($user->getName())
            . $this->current->getName()
            . $fresh->getName()
            . $boss->getName()
            . $other->getName();
    }

    private static function check(): void
    {
    }
}
```

- `web/js/util.js` : `export function formatDate(d) { return d.toISOString(); }`
  et `export default class Api { get(url) { return fetch(url); } }`.
- `web/js/index.js` : `export * from './util.js';`
- `web/js/app.js` :

```js
import Api, { formatDate as fd } from './util';
import { formatDate } from './index.js';

const api = new Api();
api.get('/x');
fd(new Date());
formatDate(new Date());
function local() { return 1; }
local();
```

- `web/js/types.ts` : `import { formatDate } from './util';` puis
  `export const today: string = formatDate(new Date());`.

## Critères d'acceptation

Sur une copie du projet d'essai dans `%TEMP%`, index prêt, sauf mention. « Ouvre X »
= onglet de X actif, curseur sur le nom déclaré.

PHP, aller à la déclaration (Ctrl+clic, puis le même avec Ctrl+B au curseur) :

1. `getName` dans `$user->getName()` (UserService) ouvre `User.php`, sur
   `getName`.
2. `$this->current->getName()` → `User::getName` ; `$fresh->getName()` →
   `User::getName` ; `$boss->getName()` → `Admin::getName`.
3. `$other->getName()` → liste de **3** candidats, titre « type inconnu »,
   dans l'ordre `App\Legacy\Order::getName`, `App\Model\Admin::getName`,
   `App\Model\User::getName`.
4. `$user->save()` → `Entity::save` ; `$user->touch()` → `Timestamps::touch` ;
   `$this->getId()` (User) → `Entity::getId` ; `parent::getName()` (Admin) →
   `User::getName`.
5. `User` dans `use App\Model\User;` → classe `User` ; `Boss` dans `new Boss('b')`
   → classe `Admin` ; `static` dans `new static($name)` → classe `User`.
6. `create` dans `User::create('root')` → `User::create` ; `ROLE_ADMIN` →
   la constante ; `check` dans `self::check()` et dans `static::check()` →
   `UserService::check`.
7. `format_name` → `App\format_name` dans `functions.php` ; `VERSION` dans
   `log(VERSION)` → la constante de `functions.php` ; `legacy_helper` →
   `global.php` (repli global).
8. `log` dans `$this->logger->log(…)` → `Logger::log` dans `vendor`, marqué
   bibliothèque.
9. `$label` dans `strlen($label)` → la ligne `$label = 'getName';`.
10. `strlen` : aucun saut, message « Aucune déclaration dans le projet » dans la
    barre d'état, curseur inchangé.
11. Ctrl+clic sur `getName` dans le commentaire et dans la chaîne `'getName'` :
    rien.

PHP, usages :

12. Ctrl+clic sur le nom de la déclaration `getName` de `User` : liste avec **5**
    usages sûrs (User.php `toArray`, Admin.php `parent::getName()`, UserService :
    `$user->`, `$this->current->`, `$fresh->`) puis **1** appel possible
    (`$other->getName()`), titre « 5 usages … 1 appel possible ».
13. Alt+F7 sur la déclaration de `Admin::getName` : fenêtre Usages, 1 usage sûr
    (`$boss->getName()`), 1 appel possible. Sur `Order::getName` : 0 usage sûr,
    1 appel possible. Le mot « tous » n'apparaît nulle part.
14. Alt+F7 sur la classe `User` : **7** usages dans **2** fichiers (Admin.php :
    `extends` ; UserService.php : `use`, type de propriété, `User::create`, type de
    paramètre, `new User`, `User::ROLE_ADMIN`). Ni le commentaire ni la chaîne ne
    sont comptés.
15. Alt+F7 sur `format_name` : 2 usages (le `use function` et l'appel) ; sur
    `UserService::check` : 2 ; sur `Entity::save` : 1 sûr, 0 appel possible.
16. Ctrl+clic sur la déclaration de `Logger::log` (vendor) : un seul usage →
    saut direct à `$this->logger->log(VERSION)`.
17. Ouvrir `UserService.php`, ajouter sans enregistrer une ligne
    `$this->current->getName();` dans `rename` : les usages de `User::getName`
    passent à 6 sûrs.

JavaScript / TypeScript :

18. `fd` dans `fd(new Date())` → `formatDate` de `util.js` ; `formatDate` dans
    `formatDate(new Date())` (app.js, importé via `index.js`) → `util.js` ;
    `Api` dans `new Api()` → la classe de `util.js` ; `get` dans `api.get('/x')`
    → `Api.get` ; `local` dans `local()` → la fonction locale ; `formatDate` dans
    `types.ts` → `util.js`.
19. Ctrl+clic sur `'./util'` (app.js) ouvre `util.js`. `fetch` : « Aucune
    déclaration dans le projet ».
20. Alt+F7 sur la déclaration `formatDate` : **6** usages dans **2** fichiers
    (app.js : 4 — deux spécificateurs d'import, deux appels ; types.ts : 2).

Gestes et robustesse :

21. Ctrl enfoncé, pointeur sur `getName` d'un appel : soulignement ; sur `strlen` :
    aucun. Relâcher Ctrl : plus de soulignement.
22. Après le saut du critère 1, Ctrl+Alt+Gauche (ou bouton « précédent » de la
    souris) ramène dans UserService.php à la position d'avant.
23. Ouvrir `src/Broken.php` et y Ctrl+cliquer partout : aucune panique ; l'index du
    reste du projet est intact (critère 1 toujours vrai).
24. Modifier `Order.php` hors de l'IDE (renommer `getName` en `title`) : en moins
    de 2 s, le critère 3 ne donne plus que 2 candidats.
25. Fermer puis rouvrir le projet : prêt à naviguer en moins de 1 s (cache).

Performances (sur `lab`, conditions de la section Performances) :

26. Chaque cible du tableau est tenue ; le rapport donne les mesures sur 20 essais
    (médiane et maximum), les symboles choisis pour les usages et le nombre de
    fichiers indexés.
27. Pendant l'indexation de `lab`, l'éditeur reste utilisable (frappe sans retard
    visible dans un fichier de 10 000 lignes).

## Hors périmètre

Analyse de types complète (inférence à travers les tableaux, les génériques, les
retours non typés), fichiers de déclarations des fonctions de PHP ou du navigateur,
modules de `node_modules` et alias de `tsconfig`, AngularJS (contrôleurs,
directives, gabarits), CSS et HTML (sélecteurs, classes, identifiants), aller à
l'implémentation (Ctrl+Alt+B), hiérarchie des types, renommage avec mise à jour des
usages, recherche partout dans les symboles (vient avec l'index, spec à part),
usages dans les commentaires et les chaînes, navigation dans le diff et la fusion,
historique de navigation conservé au redémarrage.
