//! Screen-by-screen rendering for the TUI tabs and 4 Porter-Duff RGBA translucent modals.
use crate::tui::{
    compositor::{apply_rgba_backdrop, sanitize_tui_str, Rgba, Theme},
    AppModal, AppState, HeatmapMetric, InsightDepth, SeatGroupMode, SeatViewMode, TabId,
};
use dendro_core::insight_v1::{InsightDomain, InsightResult, InsightStatus, MeasureValue};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, Tabs, Wrap},
    Frame,
};
use serde_json::Value;
use std::collections::BTreeMap;

const BRAILLE_SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Insight domains rendered in the cockpit chrome. OpenRig is the scope lens, not a domain.
const DOMAIN_LEGEND: &str = "Domains: Model · Harness · Coordination · Environment · Security";
const ADAPTER_LENS: &str =
    "[r] Rig is the OpenRig adapter scope (<role>@<rig>), not an insight domain";

fn rows(v: &Value) -> impl Iterator<Item = &Value> {
    v.as_array().into_iter().flatten()
}

/// Renders a cost object already stored on the report. Null usd stays unmarked.
fn fmt_report_cost(label: &str, value: &Value) -> String {
    let status = match value["status"].as_str() {
        Some("complete") => "complete",
        Some("partial") => "partial",
        Some("unknown") => "unknown",
        Some(other) => return format!("{label}: {}", sanitize_tui_str(other)),
        None => "unknown",
    };
    if status == "complete" {
        match value["usd"].as_f64().filter(|amount| amount.is_finite()) {
            Some(amount) => format!("{label}: complete {amount} usd"),
            None => format!("{label}: complete unavailable"),
        }
    } else {
        match value["unknown_rows"].as_u64() {
            Some(rows) => format!("{label}: {status} ({rows} unknown rows)"),
            None => format!("{label}: {status}"),
        }
    }
}

fn fmt_num(v: &Value) -> String {
    if let Some(n) = v.as_u64() {
        return n.to_string();
    }
    if let Some(n) = v.as_i64() {
        return n.to_string();
    }
    match v.as_f64().filter(|n| n.is_finite()) {
        Some(n) if n.fract() == 0.0 => format!("{n:.0}"),
        Some(n) => format!("{n:.2}"),
        None => "Unavailable".into(),
    }
}

fn fmt_pct(v: &Value) -> String {
    v.as_f64()
        .filter(|n| n.is_finite())
        .map(|n| format!("{:.1}%", n * 100.0))
        .unwrap_or_else(|| "Unavailable".into())
}

pub(super) fn fmt_compact(v: &Value) -> String {
    match v.as_f64() {
        Some(n) if n.is_finite() => fmt_compact_f64(n),
        _ => "Unavailable".into(),
    }
}

pub(super) fn fmt_compact_f64(n: f64) -> String {
    if !n.is_finite() {
        return "Unavailable".into();
    }
    let sign = if n < 0.0 { "-" } else { "" };
    let n = n.abs();
    if n >= 1_000_000_000.0 {
        format!("{sign}{:.2}B", n / 1_000_000_000.0)
    } else if n >= 1_000_000.0 {
        format!("{sign}{:.1}M", n / 1_000_000.0)
    } else if n >= 1_000.0 {
        format!("{sign}{:.1}k", n / 1_000.0)
    } else if n.fract() == 0.0 {
        format!("{sign}{n:.0}")
    } else {
        format!("{sign}{n:.1}")
    }
}

pub(super) fn fmt_share_pct(part: &Value, total: f64) -> String {
    let Some(part) = part.as_f64().filter(|n| n.is_finite()) else {
        return "(Unavailable)".into();
    };
    if !total.is_finite() || total <= 0.0 {
        return "(Unavailable)".into();
    }
    format!("({:.1}%)", part / total * 100.0)
}

fn fmt_delta_pct(next: f64, base: f64) -> String {
    if !next.is_finite() || !base.is_finite() || base == 0.0 {
        return "(Unavailable)".into();
    }
    format!("({:+.1}%)", (next - base) / base * 100.0)
}

fn fmt_duration(v: &Value) -> String {
    let Some(secs) = v.as_f64().filter(|n| n.is_finite() && *n >= 0.0) else {
        return "Unavailable".into();
    };
    let total = secs.round() as u64;
    let hours = total / 3600;
    let minutes = (total % 3600) / 60;
    let seconds = total % 60;
    if hours > 0 && minutes > 0 {
        format!("{hours}h {minutes}m")
    } else if hours > 0 {
        format!("{hours}h")
    } else if minutes > 0 {
        format!("{minutes}m {seconds}s")
    } else {
        format!("{seconds}s")
    }
}

fn fmt_duration_share(v: &Value, total: f64) -> String {
    format!("{} {}", fmt_duration(v), fmt_share_pct(v, total))
}

fn value_f64(v: &Value) -> f64 {
    v.as_f64()
        .filter(|n| n.is_finite() && *n > 0.0)
        .unwrap_or(0.0)
}

fn allocate_cells(weights: &[f64], width: usize, protect: impl Fn(usize) -> bool) -> Vec<usize> {
    let len = weights.len();
    let mut cells = vec![0usize; len];
    if width == 0 || len == 0 {
        return cells;
    }
    let total: f64 = weights.iter().copied().filter(|weight| *weight > 0.0).sum();
    if total <= 0.0 {
        return cells;
    }
    let raw: Vec<f64> = weights
        .iter()
        .map(|weight| {
            if *weight > 0.0 {
                weight / total * width as f64
            } else {
                0.0
            }
        })
        .collect();
    for (index, weight) in weights.iter().enumerate() {
        if *weight > 0.0 {
            let floor = raw[index].floor() as usize;
            let minimum = usize::from(protect(index));
            cells[index] = floor.max(minimum);
        }
    }
    let mut sum: usize = cells.iter().sum();
    while sum > width {
        let candidate = (0..len).rev().find(|&index| {
            let minimum = usize::from(weights[index] > 0.0 && protect(index));
            cells[index] > minimum
        });
        let Some(index) = candidate else {
            break;
        };
        cells[index] -= 1;
        sum -= 1;
    }
    while sum < width {
        let mut best: Option<(usize, f64)> = None;
        for index in 0..len {
            if weights[index] <= 0.0 {
                continue;
            }
            let remainder = raw[index] - cells[index] as f64;
            match best {
                Some((_, best_remainder)) if remainder <= best_remainder => {}
                _ => best = Some((index, remainder)),
            }
        }
        let Some((index, _)) = best else {
            break;
        };
        cells[index] += 1;
        sum += 1;
    }
    cells
}

fn share_bar(parts: &[(f64, char, Style)], width: usize) -> Line<'static> {
    let weights: Vec<f64> = parts.iter().map(|part| part.0).collect();
    if weights.iter().all(|weight| *weight <= 0.0) {
        return Line::from(Span::raw("·".repeat(width)));
    }
    let cells = allocate_cells(&weights, width, |_| true);
    let mut spans = Vec::new();
    let mut used = 0usize;
    for (index, (_, ch, style)) in parts.iter().enumerate() {
        let n = cells
            .get(index)
            .copied()
            .unwrap_or(0)
            .min(width.saturating_sub(used));
        if n > 0 {
            used += n;
            spans.push(Span::styled(ch.to_string().repeat(n), *style));
        }
    }
    if used < width {
        spans.push(Span::raw("·".repeat(width - used)));
    }
    if spans.is_empty() {
        spans.push(Span::raw("·".repeat(width)));
    }
    Line::from(spans)
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

pub fn render_app(frame: &mut Frame, state: &mut AppState, theme: &Theme) {
    let area = frame.area();
    frame.render_widget(Block::default().style(theme.bg_base()), area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(2),
        ])
        .split(area);

    render_header(frame, chunks[0], state, theme);
    render_tabs(frame, chunks[1], state, theme);

    let insights_ready = state.active_tab == TabId::Insights
        && (state.insights.is_some() || state.insights_error.is_some());
    if state.report.is_none() && !insights_ready {
        render_skeleton_loading(frame, chunks[2], state, theme);
    } else {
        match state.active_tab {
            TabId::Pulse => render_pulse_tab(frame, chunks[2], state, theme),
            TabId::Seats => render_seats_tab(frame, chunks[2], state, theme),
            TabId::Handoffs => render_handoffs_tab(frame, chunks[2], state, theme),
            TabId::Tokens => render_tokens_tab(frame, chunks[2], state, theme),
            TabId::Simulate => render_simulate_tab(frame, chunks[2], state, theme),
            TabId::Trace => render_trace_tab(frame, chunks[2], state, theme),
            TabId::Insights => render_insights_tab(frame, chunks[2], state, theme),
        }
    }

    render_footer(frame, chunks[3], state, theme);

    if let Some(modal) = &state.modal.clone() {
        apply_rgba_backdrop(
            frame.buffer_mut(),
            area,
            Rgba::new(6, 10, 18, 0.78),
            theme.no_color,
        );
        render_modal(frame, area, state, modal, theme);
    }
}

