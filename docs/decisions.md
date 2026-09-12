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

## D21 — Workflow Git : `main` protégée, pull request obligatoire, merge en squash

**Choix.** Une seule branche longue, `main`. Toute modification passe par une branche courte et une pull request. Fusion en **squash uniquement** ; le titre de la PR devient le sujet du commit sur `main`. Les branches sont supprimées après fusion. Un ruleset GitHub impose la PR, les vérifications CI, l'historique linéaire, et interdit la suppression de `main`.

**Pourquoi.** Le titre de la PR est le seul endroit où un développeur seul écrit systématiquement un message correct : linter le titre revient donc à linter le journal, tout en laissant les commits intermédiaires libres. Le squash garantit l'historique linéaire par construction et un commit par PR, ce que `git-cliff` lira sans bruit de merge.

Le contournement du ruleset est accordé au rôle administrateur en mode `pull_request` et non `always` : un push direct sur `main` reste refusé, mais le propriétaire peut fusionner sa propre PR en cas de blocage. Le mode `always` a été essayé puis abandonné — il laissait passer les push directs, ce qui vidait le dispositif de son sens.

Le nombre d'approbations requises est **0**. Un mainteneur seul ne peut pas approuver sa propre PR ; toute valeur supérieure est un verrouillage.

**Écarté.** Merge commits et rebase-merge (désactivés au niveau du dépôt) ; approbation obligatoire ; `bypass_mode: always`.

## D22 — CI : un job agrégateur `ci` comme seule vérification requise

**Choix.** Les jobs `fmt`, `clippy`, `test`, `msrv`, `docs` et `pr-title` alimentent un job `ci` qui échoue si l'un d'eux a échoué. Le ruleset n'exige que `ci`.

**Pourquoi.** Ajouter, renommer, scinder ou matricer un job ne demande alors aucune modification de la protection de branche. Cela évite surtout le blocage classique où une vérification requise n'est jamais rapportée — job ignoré par un filtre ou une condition — et rend la PR infusionnable indéfiniment. Le job `chromium` (D15) pourra être branché par une seule ligne.

**Écarté.** Lister chaque job comme vérification requise.

## D23 — Dépôt privé jusqu'à la première conversion V0

**Choix.** Le dépôt reste privé jusqu'à ce que V0 produise un PDF. CI est limitée à Linux pendant cette période.

**Pourquoi.** Sur un dépôt privé les minutes GitHub Actions sont décomptées du quota mensuel du plan, et les runners macOS sont facturés dix fois le tarif Linux. Une matrice trois OS coûterait cher pour un projet qui n'a encore aucune dépendance externe ni code spécifique à une plateforme.

**Conséquence à ne pas oublier.** Au passage en public, élargir la matrice `test` à macOS et Windows : D09 résout des bundles macOS et le tokenizer manipule des chemins. Ajouter aussi un `CODE_OF_CONDUCT.md` et envisager la traduction de ce fichier en anglais, puisqu'il est le document le plus utile à un contributeur et le seul qu'il ne peut pas lire.

**Écarté.** Public immédiatement ; privé jusqu'à V1.

## D24 — Le paquet Cargo porte le nom du binaire

**Choix.** Le crate qui contient le binaire s'appelle `rchtmltopdf`, pas `rchtmltopdf-cli`. Les crates de support gardent leur préfixe : `rchtmltopdf-core`, `rchtmltopdf-browser`, `rchtmltopdf-pdf`.

**Pourquoi.** D13 impose un seul nom pour le dépôt, le crate et le binaire. Avec `rchtmltopdf-cli`, `cargo install rchtmltopdf` échoue et le meilleur nom reste libre sur crates.io. Corrigé à zéro commit publié, où cela ne coûte qu'une ligne d'import ; après publication d'un tag, ce serait un changement cassant.

**Écarté.** Conserver `rchtmltopdf-cli`.

## D25 — lopdf dans le harnais de conformité, derrière un module `inspect`

**Choix.** Le crate `conformance` dépend directement de lopdf et confine ses types dans un seul module, `inspect`. Aucun corps de test ne manipule un type lopdf : ils voient `Rect`, un nombre de pages, du texte. `crates/pdf` reste vide jusqu'à ce que le produit en ait besoin (#29). Amende D12.

