// Etat de l'application + gestion des entrees clavier.

use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};

use crate::amdl::{self, Format, Event as AmdlEvent, Options, ALAC_MAXES, LRC_EXTRAS, LRC_TYPES};
use crate::api::{self, Item, Kind};

#[derive(PartialEq, Eq)]
pub enum Screen {
    Input,
    Loading,
    Ready,
    Running,
    Done,
}

#[derive(PartialEq, Eq)]
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

enum Msg {
    Loaded(Box<Item>, String),
    LoadErr(String),
    Ev(AmdlEvent),
    StackLine(String),
    StackDone(bool, String),
}

/// Journal de diagnostic, actif seulement si AMDL_TUI_DEBUG est defini.
pub fn dbg(msg: &str) {
    if std::env::var_os("AMDL_TUI_DEBUG").is_none() {
        return;
    }
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/amdl-tui-debug.log")
    {
        let _ = writeln!(f, "{msg}");
    }
}

pub struct App {
    pub screen: Screen,
    pub input: String,
    pub status: String,
    pub error: Option<String>,
    pub item: Option<Item>,
    pub checks: Vec<bool>,
    pub cursor: usize,
    pub focus: Focus,
    pub opt_cursor: usize,
    pub opts: Options,
    pub log: Vec<String>,
    pub progress: Option<u8>,
    pub done_code: Option<i32>,
    pub should_quit: bool,
    pub save_dir: PathBuf,
    pub storefront: String,
    pub target_url: String,
    pub lite_ok: Option<bool>,
    pub spinner: usize,
    /// Une operation de demarrage/arret de wrapper-lite est en cours.
    pub stack_busy: bool,
    pub stack_log: Vec<String>,
    rx: Option<Receiver<Msg>>,
}

impl App {
    pub fn new() -> Self {
        let storefront = read_storefront();
        let save_dir = amdl::default_save_dir();
        Self {
            screen: Screen::Input,
            input: String::new(),
            status: String::new(),
            error: None,
            item: None,
            checks: Vec::new(),
            cursor: 0,
            focus: Focus::Tracks,
            opt_cursor: 0,
            opts: Options::default(),
            log: Vec::new(),
            progress: None,
            done_code: None,
            should_quit: false,
            save_dir,
            storefront,
            target_url: String::new(),
            lite_ok: Some(amdl::lite_status().is_ok()),
            spinner: 0,
            stack_busy: false,
            stack_log: Vec::new(),
            rx: None,
        }
    }

    // ------------------------------------------------------------ chargement

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
        self.lite_ok = amdl::lite_status().ok().map(|_| true);