fn render_header(frame: &mut Frame, area: Rect, state: &AppState, theme: &Theme) {
    let spinner = BRAILLE_SPINNER[(state.tick_count as usize) % BRAILLE_SPINNER.len()];
    let rig_label = state
        .rig
        .as_deref()
        .map(sanitize_tui_str)
        .unwrap_or_else(|| "all".into());
    let win_label = sanitize_tui_str(&state.window);
    let mode_badge = state
        .window_mode
        .as_deref()
        .map(|m| format!(" [{m}]"))
        .unwrap_or_default();
    let status_pill = if state.loading_stage.is_some() {
        Span::styled(
            format!(
                " {spinner} {} ",
                state.loading_stage.as_deref().unwrap_or("Loading")
            ),
            theme.warn(),
        )
    } else if state.watcher_events > 0 {
        Span::styled(
            format!(" ● LIVE WATCH (+{} events) ", state.watcher_events),
            theme.band_running(),
        )
    } else {
        Span::styled(" ● READY ", theme.band_running())
    };

    let line = Line::from(vec![
        Span::styled(" DENDRO v0.8 ", theme.accent()),
        Span::styled("│", theme.muted()),
        status_pill,
        Span::styled("│ ", theme.muted()),
        Span::styled(format!("[r] Rig: {rig_label} ▾ "), theme.bg_base()),
        Span::styled("│ ", theme.muted()),
        Span::styled(
            format!("[w] Window: {win_label}{mode_badge} ▾ "),
            theme.bg_base(),
        ),
        Span::styled("│ ", theme.muted()),
        Span::styled("[:] Ask Nao  [?] Help  [q] Quit", theme.muted()),
    ]);
    let lens = Line::from(Span::styled(format!(" {ADAPTER_LENS}"), theme.muted()));
    frame.render_widget(Paragraph::new(vec![line, lens]), area);
}

fn render_tabs(frame: &mut Frame, area: Rect, state: &AppState, theme: &Theme) {
    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(area);
    let titles = [
        "[1] Pulse & Overview",
        "[2] Seats & Timeline",
        "[3] Handoffs & Flow",
        "[4] Tokens & Cache",
        "[5] Topology Simulator",
        "[6] Trace & Spotlights",
        "[7] Insights",
    ];
    let tabs = Tabs::new(titles.iter().copied().map(Line::from).collect::<Vec<_>>())
        .select(state.active_tab.index())
        .style(theme.muted())
        .highlight_style(theme.accent().add_modifier(Modifier::UNDERLINED));
    frame.render_widget(tabs, parts[0]);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!(" {DOMAIN_LEGEND}"),
            theme.muted(),
        ))),
        parts[1],
    );
    frame.render_widget(
        Block::default()
            .borders(Borders::BOTTOM)
            .border_style(theme.border(false)),
        parts[2],
    );
}

fn render_skeleton_loading(frame: &mut Frame, area: Rect, state: &AppState, theme: &Theme) {
    let spinner = BRAILLE_SPINNER[(state.tick_count as usize) % BRAILLE_SPINNER.len()];
    let stage = state
        .loading_stage
        .as_deref()
        .unwrap_or("Hydrating canonical analytics...");
    let status_info = if let Some(s) = &state.status {
        format!(
            "DB: {} · usage={} · activity={} · queue={}\nRecorded bounds: {} to {}",
            sanitize_tui_str(s["status"].as_str().unwrap_or("ready")),
            fmt_num(&s["usage_rows"]),
            fmt_num(&s["activity_rows"]),
            fmt_num(&s["queue_rows"]),
            sanitize_tui_str(s["min_timestamp"].as_str().unwrap_or("...")),
            sanitize_tui_str(s["max_timestamp"].as_str().unwrap_or("...")),
        )
    } else {
        "Inspecting local analytics database read-only...".into()
    };

    let block = Block::default()
        .title(" Instant Cockpit Preflight ")
        .borders(Borders::ALL)
        .border_style(theme.border(true));
    let text = vec![
        Line::from(""),
        Line::from(Span::styled(format!("  {spinner} {stage}"), theme.accent())),
        Line::from(""),
        Line::from(Span::styled(format!("  {status_info}"), theme.muted())),
        Line::from(""),
        Line::from(Span::styled(
            "  Forensic traces load lazily on demand so interactive navigation never blocks.",
            theme.muted(),
        )),
    ];
    frame.render_widget(Paragraph::new(text).block(block), area);
}

fn render_pulse_tab(frame: &mut Frame, area: Rect, state: &AppState, theme: &Theme) {
    let Some(report) = &state.report else { return };
    let p = &report["payload"];

    let v_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(5), Constraint::Min(8)])
        .split(area);

    // Top 5 KPI cards
    let kpi_areas = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(20),
            Constraint::Percentage(20),
            Constraint::Percentage(20),
            Constraint::Percentage(20),
            Constraint::Percentage(20),
        ])
        .split(v_chunks[0]);

    let m1_raw = fmt_num(&p["m1_yield"]);
    let token_raw = fmt_num(&p["tokens"]);
    let m1_sub = format!("raw {m1_raw} · tokens / nonreturned item");
    let token_sub = format!("raw {token_raw} · half-open [from, to)");
    let chain_sub = format!("chains: {}", fmt_num(&p["m3_rework_chains"]));
    let m4_sub = format!(
        "M4: {}",
        sanitize_tui_str(p["m4_impact"].as_str().unwrap_or("insufficient_data"))
    );
    let kpis = [
        (
            " M1 · Yield / Item ",
            fmt_compact(&p["m1_yield"]),
            m1_sub.as_str(),
        ),
        (
            " M2 · Return Rate ",
            fmt_pct(&p["m2_return_rate"]),
            "returned queue handoffs",
        ),
        (
            " M3 · Rework % ",
            fmt_pct(&p["m3_rework_proportion"]),
            chain_sub.as_str(),
        ),
        (
            " Recorded Tokens ",
            fmt_compact(&p["tokens"]),
            token_sub.as_str(),
        ),
        (
            " Eff. Parallelism ",
            fmt_num(&p["fleet_concurrency"]["effective_parallelism"]),
            m4_sub.as_str(),
        ),
    ];

    for (i, (title, val, sub)) in kpis.iter().enumerate() {
        let val_style = if val == "Unavailable" {
            theme.unavailable()
        } else {
            theme.accent()
        };
        let card = Paragraph::new(vec![
            Line::from(Span::styled(val.clone(), val_style)),
            Line::from(Span::styled((*sub).to_string(), theme.muted())),
        ])
        .block(
            Block::default()
                .title(*title)
                .borders(Borders::ALL)
                .border_style(theme.border(false)),
        );
        frame.render_widget(card, kpi_areas[i]);
    }

    // Bottom split: Left (Concurrency + Entity Hygiene), Right (Spotlights + Top Idle Seats)
    let bottom_chunks = if v_chunks[1].width >= 110 {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(v_chunks[1])
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(v_chunks[1])
    };

    // Left: Concurrency histogram + Entity hygiene
    let mut left_lines = vec![Line::from(Span::styled(
        "Concurrency Distribution (aggregate; not a temporal ribbon)",
        theme.muted(),
    ))];
    let total_conc: f64 = rows(&p["fleet_concurrency"]["histogram"])
        .filter_map(|r| r["secs"].as_f64())
        .sum();
    if total_conc <= 0.0 {
        left_lines.push(Line::from(Span::styled(
            "Unavailable: no observed concurrency duration.",
            theme.unavailable(),
        )));
    } else {
        for r in rows(&p["fleet_concurrency"]["histogram"]).take(6) {
            let secs = r["secs"].as_f64().unwrap_or(0.0);
            let filled = ((secs / total_conc * 26.0).round() as usize).min(26);
            let bar_str = format!("{}{}", "█".repeat(filled), "░".repeat(26 - filled));
            left_lines.push(Line::from(vec![
                Span::styled(
                    format!(" k={:<2} ", fmt_num(&r["active_seats"])),
                    theme.bg_base(),
                ),
                Span::styled(bar_str, theme.band_running()),
                Span::styled(format!(" {} s", fmt_num(&r["secs"])), theme.muted()),
            ]));
        }
    }
    left_lines.push(Line::from(""));
    if let Some(status) = &state.status {
        if let Some(eb) = status.get("entity_breakdown") {
            left_lines.push(Line::from(Span::styled(
                "Entity Hygiene Summary ([x] Toggle non-seat entities)",
                theme.accent(),
            )));
            left_lines.push(Line::from(format!(
                " ● {} Canonical Agent Seats (<role>@<rig>) · OpenRig adapter scope",
                fmt_num(&eb["agent_seats_count"])
            )));
            left_lines.push(Line::from(format!(
                " ○ {} Queue Endpoints (qitem-*) · {} System/Gates · {} Unlinked",
                fmt_num(&eb["queue_endpoints_count"]),
                fmt_num(&eb["system_and_gates_count"]),
                fmt_num(&eb["unlinked_ids_count"])
            )));
        }
    }
    frame.render_widget(
        Paragraph::new(left_lines)
            .block(
                Block::default()
                    .title(" Fleet Concurrency & Entity Hygiene ")
                    .borders(Borders::ALL)
                    .border_style(theme.border(false)),
            )
            .wrap(Wrap { trim: false }),
        bottom_chunks[0],
    );

    // Right: Anomaly Spotlights & Advisories
    let mut right_lines = Vec::new();
    let spotlights: Vec<&Value> = rows(&p["anomaly_spotlights"]["findings"]).collect();
    let advisories: Vec<&Value> = rows(&p["advisory_idle_claimed"]["findings"]).collect();
    if spotlights.is_empty() && advisories.is_empty() {
        right_lines.push(Line::from(Span::styled(
            "No spotlight breaches in evaluated evidence (check coverage before treating as negative).",
            theme.muted(),
        )));
    } else {
        for f in spotlights.iter().take(5) {
            let metric = sanitize_tui_str(f["metric"].as_str().unwrap_or("spotlight"));
            let seat = sanitize_tui_str(
                f["affected"]["seat"]
                    .as_str()
                    .or_else(|| f["affected"]["rig"].as_str())
                    .unwrap_or("fleet"),
            );
            right_lines.push(Line::from(vec![
                Span::styled(" ! [SPOTLIGHT] ", theme.breach()),
                Span::styled(format!("{metric} · {seat}"), theme.bg_base()),
                Span::styled(
                    format!(
                        " (obs={} vs thresh={})",
                        fmt_num(&f["value"]),
                        fmt_num(&f["threshold"])
                    ),
                    theme.muted(),
                ),
            ]));
        }
        for f in advisories.iter().take(4) {
            let cls = sanitize_tui_str(f["classification"].as_str().unwrap_or("advisory"));
            let seat = sanitize_tui_str(f["seat"].as_str().unwrap_or("seat"));
            right_lines.push(Line::from(vec![
                Span::styled(" ⚠ [ADVISORY]  ", theme.warn()),
                Span::styled(format!("{cls} · {seat}"), theme.bg_base()),
                Span::styled(
                    format!(
                        " ({}s > {}s)",
                        fmt_num(&f["duration_secs"]),
                        fmt_num(&f["threshold_secs"])
                    ),
                    theme.muted(),
                ),
            ]));
        }
    }
    right_lines.push(Line::from(""));
    right_lines.push(Line::from(Span::styled(
        "Press [i] to preview GitHub Issue Draft · [6] or [t] for Forensic Trace Explorer",
        theme.muted(),
    )));

    frame.render_widget(
        Paragraph::new(right_lines)
            .block(
                Block::default()
                    .title(format!(
                        " Anomaly Spotlights ({}) & Idle Advisories ({}) · cov={} ",
                        spotlights.len(),
                        advisories.len(),
                        sanitize_tui_str(
                            p["anomaly_spotlights"]["coverage"]
                                .as_str()
                                .unwrap_or("unknown")
                        )
                    ))
                    .borders(Borders::ALL)
                    .border_style(theme.border(true)),
            )
            .wrap(Wrap { trim: false }),
        bottom_chunks[1],
    );
}

