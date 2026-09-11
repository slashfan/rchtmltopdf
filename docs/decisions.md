# Décisions de conception

Décisions prises le 2026-09-11 lors de la revue du brief. Chaque entrée indique le choix retenu, la raison et les alternatives écartées. À amender par une nouvelle entrée, pas en réécrivant l'ancienne.

---

## D01 — Parsing de la ligne de commande : tokenizer maison + structs typées

**Choix.** Un tokenizer écrit à la main découpe `argv` selon la grammaire wkhtmltopdf (options globales, puis objets `page` / `cover` / `toc` avec leurs options propres, puis chemin de sortie), et remplit des structs d'options typées. Clap n'est utilisé, au plus, que pour le rendu de `--help` / `--version`.

**Pourquoi.** La grammaire n'est pas celle d'une CLI classique : « tout sauf le dernier argument est une entrée », options à deux valeurs (`--cookie name value`, `--custom-header name value`), unités par défaut en millimètres, options par objet. Clap se laisse plier mais chaque cas particulier devient une bataille. Décidé dès V0 car le multi-document de V2 en dépend.

**Écarté.** Clap avec pré-passe ; Clap pur mono-document (réécriture obligatoire en V2).

## D02 — Options non implémentées : acceptées, warning sur stderr, exit 0

**Choix.** Toutes les options documentées dans `wkhtmltopdf --extended-help` sont reconnues dès V1. Les options non implémentées émettent une ligne de warning sur stderr et sont ignorées. Une option réellement inconnue reste une erreur de parsing. `-q` supprime les warnings.

**Pourquoi.** Les wrappers (KnpSnappy, Laravel Snappy) émettent des options imprévues ; une erreur de parsing casse l'application en production. Le warning garde visibles les écarts de migration.

**Écarté.** Silence par défaut ; mode `--strict`.

## D03 — Valeurs par défaut : celles de wkhtmltopdf

**Choix.** A4, marges 10 mm, portrait, média `screen` émulé, backgrounds activés, JavaScript activé, `--javascript-delay 200`, accès aux fichiers locaux désactivé. `--print-media-type` bascule sur les CSS print.

**Pourquoi.** Chromium imprime en média `print` par défaut et en Letter avec marges 1 cm ; reproduire les défauts wkhtmltopdf est ce qui évite que chaque document migré change de taille et de mise en page. L'émulation `screen` avant `Page.printToPDF` est le point le plus facile à oublier.

**Écarté.** Défauts wkhtmltopdf mais média print ; défauts Chromium.

## D04 — En-têtes et pieds de page texte : templates Chromium en V1, overlay PDF ensuite

**Choix.** En V1, `[page]`, `[topage]`, `[date]`, `[title]`, `[url]` sont traduits vers `headerTemplate` / `footerTemplate` de `Page.printToPDF`. Le crate `pdf` est conçu pour qu'en V2 les en-têtes puissent être dessinés directement sur les pages (overlay via lopdf).

**Pourquoi.** Les templates Chromium couvrent l'usage courant à faible coût, mais n'exécutent pas de JavaScript, ne chargent pas de ressources externes et ne peuvent pas décaler la numérotation entre documents. L'overlay lève ces limites (`[section]`, `--header-html` avec scripts, numérotation multi-document) au prix de l'embarquement de fontes.

**Écarté.** Overlay dès V1 ; templates Chromium uniquement.

## D05 — Communication avec Chromium : couche CDP minimale sur `--remote-debugging-pipe`

**Choix.** Client JSON-RPC écrit en interne, limité aux domaines nécessaires : Target, Page, Network, Emulation, Runtime, Fetch. Transport par pipe (fd 3/4), pas de WebSocket.

**Pourquoi.** Peu de méthodes sont nécessaires ; une couche maison reste petite et sous contrôle. Le pipe évite les collisions de ports quand une application lance plusieurs conversions en parallèle. chromiumoxide est maintenu par à-coups et tire un arbre de dépendances important.

**Écarté.** chromiumoxide ; headless_chrome (synchrone, peu maintenu).

## D06 — Entrée stdin : fichier temporaire, navigation `file://`

**Choix.** Le contenu de stdin est écrit dans un fichier temporaire, puis chargé via `file://`. Les chemins relatifs se résolvent comme avec wkhtmltopdf ; les règles d'accès aux fichiers locaux (D10) s'appliquent à l'identique.

**Pourquoi.** `Page.setDocumentContent` sur `about:blank` casse toute ressource relative. Snappy écrit déjà des fichiers temporaires ; le comportement reste cohérent entre stdin et fichier.

**Écarté.** Interception Fetch avec URL synthétique ; `setDocumentContent`.

## D07 — Attente du rendu : load + réseau inactif + fontes + délai, `--window-status` en override

**Choix.** Séquence : événement `load`, puis réseau inactif (~500 ms sans requête), puis `document.fonts.ready`, puis `--javascript-delay` (200 ms par défaut). Si `--window-status` est fourni, on attend `window.status` à la place du délai. `--timeout` (D16) borne l'ensemble.

