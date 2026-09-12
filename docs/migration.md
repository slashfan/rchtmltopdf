# Migrer depuis wkhtmltopdf

> Ce guide est un point de départ. Il sera complété au fil des migrations réelles.

La promesse du projet est la **compatibilité fonctionnelle de la CLI**, pas la reproduction
pixel-perfect du rendu. Vos documents vont changer d'apparence. Ce guide explique pourquoi
et quoi faire.

## Le point numéro un : le « smart shrinking »

wkhtmltopdf redimensionne le contenu pour l'adapter à la page. Chromium ne le fait pas : il
rend toujours à **96 px CSS par pouce**. Un document migré sans modification peut donc
apparaître nettement plus grand ou plus petit.

**Options à retirer.** Elles sont acceptées avec un warning et ignorées ; laissées en place,
elles n'aident pas et brouillent le diagnostic.

```
--dpi 96
--image-dpi ...
--disable-smart-shrinking
--enable-smart-shrinking
```

**Option à recalibrer.** `--zoom` est honoré et traduit en facteur d'échelle à l'impression.
Mesuré plutôt qu'annoncé : un bloc de 50 mm sort à 50 mm sans option, à 65 mm avec
`--zoom 1.3`, à 25 mm avec `--zoom 0.5`. Le facteur est exactement celui demandé.

**Le zoom agrandit le contenu, pas le papier.** Le format de page reste celui de
`--page-size`. En pratique cela veut dire que ce qui est dimensionné en `mm`, `cm`, `pt` ou
`in` grandit, et que ce qui est dimensionné en pourcentage garde sa largeur : Chromium
compose la page à une largeur CSS divisée par le zoom, puis met l'ensemble à l'échelle.

**Procédure.** Repartez de `--zoom 1`, sans exception : une valeur héritée d'un calage
wkhtmltopdf compense un comportement qui n'existe plus, et l'empiler sur le nouveau rendu
donne un résultat deux fois décalé. Comparez ensuite une page de référence, et n'ajustez que
si l'écart gêne.

## Le point numéro deux : viewport contre largeur d'impression

`--viewport-size` agit sur ce que lisent vos scripts (`window.innerWidth`), et **pas** sur
la largeur de mise en page imprimée. Chromium compose la page à la largeur du contenu : en
A4 avec 10 mm de marges, cela fait 190 mm, soit environ 718 px CSS.

**Ni sur les media queries, à l'impression.** Mesuré plutôt que déduit : la même fixture
avec `@media (min-width: 1200px)` produit le même document à 1024 et à 1280 de viewport,
parce qu'une page imprimée est composée à la largeur du papier et que la requête est
évaluée contre celle-là. Une mise en page qui dépend d'une media query se reflow donc à
l'impression quel que soit le viewport. C'est la deuxième surprise la plus fréquente après
le smart shrinking.

Le viewport par défaut est celui de wkhtmltopdf, **1024 × 768**, appliqué à chaque
conversion : celui de Chromium n'est pas le même, et une maquette migrée a été écrite
contre le premier (D03).

## En-têtes et pieds de page : la marge doit les contenir

Un bandeau est ancré au **bord du papier** et grandit vers le contenu ; sa hauteur est celle
de son contenu, et `--margin-top` ne décide pas où il commence. Trois conséquences, toutes
mesurées :

* **Ajouter un bandeau ne déplace jamais le contenu.** Le document imprimé avec un en-tête
  occupe exactement la même zone que sans. Ajouter `--header-center` à une ligne de commande
  migrée ne peut pas la repaginer en silence.
* **C'est à la marge de faire la place.** Un bandeau de 12 pt mesure environ 28,5 pt de haut
  et une marge de 10 mm en fait 28,3 : le défaut tient tout juste. À `--header-font-size 40`,
  le bandeau mord sur le contenu, et la réponse est un `--margin-top` plus grand.
* **`--header-spacing` agit sur la marge, pas sur le bandeau.** C'est la seule chose qui
  puisse ouvrir un espace entre les deux : `--header-spacing 5` descend le contenu de 5 mm et
  laisse le bandeau où il est.

### Placeholders

`[page]`, `[topage]`, `[frompage]`, `[sitepage]`, `[sitepages]`, `[webpage]`, `[title]`,
`[doctitle]`, `[date]`, `[isodate]` et `[time]` sont substitués, ainsi que ceux définis par
`--replace`. `[section]`, `[subsection]` et `[subsubsection]` désignent une position dans le
plan du document, qui n'existe pas encore : ils s'effacent, et l'option est signalée une fois
sur stderr.

