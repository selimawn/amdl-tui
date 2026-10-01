// Rendu de l'interface : arbre d'albums repliables + panneau d'options.
//
// L'arbre est rendu ligne par ligne plutot qu'avec le widget List : le
// defilement est ainsi maitrise et le style de chaque ligne (case a cocher,
// fleche de pliage, qualite) reste entierement controle.

use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::app::{App, Focus, RowRef, Row, Screen};

const ACCENT: Color = Color::Rgb(250, 45, 72);
const DIM: Color = Color::Rgb(135, 135, 145);
const OK: Color = Color::Rgb(85, 200, 125);
const WARN: Color = Color::Rgb(240, 180, 70);
const HIRES: Color = Color::Rgb(120, 200, 255);

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

fn quality_color(q: &str) -> Color {
    match q {
        "Hi-Res" => HIRES,
        "Lossless" => OK,
        "Atmos" => Color::Rgb(200, 140, 255),
        _ => DIM,
    }
}

pub fn draw(f: &mut Frame, app: &mut App) {
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

fn header(f: &mut Frame, app: &App, area: Rect) {
    let mut lines: Vec<Line> = Vec::new();
    match &app.item {
        Some(it) if app.screen != Screen::Input => {
            lines.push(Line::from(vec![
                Span::styled("  ♪  ", Style::new().fg(ACCENT)),
                Span::styled(
                    it.title.clone(),
                    Style::new().fg(Color::White).add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!("  —  {}", it.artist), Style::new().fg(DIM)),
            ]));
            let detail = if it.is_multi() {
                format!(
                    "{} · {} albums · {} pistes · {} sur {} selectionnees",
                    it.kind.label(),
                    it.albums.len(),
                    it.total_tracks(),
                    it.total_checked(),
                    it.total_tracks()
                )
            } else {
                format!(
                    "{} · {} pistes · {} selectionnee(s)",
                    it.kind.label(),
                    it.albums[0].tracks.len(),
                    it.albums[0].checked_count()
                )
            };
            lines.push(Line::from(vec![
                Span::styled(format!("     {detail}"), Style::new().fg(DIM)),
                Span::styled(
                    format!("   ·   {}", it.storefront.to_uppercase()),
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

fn input(f: &mut Frame, app: &mut App, area: Rect) {
    let [top, mid] = Layout::vertical([Constraint::Length(5), Constraint::Min(4)]).areas(area);

    let txt = if app.input.is_empty() {
        Line::from(Span::styled(
            "Colle ici une URL music.apple.com puis Entree…",
            Style::new().fg(DIM),
        ))
    } else {
        Line::from(Span::styled(app.input.clone(), Style::new().fg(Color::White)))
    };
    let url_blk = Block::bordered()
        .border_style(Style::new().fg(if app.input.is_empty() { DIM } else { ACCENT }))
        .title(" URL (titre, album, playlist ou artiste) ");
    f.render_widget(
        Paragraph::new(Text::from(vec![
            txt,
            Line::from(Span::styled("▌", Style::new().fg(ACCENT))),
        ]))
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
        "…/tr/artist/taylor-swift/159260351           tous les albums (repliables)",
        "…/tr/album/at-the-bbc/1555697650             album complet",
        "…/tr/album/…/1555697650?i=1555697872         une piste precise",
        "…/tr/song/rehab/1555697872                   un titre",
        "…/tr/playlist/…/pl.xxxxxxxxxxxx              une playlist",
    ] {
        lines.push(Line::from(Span::styled(
            format!("    {ex}"),
            Style::new().fg(DIM),
        )));
    }
    if !app.status.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("  {}", app.status),
            Style::new().fg(OK),
        )));
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
        Paragraph::new(lines).block(
            Block::bordered()
                .border_style(Style::new().fg(DIM))
                .title(" Chargement "),
        ),
        area,
    );
}

/// Ecran principal : arbre d'albums + options.
fn ready(f: &mut Frame, app: &mut App, area: Rect) {
    let n_opts = app.rows().len();
    let opts_h = (n_opts as u16 + 3).min(area.height.saturating_sub(3));
    let [tree_area, opts_area] =
        Layout::vertical([Constraint::Min(3), Constraint::Length(opts_h)]).areas(area);

    draw_tree(f, app, tree_area);
    draw_options(f, app, opts_area);
}

fn draw_tree(f: &mut Frame, app: &mut App, area: Rect) {
    let rows = app.visible_rows();
    let inner_h = area.height.saturating_sub(2) as usize;
    let inner_w = area.width.saturating_sub(2) as usize;

    // le curseur doit rester visible : on ajuste le defilement au rendu
    if inner_h > 0 {
        if app.cursor < app.tree_scroll {
            app.tree_scroll = app.cursor;
        }
        if app.cursor >= app.tree_scroll + inner_h {
            app.tree_scroll = app.cursor + 1 - inner_h;
        }
        let max_scroll = rows.len().saturating_sub(inner_h);
        if app.tree_scroll > max_scroll {
            app.tree_scroll = max_scroll;
        }
    }

    // largeur du nom de piste : le plus long, borne
    let name_w = app
        .item
        .as_ref()
        .map(|it| {
            it.albums
                .iter()
                .flat_map(|a| a.tracks.iter())
                .map(|t| t.name.chars().count())
                .max()
                .unwrap_or(18)
        })
        .unwrap_or(18)
        .clamp(16, (inner_w.saturating_sub(30)).max(16));

    let focused = app.focus == Focus::Tracks;
    let border = if focused { ACCENT } else { DIM };
    let (count_sel, count_all) = match &app.item {
        Some(it) => (it.total_checked(), it.total_tracks()),
        None => (0, 0),
    };
    let title = format!(" Albums — {count_sel}/{count_all} pistes cochees ");

    let mut lines: Vec<Line> = Vec::new();

    let start = app.tree_scroll;
    let end = (start + inner_h).min(rows.len());
    for (i, rr) in rows[start..end].iter().enumerate() {
        let idx = start + i;
        let is_cursor = focused && idx == app.cursor;
        let is_cursor_any = idx == app.cursor;
        let base_bg = if is_cursor {
            Style::new().bg(Color::Rgb(45, 45, 55))
        } else if is_cursor_any {
            Style::new().bg(Color::Rgb(32, 32, 38))
        } else {
            Style::new()
        };

        let line = match rr {
            RowRef::Album(ai) => {
                let a = match app.item.as_ref().and_then(|it| it.albums.get(*ai)) {
                    Some(a) => a,
                    None => continue,
                };
                let tri = if a.loading {
                    Span::styled("⋯ ", Style::new().fg(WARN))
                } else if a.expanded {
                    Span::styled("▾ ", Style::new().fg(ACCENT))
                } else {
                    Span::styled("▸ ", Style::new().fg(DIM))
                };
                let cb = Span::styled(
                    format!("{} ", a.check_box()),
                    Style::new().fg(if a.none_checked() {
                        DIM
                    } else if a.all_checked() {
                        OK
                    } else {
                        WARN
                    }),
                );
                let year = if a.year.is_empty() {
                    String::new()
                } else {
                    format!("{} · ", a.year)
                };
                let w = inner_w.saturating_sub(12);
                let q = a.quality_tag();
                Line::from(vec![
                    Span::styled(" ", base_bg),
                    cb,
                    tri,
                    Span::styled(
                        pad_trunc(&format!("{year}{}", a.title), w),
                        Style::new()
                            .fg(if a.none_checked() { DIM } else { Color::White })
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!("{:>3} pistes", a.track_count),
                        Style::new().fg(DIM),
                    ),
                    Span::styled(format!("  {q}"), Style::new().fg(quality_color(&q))),
                ])
                .style(base_bg)
            }
            RowRef::Track(ai, ti) => {
                let t = match app
                    .item
                    .as_ref()
                    .and_then(|it| it.albums.get(*ai))
                    .and_then(|a| a.tracks.get(*ti))
                {
                    Some(t) => t,
                    None => continue,
                };
                let checked = app
                    .item
                    .as_ref()
                    .and_then(|it| it.albums.get(*ai))
                    .and_then(|a| a.checked.get(*ti))
                    .copied()
                    .unwrap_or(false);
                let disc = if t.disc_number > 1 {
                    format!("D{}·", t.disc_number)
                } else {
                    String::new()
                };
                let q = t.quality_tag();
                Line::from(vec![
                    Span::styled(" ", base_bg),
                    Span::styled("    ", Style::new()),
                    Span::styled(
                        format!("{} ", if checked { "[x]" } else { "[ ]" }),
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
                    Span::styled(format!("  {q}"), Style::new().fg(quality_color(&q))),
                ])
                .style(base_bg)
            }
        };
        lines.push(line);
    }

    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "  (aucune piste)",
            Style::new().fg(DIM),
        )));
    }

    let hint = if app
        .item
        .as_ref()
        .map(|it| it.is_multi())
        .unwrap_or(false)
    {
        " → ou Entree : deplier l'album "
    } else {
        " "
    };
    f.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .border_style(Style::new().fg(border))
                .title(title)
                .title_bottom(Line::from(Span::styled(hint, Style::new().fg(DIM)))),
        ),
        area,
    );
}