**Pourquoi.** D12 isole lopdf pour que la bibliothèque reste remplaçable et que le produit n'en dépende pas. Un harnais de test n'est pas le produit. Ce qu'il lui faut — la MediaBox d'une page, un nombre de pages, du texte extrait, les coordonnées d'un rectangle dessiné — sont des primitives d'inspection que la fusion, les métadonnées et les outlines n'appelleront jamais. Donner à `crates/pdf` sa première API publique dessinée par les besoins des tests, des mois avant que le produit en ait une, inverse l'ordre de conception. La garantie de D12 est tenue là où elle compte : de chaque côté, un seul module connaît lopdf.

**Écarté.** Construire la surface de lecture dans `crates/pdf` dès #17 ; lire les objets PDF à la main dans les tests.

## D26 — Placement des options : trois règles, pas une

**Choix.** Une option globale n'est acceptée qu'avant la première entrée ; une option d'objet partout ; une option de table des matières uniquement après un objet `toc`. Ailleurs, c'est une erreur qui nomme l'option et l'endroit où elle devait être. Nos propres options (`Support::Extension`) restent acceptées partout.

**Pourquoi.** Vérifié en exécutant wkhtmltopdf 0.12.6.1, pas déduit de son aide, qui ne l'écrit nulle part. Le binaire réel répond `<option> specified in incorrect location` et sort en 1 dans chacun de ces cas — y compris pour une option `toc` écrite dans la zone globale, ce qui fait qu'une option `toc` n'est pas une option d'objet avec un nom plus long. Auparavant `--toc-header-text` écrit après un objet `page` s'attachait silencieusement à cette page.

**Deux écarts assumés.** Une option globale écrite entre le mot-clé `page` et son entrée : 0.12.6.1 ne diagnostique rien, il détraque la ligne — `page --copies 2 in.html` part charger `http://2` — et nous refusons, parce que refuser est la moins mauvaise des deux façons de s'en écarter. Et nos options d'extension ne sont pas sensibles à la position : elles ne portent aucun contrat de compatibilité, et `--dump-parse` est une aide au débogage que l'on ajoute au bout d'une ligne déjà écrite.

**Écarté.** Traiter `Scope::Toc` comme `Scope::Object` ; accepter les options globales partout, qui était le comportement précédent ; reproduire le ratage de 0.12.6.1 sur `page --option`.

---

## D27 — `Implemented` veut dire « honoré de bout en bout », et un plan le prouve

**Choix.** Le marqueur `Support::Implemented` de la table d'options affirme qu'écrire l'option change ce qui sort du programme, et rien de plus faible. Pour le rendre vérifiable, le crate `browser` expose `plan` : une description pure de tout ce qu'une conversion ferait — options de lancement, commandes CDP envoyées avant la navigation, dernier barreau de l'échelle d'attente, appel d'impression, échéance — construite sans démarrer quoi que ce soit. `crates/cli/tests/plan.rs` écrit chaque option seule sur une ligne de commande et exige que le plan change ; une option `Planned` ou `NoEquivalent` doit au contraire ne jamais le changer.

**Pourquoi.** L'audit (#19) a trouvé que **38 des 58 options marquées `Implemented` ne faisaient rien du tout** : la ligne de commande remplissait un champ du modèle de réglages, aucune couche ne lisait ce champ, et l'aide annonçait l'option comme fonctionnelle. Le garde existant ne pouvait pas le voir — il demande si `apply.rs` a un bras pour l'option, question qui porte sur la ligne de commande, et les 38 en avaient un. Une était pire qu'inerte : `--default-header` descendait la marge haute à 20 mm pour faire de la place à un bandeau que rien ne dessine, donc décalait le contenu d'un centimètre sans rien imprimer.

**Contrat qui en fait un garde.** Rien au-dessus de `plan` ne décide plus rien seul : `Page::prepare` envoie ce que `plan::prepare` a décidé, `print_to_pdf` ce que `plan::print` a décidé, et `load` ne consulte que `LoadPlan`. Le jour où l'une d'elles relit un réglage directement, le plan cesse de décrire la conversion et le test au-dessus devient un second avis plutôt qu'une vérification. C'est écrit en tête du module parce que c'est la seule chose qui puisse le casser silencieusement.