**`[date]` n'est pas identique à celui de wkhtmltopdf.** Qt le rend via la locale du système,
donc le même binaire écrit une chaîne différente sur deux machines et il n'y a pas de format
à reproduire. Nous écrivons la date locale au format `AAAA-MM-JJ`, non ambigu ; `[isodate]`
ajoute l'heure et le décalage UTC.

## Cookies, en-têtes, identifiants et proxy

**`--custom-header` ne se propage pas aux sous-ressources sans `--custom-header-propagation`.**
C'est le comportement de wkhtmltopdf, et c'est celui qui surprend : sans l'option, l'en-tête
part avec la requête du document et avec aucune autre. Si votre feuille de style ou vos
images sont derrière la même authentification que la page, il faut l'option.

**`--cookie` demande un document en http ou https.** Un document local n'a pas d'origine à
laquelle rattacher un cookie ; l'option est alors signalée et ignorée plutôt que fatale. La
valeur est décodée avant d'être posée, comme l'aide de wkhtmltopdf l'annonce.

**`--username` / `--password` répondent à un 401**, ils ne sont pas envoyés d'avance. Un
mauvais mot de passe fait échouer la conversion au lieu d'imprimer la page d'erreur du
serveur.

**`--proxy` est un drapeau de lancement**, donc il vaut pour tout le processus alors que la
table le range parmi les options d'objet. Avec un seul document c'est la même chose ; à
partir de V2 deux objets demandant des proxys différents ne pourront pas être satisfaits tous
les deux. Attention aussi : **Chromium contourne le proxy pour localhost**, quoi que dise
`--proxy-server`.

## `--encoding` et les documents distants

`--encoding` dit dans quel jeu de caractères lire un document qui ne le déclare pas. Il
s'applique aux fichiers locaux et à l'entrée standard, que nous servons nous-mêmes au
navigateur avec le bon `Content-Type`. Un document récupéré en http ou https est lu comme
son serveur l'a annoncé : il n'y a aucun endroit où intervenir, et l'option est alors
signalée comme sans effet plutôt qu'acceptée en silence.

## Média par défaut

wkhtmltopdf rend avec les CSS `screen` sauf si `--print-media-type` est passé. Ce
comportement est reproduit à l'identique : `screen` par défaut, `print` avec l'option. Si
vos styles d'impression n'étaient jamais appliqués auparavant, ils ne le seront pas
davantage ici.

## Différences attendues, et qui ne sont pas des bugs

* pagination et nombre de pages ;
* métriques de fontes et césure des lignes ;
* dimensions calculées par certaines règles CSS ;
* comportement de vieux hacks CSS visant WebKit ;
* taille du fichier quand plusieurs documents sont assemblés : chaque document embarque son
  propre sous-ensemble de chaque police, là où wkhtmltopdf partageait les siennes (voir
  [D34](decisions.md)).

Si une différence vous bloque, ouvrez un rapport de compatibilité. Le formulaire demande la
sortie de `--dump-parse`, qui résout la plupart des cas immédiatement.

## Checklist

1. Retirer `--dpi`, `--image-dpi`, `--disable-smart-shrinking`, `--enable-smart-shrinking`.
2. Remettre `--zoom` à 1 avant toute comparaison.
3. Vérifier que les media queries et les scripts ne dépendent pas d'une largeur de fenêtre
   pour la mise en page : à l'impression, c'est la largeur du papier qui décide.
4. Ajouter `--enable-local-file-access`, ou `--allow <dossier>`, si le document lit des
   fichiers locaux — ce n'est plus permis par défaut, et c'est délibéré (D10).
5. Lancer une conversion avec `--dump-parse` pour vérifier que la ligne de commande est lue
   comme prévu.
6. Comparer un document de référence, pas une capture d'écran.

## Options sans équivalent

`rchtmltopdf --extended-help` marque chaque option. Celles notées « accepted, ignored »
n'ont pas d'équivalent Chromium et ne sont pas destinées à en avoir : `--grayscale`,
`--lowquality`, `--enable-plugins`, les SVG de cases à cocher, entre autres. Elles sont
acceptées pour ne pas casser les scripts existants, et ignorées.
