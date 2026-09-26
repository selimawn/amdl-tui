// Rendu de l'interface.

use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::app::{App, Focus, Screen};

const ACCENT: Color = Color::Rgb(250, 45, 72);
const DIM: Color = Color::Rgb(135, 135, 145);
const OK: Color = Color::Rgb(85, 200, 125);
const WARN: Color = Color::Rgb(240, 180, 70);

fn pad_trunc(s: &str, w: usize) -> String {
    let count = s.chars().count();
    if count > w {
        let mut out: String = s.chars().take(w.saturating_sub(1)).collect();
        out.push('…');
        out
    } else {
        format!("{s}{}", " ".repeat(w - count))
    }
}

pub fn draw(f: &mut Frame, app: &App) {
    let [head, body, foot] =
        Layout::vertical([Constraint::Length(4), Constraint::Min(6), Constraint::Length(2)])
            .areas(f.area());
    header(f, app, head);
    match app.screen {
        Screen::Input => input(f, app, body),
        Screen::Loading => loading(f, app, body),
        Screen::Ready => ready(f, app, body),
        Screen::Running => running(f, app, body),
        Screen::Done => done(f, app, body),
    }
    footer(f, app, foot);
    if app.stack_busy {
        stack_overlay(f, app);
    }
}

/// Fenetre superposee pendant le demarrage / l'arret de wrapper-lite.
fn stack_overlay(f: &mut Frame, app: &App) {
    let area = f.area();
    if area.width < 30 || area.height < 8 {
        return;
    }
    let w = area.width.saturating_sub(8).min(96);
    let h = area.height.saturating_sub(6).min(16);
    let rect = Rect::new((area.width - w) / 2, (area.height - h) / 2, w, h);
    f.render_widget(Clear, rect);

    let visible = (h as usize).saturating_sub(2);
    let start = app.stack_log.len().saturating_sub(visible);
    let mut lines: Vec<Line> = app
        .stack_log
        .iter()
        .skip(start)
        .map(|l| {
            let col = if l.contains("echoue") || l.contains("impossible") {
                ACCENT
            } else if l.contains("pret") || l.contains("marche") || l.contains("arretee") {
                OK
            } else {
                Color::Rgb(205, 205, 215)
            };
            Line::from(Span::styled(format!("  {l}"), Style::new().fg(col)))
        })
        .collect();
    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "  preparation...",
            Style::new().fg(DIM),
        )));
    }
    let title = if app.lite_ok == Some(true) {
        " Arret de wrapper-lite... "
    } else {
        " Demarrage de wrapper-lite... "
    };
    f.render_widget(
        Paragraph::new(lines)
            .block(
                Block::bordered()
                    .border_style(Style::new().fg(WARN))
                    .title(Span::styled(
                        title,
                        Style::new().fg(WARN).add_modifier(Modifier::BOLD),
                    )),
            )
            .wrap(Wrap { trim: true }),
        rect,
    );
}

fn header(f: &mut Frame, app: &App, area: Rect) {
    let mut lines: Vec<Line> = Vec::new();
    match &app.item {
        Some(it) if app.screen != Screen::Input => {
            let kind = if it.tracks.len() > 1 {
                format!("{} · {} pistes", it.kind.label(), it.tracks.len())
            } else {
                it.kind.label().to_string()
            };
            lines.push(Line::from(vec![
                Span::styled("  ♪  ", Style::new().fg(ACCENT)),
                Span::styled(
                    it.title.clone(),
                    Style::new().fg(Color::White).add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!("  —  {}", it.artist), Style::new().fg(DIM)),
            ]));
            lines.push(Line::from(vec![
                Span::styled(format!("     {kind}"), Style::new().fg(DIM)),
                Span::styled(
                    format!("   ·   storefront « {} »", it.storefront.to_uppercase()),
                    Style::new().fg(DIM),
                ),
            ]));
        }
        _ => {
            lines.push(Line::from(vec![
                Span::styled("  ♪  ", Style::new().fg(ACCENT)),
                Span::styled(
                    "amdl-tui",
                    Style::new().fg(Color::White).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    "   —   telechargeur Apple Music (wrapper-lite)",
                    Style::new().fg(DIM),
                ),
            ]));
            let lite = match app.lite_ok {
                Some(true) => Span::styled("wrapper-lite : connecte  ", Style::new().fg(OK)),
                _ => Span::styled(
                    "wrapper-lite : HORS LIGNE (Ctrl+S pour demarrer)  ",
                    Style::new().fg(WARN),
                ),
            };
            lines.push(Line::from(vec![
                Span::styled("     ", Style::new()),
                lite,
                Span::styled(
                    format!("· dossier : {}", app.save_dir.display()),
                    Style::new().fg(DIM),
                ),
            ]));
        }
    }
    let blk = Block::bordered()
        .border_style(Style::new().fg(ACCENT))
        .title(Span::styled(
            " Apple Music ",
            Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
        ));
    f.render_widget(Paragraph::new(lines).block(blk), area);
}

