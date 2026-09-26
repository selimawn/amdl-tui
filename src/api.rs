// Couche API Apple Music : token web + catalogue (album / titre / playlist).
// Le token est extrait du bundle JS de music.apple.com, exactement comme le fait
// internal/amp-api/token.go du downloader.

use regex::Regex;
use serde_json::Value;
use std::sync::OnceLock;
use std::time::Duration;
use ureq::Agent;

const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
(KHTML, like Gecko) Chrome/91.0.4472.124 Safari/537.36";

fn agent() -> &'static Agent {
    static A: OnceLock<Agent> = OnceLock::new();
    A.get_or_init(|| {
        Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .build()
            .into()
    })
}

fn get_text(url: &str) -> Result<String, String> {
    let mut resp = agent()
        .get(url)
        .header("User-Agent", UA)
        .call()
        .map_err(|e| format!("requete {url} : {e}"))?;
    resp.body_mut()
        .read_to_string()
        .map_err(|e| format!("lecture {url} : {e}"))
}

fn get_authed(url: &str, token: &str) -> Result<Value, String> {
    let mut resp = agent()
        .get(url)
        .header("User-Agent", UA)
        .header("Origin", "https://music.apple.com")
        .header("Authorization", &format!("Bearer {token}"))
        .call()
        .map_err(|e| format!("requete catalogue : {e}"))?;
    let txt = resp
        .body_mut()
        .read_to_string()
        .map_err(|e| format!("lecture catalogue : {e}"))?;
    serde_json::from_str(&txt).map_err(|e| format!("JSON invalide : {e}"))
}

/// Token web (JWT) recupere depuis le bundle JS de la page d'accueil.
pub fn get_token() -> Result<String, String> {
    let home = get_text("https://music.apple.com")?;
    let re_js = Regex::new(r"/assets/index~[^/]+\.js").unwrap();
    let js = re_js
        .find(&home)
        .ok_or("bundle JS introuvable dans la page d'accueil")?
        .as_str()
        .to_string();
    let src = get_text(&format!("https://music.apple.com{js}"))?;
    let re_tok =
        Regex::new(r"eyJ[A-Za-z0-9\-_=]+\.[A-Za-z0-9\-_=]+\.[A-Za-z0-9\-_=]+").unwrap();
    re_tok
        .find(&src)
        .map(|m| m.as_str().to_string())
        .ok_or_else(|| "token introuvable dans le bundle JS".to_string())
}

// ---------------------------------------------------------------- URL

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Album,
    Song,
    Playlist,
    Artist,
    MusicVideo,
}

impl Kind {
    pub fn label(&self) -> &'static str {
        match self {
            Kind::Album => "Album",
            Kind::Song => "Titre",
            Kind::Playlist => "Playlist",
            Kind::Artist => "Artiste",
            Kind::MusicVideo => "Clip",
        }
    }
}

pub struct ParsedUrl {
    pub raw: String,
    pub storefront: Option<String>,
    pub kind: Kind,
    pub id: String,
    pub track_id: Option<String>,
    pub base_url: String,
}

/// Analyse une URL music.apple.com. Le storefront peut manquer : on retombe
/// alors sur celui de la config.
pub fn parse_url(raw: &str, fallback_storefront: &str) -> Result<ParsedUrl, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("URL vide".into());
    }
    if !trimmed.contains("music.apple.com") {
        return Err("Ce n'est pas une URL music.apple.com".into());
    }

    let (base, query) = match trimmed.split_once('?') {
        Some((b, q)) => (b.to_string(), q.to_string()),
        None => (trimmed.to_string(), String::new()),
    };

    let segs: Vec<&str> = base
        .split('/')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>();

    let kinds: [(Kind, &str); 5] = [
        (Kind::Album, "album"),
        (Kind::Song, "song"),
        (Kind::Playlist, "playlist"),
        (Kind::Artist, "artist"),
        (Kind::MusicVideo, "music-video"),
    ];

    let mut found: Option<(Kind, usize)> = None;
    for (kind, kw) in kinds {
        if let Some(pos) = segs.iter().position(|s| *s == kw) {
            found = Some((kind, pos));
            break;
        }
    }
    let (kind, pos) = found.ok_or("Type d'URL non reconnu (album, song, playlist, artist)")?;

    let mut storefront = None;
    if pos >= 1 {
        let cand = segs[pos - 1];
        // Un code storefront fait 2 lettres minuscules ; sinon on ignore.
        if cand.len() == 2 && cand.chars().all(|c| c.is_ascii_alphabetic()) {
            storefront = Some(cand.to_lowercase());
        }
    }

    let id = segs.last().unwrap().to_string();
    if id == "album" || id == "song" || id == "playlist" || id == "artist" {
        return Err("Identifiant manquant dans l'URL".into());
    }

    // ?i=<trackId> : URL d'un titre precis dans un album
    let mut track_id = None;
    for part in query.split('&') {
        if let Some(v) = part.strip_prefix("i=") {
            if !v.is_empty() {
                track_id = Some(v.to_string());
            }
        }
    }

    // base_url sans query (utilisable pour --select sur l'album entier)
    Ok(ParsedUrl {
        raw: trimmed.to_string(),
        storefront: Some(storefront.unwrap_or_else(|| fallback_storefront.to_string())),
        kind,
        id,
        track_id,
        base_url: base,
    })
}

