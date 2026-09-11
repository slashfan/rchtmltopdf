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
Si votre document était calé avec `--zoom 1.3` pour compenser le shrinking, repartez de
`--zoom 1` et ajustez.

## Le point numéro deux : viewport contre largeur d'impression

`--viewport-size` agit sur les media queries et sur ce que lisent vos scripts
(`window.innerWidth`), mais **pas** sur la largeur de mise en page imprimée. Chromium
compose la page à la largeur du contenu : en A4 avec 10 mm de marges, cela fait 190 mm,
soit environ 718 px CSS.

Une maquette conçue pour un viewport de 1024 px se reflow donc à l'impression. C'est la
deuxième surprise la plus fréquente après le smart shrinking.

## Média par défaut

wkhtmltopdf rend avec les CSS `screen` sauf si `--print-media-type` est passé. Ce
comportement est reproduit à l'identique : `screen` par défaut, `print` avec l'option. Si
vos styles d'impression n'étaient jamais appliqués auparavant, ils ne le seront pas
davantage ici.

## Différences attendues, et qui ne sont pas des bugs

* pagination et nombre de pages ;
* métriques de fontes et césure des lignes ;
* dimensions calculées par certaines règles CSS ;
* comportement de vieux hacks CSS visant WebKit.

Si une différence vous bloque, ouvrez un rapport de compatibilité. Le formulaire demande la
sortie de `--dump-parse`, qui résout la plupart des cas immédiatement.

## Options sans équivalent

`rchtmltopdf --extended-help` marque chaque option. Celles notées « accepted, ignored »
n'ont pas d'équivalent Chromium et ne sont pas destinées à en avoir : `--grayscale`,
`--lowquality`, `--enable-plugins`, les SVG de cases à cocher, entre autres. Elles sont
acceptées pour ne pas casser les scripts existants, et ignorées.