**Pourquoi.** Le seul `load` + délai rate les webfonts et les XHR tardifs et imprime avec des fontes de substitution. Le seul réseau inactif bloque sur les pages avec long-polling ou beacons.

**Écarté.** load + délai seul ; réseau inactif seul.

## D08 — Dimensionnement : 96 dpi natif, `--zoom` honoré, dpi / smart-shrinking ignorés avec warning

**Choix.** Le contenu est rendu à 96 px CSS par pouce. `--zoom` est traduit en `scale` de `printToPDF`. `--dpi`, `--disable-smart-shrinking`, `--image-dpi` sont acceptés avec warning. Un guide de migration documente les hacks hérités à retirer.

**Pourquoi.** Le « smart shrinking » de wkhtmltopdf est le premier point de douleur des migrations ; des années de CSS y sont calées, souvent avec `--dpi 96` ou `--zoom 1.3`. L'émuler donne une formule floue et impossible à documenter. Mieux vaut un comportement prévisible et un guide.

**Écarté.** Émulation du shrink par défaut ; flag `--wk-shrink-compat` (à reconsidérer si les migrations réelles le réclament).

## D09 — Localisation de Chromium : flag > env > chemins connus > cache explicite, jamais de téléchargement silencieux

**Choix.** Ordre de résolution : `--chromium-path`, variables d'environnement (`CHROME_PATH` et équivalents), emplacements système connus (`chromium`, `chromium-browser`, `google-chrome`, `chrome-headless-shell`, bundles macOS), puis un répertoire de cache alimenté par une sous-commande explicite `fetch-chromium` épinglée sur une version Chrome for Testing. En cas d'échec, l'erreur liste tout ce qui a été tenté. L'image Docker embarque `chrome-headless-shell`.

**Pourquoi.** Un téléchargement à l'exécution en production est un signal d'alerte et casse les environnements hors ligne ; le mode système seul pénalise les postes de développement.

**Écarté.** Chromium système uniquement ; téléchargement automatique au premier lancement.

## D10 — Accès aux fichiers locaux : désactivé par défaut, `--enable-local-file-access`, `--allow <path>`

**Choix.** Sémantique de wkhtmltopdf 0.12.6. Les sous-ressources `file://` depuis une page http(s) sont toujours bloquées ; depuis un document `file://` ou stdin, autorisées seulement avec le flag. `--allow` autorise des répertoires précis.

**Pourquoi.** Du HTML contrôlé par l'utilisateur (facture) plus accès `file://` est exactement la classe de vulnérabilité qui a conduit wkhtmltopdf à changer ce défaut. Les applications Symfony passent déjà ces flags.

**Écarté.** Activé par défaut ; désactivé sans override pour stdin.

## D11 — Modèle de processus : un Chromium par invocation, crate `browser` prêt pour un pool

**Choix.** Lancement, impression, sortie, avec un `user-data-dir` temporaire à chaque fois. L'interface du crate `browser` est orientée session pour qu'un mode démon / pool (V2+) réutilise des instances chaudes sans toucher à la CLI.

**Pourquoi.** Correspond au modèle sans état de wkhtmltopdf et aux attentes de Snappy. Un démon dès V1 ajoute cycle de vie, authentification et nettoyage avant que le cœur soit stable.

**Écarté.** Mode démon dès le départ ; connexion à un Chrome existant via `--remote-debugging-port`.

## D12 — Post-traitement PDF : lopdf, isolé derrière le crate `pdf`

**Choix.** lopdf pour la fusion, l'arbre de pages, les outlines, le dictionnaire Info et, plus tard, l'ajout de content streams pour l'overlay (D04). Aucun type lopdf ne sort du crate.

**Pourquoi.** Mature, pur Rust, manipulation bas niveau des objets. L'isolation permet d'en changer.

