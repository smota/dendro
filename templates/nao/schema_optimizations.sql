-- Production projection DDL lives in crates/dendro-core/src/semantic.sql.
-- Refresh with dendro materialize: this computes a canonical decisions snapshot,
-- fences concurrent source updates, then atomically replaces only derived caches.
-- Do not run the obsolete fct_* prototype against production facts.
SELECT 'Use dendro materialize with an explicit --from/--to scope' AS refresh_instruction;