fn render_seats_tab(frame: &mut Frame, area: Rect, state: &mut AppState, theme: &Theme) {
    let filtered = state.active_seat_rows();
    let chunks = if area.width >= 115 {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
            .split(area)
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(62), Constraint::Percentage(38)])
            .split(area)
    };

    let mode_name = match state.seat_view_mode {
        SeatViewMode::Bands => "5-Band Aggregate Ribbon",
        SeatViewMode::Timeline => "Shared-Axis State Timeline",
    };
    let group_name = match state.seat_group_mode {
        SeatGroupMode::BySeat => "Seat",
        SeatGroupMode::ByRig => "Team",
    };
    let filter_badge = if state.filter_input_mode || !state.seat_filter.is_empty() {
        format!(" · Filter [/]: '{}' ", sanitize_tui_str(&state.seat_filter))
    } else {
        " · [/] Filter  [g] Team  [m] Timeline  [x] Non-seats ".into()
    };

    let mut table_rows = Vec::new();
    for (idx, r) in filtered.iter().enumerate() {
        let seat_name = sanitize_tui_str(r["seat"].as_str().unwrap_or("unknown"));
        let is_selected = idx == state.selected_seat;
        let prefix = if is_selected { "› " } else { "  " };
        let ribbon_line = match state.seat_view_mode {
            SeatViewMode::Bands => build_colored_band_line(r, 28, theme),
            SeatViewMode::Timeline => {
                if let Some(rep) = &state.report {
                    build_timeline_strip(r, &rep["payload"], 28, theme)
                } else {
                    build_colored_band_line(r, 28, theme)
                }
            }
        };
        let obs = fmt_num(&r["observed_secs"]);
        let row_style = if is_selected {
            Style::default().add_modifier(Modifier::REVERSED)
        } else {
            Style::default()
        };
        table_rows.push(
            Row::new(vec![
                Cell::from(format!("{prefix}{seat_name}")),
                Cell::from(ribbon_line),
                Cell::from(format!("{obs} s")),
            ])
            .style(row_style),
        );
    }

    let total_raw = state
        .report
        .as_ref()
        .and_then(|r| r["payload"]["activity"].as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    let hidden = total_raw.saturating_sub(filtered.len());

    let table = Table::new(
        table_rows,
        [
            Constraint::Length(28),
            Constraint::Length(30),
            Constraint::Length(12),
        ],
    )
    .header(
        Row::new(vec![group_name, mode_name, "Observed"])
            .style(theme.accent().add_modifier(Modifier::BOLD)),
    )
    .block(
        Block::default()
            .title(format!(
                " {group_name}s ({}/{total_raw} shown, {hidden} hidden){filter_badge}",
                filtered.len()
            ))
            .borders(Borders::ALL)
            .border_style(theme.border(true)),
    );
    state.seat_table_state.select(Some(state.selected_seat));
    frame.render_stateful_widget(table, chunks[0], &mut state.seat_table_state);

    // Right-hand Seat / Team Inspector
    let mut insp_lines = Vec::new();
    let inspector_title = match state.seat_group_mode {
        SeatGroupMode::BySeat => " Synchronized Seat Inspector ",
        SeatGroupMode::ByRig => " Synchronized Team Inspector ",
    };
    if let Some(r) = filtered.get(state.selected_seat) {
        let seat_name = sanitize_tui_str(r["seat"].as_str().unwrap_or("unknown"));
        let band_total = [
            "active_secs",
            "idle_with_work_secs",
            "idle_without_work_secs",
            "waiting_human_secs",
            "unknown_secs",
        ]
        .iter()
        .map(|key| r[*key].as_f64().filter(|n| n.is_finite()).unwrap_or(0.0))
        .sum::<f64>();
        let observed = r["observed_secs"]
            .as_f64()
            .filter(|n| n.is_finite() && *n > 0.0)
            .unwrap_or(band_total);
        let label = match state.seat_group_mode {
            SeatGroupMode::BySeat => "Seat",
            SeatGroupMode::ByRig => "Team",
        };
        insp_lines.push(Line::from(vec![
            Span::styled(format!("{label}: "), theme.muted()),
            Span::styled(seat_name.clone(), theme.accent()),
        ]));
        insp_lines.push(Line::from(format!(
            "Observed Duration: {}",
            fmt_duration_share(&r["observed_secs"], observed)
        )));
        insp_lines.push(Line::from(""));
        insp_lines.push(Line::from(vec![
            Span::styled(" █ RUNNING            ", theme.band_running()),
            Span::raw(fmt_duration_share(&r["active_secs"], observed)),
        ]));
        insp_lines.push(Line::from(vec![
            Span::styled(" ▓ IDLE-WITH-WORK     ", theme.band_idle_with_work()),
            Span::raw(fmt_duration_share(&r["idle_with_work_secs"], observed)),
        ]));
        insp_lines.push(Line::from(vec![
            Span::styled("   ├─ Container Hold  ", theme.band_container_hold()),
            Span::raw(format!(
                "{} ▨",
                fmt_duration_share(&r["unparked_container_secs"], observed)
            )),
        ]));
        insp_lines.push(Line::from(vec![
            Span::styled("   └─ True Leaf Stall ", theme.warn()),
            Span::raw(fmt_duration_share(&r["leaf_stall_secs"], observed)),
        ]));
        insp_lines.push(Line::from(vec![
            Span::styled(" ░ IDLE-WITHOUT-WORK  ", theme.band_idle_no_work()),
            Span::raw(fmt_duration_share(&r["idle_without_work_secs"], observed)),
        ]));
        insp_lines.push(Line::from(vec![
            Span::styled(" ▒ NEEDS-INPUT        ", theme.band_needs_input()),
            Span::raw(fmt_duration_share(&r["waiting_human_secs"], observed)),
        ]));
        insp_lines.push(Line::from(vec![
            Span::styled(" ? UNKNOWN            ", theme.band_unknown()),
            Span::raw(fmt_duration_share(&r["unknown_secs"], observed)),
        ]));
        insp_lines.push(Line::from(""));
        insp_lines.push(Line::from(format!(
            "Tool Calls: {} eng (█) / {} ceremony (░) · ratio={}",
            fmt_num(&r["engineering_calls"]),
            fmt_num(&r["ceremony_calls"]),
            fmt_pct(&r["ceremony_ratio"])
        )));
        let matched = state.report.as_ref().and_then(|rep| {
            rows(&rep["payload"]["m1_by_seat"])
                .find(|m| m["seat"].as_str() == r["seat"].as_str())
                .cloned()
        });
        let embedded = r
            .get("tokens")
            .filter(|value| !value.is_null())
            .map(|_| r.clone());
        if let Some(m1) = matched.or(embedded) {
            insp_lines.push(Line::from(format!(
                "M1 Yield: {} tok/item ({}) · Closed: {} · Tokens: {} ({})",
                fmt_compact(&m1["tokens_per_closed_item"]),
                fmt_num(&m1["tokens_per_closed_item"]),
                fmt_num(&m1["closed_items"]),
                fmt_compact(&m1["tokens"]),
                fmt_num(&m1["tokens"])
            )));
        }
        insp_lines.push(Line::from(""));
        insp_lines.push(Line::from(Span::styled(
            "[Enter] Inspect JSON  [s] Send to Simulator  [g] Team/Rig  [t] Trace",
            theme.muted(),
        )));
    } else {
        insp_lines.push(Line::from(Span::styled(
            "No matching seats in current scope.",
            theme.unavailable(),
        )));
    }

    frame.render_widget(
        Paragraph::new(insp_lines)
            .block(
                Block::default()
                    .title(inspector_title)
                    .borders(Borders::ALL)
                    .border_style(theme.border(false)),
            )
            .wrap(Wrap { trim: false }),
        chunks[1],
    );
}