fn input(f: &mut Frame, app: &App, area: Rect) {
    let [top, mid] = Layout::vertical([Constraint::Length(5), Constraint::Min(4)]).areas(area);

    let cursor = if app.screen == Screen::Input { "▌" } else { "" };
    let txt = if app.input.is_empty() {
        Line::from(Span::styled(
            "Colle ici une URL music.apple.com puis Entree…",
            Style::new().fg(DIM),
        ))
    } else {
        Line::from(Span::styled(
            app.input.clone(),
            Style::new().fg(Color::White),
        ))
    };
    let url_blk = Block::bordered()
        .border_style(Style::new().fg(if app.input.is_empty() { DIM } else { ACCENT }))
        .title(" URL (titre, album ou playlist) ");
    f.render_widget(
        Paragraph::new(Text::from(vec![txt, Line::from(Span::styled(cursor, Style::new().fg(ACCENT))) ]))
            .block(url_blk)
            .wrap(Wrap { trim: true }),
        top,
    );

    let mut lines: Vec<Line> = Vec::new();
    if let Some(e) = &app.error {
        lines.push(Line::from(Span::styled(
            format!("  ⚠  {e}"),
            Style::new().fg(ACCENT),
        )));
        lines.push(Line::from(""));
    }
    lines.push(Line::from(Span::styled(
        "  Exemples d'URL :",
        Style::new().fg(DIM).add_modifier(Modifier::BOLD),
    )));
    for ex in [
        "https://music.apple.com/tr/album/at-the-bbc/1555697650            (album complet)",
        "https://music.apple.com/tr/album/.../1555697650?i=1555697872       (une piste precise)",
        "https://music.apple.com/tr/song/rehab/1555697872                  (un titre)",
        "https://music.apple.com/tr/playlist/.../pl.xxxxxxxxxxxxxxxx        (playlist)",
    ] {
        lines.push(Line::from(Span::styled(format!("    {ex}"), Style::new().fg(DIM))));
    }
    if let Some(s) = Some(&app.status) {
        if !s.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                format!("  {s}"),
                Style::new().fg(OK),
            )));
        }
    }
    f.render_widget(
        Paragraph::new(lines).block(Block::bordered().border_style(Style::new().fg(DIM))),
        mid,
    );
}

fn loading(f: &mut Frame, app: &App, area: Rect) {
    let frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
    let sp = frames[app.spinner % frames.len()];
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("        {sp}  {}", app.status),
            Style::new().fg(WARN),
        )),
    ];
    f.render_widget(
        Paragraph::new(lines)
            .block(Block::bordered().border_style(Style::new().fg(DIM)).title(" Chargement ")),
        area,
    );
}

