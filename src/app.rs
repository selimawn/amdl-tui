// Etat de l'application : arborescence d'albums, selection, file de
// telechargement, gestion clavier ET souris.

use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};

use ratatui::crossterm::event::{KeyCode, KeyModifiers};

use crate::amdl::{self, Event as AmdlEvent, Format, Options, ALAC_MAXES, LRC_EXTRAS, LRC_TYPES};
use crate::api::{self, Item, Kind};

#[derive(PartialEq, Eq, Clone, Copy)]
pub enum Screen {
    Input,
    Loading,
    Ready,
    Running,
    Done,
}

#[derive(PartialEq, Eq, Clone, Copy)]
pub enum Focus {
    Tracks,
    Options,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Row {
    Format,
    AlacMax,
    EmbedLrc,
    SaveLrc,
    LrcType,
    LrcExtra,
    KeepOriginal,
}

/// Une ligne visible dans l'arbre (album replie/deplie ou une de ses pistes).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RowRef {
    Album(usize),
    Track(usize, usize),
}

enum Msg {
    Loaded(Box<Item>, String, String),
    LoadErr(String),
    Tracks(usize, Vec<api::Track>),
    TracksErr(usize, String),
    Ev(AmdlEvent),
    StackLine(String),
    StackDone(bool, String),
}

pub fn dbg(msg: &str) {
    if std::env::var_os("AMDL_TUI_DEBUG").is_none() {
        return;
    }
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/private/var/folders/l8/qwzvp7bn293ch9jbrnwqrn7h0000gn/T/amdl-tui-debug.log")
    {
        let _ = writeln!(f, "{msg}");
    }
}

#[derive(Clone)]
pub struct Job {
    pub url: String,
    pub selection: Option<Vec<usize>>,
    pub label: String,
}

pub struct App {
    pub screen: Screen,
    pub input: String,
    pub status: String,
    pub error: Option<String>,
    pub item: Option<Item>,
    pub cursor: usize,
    pub focus: Focus,
    pub opt_cursor: usize,
    pub opts: Options,
    pub log: Vec<String>,
    pub progress: Option<u8>,
    pub job_info: String,
    pub done_code: Option<i32>,
    pub should_quit: bool,
    pub save_dir: PathBuf,
    pub storefront: String,
    pub target_url: String,
    pub lite_ok: Option<bool>,
    pub spinner: usize,
    pub stack_busy: bool,
    pub stack_log: Vec<String>,
    pub tree_scroll: usize,
    token: Option<String>,
    rx: Option<Receiver<Msg>>,
}

impl App {
    pub fn new() -> Self {
        Self {
            screen: Screen::Input,
            input: String::new(),
            status: String::new(),
            error: None,
            item: None,
            cursor: 0,
            focus: Focus::Tracks,
            opt_cursor: 0,
            opts: Options::default(),
            log: Vec::new(),
            progress: None,
            job_info: String::new(),
            done_code: None,
            should_quit: false,
            save_dir: amdl::default_save_dir(),
            storefront: read_storefront(),
            target_url: String::new(),
            lite_ok: Some(amdl::lite_status().is_ok()),
            spinner: 0,
            stack_busy: false,
            stack_log: Vec::new(),
            tree_scroll: 0,
            token: None,
            rx: None,
        }
    }

    // ------------------------------------------------------- arborescence

    /// Lignes visibles, dans l'ordre d'affichage.
    pub fn visible_rows(&self) -> Vec<RowRef> {
        let mut out = Vec::new();
        if let Some(it) = &self.item {
            for (ai, alb) in it.albums.iter().enumerate() {
                out.push(RowRef::Album(ai));
                if alb.expanded {
                    for ti in 0..alb.tracks.len() {
                        out.push(RowRef::Track(ai, ti));
                    }
                }
            }
        }
        out
    }

    pub fn current_row(&self) -> Option<RowRef> {
        self.visible_rows().get(self.cursor).copied()
    }

    fn move_cursor(&mut self, delta: i32) {
        let n = self.visible_rows().len();
        if n == 0 {
            return;
        }
        let cur = self.cursor as i32 + delta;
        self.cursor = cur.clamp(0, n as i32 - 1) as usize;
    }