pub(super) fn build_colored_band_line(r: &Value, width: usize, theme: &Theme) -> Line<'static> {
    let spec: [(&str, char, Style); 5] = [
        ("active_secs", '█', theme.band_running()),
        ("idle_with_work_secs", '▓', theme.band_idle_with_work()),
        ("idle_without_work_secs", '░', theme.band_idle_no_work()),
        ("waiting_human_secs", '▒', theme.band_needs_input()),
        ("unknown_secs", '?', theme.band_unknown()),
    ];
    let weights: Vec<f64> = spec.iter().map(|(key, _, _)| value_f64(&r[*key])).collect();
    if width == 0 || weights.iter().all(|weight| *weight <= 0.0) {
        return Line::from(Span::styled("?".repeat(width), theme.band_unknown()));
    }
    // Positive non-unknown bands keep at least one cell so active work cannot round away.
    let cells = allocate_cells(&weights, width, |index| index < 4);
    let mut spans = Vec::new();
    let mut used = 0usize;
    for (index, (_, ch, style)) in spec.iter().enumerate() {
        let n = cells
            .get(index)
            .copied()
            .unwrap_or(0)
            .min(width.saturating_sub(used));
        if n > 0 {
            used += n;
            spans.push(Span::styled(ch.to_string().repeat(n), *style));
        }
    }
    if used < width {
        spans.push(Span::styled("?".repeat(width - used), theme.band_unknown()));
    }
    if spans.is_empty() {
        spans.push(Span::styled("?".repeat(width), theme.band_unknown()));
    }
    Line::from(spans)
}

fn build_timeline_strip(r: &Value, payload: &Value, width: usize, theme: &Theme) -> Line<'static> {
    let Some(seat) = r["seat"].as_str() else {
        return Line::from(Span::styled("?".repeat(width), theme.band_unknown()));
    };
    let evs: Vec<&Value> = rows(&payload["activity_events"])
        .filter(|e| e["seat"].as_str() == Some(seat))
        .collect();
    if evs.is_empty() {
        return build_colored_band_line(r, width, theme);
    }
    let mut spans = Vec::with_capacity(width);
    for i in 0..width {
        let idx = (i * evs.len()) / width;
        let state_str = evs
            .get(idx)
            .and_then(|e| e["state"].as_str())
            .unwrap_or("unknown");
        let (ch, st) = match state_str {
            "running" => ("█", theme.band_running()),
            "idle" => ("▓", theme.band_idle_with_work()),
            "needs_input" => ("▒", theme.band_needs_input()),
            _ => ("?", theme.band_unknown()),
        };
        spans.push(Span::styled(ch.to_string(), st));
    }
    Line::from(spans)
}

fn render_handoffs_tab(frame: &mut Frame, area: Rect, state: &AppState, theme: &Theme) {
    let Some(report) = &state.report else { return };
    let p = &report["payload"];

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
        .split(area);

    let metric_label = match state.heatmap_metric {
        HeatmapMetric::ReturnRate => "Return Rate (%)",
        HeatmapMetric::Count => "Handoff Count",
        HeatmapMetric::WaitMedian => "Pickup Wait Median (s)",
        HeatmapMetric::ExecMedian => "Execution Median (s)",
    };

    let pairs: Vec<&Value> = rows(&p["handoff_friction"]["pairs"]).collect();
    let mut pair_rows = Vec::new();
    for (idx, r) in pairs.iter().take(25).enumerate() {
        let from_s = sanitize_tui_str(r["source_seat"].as_str().unwrap_or("?"));
        let to_s = sanitize_tui_str(r["dest_seat"].as_str().unwrap_or("?"));
        let boundary = sanitize_tui_str(r["ring_boundary"].as_str().unwrap_or("unknown"));
        let val_str = match state.heatmap_metric {
            HeatmapMetric::ReturnRate => fmt_pct(&r["return_rate"]),
            HeatmapMetric::Count => fmt_num(&r["count"]),
            HeatmapMetric::WaitMedian => r["wait_median_ms"]
                .as_f64()
                .map(|ms| format!("{:.1} s", ms / 1000.0))
                .unwrap_or_else(|| "—".into()),
            HeatmapMetric::ExecMedian => r["exec_median_ms"]
                .as_f64()
                .map(|ms| format!("{:.1} s", ms / 1000.0))
                .unwrap_or_else(|| "—".into()),
        };
        let is_high_ret = r["return_rate"].as_f64().unwrap_or(0.0) >= 0.20;
        let style = if idx == state.selected_pair {
            Style::default().add_modifier(Modifier::REVERSED)
        } else if is_high_ret {
            theme.warn()
        } else {
            Style::default()
        };
        pair_rows.push(
            Row::new(vec![
                Cell::from(from_s),
                Cell::from(to_s),
                Cell::from(fmt_num(&r["count"])),
                Cell::from(fmt_num(&r["return_count"])),
                Cell::from(val_str),
                Cell::from(boundary),
            ])
            .style(style),
        );
    }

    let table = Table::new(
        pair_rows,
        [
            Constraint::Length(20),
            Constraint::Length(20),
            Constraint::Length(8),
            Constraint::Length(8),
            Constraint::Length(14),
            Constraint::Length(16),
        ],
    )
    .header(
        Row::new(vec![
            "Sender ↓",
            "Receiver →",
            "Count",
            "Returns",
            metric_label,
            "Ring Boundary",
        ])
        .style(theme.accent()),
    )
    .block(
        Block::default()
            .title(format!(
                " Handoff Friction Matrix & Flow ([v] Cycle Metric: {metric_label}) "
            ))
            .borders(Borders::ALL)
            .border_style(theme.border(true)),
    );
    frame.render_widget(table, chunks[0]);

    // Right pane: Vortex loops & boundary summary
    let mut right_lines = vec![
        Line::from(Span::styled(
            "M3 Rework Chains & Vortex Loops",
            theme.accent(),
        )),
        Line::from(format!(
            "Observed M3 Rework Chains: {} ({})",
            fmt_num(&p["m3_rework_chains"]),
            fmt_pct(&p["m3_rework_proportion"])
        )),
        Line::from(""),
    ];
    let vortex: Vec<&Value> = rows(&p["handoff_friction"]["vortex_loops"]).collect();
    if vortex.is_empty() {
        right_lines.push(Line::from(Span::styled(
            "No vortex loops breached thresholds in this window.",
            theme.muted(),
        )));
    } else {
        for v in vortex.iter().take(6) {
            right_lines.push(Line::from(vec![
                Span::styled(" ⚠ Loop ", theme.warn()),
                Span::raw(format!(
                    "chain={} · returns={} · rework_tok={}",
                    sanitize_tui_str(v["chain_id"].as_str().unwrap_or("?")),
                    fmt_num(&v["return_count"]),
                    fmt_num(&v["rework_tokens"])
                )),
            ]));
        }
    }
    right_lines.push(Line::from(""));
    right_lines.push(Line::from(Span::styled(
        "Epistemic Note: Handoff counts represent queue flow, not causal token transfer. Dark/dash (—) cells are Unavailable, never measured zero.",
        theme.muted(),
    )));

    frame.render_widget(
        Paragraph::new(right_lines)
            .block(
                Block::default()
                    .title(" Vortex Loops & Epistemic Boundaries ")
                    .borders(Borders::ALL)
                    .border_style(theme.border(false)),
            )
            .wrap(Wrap { trim: false }),
        chunks[1],
    );
}

