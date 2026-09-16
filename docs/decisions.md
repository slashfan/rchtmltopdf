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

## D39 — `--header-html` et `--footer-html` : le document de l'utilisateur encadré dans la feuille, mesuré avant l'impression

**Choix.** Un bandeau qui est un document est porté par le mécanisme de D38 : chaque feuille encadre le document dans un `<iframe>`, chargé une fois par page avec les placeholders passés **en chaîne de requête** — `?page=3&topage=9&section=...&date=...` — exactement comme wkhtmltopdf les passait « in get fashion », de sorte que le script `subst()` de son manuel, que tout en-tête migré embarque, lit `document.location.search` et fonctionne tel quel. Les paires de `--replace` sont ajoutées, un nom natif l'emporte sur une paire du même nom, comme dans le hash de wkhtmltopdf.

**La géométrie suit la règle de wkhtmltopdf pour ces bandeaux, qui n'est pas celle des bandeaux texte.** Lue dans `pdfconverter.cc` : si `--margin-top` n'a pas été écrit, le document est chargé et mesuré avant l'impression des pages, et **la hauteur de son `body` devient la marge**, plus `--header-spacing` ; le cadre part du bord du papier. Si la marge a été écrite, le document est logé dedans, son bord côté contenu sur la ligne de marge, et la marge grandit de l'espacement seul ; un document plus haut que la marge déborde du papier. Le pied de page est le miroir. Pour appliquer la règle, le modèle de réglages retient si `--margin-top` et `--margin-bottom` ont été écrits (`NamedMargins`), ce que deux longueurs ne savaient pas dire.

**Ce que ça fait au plan (D27).** L'appel d'impression ne peut pas connaître la hauteur d'un document sans le charger. `plan::print` décide tout le reste — la marge écrite ou zéro, l'espacement — et `plan::reserve` ajoute la mesure, et rien d'autre : le plan continue de décrire la conversion, et la mesure est un fait sur le document, pas un réglage. La mesure se fait à la largeur du contenu en pixels CSS, avec les feuilles de style que reçoivent les pages, sur une page qui applique la même règle d'accès au disque que la feuille finale, pour que ce qui est mesuré soit ce qui sera encadré.

**Accès au disque (D10).** Le document du bandeau est nommé sur la ligne de commande comme l'entrée l'est, donc il est lisible comme elle ; ce qu'il atteint sur le disque est jugé comme les sous-ressources de l'entrée, sous `--enable-local-file-access` et `--allow`. La feuille porte les bandeaux de tous les objets, donc elle reçoit l'union de leurs règles.

**Ce qui ne s'applique pas au document du bandeau.** `--run-script`, `--user-style-sheet`, `--custom-header`, `--username`/`--password` et `--encoding` sont ceux de l'entrée ; wkhtmltopdf en appliquait certains au chargement des en-têtes, nous non. `--javascript-delay` s'applique, le plus long des objets si plusieurs. Un document de bandeau servi en `http` depuis une feuille `file://` est un cadre hors processus pour Chromium : l'événement `load` de la feuille l'attend, mais son trafic réseau et ses polices ne sont pas observés par l'échelle d'attente.

**Écarté.** Imprimer le document du bandeau lui-même, une fois par page, comme wkhtmltopdf le chargeait une fois par page : N navigations et N impressions au lieu d'une. Inliner le HTML du document dans la feuille : `document.location.search` est alors celui de la feuille et le script du manuel ne trouve rien, et les ressources relatives sont perdues. Reproduire la superposition des bandeaux texte et HTML de wkhtmltopdf quand les deux sont écrits sur le même objet : le document remplace le texte.

## D40 — `--dump-outline` : la page du fichier plus le décalage du document, la couverture comptée

**Choix.** L'attribut `page` du XML de `--dump-outline` n'est pas `[page]`. C'est la page **du fichier** — la couverture en est une — à laquelle s'ajoute le `--page-offset` **du document dont l'entrée provient**. `numbering::number` répond pour les bandeaux, `numbering::dump_page` pour le dump : c'est la séparation que #37 annonçait, la fonction unique de V1 ne pouvant plus répondre aux deux.

**Pourquoi.** D36 laissait la question ouverte — page physique ou page affichée — faute de `--page-offset`. Elle est tranchée par la mesure, sur wkhtmltopdf 0.12.6.1 dans `debian:bookworm-slim`, la même image que la capture de `--extended-help` :

* `--page-offset 10` sur deux documents fait passer les `page` du dump de `0 1 1 2 2 3` à `10 11 11 12 12 13` : le décalage traverse le dump, qui n'est donc pas la page physique seule ;
* `cover c.html a.html` écrit `page="2"` pour le premier titre de `a.html`, alors que le pied de page de cette même page imprime `1` : la couverture compte dans le dump et pas dans `[page]`. Le dump n'est donc pas non plus le numéro affiché.

Ce que le dump numérote est la page du fichier, décalée. C'est aussi ce qu'un consommateur en fait : le XML sert à construire une table des matières hors du programme, et le numéro qu'elle imprime doit être celui que le lecteur voit sur la page.

**Écart assumé.** Avec des décalages différents par document, wkhtmltopdf applique à **toutes** les entrées le **dernier `--page-offset` écrit sur la ligne de commande**, quel que soit l'objet auquel il se rattache : `--page-offset 100 a.html --page-offset 5 b.html` décale tout de 5 — le 5 se rattache à `a.html`, et `b.html`, qui hérite du 100, est décalé de 5 quand même — et `--page-offset 7 a.html b.html --page-offset 0` ne décale rien. Ce n'est pas une règle, c'est un décalage unique écrasé à chaque occurrence et lu après coup. Nous appliquons à chaque entrée le décalage de son document. Les deux répondent la même chose dans le cas courant — un seul `--page-offset`, écrit en tête, que tous les objets héritent — et le nôtre est celui qui s'accorde avec le pied de page dans les autres.

