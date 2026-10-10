-- ==============================================================================
-- Dendro: Otimizações Estruturais de Dados para o DuckDB & Nao Semantic Layer
-- ==============================================================================

-- 1. ÍNDICES DE DESEMPENHO EM TABELAS DE FATOS E DIMENSÕES
CREATE INDEX IF NOT EXISTS idx_fct_activity_seat ON fct_seat_activity(seat_id, rig_id);
CREATE INDEX IF NOT EXISTS idx_fct_handoffs_seats ON fct_handoffs(from_seat, to_seat, is_rework);
CREATE INDEX IF NOT EXISTS idx_fct_token_burn_mission ON fct_token_burn(mission_id, seat_id);
CREATE INDEX IF NOT EXISTS idx_fct_tool_loops_seat ON fct_tool_loops(seat_id, has_error);

-- 2. VISTAS ANALÍTICAS PRÉ-AGREGADAS (Sub-Millisecond Query Response)

-- Vista 1: Ranking de Desperdício e Ociosidade de Assentos (Pergunta 1)
CREATE OR REPLACE VIEW v_seat_idle_ranking AS
SELECT 
    s.seat_id,
    s.rig_id,
    s.role,
    s.runtime,
    s.model,
    ROUND(COALESCE(SUM(a.active_seconds), 0) / 3600.0, 2) AS active_hours,
    ROUND(COALESCE(SUM(a.idle_with_work_seconds), 0) / 3600.0, 2) AS idle_with_work_hours,
    ROUND(COALESCE(SUM(a.idle_no_work_seconds), 0) / 3600.0, 2) AS idle_no_work_hours,
    ROUND(COALESCE(SUM(a.waiting_human_seconds), 0) / 3600.0, 2) AS waiting_human_hours,
    ROUND(
        COALESCE(SUM(a.idle_with_work_seconds), 0) * 100.0 / 
        NULLIF(SUM(COALESCE(a.active_seconds, 0) + COALESCE(a.idle_with_work_seconds, 0) + COALESCE(a.idle_no_work_seconds, 0)), 0),
        1
    ) AS waste_pct
FROM dim_seats s
LEFT JOIN fct_seat_activity a ON s.seat_id = a.seat_id
GROUP BY s.seat_id, s.rig_id, s.role, s.runtime, s.model;

-- Vista 2: Matriz de Retrabalho e Eficiência de Handoff (Pergunta 2)
CREATE OR REPLACE VIEW v_handoff_rework_analysis AS
SELECT 
    h.rig_id,
    h.from_seat,
    h.to_seat,
    COUNT(*) AS total_handoffs,
    COUNT(CASE WHEN h.is_rework THEN 1 END) AS rework_returns,
    ROUND(COUNT(CASE WHEN h.is_rework THEN 1 END) * 100.0 / COUNT(*), 1) AS rework_rate_pct,
    ROUND(AVG(h.queue_wait_seconds), 1) AS avg_wait_seconds,
    ROUND(AVG(h.intake_token_cost), 0) AS avg_intake_tokens
FROM fct_handoffs h
GROUP BY h.rig_id, h.from_seat, h.to_seat;

-- Vista 3: Queima de Tokens e Custo por Missão (Pergunta 3)
CREATE OR REPLACE VIEW v_mission_token_burn AS
SELECT 
    b.mission_id,
    b.rig_id,
    COUNT(DISTINCT b.seat_id) AS seats_involved,
    SUM(b.input_tokens) AS total_input_tokens,
    SUM(b.output_tokens) AS total_output_tokens,
    SUM(b.cache_read_tokens) AS total_cache_read_tokens,
    ROUND(SUM(b.cost_usd), 4) AS total_cost_usd
FROM fct_token_burn b
GROUP BY b.mission_id, b.rig_id;

-- Vista 4: Governança Executiva Consolidada (Indicadores M1 a M4)
CREATE OR REPLACE VIEW v_executive_governance AS
SELECT 
    m.rig_id,
    ROUND(m.seat_yield, 2) AS seat_yield,
    ROUND(m.handoff_efficiency * 100.0, 1) AS handoff_efficiency_pct,
    ROUND(m.rework_rate * 100.0, 1) AS build_rework_pct,
    ROUND(m.skill_exposure * 100.0, 1) AS skill_exposure_pct
FROM fct_decision_metrics m;