    // ------------------------------------------------------- selection

    fn toggle_track(&mut self, ai: usize, ti: usize) {
        if let Some(it) = &mut self.item {
            if let Some(a) = it.albums.get_mut(ai) {
                if let Some(c) = a.checked.get_mut(ti) {
                    *c = !*c;
                }
            }
        }
    }

    fn toggle_album_all(&mut self, ai: usize) {
        if let Some(it) = &mut self.item {
            if let Some(a) = it.albums.get_mut(ai) {
                let target = !a.all_checked();
                for c in a.checked.iter_mut() {
                    *c = target;
                }
            }
        }
    }

    fn set_all(&mut self, value: bool) {
        if let Some(it) = &mut self.item {
            for a in it.albums.iter_mut() {
                for c in a.checked.iter_mut() {
                    *c = value;
                }
            }
        }
    }

    fn invert_all(&mut self) {
        if let Some(it) = &mut self.item {
            for a in it.albums.iter_mut() {
                for c in a.checked.iter_mut() {
                    *c = !*c;
                }
            }
        }
    }

    fn toggle_expand(&mut self, ai: usize) {
        let (need_load, album_id) = match &mut self.item {
            Some(it) => match it.albums.get_mut(ai) {
                Some(a) => {
                    if a.loaded {
                        a.expanded = !a.expanded;
                        (false, String::new())
                    } else if a.loading {
                        (false, String::new())
                    } else {
                        a.loading = true;
                        a.expanded = true;
                        (true, a.id.clone())
                    }
                }
                None => (false, String::new()),
            },
            None => (false, String::new()),
        };
        if need_load {
            self.spawn_tracks_fetch(ai, album_id);
        }
    }

    /// Va jusqu'a une piste : deplie son album au besoin puis la selectionne.
    pub fn reveal_track(&mut self, album: usize, track: usize) {
        if let Some(it) = &mut self.item {
            if let Some(a) = it.albums.get_mut(album) {
                a.expanded = true;
            }
        }
        if self
            .item
            .as_ref()
            .and_then(|i| i.albums.get(album))
            .map(|a| !a.loaded && !a.loading)
            .unwrap_or(false)
        {
            let id = self.item.as_ref().unwrap().albums[album].id.clone();
            self.spawn_tracks_fetch(album, id);
        }
        // la ligne de la piste apparaitra apres chargement ; on se place sur l'album
        if let Some(pos) = self
            .visible_rows()
            .iter()
            .position(|r| *r == RowRef::Track(album, track))
        {
            self.cursor = pos;
        }
    }

    // ------------------------------------------------------- chargements

    fn spawn_tracks_fetch(&mut self, album: usize, album_id: String) {
        let (tx, rx): (Sender<Msg>, Receiver<Msg>) = channel();
        self.rx = Some(rx);
        let sf = self
            .item
            .as_ref()
            .map(|i| i.storefront.clone())
            .unwrap_or_else(|| self.storefront.clone());
        let cached = self.token.clone();
        std::thread::spawn(move || {
            let token = match cached {
                Some(t) => t,
                None => match api::get_token() {
                    Ok(t) => t,
                    Err(e) => {
                        let _ = tx.send(Msg::TracksErr(album, e));
                        return;
                    }
                },
            };
            match api::fetch_album_tracks(&sf, &album_id, &token) {
                Ok(tracks) => {
                    let _ = tx.send(Msg::Tracks(album, tracks));
                }
                Err(e) => {
                    let _ = tx.send(Msg::TracksErr(album, e));
                }
            }
        });
    }

    fn load_url(&mut self) {
        let raw = self.input.trim().to_string();
        if raw.is_empty() {
            return;
        }
        let (tx, rx): (Sender<Msg>, Receiver<Msg>) = channel();
        self.rx = Some(rx);
        self.screen = Screen::Loading;
        self.error = None;
        self.status = "Recuperation du token Apple et des metadonnees...".into();
        self.lite_ok = Some(amdl::lite_status().is_ok());

        let sf = self.storefront.clone();
        std::thread::spawn(move || {
            let p = match api::parse_url(&raw, &sf) {
                Ok(p) => p,
                Err(e) => {
                    let _ = tx.send(Msg::LoadErr(e));
                    return;
                }
            };
            let storefront = p.storefront.clone().unwrap_or_else(|| sf.clone());
            let token = match api::get_token() {
                Ok(t) => t,
                Err(e) => {
                    let _ = tx.send(Msg::LoadErr(e));
                    return;
                }
            };
            match api::fetch(&storefront, p.kind, &p.id, &token, p.track_id.clone()) {
                Ok(item) => {
                    let _ = tx.send(Msg::Loaded(Box::new(item), p.raw.clone(), token));
                }
                Err(e) => {
                    let _ = tx.send(Msg::LoadErr(e));
                }
            }
        });
    }

