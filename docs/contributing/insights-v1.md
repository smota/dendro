# Insights v1

Reference insights are compiled Rust types. Nothing here loads a plugin or accepts SQL from a caller.

## Add an insight

1. Add one file under `crates/dendro-core/src/insight_v1/reference/`. Implement `Insight` on a unit struct and expose it as `pub static INSIGHT`.
2. Add one line to `REFERENCE` in `reference/mod.rs`: `&your_mod::INSIGHT as &dyn Insight`.

Declare the module next to the others. `check_question_admission` admits the new `id` and `name` from that slice. Anything else stays `RegistryNotAdmitted`.

`cargo test -p dendro-core insight_v1` checks that ids and names are unique.

## Rules

- The query text is a fixed string in the insight file. Bind harness, session, and the half-open window with `?`. Do not format those values into SQL.
- Read through `InsightInput`. `usage_rows` and `warning_rows` are the shared fixed queries. A new insight may use `connection()` with its own `prepare` and `params!`.
- Unknown is `None`, or a partial/unknown cost status. It is never `0`, and a partial cost is never the smaller sum of the known rows.
- An empty population is `Refused`. An empty scope string or an inverted window is `Refused` (`undefined input`). A population that exists but cannot support a ratio is `Partial`, and that measure is `None`.
- Different populations and different formulas get different measure ids. `cache_read_ratio_of_sums` and `cache_read_mean_of_ratios` stay separate. Observed USD and estimated USD stay separate and are never added.
- Evidence is at most five stored `source_key` values, already sealed. Insights do not seal or unseal them.
- Results and measures are ordered by id.

## Run

`dendro insights [--json] [--harness H] [--session S]`

The command opens the selected database with `ReadRequest::open` and `open_store` (read-only). `--json` prints the result array. Without it, the command prints a text table.
