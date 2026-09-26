mod amdl;
mod api;
mod app;
mod stack;
mod ui;

use std::io;
use std::sync::mpsc::channel;
use std::time::Duration;

use ratatui::crossterm::event::{self, Event, KeyEventKind};

use amdl::Format;

fn main() -> io::Result<()> {
    let argv: Vec<String> = std::env::args().collect();

    // Modes de diagnostic, sans interface graphique :
    //   amdl-tui --probe <url>   affiche les metadonnees + la commande generee
    //   amdl-tui --exec  <url>   lance reellement le telechargement (sortie brute)
    if argv.len() >= 3 && (argv[1] == "--probe" || argv[1] == "--exec") {
        let url = argv[2].clone();
        let exec = argv[1] == "--exec";
        let sel = flag_value(&argv, "--select").map(|s| parse_selection(&s));
        let fmt = match flag_value(&argv, "--format").as_deref() {
            Some("flac") => Format::Flac,
            Some("aac") => Format::Aac,
            Some("atmos") => Format::Atmos,
            _ => Format::Alac,
        };
        return debug_run(&url, sel, fmt, exec);
    }

    let mut terminal = ratatui::init();
    let mut app = app::App::new();
    let res = run(&mut terminal, &mut app);
    ratatui::restore();
    res
}

fn flag_value(argv: &[String], name: &str) -> Option<String> {
    argv.iter()
        .position(|a| a == name)
        .and_then(|i| argv.get(i + 1))
        .cloned()
}

fn parse_selection(s: &str) -> Vec<usize> {
    let mut out = Vec::new();
    for part in s.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if let Some((a, b)) = part.split_once('-') {
            if let (Ok(a), Ok(b)) = (a.trim().parse::<usize>(), b.trim().parse::<usize>()) {
                for i in a..=b {
                    out.push(i);
                }
            }
        } else if let Ok(n) = part.parse::<usize>() {
            out.push(n);
        }
    }
    out
}

fn debug_run(url: &str, selection: Option<Vec<usize>>, format: Format, exec: bool) -> io::Result<()> {
    let sf = app::read_storefront();
    let parsed = match api::parse_url(url, &sf) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("URL invalide : {e}");
            std::process::exit(2);
        }
    };
    println!("storefront : {}", parsed.storefront.clone().unwrap_or_default());
    println!("type       : {:?}", parsed.kind);
    println!("id         : {}", parsed.id);
    println!("piste ?i=  : {:?}", parsed.track_id);

    let token = match api::get_token() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("token : {e}");
            std::process::exit(3);
        }
    };
    println!("token      : {}…", &token[..token.len().min(40)]);

    let item = match api::fetch_catalog(
        parsed.storefront.as_deref().unwrap_or(&sf),
        parsed.kind,
        &parsed.id,
        &token,
        parsed.track_id.clone(),
    ) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("catalogue : {e}");
            std::process::exit(4);
        }
    };

    println!("\n=== {} — {} ===", item.title, item.artist);
    println!("pistes : {}   max : {}", item.tracks.len(), item.max_quality_label());
    println!("atmos  : {}   lossless : {}", item.has_atmos(), item.has_lossless());
    for (i, t) in item.tracks.iter().enumerate() {
        println!(
            "  {:>3}. {:<50} {}  [{}]",
            i + 1,
            t.name.chars().take(50).collect::<String>(),
            t.duration(),
            t.quality_tag()
        );
    }

    let mut opts = amdl::Options::default();
    opts.format = format;
    let save_dir = amdl::default_save_dir();

    let single = item.kind == api::Kind::Song
        || (item.forced_track.is_some() && {
            let s = selection.clone();
            match s {
                None => true,
                Some(v) => v.len() == 1 && Some(v[0] - 1) == item.forced_track,
            }
        });
    let sel_for_plan = if single { None } else { selection.as_deref() };
    let use_url = if single { url.to_string() } else { item.base_url.clone() };

    let p = match amdl::plan(&item, &use_url, sel_for_plan, &opts, &save_dir) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("plan : {e}");
            std::process::exit(5);
        }
    };

    println!("\nconfig      : {}/config.yaml", p.workdir.display());
    println!("stdin       : {:?}", p.stdin);
    println!("commande    : {}", p.command_line());

    if !exec {
        println!("\n--- config.yaml genere ---");
        if let Ok(txt) = std::fs::read_to_string(p.workdir.join("config.yaml")) {
            for line in txt.lines() {
                let l = line.trim();
                for k in [
                    "storefront",
                    "alac-save-folder",
                    "convert-after-download",
                    "convert-format",
                    "alac-max",
                    "embed-lrc",
                    "lrc-type",
                    "exit-on-error",
                    "lite-server",
                ] {
                    if l.starts_with(&format!("{k}:")) {
                        println!("  {l}");
                    }
                }
            }
        }
        return Ok(());
    }

    println!("\n--- execution ---");
    let (tx, rx) = channel::<amdl::Event>();
    std::thread::spawn(move || {
        let _ = amdl::run_plan(&p, tx);
    });
    for ev in rx {
        match ev {
            amdl::Event::Line(l) => println!("{l}"),
            amdl::Event::Done(c) => {
                println!("--- code de sortie : {c} ---");
                break;
            }
        }
    }
    Ok(())
}

fn run(terminal: &mut ratatui::DefaultTerminal, app: &mut app::App) -> io::Result<()> {
    loop {
        app.poll();
        terminal.draw(|f| ui::draw(f, app))?;

        if event::poll(Duration::from_millis(80))? {
            match event::read()? {
                Event::Key(k) if k.kind == KeyEventKind::Press => {
                    if app.on_key(k.code, k.modifiers) {
                        break;
                    }
                }
                Event::Paste(s) => app.on_paste(s),
                _ => {}
            }
        }
    }
    Ok(())
}