fn render_tokens_tab(frame: &mut Frame, area: Rect, state: &mut AppState, theme: &Theme) {
    let Some(report) = &state.report else { return };
    let p = &report["payload"];

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(area);

    let token_rows = state.token_seat_rows();
    let token_total: f64 = token_rows.iter().map(|row| value_f64(&row["tokens"])).sum();
    let max_token = token_rows.len().saturating_sub(1);
    if state.token_cursor > max_token {
        state.token_cursor = max_token;
    }
    let mut seat_rows = Vec::new();
    for (idx, r) in token_rows.iter().enumerate() {
        let seat = sanitize_tui_str(r["seat"].as_str().unwrap_or("?"));
        let style = if idx == state.token_cursor {
            Style::default().add_modifier(Modifier::REVERSED)
        } else {
            Style::default()
        };
        let prefix = if idx == state.token_cursor {
            "› "
        } else {
            "  "
        };
        seat_rows.push(
            Row::new(vec![
                Cell::from(format!("{prefix}{seat}")),
                Cell::from(format!(
                    "{} {}",
                    fmt_compact(&r["tokens"]),
                    fmt_share_pct(&r["tokens"], token_total)
                )),
                Cell::from(fmt_num(&r["closed_items"])),
                Cell::from(fmt_num(&r["nonreturn_deliveries"])),
                Cell::from(fmt_compact(&r["tokens_per_closed_item"])),
            ])
            .style(style),
        );
    }

    let table = Table::new(
        seat_rows,
        [
            Constraint::Length(28),
            Constraint::Length(20),
            Constraint::Length(12),
            Constraint::Length(12),
            Constraint::Length(16),
        ],
    )
    .header(
        Row::new(vec![
            "Seat",
            "Total Tokens",
            "Closed Items",
            "Non-Return",
            "Tokens / Closed",
        ])
        .style(theme.accent()),
    )
    .block(
        Block::default()
            .title(" Per-Seat Token Consumption & M1 Yield  [↑/↓] ")
            .borders(Borders::ALL)
            .border_style(theme.border(true)),
    );
    state.token_table_state.select(Some(state.token_cursor));
    frame.render_stateful_widget(table, chunks[0], &mut state.token_table_state);

    let tc = &p["token_categories"];
    let ti = &p["task_intake_overhead"];
    let category_parts = [
        (
            value_f64(&tc["input_uncached"]),
            '█',
            theme.band_running(),
            "Input Uncached",
            &tc["input_uncached"],
        ),
        (
            value_f64(&tc["cache_read"]),
            '▓',
            theme.band_idle_with_work(),
            "Cache Read",
            &tc["cache_read"],
        ),
        (
            value_f64(&tc["cache_write"]),
            '▒',
            theme.band_needs_input(),
            "Cache Write",
            &tc["cache_write"],
        ),
        (
            value_f64(&tc["output"]),
            '░',
            theme.band_idle_no_work(),
            "Output",
            &tc["output"],
        ),
    ];
    let category_total: f64 = category_parts.iter().map(|part| part.0).sum();
    let category_bar = share_bar(
        &category_parts
            .iter()
            .map(|part| (part.0, part.1, part.2))
            .collect::<Vec<_>>(),
        28,
    );
    let intake_parts = [
        (
            value_f64(&ti["assigned_mission_tokens"]),
            '█',
            theme.band_running(),
            "Assigned Mission",
            &ti["assigned_mission_tokens"],
        ),
        (
            value_f64(&ti["task_intake_overhead"]),
            '▓',
            theme.band_idle_with_work(),
            "Intake Overhead",
            &ti["task_intake_overhead"],
        ),
        (
            value_f64(&ti["ambiguous_tokens"]),
            '▲',
            theme.warn(),
            "Ambiguous/Collide",
            &ti["ambiguous_tokens"],
        ),
        (
            value_f64(&ti["unlinked_session"]),
            '○',
            theme.muted(),
            "Unlinked Session",
            &ti["unlinked_session"],
        ),
    ];
    let intake_total: f64 = intake_parts.iter().map(|part| part.0).sum();
    let intake_bar = share_bar(
        &intake_parts
            .iter()
            .map(|part| (part.0, part.1, part.2))
            .collect::<Vec<_>>(),
        28,
    );
    let output_total = tc["output"]
        .as_f64()
        .filter(|n| n.is_finite())
        .unwrap_or(0.0);
    let reasoning_line = if tc["reasoning"].as_f64().is_some() {
        format!(
            "   Reasoning (of output): {} {}",
            fmt_compact(&tc["reasoning"]),
            fmt_share_pct(&tc["reasoning"], output_total)
        )
    } else {
        format!(
            "   Reasoning: {} (part of output; never double-counted)",
            sanitize_tui_str(tc["reasoning"].as_str().unwrap_or("not recorded"))
        )
    };
    let mut right_lines = vec![
        Line::from(Span::styled("Token Category Mix (AC6)", theme.accent())),
        category_bar,
    ];
    for part in &category_parts {
        right_lines.push(Line::from(format!(
            " {} {:<16} {} {}",
            part.1,
            part.3,
            fmt_compact(part.4),
            fmt_share_pct(part.4, category_total)
        )));
    }
    right_lines.push(Line::from(reasoning_line));
    right_lines.push(Line::from(Span::styled(
        " (Reasoning is part of output; never double-counted)",
        theme.muted(),
    )));
    right_lines.push(Line::from(""));
    right_lines.push(Line::from(Span::styled(
        "Task-Intake Classification (Analytical)",
        theme.accent(),
    )));
    right_lines.push(intake_bar);
    for part in &intake_parts {
        right_lines.push(Line::from(format!(
            " {} {:<18} {} {}",
            part.1,
            part.3,
            fmt_compact(part.4),
            fmt_share_pct(part.4, intake_total)
        )));
    }
    right_lines.push(Line::from(""));
    right_lines.push(Line::from(Span::styled(
        "Cross-Slice Cache Bleed & Compactions",
        theme.accent(),
    )));
    right_lines.push(Line::from(format!(
        " Context Compaction Events: {}",
        rows(&p["context_compactions"]["events"]).count()
    )));
    right_lines.push(Line::from(""));
    right_lines.push(Line::from(Span::styled(
        "Cost (report values; unknown is not zero)",
        theme.accent(),
    )));
    right_lines.push(Line::from(fmt_report_cost("Observed", &p["observed_cost"])));
    right_lines.push(Line::from(fmt_report_cost(
        "Estimated",
        &p["estimated_cost"],
    )));

    frame.render_widget(
        Paragraph::new(right_lines)
            .block(
                Block::default()
                    .title(" Token Categories & Attribution Audit ")
                    .borders(Borders::ALL)
                    .border_style(theme.border(false)),
            )
            .wrap(Wrap { trim: false }),
        chunks[1],
    );
}

fn token_object_total(value: &Value) -> f64 {
    ["input_uncached", "output", "cache_read", "cache_write"]
        .iter()
        .map(|key| {
            value[*key]
                .as_f64()
                .filter(|n| n.is_finite())
                .unwrap_or(0.0)
        })
        .sum()
}

