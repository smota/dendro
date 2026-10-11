# Dendro — Quick Adoption Guide (5-Minute Quickstart)

> *"Just as dendrochronology deciphers historical climate patterns through the concentric growth rings of ancient trees, Dendro analyzes the cumulative execution layers of autonomous AI squads to reveal team dynamics, hidden friction, and true operational costs."*

---

## 1. What is Dendro?

**Dendro** is a high-performance, local AI fleet intelligence and decision engine for autonomous multi-agent squads. It connects directly to local execution logs across development harnesses (Claude Code, Codex, OpenRig, AGY, Grok, Pi), normalizes execution facts into embedded **DuckDB**, and answers the three core questions for autonomous fleet operators:

1. **Seat Dynamics & Optimization:** "Are my agent seats spending time idle waiting for downstream tasks? Do I have over-provisioned seats in the squad?"
2. **Handoff Flow & Rework:** "How much context and token cost is consumed when an agent hands off work? Is there circular review-to-dev rework?"
3. **Resource & Cost Mastery:** "Which mission consumed the most budget? Did any agent enter repetitive failing tool retry loops?"

---

## 2. 5-Minute Installation & Execution

### Prerequisites
* Rust (stable 1.80+)
* Git

### Step 1: Clone and Build the Binary
```bash
git clone https://github.com/smota/dendro.git
cd dendro
cargo build --release
```

### Step 2: Run Initial Ingestion
The `run` command discovers local agent logs on your machine (`~/.openrig/`, `~/.claude/`, `~/.codex/`, `~/.gemini/`) and populates the local `dendro.duckdb` database:

```bash
./target/release/dendro run --db dendro.duckdb
```

### Step 3: Check Fleet Status
```bash
./target/release/dendro status --db dendro.duckdb
```

Outputs an executive summary including:
* Total cataloged sessions by harness.
* OpenRig rigs identified and seats mapped.
* Total tool call counts and token volume.
* Model spend and cost estimates.

---

## 3. Continuous Monitoring (Background Watcher)

To keep your fleet database updated in real-time as agents execute:

```bash
./target/release/dendro watch --db dendro.duckdb --debounce-secs 2
```

The daemon monitors filesystem alterations with a 2-second debounce to prevent write contention during high-throughput LLM token streaming.

---

## 4. Natural Language Queries with Nao

Dendro ships with pre-configured semantic model templates for [Nao](https://github.com/getnao/nao):

```bash
./target/release/dendro init-nao --target . --db dendro.duckdb
nao "Which seat had the highest idle-with-work time this week?"
nao "Compare tool failure rate between Claude Code and Codex"
nao "Which mission generated the most repetitive bash retry loops?"
```

---

## 5. Privacy & Security Guarantees

* **100% Local-First:** Zero execution data leaves your machine. No external egress.
* **Strict Secret Redaction:** API keys (`sk-...`, `ghp_...`, `Bearer ...`) and credentials across 10 families are cryptographically sanitized before storage in DuckDB.
