//! Opt-in native product metrics. No content, credentials, SDK, retry or persistent spool.
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::{mpsc, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const DEADLINE: Duration = Duration::from_millis(50);
const QUEUE: usize = 32;
const MAX_CONFIG: u64 = 16 * 1024;
static EXPORTER: OnceLock<Option<Exporter>> = OnceLock::new();

struct Config {
    address: SocketAddr,
}
fn disabled(value: &str) -> bool {
    matches!(value, "1" | "true" | "TRUE")
}
impl Config {
    fn parse(input: &str) -> Option<Self> {
        let mut endpoint = None;
        for line in input.lines() {
            let Some((key, value)) = line.trim().split_once('=') else {
                continue;
            };
            let value = value.trim();
            match key.trim() {
                "OTEL_EXPORTER_OTLP_ENDPOINT" => endpoint = Some(value),
                "OTEL_SDK_DISABLED" => {
                    if disabled(value) {
                        return None;
                    }
                }
                key if key == OFF_SWITCH && disabled(value) => return None,
                _ => {} // Deliberately ignore shell syntax, headers, service names and resource data.
            }
        }
        let address: SocketAddr = endpoint?
            .strip_prefix("http://")?
            .trim_end_matches('/')
            .parse()
            .ok()?;
        (address.ip().is_loopback() && address.port() != 0).then_some(Self { address })
    }
    fn load() -> Option<Self> {
        if [OFF_SWITCH, "OTEL_SDK_DISABLED"]
            .iter()
            .any(|key| std::env::var(key).is_ok_and(|v| disabled(&v)))
        {
            return None;
        }
        let root = std::env::var_os("XDG_CONFIG_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config"))
            })?;
        let path = root.join("observability/consumer.env");
        if !std::fs::metadata(&path).ok()?.is_file() {
            return None;
        }
        let file = std::fs::File::open(path).ok()?;
        let metadata = file.metadata().ok()?;
        if !metadata.is_file() || metadata.len() > MAX_CONFIG {
            return None;
        }
        let mut input = String::new();
        file.take(MAX_CONFIG + 1).read_to_string(&mut input).ok()?;
        if input.len() as u64 > MAX_CONFIG {
            return None;
        }
        Self::parse(&input)
    }
}

enum Message {
    Data(Vec<u8>),
    Flush(mpsc::SyncSender<()>),
}
struct Exporter {
    tx: mpsc::SyncSender<Message>,
}
impl Exporter {
    fn new(config: Config) -> Option<Self> {
        let (tx, rx) = mpsc::sync_channel(QUEUE);
        std::thread::Builder::new()
            .name(format!("{SERVICE}-telemetry"))
            .spawn(move || {
                while let Ok(message) = rx.recv() {
                    match message {
                        Message::Data(body) => {
                            let _ = post(&config, &body);
                        } // Export is best-effort, silent and never a product failure.
                        Message::Flush(done) => {
                            let _ = done.try_send(());
                        }
                    }
                }
            })
            .ok()?;
        Some(Self { tx })
    }
    fn enqueue(&self, body: Vec<u8>) {
        let _ = self.tx.try_send(Message::Data(body));
    }
}
fn exporter() -> Option<&'static Exporter> {
    EXPORTER
        .get_or_init(|| Exporter::new(Config::load()?))
        .as_ref()
}

/// Initialize once outside asynchronous request handling.
pub fn init() {
    let _ = exporter();
}