**Une option `Planned` continue d'être traduite.** Elle remplit le modèle de réglages et la conversion l'ignore. C'est l'état normal d'une option à moitié construite : le modèle est là où atterrit le travail de la couche suivante, et le vider en attendant reviendrait à écrire les deux moitiés à l'aveugle. Ce qui rend l'état honnête, c'est que rien en aval ne lit le champ — tenu par le test, pas par la discipline — et que le binaire avertit que l'option est ignorée.

**Deux exemptions, nommées.** `--quiet` et `--log-level` sont honorées avant qu'un navigateur démarre : aucun plan ne peut les voir. La liste les nomme avec le test qui les tient, et un test vérifie que chaque entrée désigne une option réelle et toujours annoncée. La liste est censée rester de cette longueur.

**Écarté.** Supprimer les bras de traduction des options démises, qui aurait vidé le modèle de réglages pour rien. Une liste manuelle « option → test qui la prouve », qui se périme sans que rien n'échoue, exactement comme le marqueur qu'elle remplacerait. Comparer chaque option aux seuls réglages par défaut : cela déclare inertes `--background`, `--enable-javascript` et `--no-print-media-type`, qui redisent un défaut — le garde cherche donc n'importe quel point de départ que l'option déplace, et la table contient son propre contraire.

---

## D28 — Passage en public, et ce que D23 avait prévu de travers

**Choix.** Le dépôt passe en public. La condition posée par D23 — « privé jusqu'à ce que V0 produise un PDF » — est remplie depuis #76. Le README porte en tête un avertissement sans ambiguïté : projet de week-end, pas d'usage en production, pas de release, pas de support, et le modèle de menace de `SECURITY.md` décrit ce que le code tente et non ce qu'il garantit.

**Pourquoi maintenant, et pas plus tard.** Sur un dépôt privé, les minutes GitHub Actions sont décomptées du quota mensuel ; sur un dépôt public, les runners standard sont gratuits et sans limite. La CI de ce projet compte neuf jobs par exécution et le rythme de développement en produit plusieurs dizaines par semaine. Les alternatives ont toutes un coût réel : réduire la CI, c'est retirer précisément les gardes sur lesquelles la discipline du projet repose (la table d'options tenue à un binaire réel, le plan tenu à ce qu'une conversion fait, la suite de conformité tenue à un Chromium épinglé) ; un runner auto-hébergé est une machine à maintenir, et il est dangereux sur un dépôt public qui accepte des pull requests extérieures.

**Correction à D23.** La note de D23 demandait d'élargir la matrice `test` à macOS **et Windows** au passage en public. La moitié Windows est infaisable et l'était déjà quand la note a été écrite : `crates/browser/src/lib.rs` porte un `compile_error!` explicite pour tout ce qui n'est pas Unix, parce que le protocole voyage sur les descripteurs 3 et 4 et que le transport par handles de Windows n'est pas écrit. `cargo test --workspace` sur Windows ne compile pas. La matrice s'élargit donc à macOS seul, où D09 résout de vrais bundles `.app`, et Windows reste hors de portée tant que le transport n'existe pas.

**Ce qui devient public en même temps que le code.** Les 45 commits et leur historique complet, les tickets et les pull requests avec leurs discussions. Les deux ont été passés au crible : aucun secret, aucun chemin local, aucune adresse personnelle en dehors de celles que git inscrit lui-même dans les commits.

**Écarté.** Rester privé en rognant la CI. Un runner auto-hébergé. Réécrire l'historique pour en retirer l'adresse de l'auteur, qui est un choix de l'auteur et pas une fuite.

---

## D29 — `--version` ne se fait pas passer pour wkhtmltopdf

**Choix.** `--version` imprime le nom et la version de *ce* programme, et jamais ceux de wkhtmltopdf. Deux lignes : `rchtmltopdf <version>`, puis une phrase qui dit ce que c'est.

**Pourquoi.** D13 donne au projet un nom propre en partie pour que les vérifications de version ne soient pas brouillées. Se déclarer `wkhtmltopdf 0.12.6 (with patched qt)` ferait passer un test de version et mentirait sur tout le reste : le moteur de rendu, les options réellement honorées, la pagination. Une application qui refuse de démarrer sans une version de wkhtmltopdf a un problème que ce programme ne peut pas résoudre en mentant sur ce qu'il est.

**Si un projet de référence verrouille la version.** Ce sera une nouvelle entrée numérotée, discutée, pas un changement discret dans `main.rs`. #31 et #32 diront si le cas se présente vraiment.

