# Projets de référence

Les cibles contre lesquelles la compatibilité de ce projet est mesurée. Ce fichier existe
pour qu'une affirmation de compatibilité dise contre quoi, et pour que le jour où l'une des
cibles cesse d'être représentative, on sache laquelle.

Le brief demandait « deux ou trois applications Symfony **open source** utilisant
KnpSnappy ». Ce n'est pas ce qui a été retenu, et [D65](decisions.md) porte le raisonnement.
Deux cibles, choisies pour ce qu'elles apportent chacune et que l'autre n'a pas : une
application de production, qui apporte de vrais documents et de vraies feuilles de style ;
une application écrite pour l'exercice, qui apporte le binaire de référence dans le même
conteneur et un corpus qui couvre l'espace des options délibérément.

## `sf-pdf-sandbox` — le harnais

| | |
| --- | --- |
| Ce que c'est | une application Symfony qui génère ses PDF par `knplabs/knp-snappy-bundle`, deux fois, avec deux binaires qui répondent tous deux au nom `wkhtmltopdf` |
| Où | dépôt voisin de celui-ci, non publié à ce jour |
| Licence | aucune, puisque non publié |
| Référence | `wkhtmltopdf 0.12.6.1 (with patched qt)`, le `.deb` officiel bookworm, dans la même image |
| Pile | knp-snappy 1.7.3, knp-snappy-bundle 1.10.6, Symfony 7.4.18 |

**Les options qu'elle émet.** La configuration globale est volontairement maigre — tout ce
qui y est posé retombe sur les soixante-douze cas et masque la ligne de commande sous test :

```yaml
knp_snappy:
    pdf:
        binary: '%env(SNAPPY_BINARY)%'   # la seule chaîne qui diffère entre les deux runs
        options:
            encoding: UTF-8
            page-size: A4
```

À quoi `knplabs/knp-snappy` ajoute `--lowquality` sur **toutes** les lignes de commande
qu'il construit, parce qu'il en fait un défaut à `true`. Aucune lecture du manuel ne
l'aurait prédit ; c'est la première chose que le corpus a apprise, et elle vaut pour toute
application Snappy.

Par-dessus ces trois-là, les cas exercent 55 options longues distinctes :

```
--allow --disable-dotted-lines --disable-external-links --disable-internal-links
--disable-javascript --disable-toc-links --dump-default-toc-xsl --dump-outline
--enable-forms --enable-local-file-access --enable-toc-back-links
--exclude-from-outline --footer-center --footer-font-size --footer-html --footer-left
--footer-line --footer-right --footer-spacing --grayscale --header-center
--header-font-size --header-html --header-left --header-line --header-right
--header-spacing --javascript-delay --keep-relative-links --load-error-handling
--load-media-error-handling --margin-bottom --margin-left --margin-right --margin-top
--no-background --no-outline --no-print-media-type --orientation --outline-depth
--page-height --page-offset --page-width --print-media-type --quiet --replace
--run-script --title --toc-header-text --toc-level-indentation --toc-text-size-shrink
--user-style-sheet --window-status --xsl-style-sheet --zoom
```

| Groupe | Cas | Ce qu'il tient |
| --- | --- | --- |
| `rendering` | 17 | média, arrière-plans, scripts, accès aux fichiers, feuilles de style |
| `exit` | 11 | les codes de sortie tels que Snappy les lit (D14) |
| `numbering` | 11 | bandeaux, substitutions, numérotation |
| `toc` | 8 | table des matières |
| `outline` | 7 | signets et `--dump-outline` |
| `page` | 7 | format, marges, orientation, zoom |
| `links` | 6 | liens internes, externes, relatifs |
| `multidoc` | 5 | plusieurs documents, couverture |

**La commande qui produit un PDF.** Rien n'est nécessaire sur l'hôte que Docker :

```bash
docker compose run --rm harness                  # les 72 cas
docker compose run --rm harness --filter=toc/    # une tranche
```

**Ce qu'elle a trouvé.** Dix défauts, chacun devenu une issue puis un test : #107 à #115 et
#122. Tous ont été trouvés en comparant au binaire réel, aucun en relisant le manuel.

**Sa limite, et elle est de principe.** Le corpus est écrit par ceux qui écrivent le code :
il mesure ce que quelqu'un a pensé à mesurer. C'est l'autre cible qui corrige ce biais.

## `snapevent` — la production

| | |
| --- | --- |
| Ce que c'est | une application Symfony privée dont les devis, factures, inventaires et feuilles de route passent par knp-snappy en production |
| Où | privée. **Aucune de ses données ne sort d'elle** : ni ici, ni dans le harnais, ni dans un message de commit. Structure seulement — nombres de pages, de liens, noms de polices, codes de sortie |
| Licence | propriétaire |

**Les options qu'elle émet**, depuis un seul service (`PdfService`), sans jamais de
configuration Snappy globale — le bundle ne reçoit qu'un chemin, par le paramètre
`wkhtmltopdf.path` :

| Option | Valeur |
| --- | --- |
| `header-html`, `footer-html` | un gabarit Twig rendu, ou la chaîne vide quand le document n'en a pas |
| `margin-top` | `10mm` sans en-tête, `25mm` avec |
| `margin-left`, `margin-right` | `10mm` |
| `margin-bottom` | `20mm`, `30mm` pour une facture |
| `header-spacing` | `5`, sur le chemin qui ne fait que rendre |
| `outline-depth` | `0`, idem |

La chaîne vide sur une option qui nomme un fichier est exactement ce que ce tableau produit,
et c'est ce qui a donné D59.

**La commande qui produit un PDF.** Une commande de développement dédiée, `snp:dev:pdf:render`,
qui rend les quatre familles depuis des identifiants figés et écrit dans un sous-répertoire
nommé par `--tag` : de quoi comparer un avant et un après sur les mêmes documents.

**La route de substitution (D13)** y est celle d'une vraie migration : le paramètre pointe
vers l'un ou l'autre binaire, l'application n'est pas touchée, et un script d'enrobage porte
l'emplacement du navigateur et la concession `--no-sandbox` que le conteneur impose.

**Ce qu'elle a trouvé.** Trois défauts qu'aucun des 72 cas ne pouvait voir, parce qu'ils
tiennent à ce que de vraies feuilles de style et de vrais gabarits font : les polices web
d'une autre origine refusées (D58), la valeur vide prise pour un fichier nommé `""` (D59,
qui empêchait purement et simplement factures et feuilles de route de sortir), et la règle
`@page` du document décidant des marges (D60, qui cachait le haut de chaque devis sous le
bandeau). Les deux mesures de poids de D62 et D63 — 1,4 Mo et 2,5 Mo repris sur des
documents que quelqu'un imprime pour de bon — viennent aussi d'elle.

**Ce qu'elle dit de la migration.** `--zoom 0.8` est le rapport du « smart shrinking »
lui-même, mesuré sur la même ligne de texte : 13,3 pt par Chromium contre 10,7 pt par
wkhtmltopdf. À ce facteur, 19 des 21 documents d'échantillon tombent sur le même nombre de
pages que la référence. Les deux qui restent sont des veuves de gabarit, pas des options.

## Ce que ces deux cibles ne couvrent pas

Elles sont toutes deux les nôtres. Une application tierce apporterait des idiomes CSS que
personne ici n'écrit, et c'est la seule chose qui manque à ce tableau. Le jour où une
migration extérieure est rapportée, elle vaut plus qu'un troisième corpus écrit ici.
