//! Ratatui main loop: poll sensors, update history, draw btop-style braille temperature charts.

mod chart;

use crate::cpu_info::read_cpu_model_from_proc;
use crate::group::{
    composite_storage_row_title, group_readings, CpuRole, PanelKind, PanelSpec, SeriesSpec,
    StorageDriveSpec,
};
use crate::history::History;
use crate::sensors::{fetch_readings, stable_series_id};

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Style, Stylize};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::{backend::CrosstermBackend, Frame, Terminal};
use std::io::stdout;
use std::time::{Duration, Instant};

const DEFAULT_POLL_MS: u64 = 1000;
const HISTORY_CAP: usize = 160;

#[derive(Clone, Copy)]
enum TempRowLabel {
    /// CPU, motherboard, flat storage list, etc.
    Normal,
    /// Drive title is above; left column composite.
    StorageComposite,
    /// Drive title is above; sensor name only + temp.
    StorageAux,
}

struct App {
    history: History,
    panels: Vec<PanelSpec>,
    last_error: Option<String>,
    poll: Duration,
    /// From `/proc/cpuinfo` for the CPU panel title.
    cpu_model: Option<String>,
}

impl App {
    fn new() -> Self {
        Self {
            history: History::new(HISTORY_CAP),
            panels: Vec::new(),
            last_error: None,
            poll: Duration::from_millis(DEFAULT_POLL_MS),
            cpu_model: None,
        }
    }

    fn poll_sensors(&mut self) {
        match fetch_readings() {
            Ok(readings) => {
                self.last_error = None;
                self.apply_readings(&readings);
            }
            Err(e) => {
                self.last_error = Some(format!("{e:#}"));
            }
        }
    }

    /// Record samples first, then rebuild panels, then ensure every series id has a buffer so the next frame never reads a missing history key.
    fn apply_readings(&mut self, readings: &[crate::sensors::SensorReading]) {
        for r in readings {
            self.history.record(&stable_series_id(r), r.value_c);
        }
        if let Some(m) = read_cpu_model_from_proc() {
            self.cpu_model = Some(m);
        }
        self.panels = group_readings(readings);
        for p in &self.panels {
            for s in &p.series {
                self.history.ensure_series(&s.id);
            }
        }
    }
}

/// Run the TUI until the user presses `q`.
pub fn run() -> Result<()> {
    enable_raw_mode()?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(out);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new();
    app.poll_sensors();
    let mut last_poll = Instant::now();
    let mut quit = false;

    while !quit {
        if last_poll.elapsed() >= app.poll {
            last_poll = Instant::now();
            app.poll_sensors();
        }

        terminal.draw(|f| draw(f, &app))?;

        let wait = Duration::from_millis(50).min(app.poll.saturating_sub(last_poll.elapsed()));
        if event::poll(wait)? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                match key.code {
                    KeyCode::Char('q') | KeyCode::Char('Q') => quit = true,
                    KeyCode::Char('r') | KeyCode::Char('R') => {
                        last_poll = Instant::now();
                        app.poll_sensors();
                    }
                    KeyCode::Char('+') | KeyCode::Char('=') => {
                        let ms = app.poll.as_millis().saturating_add(250).min(10_000) as u64;
                        app.poll = Duration::from_millis(ms.max(250));
                    }
                    KeyCode::Char('-') | KeyCode::Char('_') => {
                        let ms = app.poll.as_millis().saturating_sub(250).max(250) as u64;
                        app.poll = Duration::from_millis(ms);
                    }
                    _ => {}
                }
            }
        }
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    Ok(())
}

fn draw(frame: &mut Frame<'_>, app: &App) {
    let full = frame.area();
    let footer_lines: u16 = if app.last_error.is_some() { 2 } else { 1 };
    let footer_lines = footer_lines.min(full.height);
    let main_h = full.height.saturating_sub(footer_lines);
    let main_area = Rect {
        x: full.x,
        y: full.y,
        width: full.width,
        height: main_h,
    };
    let footer = Rect {
        x: full.x,
        y: full.y + main_h,
        width: full.width,
        height: footer_lines,
    };

    if app.panels.is_empty() {
        let msg = if let Some(ref e) = app.last_error {
            format!("btemp — no sensor data\n\n{e}\n\nInstall lm-sensors, run sensors-detect, and ensure `sensors` is on PATH.")
        } else {
            "btemp — no temperature sensors found.".to_string()
        };
        frame.render_widget(
            Paragraph::new(msg).block(Block::default().borders(Borders::ALL).title(" btemp ")),
            main_area,
        );
    } else {
        let n = app.panels.len();
        let constraints: Vec<Constraint> = (0..n).map(|_| Constraint::Fill(1)).collect();
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(main_area);
        for (i, panel) in app.panels.iter().enumerate() {
            if i < chunks.len() {
                draw_panel(
                    frame,
                    chunks[i],
                    panel,
                    &app.history,
                    app.cpu_model.as_deref(),
                );
            }
        }
    }

    let mut footer_text = format!(
        " q quit | r refresh | +/- interval ({}ms) ",
        app.poll.as_millis()
    );
    if let Some(ref e) = app.last_error {
        footer_text = format!("{e}\n{footer_text}");
    }
    frame.render_widget(
        Paragraph::new(footer_text).style(Style::default().dim()),
        footer,
    );
}

