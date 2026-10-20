//! Screen-by-screen rendering for the 6 TUI tabs and 4 Porter-Duff RGBA translucent modals.
use crate::tui::{
    compositor::{apply_rgba_backdrop, sanitize_tui_str, Rgba, Theme},
    AppModal, AppState, HeatmapMetric, SeatViewMode, TabId,
};
use dendro_core::presentation::{classify_entity_id, EntityKind};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, Tabs, Wrap},
    Frame,
};
use serde_json::Value;

const BRAILLE_SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

fn rows(v: &Value) -> impl Iterator<Item = &Value> {
    v.as_array().into_iter().flatten()
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
            Constraint::Length(1),
            Constraint::Length(2),
            Constraint::Min(10),
            Constraint::Length(2),
        ])
        .split(area);

    render_header(frame, chunks[0], state, theme);
    render_tabs(frame, chunks[1], state, theme);

    if state.report.is_none() {
        render_skeleton_loading(frame, chunks[2], state, theme);
    } else {
        match state.active_tab {
            TabId::Pulse => render_pulse_tab(frame, chunks[2], state, theme),
            TabId::Seats => render_seats_tab(frame, chunks[2], state, theme),
            TabId::Handoffs => render_handoffs_tab(frame, chunks[2], state, theme),
            TabId::Tokens => render_tokens_tab(frame, chunks[2], state, theme),
            TabId::Simulate => render_simulate_tab(frame, chunks[2], state, theme),
            TabId::Trace => render_trace_tab(frame, chunks[2], state, theme),
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
        Span::styled(" DENDRO v0.7 ", theme.accent()),
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
    frame.render_widget(Paragraph::new(line), area);
}

fn render_tabs(frame: &mut Frame, area: Rect, state: &AppState, theme: &Theme) {
    let titles = [
        "[1] Pulse & Overview",
        "[2] Seats & Timeline",
        "[3] Handoffs & Flow",
        "[4] Tokens & Cache",
        "[5] Topology Simulator",
        "[6] Trace & Spotlights",
    ];
    let tabs = Tabs::new(titles.iter().copied().map(Line::from).collect::<Vec<_>>())
        .select(state.active_tab.index())
        .block(
            Block::default()
                .borders(Borders::BOTTOM)
                .border_style(theme.border(false)),
        )
        .style(theme.muted())
        .highlight_style(theme.accent().add_modifier(Modifier::UNDERLINED));
    frame.render_widget(tabs, area);
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

    let kpis = [
        (
            " M1 · Yield / Item ",
            fmt_num(&p["m1_yield"]),
            "tokens / nonreturned item",
        ),
        (
            " M2 · Return Rate ",
            fmt_pct(&p["m2_return_rate"]),
            "returned queue handoffs",
        ),
        (
            " M3 · Rework % ",
            fmt_pct(&p["m3_rework_proportion"]),
            &format!("chains: {}", fmt_num(&p["m3_rework_chains"])),
        ),
        (
            " Recorded Tokens ",
            fmt_num(&p["tokens"]),
            "half-open [from, to)",
        ),
        (
            " Eff. Parallelism ",
            fmt_num(&p["fleet_concurrency"]["effective_parallelism"]),
            &format!(
                "M4: {}",
                sanitize_tui_str(p["m4_impact"].as_str().unwrap_or("insufficient_data"))
            ),
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
                " ● {} Canonical Agent Seats (<role>@<rig>)",
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
    let filtered = state.filtered_activity_rows();
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
    let filter_badge = if state.filter_input_mode || !state.seat_filter.is_empty() {
        format!(" · Filter [/]: '{}' ", sanitize_tui_str(&state.seat_filter))
    } else {
        " · [/] Filter  [m] Toggle Timeline  [x] Toggle qitem/non-seats ".into()
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
        Row::new(vec!["Seat", mode_name, "Observed"])
            .style(theme.accent().add_modifier(Modifier::BOLD)),
    )
    .block(
        Block::default()
            .title(format!(
                " Seats ({}/{total_raw} shown, {hidden} hidden){filter_badge}",
                filtered.len()
            ))
            .borders(Borders::ALL)
            .border_style(theme.border(true)),
    );
    state.seat_table_state.select(Some(state.selected_seat));
    frame.render_stateful_widget(table, chunks[0], &mut state.seat_table_state);

    // Right-hand Seat Inspector
    let mut insp_lines = Vec::new();
    if let Some(r) = filtered.get(state.selected_seat) {
        let seat_name = sanitize_tui_str(r["seat"].as_str().unwrap_or("unknown"));
        insp_lines.push(Line::from(vec![
            Span::styled("Seat: ", theme.muted()),
            Span::styled(seat_name.clone(), theme.accent()),
        ]));
        insp_lines.push(Line::from(format!(
            "Observed Duration: {} s",
            fmt_num(&r["observed_secs"])
        )));
        insp_lines.push(Line::from(""));
        insp_lines.push(Line::from(vec![
            Span::styled(" █ RUNNING            ", theme.band_running()),
            Span::raw(format!("{} s", fmt_num(&r["active_secs"]))),
        ]));
        insp_lines.push(Line::from(vec![
            Span::styled(" ▓ IDLE-WITH-WORK     ", theme.band_idle_with_work()),
            Span::raw(format!("{} s", fmt_num(&r["idle_with_work_secs"]))),
        ]));
        insp_lines.push(Line::from(vec![
            Span::styled("   ├─ Container Hold  ", theme.band_container_hold()),
            Span::raw(format!("{} s ▨", fmt_num(&r["unparked_container_secs"]))),
        ]));
        insp_lines.push(Line::from(vec![
            Span::styled("   └─ True Leaf Stall ", theme.warn()),
            Span::raw(format!("{} s", fmt_num(&r["leaf_stall_secs"]))),
        ]));
        insp_lines.push(Line::from(vec![
            Span::styled(" ░ IDLE-WITHOUT-WORK  ", theme.band_idle_no_work()),
            Span::raw(format!("{} s", fmt_num(&r["idle_without_work_secs"]))),
        ]));
        insp_lines.push(Line::from(vec![
            Span::styled(" ▒ NEEDS-INPUT        ", theme.band_needs_input()),
            Span::raw(format!("{} s", fmt_num(&r["waiting_human_secs"]))),
        ]));
        insp_lines.push(Line::from(vec![
            Span::styled(" ? UNKNOWN            ", theme.band_unknown()),
            Span::raw(format!("{} s", fmt_num(&r["unknown_secs"]))),
        ]));
        insp_lines.push(Line::from(""));
        insp_lines.push(Line::from(format!(
            "Tool Calls: {} eng (█) / {} ceremony (░) · ratio={}",
            fmt_num(&r["engineering_calls"]),
            fmt_num(&r["ceremony_calls"]),
            fmt_pct(&r["ceremony_ratio"])
        )));
        if let Some(rep) = &state.report {
            if let Some(m1) = rows(&rep["payload"]["m1_by_seat"])
                .find(|m| m["seat"].as_str() == r["seat"].as_str())
            {
                insp_lines.push(Line::from(format!(
                    "M1 Yield: {} tok/item · Closed: {} · Tokens: {}",
                    fmt_num(&m1["tokens_per_closed_item"]),
                    fmt_num(&m1["closed_items"]),
                    fmt_num(&m1["tokens"])
                )));
            }
        }
        insp_lines.push(Line::from(""));
        insp_lines.push(Line::from(Span::styled(
            "[Enter] Inspect JSON  [s] Send to Simulator  [t] Trace",
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
                    .title(" Synchronized Seat Inspector ")
                    .borders(Borders::ALL)
                    .border_style(theme.border(false)),
            )
            .wrap(Wrap { trim: false }),
        chunks[1],
    );
}

fn build_colored_band_line(r: &Value, width: usize, theme: &Theme) -> Line<'static> {
    let total = r["observed_secs"].as_f64().unwrap_or(0.0);
    if total <= 0.0 {
        return Line::from(Span::styled("?".repeat(width), theme.band_unknown()));
    }
    let spec: [(&str, char, Style); 5] = [
        ("active_secs", '█', theme.band_running()),
        ("idle_with_work_secs", '▓', theme.band_idle_with_work()),
        ("idle_without_work_secs", '░', theme.band_idle_no_work()),
        ("waiting_human_secs", '▒', theme.band_needs_input()),
        ("unknown_secs", '?', theme.band_unknown()),
    ];
    let mut spans = Vec::new();
    let mut used = 0usize;
    for (k, ch, st) in spec {
        let v = r[k].as_f64().unwrap_or(0.0);
        if v > 0.0 {
            let n = ((v / total * width as f64).round() as usize).min(width.saturating_sub(used));
            if n > 0 {
                used += n;
                spans.push(Span::styled(ch.to_string().repeat(n), st));
            }
        }
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

fn render_tokens_tab(frame: &mut Frame, area: Rect, state: &AppState, theme: &Theme) {
    let Some(report) = &state.report else { return };
    let p = &report["payload"];

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(area);

    let mut seat_rows = Vec::new();
    for r in rows(&p["m1_by_seat"]).take(30) {
        let seat = sanitize_tui_str(r["seat"].as_str().unwrap_or("?"));
        if !state.show_all_entities && classify_entity_id(&seat) != EntityKind::AgentSeat {
            continue;
        }
        seat_rows.push(Row::new(vec![
            Cell::from(seat),
            Cell::from(fmt_num(&r["tokens"])),
            Cell::from(fmt_num(&r["closed_items"])),
            Cell::from(fmt_num(&r["nonreturn_deliveries"])),
            Cell::from(fmt_num(&r["tokens_per_closed_item"])),
        ]));
    }

    let table = Table::new(
        seat_rows,
        [
            Constraint::Length(26),
            Constraint::Length(14),
            Constraint::Length(12),
            Constraint::Length(12),
            Constraint::Length(18),
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
            .title(" Per-Seat Token Consumption & M1 Yield ")
            .borders(Borders::ALL)
            .border_style(theme.border(true)),
    );
    frame.render_widget(table, chunks[0]);

    let tc = &p["token_categories"];
    let ti = &p["task_intake_overhead"];
    let right_lines = vec![
        Line::from(Span::styled("Token Category Mix (AC6)", theme.accent())),
        Line::from(format!(
            " █ Input Uncached:  {}",
            fmt_num(&tc["input_uncached"])
        )),
        Line::from(format!(
            " ▓ Cache Read:      {}",
            fmt_num(&tc["cache_read"])
        )),
        Line::from(format!(
            " ▒ Cache Write:     {}",
            fmt_num(&tc["cache_write"])
        )),
        Line::from(format!(" ░ Output:          {}", fmt_num(&tc["output"]))),
        Line::from(Span::styled(
            " (Reasoning is part of output; never double-counted)",
            theme.muted(),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Task-Intake Classification (Analytical)",
            theme.accent(),
        )),
        Line::from(format!(
            " ● Assigned Mission:  {}",
            fmt_num(&ti["assigned_mission_tokens"])
        )),
        Line::from(format!(
            " ● Intake Overhead:   {}",
            fmt_num(&ti["task_intake_overhead"])
        )),
        Line::from(format!(
            " ▲ Ambiguous/Collide: {}",
            fmt_num(&ti["ambiguous_tokens"])
        )),
        Line::from(format!(
            " ○ Unlinked Session:  {}",
            fmt_num(&ti["unlinked_session"])
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Cross-Slice Cache Bleed & Compactions",
            theme.accent(),
        )),
        Line::from(format!(
            " Context Compaction Events: {}",
            rows(&p["context_compactions"]["events"]).count()
        )),
    ];

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

fn render_simulate_tab(frame: &mut Frame, area: Rect, state: &AppState, theme: &Theme) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(48), Constraint::Percentage(52)])
        .split(area);

    let seats = state.canonical_seat_names();
    let mut left_lines = vec![
        Line::from(Span::styled(
            "Counterfactual Topology Controls (<1ms In-Memory)",
            theme.accent(),
        )),
        Line::from(Span::styled(
            "↑/↓ Select · [Space] Toggle Remove · [p] Set Replacement",
            theme.muted(),
        )),
        Line::from(""),
    ];

    for (idx, s) in seats.iter().take(12).enumerate() {
        let is_rem = state.sim_scenario.remove.contains(s);
        let is_rep = state.sim_scenario.replacement_seat.as_ref() == Some(s);
        let badge = if is_rem {
            "[REMOVE] "
        } else if is_rep {
            "[REPLACE]"
        } else {
            "[      ] "
        };
        let st = if idx == state.selected_seat {
            Style::default().add_modifier(Modifier::REVERSED)
        } else if is_rem {
            theme.breach()
        } else if is_rep {
            theme.band_running()
        } else {
            Style::default()
        };
        left_lines.push(Line::from(Span::styled(
            format!(" {badge} {}", sanitize_tui_str(s)),
            st,
        )));
    }
    left_lines.push(Line::from(""));
    left_lines.push(Line::from(format!(
        " [d] Direct Leaf Handoffs:         {}",
        if state.sim_scenario.direct_handoffs {
            "ON ●"
        } else {
            "OFF ○"
        }
    )));
    left_lines.push(Line::from(format!(
        " [f] Enforce 4-Eyes Model Family:  {}",
        if state.sim_scenario.enforce_model_family_diversity {
            "ON ●"
        } else {
            "OFF ○"
        }
    )));
    left_lines.push(Line::from(format!(
        " [←/→] Boundary Reset Threshold:   {} tokens",
        state
            .sim_scenario
            .boundary_reset_threshold
            .map(|n| n.to_string())
            .unwrap_or_else(|| "None".into())
    )));
    left_lines.push(Line::from(Span::styled(
        " [c] Clear Scenario Overrides",
        theme.muted(),
    )));

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
        right_lines.push(Line::from(vec![
            Span::styled("Simulation Verdict: ", theme.muted()),
            Span::styled(
                status.to_uppercase(),
                if status == "ok" {
                    theme.band_running()
                } else {
                    theme.warn()
                },
            ),
        ]));
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
            fmt_num(&sim["baseline"]["input_units"])
        )));
        right_lines.push(Line::from(format!(
            "Projected Input Units: {}",
            fmt_num(&sim["projection"]["input_units"])
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
        right_lines.push(Line::from(""));
        right_lines.push(Line::from(format!(
            "Token Conservation: conserved={} · unassigned_records={}",
            sim["conservation"]["conserved"].as_bool().unwrap_or(false),
            fmt_num(&sim["conservation"]["unassigned_work_records"])
        )));
        right_lines.push(Line::from(format!(
            "Direct Routing Cycle Delta: {} ms",
            fmt_num(&sim["routing"]["projected_cycle_reduction_ms"])
        )));
        right_lines.push(Line::from(""));
        right_lines.push(Line::from(Span::styled(
            format!(
                "Unit: {}",
                sanitize_tui_str(
                    sim["unit"]
                        .as_str()
                        .unwrap_or("uncached-equivalent input units (not currency)")
                )
            ),
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
            " [Tab/1-6] Screens  [↑/↓/j/k] Navigate  [/] Filter  [r] Rig  [w] Window  [i] Issue Draft  [?] Help  [q] Quit",
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
                Line::from("  [1] - [6] / [Tab]   Switch between the 6 analytical & simulation screens"),
                Line::from("  [↑/↓] or [j/k]      Navigate rows in tables, spotlights, or simulator seats"),
                Line::from("  [/]                 Live fuzzy filter on canonical seats"),
                Line::from("  [x]                 Toggle hidden qitem-* queue endpoints & system gates"),
                Line::from("  [m]                 Toggle 5-Band Aggregate Ribbon vs Shared-Axis Timeline"),
                Line::from("  [r] / [w]           Open Rig selector / Smart Window (1h, 24h, 7d, 30d, all)"),
                Line::from("  [:] or [a]          Open Inline Nao Semantic Drawer with autocomplete"),
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
            let mut lines = vec![
                Line::from(vec![
                    Span::styled("Ask Nao > ", theme.accent()),
                    Span::styled(sanitize_tui_str(&state.nao_input), theme.bg_base()),
                    Span::styled("█", theme.accent()),
                ]),
                Line::from(Span::styled(
                    "Press [Enter] to query · [Tab] autocomplete top suggestion · [Esc] close",
                    theme.muted(),
                )),
                Line::from(""),
                Line::from(Span::styled("Suggested Supported Intents:", theme.accent())),
            ];
            for item in suggestions.iter().take(4) {
                let name = item["intent"].as_str().unwrap_or("Intent");
                let ex = item["examples"]
                    .as_array()
                    .and_then(|a| a.first())
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                lines.push(Line::from(format!("  • {name}: \"{ex}\"")));
            }
            if let Some((_, full_text)) = &state.nao_modal_output {
                lines.push(Line::from(""));
                for l in full_text.lines().take(12) {
                    lines.push(Line::from(sanitize_tui_str(l)));
                }
            }
            frame.render_widget(
                Paragraph::new(lines)
                    .block(
                        Block::default()
                            .title(" 💬 Nao Semantic Query Bar (RGBA Overlay) ")
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