        let sf = self.storefront.clone();
        std::thread::spawn(move || {
            dbg("thread: demarrage");
            match api::parse_url(&raw, &sf) {
                Err(e) => {
                    dbg(&format!("thread: parse_url err {e}"));
                    let _ = tx.send(Msg::LoadErr(e));
                }
                Ok(p) => {
                    if p.kind == Kind::Artist {
                        let _ = tx.send(Msg::LoadErr(
                            "Les pages artiste ne sont pas gerees : utilise une URL d'album.".into(),
                        ));
                        return;
                    }
                    let storefront = p.storefront.clone().unwrap_or_else(|| sf.clone());
                    dbg(&format!("thread: storefront={storefront} kind={:?}", p.kind));
                    let token = match api::get_token() {
                        Ok(t) => t,
                        Err(e) => {
                            dbg(&format!("thread: token err {e}"));
                            let _ = tx.send(Msg::LoadErr(e));
                            return;
                        }
                    };
                    dbg("thread: token ok");
                    match api::fetch_catalog(&storefront, p.kind, &p.id, &token, p.track_id.clone())
                    {
                        Ok(item) => {
                            dbg(&format!("thread: catalogue ok, {} pistes", item.tracks.len()));
                            let _ = tx.send(Msg::Loaded(Box::new(item), p.raw.clone()));
                            dbg("thread: Loaded envoye");
                        }
                        Err(e) => {
                            dbg(&format!("thread: catalogue err {e}"));
                            let _ = tx.send(Msg::LoadErr(e));
                        }
                    }
                }
            }
            dbg("thread: fin");
        });
    }

    // ------------------------------------------------------------ selection

    pub fn checked_positions(&self) -> Vec<usize> {
        self.checks
            .iter()
            .enumerate()
            .filter(|(_, c)| **c)
            .map(|(i, _)| i + 1)
            .collect()
    }

    /// Mode « une seule piste ciblee par l'URL » ?
    fn single_mode(&self) -> bool {
        match &self.item {
            Some(it) if it.kind == Kind::Song => true,
            Some(it) => match it.forced_track {
                Some(idx) => {
                    let sel = self.checked_positions();
                    sel.len() == 1 && sel[0] == idx + 1
                }
                None => false,
            },
            None => false,
        }
    }

    fn available_formats(&self) -> Vec<Format> {
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

    pub fn row_value(&self, row: Row) -> String {
        match row {
            Row::Format => self.opts.format.label().to_string(),
            Row::AlacMax => {
                let i = self.opts.alac_max_idx.min(ALAC_MAXES.len() - 1);
                let max = ALAC_MAXES[0].1;
                if i == 0 {
                    format!("{}  [recommande : {max}]", ALAC_MAXES[i].1)
                } else {
                    ALAC_MAXES[i].1.to_string()
                }
            }
            Row::EmbedLrc => yesno(self.opts.embed_lrc),
            Row::SaveLrc => yesno(self.opts.save_lrc_file),
            Row::LrcType => {
                let i = self.opts.lrc_type_idx.min(LRC_TYPES.len() - 1);
                let extra = if LRC_TYPES[i] == "syllable-lyrics" {
                    "  (mot-a-mot)"
                } else {
                    "  (lignes)"
                };
                format!("{}{extra}", LRC_TYPES[i])
            }
            Row::LrcExtra => {
                let i = self.opts.lrc_extra_idx.min(LRC_EXTRAS.len() - 1);
                if LRC_EXTRAS[i].is_empty() {
                    "aucune".to_string()
                } else {
                    LRC_EXTRAS[i].to_string()
                }
            }
            Row::KeepOriginal => yesno(self.opts.keep_original),
        }
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
                let next = ((cur as i32 + if fwd { 1 } else { -1 }).rem_euclid(n)) as usize;
                self.opts.format = avail[next];
            }
            Row::AlacMax => {
                let n = ALAC_MAXES.len() as i32;
                let cur = self.opts.alac_max_idx as i32;
                self.opts.alac_max_idx =
                    ((cur + if fwd { 1 } else { -1 }).rem_euclid(n)) as usize;
            }
            Row::EmbedLrc => self.opts.embed_lrc = !self.opts.embed_lrc,
            Row::SaveLrc => self.opts.save_lrc_file = !self.opts.save_lrc_file,
            Row::LrcType => {
                let n = LRC_TYPES.len() as i32;
                let cur = self.opts.lrc_type_idx as i32;
                self.opts.lrc_type_idx =
                    ((cur + if fwd { 1 } else { -1 }).rem_euclid(n)) as usize;
            }
            Row::LrcExtra => {
                let n = LRC_EXTRAS.len() as i32;
                let cur = self.opts.lrc_extra_idx as i32;
                self.opts.lrc_extra_idx =
                    ((cur + if fwd { 1 } else { -1 }).rem_euclid(n)) as usize;
            }
            Row::KeepOriginal => self.opts.keep_original = !self.opts.keep_original,
        }
    }

    // ------------------------------------------------------- pile wrapper-lite

    /// Demarre ou arrete la VM colima + le conteneur wrapper-lite.
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

    // ------------------------------------------------------------ download

    fn start_download(&mut self) {
        let item = match &self.item {
            Some(i) => i.clone(),
            None => return,
        };

        let single = self.single_mode();
        let selection: Option<Vec<usize>> = if single {
            None
        } else {
            let sel = self.checked_positions();
            if sel.is_empty() {
                self.error = Some("Aucune piste cochee (Espace pour cocher, « a » pour tout).".into());
                return;
            }
            Some(sel)
        };

        let url = if single {
            if self.target_url.is_empty() {
                item.base_url.clone()
            } else {
                self.target_url.clone()
            }
        } else {
            item.base_url.clone()
        };

        let (tx, rx): (Sender<Msg>, Receiver<Msg>) = channel();
        self.rx = Some(rx);
        self.screen = Screen::Running;
        self.error = None;
        self.log.clear();
        self.progress = None;
        self.done_code = None;
        self.status = format!(
            "{} -> {}",
            self.opts.format.short(),
            self.save_dir.display()
        );

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
        let sel_for_thread = selection.clone();

        std::thread::spawn(move || {
            // amdl::spawn emet des amdl::Event ; on les relaie vers le canal
            // de l'application en Msg::Ev.
            let (atx, arx) = channel::<AmdlEvent>();
            let mtx = tx.clone();
            std::thread::spawn(move || {
                while let Ok(ev) = arx.recv() {
                    if mtx.send(Msg::Ev(ev)).is_err() {
                        break;
                    }
                }
            });
            let res = amdl::spawn(
                &item,
                &url,
                sel_for_thread.as_deref(),
                &opts,
                &save_dir,
                atx,
            );
            if let Err(e) = res {
                let _ = tx.send(Msg::Ev(AmdlEvent::Line(format!("ERREUR: {e}"))));
                let _ = tx.send(Msg::Ev(AmdlEvent::Done(-1)));
            }
        });
    }

    // ------------------------------------------------------------ messages

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
                    dbg(&format!("poll: LoadErr {e}"));
                    self.error = Some(e);
                    self.screen = Screen::Input;
                    self.rx = None;
                }
                Msg::Loaded(item, raw) => {
                    dbg(&format!("poll: Loaded {} pistes", item.tracks.len()));
                    self.checks = vec![true; item.tracks.len()];
                    self.cursor = item.forced_track.unwrap_or(0);
                    // Si l'URL ciblait une piste precise, on ne coche qu'elle.
                    if let Some(idx) = item.forced_track {
                        for (i, c) in self.checks.iter_mut().enumerate() {
                            *c = i == idx;
                        }
                    }
                    self.status = if item.tracks.is_empty() {
                        "Aucune piste trouvee.".into()
                    } else {
                        format!("{} pistes chargees", item.tracks.len())
                    };
                    self.opts.format = if item.has_lossless() {
                        Format::Alac
                    } else if item.has_atmos() {
                        Format::Atmos
                    } else {
                        Format::Aac
                    };
                    self.opt_cursor = 0;
                    self.focus = if item.tracks.is_empty() {
                        Focus::Options
                    } else {
                        Focus::Tracks
                    };
                    self.target_url = raw.clone();
                    self.item = Some(*item);
                    self.screen = Screen::Ready;
                    self.rx = None;
                }
                Msg::StackLine(l) => {
                    self.stack_log.push(l);
                    if self.stack_log.len() > 300 {
                        self.stack_log.remove(0);
                    }
                }
                Msg::StackDone(ok, msg) => {
                    dbg(&format!("poll: StackDone ok={ok} {msg}"));
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
                        if line.contains("Completed:") || line.contains("Decrypted") {
                            self.status = line.clone();
                        }
                        self.log.push(line);
                        if self.log.len() > 500 {
                            self.log.remove(0);
                        }
                    }
                    AmdlEvent::Done(code) => {
                        dbg(&format!("poll: Done code={code}"));
                        self.done_code = Some(code);
                        self.screen = Screen::Done;
                        self.rx = None;
                    }
                },
            }
        }

        // Producteur disparu sans rien envoyer : on evite le blocage silencieux.
        if disconnected && handled == 0 {
            if self.screen == Screen::Loading {
                dbg("poll: canal deconnecte sans reponse -> erreur");
                self.error = Some(
                    "Le chargement s'est interrompu. Reessaie, ou verifie ta connexion.".into(),
                );
                self.screen = Screen::Input;
                self.rx = None;
            } else if self.stack_busy {
                dbg("poll: canal deconnecte pendant l'operation sur wrapper-lite");
                self.stack_busy = false;
                self.error = Some("L'operation sur wrapper-lite s'est interrompue.".into());
                self.rx = None;
            }
        }
    }

    // ------------------------------------------------------------ clavier

    pub fn on_paste(&mut self, s: String) {
        if self.screen == Screen::Input {
            self.input.push_str(&s);
        }
    }

    /// Retourne true s'il faut quitter.
    pub fn on_key(&mut self, code: ratatui::crossterm::event::KeyCode, _mods: ratatui::crossterm::event::KeyModifiers) -> bool {
        use ratatui::crossterm::event::KeyCode;

        // Ctrl+S ou F2 : demarrer / arreter wrapper-lite.
        // (Fn+S ne peut pas etre capte : le terminal recoit un simple « s ».)
        let ctrl = _mods.contains(ratatui::crossterm::event::KeyModifiers::CONTROL);
        if (code == KeyCode::Char('s') && ctrl) || code == KeyCode::F(2) {
            if self.screen == Screen::Running {
                self.status = "Arret impossible pendant un telechargement.".into();
            } else {
                self.toggle_stack();
            }
            return false;
        }

        match self.screen {
            Screen::Input => match code {
                KeyCode::Char('c') if _mods.contains(ratatui::crossterm::event::KeyModifiers::CONTROL) => {
                    return true
                }
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
                let n_tracks = self.item.as_ref().map(|i| i.tracks.len()).unwrap_or(0);
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
                        Focus::Tracks => {
                            if n_tracks > 0 && self.cursor > 0 {
                                self.cursor -= 1;
                            }
                        }
                        Focus::Options => {
                            if self.opt_cursor > 0 {
                                self.opt_cursor -= 1;
                            }
                        }
                    },
                    KeyCode::Down | KeyCode::Char('j') => match self.focus {
                        Focus::Tracks => {
                            if n_tracks > 0 && self.cursor + 1 < n_tracks {
                                self.cursor += 1;
                            }
                        }
                        Focus::Options => {
                            if self.opt_cursor + 1 < rows.len() {
                                self.opt_cursor += 1;
                            }
                        }
                    },
                    KeyCode::Char(' ') => match self.focus {
                        Focus::Tracks => {
                            if let Some(c) = self.checks.get_mut(self.cursor) {
                                *c = !*c;
                            }
                        }
                        Focus::Options => {
                            if let Some(r) = rows.get(self.opt_cursor).copied() {
                                self.cycle_row(r, 1);
                            }
                        }
                    },
                    KeyCode::Left | KeyCode::Char('h') => {
                        if self.focus == Focus::Options {
                            if let Some(r) = rows.get(self.opt_cursor).copied() {
                                self.cycle_row(r, -1);
                            }
                        }
                    }
                    KeyCode::Right | KeyCode::Char('l') => {
                        if self.focus == Focus::Options {
                            if let Some(r) = rows.get(self.opt_cursor).copied() {
                                self.cycle_row(r, 1);
                            }
                        }
                    }
                    KeyCode::Char('a') => {
                        for c in self.checks.iter_mut() {
                            *c = true;
                        }
                    }
                    KeyCode::Char('n') => {
                        for c in self.checks.iter_mut() {
                            *c = false;
                        }
                    }
                    KeyCode::Char('i') => {
                        for c in self.checks.iter_mut() {
                            *c = !*c;
                        }
                    }
                    KeyCode::Enter => self.start_download(),
                    _ => {}
                }
            }
            Screen::Running => {
                if code == KeyCode::Char('q') || code == KeyCode::Esc {
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
                }
                _ => {}
            },
        }
        false
    }
}

fn yesno(b: bool) -> String {
    if b { "oui".into() } else { "non".into() }
}

fn extract_percent(line: &str) -> Option<u8> {
    let bytes = line.as_bytes();
    let mut num: u32 = 0;
    let mut has = false;
    for i in 0..bytes.len() {
        if bytes[i] == b'%' {
            if has {
                return Some(num.min(100) as u8);
            }
        } else if bytes[i].is_ascii_digit() {
            num = num.saturating_mul(10).saturating_add((bytes[i] - b'0') as u32);
            has = true;
        } else if bytes[i] == b' ' && has && num > 100 {
            // evite de partir d'un nombre trop long, ex. "85 MB"
            return None;
        } else if has && !bytes[i].is_ascii_digit() && bytes[i] != b' ' {
            num = 0;
            has = false;
        }
    }
    None
}

/// Lit `storefront:` dans la config du downloader.
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