fn render_simulate_tab(frame: &mut Frame, area: Rect, state: &mut AppState, theme: &Theme) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(48), Constraint::Percentage(52)])
        .split(area);

    let seats = state.canonical_seat_names();
    let list_len = 4 + seats.len();
    let cursor = state.sim_cursor.min(list_len.saturating_sub(1));
    let mut rows: Vec<(String, Style)> = Vec::with_capacity(list_len);
    let on_off = |enabled: bool| if enabled { "ON ●" } else { "OFF ○" };
    rows.push((
        format!(
            "[d] Direct Leaf Handoffs          {}",
            on_off(state.sim_scenario.direct_handoffs)
        ),
        if state.sim_scenario.direct_handoffs {
            theme.band_running()
        } else {
            Style::default()
        },
    ));
    rows.push((
        format!(
            "[f] Enforce 4-Eyes Model Family   {}",
            on_off(state.sim_scenario.enforce_model_family_diversity)
        ),
        if state.sim_scenario.enforce_model_family_diversity {
            theme.band_running()
        } else {
            Style::default()
        },
    ));
    let threshold = state
        .sim_scenario
        .boundary_reset_threshold
        .map(|n| fmt_compact_f64(n as f64))
        .unwrap_or_else(|| "OFF ○".into());
    rows.push((
        format!("[←/→] Boundary Reset Threshold   {threshold}"),
        if state.sim_scenario.boundary_reset_threshold.is_some() {
            theme.accent()
        } else {
            Style::default()
        },
    ));
    rows.push(("[c] Clear Scenario Overrides".to_string(), theme.muted()));
    let m1_tokens: BTreeMap<&str, &Value> = state
        .report
        .as_ref()
        .and_then(|report| report["payload"]["m1_by_seat"].as_array())
        .map(|rows| {
            rows.iter()
                .filter_map(|row| row["seat"].as_str().map(|seat| (seat, &row["tokens"])))
                .collect()
        })
        .unwrap_or_default();
    for seat in &seats {
        let is_rem = state.sim_scenario.remove.iter().any(|name| name == seat);
        let is_rep = state.sim_scenario.replacement_seat.as_ref() == Some(seat);
        let badge = if is_rem {
            "[REMOVE] "
        } else if is_rep {
            "[REPLACE]"
        } else {
            "[      ] "
        };
        let tokens = m1_tokens
            .get(seat.as_str())
            .map(|value| fmt_compact(value))
            .unwrap_or_else(|| "—".into());
        let style = if is_rem {
            theme.breach()
        } else if is_rep {
            theme.band_running()
        } else {
            Style::default()
        };
        rows.push((
            format!("{badge} {}  {tokens}", sanitize_tui_str(seat)),
            style,
        ));
    }
    let inner_h = chunks[0].height.saturating_sub(2) as usize;
    let view_h = inner_h.saturating_sub(3).max(1);
    let max_start = rows.len().saturating_sub(view_h);
    let start = cursor
        .saturating_sub(view_h.saturating_sub(1))
        .min(max_start);
    let mut left_lines = vec![
        Line::from(Span::styled(
            "Counterfactual Topology Controls (<1ms In-Memory)",
            theme.accent(),
        )),
        Line::from(Span::styled(
            "↑/↓ Select · [Space/Enter] Activate · [p] Replace",
            theme.muted(),
        )),
        Line::from(""),
    ];
    for (idx, (label, style)) in rows.iter().enumerate().skip(start).take(view_h) {
        let selected = idx == cursor;
        let row_style = if selected {
            style.add_modifier(Modifier::REVERSED)
        } else {
            *style
        };
        let marker = if selected { "›" } else { " " };
        left_lines.push(Line::from(Span::styled(
            format!("{marker} {label}"),
            row_style,
        )));
    }

    frame.render_widget(
        Paragraph::new(left_lines)
            .block(
                Block::default()
                    .title(" Scenario Builder ")
                    .borders(Borders::ALL)
                    .border_style(theme.border(true)),
            )
            .wrap(Wrap { trim: false }),
        chunks[0],
    );

    // Right pane: Live Simulation Result
    let mut right_lines = Vec::new();
    if let Some(sim) = &state.sim_result {
        let status = sim["status"].as_str().unwrap_or("unknown");
        let projected = status == "projected" || status == "ok";
        right_lines.push(Line::from(vec![
            Span::styled("Simulation Verdict: ", theme.muted()),
            Span::styled(
                status.to_uppercase(),
                if projected {
                    theme.band_running()
                } else {
                    theme.warn()
                },
            ),
        ]));
        if let Some(reasons) = sim["reasons"].as_array() {
            for reason in reasons {
                if let Some(text) = reason.as_str() {
                    right_lines.push(Line::from(Span::styled(
                        format!("Reason: {}", sanitize_tui_str(text)),
                        theme.warn(),
                    )));
                }
            }
        }
        if let Some(reason) = sim["reason"].as_str() {
            right_lines.push(Line::from(Span::styled(
                format!("Reason: {}", sanitize_tui_str(reason)),
                theme.warn(),
            )));
        }
        right_lines.push(Line::from(""));
        let base_u = sim["baseline"]["input_units"].as_f64().unwrap_or(0.0);
        let proj_u = sim["projection"]["input_units"].as_f64().unwrap_or(0.0);
        right_lines.push(Line::from(format!(
            "Baseline Input Units:  {}",
            fmt_compact_f64(base_u)
        )));
        right_lines.push(Line::from(format!(
            "Projected Input Units: {} {}",
            fmt_compact_f64(proj_u),
            fmt_delta_pct(proj_u, base_u)
        )));
        right_lines.push(Line::from(format!(
            "Reduction Ratio:       {}",
            fmt_pct(&sim["projection"]["reduction_ratio"])
        )));
        if base_u > 0.0 && proj_u >= 0.0 {
            let ratio = (proj_u / base_u).clamp(0.0, 1.5);
            let filled = ((ratio * 24.0).round() as usize).min(36);
            right_lines.push(Line::from(vec![
                Span::raw("Projected Bar: "),
                Span::styled("█".repeat(filled), theme.accent()),
            ]));
        }
        let base_tokens = token_object_total(&sim["baseline"]["tokens"]);
        let retained = token_object_total(&sim["conservation"]["retained_historical_tokens"]);
        let bypassed = token_object_total(&sim["conservation"]["bypassed_coordination_tokens"]);
        let unassigned = token_object_total(&sim["conservation"]["unassigned_work_tokens"]);
        let share_base = if base_tokens > 0.0 {
            base_tokens
        } else {
            base_u
        };
        right_lines.push(Line::from(""));
        right_lines.push(Line::from(Span::styled(
            format!(
                "Token Conservation: conserved={} · unassigned_records={}",
                sim["conservation"]["conserved"].as_bool().unwrap_or(false),
                fmt_num(&sim["conservation"]["unassigned_work_records"])
            ),
            theme.accent(),
        )));
        right_lines.push(Line::from(format!(
            "  Retained:   {} {}",
            fmt_compact_f64(retained),
            fmt_share_pct(&serde_json::json!(retained), share_base)
        )));
        right_lines.push(Line::from(format!(
            "  Unassigned: {} {}",
            fmt_compact_f64(unassigned),
            fmt_share_pct(&serde_json::json!(unassigned), share_base)
        )));
        right_lines.push(Line::from(format!(
            "  Bypassed:   {} {}",
            fmt_compact_f64(bypassed),
            fmt_share_pct(&serde_json::json!(bypassed), share_base)
        )));
        let boundary = &sim["boundary"];
        let boundary_status = boundary["status"].as_str().unwrap_or("not_requested");
        right_lines.push(Line::from(""));
        right_lines.push(Line::from(Span::styled(
            format!("Boundary Reset projection: {boundary_status}"),
            theme.accent(),
        )));
        if boundary.get("warm_input_units").is_some() {
            let warm = boundary["warm_input_units"].as_f64().unwrap_or(0.0);
            let reset = boundary["boundary_input_units"].as_f64().unwrap_or(0.0);
            right_lines.push(Line::from(format!(
                "  Warm {} · Reset {} {} · Reduction {}",
                fmt_compact_f64(warm),
                fmt_compact_f64(reset),
                fmt_delta_pct(reset, warm),
                fmt_pct(&boundary["reduction_ratio"])
            )));
        } else if let Some(model) = &state.sim_scenario.boundary {
            right_lines.push(Line::from(format!(
                "  Assumptions: {} turns · stale {} · clean {} · cold {}",
                model.turns,
                fmt_compact_f64(model.stale_context_tokens as f64),
                fmt_compact_f64(model.clean_context_tokens as f64),
                fmt_compact_f64(model.cold_boot_tokens as f64)
            )));
        } else {
            right_lines.push(Line::from(
                "  Not requested. [←/→] or [Space] on the threshold row enables it.",
            ));
        }
        if let Some(eligible) = boundary.get("eligible") {
            right_lines.push(Line::from(format!(
                "  Eligible at threshold: {}",
                sanitize_tui_str(&eligible.to_string())
            )));
        }
        let cycle = sim["routing"]["median_based_projected_total_cycle_reduction_ms"]
            .as_f64()
            .or_else(|| sim["routing"]["projected_cycle_reduction_ms"].as_f64());
        if let Some(cycle) = cycle {
            right_lines.push(Line::from(format!(
                "Direct Routing Cycle Delta: {} ms",
                fmt_compact_f64(cycle)
            )));
        }
        right_lines.push(Line::from(""));
        let unit = sim["projection"]["unit"]
            .as_str()
            .or_else(|| sim["unit"].as_str())
            .unwrap_or("uncached-equivalent input units (not currency)");
        right_lines.push(Line::from(Span::styled(
            format!("Unit: {}", sanitize_tui_str(unit)),
            theme.muted(),
        )));
        right_lines.push(Line::from(Span::styled(
            "Epistemic Limit: Historical scenario projection only; does not establish causal savings or mutate live OpenRig configs.",
            theme.muted(),
        )));
    } else {
        right_lines.push(Line::from(Span::styled(
            "Toggle any control on the left to evaluate a counterfactual scenario.",
            theme.muted(),
        )));
        right_lines.push(Line::from(""));
        right_lines.push(Line::from(Span::styled(
            "Token Conservation: Retained — · Unassigned — · Bypassed —",
            theme.muted(),
        )));
        right_lines.push(Line::from(Span::styled(
            "Boundary Reset projection: not requested",
            theme.muted(),
        )));
    }

    frame.render_widget(
        Paragraph::new(right_lines)
            .block(
                Block::default()
                    .title(" Live Counterfactual Projection & Conservation ")
                    .borders(Borders::ALL)
                    .border_style(theme.border(false)),
            )
            .wrap(Wrap { trim: false }),
        chunks[1],
    );
}

