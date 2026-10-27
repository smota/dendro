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
use dendro_core::insight_v1::{InsightDomain, InsightResult};
use dendro_core::presentation::{classify_entity_id, EntityKind};
use futures::StreamExt;
use ratatui::{backend::CrosstermBackend, widgets::TableState, Terminal};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
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
    Insights,
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
            Self::Insights => 6,
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Pulse => Self::Seats,
            Self::Seats => Self::Handoffs,
            Self::Handoffs => Self::Tokens,
            Self::Tokens => Self::Simulate,
            Self::Simulate => Self::Trace,
            Self::Trace => Self::Insights,
            Self::Insights => Self::Pulse,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::Pulse => Self::Insights,
            Self::Seats => Self::Pulse,
            Self::Handoffs => Self::Seats,
            Self::Tokens => Self::Handoffs,
            Self::Simulate => Self::Tokens,
            Self::Trace => Self::Simulate,
            Self::Insights => Self::Trace,
        }
    }
}

/// Insights disclosure. L3 is the existing DetailJson modal, not a fourth screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum InsightDepth {
    #[default]
    Domain,
    Insight,
    Measure,
}

fn insight_domains() -> [InsightDomain; 5] {
    [
        InsightDomain::Model,
        InsightDomain::Harness,
        InsightDomain::Coordination,
        InsightDomain::Environment,
        InsightDomain::Security,
    ]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeatViewMode {
    Bands,
    Timeline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SeatGroupMode {
    #[default]
    BySeat,
    ByRig,
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
    ReportReady {
        report: Arc<Value>,
        request: Arc<dendro_core::presentation::ReadRequest>,
        status: Value,
        from: String,
        to: String,
        mode: String,
        insights: Option<Vec<InsightResult>>,
        insights_error: Option<String>,
    },
    TraceReady {
        metric_id: String,
        trace: Value,
        request: Arc<dendro_core::presentation::ReadRequest>,
        rig: Option<String>,
        from: String,
        to: String,
    },
    NaoAnswerReady {
        question: String,
        summary: String,
        full_text: String,
    },
    WatcherPulse(usize),
}

pub struct AppState {
    pub db: PathBuf,
    read_request: Option<Arc<dendro_core::presentation::ReadRequest>>,
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
    pub seat_group_mode: SeatGroupMode,
    pub token_cursor: usize,
    pub token_table_state: TableState,
    pub heatmap_metric: HeatmapMetric,
    pub show_all_entities: bool,
    pub filter_input_mode: bool,
    pub seat_filter: String,
    pub modal: Option<AppModal>,
    pub nao_input: String,
    pub nao_loading: bool,
    pub nao_selected_suggestion: usize,
    pub last_nao_answer: Option<(String, String)>,
    pub nao_modal_output: Option<(String, String)>,
    pub sim_cursor: usize,
    pub sim_scenario: dendro_core::simulation::Scenario,
    pub sim_result: Option<Value>,
    pub insights: Option<Vec<InsightResult>>,
    pub insights_error: Option<String>,
    pub insight_depth: InsightDepth,
    pub insight_cursor: usize,
    pub insight_domain: Option<InsightDomain>,
    /// Insight row chosen at L1. L2 moves `insight_cursor` over measures.
    pub insight_index: usize,
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
            read_request: None,
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
            seat_group_mode: SeatGroupMode::BySeat,
            token_cursor: 0,
            token_table_state: TableState::default(),
            heatmap_metric: HeatmapMetric::ReturnRate,
            show_all_entities: false,
            filter_input_mode: false,
            seat_filter: String::new(),
            modal: None,
            nao_input: String::new(),
            nao_loading: false,
            nao_selected_suggestion: 0,
            last_nao_answer: None,
            nao_modal_output: None,
            sim_cursor: 0,
            sim_scenario: dendro_core::simulation::Scenario::default(),
            sim_result: None,
            insights: None,
            insights_error: None,
            insight_depth: InsightDepth::Domain,
            insight_cursor: 0,
            insight_domain: None,
            insight_index: 0,
        }
    }

    fn reset_insight_focus(&mut self) {
        self.insight_depth = InsightDepth::Domain;
        self.insight_cursor = 0;
        self.insight_domain = None;
        self.insight_index = 0;
    }

    fn domain_results(&self, domain: InsightDomain) -> Vec<InsightResult> {
        self.insights
            .as_ref()
            .map(|rows| {
                rows.iter()
                    .filter(|row| row.domain == domain)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    fn selected_insight(&self) -> Option<InsightResult> {
        let domain = self.insight_domain?;
        let rows = self.domain_results(domain);
        if rows.is_empty() {
            return None;
        }
        let index = match self.insight_depth {
            InsightDepth::Measure => self.insight_index,
            _ => self.insight_cursor,
        };
        rows.get(index.min(rows.len() - 1)).cloned()
    }

    fn insight_list_len(&self) -> usize {
        if self.insights.is_none() {
            return 0;
        }
        match self.insight_depth {
            InsightDepth::Domain => insight_domains().len(),
            InsightDepth::Insight => self
                .insight_domain
                .map(|domain| self.domain_results(domain).len())
                .unwrap_or(0),
            InsightDepth::Measure => self
                .selected_insight()
                .map(|result| result.measures.len())
                .unwrap_or(0),
        }
    }

    fn drill_insight(&mut self) {
        if self.insights.is_none() {
            return;
        }
        let domains = insight_domains();
        match self.insight_depth {
            InsightDepth::Domain => {
                let cursor = self.insight_cursor.min(domains.len().saturating_sub(1));
                let domain = domains[cursor];
                if self.domain_results(domain).is_empty() {
                    return;
                }
                self.insight_domain = Some(domain);
                self.insight_depth = InsightDepth::Insight;
                self.insight_cursor = 0;
                self.insight_index = 0;
            }
            InsightDepth::Insight => {
                let rows = self
                    .insight_domain
                    .map(|domain| self.domain_results(domain))
                    .unwrap_or_default();
                if rows.is_empty() {
                    return;
                }
                self.insight_index = self.insight_cursor.min(rows.len() - 1);
                self.insight_cursor = 0;
                self.insight_depth = InsightDepth::Measure;
            }
            InsightDepth::Measure => self.open_selected_insight_json(),
        }
    }

    fn back_insight(&mut self) {
        match self.insight_depth {
            InsightDepth::Measure => {
                self.insight_depth = InsightDepth::Insight;
                self.insight_cursor = self.insight_index;
            }
            InsightDepth::Insight => {
                let domain = self.insight_domain;
                self.insight_depth = InsightDepth::Domain;
                self.insight_cursor = domain
                    .and_then(|domain| insight_domains().iter().position(|item| *item == domain))
                    .unwrap_or(0);
            }
            InsightDepth::Domain => {}
        }
    }

    fn open_selected_insight_json(&mut self) {
        let Some(result) = self.selected_insight() else {
            return;
        };
        let pretty = serde_json::to_string_pretty(&result).unwrap_or_default();
        self.modal = Some(AppModal::DetailJson(
            format!("Insight {}", result.id),
            pretty,
        ));
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

    /// Rows for the Seats tab: one row per seat, or one summed row per team/rig.
    pub fn active_seat_rows(&self) -> Vec<Value> {
        let rows = self.filtered_activity_rows();
        if self.seat_group_mode == SeatGroupMode::BySeat {
            return rows;
        }
        let m1_rows = self
            .report
            .as_ref()
            .and_then(|report| report["payload"]["m1_by_seat"].as_array())
            .cloned()
            .unwrap_or_default();
        let mut m1_by_seat: BTreeMap<String, (f64, f64, f64)> = BTreeMap::new();
        for row in &m1_rows {
            let Some(seat) = row["seat"].as_str() else {
                continue;
            };
            let entry = m1_by_seat.entry(seat.to_owned()).or_insert((0.0, 0.0, 0.0));
            entry.0 += json_f64(&row["tokens"]);
            entry.1 += json_f64(&row["closed_items"]);
            entry.2 += json_f64(&row["nonreturn_deliveries"]);
        }
        let mut groups: BTreeMap<String, RigAggregate> = BTreeMap::new();
        for row in &rows {
            let seat = row["seat"].as_str().unwrap_or("");
            let rig = rig_of_seat(seat);
            let group = groups.entry(rig).or_default();
            group.seats += 1;
            group.observed_secs += json_f64(&row["observed_secs"]);
            group.active_secs += json_f64(&row["active_secs"]);
            group.idle_with_work_secs += json_f64(&row["idle_with_work_secs"]);
            group.idle_without_work_secs += json_f64(&row["idle_without_work_secs"]);
            group.waiting_human_secs += json_f64(&row["waiting_human_secs"]);
            group.unknown_secs += json_f64(&row["unknown_secs"]);
            group.unparked_container_secs += json_f64(&row["unparked_container_secs"]);
            group.leaf_stall_secs += json_f64(&row["leaf_stall_secs"]);
            group.engineering_calls += json_f64(&row["engineering_calls"]);
            group.ceremony_calls += json_f64(&row["ceremony_calls"]);
            if group.seen_seats.insert(seat.to_owned()) {
                if let Some((tokens, closed, nonreturn)) = m1_by_seat.get(seat) {
                    group.tokens += tokens;
                    group.closed_items += closed;
                    group.nonreturn_deliveries += nonreturn;
                }
            }
        }
        groups
            .into_iter()
            .map(|(rig, group)| group.into_row(&rig))
            .collect()
    }

    pub fn token_seat_rows(&self) -> Vec<Value> {
        let Some(report) = &self.report else {
            return Vec::new();
        };
        report["payload"]["m1_by_seat"]
            .as_array()
            .map(|rows| {
                rows.iter()
                    .filter(|row| {
                        let seat = row["seat"].as_str().unwrap_or("");
                        self.show_all_entities || classify_entity_id(seat) == EntityKind::AgentSeat
                    })
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    fn sim_list_len(&self) -> usize {
        4 + self.canonical_seat_names().len()
    }

    fn clamp_sim_cursor(&mut self) {
        let max = self.sim_list_len().saturating_sub(1);
        self.sim_cursor = self.sim_cursor.min(max);
    }

    pub fn recompute_simulation(&mut self) {
        if let Some(rep) = &self.report {
            if let Ok(res) = dendro_core::simulation::simulate(rep, &self.sim_scenario) {
                self.sim_result = Some(res);
            }
        }
    }
}

fn json_f64(value: &Value) -> f64 {
    value.as_f64().filter(|n| n.is_finite()).unwrap_or(0.0)
}

fn json_whole(n: f64) -> Value {
    if n.is_finite() && n.fract() == 0.0 && (i64::MIN as f64..=i64::MAX as f64).contains(&n) {
        Value::from(n as i64)
    } else {
        serde_json::Number::from_f64(n)
            .map(Value::Number)
            .unwrap_or(Value::Null)
    }
}

fn rig_of_seat(seat: &str) -> String {
    match seat.split_once('@') {
        Some((_, rig)) if !rig.is_empty() => rig.to_string(),
        _ => "unscoped".to_string(),
    }
}

#[derive(Default)]
struct RigAggregate {
    seats: usize,
    seen_seats: BTreeSet<String>,
    observed_secs: f64,
    active_secs: f64,
    idle_with_work_secs: f64,
    idle_without_work_secs: f64,
    waiting_human_secs: f64,
    unknown_secs: f64,
    unparked_container_secs: f64,
    leaf_stall_secs: f64,
    engineering_calls: f64,
    ceremony_calls: f64,
    tokens: f64,
    closed_items: f64,
    nonreturn_deliveries: f64,
}

impl RigAggregate {
    fn into_row(self, rig: &str) -> Value {
        let calls = self.engineering_calls + self.ceremony_calls;
        let ceremony_ratio = if calls > 0.0 {
            self.ceremony_calls / calls
        } else {
            0.0
        };
        let tokens_per_closed_item = if self.closed_items > 0.0 {
            json_whole(self.tokens / self.closed_items)
        } else {
            Value::Null
        };
        serde_json::json!({
            "seat": format!("@{rig} ({} seats)", self.seats),
            "rig": rig,
            "observed_secs": json_whole(self.observed_secs),
            "active_secs": json_whole(self.active_secs),
            "idle_with_work_secs": json_whole(self.idle_with_work_secs),
            "idle_without_work_secs": json_whole(self.idle_without_work_secs),
            "waiting_human_secs": json_whole(self.waiting_human_secs),
            "unknown_secs": json_whole(self.unknown_secs),
            "unparked_container_secs": json_whole(self.unparked_container_secs),
            "leaf_stall_secs": json_whole(self.leaf_stall_secs),
            "engineering_calls": json_whole(self.engineering_calls),
            "ceremony_calls": json_whole(self.ceremony_calls),
            "ceremony_ratio": ceremony_ratio,
            "tokens": json_whole(self.tokens),
            "closed_items": json_whole(self.closed_items),
            "nonreturn_deliveries": json_whole(self.nonreturn_deliveries),
            "tokens_per_closed_item": tokens_per_closed_item,
        })
    }
}

/// Restores the terminal on return, error, panic unwind, and cancellation.
/// `process::exit` skips destructors, so callers restore before any exit that
/// can run while this guard is armed. A failed alternate-screen switch restores
/// raw mode before the error returns.
struct TerminalGuard {
    restored: bool,
}

impl TerminalGuard {
    fn enter() -> Result<Self> {
        enable_raw_mode()?;
        let mut guard = Self { restored: false };
        if let Err(err) = execute!(io::stdout(), EnterAlternateScreen) {
            guard.restore();
            return Err(err.into());
        }
        Ok(guard)
    }

    fn restore(&mut self) {
        if self.restored {
            return;
        }
        self.restored = true;
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        self.restore();
    }
}

fn spawn_hydration(
    tx: mpsc::Sender<WorkerEvent>,
    db: PathBuf,
    cached: Option<Arc<dendro_core::presentation::ReadRequest>>,
    rig: Option<String>,
    window: String,
    fixed_from: Option<String>,
    fixed_to: Option<String>,
) {
    tokio::task::spawn_blocking(move || {
        let result = (|| -> Result<()> {
            let request = match cached {
                Some(request) if request.is_current()? => request,
                _ => Arc::new(dendro_core::presentation::ReadRequest::open(&db)?),
            };
            let status = request.status(rig.as_deref())?;
            let (from, to, mode) = request.window(
                rig.as_deref(),
                Some(&window),
                fixed_from.as_deref(),
                fixed_to.as_deref(),
            )?;
            let mut report = dendro_core::pipeline::report_json_physical(
                request.database(),
                "decisions",
                rig.as_deref(),
                &from,
                &to,
            )?;
            report["window_resolution"] = serde_json::json!(mode.clone());
            // Rig is not an insight scope field. Evaluate the resolved window only.
            let insights_scope = dendro_core::insight_v1::Scope {
                harness: None,
                session: None,
                from: Some(from.clone()).filter(|value| !value.is_empty()),
                to: Some(to.clone()).filter(|value| !value.is_empty()),
            };
            let (insights, insights_error) = match request.open_store() {
                Ok(store) => (
                    Some(dendro_core::insight_v1::evaluate(&store, &insights_scope)),
                    None,
                ),
                Err(error) => (None, Some(dendro_core::redact(&error.to_string()))),
            };
            let _ = tx.blocking_send(WorkerEvent::ReportReady {
                report: Arc::new(report),
                request,
                status,
                from,
                to,
                mode,
                insights,
                insights_error,
            });
            Ok(())
        })();
        if let Err(error) = result {
            let _ = tx.blocking_send(WorkerEvent::StatusReady { status: serde_json::json!({"status":"unavailable", "error": dendro_core::redact(&error.to_string())}), from: String::new(), to: String::new(), mode: "unavailable".into() });
        }
    });
}

fn spawn_trace_fetch(
    tx: mpsc::Sender<WorkerEvent>,
    request: Option<Arc<dendro_core::presentation::ReadRequest>>,
    rig: Option<String>,
    from: String,
    to: String,
    metric_id: String,
) {
    let Some(request) = request else {
        return;
    };
    if from.is_empty() || to.is_empty() {
        return;
    }
    tokio::task::spawn_blocking(move || {
        if let Ok(trace) = dendro_core::pipeline::trace_json_physical(
            request.database(),
            &metric_id,
            rig.as_deref(),
            &from,
            &to,
        ) {
            let _ = tx.blocking_send(WorkerEvent::TraceReady {
                metric_id,
                trace,
                request,
                rig,
                from,
                to,
            });
        }
    });
}

fn spawn_nao_query(
    tx: mpsc::Sender<WorkerEvent>,
    db: PathBuf,
    report: Option<Arc<Value>>,
    question: String,
) {
    tokio::task::spawn_blocking(move || {
        let ans = if let Some(report) = report.as_deref() {
            dendro_core::semantic::answer_from_report(report, &question, None)
        } else {
            dendro_core::semantic::answer(&db, &question, None).unwrap_or_else(|err| {
                serde_json::json!({
                    "status": "insufficient_data",
                    "message": format!("Insufficient data: {err}")
                })
            })
        };
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
    });
}

fn nao_suggestion_example(state: &AppState) -> Option<String> {
    let suggestions = dendro_core::semantic::suggest_closest_intents(&state.nao_input);
    let index = state
        .nao_selected_suggestion
        .min(suggestions.len().saturating_sub(1));
    suggestions
        .get(index)
        .and_then(|item| item["examples"].as_array())
        .and_then(|examples| examples.first())
        .and_then(|value| value.as_str())
        .map(str::to_owned)
}

fn ensure_sim_boundary(state: &mut AppState) {
    if state.sim_scenario.boundary.is_none() {
        state.sim_scenario.boundary = Some(dendro_core::simulation::BoundaryModel {
            turns: 12,
            stale_context_tokens: 280_000,
            clean_context_tokens: 60_000,
            cold_boot_tokens: 25_000,
        });
    }
}

fn toggle_boundary_threshold(state: &mut AppState) {
    if state.sim_scenario.boundary_reset_threshold.is_some() {
        state.sim_scenario.boundary_reset_threshold = None;
        state.sim_scenario.boundary = None;
    } else {
        state.sim_scenario.boundary_reset_threshold = Some(250_000);
        ensure_sim_boundary(state);
    }
    state.recompute_simulation();
}

fn cycle_sim_seat(state: &mut AppState, seat: String) {
    let is_removed = state.sim_scenario.remove.iter().any(|name| name == &seat);
    let is_replacement = state.sim_scenario.replacement_seat.as_ref() == Some(&seat);
    if is_removed {
        state.sim_scenario.remove.retain(|name| name != &seat);
        state.sim_scenario.replacement_seat = Some(seat);
    } else if is_replacement {
        state.sim_scenario.replacement_seat = None;
    } else {
        state.sim_scenario.remove.push(seat);
    }
    state.recompute_simulation();
}

fn activate_sim_cursor(state: &mut AppState) {
    state.clamp_sim_cursor();
    match state.sim_cursor {
        0 => {
            state.sim_scenario.direct_handoffs = !state.sim_scenario.direct_handoffs;
            state.recompute_simulation();
        }
        1 => {
            state.sim_scenario.enforce_model_family_diversity =
                !state.sim_scenario.enforce_model_family_diversity;
            state.recompute_simulation();
        }
        2 => toggle_boundary_threshold(state),
        3 => {
            state.sim_scenario = dendro_core::simulation::Scenario::default();
            state.recompute_simulation();
        }
        cursor => {
            let index = cursor - 4;
            if let Some(seat) = state.canonical_seat_names().get(index).cloned() {
                cycle_sim_seat(state, seat);
            }
        }
    }
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
        let report = tokio::task::spawn_blocking(move || -> Result<Value> {
            let request = dendro_core::presentation::ReadRequest::open(&db)?;
            let (from, to, mode) = request.window(
                rig.as_deref(),
                Some(&win_str),
                from.as_deref(),
                to.as_deref(),
            )?;
            let mut report = dendro_core::pipeline::report_json_physical(
                request.database(),
                "decisions",
                rig.as_deref(),
                &from,
                &to,
            )?;
            report["window_resolution"] = serde_json::json!(mode);
            Ok(report)
        })
        .await??;
        dendro_core::presentation::terminal_ui(&report, &[], true)?;
        return Ok(());
    }

    let mut guard = TerminalGuard::enter()?;
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
        state.read_request.clone(),
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
    let mut shutdown = crate::ShutdownEvents::new()?;

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
            _cause = shutdown.next() => {
                // Native console/signal event. Restore before the function
                // returns; do not describe a Windows event as a Unix signal.
                dendro_core::RUN_CANCEL.request();
                break;
            }
            _ = tick.tick() => {
                state.tick_count = state.tick_count.wrapping_add(1);
            }
            Some(ev) = worker_rx.recv() => {
                match ev {
                    WorkerEvent::StatusReady { status, from, to, mode } => {
                        state.insights = None;
                        state.insights_error = status
                            .get("error")
                            .and_then(|value| value.as_str())
                            .map(str::to_owned)
                            .or_else(|| Some("unavailable".to_string()));
                        state.reset_insight_focus();
                        state.status = Some(status);
                        state.resolved_from = from;
                        state.resolved_to = to;
                        state.window_mode = Some(mode);
                        state.loading_stage = None;
                        state.report = None;
                        state.read_request = None;
                        state.traces.clear();
                    }
                    WorkerEvent::ReportReady {
                        report,
                        request,
                        status,
                        from,
                        to,
                        mode,
                        insights,
                        insights_error,
                    } => {
                        state.traces.clear();
                        state.read_request = Some(request);
                        state.report = Some(report);
                        state.status = Some(status);
                        state.resolved_from = from;
                        state.resolved_to = to;
                        state.window_mode = Some(mode);
                        state.insights = insights;
                        state.insights_error = insights_error;
                        state.reset_insight_focus();
                        state.loading_stage = None;
                        state.recompute_simulation();
                        if !theme.no_color {
                            effects.add_effect(fx::coalesce((250, Interpolation::SineOut)));
                        }
                    }
                    WorkerEvent::TraceReady { metric_id, trace, request, rig, from, to } => {
                        if state.read_request.as_ref().is_some_and(|current| Arc::ptr_eq(current, &request)) && state.rig == rig && state.resolved_from == from && state.resolved_to == to {
                            state.traces.insert(metric_id, trace);
                        }
                    }
                    WorkerEvent::NaoAnswerReady { question, summary, full_text } => {
                        state.nao_loading = false;
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
                        dendro_core::RUN_CANCEL.request();
                        break;
                    }
                }
            }
        }
    }

    guard.restore();
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
                    state.nao_selected_suggestion = 0;
                }
                KeyCode::Up => {
                    state.nao_selected_suggestion = state.nao_selected_suggestion.saturating_sub(1);
                }
                KeyCode::Down => {
                    let count =
                        dendro_core::semantic::suggest_closest_intents(&state.nao_input).len();
                    if count > 0 {
                        state.nao_selected_suggestion =
                            (state.nao_selected_suggestion + 1).min(count - 1);
                    }
                }
                KeyCode::Tab => {
                    if let Some(example) = nao_suggestion_example(state) {
                        state.nao_input = example;
                        state.nao_selected_suggestion = 0;
                    }
                }
                KeyCode::Enter => {
                    let mut question = state.nao_input.trim().to_owned();
                    if question.is_empty() {
                        if let Some(example) = nao_suggestion_example(state) {
                            state.nao_input = example.clone();
                            question = example;
                        }
                    }
                    if !question.is_empty() {
                        state.nao_loading = true;
                        spawn_nao_query(
                            worker_tx.clone(),
                            state.db.clone(),
                            state.report.clone(),
                            question,
                        );
                    }
                }
                KeyCode::Char(c) => {
                    state.nao_input.push(c);
                    state.nao_selected_suggestion = 0;
                }
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
            state.nao_selected_suggestion = 0;
            state.modal = Some(AppModal::NaoAsk);
            trigger_transition(effects);
        }
        KeyCode::Char('/') => {
            state.active_tab = TabId::Seats;
            state.filter_input_mode = true;
        }
        KeyCode::Char('7') => {
            state.active_tab = TabId::Insights;
            trigger_transition(effects);
        }
        KeyCode::Right | KeyCode::Enter if state.active_tab == TabId::Insights => {
            state.drill_insight();
        }
        KeyCode::Left if state.active_tab == TabId::Insights => {
            state.back_insight();
        }
        KeyCode::Tab | KeyCode::Right if state.active_tab != TabId::Simulate => {
            state.active_tab = state.active_tab.next();
            if state.active_tab == TabId::Trace
                && !state.traces.contains_key(&state.active_trace_metric)
            {
                spawn_trace_fetch(
                    worker_tx.clone(),
                    state.read_request.clone(),
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
                    state.read_request.clone(),
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
                    state.read_request.clone(),
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
            } else if state.active_tab == TabId::Tokens {
                let max = state.token_seat_rows().len().saturating_sub(1);
                state.token_cursor = state.token_cursor.min(max).saturating_sub(1);
            } else if state.active_tab == TabId::Simulate {
                state.clamp_sim_cursor();
                state.sim_cursor = state.sim_cursor.saturating_sub(1);
            } else if state.active_tab == TabId::Insights {
                state.insight_cursor = state.insight_cursor.saturating_sub(1);
            } else {
                state.selected_seat = state.selected_seat.saturating_sub(1);
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if state.active_tab == TabId::Trace {
                state.trace_scroll = state.trace_scroll.saturating_add(3);
            } else if state.active_tab == TabId::Handoffs {
                state.selected_pair = state.selected_pair.saturating_add(1);
            } else if state.active_tab == TabId::Tokens {
                let max = state.token_seat_rows().len().saturating_sub(1);
                state.token_cursor = (state.token_cursor + 1).min(max);
            } else if state.active_tab == TabId::Simulate {
                state.clamp_sim_cursor();
                let max = state.sim_list_len().saturating_sub(1);
                state.sim_cursor = (state.sim_cursor + 1).min(max);
            } else if state.active_tab == TabId::Seats {
                let max = state.active_seat_rows().len().saturating_sub(1);
                state.selected_seat = (state.selected_seat + 1).min(max);
            } else if state.active_tab == TabId::Insights {
                let max = state.insight_list_len().saturating_sub(1);
                state.insight_cursor = (state.insight_cursor + 1).min(max);
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
        KeyCode::Char('g') if state.active_tab == TabId::Seats => {
            state.seat_group_mode = match state.seat_group_mode {
                SeatGroupMode::BySeat => SeatGroupMode::ByRig,
                SeatGroupMode::ByRig => SeatGroupMode::BySeat,
            };
            state.selected_seat = 0;
        }
        KeyCode::Char('s') if state.active_tab == TabId::Seats => {
            state.active_tab = TabId::Simulate;
            state.recompute_simulation();
            trigger_transition(effects);
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
                state.read_request.clone(),
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
                    state.read_request.clone(),
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
            if let Some(row) = state.active_seat_rows().get(state.selected_seat) {
                let seat = row["seat"].as_str().unwrap_or("Seat").to_owned();
                let pretty = serde_json::to_string_pretty(row).unwrap_or_default();
                state.modal = Some(AppModal::DetailJson(
                    format!("Seat Evidence: {seat}"),
                    pretty,
                ));
            }
        }
        KeyCode::Enter | KeyCode::Char(' ') if state.active_tab == TabId::Simulate => {
            activate_sim_cursor(state);
        }
        KeyCode::Char('p') if state.active_tab == TabId::Simulate => {
            state.clamp_sim_cursor();
            if state.sim_cursor >= 4 {
                if let Some(seat) = state
                    .canonical_seat_names()
                    .get(state.sim_cursor - 4)
                    .cloned()
                {
                    if state.sim_scenario.replacement_seat.as_ref() == Some(&seat) {
                        state.sim_scenario.replacement_seat = None;
                    } else {
                        state.sim_scenario.remove.retain(|name| name != &seat);
                        state.sim_scenario.replacement_seat = Some(seat);
                    }
                    state.recompute_simulation();
                }
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
            ensure_sim_boundary(state);
            state.recompute_simulation();
        }
        KeyCode::Right if state.active_tab == TabId::Simulate => {
            let cur = state
                .sim_scenario
                .boundary_reset_threshold
                .unwrap_or(200_000);
            state.sim_scenario.boundary_reset_threshold = Some(cur.saturating_add(50_000));
            ensure_sim_boundary(state);
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
                TabId::Insights,
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

    #[test]
    fn compact_numbers_match_the_k_m_b_scale() {
        assert_eq!(views::fmt_compact_f64(950.0), "950");
        assert_eq!(views::fmt_compact_f64(12_400.0), "12.4k");
        assert_eq!(views::fmt_compact_f64(113_619_968.0), "113.6M");
        assert_eq!(views::fmt_compact_f64(4_688_911_688.0), "4.69B");
        assert_eq!(views::fmt_share_pct(&json!(2_400.0), 100_000.0), "(2.4%)");
    }

    #[test]
    fn positive_active_band_keeps_a_cell_and_fills_the_width() {
        let theme = Theme { no_color: true };
        let line = views::build_colored_band_line(
            &json!({
                "observed_secs": 1000,
                "active_secs": 1,
                "idle_with_work_secs": 0,
                "idle_without_work_secs": 0,
                "waiting_human_secs": 0,
                "unknown_secs": 999
            }),
            28,
            &theme,
        );
        let text: String = line
            .spans
            .iter()
            .map(|span| span.content.to_string())
            .collect();
        assert!(text.starts_with('█'), "{text}");
        assert_eq!(text.chars().count(), 28);
    }

    #[test]
    fn rig_aggregation_sums_bands_and_matching_tokens() {
        let mut state = AppState::new(
            PathBuf::from("dendro.duckdb"),
            None,
            "7d".into(),
            None,
            None,
        );
        state.report = Some(sample_report());
        state.seat_group_mode = SeatGroupMode::ByRig;
        let rows = state.active_seat_rows();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["seat"], "@dendro-dev (1 seats)");
        assert_eq!(rows[0]["active_secs"], 2400);
        assert_eq!(rows[0]["tokens"], 80000);
        assert_eq!(rows[0]["closed_items"], 2);
    }

    #[test]
    fn scenario_cursor_cycles_remove_then_replace_and_enables_boundary() {
        let mut state = AppState::new(
            PathBuf::from("dendro.duckdb"),
            None,
            "7d".into(),
            None,
            None,
        );
        state.report = Some(sample_report());
        state.sim_cursor = 4;
        activate_sim_cursor(&mut state);
        assert_eq!(
            state.sim_scenario.remove,
            vec!["dev-build@dendro-dev".to_string()]
        );
        assert!(state.sim_scenario.replacement_seat.is_none());
        activate_sim_cursor(&mut state);
        assert!(state.sim_scenario.remove.is_empty());
        assert_eq!(
            state.sim_scenario.replacement_seat.as_deref(),
            Some("dev-build@dendro-dev")
        );
        activate_sim_cursor(&mut state);
        assert!(state.sim_scenario.replacement_seat.is_none());
        state.sim_cursor = 2;
        activate_sim_cursor(&mut state);
        assert_eq!(state.sim_scenario.boundary_reset_threshold, Some(250_000));
        let boundary = state.sim_scenario.boundary.as_ref().unwrap();
        assert_eq!(boundary.turns, 12);
        assert_eq!(boundary.stale_context_tokens, 280_000);
        assert_eq!(boundary.clean_context_tokens, 60_000);
        assert_eq!(boundary.cold_boot_tokens, 25_000);
        activate_sim_cursor(&mut state);
        assert!(state.sim_scenario.boundary_reset_threshold.is_none());
        assert!(state.sim_scenario.boundary.is_none());
    }

    fn screen_text(state: &mut AppState) -> String {
        let backend = TestBackend::new(180, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        let theme = Theme { no_color: true };
        terminal
            .draw(|frame| views::render_app(frame, state, &theme))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let mut text = String::new();
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                text.push_str(buffer[(x, y)].symbol());
            }
            text.push('\n');
        }
        text
    }

    fn press(state: &mut AppState, code: KeyCode) {
        let (tx, _rx) = tokio::sync::mpsc::channel(1);
        let mut effects: EffectManager<()> = EffectManager::default();
        let theme = Theme { no_color: true };
        let _ = handle_key_event(
            state,
            crossterm::event::KeyEvent::new(code, KeyModifiers::NONE),
            &tx,
            &mut effects,
            &theme,
        );
    }

    fn cost_insight() -> InsightResult {
        use dendro_core::insight_v1::{InsightPopulation, InsightStatus, Measure, MeasureValue};
        InsightResult {
            id: "I04",
            name: "model.cost",
            version: "v1",
            domain: InsightDomain::Model,
            status: InsightStatus::Computed,
            population: InsightPopulation {
                what: "usage_facts".into(),
                n: 3,
            },
            measures: vec![
                Measure {
                    id: "m.cost".into(),
                    name: "cost".into(),
                    value: Some(MeasureValue::Float(1.5)),
                    unit: "usd".into(),
                    basis: "status=complete".into(),
                },
                Measure {
                    id: "m.gap".into(),
                    name: "gap".into(),
                    value: None,
                    unit: "usd".into(),
                    basis: "unknown".into(),
                },
            ],
            coverage: "usage_facts".into(),
            evidence: vec![],
        }
    }

    fn refused_insight(
        id: &'static str,
        name: &'static str,
        domain: InsightDomain,
    ) -> InsightResult {
        use dendro_core::insight_v1::InsightPopulation;
        InsightResult {
            id,
            name,
            version: "v1",
            domain,
            status: dendro_core::insight_v1::InsightStatus::Refused {
                reason: "empty population".into(),
            },
            population: InsightPopulation {
                what: "usage_facts".into(),
                n: 0,
            },
            measures: vec![],
            coverage: "empty population".into(),
            evidence: vec![],
        }
    }

    fn report_with_costs() -> Arc<Value> {
        let mut report = (*sample_report()).clone();
        report["payload"]["observed_cost"] = json!({
            "status": "complete",
            "usd": 2.5,
            "unknown_rows": 0,
            "known_rows": 3
        });
        report["payload"]["estimated_cost"] = json!({
            "status": "partial",
            "usd": null,
            "unknown_rows": 4,
            "known_rows": 1
        });
        Arc::new(report)
    }

    #[test]
    fn insights_tab_renders_populated_results_and_report_costs() {
        let mut state = AppState::new(
            PathBuf::from("dendro.duckdb"),
            None,
            "7d".into(),
            None,
            None,
        );
        state.loading_stage = None;
        state.report = Some(report_with_costs());
        state.insights = Some(vec![cost_insight()]);
        state.active_tab = TabId::Tokens;
        let tokens = screen_text(&mut state);
        assert!(tokens.contains("Observed: complete 2.5 usd"), "{tokens}");
        assert!(
            tokens.contains("Estimated: partial (4 unknown rows)"),
            "{tokens}"
        );
        assert!(!tokens.contains("Estimated: partial 0"), "{tokens}");
        assert!(!tokens.contains("Estimated: 0"), "{tokens}");

        press(&mut state, KeyCode::Char('7'));
        assert_eq!(state.active_tab, TabId::Insights);
        let domains = screen_text(&mut state);
        assert!(domains.contains("DENDRO v0.8"), "{domains}");
        assert!(domains.contains("Domains: Model"), "{domains}");
        assert!(domains.contains("Harness"), "{domains}");
        assert!(domains.contains("Coordination"), "{domains}");
        assert!(domains.contains("Environment"), "{domains}");
        assert!(domains.contains("Security"), "{domains}");
        assert!(
            domains.contains("[r] Rig is the OpenRig adapter scope (<role>@<rig>)"),
            "{domains}"
        );
        assert!(domains.contains("[r] Rig:"), "{domains}");
        assert!(domains.contains("model"), "{domains}");
        assert!(domains.contains("computed"), "{domains}");
        assert!(
            domains.contains("unknown (no registered insight)"),
            "{domains}"
        );
        assert!(!domains.contains("n=0"), "{domains}");

        press(&mut state, KeyCode::Enter);
        assert_eq!(state.insight_depth, InsightDepth::Insight);
        let insights = screen_text(&mut state);
        assert!(insights.contains("I04"), "{insights}");
        assert!(insights.contains("model.cost"), "{insights}");
        assert!(insights.contains("usage_facts n=3"), "{insights}");
        assert!(insights.contains("cost 1.5 usd"), "{insights}");
        assert!(insights.contains("gap unknown usd"), "{insights}");
        assert!(!insights.contains("gap 0"), "{insights}");
        assert!(!insights.contains("gap unknown 0"), "{insights}");

        press(&mut state, KeyCode::Right);
        assert_eq!(state.insight_depth, InsightDepth::Measure);
        let measures = screen_text(&mut state);
        assert!(measures.contains("1.5 usd"), "{measures}");
        assert!(measures.contains("unknown usd"), "{measures}");
        assert!(!measures.contains("unknown 0"), "{measures}");

        press(&mut state, KeyCode::Enter);
        assert!(matches!(state.modal, Some(AppModal::DetailJson(_, _))));
        let detail = screen_text(&mut state);
        assert!(detail.contains("I04"), "{detail}");
        assert!(detail.contains("computed"), "{detail}");
        assert!(detail.contains("v1"), "{detail}");
        assert!(detail.contains("null"), "{detail}");
    }

    #[test]
    fn insights_tab_renders_all_refused_results_as_refused() {
        let mut state = AppState::new(
            PathBuf::from("dendro.duckdb"),
            None,
            "7d".into(),
            None,
            None,
        );
        state.loading_stage = None;
        state.report = Some(sample_report());
        state.insights = Some(vec![
            refused_insight("I02", "model.tokens.direct", InsightDomain::Model),
            refused_insight("I04", "model.cost", InsightDomain::Model),
            refused_insight("I08", "harness.action_failure", InsightDomain::Harness),
            refused_insight("I09", "work.lineage_shape", InsightDomain::Coordination),
            refused_insight("I14", "environment.resources", InsightDomain::Environment),
            refused_insight("I18", "coverage.source", InsightDomain::Environment),
        ]);
        state.active_tab = TabId::Insights;
        let domains = screen_text(&mut state);
        assert!(domains.contains("refused"), "{domains}");
        assert!(
            domains.contains("unknown (no registered insight)"),
            "{domains}"
        );
        assert!(!domains.contains("n=0"), "{domains}");
        assert!(!domains.contains("0 usd"), "{domains}");

        state.drill_insight();
        let insights = screen_text(&mut state);
        assert!(
            insights.contains("refused (empty population)"),
            "{insights}"
        );
        assert!(insights.contains("I02"), "{insights}");
        assert!(insights.contains('—'), "{insights}");
        assert!(!insights.contains("n=0"), "{insights}");
        assert!(!insights.contains("0 usd"), "{insights}");

        state.drill_insight();
        let measures = screen_text(&mut state);
        assert!(
            measures.contains("no measures — refused, partial, and unknown are not zero"),
            "{measures}"
        );
        assert!(!measures.contains("n=0"), "{measures}");
        assert!(!measures.contains("0 usd"), "{measures}");
    }

    #[test]
    fn insights_tab_renders_store_error_without_invented_numbers() {
        let mut state = AppState::new(
            PathBuf::from("dendro.duckdb"),
            None,
            "7d".into(),
            None,
            None,
        );
        state.loading_stage = None;
        state.report = None;
        state.status = Some(json!({"status": "unavailable", "error": "store read failed"}));
        state.insights = None;
        state.insights_error = Some("store read failed".into());
        state.active_tab = TabId::Insights;
        let text = screen_text(&mut state);
        assert!(
            text.contains("error: insight store did not return results"),
            "{text}"
        );
        assert!(text.contains("store read failed"), "{text}");
        assert!(text.contains("[7] Insights"), "{text}");
        assert!(text.contains("DENDRO v0.8"), "{text}");
        assert!(text.contains("Domains: Model"), "{text}");
        assert!(
            text.contains("OpenRig adapter scope (<role>@<rig>)"),
            "{text}"
        );
        assert!(!text.contains("computed"), "{text}");
        assert!(!text.contains("n=0"), "{text}");
        assert!(!text.contains("0 usd"), "{text}");
        assert!(!text.contains("0.0"), "{text}");
        state.drill_insight();
        assert!(state.modal.is_none());
        assert_eq!(state.insight_depth, InsightDepth::Domain);
    }
}
