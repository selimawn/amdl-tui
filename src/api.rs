// Couche API Apple Music : token web + catalogue (album, titre, playlist, artiste).
//
// Le modele est une arborescence d'albums repliables : un album est un noeud
// contenant ses pistes (chargees paresseusement), un artiste est une liste de
// noeuds. Un album seul ou un titre est donc juste un cas particulier.

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
    let re_tok = Regex::new(r"eyJ[A-Za-z0-9\-_=]+\.[A-Za-z0-9\-_=]+\.[A-Za-z0-9\-_=]+").unwrap();
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
}

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

    let segs: Vec<&str> = base.split('/').filter(|s| !s.is_empty()).collect();

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
        if cand.len() == 2 && cand.chars().all(|c| c.is_ascii_alphabetic()) {
            storefront = Some(cand.to_lowercase());
        }
    }

    let id = segs.last().unwrap().to_string();
    if ["album", "song", "playlist", "artist"].contains(&id.as_str()) {
        return Err("Identifiant manquant dans l'URL".into());
    }

    let mut track_id = None;
    for part in query.split('&') {
        if let Some(v) = part.strip_prefix("i=") {
            if !v.is_empty() {
                track_id = Some(v.to_string());
            }
        }
    }

    Ok(ParsedUrl {
        raw: trimmed.to_string(),
        storefront: Some(storefront.unwrap_or_else(|| fallback_storefront.to_string())),
        kind,
        id,
        track_id,
    })
}

// ---------------------------------------------------------------- Pistes

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
        quality_tag(&self.traits)
    }
}

fn quality_tag(traits: &[String]) -> String {
    if traits.iter().any(|t| t == "hi-res-lossless") {
        "Hi-Res".into()
    } else if traits.iter().any(|t| t == "lossless") {
        "Lossless".into()
    } else if traits.iter().any(|t| t == "atmos") {
        "Atmos".into()
    } else {
        "AAC".into()
    }
}

// ---------------------------------------------------------------- Albums

/// Un album (ou une playlist) et ses pistes, chargeable a la demande.
#[derive(Clone)]
pub struct AlbumNode {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub year: String,
    pub track_count: u32,
    pub traits: Vec<String>,
    pub tracks: Vec<Track>,
    pub checked: Vec<bool>,
    pub loaded: bool,
    pub loading: bool,
    pub expanded: bool,
}

impl AlbumNode {
    fn empty(id: String, title: String, artist: String) -> Self {
        Self {
            id,
            title,
            artist,
            year: String::new(),
            track_count: 0,
            traits: Vec::new(),
            tracks: Vec::new(),
            checked: Vec::new(),
            loaded: false,
            loading: false,
            expanded: false,
        }
    }

    pub fn has(&self, t: &str) -> bool {
        self.traits.iter().any(|x| x == t)
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

    pub fn max_quality_label(&self) -> String {
        if self.has_hires() {
            "Hi-Res Lossless".into()
        } else if self.has_lossless() {
            "Lossless".into()
        } else if self.has_atmos() {
            "Atmos".into()
        } else {
            "AAC".into()
        }
    }

    pub fn quality_tag(&self) -> String {
        quality_tag(&self.traits)
    }

    pub fn checked_count(&self) -> usize {
        self.checked.iter().filter(|c| **c).count()
    }
    pub fn all_checked(&self) -> bool {
        !self.checked.is_empty() && self.checked.iter().all(|c| *c)
    }
    pub fn none_checked(&self) -> bool {
        self.checked.iter().all(|c| !*c)
    }
    /// Marqueur de case a cocher, avec l'etat partiel.
    pub fn check_box(&self) -> &'static str {
        if self.none_checked() {
            "[ ]"
        } else if self.all_checked() {
            "[x]"
        } else {
            "[-]"
        }
    }
    /// Positions 1-based des pistes cochees, pour `amdl --select`.
    pub fn selected_positions(&self) -> Vec<usize> {
        self.checked
            .iter()
            .enumerate()
            .filter(|(_, c)| **c)
            .map(|(i, _)| i + 1)
            .collect()
    }
}

// ---------------------------------------------------------------- Item

#[derive(Clone)]
pub struct Item {
    pub kind: Kind,
    pub storefront: String,
    pub id: String,
    pub title: String,
    pub artist: String,
    pub albums: Vec<AlbumNode>,
    /// URL « conteneur » : album, playlist ou artiste.
    pub base_url: String,
    /// Index (dans albums[0]) de la piste ciblee par ?i=, si presente.
    pub forced_track: Option<usize>,
}

