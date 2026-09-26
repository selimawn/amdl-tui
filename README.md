# amdl-tui

Interface terminal (Rust + [ratatui](https://ratatui.rs)) pour
[apple-music-downloader](https://github.com/zhaarey/apple-music-downloader).

On colle une URL `music.apple.com`, l'interface récupère les métadonnées depuis
l'API Apple, affiche les pistes d'un album, et lance `amdl` avec la bonne
configuration.

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
| `Entrée` | analyser l'URL saisie / lancer le téléchargement |
| `Esc` | revenir en arrière, ou quitter (champ URL vide) |
| `q` | quitter (sauf sur l'écran de saisie, où `q` s'écrit) |
| **`Ctrl+S`** ou **`F2`** | **démarrer / arrêter colima + wrapper-lite** |
| `↑` `↓` / `k` `j` | naviguer dans les pistes ou les options |
| `Espace` | cocher une piste / changer une option |
| `Tab` | passer des pistes aux options |
| `a` / `n` / `i` | tout cocher / décocher / inverser |

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
| `src/api.rs` | token web Apple + catalogue (`amp-api.music.apple.com`), pagination |
| `src/amdl.rs` | génère un `config.yaml` dédié, construit la commande, streame la sortie |
| `src/stack.rs` | démarrage / arrêt de colima + wrapper-lite |
| `src/app.rs` | machine à états, clavier, messages |
| `src/ui.rs` | rendu ratatui |
| `src/main.rs` | boucle d'événements + modes de diagnostic |

Le token est extrait du bundle JS de `music.apple.com` (2 requêtes + regex),
exactement comme `internal/amp-api/token.go` du downloader. L'ordre des pistes
suit `relationships.tracks.data`, ce qui garantit que les indices envoyés à
`amdl --select` correspondent.

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

Testé le 26/09/2026 sur macOS 27.0 / Apple Silicon, storefront `tr` :

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