**Écarté.** pdf-writer + pdf-rs ; appel à qpdf / pdftk (dépendance runtime que les utilisateurs de wkhtmltopdf n'ont jamais eue).

## D13 — Nom : nom propre, symlink `wkhtmltopdf` livré par les paquets

**Choix.** Un seul nom pour le dépôt, le crate et le binaire (`rchtmltopdf`). Les paquets installent un symlink `wkhtmltopdf` et déclarent `Provides` / `Conflicts` avec le paquet distro. L'image Docker expose les deux noms.

**Pourquoi.** Un binaire littéralement nommé `wkhtmltopdf` entre en conflit avec les paquets distro, brouille les rapports de bug et les contrôles de version, et pose des questions de marque. Le symlink préserve le remplacement transparent.

**Écarté.** Binaire nommé `wkhtmltopdf` ; nom propre sans symlink.

## D14 — Codes de sortie : sémantique wkhtmltopdf, `--load-error-handling abort|skip|ignore`

**Choix.** Échec du document principal : exit 1, pas de PDF. Échec d'une sous-ressource : `abort` (défaut) écrit le PDF mais sort en 1 avec `Exit with code 1 due to network error: <Nom>` sur stderr ; `skip` et `ignore` sortent en 0. `--load-media-error-handling` vaut `ignore` par défaut. Les lignes de progression vont sur stderr, supprimées par `-q`.

**Pourquoi.** Snappy lève une exception quand le code de sortie est non nul et que stderr n'est pas vide ; les applications dépendent de cette sémantique et de ces options.

**Écarté.** Toujours 0 si un PDF est produit ; schéma de codes propre.

## D15 — Tests : assertions structurelles, Chromium réel en CI, matrice de compatibilité

**Choix.** Tests unitaires sur le tokenizer et la traduction des options. Tests d'intégration en CI avec un Chrome for Testing épinglé, assertions via lopdf sur nombre de pages, format, marges, texte extrait, métadonnées. Une matrice rejoue des jeux d'options réels issus de KnpSnappy, Laravel Snappy et de la doc wkhtmltopdf. Pas de comparaison pixel.

**Pourquoi.** La partie risquée est CDP + comportement Chromium ; elle doit être testée. Les goldens pixel sont fragiles entre versions de Chromium et de fontes, et le projet ne promet pas la parité pixel.

**Écarté.** Goldens pixel ; tests unitaires seuls.

## D16 — Timeout : `--timeout` global, 30 s par défaut, exit 1, pas de PDF partiel

**Choix.** Borne lancement de Chromium + navigation + attente + impression. `--timeout 0` désactive. `--no-stop-slow-scripts` est accepté et allonge l'attente des scripts.

**Pourquoi.** wkhtmltopdf n'a pas de timeout et un Chromium bloqué fuit un processus et bloque l'appelant. Un PDF partiel silencieux dans une chaîne de facturation est dangereux.

**Écarté.** Pas de timeout ; timeout avec PDF partiel.

## D17 — Runtime : Tokio, flavor `current_thread`

**Choix.** Runtime Tokio mono-thread par invocation.

**Pourquoi.** CDP est événementiel (événements concurrents + requête/réponse), l'async s'impose. Une conversion par processus n'a pas besoin de pool de threads ; le démarrage reste minimal. Un futur mode pool bascule en multi-thread sans changer d'API.

**Écarté.** Synchrone avec thread lecteur ; Tokio multi-thread.

## D18 — Licence : MIT OR Apache-2.0

**Choix.** Double licence standard de l'écosystème Rust.

**Pourquoi.** Aucun code de wkhtmltopdf (LGPL) n'est réutilisé, seulement l'interface CLI ; aucune obligation LGPL ne s'applique. Permissif, adoption facile en entreprise, compatible avec une future API C.

**Écarté.** LGPL-3.0 ; AGPL-3.0.

## D19 — Distribution V1 : binaires Linux statiques + image Docker, puis .deb / .rpm / macOS

**Choix.** Binaires musl x86_64 et aarch64 via cargo-dist ou cross, publiés sur GitHub Releases. Image Docker Debian slim avec `chrome-headless-shell`, fontes et symlink `wkhtmltopdf`. `.deb`, `.rpm` et Homebrew suivent une fois la chaîne de release stable.

**Pourquoi.** L'image Docker est l'artefact dont la plupart des déploiements Symfony ont réellement besoin ; le binaire seul couvre les VM et les images maison. Tout livrer d'un coup crée une matrice de release trop large avant stabilisation du cœur.

**Écarté.** Docker seul ; tout d'un coup.

## D20 — wkhtmltoimage : reporté en V2+

**Choix.** Hors V1. À ajouter plus tard comme second binaire partageant le crate `browser` (`Page.captureScreenshot`).

**Pourquoi.** Concentrer V1 sur le PDF et prouver la couche de compatibilité CLI d'abord. Le coût marginal restera faible.

**Écarté.** Second binaire en V1 ; hors périmètre définitif.

---

## Conséquences transverses

- **Le brief doit gagner une section « smart shrinking »** dans les contraintes, et un guide de migration (options à retirer, différences de taille attendues).
- **Le critère de réussite V1 devient concret** : deux ou trois projets Symfony open source utilisant Snappy génèrent leurs PDF habituels avec le nouveau binaire sans modification de code.
- **Le crate `wkhtmltopdf-cli` du brief devient le tokenizer + table complète des options** (D01, D02) ; les crates `browser` et `pdf` gardent des interfaces neutres pour D04 et D11.
- **Nommage à unifier** dans le brief : `rchtmltopdf` partout, plus le symlink `wkhtmltopdf` (D13).

---

## Risques ouverts

### R01 — La table d'options n'a pas été confrontée à un binaire réel

`crates/cli/src/table.rs` a été écrite à partir de l'aide documentée de wkhtmltopdf 0.12.6, sans binaire disponible sur la machine de développement. Deux points sont les plus susceptibles d'être faux :

* les alias courts (`-s`, `-O`, `-T`, `-B`, `-L`, `-R`, `-d`, `-p`, `-n`, `-g`, `-l`, `-q`, `-H`, `-V`, `-h`) ;
* la frontière exacte entre options globales et options par objet.

À faire avant V1 : exécuter `wkhtmltopdf --extended-help` sur un binaire 0.12.6, diffuser la sortie dans un test de conformité, et réconcilier. Tant que ce n'est pas fait, la table est une hypothèse documentée, pas une référence.
