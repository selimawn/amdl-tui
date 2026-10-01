// Lance le binaire amdl avec une config.yaml dediee et streame sa sortie.
//
// Deux details importants :
//  * amdl lit `config.yaml` dans le repertoire courant -> on genere la notre
//    dans un dossier de travail dedie (et pas dans le depot du downloader).
//  * on force `exit-on-error: true`, sinon amdl attend une touche Entree en cas
//    d'erreur et le TUI resterait bloque pour toujours.

use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Alac,
    Flac,
    Atmos,
    Aac,
}

impl Format {
    pub fn label(&self) -> &'static str {
        match self {
            Format::Alac => "ALAC (.m4a, sans conversion)",
            Format::Flac => "FLAC (conversion ffmpeg)",
            Format::Atmos => "Dolby Atmos (EC-3)",
            Format::Aac => "AAC 256 kbps",
        }
    }
    pub fn short(&self) -> &'static str {
        match self {
            Format::Alac => "ALAC",
            Format::Flac => "FLAC",
            Format::Atmos => "Atmos",
            Format::Aac => "AAC",
        }
    }
}

pub const ALAC_MAXES: [(u32, &str); 4] = [
    (192000, "192 kHz (max)"),
    (96000, "96 kHz"),
    (48000, "48 kHz"),
    (44100, "44.1 kHz"),
];

pub const LRC_TYPES: [&str; 2] = ["lyrics", "syllable-lyrics"];
pub const LRC_EXTRAS: [&str; 3] = ["", "translation", "pronunciation"];

pub struct Options {
    pub format: Format,
    pub alac_max_idx: usize,
    pub embed_lrc: bool,
    pub save_lrc_file: bool,
    pub lrc_type_idx: usize,
    pub lrc_extra_idx: usize,
    pub keep_original: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            format: Format::Alac,
            alac_max_idx: 0, // 192 kHz : le maximum, recommande par defaut
            embed_lrc: true,
            save_lrc_file: false,
            lrc_type_idx: 0,
            lrc_extra_idx: 0,
            keep_original: false,
        }
    }
}

pub fn home() -> PathBuf {
    std::env::var("HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("."))
}

pub fn amdl_bin() -> PathBuf {
    home().join("apple-music-downloader/amdl")
}

pub fn source_config() -> PathBuf {
    home().join("apple-music-downloader/config.yaml")
}

pub fn work_dir() -> PathBuf {
    home().join("Library/Caches/amdl-tui")
}

pub fn default_save_dir() -> PathBuf {
    home().join("Desktop/Musiques")
}

pub enum Event {
    Line(String),
    /// Code de sortie du processus.
    Done(i32),
}

/// Ecrit une config.yaml derivee de celle du downloader, avec surcharges.
fn build_config(overrides: &[(&str, String)]) -> Result<PathBuf, String> {
    let src = source_config();
    let dir = work_dir();
    fs::create_dir_all(&dir).map_err(|e| format!("creation {} : {e}", dir.display()))?;
    let dst = dir.join("config.yaml");

    let original = fs::read_to_string(&src)
        .map_err(|e| format!("lecture {} : {e}", src.display()))?;

    let mut applied: Vec<&str> = Vec::new();
    let mut out = String::with_capacity(original.len() + 512);

    for line in original.lines() {
        let trimmed = line.trim_start();
        // cles plates de premier niveau uniquement
        if !trimmed.starts_with('#') && !trimmed.starts_with('-') {
            if let Some((key, _)) = trimmed.split_once(':') {
                let key = key.trim();
                if let Some((_, val)) = overrides.iter().find(|(k, _)| *k == key) {
                    out.push_str(&format!("{key}: {val}\n"));
                    applied.push(key);
                    continue;
                }
            }
        }
        out.push_str(line);
        out.push('\n');
    }

    for (k, v) in overrides {
        if !applied.contains(k) {
            out.push_str(&format!("{k}: {v}\n"));
        }
    }

    fs::write(&dst, out).map_err(|e| format!("ecriture {} : {e}", dst.display()))?;
    Ok(dir)
}

fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "'"))
}