// ---------------------------------------------------------------- Modeles

#[derive(Clone)]
pub struct Track {
    pub id: String,
    pub name: String,
    pub track_number: u32,
    pub disc_number: u32,
    pub duration_ms: u64,
    pub traits: Vec<String>,
    pub rating: String,
    pub kind: String,
}

impl Track {
    pub fn is_video(&self) -> bool {
        self.kind == "music-videos"
    }
    pub fn duration(&self) -> String {
        let s = self.duration_ms / 1000;
        format!("{}:{:02}", s / 60, s % 60)
    }
    pub fn quality_tag(&self) -> String {
        if self.traits.iter().any(|t| t == "hi-res-lossless") {
            "Hi-Res".into()
        } else if self.traits.iter().any(|t| t == "lossless") {
            "Lossless".into()
        } else if self.traits.iter().any(|t| t == "atmos") {
            "Atmos".into()
        } else {
            "AAC".into()
        }
    }
}

#[derive(Clone)]
pub struct Item {
    pub kind: Kind,
    pub storefront: String,
    pub id: String,
    pub title: String,
    pub artist: String,
    pub track_count: u32,
    pub album_traits: Vec<String>,
    pub tracks: Vec<Track>,
    /// URL sans ?i= : necessaire pour telecharger un album par selection.
    pub base_url: String,
    /// Index (0-based) impose par ?i=<trackId>
    pub forced_track: Option<usize>,
}

impl Item {
    pub fn is_single(&self) -> bool {
        self.kind == Kind::Song || (self.kind == Kind::Album && self.forced_track.is_some())
    }

    pub fn all_traits(&self) -> Vec<String> {
        let mut v = self.album_traits.clone();
        for t in &self.tracks {
            for tr in &t.traits {
                if !v.contains(tr) {
                    v.push(tr.clone());
                }
            }
        }
        v
    }

    pub fn has(&self, t: &str) -> bool {
        self.all_traits().iter().any(|x| x == t)
    }

    pub fn has_hires(&self) -> bool {
        self.has("hi-res-lossless")
    }
    pub fn has_lossless(&self) -> bool {
        self.has("lossless") || self.has_hires()
    }
    pub fn has_atmos(&self) -> bool {
        self.has("atmos")
    }

    /// Meilleure qualite disponible, pour l'affichage.
    pub fn max_quality_label(&self) -> String {
        if self.has_hires() {
            "Hi-Res Lossless (max 24-bit/192 kHz)".into()
        } else if self.has_lossless() {
            "Lossless (max 24-bit/48 kHz)".into()
        } else {
            "AAC 256 kbps".into()
        }
    }
}

fn str_at(v: &Value, path: &[&str]) -> String {
    let mut cur = v;
    for k in path {
        cur = &cur[k];
    }
    cur.as_str().unwrap_or("").to_string()
}

fn u64_at(v: &Value, path: &[&str]) -> u64 {
    let mut cur = v;
    for k in path {
        cur = &cur[k];
    }
    cur.as_u64().unwrap_or(0)
}