impl Item {
    pub fn all_traits(&self) -> Vec<String> {
        let mut v: Vec<String> = Vec::new();
        for a in &self.albums {
            for t in &a.traits {
                if !v.contains(t) {
                    v.push(t.clone());
                }
            }
        }
        v
    }
    pub fn has_lossless(&self) -> bool {
        self.albums.iter().any(|a| a.has_lossless())
    }
    pub fn has_atmos(&self) -> bool {
        self.albums.iter().any(|a| a.has_atmos())
    }
    pub fn has_hires(&self) -> bool {
        self.albums.iter().any(|a| a.has_hires())
    }
    pub fn is_multi(&self) -> bool {
        self.albums.len() > 1
    }
    pub fn total_tracks(&self) -> usize {
        self.albums.iter().map(|a| a.tracks.len()).sum()
    }
    pub fn total_checked(&self) -> usize {
        self.albums.iter().map(|a| a.checked_count()).sum()
    }
    /// Un seul album coche avec toutes ses pistes et un ?i= : mode piste unique.
    pub fn is_single_track(&self) -> bool {
        if self.forced_track.is_none() || self.albums.len() != 1 {
            return false;
        }
        let a = &self.albums[0];
        a.checked_count() == 1
            && a.checked
                .iter()
                .position(|c| *c)
                .map(|i| Some(i) == self.forced_track)
                .unwrap_or(false)
    }
    pub fn max_quality_label(&self) -> String {
        if self.has_hires() {
            "Hi-Res Lossless (max 24-bit/192 kHz)".into()
        } else if self.has_lossless() {
            "Lossless (max 24-bit/48 kHz)".into()
        } else if self.has_atmos() {
            "Dolby Atmos".into()
        } else {
            "AAC 256 kbps".into()
        }
    }
}

// ---------------------------------------------------------------- Extraction

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

fn year_of(v: &Value) -> String {
    let d = str_at(v, &["attributes", "releaseDate"]);
    d.get(0..4).unwrap_or("").to_string()
}

/// Charge les pistes d'un album (et suit la pagination).
pub fn fetch_album_tracks(storefront: &str, album_id: &str, token: &str) -> Result<Vec<Track>, String> {
    let url = format!(
        "https://amp-api.music.apple.com/v1/catalog/{storefront}/albums/{album_id}\
?omit%5Bresource%5D=autos&include=tracks,artists&extend=editorialVideo,extendedAssetUrls&l="
    );
    let v = get_authed(&url, token)?;
    if v["data"].as_array().map(|a| a.is_empty()).unwrap_or(true) {
        return Err(format!("album {album_id} introuvable"));
    }
    let d = &v["data"][0];
    let mut tracks = parse_tracks(&d["relationships"]["tracks"]["data"]);

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
    Ok(tracks)
}

/// Charge tous les albums d'un artiste (avec pagination).
pub fn fetch_artist_albums(
    storefront: &str,
    artist_id: &str,
    token: &str,
) -> Result<(String, Vec<AlbumNode>), String> {
    // nom de l'artiste + premiers albums
    let mut url = format!(
        "https://amp-api.music.apple.com/v1/catalog/{storefront}/artists/{artist_id}/albums\
?limit=100&extend=extendedAssetUrls&l="
    );
    let mut out: Vec<AlbumNode> = Vec::new();
    let mut artist_name = String::new();
    let mut guard = 0;

    loop {
        guard += 1;
        if guard > 25 {
            break;
        }
        let v = get_authed(&url, token)?;
        let data = match v["data"].as_array() {
            Some(a) => a,
            None => break,
        };
        if guard == 1 && data.is_empty() {
            return Err(format!("artiste {artist_id} introuvable"));
        }
        for alb in data {
            let at = &alb["attributes"];
            let name = str_at(at, &["name"]);
            let artist = str_at(at, &["artistName"]);
            if artist_name.is_empty() && !artist.is_empty() {
                artist_name = artist.clone();
            }
            let mut node = AlbumNode::empty(str_at(&alb["id"], &[]), name, artist);
            node.year = year_of(alb);
            node.track_count = u64_at(at, &["trackCount"]) as u32;
            node.traits = traits_at(at, &["audioTraits"]);
            out.push(node);
        }
        let next = str_at(&v, &["next"]);
        if next.is_empty() {
            break;
        }
        url = format!("https://amp-api.music.apple.com{next}");
    }

    Ok((artist_name, out))
}