**Écarté.** Imprimer la version de wkhtmltopdf. Un drapeau `--fake-version` qui la produirait sur demande : il existerait pour tromper un test, et le premier rapport de bug arriverait de quelqu'un qui ne saurait pas lequel des deux programmes il exécute.

---

## D30 — MSRV portée à 1.88, parce que lopdf l'exige

**Choix.** `rust-version` passe de 1.85 à 1.88 dans le manifeste de l'espace de travail, et le job `msrv` de la CI suit.

**Pourquoi.** `lopdf@0.45` déclare `rust-version = "1.88"` : cargo refuse de le compiler plus bas, et D12 a choisi lopdf pour la couche PDF. Le choix réel était donc entre épingler une vieille version de lopdf pour protéger un nombre que personne n'avait demandé, et déplacer le nombre. 1.88 date de juin 2025 ; la distribution prévue est faite de binaires statiques et d'une image Docker (D19), pas de paquets de distribution dont la chaîne d'outils serait figée. Personne ne compile ce projet avec un rustc de plus d'un an sans le vouloir.

**Ce qui l'a attrapé.** Le job `msrv`, sur la pull request qui introduisait la dépendance. Sans lui, la première personne à compiler sur une chaîne plus ancienne aurait découvert le problème à notre place, et le message de cargo ne dit pas quelle décision l'a causé.

**Ce que ce n'est pas.** Une autorisation à utiliser la syntaxe la plus récente. La règle de `CONTRIBUTING.md` tient : relever la MSRV reste une décision délibérée, jamais un effet de bord.

**Écarté.** Épingler lopdf à une version antérieure à ses let-chains, ce qui aurait signifié porter une bibliothèque PDF vieillissante pour une promesse que personne ne réclame. Exclure `crates/pdf` du job `msrv`, ce qui aurait rendu la promesse fausse sans la retirer — le crate est compilé par quiconque installe le binaire depuis les sources.

---

## D31 — Pas de `fetch-chromium` : on documente l'installation, on facilite le chemin

**Choix.** Le binaire ne télécharge rien, jamais, sous aucune sous-commande. Le quatrième barreau de D09 reste un répertoire de cache que *quelqu'un d'autre* remplit — une étape de CI, une image de conteneur, une personne avec une archive. À la place : le README documente quatre façons d'installer un navigateur, l'erreur « introuvable » les répète avec la version épinglée, un chemin donné peut désormais être un exécutable, un répertoire ou un bundle `.app` macOS, et `--dump-chromium` dit ce qui serait utilisé et d'où il vient.