/// Convertit des positions 1-based triees en "1,3,5-7".
pub fn format_selection(positions: &[usize]) -> String {
    let mut sorted: Vec<usize> = positions.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let mut parts: Vec<String> = Vec::new();
    let mut i = 0;
    while i < sorted.len() {
        let start = sorted[i];
        let mut end = start;
        while i + 1 < sorted.len() && sorted[i + 1] == end + 1 {
            end = sorted[i + 1];
            i += 1;
        }
        if end > start {
            parts.push(format!("{start}-{end}"));
        } else {
            parts.push(format!("{start}"));
        }
        i += 1;
    }
    parts.join(",")
}

fn stream_reader<R: Read + Send + 'static>(r: R, tx: Sender<Event>) {
    std::thread::spawn(move || {
        let mut rd = r;
        let mut buf = [0u8; 8192];
        let mut acc: Vec<u8> = Vec::new();
        loop {
            match rd.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    for &b in &buf[..n] {
                        // amdl ecrit ses barres de progression avec \r
                        if b == b'\n' || b == b'\r' {
                            if !acc.is_empty() {
                                let line = String::from_utf8_lossy(&acc).trim().to_string();
                                acc.clear();
                                if !line.is_empty() {
                                    let _ = tx.send(Event::Line(line));
                                }
                            }
                        } else {
                            acc.push(b);
                        }
                    }
                }
                Err(_) => break,
            }
        }
        if !acc.is_empty() {
            let line = String::from_utf8_lossy(&acc).trim().to_string();
            if !line.is_empty() {
                let _ = tx.send(Event::Line(line));
            }
        }
    });
}

/// Description complete de l'execution amdl, sans l'executer.
pub struct Plan {
    pub workdir: PathBuf,
    pub args: Vec<String>,
    pub stdin: Option<String>,
}

impl Plan {
    pub fn command_line(&self) -> String {
        format!("amdl {}", self.args.join(" "))
    }
}

/// Construit la config.yaml dediee et la ligne de commande amdl.
pub fn plan(
    storefront: &str,
    url: &str,
    selection: Option<&[usize]>,
    opts: &Options,
    save_dir: &PathBuf,
) -> Result<Plan, String> {
    let dir_str = save_dir.to_string_lossy().to_string();
    let overrides: Vec<(&str, String)> = vec![
        ("storefront", quote(storefront)),
        ("lite-server", quote("http://127.0.0.1:12340")),
        ("alac-save-folder", quote(&dir_str)),
        ("atmos-save-folder", quote(&dir_str)),
        ("aac-save-folder", quote(&dir_str)),
        ("mv-save-folder", quote(&dir_str)),
        ("exit-on-error", "true".into()),
        ("get-m3u8-mode", "hires".into()),
        ("embed-lrc", opts.embed_lrc.to_string()),
        ("save-lrc-file", opts.save_lrc_file.to_string()),
        (
            "lrc-type",
            quote(LRC_TYPES[opts.lrc_type_idx.min(LRC_TYPES.len() - 1)]),
        ),
        (
            "lrc-extra",
            quote(LRC_EXTRAS[opts.lrc_extra_idx.min(LRC_EXTRAS.len() - 1)]),
        ),
        (
            "alac-max",
            ALAC_MAXES[opts.alac_max_idx.min(ALAC_MAXES.len() - 1)]
                .0
                .to_string(),
        ),
        ("atmos-max", "2768".into()),
        ("convert-format", quote("flac")),
        (
            "convert-after-download",
            (opts.format == Format::Flac).to_string(),
        ),
        ("convert-keep-original", opts.keep_original.to_string()),
    ];

    let wd = build_config(&overrides)?;

    let mut args: Vec<String> = Vec::new();
    match opts.format {
        Format::Atmos => {
            args.push("--atmos".into());
            args.push("--atmos-max".into());
            args.push("2768".into());
        }
        Format::Aac => {
            args.push("--aac".into());
            args.push("--aac-type".into());
            args.push("aac-lc".into());
        }
        Format::Alac | Format::Flac => {
            args.push("--alac-max".into());
            args.push(
                ALAC_MAXES[opts.alac_max_idx.min(ALAC_MAXES.len() - 1)]
                    .0
                    .to_string(),
            );
        }
    }

    let stdin = if let Some(sel) = selection {
        args.push("--select".into());
        Some(format_selection(sel))
    } else {
        None
    };
    args.push(url.to_string());

    Ok(Plan {
        workdir: wd,
        args,
        stdin,
    })
}