fn draw_options(f: &mut Frame, app: &mut App, area: Rect) {
    let rows = app.rows();
    let focused = app.focus == Focus::Options;
    let border = if focused { ACCENT } else { DIM };

    let mut lines: Vec<Line> = Vec::new();

    for (i, row) in rows.iter().enumerate() {
        let sel = focused && i == app.opt_cursor;
        let sel_any = i == app.opt_cursor;
        let bg = if sel {
            Style::new().bg(Color::Rgb(45, 45, 55))
        } else if sel_any {
            Style::new().bg(Color::Rgb(32, 32, 38))
        } else {
            Style::new()
        };
        lines.push(
            Line::from(vec![
                Span::styled(
                    if sel { " ▸ " } else { "   " },
                    Style::new().fg(if sel { ACCENT } else { DIM }),
                ),
                Span::styled(
                    format!("{:<20}", App::row_label(*row)),
                    Style::new().fg(if sel { ACCENT } else { DIM }),
                ),
                Span::styled(" ‹ ", Style::new().fg(if sel { ACCENT } else { DIM })),
                Span::styled(
                    app.row_value(*row),
                    Style::new()
                        .fg(if sel { Color::White } else { Color::Rgb(205, 205, 215) })
                        .add_modifier(if sel { Modifier::BOLD } else { Modifier::empty() }),
                ),
                Span::styled(" ›", Style::new().fg(if sel { ACCENT } else { DIM })),
            ])
            .style(bg),
        );
    }

    let maxq = match &app.item {
        Some(it) => it.max_quality_label(),
        None => String::new(),
    };
    lines.push(Line::from(Span::styled(
        format!("    Max detecte : {maxq}"),
        Style::new().fg(DIM),
    )));

    f.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .border_style(Style::new().fg(border))
                .title(" Options "),
        ),
        area,
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
                .title(format!(" {} ", app.job_info)),
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
            let col = if l.starts_with("ERREUR") {
                ACCENT
            } else if l.starts_with("─────") {
                WARN
            } else if l.contains("Decrypted") || l.contains("Conversion") {
                OK
            } else {
                Color::Rgb(200, 200, 210)
            };
            Line::from(Span::styled(format!("  {l}"), Style::new().fg(col)))
        })
        .collect();
    f.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .border_style(Style::new().fg(DIM))
                .title(" Journal "),
        ),
        log_area,
    );
}