/// Best effort, bounded enqueue. Strings must be members of fixed product vocabularies.
pub fn record(operation: &str, outcome: &str, duration: Option<f64>) {
    let Some(Some(exporter)) = EXPORTER.get() else {
        return;
    };
    let context = std::env::var("TRACEPARENT").ok();
    if let Some(body) = payload(operation, outcome, duration, context.as_deref()) {
        exporter.enqueue(body);
    }
}
/// Optional CLI shutdown drain. Never starts an exporter, never waits beyond 150 ms.
pub fn flush() {
    let Some(Some(exporter)) = EXPORTER.get() else {
        return;
    };
    let (done, received) = mpsc::sync_channel(1);
    if exporter.tx.try_send(Message::Flush(done)).is_ok() {
        let _ = received.recv_timeout(Duration::from_millis(150));
    }
}
fn trace_context(parent: &str) -> Option<(&str, &str)> {
    if parent.len() != 55 {
        return None;
    }
    let parts: Vec<_> = parent.split('-').collect();
    if parts.len() != 4
        || parts[0] != "00"
        || parts[1].len() != 32
        || parts[2].len() != 16
        || parts[3].len() != 2
    {
        return None;
    }
    if !parts.iter().all(|p| {
        p.bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    }) || parts[1].bytes().all(|b| b == b'0')
        || parts[2].bytes().all(|b| b == b'0')
    {
        return None;
    }
    Some((parts[1], parts[2]))
}
fn payload(
    operation: &str,
    outcome: &str,
    duration: Option<f64>,
    parent: Option<&str>,
) -> Option<Vec<u8>> {
    if !OPERATIONS.contains(&operation)
        || !OUTCOMES.contains(&outcome)
        || duration.is_some_and(|d| !d.is_finite() || d < 0.0)
    {
        return None;
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_nanos();
    let attrs = serde_json::json!([
        {"key":"operation", "value":{"stringValue":operation}},
        {"key":"outcome", "value":{"stringValue":outcome}}
    ]);
    let start = now.saturating_sub(duration.map(|d| (d * 1e9) as u128).unwrap_or(1));
    let mut metrics = vec![
        serde_json::json!({"name":format!("{SERVICE}.operation.count"),"unit":"{operation}","sum":{"aggregationTemporality":1,"isMonotonic":true,"dataPoints":[{"attributes":attrs,"startTimeUnixNano":start.to_string(),"timeUnixNano":now.to_string(),"asInt":"1"}]}}),
    ];
    if let Some(seconds) = duration {
        let mut point = serde_json::json!({"attributes":attrs,"startTimeUnixNano":start.to_string(),"timeUnixNano":now.to_string(),"count":"1","sum":seconds,"min":seconds,"max":seconds,"bucketCounts":["1"],"explicitBounds":[]});
        if let Some((trace, span)) = parent.and_then(trace_context) {
            point["exemplars"] = serde_json::json!([{"timeUnixNano":now.to_string(),"asDouble":seconds,"traceId":trace,"spanId":span}]);
        }
        metrics.push(serde_json::json!({"name":format!("{SERVICE}.operation.duration"),"unit":"s","histogram":{"aggregationTemporality":1,"dataPoints":[point]}}));
    }
    serde_json::to_vec(&serde_json::json!({"resourceMetrics":[{"resource":{"attributes":[{"key":"service.name","value":{"stringValue":SERVICE}}]},"scopeMetrics":[{"scope":{"name":"native-product"},"metrics":metrics}]}]})).ok()
}
fn post(config: &Config, body: &[u8]) -> std::io::Result<()> {
    if body.len() > 8192 {
        return Err(std::io::Error::other("telemetry payload limit"));
    }
    let end = Instant::now() + Duration::from_millis(150);
    let mut socket = TcpStream::connect_timeout(&config.address, DEADLINE)?;
    let mut request = format!(
        "POST /v1/metrics HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", config.address, body.len()
    ).into_bytes();
    request.extend_from_slice(body);
    let mut pending = request.as_slice();
    let budget = || {
        end.checked_duration_since(Instant::now())
            .filter(|d| !d.is_zero())
            .map(|d| d.min(DEADLINE))
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::TimedOut, "telemetry deadline"))
    };
    while !pending.is_empty() {
        socket.set_write_timeout(Some(budget()?))?;
        match socket.write(pending) {
            Ok(0) => return Err(std::io::ErrorKind::WriteZero.into()),
            Ok(count) => pending = &pending[count..],
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    socket.set_read_timeout(Some(budget()?))?;
    let mut response = [0; 256];
    let count = socket.read(&mut response)?;
    if !response[..count].starts_with(b"HTTP/1.1 2")
        && !response[..count].starts_with(b"HTTP/1.0 2")
    {
        return Err(std::io::Error::other("collector refused export"));
    }
    Ok(())
}

/// Runtime duration timer; no arguments or error messages cross this boundary.
pub struct Operation {
    name: &'static str,
    start: Instant,
}
impl Operation {
    pub fn start(name: &'static str) -> Self {
        Self {
            name,
            start: Instant::now(),
        }
    }
    pub fn finish(self, success: bool) {
        record(
            self.name,
            if success { "success" } else { "failure" },
            Some(self.start.elapsed().as_secs_f64()),
        );
    }
}
const OPERATIONS: &[&str] = &[
    "cli.operation",
    "cli.status",
    "cli.run",
    "cli.watch",
    "cli.run_agent",
    "cli.report",
    "cli.trace",
    "cli.serve",
    "cli.advise",
    "cli.assert",
    "cli.issue_draft",
    "cli.insights",
    "cli.storage",
    "cli.extension",
    "http.connection",
];
const OUTCOMES: &[&str] = &[
    "success",
    "failure",
    "complete",
    "idle",
    "cancelled",
    "error",
];
const SERVICE: &str = "dendro";
const OFF_SWITCH: &str = "DENDRO_TELEMETRY_DISABLED";

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    #[test]
    fn config_is_data_loopback_only_and_opt_in() {
        let c = Config::parse("OTEL_EXPORTER_OTLP_ENDPOINT=http://127.0.0.1:24318\nOTEL_EXPORTER_OTLP_PROTOCOL=http/protobuf\nTOKEN=secret\n").unwrap();
        assert_eq!(c.address.port(), 24318);
        for input in [
            "",
            "OTEL_EXPORTER_OTLP_ENDPOINT=https://127.0.0.1:24318",
            "OTEL_EXPORTER_OTLP_ENDPOINT=http://example.org:24318",
            "OTEL_EXPORTER_OTLP_ENDPOINT=http://127.0.0.1:24318/secret",
            "OTEL_EXPORTER_OTLP_ENDPOINT=$(touch /tmp/no)",
            "OTEL_EXPORTER_OTLP_ENDPOINT=http://user:pass@127.0.0.1:24318",
        ] {
            assert!(Config::parse(input).is_none(), "{input}");
        }
        assert!(Config::parse(
            "OTEL_EXPORTER_OTLP_ENDPOINT=http://127.0.0.1:24318\nOTEL_SDK_DISABLED=true"
        )
        .is_none());
        assert!(Config::parse(&format!(
            "OTEL_EXPORTER_OTLP_ENDPOINT=http://127.0.0.1:24318\n{OFF_SWITCH}=1"
        ))
        .is_none());
    }
    #[test]
    fn schema_contains_only_fixed_labels_and_actual_duration() {
        let body = payload("cli.operation", "success", Some(0.25), None).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let resource = &value["resourceMetrics"][0];
        assert_eq!(
            resource["resource"]["attributes"][0]["value"]["stringValue"],
            SERVICE
        );
        let metrics = resource["scopeMetrics"][0]["metrics"].as_array().unwrap();
        assert_eq!(metrics.len(), 2);
        assert_eq!(metrics[0]["sum"]["dataPoints"][0]["asInt"], "1");
        assert_eq!(metrics[1]["histogram"]["dataPoints"][0]["sum"], 0.25);
        assert_eq!(
            metrics[0]["sum"]["dataPoints"][0]["attributes"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert!(payload("private/task/path", "success", None, None).is_none());
        assert!(payload("cli.operation", "secret-message", None, None).is_none());
        assert!(payload("cli.operation", "success", Some(f64::NAN), None).is_none());
        assert!(payload("cli.operation", "success", Some(-1.0), None).is_none());
    }
    #[test]
    fn correlation_accepts_only_valid_nonzero_traceparent() {
        let parent = "00-0123456789abcdef0123456789abcdef-0123456789abcdef-01";
        assert!(trace_context(parent).is_some());
        for bad in [
            "garbage",
            "00-00000000000000000000000000000000-0123456789abcdef-01",
            "00-0123456789abcdef0123456789abcdef-0000000000000000-01",
            "ff-0123456789abcdef0123456789abcdef-0123456789abcdef-01",
        ] {
            assert!(trace_context(bad).is_none());
        }
        let body = payload("cli.operation", "success", Some(0.5), Some(parent)).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            v["resourceMetrics"][0]["scopeMetrics"][0]["metrics"][1]["histogram"]["dataPoints"][0]
                ["exemplars"][0]["traceId"],
            "0123456789abcdef0123456789abcdef"
        );
    }
    #[test]
    fn transport_posts_json_without_headers_or_credentials_from_consumer() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = Vec::new();
            let mut buf = [0; 4096];
            loop {
                let n = socket.read(&mut buf).unwrap();
                if n == 0 {
                    break;
                }
                request.extend_from_slice(&buf[..n]);
                if let Some(split) = request.windows(4).position(|x| x == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&request[..split]);
                    let len: usize = headers
                        .lines()
                        .find_map(|x| x.strip_prefix("Content-Length: "))
                        .unwrap()
                        .parse()
                        .unwrap();
                    if request.len() >= split + 4 + len {
                        break;
                    }
                }
            }
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
                .unwrap();
            request
        });
        let config = Config { address: addr };
        let body = payload("cli.operation", "success", Some(0.1), None).unwrap();
        assert!(post(&config, &body).is_ok());
        let request = String::from_utf8(handle.join().unwrap()).unwrap();
        assert!(request.starts_with("POST /v1/metrics HTTP/1.1\r\n"));
        assert!(request.contains("Content-Type: application/json\r\n"));
        assert!(!request.contains("Authorization"));
    }
    #[test]
    fn refused_and_stalled_export_are_bounded() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let config = Config {
            address: listener.local_addr().unwrap(),
        };
        drop(listener);
        assert!(post(&config, b"{}").is_err());
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let config = Config {
            address: listener.local_addr().unwrap(),
        };
        let handle = std::thread::spawn(move || {
            let (_socket, _) = listener.accept().unwrap();
            std::thread::sleep(Duration::from_millis(250));
        });
        let start = Instant::now();
        assert!(post(&config, b"{}").is_err());
        assert!(start.elapsed() < Duration::from_millis(200));
        handle.join().unwrap();
    }
    #[test]
    fn collector_error_and_oversized_payload_are_refused() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let config = Config {
            address: listener.local_addr().unwrap(),
        };
        let worker = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(1)))
                .unwrap();
            let mut bytes = [0; 1024];
            let _ = socket.read(&mut bytes).unwrap();
            socket
                .write_all(b"HTTP/1.1 503 Unavailable\r\nContent-Length: 0\r\n\r\n")
                .unwrap();
        });
        assert!(post(&config, b"{}").is_err());
        worker.join().unwrap();
        assert!(post(&config, &vec![0; 8193]).is_err());
    }
    #[test]
    fn full_queue_drops_without_waiting() {
        let (tx, _rx) = std::sync::mpsc::sync_channel(1);
        let exporter = Exporter { tx };
        exporter.enqueue(vec![1]);
        let start = Instant::now();
        exporter.enqueue(vec![2]);
        assert!(start.elapsed() < Duration::from_millis(20));
    }
}