fn render_trace_tab(frame: &mut Frame, area: Rect, state: &AppState, theme: &Theme) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(area);

    let mut left_lines = vec![
        Line::from(Span::styled(
            "Forensic Traces ([1] handoff_friction · [2] activity · [3] tokens)",
            theme.accent(),
        )),
        Line::from(""),
    ];

    if let Some(trace) = state.traces.get(&state.active_trace_metric) {
        let inputs = &trace["inputs"];
        left_lines.push(Line::from(format!(
            "Active Trace Metric: {}",
            sanitize_tui_str(&state.active_trace_metric)
        )));
        left_lines.push(Line::from(format!(
            "Formula: {}",
            sanitize_tui_str(
                inputs["tool_loop_trace"]["formula_version"]
                    .as_str()
                    .unwrap_or("k11-tool-loop-trace-v1")
            )
        )));
        left_lines.push(Line::from(format!(
            "Tool Loops Detected: {}",
            fmt_num(&inputs["tool_loops"]["count"])
        )));
        left_lines.push(Line::from(format!(
            "Outcome Coverage: evaluated={} · excluded={}",
            fmt_num(&inputs["outcome_coverage"]["evaluated_sources"]),
            rows(&inputs["outcome_coverage_exclusions"]).count()
        )));
        left_lines.push(Line::from(""));
        for (idx, ep) in rows(&inputs["tool_loop_trace"]["episodes"])
            .take(10)
            .enumerate()
        {
            left_lines.push(Line::from(format!(
                " • Episode #{idx}: harness={} · span={}s",
                sanitize_tui_str(ep["harness"].as_str().unwrap_or("?")),
                sanitize_tui_str(ep["span_seconds"].as_str().unwrap_or("?"))
            )));
        }
    } else {
        left_lines.push(Line::from(Span::styled(
            format!(
                "Trace '{}' loading or press [1]/[2]/[3] to load on demand...",
                sanitize_tui_str(&state.active_trace_metric)
            ),
            theme.warn(),
        )));
    }
    left_lines.push(Line::from(""));
    left_lines.push(Line::from(Span::styled(
        "[1/2/3] Switch Trace Metric · [i] Preview Issue Draft Modal",
        theme.muted(),
    )));

    frame.render_widget(
        Paragraph::new(left_lines)
            .block(
                Block::default()
                    .title(" Tool-Loop & Trace Summary ")
                    .borders(Borders::ALL)
                    .border_style(theme.border(true)),
            )
            .wrap(Wrap { trim: false }),
        chunks[0],
    );

    let right_text = if let Some(trace) = state.traces.get(&state.active_trace_metric) {
        let pretty = serde_json::to_string_pretty(trace).unwrap_or_default();
        sanitize_tui_str(&pretty)
            .lines()
            .skip(state.trace_scroll)
            .take(area.height as usize)
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        "Waiting for on-demand trace evaluation...".into()
    };

    frame.render_widget(
        Paragraph::new(right_text)
            .block(
                Block::default()
                    .title(" Redacted Forensic Evidence JSON (↑/↓ Scroll) ")
                    .borders(Borders::ALL)
                    .border_style(theme.border(false)),
            )
            .wrap(Wrap { trim: false }),
        chunks[1],
    );
}

fn domain_name(domain: InsightDomain) -> &'static str {
    match domain {
        InsightDomain::Model => "model",
        InsightDomain::Harness => "harness",
        InsightDomain::Coordination => "coordination",
        InsightDomain::Environment => "environment",
        InsightDomain::Security => "security",
    }
}

fn status_label(status: &InsightStatus) -> String {
    match status {
        InsightStatus::Computed => "computed".to_string(),
        InsightStatus::Partial { reason } => {
            format!("partial ({})", sanitize_tui_str(reason))
        }
        InsightStatus::Refused { reason } => {
            format!("refused ({})", sanitize_tui_str(reason))
        }
    }
}

fn fmt_measure_value(value: Option<MeasureValue>) -> String {
    match value {
        Some(MeasureValue::Int(number)) => number.to_string(),
        Some(MeasureValue::Float(number)) if number.is_finite() => number.to_string(),
        _ => "unknown".to_string(),
    }
}

fn population_label(result: &InsightResult) -> String {
    match result.status {
        InsightStatus::Refused { .. } => "—".to_string(),
        _ => format!(
            "{} n={}",
            sanitize_tui_str(&result.population.what),
            result.population.n
        ),
    }
}

