# Brief — remplacement moderne de `wkhtmltopdf` en Rust

## Objectif

Créer un outil open source moderne en Rust capable de remplacer `wkhtmltopdf` sur ses usages les plus courants, tout en utilisant un moteur Chromium récent pour le rendu HTML/CSS/JavaScript.

L'objectif principal est la **compatibilité de l'interface CLI**, pas la reproduction pixel-perfect du rendu historique WebKit.

> `wkhtmltopdf` interface, modern Chromium rendering.

## Principe

Le binaire Rust agit comme orchestrateur :

```text
wkhtmltopdf-compatible CLI
        ↓
Rust core
        ↓
Chrome DevTools Protocol
        ↓
Chromium / chrome-headless-shell
        ↓
PDF
```

Le moteur HTML n'est pas réimplémenté.

Chromium assure le rendu et l'impression PDF via `Page.printToPDF`.

## Expérience utilisateur cible

Une commande existante comme :

```bash
wkhtmltopdf \
    --page-size A4 \
    --margin-top 15mm \
    --footer-center "Page [page] / [topage]" \
    https://example.com/invoice/42 \
    invoice.pdf
```

doit fonctionner sans modification ou avec un minimum d'adaptation.

Le projet, le crate et le binaire portent un seul nom :

```text
rchtmltopdf
```

Les paquets (Docker, `.deb`, `.rpm`) installent en plus un symlink `wkhtmltopdf` et déclarent `Provides` / `Conflicts` avec le paquet distro, afin de permettre un remplacement transparent dans les applications existantes sans collision de nom ni confusion dans les rapports de bug (voir [D13](decisions.md)).

## Scope V0

Supporter :

* URL HTTP/HTTPS
* fichier HTML local
* entrée depuis `stdin`
* sortie fichier ou `stdout`
* lancement/détection de Chromium
* formats A4, A3, Letter...
* largeur/hauteur personnalisées
* marges
* portrait/paysage
* backgrounds
* CSS print
* JavaScript
* timeout
* codes de sortie propres

Exemples :

```bash
rchtmltopdf page.html page.pdf
rchtmltopdf https://example.com page.pdf
cat page.html | rchtmltopdf - page.pdf
```

## Scope V1

Ajouter la compatibilité avec les options `wkhtmltopdf` les plus utilisées :

```text
--page-size
--page-width
--page-height
--margin-top
--margin-right
--margin-bottom
--margin-left
--orientation
--zoom
--javascript-delay
--cookie
--custom-header
--username
--password
--user-style-sheet
--print-media-type
--enable-local-file-access
--footer-center
--footer-left
--footer-right
--header-*
```

Dès V1, **toutes** les options de `wkhtmltopdf --extended-help` sont reconnues par le parser. Celles sans équivalent Chromium sont acceptées et ignorées avec un warning explicite sur stderr (supprimé par `-q`). Une option réellement inconnue reste une erreur. Un wrapper comme KnpSnappy ne doit jamais casser sur une option que nous n'avons pas encore implémentée (voir [D02](decisions.md)).

Les valeurs par défaut sont celles de wkhtmltopdf, pas celles de Chromium : A4, marges 10 mm, portrait, média `screen` émulé, backgrounds et JavaScript activés, `--javascript-delay 200`, accès aux fichiers locaux désactivé (voir [D03](decisions.md), [D10](decisions.md)).

Les codes de sortie et les messages stderr reprennent la sémantique wkhtmltopdf, y compris `--load-error-handling abort|skip|ignore` et `--load-media-error-handling` (voir [D14](decisions.md)). Un `--timeout` global (30 s par défaut) borne l'ensemble de la conversion (voir [D16](decisions.md)).

## Scope ultérieur

V2 :

* plusieurs documents
* cover
* headers/footers HTML
* numérotation avancée
* fusion PDF
* bookmarks / outline

V3 :

* génération automatique de table des matières
* compatibilité `toc`
* pagination multi-document

Sans jalon :

* API C compatible `libwkhtmltox` — possible, pas prévue, et attendant que quelqu'un en ait
  besoin ([D43](decisions.md)). Le critère de réussite passe par la ligne de commande :
  Snappy lance le binaire, il ne lie rien.

## Architecture Rust envisagée

```text
crates/
    core/
        modèle de document
        configuration
        erreurs

    browser/
        lancement Chromium
        communication CDP
        navigation
        attente du rendu

    pdf/
        génération
        merge
        metadata
        outlines

    cli/
        tokenizer de la grammaire wkhtmltopdf
        (globales, objets page/cover/toc, sortie)
        table complète des options wkhtmltopdf
        traduction vers le modèle de document
```