fn done(f: &mut Frame, app: &App, area: Rect) {
    let code = app.done_code.unwrap_or(-1);
    let good = code == 0;
    let [msg_area, log_area] =
        Layout::vertical([Constraint::Length(6), Constraint::Min(3)]).areas(area);

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
    lines.push(Line::from(Span::styled(
        format!("     {}", app.job_info),
        Style::new().fg(DIM),
    )));
    f.render_widget(
        Paragraph::new(lines).block(
            Block::bordered().border_style(Style::new().fg(if good { OK } else { ACCENT })),
        ),
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
        Paragraph::new(tail).block(
            Block::bordered()
                .border_style(Style::new().fg(DIM))
                .title(" Journal "),
        ),
        log_area,
    );
}

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

fn footer(f: &mut Frame, app: &App, area: Rect) {
    let help: &str = match app.screen {
        Screen::Input => "Entree = analyser    Esc = effacer/quitter    Ctrl+S = wrapper-lite",
        Screen::Loading => "Esc = annuler    Ctrl+S = wrapper-lite",
        Screen::Ready => {
            if app.focus == Focus::Tracks {
                "↑↓ naviguer    →/Entree deplier    ← replier    Espace cocher    a tout  n rien  i inverser    Tab options    Entree (sur piste) lance    Esc retour    q quitter"
            } else {
                "↑↓ option    ←→/Espace changer    Tab pistes    Entree lancer    Esc retour    q quitter"
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

#[allow(dead_code)]
fn unused(_: Row) {}