fn traits_at(v: &Value, path: &[&str]) -> Vec<String> {
    let mut cur = v;
    for k in path {
        cur = &cur[k];
    }
    cur.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

fn parse_tracks(v: &Value) -> Vec<Track> {
    v.as_array()
        .map(|arr| {
            arr.iter()
                .map(|t| Track {
                    id: str_at(t, &["id"]),
                    name: str_at(t, &["attributes", "name"]),
                    track_number: u64_at(t, &["attributes", "trackNumber"]) as u32,
                    disc_number: u64_at(t, &["attributes", "discNumber"]) as u32,
                    duration_ms: u64_at(t, &["attributes", "durationInMillis"]),
                    traits: traits_at(t, &["attributes", "audioTraits"]),
                    rating: str_at(t, &["attributes", "contentRating"]),
                    kind: str_at(t, &["type"]),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Recupere une ressource du catalogue + ses pistes paginees.
pub fn fetch_catalog(
    storefront: &str,
    kind: Kind,
    id: &str,
    token: &str,
    forced_track: Option<String>,
) -> Result<Item, String> {
    let seg = match kind {
        Kind::Album => "albums",
        Kind::Song => "songs",
        Kind::Playlist => "playlists",
        Kind::MusicVideo => "music-videos",
        Kind::Artist => return Err("Les artistes ne sont pas geres par cette interface".into()),
    };

    let url = format!(
        "https://amp-api.music.apple.com/v1/catalog/{storefront}/{seg}/{id}\
?omit%5Bresource%5D=autos&include=tracks,artists&extend=editorialVideo,extendedAssetUrls&l="
    );
    let v = get_authed(&url, token)?;

    if v["data"].as_array().map(|a| a.is_empty()).unwrap_or(true) {
        return Err(format!(
            "{id} introuvable sur le storefront « {storefront} »"
        ));
    }

    let d = &v["data"][0];
    let title = str_at(d, &["attributes", "name"]);
    let artist = str_at(d, &["attributes", "artistName"]);
    let track_count = if kind == Kind::Song {
        1
    } else {
        u64_at(d, &["attributes", "trackCount"]) as u32
    };
    let album_traits = traits_at(d, &["attributes", "audioTraits"]);

    let mut tracks = parse_tracks(&d["relationships"]["tracks"]["data"]);

    // Pagination : les albums longs arrivent par pages de 100.
    let mut next = str_at(&d["relationships"]["tracks"], &["next"]);
    let mut guard = 0;
    while !next.is_empty() && guard < 20 {
        guard += 1;
        let page_url = format!(
            "https://amp-api.music.apple.com{next}?omit%5Bresource%5D=autos&include=artists&extend=editorialVideo,extendedAssetUrls"
        );
        let page = get_authed(&page_url, token)?;
        tracks.extend(parse_tracks(&page["data"]));
        next = str_at(&page, &["next"]);
    }

    // Un titre seul n'expose pas relationships.tracks : on le fabrique.
    if kind == Kind::Song && tracks.is_empty() {
        tracks.push(Track {
            id: str_at(d, &["id"]),
            name: if title.is_empty() {
                id.to_string()
            } else {
                title.clone()
            },
            track_number: u64_at(d, &["attributes", "trackNumber"]) as u32,
            disc_number: u64_at(d, &["attributes", "discNumber"]) as u32,
            duration_ms: u64_at(d, &["attributes", "durationInMillis"]),
            traits: traits_at(d, &["attributes", "audioTraits"]),
            rating: str_at(d, &["attributes", "contentRating"]),
            kind: "songs".into(),
        });
    }

    let mut forced_idx = None;
    if let Some(tid) = forced_track {
        if let Some(pos) = tracks.iter().position(|t| t.id == tid) {
            forced_idx = Some(pos);
        }
    }

    let base_url = format!(
        "https://music.apple.com/{storefront}/{}/{}",
        match kind {
            Kind::Album => "album",
            Kind::Playlist => "playlist",
            Kind::Song => "song",
            Kind::MusicVideo => "music-video",
            Kind::Artist => "artist",
        },
        id
    );

    Ok(Item {
        kind,
        storefront: storefront.to_string(),
        id: id.to_string(),
        title,
        artist,
        track_count,
        album_traits,
        tracks,
        base_url,
        forced_track: forced_idx,
    })
}