Le crate `cli` n'est pas une CLI Clap classique : la grammaire wkhtmltopdf (options par objet, options à deux valeurs, unités implicites en mm) impose un tokenizer maison alimentant des structs typées (voir [D01](decisions.md)).

Les crates `browser` et `pdf` gardent des interfaces neutres : `browser` est orienté session pour permettre plus tard un mode pool/démon sans toucher à la CLI ([D11](decisions.md)) ; `pdf` isole lopdf et prévoit l'overlay d'en-têtes sur les pages en V2 ([D04](decisions.md), [D12](decisions.md)).

Technologies retenues :

```text
Rust (MIT OR Apache-2.0)
Tokio, runtime current_thread
Chromium / chrome-headless-shell
Chrome DevTools Protocol via --remote-debugging-pipe
couche CDP minimale maison (Target, Page, Network, Emulation, Runtime, Fetch)
lopdf
```

## Distribution

Chromium est résolu dans cet ordre (voir [D09](decisions.md)) :

1. `--chromium-path` ;
2. variables d'environnement (`CHROME_PATH` et équivalents) ;
3. emplacements système connus (`chromium`, `chromium-browser`, `google-chrome`, `chrome-headless-shell`, bundles macOS) ;
4. répertoire de cache (`RCHTMLTOPDF_CACHE_DIR`) rempli par quelqu'un d'autre — étape de CI, image de conteneur, personne avec une archive. Le binaire ne télécharge rien (D31) ; le README documente l'installation et l'erreur « introuvable » la répète.

Aucun téléchargement n'a lieu à l'exécution sans demande explicite. En cas d'échec, l'erreur liste tout ce qui a été tenté.

Artefacts V1, par ordre de priorité (voir [D19](decisions.md)) :

```text
binaires Linux statiques (musl) x86_64 et aarch64, GitHub Releases
image Docker Debian slim + chrome-headless-shell + fontes + symlink wkhtmltopdf
```

Ensuite :

```text
.deb
.rpm
Homebrew / macOS
```

## Contraintes

Le projet ne garantit pas un rendu identique à `wkhtmltopdf`.

Chromium et l'ancien WebKit de wkhtmltopdf peuvent produire des différences de :

* pagination
* métriques de fonts
* line wrapping
* dimensions CSS
* comportement de certains anciens hacks CSS

La promesse doit donc être :

> compatibilité fonctionnelle de la CLI, pas compatibilité pixel-perfect.

### Smart shrinking et dimensionnement

C'est le premier point de douleur attendu lors des migrations.

Par défaut, wkhtmltopdf redimensionne le contenu pour l'adapter à la page (« smart shrinking »). Des années de CSS ont été calées sur ce comportement, souvent avec des contournements comme `--disable-smart-shrinking`, `--dpi 96` ou `--zoom 1.3`.

Chromium rend toujours à 96 px CSS par pouce. Conséquences :

* un document migré sans modification peut apparaître nettement plus grand ou plus petit ;
* les anciens contournements aggravent le problème au lieu de le corriger.

Position du projet (voir [D08](decisions.md)) :

* rendu natif à 96 dpi, sans émulation du shrink ;
* `--zoom` est honoré (traduit en `scale` de `Page.printToPDF`) ;
* `--dpi`, `--disable-smart-shrinking`, `--image-dpi` sont acceptés avec un warning ;
* un **guide de migration** documente les options à retirer et les différences de taille attendues.

Un flag d'émulation approximative du shrink pourra être reconsidéré si les migrations réelles le réclament.

## Critère de réussite V1

Une application PHP/Symfony utilisant actuellement `wkhtmltopdf` doit pouvoir remplacer le chemin du binaire :

```text
/usr/bin/wkhtmltopdf
```

par le nouveau binaire et continuer à générer ses PDF habituels sans modification du code applicatif.

Concrètement, le critère est validé quand :

1. deux ou trois projets Symfony open source utilisant KnpSnappy, choisis au démarrage du projet, génèrent leurs PDF habituels avec `rchtmltopdf` à la place de `wkhtmltopdf`, sans changement de code ;
2. la matrice de compatibilité (jeux d'options réels issus de KnpSnappy, Laravel Snappy et de la doc wkhtmltopdf) passe en CI contre un Chrome for Testing épinglé, avec assertions structurelles sur les PDF produits : nombre de pages, format, marges, texte extrait, métadonnées (voir [D15](decisions.md)) ;
3. aucune option de `--extended-help` ne provoque d'erreur de parsing.

Les différences visuelles liées au dimensionnement (voir « Smart shrinking ») ne sont pas un échec du critère ; elles relèvent du guide de migration.

## Décisions

Les choix de conception détaillés, avec leurs alternatives écartées, sont consignés dans [decisions.md](decisions.md).