**Pourquoi.** C'est le seul endroit où le binaire lui-même aurait besoin de HTTPS. Tout le reste délègue le réseau à Chromium, et c'est précisément ce qui rend la construction statique musl triviale (#34) ; y ajouter une pile TLS en est le risque numéro un. Suivent une vérification de somme de contrôle, une extraction d'archive et une matrice plateforme/architecture — beaucoup de surface à maintenir pour un projet de week-end, et une duplication de ce que `apt`, `brew` et `@puppeteer/browsers` font déjà mieux.

**Ce que cela ne change pas.** D09 tient en entier : l'ordre de résolution, l'interdiction de télécharger au moment de la conversion, et l'erreur qui liste tout ce qui a été tenté. Seule la façon dont le quatrième barreau se remplit change, et elle n'était de toute façon jamais automatique.

**Ce que cela oblige à faire.** Si l'erreur ne dit pas quoi installer, ce choix devient hostile : la personne qui la lit n'a alors ni navigateur ni instruction. C'est pourquoi le message et la section du README font partie de cette décision et pas d'un ticket séparé.

**Écarté.** `fetch-chromium` tel que D09 l'annonçait (#33, fermé). Un téléchargement au premier lancement, qui était déjà écarté par D09. Ne rien faire : laisser l'erreur promettre une commande qui n'existerait jamais aurait été le pire des trois.

---

## D32 — Release à la main sur runners natifs, sans cross ni framework

**Choix.** Les binaires musl x86_64 et aarch64 sont construits par une matrice écrite à la main dans `.github/workflows/release.yml`, chacun sur un runner de son architecture. Ni `cross`, ni `cargo-dist`, ni aucun autre cadre de publication. L'archive est un `tar.gz` contenant le binaire, un lien symbolique `wkhtmltopdf` (D13), les licences et le journal des changements.

**Pourquoi.** D19 disait « via cargo-dist ou cross », écrit avant que le dépôt soit public. Deux choses ont changé. Les runners Arm sont gratuits sur un dépôt public (D28), donc l'émulation n'a plus de raison d'être : un binaire que personne n'a exécuté sur l'architecture qu'il prétend supporter ne prouve rien. Et le lien symbolique est une contrainte que les cadres de publication tiennent mal — leur disposition d'archive est une opinion, et `.zip` n'a pas de façon portable de porter un lien. La matrice fait une trentaine de lignes, l'image Docker doit être écrite à la main de toute façon, et une dépendance de moins dans la chaîne qui signe ce que les gens téléchargent est une bonne chose en soi.

**Ce qui est vérifié plutôt que supposé.** Que le binaire est réellement statique — c'est tout l'intérêt d'une construction musl, et un binaire lié dynamiquement fonctionnerait en CI pour échouer précisément là où il était destiné. Que le lien symbolique survit à l'archivage, en la déballant. Que le binaire démarre.

**Le workflow s'exécute sur les pull requests** qui touchent les manifestes ou lui-même, sans publier. Un workflow de publication qui n'a jamais tourné ne fonctionne pas, et le moment de le découvrir n'est pas pendant qu'on pose une étiquette.

**Écarté.** `cross` et l'émulation qemu. `cargo-dist`, à revoir à la 1.0 quand les installeurs et Homebrew entreront dans le périmètre — c'est là que sa valeur apparaît. Un `.zip` en plus du `tar`, qui aurait livré un `wkhtmltopdf` qui n'est pas un lien.

---

## D33 — L'image Docker ne désactive pas le bac à sable

**Choix.** L'image ne met pas `--no-sandbox` dans son point d'entrée. Un `docker run` nu échoue, avec le message qui nomme la cause et les trois façons d'y remédier. Le README documente la commande recommandée — `--cap-add=SYS_ADMIN`, qui laisse au navigateur son bac à sable — et présente `--no-sandbox` comme le renoncement explicite, pour du HTML que l'on a soi-même produit.

**Pourquoi.** Le modèle de menace est du HTML non fiable (D10), et l'image est l'artefact que la plupart des déploiements utiliseront : y désactiver le bac à sable par défaut serait exactement la posture que ce projet existe pour améliorer. Le coût est une première exécution qui échoue, et il est supportable parce que **l'échec s'explique** : le message dit ce qui s'est passé, pourquoi c'est habituel dans un conteneur, et quoi faire — l'inverse d'un plantage muet.

L'asymétrie compte aussi. Ne pas l'intégrer laisse les deux options ouvertes à l'opérateur ; l'intégrer oblige qui veut le bac à sable à écraser le point d'entrée pour le récupérer.

**Mesuré.** Dans un démon Docker par défaut, trois commandes fonctionnent et chacune concède quelque chose : `--cap-add=SYS_ADMIN` (bac à sable conservé, capacité large accordée), `--security-opt seccomp=unconfined` (bac à sable conservé, seccomp désactivé), `--no-sandbox` (seccomp et capacités de Docker intacts, bac à sable du navigateur désactivé). Aucune n'est gratuite, ce qui est précisément pourquoi le choix revient à l'opérateur et pas à l'image.

**Vérifié en CI.** Que la commande recommandée convertit, et que la commande nue **refuse** — la deuxième moitié compte autant : sans elle, un changement de configuration du démon ou du point d'entrée pourrait rendre l'image silencieusement permissive sans que rien ne le remarque.

**Écarté.** Intégrer `--no-sandbox` avec un avertissement en gras dans la documentation. Un point d'entrée qui teste si le bac à sable fonctionne et l'abandonne sinon : l'opérateur n'apprendrait jamais dans quel mode il tourne, et c'est ce que D10 existe pour empêcher. Embarquer le profil seccomp de Chromium, à revoir si les projets de référence (#32) montrent que `SYS_ADMIN` pose problème.

## D34 — Fusion : les fontes ne sont pas dédoublonnées entre documents

**Choix.** La fusion (#36) renumérote chaque objet, reconstruit un arbre de pages unique, recopie sur chaque page ce qu'elle héritait de son ancien arbre, garde le dictionnaire Info du premier document, et laisse chaque document avec ses propres fontes. Un PDF de dix documents dans la même police embarque dix sous-ensembles de cette police.

**Pourquoi.** Chromium sous-ensemble une fonte par document : deux documents dans la même police portent deux sous-ensembles différents, sous deux noms différents, avec deux tables de glyphes qui ne se recouvrent qu'en partie. Les réunir est une opération sur le programme de fonte — fusionner les glyphes, refaire `cmap`, `hmtx` et `loca`, réécrire les flux de contenu qui les référencent — et non une opération PDF. C'est le prix d'une fusion après impression, et il est connu : wkhtmltopdf imprimait ses documents dans une seule session Qt, qui partageait ses fontes, et un document migré qui en assemble plusieurs sera plus lourd. Le guide de migration le dit.

**Mesuré.** Trois documents d'un paragraphe, dans la même police embarquée : 5,5 Ko seul, 9,0 Ko à deux, 12,9 Ko à trois, avec autant de `FontFile` que de documents. Chaque document ajoute son sous-ensemble, environ 3 Ko ici, et rien d'autre ne grossit.

**Écarté.** Le dédoublonnage par sous-ensemble, à reconsidérer si les projets de référence (#32) montrent des documents où la taille compte. Concaténer les documents dans un même DOM pour n'imprimer qu'une fois : une seule fonte, mais les options, marges et en-têtes propres à chaque objet disparaissent, et c'est précisément ce que la grammaire wkhtmltopdf promet.

## D35 — Plusieurs documents : un navigateur, une page par document, relancé si le lancement diffère

**Choix.** Une conversion à plusieurs documents lance un Chromium (D11), ouvre une page par document, charge et imprime chacun dans l'ordre de la ligne de commande, puis fusionne (D34). Le navigateur n'est relancé que pour un document dont les options se décident sur la ligne de commande du navigateur et non par le protocole : `--proxy`, `--minimum-font-size`, `--no-images`, `--no-stop-slow-scripts`. Deux documents qui partagent ces options partagent un navigateur ; deux qui diffèrent en ont chacun un, et le résultat est le même.

**Pourquoi.** Un lancement coûte une demi-seconde et un profil temporaire, et le cas courant — dix factures avec les mêmes options — n'en a besoin que d'un. Relancer plutôt que d'imprimer avec les mauvaises options, parce qu'une option d'objet honorée pour le premier document et ignorée pour le second est exactement le genre d'écart silencieux que D27 existe pour interdire.

**Écarté.** Un navigateur par document : simple, correct, et dix fois plus lent pour le cas courant. Charger toutes les pages avant d'en imprimer une : un `--javascript-delay` par page se recouvre, mais la mémoire est celle de tous les documents à la fois, et l'ordre d'impression doit être reconstitué. Un pool ou un démon : toujours différé (D11).

## D36 — Outline : celui que Chromium génère à l'impression, borné et fusionné après coup

**Choix.** `Page.printToPDF` est appelé avec `generateDocumentOutline`, et Chromium dérive l'outline des titres `<h1>` à `<h6>`, imbriqués par niveau, avec une destination par entrée. Le crate `pdf` fait le reste après l'impression : `--outline-depth` décroche les entrées sous la borne et les élague du fichier, `--no-outline` retire la racine, la fusion (#36) enchaîne les outlines des documents sous une racine unique, et `--dump-outline` écrit ce que le fichier porte, au format XML de wkhtmltopdf. Un document `--exclude-from-outline`, ou un `cover`, est simplement imprimé sans outline.

**Pourquoi.** C'est le chemin le moins cher, et #40 demandait de l'essayer avant de construire quoi que ce soit. L'alternative — extraire les titres par script, puis retrouver la page de chacun — bute sur le problème que la table des matières (V3) devra résoudre de toute façon : quelle page porte tel titre n'est connu qu'après l'impression. Chromium le sait au moment d'imprimer, et l'écrit.

**Ce que ça coûte.** L'outline de Chromium est tout ou rien par document : la profondeur se coupe après coup, ce qui est une passe de plus sur le fichier. Les titres sont ceux que Chromium extrait — balisage retiré, entités résolues — et non le HTML brut. `page` dans le XML est la page physique dans le fichier ; quand `--page-offset` existera (#39), c'est là qu'il faudra choisir entre page physique et page affichée. `link` et `backLink` sont vides : ils nommaient des ancres que wkhtmltopdf plantait dans le document, et rien ne les plante encore (#43).

**Écarté.** Extraire les titres soi-même (voir ci-dessus). Imprimer avec l'outline seulement quand `--outline-depth` vaut sa valeur par défaut, pour économiser la passe : la passe est aussi celle qui lit l'outline pour `--dump-outline`, et deux chemins pour un même résultat sont deux chemins à tester.

## D37 — Liens : ancres résolues avant la fusion, liens entre documents rendus internes, formulaires sans équivalent

**Choix.** Chromium écrit une annotation par `<a href>` : une destination *nommée* pour une ancre du document, résolue par une table `Dests` dans le catalogue, et une action `URI` pour tout le reste, résolue en absolu contre l'URL du document. Le crate `pdf` fait trois choses avec. Avant qu'une fusion abandonne le catalogue, chaque nom est résolu vers la destination explicite qu'il désignait, sur l'annotation même. Un lien `URI` vers un autre document de la même conversion — exactement l'URL contre laquelle ce document a été imprimé, avec ou sans fragment — devient une destination dans le fichier : l'ancre nommée, ou la première page du document. Et `--disable-internal-links` / `--disable-external-links` retirent l'une ou l'autre sorte, par objet ; `--keep-relative-links` réécrit en relatif un lien situé sous le répertoire du document.

`--enable-forms` et `--disable-forms` sont classés sans équivalent : le chemin d'impression de Chromium dessine un contrôle de formulaire tel qu'il s'affiche et n'écrit aucun champ derrière — ni `AcroForm`, ni `Widget` — mesuré sur la 153 épinglée.

**Pourquoi.** Résoudre les noms est ce qui rend la fusion correcte : sans cela, un document fusionné a des liens qui pointent sur rien, et la conversion d'un seul document, qui garde son catalogue, les aurait laissés fonctionner — l'écart le plus discret possible. Rendre internes les liens entre documents est ce que faisait wkhtmltopdf, et c'est l'usage : une table des matières écrite à la main dans un premier document, qui renvoie aux suivants. Une table de noms fusionnée aurait été l'alternative, et deux documents qui définissent `#top` entrent en collision ; une destination explicite sur l'annotation n'a pas ce problème.

**Ce que ça coûte.** `--keep-relative-links` ne peut défaire que ce qu'il reconnaît : un lien résolu *sous* le répertoire du document redevient relatif, un lien vers `../` reste absolu, parce que le navigateur a effacé ce qui était écrit. L'arbre de noms sous `Names`, que la spécification permet et que Chromium n'écrit pas, n'est pas lu. Un lien entre documents dont l'URL diffère de celle d'impression — `b.html` contre `./b.html` sont identiques une fois résolus, mais `B.html` sur un système sensible à la casse ne l'est pas — reste une `URI`.

**Écarté.** Fusionner les tables `Dests` en préfixant les noms par document : plus d'objets pour le même résultat, et un lien entre documents aurait de toute façon dû être réécrit. Produire des champs de formulaire soi-même à partir du DOM : c'est écrire un moteur de formulaires PDF pour une option que wkhtmltopdf lui-même désactivait par défaut.

## D38 — En-têtes et pieds de page : imprimés par Chromium en feuilles, une par page, puis apposés

**Choix.** Les documents sont imprimés sans bandeaux et fusionnés. Le nombre de pages de chacun est alors lu sur le résultat, la numérotation est calculée dans les deux repères de wkhtmltopdf (`[page]`/`[topage]` sur l'ensemble, `[sitepage]`/`[sitepages]` dans le document, `[frompage]`, décalage `--page-offset`, couverture non comptée), et un document HTML d'une **feuille par page** est construit, chaque feuille aux dimensions du papier portant l'en-tête et le pied de sa page avec tous les placeholders déjà substitués. Le navigateur qui a imprimé les pages imprime ce document, et le crate `pdf` appose chaque feuille sur sa page : les ressources de la feuille sont déplacées sur la page sous des noms qui lui sont propres, chaque opérateur qui les nomme est réécrit, et le dessin de la feuille est ajouté **en ligne** après celui de la page, encadré par `q`/`Q`.

**Pourquoi.** D04 prévoyait un overlay dessiné par nous-mêmes, et #38 en liste le prix : embarquer une police, la sous-ensembler, écrire son descripteur et sa table `ToUnicode`, mesurer le texte pour centrer et aligner à droite, substituer `Arial` sur une machine qui ne l'a pas. Faire imprimer les bandeaux par Chromium supprime tout cela : la typographie, les polices, la mesure, l'extraction du texte restent celles du navigateur, et le coût est **une impression supplémentaire par conversion**, pas par page. Ce qui est réellement écrit à la main tient en une fonction : renommer des ressources et concaténer des flux de contenu. Le même mécanisme portera `--header-html` et `--footer-html`, chaque feuille embarquant alors le document de l'utilisateur.

**Pourquoi en ligne et pas en Form XObject.** Un XObject serait plus propre — une ressource par page, un `Do` — mais le texte qu'il contient est invisible pour les extracteurs, celui de lopdf compris, et un pied de page que personne ne peut extraire fait échouer les assertions de la matrice de compatibilité elle-même (D15).

**Géométrie.** Un bandeau est ancré au bord du papier dans une boîte d'au moins la hauteur de la marge de ce côté, sa ligne alignée côté contenu. Conséquences, toutes tenues par `conformance/tests/bands.rs` : le contenu ne bouge jamais ; le filet d'un `--header-line` est exactement sur la ligne de marge ; `--header-spacing` écarte le contenu et pas le bandeau ; un bandeau plus haut que la marge grandit vers le contenu et mord dessus, comme dans wkhtmltopdf. Un seul écart mesuré avec les gabarits Chromium : le filet du bandeau par défaut remonte d'environ un point, sur la marge au lieu d'un peu en dessous.

**Numérotation.** `[page]` = décalage + rang courant parmi les pages comptées ; `[topage]` = décalage + total des pages comptées ; `[frompage]` = décalage + rang de la première page du document ; une couverture ne compte pas et le décalage est celui de l'objet. `[section]`, `[subsection]`, `[subsubsection]` : le dernier titre du niveau sur la page ou avant elle, dans le même document, lu dans l'outline (D36), qui est alors généré même sous `--no-outline`.

**Écarté.** L'overlay maison de D04 (voir ci-dessus). Imprimer deux fois chaque document, la seconde avec les bons numéros dans les gabarits : le double du coût, et faux pour toute page à effets de bord, contenu daté ou nonce (#39). Un gabarit Chromium par page via une impression par page : N impressions au lieu d'une.

---

## Conséquences transverses

- **Le brief doit gagner une section « smart shrinking »** dans les contraintes, et un guide de migration (options à retirer, différences de taille attendues).
- **Le critère de réussite V1 devient concret** : deux ou trois projets Symfony open source utilisant Snappy génèrent leurs PDF habituels avec le nouveau binaire sans modification de code.
- **Le crate `wkhtmltopdf-cli` du brief devient le tokenizer + table complète des options** (D01, D02) ; les crates `browser` et `pdf` gardent des interfaces neutres pour D04 et D11.
- **Nommage à unifier** dans le brief : `rchtmltopdf` partout, plus le symlink `wkhtmltopdf` (D13).

---

## Risques ouverts

### R01 — La table d'options n'a pas été confrontée à un binaire réel *(résolu)*

`crates/cli/src/table.rs` a été écrite à partir de l'aide documentée de wkhtmltopdf 0.12.6, sans binaire disponible sur la machine de développement. Deux points sont les plus susceptibles d'être faux :

* les alias courts (`-s`, `-O`, `-T`, `-B`, `-L`, `-R`, `-d`, `-p`, `-n`, `-g`, `-l`, `-q`, `-H`, `-V`, `-h`) ;
* la frontière exacte entre options globales et options par objet.

**Résolu (#18).** `wkhtmltopdf 0.12.6.1 (with patched qt)` a été exécuté dans un conteneur, son `--extended-help` est versionné à `crates/cli/tests/fixtures/`, et `reference_help.rs` tient la table dessus à chaque exécution des tests.

Le verdict dément la moitié de la crainte. Les 122 options étaient toutes présentes, **tous les alias courts étaient corrects**, toutes les arités aussi. Trois écarts seulement :

* `--cookie-jar` était classée option d'objet alors qu'elle est globale ;
* `--redirect-delay` figurait dans la table et n'existe pas en 0.12.6.1 ;
* les règles de *placement* sont au nombre de trois et pas d'une (D26), ce qui était le vrai risque et n'était pas celui qui avait été écrit ici.

La table n'est plus une hypothèse. Elle ne peut plus dériver sans qu'un test échoue.