fn ready(f: &mut Frame, app: &App, area: Rect) {
    let item = match &app.item {
        Some(i) => i,
        None => return,
    };

    let rows = app.rows();
    // +2 pour la bordure, +1 pour la ligne « Max detecte » du bas.
    let opts_h = (rows.len() as u16 + 3).min(area.height.saturating_sub(3));
    let [tracks_area, opts_area] =
        Layout::vertical([Constraint::Min(3), Constraint::Length(opts_h)]).areas(area);

    // ---- pistes
    let focused_tracks = app.focus == Focus::Tracks;
    let border = if focused_tracks { ACCENT } else { DIM };
    let title = if item.tracks.is_empty() {
        " Pistes ".to_string()
    } else {
        let n = app.checked_positions().len();
        format!(" Pistes — {n}/{} cochee(s) ", item.tracks.len())
    };

    if item.tracks.is_empty() {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "  (titre seul — pas de liste de pistes)",
                Style::new().fg(DIM),
            )))
            .block(Block::bordered().border_style(Style::new().fg(border)).title(title)),
            tracks_area,
        );
    } else {
        let name_w = item
            .tracks
            .iter()
            .map(|t| t.name.chars().count())
            .max()
            .unwrap_or(10)
            .clamp(12, 58);
        let items: Vec<ListItem> = item
            .tracks
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let checked = app.checks.get(i).copied().unwrap_or(false);
                let box_ = if checked { "[x]" } else { "[ ]" };
                let disc = if t.disc_number > 1 {
                    format!("D{}·", t.disc_number)
                } else {
                    String::new()
                };
                let quality = t.quality_tag();
                let qcol = match quality.as_str() {
                    "Hi-Res" => OK,
                    "Lossless" => Color::Rgb(120, 200, 255),
                    "Atmos" => Color::Rgb(200, 140, 255),
                    _ => DIM,
                };
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("{box_} "),
                        Style::new().fg(if checked { OK } else { DIM }),
                    ),
                    Span::styled(
                        format!("{disc}{:>2}. ", t.track_number),
                        Style::new().fg(DIM),
                    ),
                    Span::styled(
                        pad_trunc(&t.name, name_w),
                        Style::new().fg(if checked { Color::White } else { DIM }),
                    ),
                    Span::styled(
                        if t.is_video() { "  [clip]" } else { "" },
                        Style::new().fg(WARN),
                    ),
                    Span::styled(format!("  {}", t.duration()), Style::new().fg(DIM)),
                    Span::styled(format!("  {quality}"), Style::new().fg(qcol)),
                ]))
            })
            .collect();

        let mut st = ListState::default();
        st.select(Some(app.cursor));
        let list = List::new(items)
            .block(Block::bordered().border_style(Style::new().fg(border)).title(title))
            .highlight_style(if focused_tracks {
                Style::new().bg(Color::Rgb(45, 45, 55)).add_modifier(Modifier::BOLD)
            } else {
                Style::new()
            })
            .highlight_symbol("▸ ");
        f.render_stateful_widget(list, tracks_area, &mut st);
    }

    // ---- options
    let focused_opts = app.focus == Focus::Options;
    let oborder = if focused_opts { ACCENT } else { DIM };
    let mut lines: Vec<Line> = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        let selected = focused_opts && i == app.opt_cursor;
        let marker = if selected { "▸ " } else { "  " };
        let key = match row {
            crate::app::Row::Format => "Format",
            crate::app::Row::AlacMax => "Qualite ALAC",
            crate::app::Row::EmbedLrc => "Paroles integrees",
            crate::app::Row::SaveLrc => "Fichier .lrc",
            crate::app::Row::LrcType => "Type de paroles",
            crate::app::Row::LrcExtra => "Traduction",
            crate::app::Row::KeepOriginal => "Garder l'original",
        };
        let style = if selected {
            Style::new().fg(Color::White).add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(Color::Rgb(200, 200, 210))
        };
        lines.push(Line::from(vec![
            Span::styled(
                format!(" {marker}"),
                Style::new().fg(if selected { ACCENT } else { DIM }),
            ),
            Span::styled(format!("{:<20}", key), Style::new().fg(if selected { ACCENT } else { DIM })),
            Span::styled(" ‹ ", Style::new().fg(if selected { ACCENT } else { DIM })),
            Span::styled(app.row_value(*row), style),
            Span::styled(" ›", Style::new().fg(if selected { ACCENT } else { DIM })),
        ]));
    }
    lines.push(Line::from(Span::styled(
        format!("    Max detecte : {}", item.max_quality_label()),
        Style::new().fg(DIM),
    )));
    f.render_widget(
        Paragraph::new(lines)
            .block(Block::bordered().border_style(Style::new().fg(oborder)).title(" Options ")),
        opts_area,
    );
}

