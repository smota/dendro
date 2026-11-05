use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

struct Collector {
    address: std::net::SocketAddr,
    stop: Arc<AtomicBool>,
    handle: std::thread::JoinHandle<Vec<serde_json::Value>>,
}
impl Collector {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let handle = std::thread::spawn(move || {
            let mut bodies = Vec::new();
            while !stopped.load(Ordering::Relaxed) {
                let (mut socket, _) = match listener.accept() {
                    Ok(value) => value,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(1));
                        continue;
                    }
                    Err(e) => panic!("collector: {e}"),
                };
                socket
                    .set_read_timeout(Some(Duration::from_secs(1)))
                    .unwrap();
                let mut request = Vec::new();
                let mut chunk = [0; 4096];
                loop {
                    let n = socket.read(&mut chunk).unwrap();
                    assert_ne!(n, 0);
                    request.extend_from_slice(&chunk[..n]);
                    if let Some(split) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                        let header = String::from_utf8_lossy(&request[..split]);
                        assert!(header.contains("Content-Type: application/json"));
                        let length: usize = header
                            .lines()
                            .find_map(|l| l.strip_prefix("Content-Length: "))
                            .unwrap()
                            .parse()
                            .unwrap();
                        if request.len() >= split + 4 + length {
                            bodies.push(
                                serde_json::from_slice(&request[split + 4..split + 4 + length])
                                    .unwrap(),
                            );
                            break;
                        }
                    }
                }
                socket
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
                    .unwrap();
            }
            bodies
        });
        Self {
            address,
            stop,
            handle,
        }
    }
    fn finish(self) -> Vec<serde_json::Value> {
        self.stop.store(true, Ordering::Relaxed);
        self.handle.join().unwrap()
    }
}
fn config(tag: &str, address: std::net::SocketAddr) -> PathBuf {
    let root = std::env::temp_dir().join(format!("native-telemetry-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(root.join("observability")).unwrap();
    std::fs::write(root.join("observability/consumer.env"),format!("OTEL_EXPORTER_OTLP_ENDPOINT=http://{address}\nOTEL_EXPORTER_OTLP_PROTOCOL=http/protobuf\nOTEL_EXPORTER_OTLP_HEADERS=do-not-export\n")).unwrap();
    root
}
fn metrics(bodies: &[serde_json::Value]) -> Vec<&serde_json::Value> {
    bodies
        .iter()
        .flat_map(|b| {
            b["resourceMetrics"][0]["scopeMetrics"][0]["metrics"]
                .as_array()
                .unwrap()
        })
        .collect()
}
fn has(bodies: &[serde_json::Value], operation: &str, outcome: &str) -> bool {
    metrics(bodies).iter().any(|m| {
        m["sum"]["dataPoints"][0]["attributes"][0]["value"]["stringValue"] == operation
            && m["sum"]["dataPoints"][0]["attributes"][1]["value"]["stringValue"] == outcome
    })
}
fn command(root: &Path) -> Command {
    let mut c = Command::new(BINARY);
    c.env("XDG_CONFIG_HOME", root)
        .env_remove(OFF_SWITCH)
        .env_remove("OTEL_SDK_DISABLED");
    c.current_dir(root);
    c
}

const BINARY: &str = env!("CARGO_BIN_EXE_dendro");
const OFF_SWITCH: &str = "DENDRO_TELEMETRY_DISABLED";
fn output(c: &mut Command) -> Output {
    c.output().unwrap()
}
#[test]
fn failed_native_cli_operation_is_safe_opt_in_and_fail_open() {
    let collector = Collector::start();
    let root = config("dendro-cli", collector.address);
    let result = output(
        command(&root)
            .args(["status", "--storage-root"])
            .arg(root.join("private-nonexistent-root")),
    );
    assert!(!result.status.success());
    let bodies = collector.finish();
    assert!(has(&bodies, "cli.status", "failure"));
    assert!(metrics(&bodies)
        .iter()
        .any(|m| m["histogram"]["dataPoints"][0]["sum"]
            .as_f64()
            .is_some_and(|d| d >= 0.0)));
    let text = serde_json::to_string(&bodies).unwrap();
    for private in ["private-nonexistent-root", "do-not-export", "storage-root"] {
        assert!(!text.contains(private));
    }
    let collector = Collector::start();
    let off = config("dendro-off", collector.address);
    let disabled = output(
        command(&off)
            .env(OFF_SWITCH, "true")
            .args(["status", "--storage-root"])
            .arg(off.join("private-nonexistent-root")),
    );
    assert_eq!(result.status.code(), disabled.status.code());
    assert!(collector.finish().is_empty());
    let before = std::time::Instant::now();
    let refused = output(
        command(&off)
            .args(["status", "--storage-root"])
            .arg(off.join("private-nonexistent-root")),
    );
    assert_eq!(result.status.code(), refused.status.code());
    assert!(before.elapsed() < Duration::from_secs(2));
    std::fs::remove_dir_all(root).unwrap();
    std::fs::remove_dir_all(off).unwrap();
}

#[test]
fn native_http_fixture_helper() {
    if std::env::var_os("DENDRO_TELEMETRY_HTTP_FIXTURE").is_none() {
        return;
    }
    let root = std::env::current_dir().unwrap();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let server = dendro_cli::connected_web::ConnectedServer::bind(
            "127.0.0.1:0",
            root.join("unused.duckdb"),
            root.clone(),
            None,
            None,
        )
        .await
        .unwrap();
        let address = server.local_addr().unwrap();
        let worker = tokio::spawn(server.run());
        let mut socket = tokio::net::TcpStream::connect(address).await.unwrap();
        socket
            .write_all(b"GET /?private-session-content HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .await
            .unwrap();
        let mut response = String::new();
        tokio::time::timeout(Duration::from_secs(2), socket.read_to_string(&mut response))
            .await
            .unwrap()
            .unwrap();
        assert!(response.starts_with("HTTP/1.1 302"));
        worker.abort();
        let _ = worker.await;
        tokio::task::spawn_blocking(dendro_cli::telemetry::flush)
            .await
            .unwrap();
    });
}

#[test]
fn native_http_handler_exports_runtime_without_request_content() {
    let collector = Collector::start();
    let root = config("dendro-http", collector.address);
    let result = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "native_http_fixture_helper", "--nocapture"])
        .env("DENDRO_TELEMETRY_HTTP_FIXTURE", "1")
        .env("XDG_CONFIG_HOME", &root)
        .env_remove(OFF_SWITCH)
        .env_remove("OTEL_SDK_DISABLED")
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let bodies = collector.finish();
    assert!(has(&bodies, "http.connection", "success"));
    assert!(!serde_json::to_string(&bodies)
        .unwrap()
        .contains("private-session-content"));
    std::fs::remove_dir_all(root).unwrap();
}