/// Execute un plan prepare et pousse la sortie dans `tx`.
/// Envoie `Event::Done` a la fin.
pub fn run_plan(p: &Plan, tx: Sender<Event>) -> Result<(), String> {
    let code = run_plan_quiet(p, tx.clone())?;
    let _ = tx.send(Event::Done(code));
    Ok(())
}

/// Comme `run_plan` mais SANS envoyer `Done` : utilise quand plusieurs
/// commandes s'enchainent et qu'un seul `Done` doit conclure l'ensemble.
pub fn run_plan_quiet(p: &Plan, tx: Sender<Event>) -> Result<i32, String> {
    let bin = amdl_bin();
    if !bin.exists() {
        return Err(format!("binaire amdl introuvable : {}", bin.display()));
    }

    let mut cmd = Command::new(&bin);
    // ~/.cargo/bin est indispensable : amdl charge son cdylib temari via le
    // binding Go, dont le chemin « bundle » est bugue sur macOS (il cherche
    // lib/darwin-arm64 alors que le dossier livre s'appelle lib/macos-arm64).
    // Il retombe donc sur un self-build Rust qui, meme s'il est deja en cache,
    // exige `cargo` dans le PATH.
    let cargo_bin = home().join(".cargo/bin");
    let path_env = format!(
        "{}:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin",
        cargo_bin.display()
    );
    cmd.args(&p.args)
        .current_dir(&p.workdir)
        .env("PATH", path_env)
        .stdin(if p.stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().map_err(|e| format!("lancement amdl : {e}"))?;

    if let Some(payload) = &p.stdin {
        if let Some(mut si) = child.stdin.take() {
            // amdl attend une ligne du type "1,3,5-7" puis Entree
            let _ = si.write_all(payload.as_bytes());
            let _ = si.write_all(b"\n");
            let _ = si.flush();
            drop(si);
        }
    }

    if let Some(out) = child.stdout.take() {
        stream_reader(out, tx.clone());
    }
    if let Some(err) = child.stderr.take() {
        stream_reader(err, tx.clone());
    }

    let status = child.wait().map_err(|e| format!("attente amdl : {e}"))?;
    Ok(status.code().unwrap_or(-1))
}

/// Prepare puis execute amdl, pour une seule commande.
pub fn spawn(
    storefront: &str,
    url: &str,
    selection: Option<&[usize]>,
    opts: &Options,
    save_dir: &PathBuf,
    tx: Sender<Event>,
) -> Result<(), String> {
    let p = plan(storefront, url, selection, opts, save_dir)?;
    run_plan(&p, tx)
}

/// Verifie que wrapper-lite repond sur /status.
pub fn lite_status() -> Result<String, String> {
    let mut resp = ureq::agent()
        .get("http://127.0.0.1:12340/status")
        .config()
        .timeout_global(Some(std::time::Duration::from_secs(3)))
        .build()
        .call()
        .map_err(|e| e.to_string())?;
    let txt = resp
        .body_mut()
        .read_to_string()
        .map_err(|e| e.to_string())?;
    Ok(txt)
}