fn running(f: &mut Frame, app: &App, area: Rect) {
    let [gauge_area, log_area] =
        Layout::vertical([Constraint::Length(3), Constraint::Min(3)]).areas(area);

    let pct = app.progress.unwrap_or(0) as f64;
    let g = Gauge::default()
        .block(
            Block::bordered()
                .border_style(Style::new().fg(ACCENT))
                .title(" Telechargement "),
        )
        .gauge_style(Style::new().fg(ACCENT))
        .ratio((pct / 100.0).clamp(0.0, 1.0))
        .label(format!("{:.0}%", pct));
    f.render_widget(g, gauge_area);

    let visible = log_area.height.saturating_sub(2) as usize;
    let start = app.log.len().saturating_sub(visible);
    let lines: Vec<Line> = app
        .log
        .iter()
        .skip(start)
        .map(|l| {
            let col = if l.contains("ERREUR") || l.starts_with("ERREUR") {
                ACCENT
            } else if l.contains("Decrypted") || l.contains("Completed") {
                OK
            } else {
                Color::Rgb(200, 200, 210)
            };
            Line::from(Span::styled(format!("  {l}"), Style::new().fg(col)))
        })
        .collect();
    f.render_widget(
        Paragraph::new(lines)
            .block(Block::bordered().border_style(Style::new().fg(DIM)).title(" Journal ")),
        log_area,
    );
}

fn done(f: &mut Frame, app: &App, area: Rect) {
    let code = app.done_code.unwrap_or(-1);
    let good = code == 0;
    let [msg_area, log_area] =
        Layout::vertical([Constraint::Length(IP_HEIGHT), Constraint::Min(3)]).areas(area);

    let mut lines = vec![Line::from("")];
    if good {
        lines.push(Line::from(Span::styled(
            "  ✔  Termine",
            Style::new().fg(OK).add_modifier(Modifier::BOLD),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            format!("  ✖  Termine avec des erreurs (code {code})"),
            Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
        )));
    }
    lines.push(Line::from(Span::styled(
        format!("     Dossier : {}", app.save_dir.display()),
        Style::new().fg(DIM),
    )));
    f.render_widget(
        Paragraph::new(lines).block(Block::bordered().border_style(Style::new().fg(
            if good { OK } else { ACCENT },
        ))),
        msg_area,
    );

    let visible = log_area.height.saturating_sub(2) as usize;
    let start = app.log.len().saturating_sub(visible);
    let tail: Vec<Line> = app
        .log
        .iter()
        .skip(start)
        .map(|l| Line::from(Span::styled(format!("  {l}"), Style::new().fg(DIM))))
        .collect();
    f.render_widget(
        Paragraph::new(tail)
            .block(Block::bordered().border_style(Style::new().fg(DIM)).title(" Journal ")),
        log_area,
    );
}

const IP_HEIGHT: u16 = 5;

fn footer(f: &mut Frame, app: &App, area: Rect) {
    let help = match app.screen {
        Screen::Input => {
            "Entree = analyser    Esc = effacer/quitter    Ctrl+S = demarrer/arreter wrapper-lite"
        }
        Screen::Loading => "Esc = annuler    Ctrl+S = wrapper-lite",
        Screen::Ready => {
            if app.focus == Focus::Tracks {
                "↑↓ = naviguer    Espace = cocher    a = tout    n = rien    i = inverser    Tab = options    Entree = lancer    Esc = retour    q = quitter"
            } else {
                "↑↓ = option    ←→/Espace = changer    Tab = pistes    Entree = lancer    Esc = retour    q = quitter"
            }
        }
        Screen::Running => "patientez…    q = quitter",
        Screen::Done => "Entree = nouvel URL    q = quitter",
    };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!("  {help}"),
            Style::new().fg(DIM),
        ))),
        area,
    );
}
