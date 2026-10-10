#!/usr/bin/env python3
"""
Dendro — Benchmark de Validação de Performance para Nao & DuckDB
Mede tempos de resposta das consultas analíticas para garantir latência < 20ms.
"""

import time
import os
import duckdb

def run_benchmark():
    db_path = "templates/nao/benchmark.duckdb"
    if os.path.exists(db_path):
        os.remove(db_path)

    print("🌲 Dendro — Inicializando Benchmark de Performance do Nao...")
    con = duckdb.connect(db_path)

    # 1. Criação de tabelas
    con.execute("""
        CREATE TABLE dim_seats (
            seat_id VARCHAR PRIMARY KEY,
            rig_id VARCHAR,
            logical_id VARCHAR,
            role VARCHAR,
            runtime VARCHAR,
            model VARCHAR
        );

        CREATE TABLE fct_seat_activity (
            id VARCHAR PRIMARY KEY,
            seat_id VARCHAR,
            rig_id VARCHAR,
            window_start TIMESTAMP WITH TIME ZONE,
            window_end TIMESTAMP WITH TIME ZONE,
            active_seconds DOUBLE,
            idle_with_work_seconds DOUBLE,
            idle_no_work_seconds DOUBLE,
            waiting_human_seconds DOUBLE
        );

        CREATE TABLE fct_handoffs (
            handoff_id VARCHAR PRIMARY KEY,
            rig_id VARCHAR,
            from_seat VARCHAR,
            to_seat VARCHAR,
            queue_wait_seconds DOUBLE,
            intake_token_cost DOUBLE,
            is_rework BOOLEAN
        );

        CREATE TABLE fct_token_burn (
            burn_id VARCHAR PRIMARY KEY,
            mission_id VARCHAR,
            seat_id VARCHAR,
            rig_id VARCHAR,
            model_name VARCHAR,
            input_tokens BIGINT,
            output_tokens BIGINT,
            cache_read_tokens BIGINT,
            cost_usd DOUBLE
        );

        CREATE TABLE fct_tool_loops (
            loop_id VARCHAR PRIMARY KEY,
            seat_id VARCHAR,
            tool_name VARCHAR,
            repeat_count INTEGER,
            has_error BOOLEAN
        );

        CREATE TABLE fct_decision_metrics (
            rig_id VARCHAR PRIMARY KEY,
            seat_yield DOUBLE,
            handoff_efficiency DOUBLE,
            rework_rate DOUBLE,
            skill_exposure DOUBLE
        );
    """)

    # 2. Inserção de dados sintéticos representativos (escala do host)
    print("📦 Populando dados sintéticos representativos...")
    
    # 8 assentos
    con.execute("""
        INSERT INTO dim_seats VALUES
        ('orch-arch@dendro-dev', 'dendro-dev', 'arch', 'architect', 'codex', 'gpt-6.1'),
        ('dev-build@dendro-dev', 'dendro-dev', 'build', 'builder', 'pi', 'grok-cli/grok-4.7'),
        ('dev-build-jr@dendro-dev', 'dendro-dev', 'build-jr', 'builder-jr', 'pi', 'grok-cli/grok-4.7'),
        ('dev-build-sr@dendro-dev', 'dendro-dev', 'build-sr', 'builder-sr', 'pi', 'grok-cli/grok-4.7'),
        ('dev-qa@dendro-dev', 'dendro-dev', 'qa', 'qa', 'codex', 'gpt-6.1'),
        ('rev-review@dendro-dev', 'dendro-dev', 'review', 'reviewer', 'codex', 'gpt-6.1'),
        ('pm-manager@dendro-pm', 'dendro-pm', 'manager', 'product-manager', 'claude-code', 'claude-3-7'),
        ('pm-analyst@dendro-pm', 'dendro-pm', 'analyst', 'analyst', 'claude-code', 'claude-3-7');
    """)

    # 5.000 amostras de atividade
    con.execute("""
        INSERT INTO fct_seat_activity
        SELECT 
            'act_' || i,
            CASE (i % 8)
                WHEN 0 THEN 'orch-arch@dendro-dev'
                WHEN 1 THEN 'dev-build@dendro-dev'
                WHEN 2 THEN 'dev-build-jr@dendro-dev'
                WHEN 3 THEN 'dev-build-sr@dendro-dev'
                WHEN 4 THEN 'dev-qa@dendro-dev'
                WHEN 5 THEN 'rev-review@dendro-dev'
                WHEN 6 THEN 'pm-manager@dendro-pm'
                ELSE 'pm-analyst@dendro-pm'
            END,
            CASE WHEN (i % 8) >= 6 THEN 'dendro-pm' ELSE 'dendro-dev' END,
            NOW() - INTERVAL (i * 10) MINUTE,
            NOW() - INTERVAL (i * 10 - 5) MINUTE,
            (i % 120) * 1.5,
            (i % 40) * 2.0,
            (i % 90) * 3.0,
            CASE WHEN (i % 50) = 0 THEN 180.0 ELSE 0.0 END
        FROM range(1, 5001) t(i);
    """)

    # 1.000 handoffs
    con.execute("""
        INSERT INTO fct_handoffs
        SELECT 
            'hf_' || i,
            'dendro-dev',
            'dev-build@dendro-dev',
            CASE WHEN (i % 5 = 0) THEN 'dev-build@dendro-dev' ELSE 'dev-qa@dendro-dev' END,
            12.5 + (i % 30),
            1200 + (i % 800),
            CASE WHEN (i % 7 = 0) THEN TRUE ELSE FALSE END
        FROM range(1, 1001) t(i);
    """)

    # 2.000 registros de token burn
    con.execute("""
        INSERT INTO fct_token_burn
        SELECT 
            'burn_' || i,
            'mission_' || (i % 25),
            'dev-build-sr@dendro-dev',
            'dendro-dev',
            'grok-cli/grok-4.7',
            45000 + (i * 12),
            8200 + (i * 4),
            120000 + (i * 30),
            0.12 + (i * 0.001)
        FROM range(1, 2001) t(i);
    """)

    # Decision metrics
    con.execute("""
        INSERT INTO fct_decision_metrics VALUES
        ('dendro-dev', 3.84, 0.942, 0.048, 0.32),
        ('agentflow-dev', 1.82, 0.765, 0.221, 0.15);
    """)

    # 3. Aplicar otimizações de schema
    print("⚡ Aplicando otimizações de schema (Views pré-agregadas & índices)...")
    with open("templates/nao/schema_optimizations.sql") as f:
        con.execute(f.read())

    # 4. Executar benchmark das consultas dos 4 dashboards
    dashboards = [
        ("Dashboard 1 (Seat Dynamics)", "SELECT * FROM v_seat_idle_ranking ORDER BY waste_pct DESC;"),
        ("Dashboard 2 (Handoff Flow & Rework)", "SELECT * FROM v_handoff_rework_analysis ORDER BY rework_returns DESC;"),
        ("Dashboard 3 (Cost & Token Burn)", "SELECT * FROM v_mission_token_burn ORDER BY total_cost_usd DESC LIMIT 10;"),
        ("Dashboard 4 (Executive Governance)", "SELECT * FROM v_executive_governance;"),
    ]

    print("\n⏱️  RESULTADOS DE BENCHMARK DE PERFORMANCE DO NAO (DUCKDB IN-MEMORY):")
    print("=" * 75)
    print(f"{'Consulta / Dashboard':<42} | {'Linhas':<8} | {'Latência (ms)':<15}")
    print("-" * 75)

    all_passed = True
    for name, query in dashboards:
        # Aquecimento
        con.execute(query).fetchall()
        
        # Medição precisa
        times = []
        for _ in range(10):
            t0 = time.perf_counter()
            res = con.execute(query).fetchall()
            t1 = time.perf_counter()
            times.append((t1 - t0) * 1000.0)

        avg_ms = sum(times) / len(times)
        min_ms = min(times)
        row_count = len(res)
        
        status = "✓ OK" if avg_ms < 20.0 else "⚠ SLOW"
        print(f"{name:<42} | {row_count:<8} | {avg_ms:6.2f} ms ({status})")
        if avg_ms >= 20.0:
            all_passed = False

    print("=" * 75)
    con.close()
    if os.path.exists(db_path):
        os.remove(db_path)

    if all_passed:
        print("🚀 SUCESSO: Todas as consultas analíticas responderam em menos de 20ms!")
        return 0
    else:
        print("Aviso: Algumas consultas excederam o limiar desejado.")
        return 1

if __name__ == "__main__":
    exit(run_benchmark())
