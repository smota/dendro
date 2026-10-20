//! Modern, async, interactive TUI cockpit for Dendro built on `ratatui`, `crossterm` (`EventStream`),
//! `tachyonfx` shader animations, and an OpenTUI-inspired Porter-Duff RGBA compositor.
pub mod compositor;
pub mod views;

use anyhow::Result;
use compositor::Theme;
use crossterm::{
    event::{Event, EventStream, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use dendro_core::presentation::{classify_entity_id, EntityKind};
use futures::StreamExt;
use ratatui::{backend::CrosstermBackend, widgets::TableState, Terminal};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    io::{self, IsTerminal},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use tachyonfx::{fx, EffectManager, Interpolation};
use tokio::sync::mpsc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabId {
    Pulse,
    Seats,
    Handoffs,
    Tokens,
    Simulate,
    Trace,
}

impl TabId {
    pub fn index(self) -> usize {
        match self {
            Self::Pulse => 0,
            Self::Seats => 1,
            Self::Handoffs => 2,
            Self::Tokens => 3,
            Self::Simulate => 4,
            Self::Trace => 5,
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Pulse => Self::Seats,
            Self::Seats => Self::Handoffs,
            Self::Handoffs => Self::Tokens,
            Self::Tokens => Self::Simulate,
            Self::Simulate => Self::Trace,
            Self::Trace => Self::Pulse,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::Pulse => Self::Trace,
            Self::Seats => Self::Pulse,
            Self::Handoffs => Self::Seats,
            Self::Tokens => Self::Handoffs,
            Self::Simulate => Self::Tokens,
            Self::Trace => Self::Simulate,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeatViewMode {
    Bands,
    Timeline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeatmapMetric {
    ReturnRate,
    Count,
    WaitMedian,
    ExecMedian,
}

impl HeatmapMetric {
    pub fn next(self) -> Self {
        match self {
            Self::ReturnRate => Self::Count,
            Self::Count => Self::WaitMedian,
            Self::WaitMedian => Self::ExecMedian,
            Self::ExecMedian => Self::ReturnRate,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AppModal {
    Help,
    NaoAsk,
    IssueDraft(String),
    DetailJson(String, String),
}

pub enum WorkerEvent {
    StatusReady {
        status: Value,
        from: String,
        to: String,
        mode: String,
    },
    ReportReady(Arc<Value>),
    TraceReady(String, Value),
    NaoAnswerReady {
        question: String,
        summary: String,
        full_text: String,
    },
    WatcherPulse(usize),
}

pub struct AppState {
    pub db: PathBuf,
    pub rig: Option<String>,
    pub window: String,
    pub fixed_from: Option<String>,
    pub fixed_to: Option<String>,
    pub resolved_from: String,
    pub resolved_to: String,
    pub window_mode: Option<String>,
    pub active_tab: TabId,
    pub status: Option<Value>,
    pub report: Option<Arc<Value>>,
    pub traces: BTreeMap<String, Value>,
    pub active_trace_metric: String,
    pub trace_scroll: usize,
    pub loading_stage: Option<String>,
    pub tick_count: u64,
    pub watcher_events: usize,
    pub selected_seat: usize,
    pub selected_pair: usize,
    pub seat_table_state: TableState,
    pub seat_view_mode: SeatViewMode,
    pub heatmap_metric: HeatmapMetric,
    pub show_all_entities: bool,
    pub filter_input_mode: bool,
    pub seat_filter: String,
    pub modal: Option<AppModal>,
    pub nao_input: String,
    pub last_nao_answer: Option<(String, String)>,
    pub nao_modal_output: Option<(String, String)>,
    pub sim_scenario: dendro_core::simulation::Scenario,
    pub sim_result: Option<Value>,
}

impl AppState {
    pub fn new(
        db: PathBuf,
        rig: Option<String>,
        window: String,
        fixed_from: Option<String>,
        fixed_to: Option<String>,
    ) -> Self {
        Self {
            db,
            rig,
            window,
            fixed_from,
            fixed_to,
            resolved_from: String::new(),
            resolved_to: String::new(),
            window_mode: None,
            active_tab: TabId::Pulse,
            status: None,
            report: None,
            traces: BTreeMap::new(),
            active_trace_metric: "handoff_friction".into(),
            trace_scroll: 0,
            loading_stage: Some("[1/2] Inspecting database & resolving window...".into()),
            tick_count: 0,
            watcher_events: 0,
            selected_seat: 0,
            selected_pair: 0,
            seat_table_state: TableState::default(),
            seat_view_mode: SeatViewMode::Bands,
            heatmap_metric: HeatmapMetric::ReturnRate,
            show_all_entities: false,
            filter_input_mode: false,
            seat_filter: String::new(),
            modal: None,
            nao_input: String::new(),
            last_nao_answer: None,
            nao_modal_output: None,
            sim_scenario: dendro_core::simulation::Scenario::default(),
            sim_result: None,
        }
    }

    pub fn filtered_activity_rows(&self) -> Vec<Value> {
        let Some(rep) = &self.report else {
            return Vec::new();
        };
        let Some(arr) = rep["payload"]["activity"].as_array() else {
            return Vec::new();
        };
        let filter_lower = self.seat_filter.to_ascii_lowercase();
        let mut rows: Vec<Value> = arr
            .iter()
            .filter(|r| {
                let seat = r["seat"].as_str().unwrap_or("");
                if !self.show_all_entities && classify_entity_id(seat) != EntityKind::AgentSeat {
                    return false;
                }
                if !filter_lower.is_empty() && !seat.to_ascii_lowercase().contains(&filter_lower) {
                    return false;
                }
                true
            })
            .cloned()
            .collect();
        if rows.is_empty() && !self.show_all_entities && filter_lower.is_empty() {
            rows = arr.clone();
        }
        rows
    }

    pub fn canonical_seat_names(&self) -> Vec<String> {
        self.filtered_activity_rows()
            .into_iter()
            .filter_map(|r| r["seat"].as_str().map(str::to_owned))
            .collect()
    }

    pub fn recompute_simulation(&mut self) {
        if let Some(rep) = &self.report {
            if let Ok(res) = dendro_core::simulation::simulate(rep, &self.sim_scenario) {
                self.sim_result = Some(res);
            }
        }
    }
}

/// RAII Guard ensuring raw mode and alternate screen are always restored on exit or panic.
struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> Result<Self> {
        enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen)?;
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
    }
}

fn spawn_hydration(
    tx: mpsc::Sender<WorkerEvent>,
    db: PathBuf,
    rig: Option<String>,
    window: String,
    fixed_from: Option<String>,
    fixed_to: Option<String>,
) {
    tokio::task::spawn_blocking(move || {
        let status = dendro_core::presentation::database_status(&db, rig.as_deref())
            .unwrap_or_else(|_| serde_json::json!({"status": "unavailable"}));
        let (from, to, mode) = dendro_core::presentation::resolve_window(
            &db,
            rig.as_deref(),
            Some(window.as_str()),
            fixed_from.as_deref(),
            fixed_to.as_deref(),
        )
        .unwrap_or_else(|_| {
            (
                "2026-10-08T00:00:00Z".into(),
                "2026-10-11T00:00:00Z".into(),
                "fallback".into(),
            )
        });
        let _ = tx.blocking_send(WorkerEvent::StatusReady {
            status,
            from: from.clone(),
            to: to.clone(),
            mode: mode.clone(),
        });
        if let Ok(mut rep) = dendro_core::report_json(&db, "decisions", rig.as_deref(), &from, &to)
        {
            rep["window_resolution"] = serde_json::json!(mode);
            let _ = tx.blocking_send(WorkerEvent::ReportReady(Arc::new(rep)));
        }
    });
}

fn spawn_trace_fetch(
    tx: mpsc::Sender<WorkerEvent>,
    db: PathBuf,
    rig: Option<String>,
    from: String,
    to: String,
    metric_id: String,
) {
    if from.is_empty() || to.is_empty() {
        return;
    }
    tokio::task::spawn_blocking(move || {
        if let Ok(tr) = dendro_core::trace_json(&db, &metric_id, rig.as_deref(), &from, &to) {
            let _ = tx.blocking_send(WorkerEvent::TraceReady(metric_id, tr));
        }
    });
}

fn spawn_nao_query(
    tx: mpsc::Sender<WorkerEvent>,
    db: PathBuf,
    rig: Option<String>,
    from: String,
    to: String,
    question: String,
) {
    tokio::task::spawn_blocking(move || {
        let cached = dendro_core::semantic::cached_scope(&db);
        if cached.as_ref().is_none_or(|(_, _, _, fresh)| !*fresh)
            && !from.is_empty()
            && !to.is_empty()
        {
            let _ = dendro_core::semantic::refresh(&db, rig.as_deref(), &from, &to);
        }
        if let Ok(ans) = dendro_core::semantic::answer(&db, &question, None) {
            let full_text = dendro_core::presentation::human_ask_answer(&question, &ans);
            let summary = if ans["status"] == "ok" {
                if let Some(first_row) = ans["rows"].as_array().and_then(|a| a.first()) {
                    format!(
                        "{} (top ranked in scope)",
                        first_row["seat"].as_str().unwrap_or("ok")
                    )
                } else {
                    "ok (open [:] Nao drawer for full breakdown)".into()
                }
            } else {
                ans["message"]
                    .as_str()
                    .unwrap_or("insufficient_data")
                    .to_owned()
            };
            let _ = tx.blocking_send(WorkerEvent::NaoAnswerReady {
                question,
                summary,
                full_text,
            });
        }
    });
}

/// Entrypoint for `dendro dashboard` and `dendro ui`.
/// Preserves R1 deterministic text output when `once == true` or non-TTY;
/// launches the <50ms async `ratatui` + `tachyonfx` + RGBA compositor cockpit in interactive terminals.
pub async fn run_interactive_or_once(
    db: PathBuf,
    rig: Option<String>,
    window: Option<String>,
    from: Option<String>,
    to: Option<String>,
    once: bool,
    watch_paths: Vec<PathBuf>,
) -> Result<()> {
    let win_str = window.unwrap_or_else(|| "all".into());
    if once || !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        let (f, t, mode) = tokio::task::spawn_blocking({
            let db = db.clone();
            let rig = rig.clone();
            let win_str = win_str.clone();
            let from = from.clone();
            let to = to.clone();
            move || -> Result<(String, String, String)> {
                dendro_core::presentation::resolve_window(
                    &db,
                    rig.as_deref(),
                    Some(win_str.as_str()),
                    from.as_deref(),
                    to.as_deref(),
                )
            }
        })
        .await??;
        let report = tokio::task::spawn_blocking(move || -> Result<Value> {
            let mut r = dendro_core::report_json(&db, "decisions", rig.as_deref(), &f, &t)?;
            r["window_resolution"] = serde_json::json!(mode);
            Ok(r)
        })
        .await??;
        dendro_core::presentation::terminal_ui(&report, &[], true)?;
        return Ok(());
    }

    let _guard = TerminalGuard::enter()?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;
    let theme = Theme::detect();
    let mut state = AppState::new(db.clone(), rig.clone(), win_str.clone(), from, to);

    let mut effects: EffectManager<()> = EffectManager::default();
    if !theme.no_color {
        effects.add_effect(fx::fade_from_fg(
            ratatui::style::Color::Rgb(9, 13, 22),
            (320, Interpolation::QuadOut),
        ));
    }

    let (worker_tx, mut worker_rx) = mpsc::channel::<WorkerEvent>(32);
    spawn_hydration(
        worker_tx.clone(),
        state.db.clone(),
        state.rig.clone(),
        state.window.clone(),
        state.fixed_from.clone(),
        state.fixed_to.clone(),
    );

    // Wire DendroWatcher for live file-change pulse notifications (1.75s debounce per AGENTS.md)
    let (watch_tx, mut watch_rx) = mpsc::channel(16);
    let mut watcher = dendro_watcher::DendroWatcher::new(Duration::from_millis(1750));
    for p in watch_paths {
        watcher.add_path(p);
    }
    let _ = watcher.run(watch_tx).await;
    let watch_forward_tx = worker_tx.clone();
    tokio::spawn(async move {
        while let Some(evs) = watch_rx.recv().await {
            if watch_forward_tx
                .send(WorkerEvent::WatcherPulse(evs.len()))
                .await
                .is_err()
            {
                break;
            }
        }
    });

    let mut reader = EventStream::new();
    let mut tick = tokio::time::interval(Duration::from_millis(50));
    let mut last_frame = Instant::now();
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;

    loop {
        let elapsed = last_frame.elapsed();
        last_frame = Instant::now();

        terminal.draw(|frame| {
            let area = frame.area();
            views::render_app(frame, &mut state, &theme);
            if !theme.no_color && effects.is_running() {
                effects.process_effects(elapsed.into(), frame.buffer_mut(), area);
            }
        })?;

        tokio::select! {
            _ = interrupt.recv() => break,
            _ = terminate.recv() => break,
            _ = tick.tick() => {
                state.tick_count = state.tick_count.wrapping_add(1);
            }
            Some(ev) = worker_rx.recv() => {
                match ev {
                    WorkerEvent::StatusReady { status, from, to, mode } => {
                        state.status = Some(status);
                        state.resolved_from = from;
                        state.resolved_to = to;
                        state.window_mode = Some(mode);
                        state.loading_stage = Some("[2/2] Evaluating canonical decision metrics...".into());
                    }
                    WorkerEvent::ReportReady(rep) => {
                        state.report = Some(rep);
                        state.loading_stage = None;
                        state.recompute_simulation();
                        if !theme.no_color {
                            effects.add_effect(fx::coalesce((250, Interpolation::SineOut)));
                        }
                    }
                    WorkerEvent::TraceReady(metric_id, tr) => {
                        state.traces.insert(metric_id, tr);
                    }
                    WorkerEvent::NaoAnswerReady { question, summary, full_text } => {
                        state.last_nao_answer = Some((question.clone(), summary));
                        state.nao_modal_output = Some((question, full_text));
                    }
                    WorkerEvent::WatcherPulse(n) => {
                        state.watcher_events = state.watcher_events.saturating_add(n);
                    }
                }
            }
            Some(Ok(crossterm_ev)) = reader.next() => {
                if let Event::Key(key) = crossterm_ev {
                    if key.kind == KeyEventKind::Release {
                        continue;
                    }
                    if handle_key_event(&mut state, key, &worker_tx, &mut effects, &theme) {
                        break;
                    }
                }
            }
        }
    }

    Ok(())
}

fn handle_key_event(
    state: &mut AppState,
    key: crossterm::event::KeyEvent,
    worker_tx: &mpsc::Sender<WorkerEvent>,
    effects: &mut EffectManager<()>,
    theme: &Theme,
) -> bool {
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return true;
    }

    // Modal handling first
    if let Some(modal) = state.modal.clone() {
        match modal {
            AppModal::NaoAsk => match key.code {
                KeyCode::Esc => state.modal = None,
                KeyCode::Backspace => {
                    state.nao_input.pop();
                }
                KeyCode::Tab => {
                    let sugg = dendro_core::semantic::suggest_closest_intents(&state.nao_input);
                    if let Some(ex) = sugg
                        .first()
                        .and_then(|s| s["examples"].as_array())
                        .and_then(|a| a.first())
                        .and_then(|v| v.as_str())
                    {
                        state.nao_input = ex.to_owned();
                    }
                }
                KeyCode::Enter => {
                    let q = state.nao_input.trim().to_owned();
                    if !q.is_empty() {
                        spawn_nao_query(
                            worker_tx.clone(),
                            state.db.clone(),
                            state.rig.clone(),
                            state.resolved_from.clone(),
                            state.resolved_to.clone(),
                            q,
                        );
                    }
                }
                KeyCode::Char(c) => state.nao_input.push(c),
                _ => {}
            },
            _ => {
                if matches!(
                    key.code,
                    KeyCode::Esc | KeyCode::Char('q') | KeyCode::Tab | KeyCode::Enter
                ) {
                    state.modal = None;
                }
            }
        }
        return false;
    }

    // Seat filter input mode
    if state.filter_input_mode {
        match key.code {
            KeyCode::Esc | KeyCode::Enter => state.filter_input_mode = false,
            KeyCode::Backspace => {
                state.seat_filter.pop();
                state.selected_seat = 0;
            }
            KeyCode::Char(c) => {
                state.seat_filter.push(c);
                state.selected_seat = 0;
            }
            _ => {}
        }
        return false;
    }

    let trigger_transition = |effects: &mut EffectManager<()>| {
        if !theme.no_color {
            effects.add_effect(fx::fade_from_fg(
                ratatui::style::Color::Rgb(16, 27, 43),
                (180, Interpolation::QuadOut),
            ));
        }
    };

    match key.code {
        KeyCode::Char('q') => return true,
        KeyCode::Char('?') | KeyCode::Char('h') => {
            state.modal = Some(AppModal::Help);
            trigger_transition(effects);
        }
        KeyCode::Char(':') | KeyCode::Char('a') => {
            state.modal = Some(AppModal::NaoAsk);
            trigger_transition(effects);
        }
        KeyCode::Char('/') => {
            state.active_tab = TabId::Seats;
            state.filter_input_mode = true;
        }
        KeyCode::Tab | KeyCode::Right if state.active_tab != TabId::Simulate => {
            state.active_tab = state.active_tab.next();
            if state.active_tab == TabId::Trace
                && !state.traces.contains_key(&state.active_trace_metric)
            {
                spawn_trace_fetch(
                    worker_tx.clone(),
                    state.db.clone(),
                    state.rig.clone(),
                    state.resolved_from.clone(),
                    state.resolved_to.clone(),
                    state.active_trace_metric.clone(),
                );
            }
            trigger_transition(effects);
        }
        KeyCode::BackTab => {
            state.active_tab = state.active_tab.prev();
            trigger_transition(effects);
        }
        KeyCode::Char('1') if state.active_tab != TabId::Trace => {
            state.active_tab = TabId::Pulse;
            trigger_transition(effects);
        }
        KeyCode::Char('2') if state.active_tab != TabId::Trace => {
            state.active_tab = TabId::Seats;
            trigger_transition(effects);
        }
        KeyCode::Char('3') if state.active_tab != TabId::Trace => {
            state.active_tab = TabId::Handoffs;
            trigger_transition(effects);
        }
        KeyCode::Char('4') => {
            state.active_tab = TabId::Tokens;
            trigger_transition(effects);
        }
        KeyCode::Char('5') => {
            state.active_tab = TabId::Simulate;
            state.recompute_simulation();
            trigger_transition(effects);
        }
        KeyCode::Char('6') | KeyCode::Char('t') => {
            state.active_tab = TabId::Trace;
            if !state.traces.contains_key(&state.active_trace_metric) {
                spawn_trace_fetch(
                    worker_tx.clone(),
                    state.db.clone(),
                    state.rig.clone(),
                    state.resolved_from.clone(),
                    state.resolved_to.clone(),
                    state.active_trace_metric.clone(),
                );
            }
            trigger_transition(effects);
        }
        KeyCode::Char(c @ ('1' | '2' | '3')) if state.active_tab == TabId::Trace => {
            let m = match c {
                '1' => "handoff_friction",
                '2' => "activity",
                _ => "tokens",
            };
            state.active_trace_metric = m.into();
            state.trace_scroll = 0;
            if !state.traces.contains_key(m) {
                spawn_trace_fetch(
                    worker_tx.clone(),
                    state.db.clone(),
                    state.rig.clone(),
                    state.resolved_from.clone(),
                    state.resolved_to.clone(),
                    m.into(),
                );
            }
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if state.active_tab == TabId::Trace {
                state.trace_scroll = state.trace_scroll.saturating_sub(3);
            } else if state.active_tab == TabId::Handoffs {
                state.selected_pair = state.selected_pair.saturating_sub(1);
            } else {
                state.selected_seat = state.selected_seat.saturating_sub(1);
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if state.active_tab == TabId::Trace {
                state.trace_scroll = state.trace_scroll.saturating_add(3);
            } else if state.active_tab == TabId::Handoffs {
                state.selected_pair = state.selected_pair.saturating_add(1);
            } else {
                let max = state.filtered_activity_rows().len().saturating_sub(1);
                state.selected_seat = (state.selected_seat + 1).min(max);
            }
        }
        KeyCode::Char('x') => {
            state.show_all_entities = !state.show_all_entities;
            state.selected_seat = 0;
        }
        KeyCode::Char('m') if state.active_tab == TabId::Seats => {
            state.seat_view_mode = match state.seat_view_mode {
                SeatViewMode::Bands => SeatViewMode::Timeline,
                SeatViewMode::Timeline => SeatViewMode::Bands,
            };
        }
        KeyCode::Char('v') if state.active_tab == TabId::Handoffs => {
            state.heatmap_metric = state.heatmap_metric.next();
        }
        KeyCode::Char('w') => {
            let next_win = match state.window.as_str() {
                "1h" => "24h",
                "24h" => "7d",
                "7d" => "30d",
                "30d" => "all",
                _ => "1h",
            };
            state.window = next_win.into();
            state.fixed_from = None;
            state.fixed_to = None;
            state.loading_stage = Some(format!("Switching window to {next_win}..."));
            spawn_hydration(
                worker_tx.clone(),
                state.db.clone(),
                state.rig.clone(),
                state.window.clone(),
                None,
                None,
            );
        }
        KeyCode::Char('r') => {
            let named_rigs: Vec<String> = state
                .status
                .as_ref()
                .and_then(|s| s["entity_breakdown"]["named_rigs"].as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default();
            if !named_rigs.is_empty() {
                state.rig = match &state.rig {
                    None => Some(named_rigs[0].clone()),
                    Some(cur) => {
                        let idx = named_rigs.iter().position(|r| r == cur);
                        match idx {
                            Some(i) if i + 1 < named_rigs.len() => Some(named_rigs[i + 1].clone()),
                            _ => None,
                        }
                    }
                };
                state.loading_stage = Some("Switching rig scope...".into());
                spawn_hydration(
                    worker_tx.clone(),
                    state.db.clone(),
                    state.rig.clone(),
                    state.window.clone(),
                    state.fixed_from.clone(),
                    state.fixed_to.clone(),
                );
            }
        }
        KeyCode::Char('i') => {
            if let Some(rep) = &state.report {
                let draft = dendro_core::spotlight::issue_draft(
                    &rep["payload"],
                    "anomaly_spotlights",
                    None,
                )
                .or_else(|_| dendro_core::spotlight::issue_draft(&rep["payload"], "m3", None))
                .unwrap_or_else(|e| format!("Issue draft note: {e}"));
                state.modal = Some(AppModal::IssueDraft(draft));
                trigger_transition(effects);
            }
        }
        KeyCode::Enter if state.active_tab == TabId::Seats => {
            if let Some(row) = state.filtered_activity_rows().get(state.selected_seat) {
                let seat = row["seat"].as_str().unwrap_or("Seat").to_owned();
                let pretty = serde_json::to_string_pretty(row).unwrap_or_default();
                state.modal = Some(AppModal::DetailJson(
                    format!("Seat Evidence: {seat}"),
                    pretty,
                ));
            }
        }
        // Simulator controls on Tab 5
        KeyCode::Char(' ') if state.active_tab == TabId::Simulate => {
            if let Some(seat) = state
                .canonical_seat_names()
                .get(state.selected_seat)
                .cloned()
            {
                if let Some(pos) = state.sim_scenario.remove.iter().position(|s| s == &seat) {
                    state.sim_scenario.remove.remove(pos);
                } else {
                    state.sim_scenario.remove.push(seat);
                }
                state.recompute_simulation();
            }
        }
        KeyCode::Char('p') if state.active_tab == TabId::Simulate => {
            if let Some(seat) = state
                .canonical_seat_names()
                .get(state.selected_seat)
                .cloned()
            {
                if state.sim_scenario.replacement_seat.as_ref() == Some(&seat) {
                    state.sim_scenario.replacement_seat = None;
                } else {
                    state.sim_scenario.replacement_seat = Some(seat);
                }
                state.recompute_simulation();
            }
        }
        KeyCode::Char('d') if state.active_tab == TabId::Simulate => {
            state.sim_scenario.direct_handoffs = !state.sim_scenario.direct_handoffs;
            state.recompute_simulation();
        }
        KeyCode::Char('f') if state.active_tab == TabId::Simulate => {
            state.sim_scenario.enforce_model_family_diversity =
                !state.sim_scenario.enforce_model_family_diversity;
            state.recompute_simulation();
        }
        KeyCode::Left if state.active_tab == TabId::Simulate => {
            let cur = state
                .sim_scenario
                .boundary_reset_threshold
                .unwrap_or(250_000);
            state.sim_scenario.boundary_reset_threshold =
                Some(cur.saturating_sub(50_000).max(50_000));
            state.recompute_simulation();
        }
        KeyCode::Right if state.active_tab == TabId::Simulate => {
            let cur = state
                .sim_scenario
                .boundary_reset_threshold
                .unwrap_or(200_000);
            state.sim_scenario.boundary_reset_threshold = Some(cur.saturating_add(50_000));
            state.recompute_simulation();
        }
        KeyCode::Char('c') if state.active_tab == TabId::Simulate => {
            state.sim_scenario = dendro_core::simulation::Scenario::default();
            state.recompute_simulation();
        }
        _ => {}
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use serde_json::json;

    fn sample_report() -> Arc<Value> {
        Arc::new(json!({
            "metric": "decisions",
            "definition_version": "r1",
            "window_resolution": "recorded_bounds",
            "payload": {
                "product": "AI Fleet Intelligence",
                "from": "2026-10-08T00:00:00Z",
                "to": "2026-10-09T00:00:00Z",
                "tokens": 120000,
                "m1_yield": 40000,
                "m2_return_rate": 0.25,
                "m3_rework_chains": 1,
                "m3_rework_proportion": 0.25,
                "m4_impact": "insufficient_data",
                "m1_by_seat": [
                    {"seat": "dev-build@dendro-dev", "tokens": 80000, "closed_items": 2, "nonreturn_deliveries": 2, "tokens_per_closed_item": 40000},
                    {"seat": "rev-review@dendro-dev", "tokens": 40000, "closed_items": 1, "nonreturn_deliveries": 1, "tokens_per_closed_item": 40000}
                ],
                "activity": [
                    {"seat": "dev-build@dendro-dev", "observed_secs": 3600, "active_secs": 2400, "idle_with_work_secs": 600, "idle_without_work_secs": 600, "waiting_human_secs": 0, "unknown_secs": 0, "unparked_container_secs": 200, "leaf_stall_secs": 400, "engineering_calls": 40, "ceremony_calls": 5, "ceremony_ratio": 0.111},
                    {"seat": "qitem-20261010033627-be652926", "observed_secs": 100, "active_secs": 0, "idle_with_work_secs": 0, "idle_without_work_secs": 0, "waiting_human_secs": 0, "unknown_secs": 100}
                ],
                "handoff_friction": {
                    "pairs": [
                        {"source_seat": "dev-build@dendro-dev", "dest_seat": "rev-review@dendro-dev", "count": 4, "return_count": 1, "return_rate": 0.25, "wait_median_ms": 12000.0, "exec_median_ms": 45000.0, "ring_boundary": "in_ring"}
                    ],
                    "vortex_loops": []
                },
                "token_categories": {"input_uncached": 20000, "cache_read": 80000, "cache_write": 10000, "output": 10000},
                "fleet_concurrency": {"effective_parallelism": 1.4, "histogram": [{"active_seats": 1, "secs": 2400}]},
                "task_intake_overhead": {"assigned_mission_tokens": 100000, "task_intake_overhead": 20000, "ambiguous_tokens": 0, "unlinked_session": 0},
                "anomaly_spotlights": {"coverage": "observed", "findings": []},
                "advisory_idle_claimed": {"coverage": "observed", "findings": []}
            }
        }))
    }

    #[test]
    fn all_six_screens_and_rgba_modals_render_cleanly_at_120_and_80_cols() {
        for width in [120u16, 80u16] {
            let backend = TestBackend::new(width, 32);
            let mut terminal = Terminal::new(backend).unwrap();
            let theme = Theme { no_color: false };
            let mut state = AppState::new(
                PathBuf::from("dendro.duckdb"),
                Some("dendro-dev".into()),
                "7d".into(),
                None,
                None,
            );
            state.report = Some(sample_report());
            state.loading_stage = None;
            state.recompute_simulation();

            // Verify entity hygiene hides qitem-* by default on Seats tab
            assert_eq!(state.filtered_activity_rows().len(), 1);
            state.show_all_entities = true;
            assert_eq!(state.filtered_activity_rows().len(), 2);
            state.show_all_entities = false;

            for tab in [
                TabId::Pulse,
                TabId::Seats,
                TabId::Handoffs,
                TabId::Tokens,
                TabId::Simulate,
                TabId::Trace,
            ] {
                state.active_tab = tab;
                terminal
                    .draw(|f| views::render_app(f, &mut state, &theme))
                    .unwrap();
            }

            for modal in [
                AppModal::Help,
                AppModal::NaoAsk,
                AppModal::IssueDraft("## Draft".into()),
                AppModal::DetailJson("Seat".into(), "{\"ok\":true}".into()),
            ] {
                state.modal = Some(modal);
                terminal
                    .draw(|f| views::render_app(f, &mut state, &theme))
                    .unwrap();
            }
        }
    }
}
