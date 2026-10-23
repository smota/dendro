# Draft Grok oracles for Dendro v0.8.0

This directory is an independent draft expected-value pack. It is not accepted and not frozen. Claude review is required before any freeze. Nothing here authorizes builders to treat these vectors as the sealed T02 oracle.

## Author identity actually observed

| Field | Value |
|---|---|
| Session preamble model | Grok 4.7 |
| Model environment variable | unknown |
| `GROK_AGENT` | `1` |
| `GROK_SESSION_ID` | `01a16629-3fec-7cd2-a48b-ef28915ee843` |
| Distinct worker run id | unknown |
| Launch receipt | unknown |
| Host | Microsoft Windows NT 10.0.26300.0 |
| Shell | pwsh 7.6.5 |
| Foreign `CODEX_SESSION_ID` seen in the environment | `01a1650f-8575-7063-9296-f72c08de5175` (not this author) |

Backend model metadata was not exposed as an environment variable. The preamble model is the session identity, not a separate runtime receipt.

## Provenance

Inputs, SHA-256:

| File | SHA-256 |
|---|---|
| DESIGN.md | `7f6ca2da4ea5b4c7e6d78b565c3857a5e716a277c0dfec7abe37f40eb729f638` |
| ADVERSARIAL.md | `f0e3cbf6f3359a14f9816e509e38498fd5fc7bd2853778628cb6e0e2a1dd9a0f` |
| tasks.json | `b2433989ee71c654eeaf9b739ed427930c337732032aabac19e5facf17804d87` |

No implementation source, raw private trace, or product test output was read. Expected numbers are derived from the fixture statements and the normative arithmetic in those three files. Where a necessary definition is missing, the vector status is `unresolved_definition` or the `unresolved` array says so. Those gaps are not filled.

## Contents

- `vectors.jsonl`: 175 vectors, one JSON object per line.
- Pack files: accounting.json (16), domain.json (24), f-fixtures.json (93), insight.json (23), native-platform.json (9), security.json (10).
- `disputed-definitions.json`: vectors with a dispute flag or an unresolved field.
- `manifest.json`: counts, identity, input hashes.

Each vector has `vectorID`, `inputs`, `expected.status`, `expected.reasons`, `expected.values`, `sourceDefinition` (`file`, `line`, `symbol`), and draft flags `acceptanceState=draft_pending_claude_review`, `frozen=false`.

Rational amounts use `numerator/denominator` in lowest terms for the quantity. Currency canonical form keeps the minimal power-of-ten denominator from DESIGN section 2: USD `0.02` is `2/100`, and USD `10/100` is `1/10`. A source literal is kept beside that form when the fixture wrote a decimal.

## Verification performed

- Pointer lines were read and checked for the fixture id or a non-empty contract line.
- Fixture arithmetic was recomputed with exact rationals: concurrency `20/10=2`; abutment and overlap unions; allocation products and residuals; token total `120` versus forbidden `160`; counter delta `50`; cost `30/2=15`; cache `(90+1)/(100+10)=91/110` and mean `1/2`; `19/40=.475`; `11/16=.6875`; estimate `6/1000`; decimal equality `10/100=1/10`.
- Vector ids are unique. JSON was parsed back after write.
- No product code, commit, install, or freeze path was written.

## Deliberate gaps

Schema-3 golden database bytes, measured benchmark timings, ETW event identifiers, the MiB base (1000 versus 1024), HTML entity spelling, F05 parent direct tokens, F09 decrease endpoints, F10 attempt-success versus I20 accepted work, and I14 RSS versus Windows working set are unresolved or disputed in the vectors. See `disputed-definitions.json`.