    // ------------------------------------------------------- options

    pub fn rows(&self) -> Vec<Row> {
        let mut r = vec![Row::Format];
        if matches!(self.opts.format, Format::Alac | Format::Flac) {
            r.push(Row::AlacMax);
        }
        r.push(Row::EmbedLrc);
        r.push(Row::SaveLrc);
        r.push(Row::LrcType);
        r.push(Row::LrcExtra);
        if self.opts.format == Format::Flac {
            r.push(Row::KeepOriginal);
        }
        r
    }

    pub fn row_label(row: Row) -> &'static str {
        match row {
            Row::Format => "Format",
            Row::AlacMax => "Qualite ALAC",
            Row::EmbedLrc => "Paroles integrees",
            Row::SaveLrc => "Fichier .lrc",
            Row::LrcType => "Type de paroles",
            Row::LrcExtra => "Traduction",
            Row::KeepOriginal => "Garder l'original",
        }
    }

    pub fn row_value(&self, row: Row) -> String {
        match row {
            Row::Format => self.opts.format.label().to_string(),
            Row::AlacMax => {
                let i = self.opts.alac_max_idx.min(ALAC_MAXES.len() - 1);
                if i == 0 {
                    format!("{}  (recommande)", ALAC_MAXES[i].1)
                } else {
                    ALAC_MAXES[i].1.to_string()
                }
            }
            Row::EmbedLrc => yesno(self.opts.embed_lrc),
            Row::SaveLrc => yesno(self.opts.save_lrc_file),
            Row::LrcType => {
                let i = self.opts.lrc_type_idx.min(LRC_TYPES.len() - 1);
                if LRC_TYPES[i] == "syllable-lyrics" {
                    "mot-a-mot".into()
                } else {
                    "lignes".into()
                }
            }
            Row::LrcExtra => {
                let i = self.opts.lrc_extra_idx.min(LRC_EXTRAS.len() - 1);
                if LRC_EXTRAS[i].is_empty() {
                    "aucune".into()
                } else {
                    LRC_EXTRAS[i].to_string()
                }
            }
            Row::KeepOriginal => yesno(self.opts.keep_original),
        }
    }

    pub fn available_formats(&self) -> Vec<Format> {
        let (ll, at) = match &self.item {
            Some(it) => (it.has_lossless(), it.has_atmos()),
            None => (true, false),
        };
        let mut v = Vec::new();
        if ll {
            v.push(Format::Alac);
            v.push(Format::Flac);
        }
        if at {
            v.push(Format::Atmos);
        }
        v.push(Format::Aac);
        v
    }

    fn cycle_row(&mut self, row: Row, dir: i32) {
        let fwd = dir >= 0;
        match row {
            Row::Format => {
                let avail = self.available_formats();
                if avail.is_empty() {
                    return;
                }
                let cur = avail.iter().position(|f| *f == self.opts.format).unwrap_or(0);
                let n = avail.len() as i32;
                self.opts.format = avail[((cur as i32 + if fwd { 1 } else { -1 }).rem_euclid(n)) as usize];
            }
            Row::AlacMax => {
                let n = ALAC_MAXES.len() as i32;
                self.opts.alac_max_idx =
                    ((self.opts.alac_max_idx as i32 + if fwd { 1 } else { -1 }).rem_euclid(n)) as usize;
            }
            Row::EmbedLrc => self.opts.embed_lrc = !self.opts.embed_lrc,
            Row::SaveLrc => self.opts.save_lrc_file = !self.opts.save_lrc_file,
            Row::LrcType => {
                let n = LRC_TYPES.len() as i32;
                self.opts.lrc_type_idx =
                    ((self.opts.lrc_type_idx as i32 + if fwd { 1 } else { -1 }).rem_euclid(n)) as usize;
            }
            Row::LrcExtra => {
                let n = LRC_EXTRAS.len() as i32;
                self.opts.lrc_extra_idx =
                    ((self.opts.lrc_extra_idx as i32 + if fwd { 1 } else { -1 }).rem_euclid(n)) as usize;
            }
            Row::KeepOriginal => self.opts.keep_original = !self.opts.keep_original,
        }
    }

    // ------------------------------------------------------- telechargement

    fn album_url(&self, it: &Item, ai: usize) -> String {
        let sf = &it.storefront;
        match it.kind {
            Kind::Song => self.target_url.clone(),
            Kind::Playlist => format!("https://music.apple.com/{sf}/playlist/{}", it.albums[ai].id),
            Kind::MusicVideo => {
                format!("https://music.apple.com/{sf}/music-video/{}", it.albums[ai].id)
            }
            _ => format!("https://music.apple.com/{sf}/album/{}", it.albums[ai].id),
        }
    }

    fn build_jobs(&self) -> Vec<Job> {
        let it = match &self.item {
            Some(i) => i,
            None => return Vec::new(),
        };

        // Cas « une seule piste ciblee par ?i= » : on passe l'URL d'origine.
        if it.is_single_track() {
            return vec![Job {
                url: if self.target_url.is_empty() {
                    it.base_url.clone()
                } else {
                    self.target_url.clone()
                },
                selection: None,
                label: it.albums[0].title.clone(),
            }];
        }

        let mut jobs = Vec::new();
        for (ai, alb) in it.albums.iter().enumerate() {
            if alb.checked_count() == 0 || alb.tracks.is_empty() {
                continue;
            }
            let positions = alb.selected_positions();
            let n = positions.len();
            let whole = n == alb.tracks.len();
            jobs.push(Job {
                url: self.album_url(it, ai),
                selection: if whole { None } else { Some(positions) },
                label: format!(
                    "{} ({})",
                    alb.title,
                    if whole {
                        "album complet".to_string()
                    } else {
                        format!("{n} pistes")
                    }
                ),
            });
        }
        jobs
    }

    fn start_download(&mut self) {
        let it = match &self.item {
            Some(i) => i.clone(),
            None => return,
        };
        let jobs = self.build_jobs();
        if jobs.is_empty() {
            self.error = Some(
                "Rien a telecharger : deplie un album (→ ou Entree) et coche au moins une piste."
                    .into(),
            );
            return;
        }

        let (tx, rx): (Sender<Msg>, Receiver<Msg>) = channel();
        self.rx = Some(rx);
        self.screen = Screen::Running;
        self.error = None;
        self.log.clear();
        self.progress = None;
        self.done_code = None;
        self.job_info = format!("0/{}", jobs.len());
        self.status = format!("{} -> {}", self.opts.format.short(), self.save_dir.display());

        let opts = Options {
            format: self.opts.format,
            alac_max_idx: self.opts.alac_max_idx,
            embed_lrc: self.opts.embed_lrc,
            save_lrc_file: self.opts.save_lrc_file,
            lrc_type_idx: self.opts.lrc_type_idx,
            lrc_extra_idx: self.opts.lrc_extra_idx,
            keep_original: self.opts.keep_original,
        };
        let save_dir = self.save_dir.clone();
        let storefront = it.storefront.clone();
        let total = jobs.len();

        std::thread::spawn(move || {
            let (atx, arx) = channel::<AmdlEvent>();
            let mtx = tx.clone();
            std::thread::spawn(move || {
                while let Ok(ev) = arx.recv() {
                    if mtx.send(Msg::Ev(ev)).is_err() {
                        break;
                    }
                }
            });

            let mut worst = 0;
            for (n, job) in jobs.iter().enumerate() {
                let _ = tx.send(Msg::Ev(AmdlEvent::Line(format!(
                    "───── {}/{} · {} ─────",
                    n + 1,
                    total,
                    job.label
                ))));
                let plan = match amdl::plan(
                    &storefront,
                    &job.url,
                    job.selection.as_deref(),
                    &opts,
                    &save_dir,
                ) {
                    Ok(p) => p,
                    Err(e) => {
                        let _ = tx.send(Msg::Ev(AmdlEvent::Line(format!("ERREUR: {e}"))));
                        worst = -1;
                        continue;
                    }
                };
                match amdl::run_plan_quiet(&plan, atx.clone()) {
                    Ok(code) => {
                        if code != 0 {
                            worst = code;
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Msg::Ev(AmdlEvent::Line(format!("ERREUR: {e}"))));
                        worst = -1;
                    }
                }
            }
            let _ = tx.send(Msg::Ev(AmdlEvent::Done(worst)));
        });
    }

    // ------------------------------------------------------- messages

    pub fn poll(&mut self) {
        if self.screen == Screen::Loading {
            self.spinner = self.spinner.wrapping_add(1);
        }
        let mut disconnected = false;
        let mut handled = 0usize;
        loop {
            let next = match &self.rx {
                Some(rx) => match rx.try_recv() {
                    Ok(m) => Some(m),
                    Err(std::sync::mpsc::TryRecvError::Empty) => None,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        disconnected = true;
                        None
                    }
                },
                None => None,
            };
            let m = match next {
                Some(m) => m,
                None => break,
            };
            handled += 1;
            match m {
                Msg::LoadErr(e) => {
                    self.error = Some(e);
                    self.screen = Screen::Input;
                    self.rx = None;
                }
                Msg::Loaded(item, raw, token) => {
                    self.token = Some(token);
                    self.target_url = raw.clone();
                    self.cursor = 0;
                    self.opt_cursor = 0;
                    self.focus = Focus::Tracks;
                    self.opts.format = if item.has_lossless() {
                        Format::Alac
                    } else if item.has_atmos() {
                        Format::Atmos
                    } else {
                        Format::Aac
                    };
                    self.status = match item.kind {
                        Kind::Artist => format!("{} albums charges", item.albums.len()),
                        _ => format!("{} pistes", item.total_tracks()),
                    };
                    self.item = Some(*item);
                    self.screen = Screen::Ready;
                    self.rx = None;
                }
                Msg::Tracks(ai, tracks) => {
                    if let Some(it) = &mut self.item {
                        if let Some(a) = it.albums.get_mut(ai) {
                            a.tracks = tracks;
                            // Deplier un album ne doit RIEN cocher : c'est une
                            // exploration, l'utilisateur choisit ensuite.
                            a.checked = vec![false; a.tracks.len()];
                            a.loaded = true;
                            a.loading = false;
                            if a.track_count == 0 {
                                a.track_count = a.tracks.len() as u32;
                            }
                        }
                    }
                    self.rx = None;
                }
                Msg::TracksErr(ai, e) => {
                    if let Some(it) = &mut self.item {
                        if let Some(a) = it.albums.get_mut(ai) {
                            a.loading = false;
                        }
                    }
                    self.error = Some(e);
                    self.rx = None;
                }
                Msg::StackLine(l) => {
                    self.stack_log.push(l);
                    if self.stack_log.len() > 300 {
                        self.stack_log.remove(0);
                    }
                }
                Msg::StackDone(ok, msg) => {
                    self.stack_busy = false;
                    self.lite_ok = Some(crate::stack::lite_up());
                    self.status = msg.clone();
                    if !ok {
                        self.error = Some(msg);
                    }
                    self.rx = None;
                }
                Msg::Ev(ev) => match ev {
                    AmdlEvent::Line(line) => {
                        if let Some(p) = extract_percent(&line) {
                            self.progress = Some(p);
                        }
                        if line.starts_with("─────") {
                            self.job_info = line.clone();
                        }
                        self.log.push(line);
                        if self.log.len() > 2000 {
                            self.log.remove(0);
                        }
                    }
                    AmdlEvent::Done(code) => {
                        self.done_code = Some(code);
                        self.screen = Screen::Done;
                        self.rx = None;
                    }
                },
            }
        }

        if disconnected && handled == 0 {
            if self.screen == Screen::Loading {
                self.error = Some("Le chargement s'est interrompu.".into());
                self.screen = Screen::Input;
                self.rx = None;
            } else if self.stack_busy {
                self.stack_busy = false;
                self.error = Some("L'operation sur wrapper-lite s'est interrompue.".into());
                self.rx = None;
            }
        }
    }

    // ------------------------------------------------------- pile wrapper-lite

    pub fn toggle_stack(&mut self) {
        if self.stack_busy || self.rx.is_some() || self.screen == Screen::Running {
            return;
        }
        let (tx, rx): (Sender<Msg>, Receiver<Msg>) = channel();
        self.rx = Some(rx);
        self.stack_busy = true;
        self.stack_log.clear();
        self.error = None;

        std::thread::spawn(move || {
            let (ltx, lrx) = channel::<String>();
            let mtx = tx.clone();
            std::thread::spawn(move || {
                while let Ok(l) = lrx.recv() {
                    if mtx.send(Msg::StackLine(l)).is_err() {
                        break;
                    }
                }
            });
            let res = crate::stack::toggle(&ltx);
            drop(ltx);
            let (ok, msg) = match res {
                Ok(true) => (true, "wrapper-lite : en marche".to_string()),
                Ok(false) => (true, "wrapper-lite : arrete".to_string()),
                Err(e) => (false, e),
            };
            let _ = tx.send(Msg::StackDone(ok, msg));
        });
    }

    // ------------------------------------------------------- clavier

    pub fn on_paste(&mut self, s: String) {
        if self.screen == Screen::Input {
            self.input.push_str(&s);
        }
    }

    pub fn on_key(&mut self, code: KeyCode, mods: KeyModifiers) -> bool {
        // Ctrl+S / F2 : demarrer ou arreter wrapper-lite
        if (code == KeyCode::Char('s') && mods.contains(KeyModifiers::CONTROL)) || code == KeyCode::F(2)
        {
            if self.screen == Screen::Running {
                self.status = "Arret impossible pendant un telechargement.".into();
            } else {
                self.toggle_stack();
            }
            return false;
        }
        if code == KeyCode::Char('c') && mods.contains(KeyModifiers::CONTROL) {
            return true;
        }

        match self.screen {
            Screen::Input => match code {
                KeyCode::Enter => self.load_url(),
                KeyCode::Esc => {
                    if self.input.is_empty() {
                        return true;
                    }
                    self.input.clear();
                    self.error = None;
                }
                KeyCode::Backspace => {
                    self.input.pop();
                }
                KeyCode::Char(c) => self.input.push(c),
                _ => {}
            },
            Screen::Loading => {
                if code == KeyCode::Esc || code == KeyCode::Char('q') {
                    self.screen = Screen::Input;
                    self.rx = None;
                }
            }
            Screen::Ready => {
                let rows = self.rows();
                match code {
                    KeyCode::Char('q') => return true,
                    KeyCode::Esc => {
                        self.screen = Screen::Input;
                        self.error = None;
                    }
                    KeyCode::Tab | KeyCode::BackTab => {
                        self.focus = if self.focus == Focus::Tracks {
                            Focus::Options
                        } else {
                            Focus::Tracks
                        };
                    }
                    KeyCode::Up | KeyCode::Char('k') => match self.focus {
                        Focus::Tracks => self.move_cursor(-1),
                        Focus::Options => {
                            if self.opt_cursor > 0 {
                                self.opt_cursor -= 1;
                            }
                        }
                    },
                    KeyCode::Down | KeyCode::Char('j') => match self.focus {
                        Focus::Tracks => self.move_cursor(1),
                        Focus::Options => {
                            if self.opt_cursor + 1 < rows.len() {
                                self.opt_cursor += 1;
                            }
                        }
                    },
                    KeyCode::Right | KeyCode::Char('l') => match self.focus {
                        Focus::Tracks => {
                            if let Some(RowRef::Album(ai)) = self.current_row() {
                                let expanded = self
                                    .item
                                    .as_ref()
                                    .and_then(|i| i.albums.get(ai))
                                    .map(|a| a.expanded)
                                    .unwrap_or(false);
                                if !expanded {
                                    self.toggle_expand(ai);
                                } else {
                                    self.move_cursor(1);
                                }
                            }
                        }
                        Focus::Options => {
                            if let Some(r) = rows.get(self.opt_cursor).copied() {
                                self.cycle_row(r, 1);
                            }
                        }
                    },
                    KeyCode::Left | KeyCode::Char('h') => match self.focus {
                        Focus::Tracks => {
                            if let Some(RowRef::Album(ai)) = self.current_row() {
                                let expanded = self
                                    .item
                                    .as_ref()
                                    .and_then(|i| i.albums.get(ai))
                                    .map(|a| a.expanded)
                                    .unwrap_or(false);
                                if expanded {
                                    self.toggle_expand(ai);
                                }
                            }
                        }
                        Focus::Options => {
                            if let Some(r) = rows.get(self.opt_cursor).copied() {
                                self.cycle_row(r, -1);
                            }
                        }
                    },
                    KeyCode::Char(' ') if self.focus == Focus::Tracks => match self.current_row() {
                        Some(RowRef::Album(ai)) => self.toggle_album_all(ai),
                        Some(RowRef::Track(ai, ti)) => self.toggle_track(ai, ti),
                        None => {}
                    },
                    KeyCode::Char(' ') => {
                        if let Some(r) = rows.get(self.opt_cursor).copied() {
                            self.cycle_row(r, 1);
                        }
                    }
                    KeyCode::Enter => match self.focus {
                        Focus::Tracks => match self.current_row() {
                            Some(RowRef::Album(ai)) => self.toggle_expand(ai),
                            _ => self.start_download(),
                        },
                        Focus::Options => self.start_download(),
                    },
                    KeyCode::Char('a') => self.set_all(true),
                    KeyCode::Char('n') => self.set_all(false),
                    KeyCode::Char('i') => self.invert_all(),
                    KeyCode::Char('e') => {
                        if let Some(RowRef::Album(ai)) = self.current_row() {
                            if self
                                .item
                                .as_ref()
                                .and_then(|i| i.albums.get(ai))
                                .map(|a| !a.expanded)
                                .unwrap_or(false)
                            {
                                self.toggle_expand(ai);
                            }
                        }
                    }
                    KeyCode::Char('c') => {
                        if let Some(RowRef::Album(ai)) = self.current_row() {
                            if self
                                .item
                                .as_ref()
                                .and_then(|i| i.albums.get(ai))
                                .map(|a| a.expanded)
                                .unwrap_or(false)
                            {
                                self.toggle_expand(ai);
                            }
                        }
                    }
                    _ => {}
                }
            }
            Screen::Running => {
                if code == KeyCode::Char('q') && !mods.contains(KeyModifiers::CONTROL) {
                    return true;
                }
                if code == KeyCode::Esc {
                    return true;
                }
            }
            Screen::Done => match code {
                KeyCode::Char('q') | KeyCode::Esc => return true,
                KeyCode::Enter => {
                    self.screen = Screen::Input;
                    self.input.clear();
                    self.log.clear();
                    self.progress = None;
                    self.error = None;
                    self.item = None;
                }
                _ => {}
            },
        }
        false
    }
}

fn yesno(b: bool) -> String {
    if b {
        "oui".into()
    } else {
        "non".into()
    }
}

fn extract_percent(line: &str) -> Option<u8> {
    let bytes = line.as_bytes();
    let mut num: u32 = 0;
    let mut has = false;
    for (i, b) in bytes.iter().enumerate() {
        if *b == b'%' {
            if has {
                return Some(num.min(100) as u8);
            }
        } else if b.is_ascii_digit() {
            num = num.saturating_mul(10).saturating_add((*b - b'0') as u32);
            has = true;
        } else if *b == b' ' && has && num > 100 {
            return None;
        } else if has && !b.is_ascii_digit() && *b != b' ' {
            num = 0;
            has = false;
        }
        let _ = i;
    }
    None
}

pub fn read_storefront() -> String {
    let path = amdl::source_config();
    if let Ok(txt) = std::fs::read_to_string(&path) {
        for line in txt.lines() {
            let l = line.trim();
            if let Some(rest) = l.strip_prefix("storefront:") {
                let v = rest.trim().trim_matches('"').trim();
                if !v.is_empty() && !v.contains("enter your") {
                    return v.to_string();
                }
            }
        }
    }
    "us".to_string()
}