/// Construit l'Item correspondant a une URL analysee.
pub fn fetch(
    storefront: &str,
    kind: Kind,
    id: &str,
    token: &str,
    forced_track: Option<String>,
) -> Result<Item, String> {
    let seg = match kind {
        Kind::Album => "album",
        Kind::Song => "song",
        Kind::Playlist => "playlist",
        Kind::Artist => "artist",
        Kind::MusicVideo => "music-video",
    };
    let base_url = format!("https://music.apple.com/{storefront}/{seg}/{id}");

    match kind {
        Kind::Artist => {
            let (artist, albums) = fetch_artist_albums(storefront, id, token)?;
            if albums.is_empty() {
                return Err("Cet artiste n'a aucun album sur ce storefront.".into());
            }
            Ok(Item {
                kind,
                storefront: storefront.to_string(),
                id: id.to_string(),
                title: if artist.is_empty() {
                    id.to_string()
                } else {
                    artist.clone()
                },
                artist,
                albums,
                base_url,
                forced_track: None,
            })
        }
        Kind::Album | Kind::Playlist | Kind::MusicVideo => {
            let url = format!(
                "https://amp-api.music.apple.com/v1/catalog/{storefront}/{seg}s/{id}\
?omit%5Bresource%5D=autos&include=tracks,artists&extend=editorialVideo,extendedAssetUrls&l="
            );
            let v = get_authed(&url, token)?;
            if v["data"].as_array().map(|a| a.is_empty()).unwrap_or(true) {
                return Err(format!("{id} introuvable sur le storefront « {storefront} »"));
            }
            let d = &v["data"][0];
            let title = str_at(d, &["attributes", "name"]);
            let artist = str_at(d, &["attributes", "artistName"]);
            let mut node = AlbumNode::empty(str_at(d, &["id"]), title.clone(), artist.clone());
            node.year = year_of(d);
            node.track_count = u64_at(d, &["attributes", "trackCount"]) as u32;
            node.traits = traits_at(d, &["attributes", "audioTraits"]);
            node.tracks = parse_tracks(&d["relationships"]["tracks"]["data"]);

            let mut next = str_at(&d["relationships"]["tracks"], &["next"]);
            let mut guard = 0;
            while !next.is_empty() && guard < 20 {
                guard += 1;
                let page_url = format!(
                    "https://amp-api.music.apple.com{next}?omit%5Bresource%5D=autos&include=artists&extend=editorialVideo,extendedAssetUrls"
                );
                let page = get_authed(&page_url, token)?;
                node.tracks.extend(parse_tracks(&page["data"]));
                next = str_at(&page, &["next"]);
            }

            node.checked = vec![true; node.tracks.len()];
            node.loaded = true;
            node.expanded = true;

            let forced = forced_track.and_then(|tid| node.tracks.iter().position(|t| t.id == tid));
            if let Some(idx) = forced {
                for (i, c) in node.checked.iter_mut().enumerate() {
                    *c = i == idx;
                }
            }

            Ok(Item {
                kind,
                storefront: storefront.to_string(),
                id: id.to_string(),
                title,
                artist,
                albums: vec![node],
                base_url,
                forced_track: forced,
            })
        }
        Kind::Song => {
            let url = format!(
                "https://amp-api.music.apple.com/v1/catalog/{storefront}/songs/{id}\
?omit%5Bresource%5D=autos&include=albums&extend=extendedAssetUrls&l="
            );
            let v = get_authed(&url, token)?;
            if v["data"].as_array().map(|a| a.is_empty()).unwrap_or(true) {
                return Err(format!("titre {id} introuvable sur le storefront « {storefront} »"));
            }
            let d = &v["data"][0];
            let title = str_at(d, &["attributes", "name"]);
            let artist = str_at(d, &["attributes", "artistName"]);
            let album_name = str_at(d, &["attributes", "albumName"]);

            let mut node = AlbumNode::empty(
                str_at(d, &["attributes", "playParams", "id"]),
                if album_name.is_empty() {
                    title.clone()
                } else {
                    album_name
                },
                artist.clone(),
            );
            node.year = year_of(d);
            node.traits = traits_at(d, &["attributes", "audioTraits"]);
            node.tracks = vec![Track {
                id: str_at(d, &["id"]),
                name: title.clone(),
                track_number: u64_at(d, &["attributes", "trackNumber"]) as u32,
                disc_number: u64_at(d, &["attributes", "discNumber"]) as u32,
                duration_ms: u64_at(d, &["attributes", "durationInMillis"]),
                traits: traits_at(d, &["attributes", "audioTraits"]),
                rating: str_at(d, &["attributes", "contentRating"]),
                kind: "songs".into(),
            }];
            node.track_count = 1;
            node.checked = vec![true];
            node.loaded = true;
            node.expanded = true;

            Ok(Item {
                kind,
                storefront: storefront.to_string(),
                id: id.to_string(),
                title,
                artist,
                albums: vec![node],
                base_url,
                forced_track: Some(0),
            })
        }
    }
}

/// Supprime les doublons d'albums (un artiste peut renvoyer plusieurs editions).
pub fn dedup_albums(albums: &mut Vec<AlbumNode>) {
    let mut seen: Vec<(String, u32)> = Vec::new();
    albums.retain(|a| {
        let key = (a.title.to_lowercase(), a.track_count);
        if seen.contains(&key) {
            false
        } else {
            seen.push(key);
            true
        }
    });
}
