# amdl-tui

Interface terminal (Rust + [ratatui](https://ratatui.rs)) pour
[apple-music-downloader](https://github.com/zhaarey/apple-music-downloader).

On colle une URL `music.apple.com` — **titre, album, playlist ou artiste** —
l'interface récupère les métadonnées depuis l'API Apple, affiche une
arborescence d'albums repliables, et lance `amdl` avec la bonne configuration.

Coller l'URL d'un artiste liste **tous ses albums** (avec pagination) : on
déplie ceux qui nous intéressent, on coche des pistes dans plusieurs albums à
la fois, et le téléchargement s'enchaîne album par album.

## Prérequis

| Élément | Où |
|---|---|
| `amdl` compilé | `~/apple-music-downloader/amdl` |
| `wrapper-lite` (image Docker) | `~/wrapper-lite` → image `wrapper-lite:local` |
| VM `colima` profil `amdl` | x86_64 + Rosetta, 1,5 Go |
| `ffmpeg` | pour la conversion FLAC |
| Toolchain Rust | `cargo`, et `~/.cargo/bin` dans le `PATH` |

## Lancement

Double-cliquer sur `~/Desktop/amdl-tui.command`.

Le script **ne démarre rien** : il recompile si nécessaire puis ouvre
l'interface. C'est l'interface qui indique l'état de `wrapper-lite` et permet de
le piloter.

## Raccourcis

| Touche | Effet |
|---|---|
| `Entrée` | analyser l'URL saisie |
| `Esc` | revenir en arrière, ou quitter (champ URL vide) |
| `q` | quitter (sauf sur l'écran de saisie, où `q` s'écrit) |
| **`Ctrl+S`** ou **`F2`** | **démarrer / arrêter colima + wrapper-lite** |
| `↑` `↓` / `k` `j` | naviguer dans l'arbre ou les options |
| `→` / `Entrée` sur un album | déplier (et charger les pistes) |
| `←` | replier l'album |
| `Entrée` sur une piste | lancer le téléchargement |
| `Espace` | cocher une piste, ou tout l'album si la ligne est un album |
| `Tab` | passer de l'arbre aux options |
| `a` / `n` / `i` | tout cocher / décocher / inverser (tous albums) |
| `e` / `c` | déplier / replier l'album sous le curseur |

> `Fn+S` n'est pas utilisable : la touche `Fn` sert à basculer la rangée de
> fonctions, et l'OS l'envoie au terminal comme un simple `s`. `Ctrl+S` est
> capté avant le champ de saisie, donc il ne s'écrit jamais dans l'URL.

## Formats

Le sélecteur ne propose que ce que l'album propose réellement :

| Format | Effet |
|---|---|
| `ALAC` | `.m4a` sans conversion (qualité max 192 kHz par défaut) |
| `FLAC` | téléchargement ALAC puis conversion `ffmpeg` |
| `Atmos` | `--atmos`, si l'album a des pistes Atmos |
| `AAC` | `--aac --aac-type aac-lc` |

Les fichiers vont dans `~/Desktop/Musiques/<Artiste>/<Album>/`.

## Architecture

| Fichier | Rôle |
|---|---|
| `src/api.rs` | token web Apple + catalogue (`amp-api.music.apple.com`), pagination, arbre d'albums |
| `src/amdl.rs` | génère un `config.yaml` dédié, construit les commandes, streame la sortie |
| `src/stack.rs` | démarrage / arrêt de colima + wrapper-lite |
| `src/app.rs` | machine à états, arborescence, sélection, clavier |
| `src/ui.rs` | rendu ratatui de l'arbre et des options |
| `src/main.rs` | boucle d'événements + modes de diagnostic |

### Le modèle : une arborescence d'albums

`AlbumNode` est l'unité de base — un album avec ses pistes. Un `Item` contient
une liste d'albums, ce qui couvre tous les cas d'un seul coup :

| URL collée | Albums | Pistes |
|---|---|---|
| un titre | 1 nœud | la piste, chargée d'emblée |
| un album | 1 nœud | chargées d'emblée |
| une playlist | 1 nœud | chargées d'emblée |
| **un artiste** | **N nœuds** | **chargées à la demande, au dépliage** |

Un artiste peut avoir des centaines d'albums : charger toutes les pistes serait
des centaines de requêtes. Les pistes ne sont donc récupérées **qu'au premier
dépliage** de l'album (indicateur `⋯` pendant le chargement).

### Deux détails qui comptent

**Les indices de `--select`.** L'ordre des pistes suit
`relationships.tracks.data`, exactement l'ordre que `amdl` utilise pour
construire ses `TaskNum` (1-based). Les positions envoyées sur `stdin` tombent
donc juste.

**Le rendu de l'arbre.** Les lignes sont écrites une par une plutôt qu'avec le
widget `List` : le défilement est ainsi maîtrisé (pour garder le curseur
visible quel que soit le nombre d'albums dépliés) et le style de chaque ligne
— case à cocher, flèche de pliage, badge de qualité — reste entièrement
contrôlé.

### File de téléchargement

Sélectionner des pistes dans plusieurs albums produit **une tâche par album**
(celles vides sont ignorées), exécutées séquentiellement :

- album entièrement coché → pas de `--select`, l'album entier d'un coup ;
- sélection partielle → `--select` avec les positions sur `stdin` ;
- URL `?i=` avec une seule piste cochée → l'URL d'origine, sans `--select`.

## Modes de diagnostic

```bash
BIN=~/amdl-tui/target/release/amdl-tui

# métadonnées + commande générée, sans rien télécharger
$BIN --probe "https://music.apple.com/tr/album/at-the-bbc/1555697650"

# téléchargement réel, sortie brute dans le terminal
$BIN --exec "<url>" --format flac --select "1,3,5-7"

# journal détaillé (dans /tmp/amdl-tui-debug.log)
AMDL_TUI_DEBUG=1 $BIN
```

## Validation

Testé sur macOS 27.0 / Apple Silicon, storefront `tr` :

| Scénario | Résultat |
|---|---|
| Album (38 pistes) → liste complète | ✅ |
| URL `?i=<track>` → une seule piste pré-cochée | ✅ `1/38 cochee(s)` |
| Album + sélection `15,31` → pipe `stdin` | ✅ `Completed: 2/2` |
| Conversion FLAC (ALAC → ffmpeg → `.flac`) | ✅ `Conversion completed in 343ms` |
| Téléchargement lancé depuis l'interface | ✅ |
| `Ctrl+S` arrêt puis redémarrage | ✅ `ok=true` dans les deux sens |
| Ouverture sans rien démarrer | ✅ affiche `HORS LIGNE` |
| ALAC 24-bit/96 kHz, pochette + paroles LRC intégrées | ✅ |
| **Artiste** → tous ses albums, avec pagination | ✅ 187 albums pour Taylor Swift |
| **Dépliage** d'un album → pistes chargées à la demande | ✅ rien n'est coché |
| Cocher / décocher, `a` / `n` / `i` sur plusieurs albums | ✅ |

Non vérifié de bout en bout : l'enchaînement de la **file multi-albums** (une
tâche par album) — testé sur un seul album à la fois pour l'instant.

## Pièges connus

1. **`~/.cargo/bin` doit être dans le `PATH`** du processus `amdl`. Le binding Go
   de Temari cherche `lib/darwin-arm64/libtemari.dylib` alors que le dossier
   livré s'appelle `lib/macos-arm64/` (bug : `platformKey()` utilise
   `runtime.GOOS`, qui vaut `darwin` sur macOS). Il retombe donc sur un
   self-build Rust qui exige `cargo` dans le `PATH`, même si la bibliothèque est
   déjà en cache dans `~/Library/Caches/temari`.
2. **`config.yaml` est lu dans le répertoire courant.** L'interface ne touche
   jamais à `~/apple-music-downloader/config.yaml` : elle écrit sa version dans
   `~/Library/Caches/amdl-tui/` et y lance `amdl`.
3. **`exit-on-error: true`** est forcé, sinon `amdl` attend une touche `Entrée`
   après une erreur et l'interface resterait bloquée.
4. **Quitter pendant un téléchargement** ne tue pas `amdl` : le fichier arrive
   quand même.
5. L'`entrypoint.sh` de la branche `lite` cherche la session au mauvais chemin
   (`data/data/com.apple.android.music/files/mpl_db/kvs.sqlitedb` alors que le
   launcher écrit dans `<base-dir>/mpl_db`) : il redemande un login à chaque
   démarrage. C'est pour ça que `stack.rs` lance le service directement
   (`--entrypoint /app/wrapper-lite-rootless`).