fn cpu_composite_and_cores(series: &[SeriesSpec]) -> (Vec<&SeriesSpec>, Vec<&SeriesSpec>) {
    let mut composites = Vec::new();
    let mut cores = Vec::new();
    for s in series {
        if s.cpu_role == Some(CpuRole::Composite) {
            composites.push(s);
        } else {
            cores.push(s);
        }
    }
    (composites, cores)
}

fn draw_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    panel: &PanelSpec,
    history: &History,
    cpu_model: Option<&str>,
) {
    let title = if panel.kind == PanelKind::Cpu {
        if let Some(m) = cpu_model {
            let short = truncate(m, 56);
            format!(" {} — {} ", panel.title, short)
        } else {
            format!(" {} (CPU) ", panel.title)
        }
    } else {
        format!(" {} ", panel.title)
    };
    let block = Block::default().borders(Borders::ALL).title(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    if panel.kind == PanelKind::Storage
        && !panel.storage_drives.is_empty()
        && inner.width >= 44
    {
        draw_storage_by_drive(frame, inner, &panel.storage_drives, history);
        return;
    }

    // Use Fill so each graph row shares the panel's vertical space (fixed Length(8) left a large empty band).
    let cpu_parts = (panel.kind == PanelKind::Cpu).then(|| cpu_composite_and_cores(&panel.series));

    let mut rows: Vec<Constraint> = Vec::new();
    match &cpu_parts {
        Some((composites, cores)) => {
            if !composites.is_empty() {
                rows.push(Constraint::Fill(1));
            }
            for _ in cores {
                rows.push(Constraint::Fill(1));
            }
        }
        None => {
            for _ in &panel.series {
                rows.push(Constraint::Fill(1));
            }
        }
    }

    if rows.is_empty() {
        rows.push(Constraint::Fill(1));
    }

    let row_areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints(rows)
        .split(inner);

    let mut row_idx = 0usize;

    if let Some((composites, cores)) = cpu_parts {
        if !composites.is_empty() && row_idx < row_areas.len() {
            let area = row_areas[row_idx];
            row_idx += 1;
            draw_composite_row(frame, area, composites.as_slice(), history);
        }
        for s in cores {
            if row_idx < row_areas.len() {
                draw_series_row(frame, row_areas[row_idx], s, history);
                row_idx += 1;
            }
        }
    } else {
        for s in &panel.series {
            if row_idx < row_areas.len() {
                draw_series_row(frame, row_areas[row_idx], s, history);
                row_idx += 1;
            }
        }
    }
}

fn draw_storage_by_drive(
    frame: &mut Frame<'_>,
    inner: Rect,
    drives: &[StorageDriveSpec],
    history: &History,
) {
    let constraints: Vec<Constraint> = (0..drives.len())
        .map(|_| Constraint::Fill(1))
        .collect();
    let drive_areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(inner);

    for (area, drive) in drive_areas.iter().zip(drives) {
        draw_one_storage_drive(frame, *area, drive, history);
    }
}

fn draw_one_storage_drive(
    frame: &mut Frame<'_>,
    area: Rect,
    drive: &StorageDriveSpec,
    history: &History,
) {
    if area.height < 2 {
        return;
    }
    let header_h = 1u16.min(area.height);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(header_h), Constraint::Fill(1)])
        .split(area);

    frame.render_widget(
        Paragraph::new(truncate(&drive.display_name, 72)).style(Style::default().bold()),
        chunks[0],
    );

    let body = chunks[1];
    if body.height == 0 || body.width == 0 {
        return;
    }

    let has_right = !drive.sensors.is_empty();
    let has_left = drive.composite.is_some();

    if !has_left && !has_right {
        return;
    }

    if !has_right {
        if let Some(ref c) = drive.composite {
            // Not the 67/33 drive layout: use the same label rules as CPU/other single-column rows.
            draw_labeled_braille_row(frame, body, c, history, TempRowLabel::Normal);
        }
        return;
    }

    if !has_left {
        let rows: Vec<Constraint> = (0..drive.sensors.len())
            .map(|_| Constraint::Fill(1))
            .collect();
        let areas = Layout::default()
            .direction(Direction::Vertical)
            .constraints(rows)
            .split(body);
        for (r, s) in areas.iter().zip(&drive.sensors) {
            draw_labeled_braille_row(frame, *r, s, history, TempRowLabel::StorageAux);
        }
        return;
    }

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(67), Constraint::Percentage(33)])
        .split(body);

    let composite = drive.composite.as_ref().unwrap();
    draw_labeled_braille_row(
        frame,
        cols[0],
        composite,
        history,
        TempRowLabel::StorageComposite,
    );

    let rows: Vec<Constraint> = (0..drive.sensors.len())
        .map(|_| Constraint::Fill(1))
        .collect();
    let right_areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints(rows)
        .split(cols[1]);
    for (r, s) in right_areas.iter().zip(&drive.sensors) {
        draw_labeled_braille_row(frame, *r, s, history, TempRowLabel::StorageAux);
    }
}