fn measures_label(result: &InsightResult) -> String {
    if result.measures.is_empty() {
        return "—".to_string();
    }
    result
        .measures
        .iter()
        .map(|measure| {
            format!(
                "{} {} {}",
                sanitize_tui_str(&measure.name),
                fmt_measure_value(measure.value),
                sanitize_tui_str(&measure.unit)
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn domain_status_label(rows: &[InsightResult]) -> String {
    if rows.is_empty() {
        return "unknown (no registered insight)".to_string();
    }
    let mut computed = 0u64;
    let mut partial = 0u64;
    let mut refused = 0u64;
    for row in rows {
        match row.status {
            InsightStatus::Computed => computed += 1,
            InsightStatus::Partial { .. } => partial += 1,
            InsightStatus::Refused { .. } => refused += 1,
        }
    }
    if computed > 0 && partial == 0 && refused == 0 {
        "computed".to_string()
    } else if refused > 0 && computed == 0 && partial == 0 {
        "refused".to_string()
    } else if partial > 0 && computed == 0 && refused == 0 {
        "partial".to_string()
    } else {
        format!("partial ({computed} computed, {partial} partial, {refused} refused)")
    }
}

fn render_insights_tab(frame: &mut Frame, area: Rect, state: &mut AppState, theme: &Theme) {
    let (title, lines) = if let Some(error) = state
        .insights_error
        .as_deref()
        .filter(|_| state.insights.is_none())
    {
        (
            " Insights unavailable ",
            vec![
                domain_legend_line(theme),
                Line::from(Span::styled(
                    "error: insight store did not return results",
                    theme.breach(),
                )),
                Line::from(sanitize_tui_str(error)),
                Line::from(Span::styled(
                    "Refused, partial, and unknown stay unmarked. They are not zero.",
                    theme.unavailable(),
                )),
            ],
        )
    } else if state.insights.is_none() {
        (
            " Insights ",
            vec![
                domain_legend_line(theme),
                Line::from(Span::styled("Insights not loaded yet.", theme.muted())),
            ],
        )
    } else {
        let title = match state.insight_depth {
            InsightDepth::Domain => " Insights · domains (L0)  [Enter/Right] open ",
            InsightDepth::Insight => " Insights · per insight (L1)  [Enter/Right] measures ",
            InsightDepth::Measure => " Insights · measures (L2)  [Enter] JSON ",
        };
        (title, insight_rows(state, theme))
    };

    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .title(title)
                    .borders(Borders::ALL)
                    .border_style(theme.border(true)),
            )
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn domain_legend_line(theme: &Theme) -> Line<'static> {
    Line::from(Span::styled(
        format!(" {DOMAIN_LEGEND}    {ADAPTER_LENS}"),
        theme.muted(),
    ))
}

fn insight_rows(state: &mut AppState, theme: &Theme) -> Vec<Line<'static>> {
    let domains = super::insight_domains();
    let mut lines = vec![
        domain_legend_line(theme),
        Line::from(Span::styled(
            "id      name                      domain          status                         population          measures",
            theme.muted(),
        )),
    ];
    match state.insight_depth {
        InsightDepth::Domain => {
            let len = domains.len();
            if len > 0 && state.insight_cursor >= len {
                state.insight_cursor = len - 1;
            }
            for (index, domain) in domains.iter().copied().enumerate() {
                let rows = state.domain_results(domain);
                let count = if rows.is_empty() {
                    "—".to_string()
                } else {
                    format!("{} insights", rows.len())
                };
                lines.push(insight_line(
                    index == state.insight_cursor,
                    format!(
                        "{:<8}{:<26}{:<16}{:<31}{:<20}—",
                        "—",
                        "—",
                        domain_name(domain),
                        domain_status_label(&rows),
                        count
                    ),
                    &domain_status_label(&rows),
                    theme,
                ));
            }
            lines.push(Line::from(Span::styled(
                "[Enter/Right] per-insight   [Left] back   population is not summed",
                theme.muted(),
            )));
        }
        InsightDepth::Insight => {
            let domain = state.insight_domain.unwrap_or(InsightDomain::Model);
            let rows = state.domain_results(domain);
            if !rows.is_empty() && state.insight_cursor >= rows.len() {
                state.insight_cursor = rows.len() - 1;
            }
            if rows.is_empty() {
                lines.push(Line::from("no insights in this domain"));
            }
            for (index, result) in rows.iter().enumerate() {
                lines.push(insight_line(
                    index == state.insight_cursor,
                    format!(
                        "{:<8}{:<26}{:<16}{:<31}{:<20}{}",
                        sanitize_tui_str(result.id),
                        sanitize_tui_str(result.name),
                        domain_name(result.domain),
                        status_label(&result.status),
                        population_label(result),
                        measures_label(result)
                    ),
                    &status_label(&result.status),
                    theme,
                ));
            }
            lines.push(Line::from(Span::styled(
                "[Enter/Right] measures   [Left] domains",
                theme.muted(),
            )));
        }
        InsightDepth::Measure => {
            let Some(result) = state.selected_insight() else {
                lines.push(Line::from("no insight selected"));
                return lines;
            };
            lines.push(insight_line(
                false,
                format!(
                    "{:<8}{:<26}{:<16}{:<31}{:<20}{}",
                    sanitize_tui_str(result.id),
                    sanitize_tui_str(result.name),
                    domain_name(result.domain),
                    status_label(&result.status),
                    population_label(&result),
                    "—"
                ),
                &status_label(&result.status),
                theme,
            ));
            if result.measures.is_empty() {
                lines.push(Line::from(
                    "no measures — refused, partial, and unknown are not zero",
                ));
            } else if state.insight_cursor >= result.measures.len() {
                state.insight_cursor = result.measures.len() - 1;
            }
            for (index, measure) in result.measures.iter().enumerate() {
                let value = format!(
                    "{} {}",
                    fmt_measure_value(measure.value),
                    sanitize_tui_str(&measure.unit)
                );
                lines.push(insight_line(
                    index == state.insight_cursor,
                    format!(
                        "{:<8}{:<26}{:<16}{:<31}{:<20}{value}",
                        sanitize_tui_str(&measure.id),
                        sanitize_tui_str(&measure.name),
                        domain_name(result.domain),
                        "—",
                        "—",
                    ),
                    &fmt_measure_value(measure.value),
                    theme,
                ));
            }
            lines.push(Line::from(Span::styled(
                "[Enter] JSON detail   [Left] per-insight",
                theme.muted(),
            )));
        }
    }
    lines
}

fn insight_line(selected: bool, text: String, status: &str, theme: &Theme) -> Line<'static> {
    let style = if selected {
        Style::default().add_modifier(Modifier::REVERSED)
    } else if status.starts_with("computed") {
        Style::default()
    } else if status.starts_with("partial") {
        theme.warn()
    } else {
        theme.unavailable()
    };
    let marker = if selected { "› " } else { "  " };
    Line::from(Span::styled(format!("{marker}{text}"), style))
}

fn render_footer(frame: &mut Frame, area: Rect, state: &AppState, theme: &Theme) {
    let ask_preview = if let Some((q, ans_summary)) = &state.last_nao_answer {
        format!(
            " 💬 Nao: \"{}\" → {}",
            sanitize_tui_str(q),
            sanitize_tui_str(ans_summary)
        )
    } else {
        " 💬 Press [:] or [a] to ask Nao a question in plain English (with live intent autocomplete)".into()
    };
    let lines = vec![
        Line::from(Span::styled(ask_preview, theme.accent())),
        Line::from(Span::styled(
            " [Tab/1-7] Screens  [↑/↓/j/k] Navigate  [/] Filter  [r] Rig  [w] Window  [i] Issue Draft  [?] Help  [q] Quit",
            theme.muted(),
        )),
    ];
    frame.render_widget(Paragraph::new(lines), area);
}

fn render_modal(frame: &mut Frame, area: Rect, state: &AppState, modal: &AppModal, theme: &Theme) {
    let popup = centered_rect(76, 70, area);
    frame.render_widget(Clear, popup);

    match modal {
        AppModal::Help => {
            let lines = vec![
                Line::from(Span::styled(
                    "Dendro Interactive Cockpit — Keyboard & Epistemic Guide",
                    theme.accent(),
                )),
                Line::from(""),
                Line::from("  [1] - [7] / [Tab]   Switch screens. [7] Insights: Enter/Right deepens, Left backs out"),
                Line::from("  Insights levels:    L0 domain, L1 insight, L2 measures, L3 JSON detail"),
                Line::from("  Insight domains:    Model, Harness, Coordination, Environment, Security"),
                Line::from("  [r] Rig             OpenRig adapter scope (<role>@<rig>), not an insight domain"),
                Line::from("  [↑/↓] or [j/k]      Navigate rows in tables, spotlights, or simulator seats"),
                Line::from("  [/]                 Live fuzzy filter on canonical seats"),
                Line::from("  [x]                 Toggle hidden qitem-* queue endpoints & system gates"),
                Line::from("  [m]                 Toggle 5-Band Aggregate Ribbon vs Shared-Axis Timeline"),
                Line::from("  [g]                 Toggle per-seat rows and team/rig aggregation"),
                Line::from("  [s]                 Jump from Seats to the Topology Simulator"),
                Line::from("  [r] / [w]           Open Rig selector / Smart Window (1h, 24h, 7d, 30d, all)"),
                Line::from("  [:] or [a]          Open Nao Semantic Query with autocomplete"),
                Line::from("  [i]                 Preview redacted GitHub Issue Draft modal"),
                Line::from("  [Esc]               Close active modal / return to screen"),
                Line::from("  [q] / [Ctrl-C]      Exit cleanly"),
                Line::from(""),
                Line::from(Span::styled("Activity Band Legend & Epistemic Rules:", theme.accent())),
                Line::from("  █ RUNNING  ▓ IDLE-WITH-WORK (▨ Container Hold)  ░ IDLE-NO-WORK  ▒ NEEDS-INPUT  ? UNKNOWN"),
                Line::from("  • Unavailable / — means insufficient data, NEVER measured zero (0 / 0.0%)."),
                Line::from("  • Handoff counts and topology simulations are observational; Fleet Governance stays with you."),
            ];
            frame.render_widget(
                Paragraph::new(lines)
                    .block(
                        Block::default()
                            .title(" [?] Dendro Help (Press Esc to close) ")
                            .borders(Borders::ALL)
                            .border_style(theme.border(true))
                            .style(theme.bg_base()),
                    )
                    .wrap(Wrap { trim: false }),
                popup,
            );
        }
        AppModal::NaoAsk => {
            let suggestions = dendro_core::semantic::suggest_closest_intents(&state.nao_input);
            let selected = state
                .nao_selected_suggestion
                .min(suggestions.len().saturating_sub(1));
            let mut lines = vec![
                Line::from(vec![
                    Span::styled("Ask Nao > ", theme.accent()),
                    Span::styled(
                        sanitize_tui_str(&state.nao_input),
                        theme.accent().add_modifier(Modifier::BOLD),
                    ),
                    Span::styled("█", theme.accent()),
                ]),
                Line::from(Span::styled(
                    "Press [Enter] to query · [Tab] fill selected suggestion · [↑/↓] choose · [Esc] close",
                    theme.muted(),
                )),
            ];
            if state.nao_loading {
                let spinner = BRAILLE_SPINNER[(state.tick_count as usize) % BRAILLE_SPINNER.len()];
                lines.push(Line::from(Span::styled(
                    format!(" {spinner} Querying conversational fleet intelligence..."),
                    theme.warn().add_modifier(Modifier::BOLD),
                )));
            }
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "Suggested Supported Intents:",
                theme.accent(),
            )));
            for (idx, item) in suggestions.iter().enumerate() {
                let name = item["intent"].as_str().unwrap_or("Intent");
                let ex = item["examples"]
                    .as_array()
                    .and_then(|a| a.first())
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let marker = if idx == selected { "›" } else { " " };
                let style = if idx == selected {
                    theme.accent().add_modifier(Modifier::REVERSED)
                } else {
                    Style::default()
                };
                lines.push(Line::from(Span::styled(
                    format!(" {marker} {name}: \"{ex}\""),
                    style,
                )));
            }
            if !state.nao_loading {
                if let Some((_, full_text)) = &state.nao_modal_output {
                    lines.push(Line::from(""));
                    for l in full_text.lines().take(12) {
                        lines.push(Line::from(sanitize_tui_str(l)));
                    }
                }
            }
            frame.render_widget(
                Paragraph::new(lines)
                    .block(
                        Block::default()
                            .title(" 💬 Nao Semantic Query — Conversational Fleet Intelligence ")
                            .borders(Borders::ALL)
                            .border_style(theme.border(true))
                            .style(theme.bg_base()),
                    )
                    .wrap(Wrap { trim: false }),
                popup,
            );
        }
        AppModal::IssueDraft(draft_md) => {
            let lines: Vec<Line> = sanitize_tui_str(draft_md)
                .lines()
                .map(|l| Line::from(l.to_owned()))
                .collect();
            frame.render_widget(
                Paragraph::new(lines)
                    .block(
                        Block::default()
                            .title(" [i] Redacted GitHub Issue Draft Preview (Esc to close) ")
                            .borders(Borders::ALL)
                            .border_style(theme.border(true))
                            .style(theme.bg_base()),
                    )
                    .wrap(Wrap { trim: false }),
                popup,
            );
        }
        AppModal::DetailJson(title, body) => {
            let lines: Vec<Line> = sanitize_tui_str(body)
                .lines()
                .take(popup.height as usize)
                .map(|l| Line::from(l.to_owned()))
                .collect();
            frame.render_widget(
                Paragraph::new(lines)
                    .block(
                        Block::default()
                            .title(format!(" {} (Esc to close) ", sanitize_tui_str(title)))
                            .borders(Borders::ALL)
                            .border_style(theme.border(true))
                            .style(theme.bg_base()),
                    )
                    .alignment(Alignment::Left)
                    .wrap(Wrap { trim: false }),
                popup,
            );
        }
    }
}
