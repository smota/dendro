# Native product telemetry

The Rust product exports best-effort OTLP metrics only when `observability/consumer.env`
exists under `$XDG_CONFIG_HOME` (or `~/.config`). It reads at most 16 KiB as data,
never as shell code. Only `OTEL_EXPORTER_OTLP_ENDPOINT`, `OTEL_SDK_DISABLED`, and
`DENDRO_TELEMETRY_DISABLED` are interpreted. All other fields are ignored.
The endpoint must be a numeric loopback HTTP socket address, normally
`http://127.0.0.1:24318`. No hostname resolution, remote endpoints, credentials,
headers, resource overrides, redirects, retries or persistent spool are supported.

Set `DENDRO_TELEMETRY_DISABLED=1` (or `true`) or `OTEL_SDK_DISABLED=true`
in the process environment or consumer file before starting the process to disable
export. Configuration is read once per process. Missing/invalid configuration disables it.

Requests go to `/v1/metrics` with **Content-Type: application/json**. This intentionally
uses OTLP JSON even when the shared consumer declares the standard `http/protobuf`
protocol. The existing collector must accept OTLP JSON; this does not change its configuration.
No SDK or additional dependency is installed.

`service.name=dendro`, instrumentation scope `native-product`. Metrics:

- `dendro.operation.count`: delta monotonic sum, one observed operation.
- `dendro.operation.duration`: delta histogram in seconds, one actual runtime sample,
  with count/sum/min/max and one unbounded bucket.

Only fixed `operation` and `outcome` labels are exported. No paths, arguments,
task/graph/session names or IDs, prompts, errors, messages, content, models or host
identity are included. Valid version-00 `TRACEPARENT` values attach trace/span IDs
as duration exemplars, never metric labels. Invalid and zero IDs are ignored.
This is correlation with a caller's span, not creation of a new tracing span.

One background thread owns a 32-item queue. Enqueue never waits; overflow drops
observations. Connect/read/write waits have 50 ms socket deadlines and each request has a
150 ms total budget. Request bodies are capped at 8 KiB. HTTP response
inspection is capped at 256 bytes; failures produce no output and cannot change
operation results. Normal CLI shutdown drains for at most 150 ms, then permits loss.
Crashes, forced exits, a slow collector and queue pressure can lose observations.
This is operational monitoring, not an exact accounting ledger or a replacement
for persisted product evidence. Source builds do not activate installed binaries.

Parsed CLI operations report fixed categories: status, run, watch, run_agent,
report, trace, serve, advise, assert, issue_draft, insights, storage, extension.
Success means normal return or explicit zero exit; nonzero exits/returned errors
are failure. Duration spans native runtime setup and the operation. Help/version
and parse failures are excluded; panic/abort do not fabricate completion samples.
Long-running watch/serve operations report only when they end.

`http.connection` measures each connected dashboard HTTP handler from entry through
its return, including SSE connection lifetime. Success means handler returned Ok;
failure means handler returned an error. This is transport/handler outcome, not
HTTP status or query-semantic success: served 404/405 or an HTTP-200 error payload
can still be successful handler returns. No URI/query/header/session content is read
for telemetry and no route label is derived from it. CLI initialization is outside
Tokio; the library server initializes configuration with spawn_blocking before accepts.

This monitors native product operation health; it neither re-ingests flight-recorder
history nor scrapes user session content. Adopted SDLC runner telemetry remains separate.