fn draw_labeled_braille_row(
    frame: &mut Frame<'_>,
    area: Rect,
    spec: &SeriesSpec,
    history: &History,
    label_kind: TempRowLabel,
) {
    if area.width < 6 || area.height == 0 {
        return;
    }
    let label_w = label_column_width(area.width, label_kind);
    let label_area = Rect {
        x: area.x,
        y: area.y,
        width: label_w,
        height: area.height,
    };
    let chart_area = Rect {
        x: area.x + label_w,
        y: area.y,
        width: area.width.saturating_sub(label_w).max(1),
        height: area.height,
    };

    let last_v = history
        .buffer(&spec.id)
        .and_then(|b| b.last())
        .unwrap_or(f64::NAN);

    let temp_s = format_temp_c(last_v);

    let label_text = match label_kind {
        TempRowLabel::StorageComposite => {
            let head = composite_storage_row_title(&spec.label);
            format!("{head} {temp_s}")
        }
        TempRowLabel::StorageAux => format!("{} {}", truncate(&spec.label, 18), temp_s),
        TempRowLabel::Normal => {
            if spec.display_name != spec.label
                && !spec.label.eq_ignore_ascii_case("composite")
            {
                format!(
                    "{} · {} {}",
                    truncate(&spec.display_name, 22),
                    truncate(&spec.label, 14),
                    temp_s
                )
            } else {
                format!("{} {}", truncate(&spec.display_name, 26), temp_s)
            }
        }
    };

    frame.render_widget(
        Paragraph::new(label_text).style(Style::default().bold()),
        label_area,
    );

    let vals: Vec<f64> = history
        .buffer(&spec.id)
        .map(|b| b.as_slice())
        .unwrap_or_default();
    chart::render_braille_temp_canvas(frame, chart_area, &vals, HISTORY_CAP);
}

fn draw_composite_row(
    frame: &mut Frame<'_>,
    area: Rect,
    composites: &[&SeriesSpec],
    history: &History,
) {
    if area.width < 4 {
        return;
    }
    let n = composites.len().max(1) as u16;
    let w = (area.width / n).max(1);
    for (i, spec) in composites.iter().enumerate() {
        let x = area.x + (i as u16) * w;
        // One column less than an even split leaves a narrow gutter so adjacent braille columns do not blend.
        let sub = Rect {
            x,
            y: area.y,
            width: w.saturating_sub(1),
            height: area.height,
        };
        draw_series_row(frame, sub, spec, history);
    }
}

fn draw_series_row(frame: &mut Frame<'_>, area: Rect, spec: &SeriesSpec, history: &History) {
    draw_labeled_braille_row(frame, area, spec, history, TempRowLabel::Normal);
}

/// Wider label column so text + `°C` is not clipped; chart gets the remainder.
fn label_column_width(area_w: u16, kind: TempRowLabel) -> u16 {
    let pct = match kind {
        TempRowLabel::StorageComposite => 30u16,
        TempRowLabel::StorageAux => 28,
        TempRowLabel::Normal => 32,
    };
    let w = (area_w.saturating_mul(pct) / 100).max(11);
    w.min(area_w.saturating_sub(4).max(11))
        .min(26)
        .max(11)
}

fn format_temp_c(value: f64) -> String {
    if value.is_finite() {
        format!("{value:.1}\u{00B0}C")
    } else {
        "--".to_string()
    }
}

fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    s.chars().take(max_chars.saturating_sub(1)).collect::<String>() + "…"
}