**Écarté.** Reproduire le dernier décalage écrit, pour l'écart ci-dessus : personne n'en dépend délibérément, et il contredit le numéro imprimé. Reproduire aussi le débordement de wkhtmltopdf sur un décalage négatif — il calcule en `unsigned`, donc tout nombre qui passerait sous zéro déborde, et `--page-offset -3` lui fait écrire `4294967294` là où nous écrivons `-2` — : nous écrivons le nombre négatif. Faire du dump le numéro affiché, couverture non comptée : mesure contraire. Écrire les éléments `item` sans titre que wkhtmltopdf ajoute autour des titres de chaque document, et dont le `page` vaut le début du document moins un : c'est la forme de l'outline (#40), pas la numérotation, et rien ici ne la change.

## D41 — Table des matières : le HTML de la feuille par défaut, écrit directement, et une longueur qui se stabilise

**Choix.** L'objet `toc` est généré, pas transformé. wkhtmltopdf construisait l'outline en XML, le passait dans une feuille XSL et imprimait le résultat ; nous écrivons directement le balisage que sa feuille **par défaut** produisait — même `h1`, mêmes `ul`/`li`/`div`/`a`/`span`, même CSS — à partir des mêmes entrées, avec les quatre `TOC Options` substituées là où cette feuille les portait : `--toc-header-text` dans le `h1`, `--toc-level-indentation` dans `ul ul {padding-left}`, `--toc-text-size-shrink` dans `ul ul {font-size}`, `--disable-dotted-lines` sur le `border-bottom` du `div`. `--xsl-style-sheet` et `--dump-default-toc-xsl` passent sans équivalent.

**Pourquoi pas la transformation.** Les deux chemins ont été essayés avant de choisir. Chromium **fait** la transformation : la feuille par défaut de wkhtmltopdf, appliquée à son XML avec une instruction `<?xml-stylesheet?>`, produit exactement sa table sur le Chrome 153 épinglé. Mais Chrome retire XSLT — avertissement de dépréciation depuis 143, désactivé sur Stable en **158, le 17 novembre 2026** — et D09 fait passer le navigateur du système avant tout autre, donc ce chemin n'a pas un bug mais une date de péremption, et elle tombe avant la V3. Rust n'a pas de moteur XSLT qui vaille la peine d'être lié.

**Les numéros sont ceux du dump (D40).** Mesuré sur wkhtmltopdf 0.12.6.1 : `--page-offset 100 toc a.html` imprime `102` pour le premier titre, et derrière une couverture d'une page le premier titre est `3`. C'est la page du fichier plus le décalage du document, exactement ce que `numbering::dump_page` répond pour `--dump-outline` — une fonction, deux consommateurs.

**La table se liste elle-même.** `toc a.html` imprime « Table of Contents 1 » en tête de sa propre liste : quand wkhtmltopdf construit la liste, la page de la table a déjà été imprimée et porte un `h1`. Reproduit plutôt que corrigé — c'est la ligne que porte tout document déjà migré — et l'entrée est fabriquée à partir du texte d'en-tête plutôt que découverte, pour que la liste ne dépende pas d'une impression de plus.

**La longueur se stabilise.** Une table liste les pages des documents derrière elle, et ses propres pages les décalent : les numéros dépendent de la longueur et la longueur peut dépendre des numéros. Elle est donc construite, mesurée et reconstruite jusqu'à ce que les deux s'accordent, quatre passes au plus. wkhtmltopdf converge de la même façon et exactement : avec 120 titres, sa table fait trois pages et numérote le premier titre `4` ; la nôtre aussi. Une seule passe l'aurait numéroté `2`, et c'est ce que le test de conformance vérifie en revenant à une passe.

**Écarté.** Transformer dans le navigateur (ci-dessus) : élégant, vérifié, et mort en novembre 2026. Lier libxslt : dépendance C dans un projet qui tient son inventaire, travail de compilation croisée pour les binaires musl statiques (#92) et l'image Docker, et XSLT 1.0 là où la feuille de wkhtmltopdf se déclare 2.0. Réserver la place de la table plutôt qu'itérer : il faut connaître le nombre d'entrées et la hauteur d'une ligne pour réserver juste, et se tromper décale tout le document. Ne pas reproduire l'auto-entrée : elle change le contenu visible d'un document migré, ce qui n'est pas de la parité au pixel près.

## D42 — Liens de la table des matières : le navigateur écrit l'annotation, le marqueur devient la destination

**Choix.** Une entrée de la table est un `<a href>` dont la cible est un marqueur — `rchtmltopdf-contents:PAGE,LEFT,TOP`, la page du fichier fini et l'endroit où le titre se trouve — et la fusion transforme chaque marqueur en destination explicite une fois que toutes les pages ont un numéro. `--disable-toc-links` n'écrit pas de `href` du tout : le navigateur n'écrit alors aucune annotation, plutôt qu'une annotation qui ne mène nulle part.

**Pourquoi passer par le navigateur.** Seul lui sait où une ligne de la table a atterri sur la page, donc l'annotation doit être celle qu'il écrit pour un `<a href>`. Et seule l'impression sait où un titre a atterri : c'est la destination que Chromium écrit dans l'outline, `[page /XYZ left top 0]`, qui est reprise telle quelle. Aucune ancre n'est plantée dans le document de l'utilisateur — le lien vise le titre exactement, sans y toucher.

**Ni interne ni externe.** Le marqueur est un `URI` jusqu'à la fusion, et `--disable-external-links` l'aurait emporté. C'est le lien du programme, écrit par le programme dans un document qu'il a lui-même engendré : `link_kind` le classe à part, donc ni `--disable-external-links` ni `--disable-internal-links` ne le touchent. Mesuré sur wkhtmltopdf 0.12.6.1, qui fait de même : ses quatre annotations survivent aux deux options et ne disparaissent que sous `--disable-toc-links`.

**Le schéma plutôt qu'une URL relative.** Chromium conserve un schéma inconnu verbatim ; un marqueur qui ressemble à un chemin aurait été résolu contre l'URL du document. Vérifié sur la 153 épinglée, avec les deux formes.

**`--enable-toc-back-links` reste `Planned`.** Un lien retour est une annotation posée sur le titre, donc il faut sa *boîte*, et Chromium ne donne que son point : la destination de l'outline porte `left` et `top`, pas la hauteur ni la largeur. wkhtmltopdf l'obtenait en enveloppant le titre dans un `<a>` avant d'imprimer — ce qui modifie le document de l'utilisateur, et une règle `a { }` de sa feuille de style repeint alors ses titres. C'est probablement pourquoi l'option est désactivée par défaut chez lui. Deux sorties existent, aucune n'est prise ici : approximer la boîte (une bande pleine largeur d'une hauteur supposée, fausse pour un titre long) ou injecter l'ancre (parité exacte, verrue comprise). La ligne par défaut de wkhtmltopdf est servie exactement en attendant.

**Écarté.** Écrire l'annotation nous-mêmes sur la page de la table : il faudrait savoir où chaque ligne a été posée, ce que seul le navigateur sait. Viser le haut de la page plutôt que le titre : deux titres sur la même page deviennent le même lien. Planter des ancres `__WKANCHOR` comme wkhtmltopdf : elles ne servaient qu'à nommer une destination que nous savons désigner directement, et elles touchent au document.

## D43 — API C `libwkhtmltox` : un « peut-être », pas une dette

**Choix.** L'API C n'est pas construite, et ne l'est pas à une date. Elle reste possible — D18 a choisi une licence permissive en partie pour cela — mais elle n'appartient à aucun jalon : elle attend que quelqu'un en ait besoin. #45 sort du jalon V3 et porte l'étiquette `maybe`.

**Pourquoi.** Le critère de réussite du projet est écrit dans le brief : des projets Symfony qui utilisent Snappy produisent leurs PDF habituels avec le nouveau binaire, sans modification. Snappy lance le binaire ; il ne lie rien. L'écosystème PHP entier passe par la ligne de commande, et c'est la ligne de commande que ce projet promet. Les consommateurs de `libwkhtmltox` que #45 nommait — les enrobages C# et Java — ne sont pas ceux que le projet cherche à servir.

**Ce que ça coûterait de le faire quand même.** L'API C est *avec état* et pilotée par rappels, là où ce pipeline est asynchrone et en une passe ; et ses noms de réglages forment un **troisième** vocabulaire, après celui de la ligne de commande et le nôtre. Deux surfaces à tenir en accord pour un public que personne n'a encore réclamé, dans un projet qui tient déjà une table d'options à un binaire réel.

**Ce que « peut-être » veut dire ici.** Pas un refus : rien dans l'architecture ne l'empêche, la séparation `core` / `browser` / `pdf` (D12, D20) reste ce qu'il faudrait pour l'exposer. La décision est de ne pas porter la dette d'une promesse tant que personne n'a dit en avoir besoin — c'est la même règle que #46 vient d'appliquer aux options : une échéance qu'on n'a pas l'intention de tenir est pire que pas d'échéance du tout.

**Écarté.** La garder dans V3 : le jalon se serait fermé sur un point que personne n'avait décidé de faire. La marquer `wontfix` : c'est plus fort que ce qui est su, et la licence a été choisie pour laisser la porte ouverte.

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

## D44 — Un document qui n'est jamais arrivé : exit 1 sous les trois gestionnaires, et le fichier écrit sauf sous `abort`

**Choix.** Une navigation qui échoue — hôte introuvable, connexion refusée — est désormais *rapportée* par `Page::load` dans `LoadReport.document`, avec `navigation_failed`, au lieu d'être levée. C'est `--load-error-handling` qui tranche ensuite : `abort` arrête la conversion et n'écrit rien, comme le veut D14 ; `skip` retire le document et écrit le fichier avec les autres ; `ignore` laisse une **page blanche** à sa place, de sorte que la page suivante garde son numéro. Dans les trois cas le code de sortie est 1, avec `Exit with code 1 due to network error: <Nom>`.

**Pourquoi.** Mesuré sur wkhtmltopdf 0.12.6.1 par le harnais Symfony/Snappy du 2026-09-13 (#32, cas `exit/missing-document-*`), trois documents dont le deuxième pointe sur un hôte qui ne résout pas : sous `skip`, `doc.pdf` fait deux pages et sort en 1 ; sous `ignore`, il en fait **trois**, celle du milieu vide, et sort en 1 ; sous `abort`, rien n'est écrit et il sort en 1. Chez lui `multipageloader.cc` ligne 397 positionne `httpErrorCode` quel que soit le gestionnaire, et seul `abort` interrompt. Ici l'échec était levé depuis `render.rs` avant que le `match` de `convert.rs` ne soit atteint, donc `skip` et `ignore` se comportaient comme `abort` : exit 1 et aucun fichier. Les deux tests de conformité qui couvraient `skip` et `ignore` utilisaient un 404, qui produit un document, et ne voyaient pas le trou.

**Ce que D14 disait, et ce qui en reste.** D14 — « échec du document principal : exit 1, pas de PDF » — reste vrai quand il ne reste rien à imprimer, et « `skip` et `ignore` sortent en 0 » reste vrai pour une **sous-ressource**. Pour un document, non : la phrase est amendée ici. Le PDF n'est plus refusé au seul motif qu'un document sur plusieurs a échoué.

**Non mesuré, donc non touché.** Un document qui arrive *mal* — un 404 avec un corps — garde le comportement actuel : `ignore` imprime le corps du serveur et sort en 0. Le corpus du harnais n'a pas ce cas pour un document ; l'ajouter relève de #32, et le code de sortie suivra la mesure.

**Écarté.** Imprimer la page telle que le navigateur l'a laissée sous `ignore` : Chromium y affiche son propre écran d'erreur, là où wkhtmltopdf laissait du vide. Retirer le document sous `ignore` comme sous `skip` : les deux options ne se distingueraient plus, et la page suivante changerait de numéro. Fabriquer la page blanche dans le crate `pdf` : elle doit avoir le format, les marges et les bandeaux de l'objet qu'elle remplace, que seule l'impression connaît — `about:blank` imprimé avec la même commande les a tous.

## D45 — Une couverture compte dans `[page]` et `[topage]`, et D38 comme D40 s'en trouvent amendées

**Choix.** Les pages d'une couverture comptent dans `[page]` et `[topage]` comme celles de n'importe quel objet. Derrière une couverture d'une page, la première page du document imprime `2` et le total en tient compte. Ce qu'une couverture n'a pas, c'est le bandeau : `cover` vide l'en-tête et le pied avant de lire ses propres options, donc le numéro qu'elle porte n'est imprimé nulle part. Le drapeau `counted` de `numbering::Part` et de `plan::Numbering` disparaît : plus rien ne le mettait à `false` qu'une mesure fausse.

**Pourquoi.** Mesuré par le harnais Symfony/Snappy du 2026-09-13 (#32) sur wkhtmltopdf 0.12.6.1, cas `multidoc/cover-and-the-page-count` : `--footer-center 'PAGE=[page] OF=[topage]' cover cover.html doc.html` donne un fichier de quatre pages où la couverture ne porte rien et les trois suivantes impriment `PAGE=2 OF=4`, `PAGE=3 OF=4`, `PAGE=4 OF=4`. Nous imprimions `PAGE=1 OF=3`. Même résultat dans `toc/behind-a-cover`, où la table derrière la couverture imprime `PAGE=2 OF=4`. Chez wkhtmltopdf le mot-clé `cover` ne fait que vider les bandeaux et poser `includeInOutline = false` (`pdfcommandlineparser.cc` lignes 176–180) ; `pagesCount` garde sa valeur par défaut `true` (`pdfsettings.cc` ligne 406) et la boucle des bandeaux incrémente le numéro pour tout objet dont `pagesCount` est vrai (`pdfconverter.cc` ligne 465).

**Ce que D38 et D40 disaient.** D38 écrivait « une couverture ne compte pas » dans son paragraphe *Numérotation*, et D40 tirait de la même observation sa deuxième puce : « le pied de page de cette même page imprime `1` », d'où la séparation entre `number` et `dump_page`. La première affirmation est fausse, la seconde décrivait un écart qui n'existe pas. Ce qui reste de D40 est intact et c'est l'essentiel : le dump numérote la page **du fichier** plus le décalage du document. Les deux fonctions répondent désormais la même chose ; elles restent deux parce que ce sont deux questions, et que #44 n'a pas fini de trancher le décalage entre documents.

**Ce que cela change ailleurs.** `--dump-outline` ne bouge pas. La table des matières ne bouge pas : elle imprime déjà le numéro du dump (D41), qui comptait déjà la couverture — c'est le pied de page qui la rejoint, pas l'inverse.

**Écarté.** Garder `counted` pour un usage futur : rien sur la ligne de commande de wkhtmltopdf ne met `pagesCount` à `false`, donc le drapeau n'aurait décrit personne. Fondre `number` et `dump_page` en une fonction maintenant qu'elles s'accordent : deux consommateurs, deux questions, et une mesure à venir (#44) peut les séparer à nouveau ; un test tient l'accord tant qu'il dure.

## D46 — `[title]` est le titre du document, `[doctitle]` celui du fichier, et c'est une mesure de plus que le plan ne peut pas faire

**Choix.** `[title]` imprime le `<title>` du document sur lequel le bandeau est dessiné ; `[doctitle]` imprime le titre du fichier produit, c'est-à-dire `--title`, ou à défaut le titre du premier document. Les deux sont lus **après l'impression**, dans le dictionnaire Info que Chromium écrit dans chaque partie imprimée : `rchtmltopdf_pdf::title` les rend, la conversion remplit `Context::document_title` et `Context::title` partie par partie, et le plan laisse le premier vide.

**Pourquoi.** Mesuré par le harnais Symfony/Snappy du 2026-09-13 (#32, cas `numbering/document-placeholders`) : avec `--title 'The title given on the command line'` sur un document dont le `<title>` est « The title element of the document », wkhtmltopdf imprime `TITLE=The title element of the document` et `DOCTITLE=The title given on the command line`. Nous imprimions l'option dans les deux. Chez lui `pdfconverter.cc` lignes 590-591 : `[title]` vaut `object.page->mainFrame()->title()`, `[doctitle]` le titre du document produit, choisi lignes 409-415 — `--title`, sinon le titre du premier objet qui n'est pas une table des matières, exactement la partie dont la fusion reprend le dictionnaire Info (#107).

**Ce que ça fait au plan (D27).** Le plan décrit tout ce qu'une conversion ferait, sans rien démarrer, et rien au-dessus de lui ne décide. Le titre d'un document n'est pas une décision : c'est une **mesure**, au même titre que la hauteur d'un bandeau qui est un document (D39), et elle n'existe pas avant que le navigateur ait chargé quoi que ce soit. Le plan garde donc ce que la ligne de commande dit — `--title` remplit `Context::title`, donc l'option continue de changer le plan et le test de D27 la tient toujours — et la conversion complète les deux champs entre l'impression et le dessin des bandeaux. **Deuxième exception, et le même motif** : une mesure entre dans le plan par un argument, jamais par une lecture des réglages en aval.

**Écarté.** Lire le `<title>` dans la page avant l'impression, par `Runtime.evaluate` : un appel de plus par document pour une chaîne que l'impression écrit déjà, et qui serait lue à un moment où le document peut encore changer. Faire de `[title]` un synonyme de `[doctitle]`, comme avant : mesure contraire, et le cas à plusieurs documents — chaque page nommant le sien — est précisément celui qu'un migrant retrouve dans ses pieds de page.

## D47 — `[section]` nomme le **premier** titre qui commence sur la page, et le cache ne se remet jamais à zéro

**Choix.** Pour chacun des trois niveaux, `[section]`, `[subsection]` et `[subsubsection]` nomment le **premier** titre de ce niveau qui commence sur la page. Une page où aucun titre du niveau ne commence garde ce que la page précédente nommait — et non le dernier titre écrit avant elle. Rien ne se remet à zéro au passage d'un document au suivant : un titre du premier document reste en vigueur dans le second tant que celui-ci n'en apporte pas un du même niveau. D38 disait le contraire sur les deux points.

**Pourquoi.** Mesuré par le harnais Symfony/Snappy du 2026-09-13 (#32), cas `numbering/sections` : sur un document dont chaque page porte un `h1`, deux `h2` et leurs `h3`, wkhtmltopdf imprime `SECTION=Chapter 1 SUB=Topic 1.1 SUBSUB=Point 1.1.1` là où nous imprimions `SUB=Topic 1.2 SUBSUB=Point 1.2.2`. Chez lui, le deuxième titre d'un niveau sur une page n'est nommé nulle part.

**La règle est un cache, et sa forme explique les deux moitiés.** `outline.cc` 0.12.6, `OutlinePrivate::buildHFCache` lignes 275–286 : `hfCache[niveau]` est **une** liste indexée par la page, commune à tout le fichier, qui commence par un `NULL` en position zéro. Les titres sont parcourus dans l'ordre de lecture ; pour chacun, la liste est d'abord comblée jusqu'à la page précédente en répétant son dernier élément (`push_back(hfCache[level].back())`), puis le titre n'est ajouté **que si la case suivante est exactement la sienne** (`if (hfCache[level].size() == page)`). Un deuxième titre du même niveau sur la même page trouve donc la case déjà prise et n'est écrit nulle part : c'est le premier qui gagne. Et le comblement recopie la valeur de la page précédente, pas le dernier titre rencontré : une page sans titre hérite du **premier titre de la dernière page qui en avait un**. `Outline::fillHeaderFooterParms` lignes 294–321 comble de la même façon jusqu'à la page demandée et lit `hfCache[0..2][page]`, `NULL` donnant la chaîne vide.

**Ce que D38 disait.** Son paragraphe *Numérotation* écrivait « le dernier titre du niveau sur la page ou avant elle, dans le même document ». Les deux moitiés sont fausses : ni le dernier, ni borné au document. La page 1 des mesures le montre pour la première ; pour la seconde, le `page` que le cache indexe est `j->page + prefixSum[j->document]` (ligne 279), une page du **fichier**, et le comblement de la ligne 281 ne sait rien des frontières entre documents.

**Mesuré d'un côté, lu de l'autre.** Le corpus du harnais n'a pas de cas à plusieurs documents portant des titres, donc la moitié « rien ne se remet à zéro » vient de la lecture du source et non d'une mesure. Elle est tenue par `conformance/tests/numbering.rs: a_heading_stays_in_force_into_the_next_document`, qui fige notre comportement en attendant qu'un cas du harnais la mesure — l'ajouter relève de #32, comme le reste du décalage entre documents (#44).

**Écarté.** Garder « le dernier titre sur la page ou avant elle » parce que la phrase se lit mieux et que le résultat paraît plus juste : c'est une règle que nous aurions inventée, et un pied de page migré change de texte sans prévenir. Remettre le cache à zéro à chaque document, ce que D38 supposait : rien dans `buildHFCache` ne le fait, et un document sans titre de niveau 2 afficherait un `[subsection]` vide là où wkhtmltopdf en affiche un. Reproduire le cache tel quel, en liste de pages : trois chaînes reportées d'une page à l'autre disent la même chose, et `number` parcourt déjà les pages dans l'ordre.

## D48 — Ce que stderr dit quand un chargement échoue : la ligne de la requête, la ressource ignorée, et la ligne de sortie partout

**Choix.** Trois ajouts, tous mesurés contre le binaire réel.

1. **La requête elle-même est nommée**, avant que le gestionnaire ne dise quoi en faire : `Failed to load <url>, with network status code <n> and http status code <n> - <Nom>`, pour le document, sous `abort` comme sous `skip` et `ignore`. C'est la seule ligne qui porte l'adresse sous les trois, et les deux nombres distinguent un serveur qui a refusé d'un serveur qui n'a jamais été joint. Elle sort à tous les niveaux sauf `none` : `LogLevel::shows_errors` est ajouté pour cela, wkhtmltopdf l'écrivant par `error()` et non par `warning()`.
2. **Une sous-ressource perdue sous `ignore` est nommée**, comme sous `skip`. `ignore` est le défaut de `--load-media-error-handling`, et il ne disait rien du tout : un document qui s'imprime sans sa feuille de style est le plus difficile des échecs à diagnostiquer, précisément parce qu'il s'imprime.
3. **`Exit with code 1 due to network error: <Nom>` termine désormais toute sortie en 1 causée par un chargement**, y compris sous `abort` et y compris quand le document manquait sur le disque. Elle est écrite une fois, par le point d'entrée, à partir de `ConvertError::network_error` : un seul endroit, donc pas de course entre deux sites qui l'écriraient tous les deux.

**Pourquoi.** Harnais Symfony/Snappy du 2026-09-13 (#32), cas `exit/missing-document-abort`, `exit/missing-document-skip`, `exit/missing-document-ignore`, `exit/main-document-missing`, `exit/failing-media-default-is-ignore`. Chez wkhtmltopdf, `multipageloader.cc` 0.12.6 : lignes 410–411 pour la ligne de la requête, lignes 304–309 pour la remarque du gestionnaire (`Failed loading page <url> (skipped)`, que nous écrivions déjà mot pour mot), lignes 422–425 pour la sous-ressource sous `skip` et `ignore`. Sur `exit/missing-document-skip` le binaire de référence écrit les trois lignes l'une après l'autre ; nous en écrivions deux.

**Ce qui n'est pas reproduit, et pourquoi.** La fin de la ligne de requête est chez Qt une phrase — `Host x not found` — produite par `QNetworkReply::errorString()` ; elle est intraduisible sans Qt, donc le nom de l'erreur prend sa place, celui-là même que porte la ligne de sortie. Les préfixes `Error:` et `Warning:` de wkhtmltopdf restent `rchtmltopdf: ` et `rchtmltopdf: warning: `, comme partout ailleurs : un script qui cherche `Failed to load` le trouve dans les deux cas, et une ligne sur deux préfixée autrement serait une incohérence sans contrepartie. Enfin, pour un fichier absent du disque nous nommons `ContentNotFoundError` là où wkhtmltopdf nomme `HostNotFoundError` : il n'y arrive qu'en analysant `/build/fixtures/x.html` comme l'URL `http://build/fixtures/x.html` et en ne résolvant pas l'hôte `build`. C'est son artefact, pas sa règle, et le copier reviendrait à annoncer une panne de DNS pour un fichier manquant.

**Ce que ça change au modèle.** `InputError` porte un `NetworkError` : wkhtmltopdf allait chercher un fichier local par la même pile que le réseau, donc un échec de lecture a toujours eu un nom dans le vocabulaire que les applications lisent (D14). `render::Failed` porte le statut HTTP — zéro quand rien n'a répondu — parce que la ligne de requête l'imprime à côté du code Qt. Et `ConvertError::DocumentFailed` remplace la `Error::Navigation` que `abort` levait : il portait la panne sous forme de phrase, il la porte maintenant telle quelle, ce qui permet à la ligne de sortie de la nommer.

**Écarté.** Reproduire les phrases de Qt, par une table de nos propres traductions : trois cents chaînes à inventer et à maintenir pour une fin de ligne que personne n'analyse, alors que le nom qui la remplace est déjà le contrat. Écrire la ligne de sortie depuis chaque site qui décide d'un échec : elle est apparue deux fois dans un premier essai — une fois pour le média sous `abort`, une fois pour le document — et la sortir du point d'entrée la rend unique par construction. Laisser `ignore` muet pour ne pas bavarder sur stderr : c'est le défaut, donc c'est précisément le cas où personne ne saura pourquoi le PDF est nu.

## D49 — Ce qui décide du code de sortie quand une sous-ressource ne charge pas : l'extension, comme chez wkhtmltopdf, et un fichier refusé est toujours une erreur réseau

**Choix.** Deux règles, toutes deux celles de wkhtmltopdf 0.12.6.

1. **Un « média » est une requête dont l'URL porte l'une de six extensions** — `css`, `js`, `png`, `jpg`, `jpeg`, `gif` — et c'est la seule chose que `--load-media-error-handling` juge. Toute autre requête qui échoue est une **erreur réseau** : la ligne de requête de D48 sur stderr, exit 1 et `Exit with code 1 due to network error: <Nom>`, quel que soit l'un ou l'autre gestionnaire, le PDF écrit. Cela couvre le document d'un cadre (`.html`), une police (`.woff2`), un `.svg`, une requête sans extension. La règle est reproduite telle qu'elle est écrite, `QFileInfo::completeSuffix` compris : le suffixe est tout ce qui suit le **premier** point du dernier segment de l'URL, en minuscules, la chaîne de requête retirée ; `jquery.min.js` a pour suffixe `min.js` et n'est donc pas un média. Elle vit dans `core`, fonction pure sur la chaîne de l'URL (`is_media_file`), testée sans navigateur.
2. **Un fichier local refusé par la règle de D10 est une erreur réseau**, sur le document comme sur un bandeau-document, quelle que soit son extension : exit 1, le PDF écrit, une ligne par fichier refusé, et la ligne de sortie nomme `ContentAccessDenied`. Aucune option ne l'éteint, sinon accorder l'accès.

**Pourquoi.** Harnais Symfony/Snappy des 2026-09-13 et 2026-09-14 (#32, cas `exit/failing-frame-default-abort` et `rendering/local-assets-denied`, #112) : wkhtmltopdf sort en 1 avec le PDF écrit dans les deux cas, nous sortions en 0, et Snappy ne levait plus. Chez lui, `multipageloader.cc` 0.12.6, `ResourceObject::amfinished` : toute requête en échec est classée par `completeSuffix()`, en minuscules, `?…` retiré, contre `mediaFilesExtensions` (`loadsettings.cc`) ; hors liste, `httpErrorCode` est positionné sans qu'aucun gestionnaire soit consulté, et `handleError` (`utilities.cc`) sort en 1 dès que ce code est non nul, conversion réussie ou non (`wkhtmltopdf.cc`, ligne 238). Le code est aussi collecté depuis le chargeur de la table des matières et ceux des bandeaux (`pdfconverter.cc`). Un fichier bloqué est remplacé par une requête vers `about:blank`, que Qt échoue en `ProtocolUnknownError` ; sans extension, ce n'est pas un média, d'où l'exit 1. Ici, `render.rs` rangeait tout ce qui n'est pas la navigation dans `media`, jugé par `--load-media-error-handling`, `ignore` par défaut, et un refus n'était qu'un warning. Une application Snappy qui avait oublié `--enable-local-file-access` recevait une exception avec wkhtmltopdf, et livrait en silence un PDF sans style avec nous.

**Ce que D14 disait, et ce qui en reste.** « Échec d'une sous-ressource : `abort` écrit le PDF mais sort en 1 […] ; `skip` et `ignore` sortent en 0 » reste vrai pour un **média** au sens ci-dessus, et pour lui seul. `--load-media-error-handling` n'a jamais jugé que ces six extensions.

**Le nom sur la ligne de sortie.** `ContentAccessDenied` pour un fichier refusé, là où wkhtmltopdf écrit `ProtocolUnknownError`. C'est le nom de l'échec réel, celui que la variante porte déjà pour « un fichier que la règle n'autorise pas », et le `ProtocolUnknownError` de wkhtmltopdf est l'artefact de son remplacement par `about:blank`, pas sa règle : le raisonnement de D48 pour `ContentNotFoundError`. Le harnais signalera la différence de nom ; elle est attendue.

**Ce qui change sur stderr.** Un fichier refusé n'est plus dit deux fois. Depuis D48 le navigateur le rapportait aussi comme un chargement échoué et les deux lignes sortaient ; la ligne de refus, qui porte le remède, reste, et l'autre est retirée par URL.

**Non mesuré, donc dit ici plutôt que fait.** Les conséquences de la règle au-delà des deux cas mesurés — police, svg, `min.css`, requête sans extension — sont lues dans la source et non mesurées par le harnais ; les mesurer relève de #32. Un document distant qui lit un fichier local est refusé par Chromium avant toute requête, donc rien n'est rapporté et la conversion sort en 0, là où la source de wkhtmltopdf dit exit 1 ; le harnais n'a pas ce cas. Les médias d'un bandeau-document ne sont pas collectés du tout, refus mis à part ; c'est un manque distinct.

**Écarté.** Ne traiter que les deux cas mesurés, le document d'un cadre par son type de ressource Chromium et le refus : c'est reproduire l'écart que #112 signale, un silence là où wkhtmltopdf lève, pour les polices et les svg cette fois. Classer par type de ressource Chromium (`Stylesheet`, `Script`, `Image`) plutôt que par extension : plus propre, mais ce n'est pas la règle contre laquelle les applications ont été réglées, et `jquery.min.js` changerait de camp. Copier `ProtocolUnknownError` : voir plus haut.

---

## D50 — La version de `Cargo.toml` commande la publication ; macOS entre dans la matrice

**Choix.** Une release n'est plus déclenchée par une étiquette posée à la main. Le workflow `release.yml` tourne à chaque fusion sur `main`, lit `version` dans le manifeste de l'espace de travail, et publie cette version si aucune étiquette `v<version>` n'existe encore : quatre archives, l'image Docker et son manifeste, la release GitHub avec un `SHA256SUMS`, et l'étiquette elle-même, posée par `gh release create` sur le commit de fusion. Une fusion qui ne touche pas la version ne construit rien. Une étiquette poussée à la main reste acceptée, pour republier, mais elle doit correspondre au manifeste ou rien n'est construit. Le job `version` de la CI refuse sur une pull request une version qui recule sous la dernière release, et annonce ce que la fusion publiera. La matrice de D32 gagne deux cibles macOS, `aarch64-apple-darwin` construit et exécuté sur un runner Apple silicon, `x86_64-apple-darwin` compilé en croisé depuis le même runner. Les archives ne portent plus de version dans leur nom, `rchtmltopdf-linux-x86_64.tar.gz` et non `rchtmltopdf-0.1.0-x86_64-unknown-linux-musl.tar.gz` ; la version est sur le répertoire qu'elles contiennent. La première version publiée est 0.1.0.

**Pourquoi.** Le workflow de D32 existait depuis le 12 septembre et n'avait jamais publié : il attendait une étiquette que personne ne posait, et le dépôt s'est retrouvé public, avec un README qui explique comment installer un navigateur pour un binaire qu'on ne pouvait pas télécharger. Le geste manuel est précisément celui qu'on oublie, et il est aussi celui qui peut dériver : une étiquette `v0.2.0` sur un manifeste qui dit encore 0.1.0 publie des binaires qui annoncent une autre version que la leur. Faire commander le manifeste supprime le geste et la dérive à la fois ; c'est le modèle éprouvé sur refrain, repris tel quel, y compris ses trois gardes. Le nom d'archive sans version est ce qui rend `releases/latest/download/…` utilisable dans un README, un Dockerfile ou un script d'installation, sans le réécrire à chaque version. macOS est dans la matrice parce que le README documente `brew install --cask chromium` et que D09 résout les bundles `.app` : c'est une plateforme que le programme prend en charge, et les runners macOS sont gratuits sur un dépôt public (D28). L'exception à « chaque binaire s'exécute sur son architecture » est l'archive Intel : un runner Intel se paie et Apple n'en fabrique plus, et le README dit lequel des quatre n'a été exécuté par personne.

**Ce que D32 garde.** Tout le reste : la matrice écrite à la main sans `cross` ni `cargo-dist`, les runners natifs, le `tar` pour le lien symbolique, la vérification que le binaire musl est réellement statique et que le lien survit à l'archivage, et le workflow qui tourne sans publier sur toute pull request qui pourrait le casser.

**Ce que ça change à D28.** Le README ne dit plus « pas de release » ; il dit que les releases sont coupées quand ça arrange, sans support, et le reste de l'avertissement tient en entier. `SECURITY.md` dit qu'un correctif atterrit sur `main` et devient une release quand la version est levée, jamais un rétroportage.

**Écarté.** Poser l'étiquette `v0.0.1` à la main pour publier ce qui existait déjà : ça règle le symptôme et garde le geste. Un workflow qui pousse une étiquette avec le jeton du dépôt puis attend le run qu'elle déclenche : le jeton ne déclenche aucun run, c'est une règle de GitHub, et la contourner demande un jeton personnel à faire tourner. Signer ou notariser les binaires macOS : un compte développeur payant pour un projet de week-end ; `xattr -d com.apple.quarantine` est documenté à la place. Émuler un Mac Intel : rien à émuler avec, et un binaire non exécuté est dit tel plutôt que déguisé.

---

## D51 — `--page-offset` est un décalage unique pour toute la sortie, et `[frompage]` est la première page de cette sortie

**Choix.** `--page-offset` cesse d'être une option d'objet. C'est un nombre unique pour la conversion : écrit n'importe où sur la ligne, le dernier écrit l'emporte, et il décale `[page]`, `[topage]`, `[frompage]` et l'attribut `page` de `--dump-outline` — jamais `[sitepage]` ni `[sitepages]`. Le champ quitte `ObjectSettings` pour `GlobalSettings`. Et `[frompage]` n'est pas le début du document dont la page provient : c'est `décalage + 1`, la première page de la sortie, donc le même nombre sur toutes les pages. Sa place dans la table reste `Scope::Object`, qui dit où l'option peut être écrite (D26) et non où sa valeur atterrit : l'aide de wkhtmltopdf la range parmi les options de page, et elle est acceptée partout.

**Pourquoi.** Mesuré sur wkhtmltopdf 0.12.6.1 dans `debian:bookworm-slim`, la même image que la capture de `--extended-help`, sur deux documents de deux pages, pied de page `[page]|[topage]|[frompage]|[sitepage]|[sitepages]` :

* sans décalage : `1|4|1|1|2`, `2|4|1|2|2`, `3|4|1|1|2`, `4|4|1|2|2`. `[frompage]` vaut `1` sur les pages du second document, pas `3` ;
* `--page-offset 100` écrit **après** le second document : `101|104|101|…` sur les quatre pages, le premier document compris ;
* `--page-offset 10 a.html --page-offset 100 b.html` : `101|104|101|…` partout, le dernier écrit ayant tout emporté.

La source dit la même chose et dit pourquoi : `pageOffset` est un membre de `PdfGlobal` et non de `PdfObject` (`pdfsettings.hh`), et `outline.cc` remplit `frompage` avec `off+1`, `topage` avec `off+pageCount` et `page` avec `page+off`, sans jamais regarder l'objet. Le rangement de l'aide parmi les options de page est un classement de documentation, pas un stockage.

**Ce que D40 disait, et pourquoi c'est renversé.** D40 assumait l'écart : appliquer à chaque entrée le décalage de son document, parce que « le nôtre est celui qui s'accorde avec le pied de page ». Cet argument reposait sur une numérotation de bandeau elle-même mesurée à côté — D45 a corrigé la couverture — et il tombe entièrement ici : le pied de page applique lui aussi un décalage unique, donc s'accorder avec lui, c'est faire comme wkhtmltopdf. Il ne restait plus qu'un écart sans raison, et une option d'objet dont la valeur ne décrivait aucun objet. `numbering::dump_page` n'a plus besoin des parts : c'est une addition.

**Ce que la table des matières et le dump deviennent.** Rien ne change pour eux qu'un décalage écrit tardivement, qui les atteint désormais aussi. La table imprime toujours le numéro du dump (D41), et les deux fonctions de `numbering` répondent toujours la même chose (D45).

**Mesuré et non reproduit : la couverture de plusieurs pages.** wkhtmltopdf ajoute une couverture à ses compteurs d'outline par `addEmptyWebPage`, qui compte **une** page quelle que soit la longueur réelle de la couverture, alors que `[page]` compte ses pages une à une. `cover three.html one.html`, trois pages de couverture puis une de document, imprime `[page]` = 4 et `[topage]` = 2 sur la dernière page : un numéro de page supérieur au total, et un dump dont les préfixes sont faux d'autant. C'est un défaut, au même titre que le débordement en `unsigned` d'un décalage négatif (D40) : chez nous une couverture compte ses pages partout, et `[page]` ne dépasse jamais `[topage]`.

**Ce qui était déjà juste et qui est maintenant tenu par un test.** Une table des matières compte dans `[page]` et forme son propre repère de site — `1|5|1|1|1` sur la table, `2|5|1|1|3` sur la page derrière elle — et une couverture aussi : le document qui la suit repart à `1` dans le repère du site. #44 posait ces deux questions ; la mesure dit oui aux deux, et la conformité les garde.

**Écarté.** Garder le décalage par document en le documentant comme une amélioration : c'est un écart silencieux sur une ligne de commande banale — `--page-offset` écrit après l'entrée — et un migrant ne le lit nulle part, il le découvre dans ses pieds de page. Faire de `--page-offset` une option `Scope::Global` dans la table : `reference_help.rs` tient la table à l'aide du binaire réel, section comprise, et l'option y est une option de page ; la portée dit la place, pas la destination. Fondre `number` et `dump_page` maintenant qu'elles ne peuvent plus diverger : l'argument de D45 tient, deux questions restent deux fonctions, et un test tient leur accord. Reproduire le défaut de la couverture de plusieurs pages : il contredit `[page]`.

## D52 — `--dump-outline` écrit un `item` par objet autour de ses titres, et le titre d'un document se lit dans la page, pas dans le fichier imprimé

**Choix.** Le XML de `--dump-outline` porte **un `item` par objet** de la sortie — page, couverture, table des matières — dans l'ordre où ils ont été écrits, et les titres de chaque objet sont imbriqués sous le sien. L'item d'un objet a pour `title` le `<title>` du document, ou le libellé de la table des matières (`--toc-header-text`), ou rien du tout pour un objet que l'outline laisse dehors — une couverture, un `--exclude-from-outline` —, qui garde son item et perd ses titres ; et pour `page` le nombre de pages **avant** lui, plus `--page-offset` (D51) : `0` pour le premier, un de moins que la page où il commence. Les signets du fichier ne changent pas : ils restent la suite plate des titres, comme dans les fichiers de wkhtmltopdf. `outline::dump` construit l'arbre à partir de l'outline fusionné, en rangeant chaque titre de premier niveau sous l'objet dont les pages le portent, et `numbering::dump_object` numérote l'item.

Et le titre d'un document se lit **dans la page**, par `document.title` une fois la page arrivée et les scripts exécutés, et non plus dans le dictionnaire Info de la partie imprimée : `LoadReport::title` le rapporte, la conversion le garde avec chaque partie, et `[title]`, le repli de `[doctitle]`, le titre du fichier produit et l'item du dump en font tous le même usage. Le titre du fichier est désormais écrit dans tous les cas — `--title`, sinon celui du premier document qui n'est pas une table des matières —, et un document sans `<title>` laisse un titre vide.

**Pourquoi.** Mesuré sur wkhtmltopdf 0.12.6.1 dans `debian:bookworm-slim`, la même image que la capture de `--extended-help` (#127) :

* `three.html` (trois `h1`, un par page) puis `one.html` : `Three` à `0` sur `Alpha 1, Beta 2, Gamma 3`, puis `One` à `3` sur `Solo 4`. Nous écrivions les quatre titres à plat, et un consommateur qui parcourt `outline/item` obtenait une autre forme d'arbre à chaque conversion, un seul document compris ;
* `--exclude-from-outline three.html` : une seule ligne, `title="" page="0"` ;
* `toc three.html` : `Table of Contents` à `0` sur son propre `h1` à `1`, puis `Three` à `1` sur `2 3 4` ;
* `cover one.html toc three.html` : `"" 0`, `Table of Contents 1`, `Three 2`.

La source dit pourquoi : `outline.cc` tient un `OutlineItem` racine par objet (`addWebPage`, `addEmptyWebPage`), dont `value` est `mainFrame()->title()` — `captionText` pour une table —, et `dump` les écrit tous quand `printOutline` ne parcourt que leurs enfants. Le `page` de la racine vaut `prefixSum[document]`, le nombre de pages avant l'objet.

Le titre, lui, est une mesure de plus (D46), et la mesure de D46 était faite au mauvais endroit. Chromium n'écrit pas seulement le `<title>` dans le dictionnaire Info : quand il n'y en a pas, ou qu'il est vide, il y met **l'URL** — `notitle.html` pour un fichier, `127.0.0.1:18765/dir/page.html?x=1` pour une adresse, le nom du fichier temporaire pour l'entrée standard —, et rien dans le dictionnaire ne distingue ce repli d'un titre qui dirait la même chose. wkhtmltopdf, sur le même document sans `<title>`, imprime `T=|D=` pour `T=[title]|D=[doctitle]` et laisse le titre du fichier vide (`pdfinfo` : `Title:` nu). Nous imprimions `notitle.html` aux deux, et le fichier le portait. `document.title` répond exactement ce que `mainFrame()->title()` répondait, au même moment.

**Ce que D40 disait.** Son *Écarté* renvoyait les items sans titre « à la forme de l'outline (#40), pas la numérotation ». #40 est fermé sans qu'elle ait été construite ; c'est fait ici, et la numérotation de D40 et D51 s'y applique sans changement.

**Mesuré et non reproduit.** Un objet que l'outline exclut compte **une** page dans les items qui le suivent, quelle que soit sa longueur : `addEmptyWebPage` fait `pageCount += 1` sans regarder. `three.html --exclude-from-outline one.html` chez lui écrit `One page="1"` là où le second document commence page 4. C'est le compteur que D51 relève déjà pour une couverture de plusieurs pages, et la même raison : nos items comptent les pages réelles.

**Écarté.** Reconnaître le repli de Chromium dans le dictionnaire Info et l'effacer quand il ressemble à l'URL : la forme du repli dépend de l'origine du document, et un titre qui nomme le fichier serait effacé avec lui. Laisser le titre du fichier à ce que Chromium a écrit quand `--title` manque, comme avant : c'est là que le repli atteignait le lecteur. Reproduire les items sans mettre leurs titres — écrire `title=""` partout : le dump sert à construire une table des matières hors du programme, et le titre du document y est ce qu'un consommateur attend. Reproduire `--outline-depth` dans le dump comme wkhtmltopdf ne le fait pas : c'est #115, et rien ici ne le change.

## D53 — `--outline-depth` borne les signets du fichier, et rien d'autre

**Choix.** `--outline-depth` ne coupe que l'outline **que le fichier porte**. Le dump de `--dump-outline`, les titres qu'une table des matières liste et ceux que `[section]`, `[subsection]` et `[subsubsection]` nomment voient l'arbre entier, à toute profondeur. `rchtmltopdf_pdf::outline` lit tout, puis décroche du fichier ce qui pend sous la borne : ce qu'elle rend est l'arbre complet, et la coupe est celle du fichier seul. `--outline-depth 0` est un fichier sans signet et un dump qui les a tous.

**Pourquoi.** Mesuré sur wkhtmltopdf 0.12.6.1 dans `debian:bookworm-slim`, la même image que la capture de `--extended-help`, avec `--outline-depth 1` sur un document de trois niveaux (`Alpha` › `Alpha One` › `Alpha One A`, puis `Beta` › `Beta One`), une table des matières devant et un pied de page `S=[section]|SS=[subsection]|SSS=[subsubsection]` : les signets du fichier sont `Table of Contents`, `Alpha`, `Beta` et rien d'autre ; le dump porte les trois niveaux ; la table liste `Alpha One` et `Alpha One A` ; le pied de la première page imprime `S=Alpha|SS=Alpha One|SSS=Alpha One A`. Le harnais Symfony/Snappy du 2026-09-13 (#115) l'avait vu sur le dump seul : 14 items chez lui, 6 chez nous.

La source dit la même chose : dans `outline.cc`, `outlineChildren` — qui écrit les signets — s'arrête à `level + 1 > settings.outlineDepth`, et c'est la seule lecture de `outlineDepth` du fichier. `dumpChildren`, `buildHFCache` et la table, construite sur le dump, n'en tiennent pas compte.

**Ce que D36 disait.** « `--dump-outline` écrit ce que le fichier porte » : c'était la commodité d'une seule passe, et la mesure la contredit. La passe reste une — lire puis couper —, c'est ce qu'elle rend qui change.

**Écarté.** Lire l'outline deux fois, une fois entière pour le dump et une fois coupée pour le reste : deux chemins pour un résultat que la mesure dit unique. Ajouter l'écart à D36 et aux attentes du harnais, comme #115 le proposait en second : un consommateur du dump sous `--outline-depth` est rare, mais la table des matières et les bandeaux ne le sont pas, et la même coupe les atteignait.

## D54 — L'ordre d'extraction d'une table des matières est celui de l'extracteur, pas de la feuille : D41 tient

**Choix.** La feuille de style de la table des matières reste celle que D41 a choisie, `span {float: right;}` compris. L'écart que le harnais Symfony/Snappy relève sur `toc/default`, `toc/options` et `toc/enable-toc-back-links` — `pdftotext` lit chez wkhtmltopdf chaque numéro à côté de son titre, et chez nous les numéros d'une liste groupés après ses titres — est déclaré **attendu** dans le harnais, et n'est pas reproduit.

**Pourquoi.** #122 attribuait l'écart au flux de contenu : Chromium peindrait les flottants d'une liste en groupe, et un extracteur ne verrait que cet ordre. Mesuré sur le même balisage et la même feuille, imprimés par les deux binaires dans `debian:bookworm-slim`, `pdftotext` 22.12.0 :

* en **ordre brut** (`-raw`, l'ordre du flux de contenu), les deux fichiers lisent la même chose : `2 2 2 3 4`, **puis** les titres. Qt peignait les flottants d'abord, exactement comme Chromium. Ce n'est pas là que les deux diffèrent ;
* en ordre de lecture (le mode par défaut, celui du harnais), un titre assez long pour passer à la ligne fait grouper les numéros chez wkhtmltopdf aussi : `Deep One | Section Two … | Chapter Two | 2 | 3 | 4`. L'ordre est celui que l'analyse de mise en page de poppler décide, à partir de la géométrie des mots, et la géométrie des deux moteurs diffère de moins d'un point ;
* le même document avec `div {display: flex; justify-content: space-between;}` à la place du flottant donne chez nous des boîtes de mots **identiques au centième de point** et un ordre de lecture **identique** — alors que son ordre brut, lui, met chaque numéro derrière son titre. Changer la feuille change le flux et ne change pas ce que le harnais lit.

Ce que le harnais compare n'est donc pas une propriété de la table mais une décision de l'extracteur sur une géométrie que la promesse du projet ne couvre pas : une compatibilité fonctionnelle, jamais la parité au pixel près. Tous les mots et tous les numéros sont dans les deux fichiers, et les numéros s'accordent ; c'est ce que les tests de conformance tiennent (`contents.rs`), et c'est ce qu'un consommateur qui lit une table peut attendre.

**Écarté.** Remplacer le flottant par une rangée flex ou une cellule de tableau : mesuré sans effet sur ce que le harnais lit, et une feuille de plus à justifier face à « même CSS ». Ajuster la géométrie jusqu'à ce que poppler groupe comme chez Qt : c'est la parité au pixel près par un autre nom, et poppler n'est qu'un extracteur parmi d'autres. Une assertion de conformance sur l'ordre du texte extrait, comme #122 le demandait : elle testerait l'heuristique de l'extracteur, pas le programme.

## D55 — Un document absent du disque est jugé par `--load-error-handling`, comme un document que le réseau n'a pas rendu

**Choix.** Un chemin qui ne désigne aucun fichier n'est plus refusé avant tout démarrage quel que soit le gestionnaire. Sous `abort`, rien ne change : la conversion s'arrête aussitôt, sans navigateur, avec `could not read <chemin>: no such file` et la ligne de sortie (D48). Sous `skip` et `ignore`, le document est **porté jusqu'à la boucle** avec sa raison — `input::resolve_or_report`, `Resolved::missing` — et y est traité comme une navigation qui a échoué : la ligne de requête le nomme (`Failed to load file://…, with network status code 203 and http status code 0 - ContentNotFoundError`), `skip` l'écarte et écrit les autres, `ignore` laisse une page blanche à sa place (D44), et les deux sortent en 1 avec le fichier écrit. Le nom reste le nôtre : `ContentNotFoundError`, D48 ayant refusé le `HostNotFoundError` que wkhtmltopdf n'obtient qu'en lisant le chemin comme une URL.

**Pourquoi.** Mesuré sur wkhtmltopdf 0.12.6.1 dans `debian:bookworm-slim`, `nope.html one.html` avec `--load-error-handling skip` puis `ignore` : les deux écrivent le PDF — le seul document dans un cas, une page blanche puis le document dans l'autre — et sortent en 1 : `Failed loading page http:///m/nope.html (skipped)`, resp. `(ignored)`, puis `Exit with code 1 due to network error: HostNotFoundError`. Nous n'écrivions rien sous aucun des trois, et disions `could not read` avant même de lancer le navigateur. Le harnais ne l'avait pas vu : ses cas `exit/missing-document-*` manquent un hôte, pas un fichier. La raison chez lui est structurelle — `guessUrlFromString` fait d'un chemin inexistant une URL `http://`, et le chargeur la traite comme les autres —, donc le gestionnaire y voit un document parmi les autres, et un migrant qui écrit `skip` pour qu'une conversion de dix documents survive à un fichier manquant a raison d'y compter.

**Écarté.** Envoyer le chemin au navigateur pour qu'il échoue lui-même : une seconde de lancement pour découvrir ce que `canonicalize` sait déjà, et une page d'erreur de Chromium à la place de la page blanche de D44. Différer aussi sous `abort` : rien ne sera écrit, donc rien à démarrer, et le message dit de quel fichier il s'agit sans attendre. Faire du chemin une URL `http://` comme wkhtmltopdf : c'est l'artefact que D48 a déjà refusé, et il enverrait une requête DNS pour un fichier manquant.

## D56 — `link` et `backLink` du dump portent les noms d'ancre de wkhtmltopdf, bizarreries comprises

**Choix.** Chaque `item` de `--dump-outline` porte dans `link` et `backLink` les noms que wkhtmltopdf y écrivait : `__WKANCHOR_` suivi d'un compteur en base 36 minuscule, **deux par item** en ordre de lecture, l'item de l'objet compris, sur un compteur unique pour toute la conversion. L'ordre est celui de wkhtmltopdf et non celui de la sortie : les objets `page` d'abord, dans l'ordre de la ligne de commande, puis les tables des matières ; un objet que l'outline exclut — couverture, `--exclude-from-outline` — n'a pas d'ancre et ne consomme aucun numéro. Les items d'une table portent le **même nom dans les deux attributs**, le premier de leur paire, le second étant consommé sans être écrit. `outline::dump` nomme après avoir construit l'arbre (D52), et rien d'autre ne lit ces noms : les liens de la table des matières restent ceux de D42.

**Pourquoi.** Mesuré sur wkhtmltopdf 0.12.6.1 dans `debian:bookworm-slim` : `levels.html` (trois niveaux) numérote `0/1` sa racine puis `2/3, 4/5, 6/7, 8/9, a/b` ses titres en pré-ordre ; `one.html toc three.html` donne `One 0/1, Solo 2/3`, puis `Three 4/5` et ses titres jusqu'à `a/b`, et la table **entre les deux** reçoit `c/c` et `e/e` ; `--outline-depth` n'y change rien ; un document exclu écrit `link="" backLink=""`. La source explique chaque trait (`outline.cc` 0.12.6) : `fillAnchors` tire deux numéros par item quand l'item n'a pas de prédécesseur de même forme, `addWebPage` le fait pour chaque page au prétraitement, la table n'est construite qu'ensuite, et sa reconstruction jusqu'à ce que sa longueur se stabilise — toujours au moins une — retombe sur le prédécesseur et copie `other->anchor` dans **les deux** champs (`tocAnchor = other->anchor`, ligne 62). Pourquoi le reproduire : la feuille XSL par défaut de wkhtmltopdf, celle que `--xsl-style-sheet` personnalise et dont tout consommateur du dump part, teste la présence de l'attribut ; un dump aux attributs vides lui fait construire une table sans aucun lien, ce que le harnais mesure depuis sa première exécution.

**Écarté.** Numéroter dans l'ordre de la sortie, table comprise : plus lisible, faux à la mesure. Écrire un nom distinct dans `backLink` d'une table : c'est corriger wkhtmltopdf, et un consommateur qui a appris ses noms sur ses dumps les retrouve tels quels. Faire de ces noms de vraies destinations dans le PDF : D42 lie déjà la table à ses titres par ses propres marqueurs, et le dump n'a jamais promis que ses ancres soient dans le fichier — chez wkhtmltopdf non plus.

## D57 — `--enable-toc-back-links` : une annotation sur la boîte du titre, mesurée avant l'impression, vers la ligne de la table

**Choix.** `--enable-toc-back-links` et `--disable-toc-back-links` passent à `Implemented`. Sous la première, chaque titre d'un objet qui la porte reçoit une annotation de lien, posée sur sa boîte, qui mène à sa ligne dans la table des matières ; le titre de la table elle-même en reçoit une aussi, vers l'entrée qu'elle fait d'elle-même (D41). La boîte est celle de wkhtmltopdf, à une mesure près : sa gauche et son haut sont la destination que Chromium écrit pour le titre dans l'outline (D36, D42), sa droite est le bord droit du contenu de la page, et sa hauteur est celle du titre, lue par `getBoundingClientRect` une fois la page arrivée — `Page::heading_boxes`, appelée seulement quand l'option le demande. L'entrée visée est retrouvée par ce qu'elle vise : son propre lien porte la destination du titre (D42), et le titre dont la page et la position y répondent est renvoyé vers le coin de cette entrée — `rchtmltopdf_pdf::contents_back_links`, sur le fichier fini. Sous `--disable-toc-links` avec les liens retour, la table écrit quand même ses marqueurs pour que ses entrées aient une boîte, et la fusion les lui retire une fois les liens retour posés. `TocSettings::back_links` est hérité par chaque objet comme les autres réglages de table, et c'est celui de l'objet dont les titres sont en cause qui compte, comme chez wkhtmltopdf.

**Pourquoi.** Mesuré sur wkhtmltopdf 0.12.6.1 dans `debian:bookworm-slim`, `--enable-toc-back-links toc levels.html` (trois niveaux, deux pages) : une annotation par titre, `Rect [33 792 561 813]` pour un `h1` — la largeur du conteneur, la hauteur du titre —, dont la destination est l'ancre plantée sur l'entrée ; et une septième sur la page de la table, sur son propre `h1`, vers sa propre entrée. La source le confirme (`pdfconverter.cc` 0.12.6, lignes 856–858) : `painter->addLink(webPrinter->elementLocation(element).second, tocAnchor)` — la boîte de l'élément dans la mise en page imprimée, et **rien n'est ajouté au document**. D42 supposait le contraire, un `<a>` enveloppant le titre qu'une règle `a { }` aurait repeint, et en avait déduit que les deux issues étaient mauvaises ; la mesure enlève la verrue, et ne laisse que la question de la boîte.

**La boîte, et ce qu'elle approxime.** Chromium ne donne que le coin du titre. La hauteur est lue dans la mise en page du viewport (1024 px, D03), qui n'est pas celle de l'impression (environ 718 px CSS en A4) : elles coïncident pour tout titre qui tient sur une ligne, et un titre qui passe à la ligne à l'impression et pas à l'écran reçoit une annotation d'une ligne au lieu de deux. La largeur n'est pas lue du tout — à 1024 elle déborderait du papier — mais courue jusqu'au bord droit du contenu, ce que fait la boîte d'un titre de bloc chez wkhtmltopdf ; un titre dans une colonne étroite a une annotation plus large que lui. Le facteur est 0,75 point par pixel CSS, multiplié par `--zoom`, mesuré à 1 et à 1,5.

**Écarté.** Envelopper le titre dans un `<a>` de notre cru : parité de rectangle exacte, mais c'est toucher au document, et wkhtmltopdf ne le faisait pas. Redimensionner le viewport à la largeur d'impression le temps de la mesure : la hauteur des titres longs y gagnerait, au prix d'un `resize` que les scripts du document verraient, pour un cas rare. Lire la boîte dans le PDF imprimé : il faudrait retrouver le texte du titre parmi les glyphes, ce que rien ici ne sait faire et que D36 a écarté. Viser le haut de la page de la table plutôt que la ligne : deux entrées sur la même page deviennent le même lien, l'argument de D42 en sens inverse.
