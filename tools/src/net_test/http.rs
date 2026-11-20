//! The responder of `/metrics`: a minimal HTTP/1.0 server on one thread that serves the text of a closure. It answers
//! one request at a time, GET and HEAD of `/metrics` only, and never waits for a silent client longer than the
//! deadline of the request head. The closure it serves must not call into the isolate.

use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::exposition::CONTENT_TYPE;

/// The time a connection has to send its whole request head.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(2);
/// How often the accept loop and the read of a request head look at the stop.
const POLL: Duration = Duration::from_millis(100);
/// The largest request head that the responder reads.
const MAX_REQUEST: usize = 8192;
/// The content type of the answers that are not the metrics.
const PLAIN_TYPE: &str = "text/plain; charset=utf-8";

/// The server of `/metrics`, bound at the start and served on its own thread until it is stopped.
pub struct MetricsServer {
    port: u16,
    stopping: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl MetricsServer {
    /// Binds the host (an IPv6 literal may come in brackets) and the port (0 takes a free one) and serves what `body`
    /// returns for every request on a new thread. The error is the text after `NetTest: `.
    pub fn start(host: &str, port: u16, body: impl Fn() -> String + Send + 'static) -> Result<Self, String> {
        let name = host
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
            .filter(|name| !name.is_empty())
            .unwrap_or(host);
        let addresses: Vec<_> = (name, port)
            .to_socket_addrs()
            .map_err(|_| format!("cannot resolve the host '{host}' of --metrics-port"))?
            .collect();
        if addresses.is_empty() {
            return Err(format!("cannot resolve the host '{host}' of --metrics-port"));
        }
        let listener = TcpListener::bind(&addresses[..])
            .map_err(|_| format!("cannot listen on {host}:{port} for /metrics (is the port in use?)"))?;
        // The accept loop polls, so that the stop is prompt.
        listener
            .set_nonblocking(true)
            .map_err(|error| format!("cannot serve /metrics on {host}:{port}: {error}"))?;
        let port = listener
            .local_addr()
            .map_err(|error| format!("cannot serve /metrics on {host}:{port}: {error}"))?
            .port();
        let stopping = Arc::new(AtomicBool::new(false));
        let thread = std::thread::Builder::new()
            .name("nettest-metrics".to_owned())
            .spawn({
                let stopping = Arc::clone(&stopping);
                move || serve(&listener, &body, &stopping)
            })
            .map_err(|error| format!("cannot start the thread of /metrics: {error}"))?;
        Ok(MetricsServer {
            port,
            stopping,
            thread: Some(thread),
        })
    }

    /// The bound port, the one that was taken for port 0.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Stops the thread within a poll, whether or not a client is connected, and closes the socket.
    pub fn stop(&mut self) {
        self.stopping.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            thread.thread().unpark();
            let _ = thread.join();
        }
    }
}

impl Drop for MetricsServer {
    fn drop(&mut self) {
        self.stop();
    }
}

fn serve(listener: &TcpListener, body: &dyn Fn() -> String, stopping: &AtomicBool) {
    while !stopping.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((client, _)) => answer(client, body, stopping),
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            // Nothing to accept (or a failed accept): wait a poll, or until the stop wakes the thread.
            Err(_) => std::thread::park_timeout(POLL),
        }
    }
}

fn response(status: &str, content_type: &str, body: &str) -> String {
    format!(
        "HTTP/1.0 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

/// Reads the request head within one deadline for the whole head, so that a slow client holds neither the responder
/// nor the stop for longer than that; `None` when the deadline or the stop came first.
fn read_head(client: &mut TcpStream, stopping: &AtomicBool) -> Option<Vec<u8>> {
    let deadline = Instant::now() + REQUEST_TIMEOUT;
    let mut request = Vec::new();
    let mut buffer = [0_u8; 1024];
    while !ends_head(&request) && request.len() < MAX_REQUEST {
        let left = deadline.saturating_duration_since(Instant::now());
        if stopping.load(Ordering::SeqCst) || left.is_zero() {
            return None;
        }
        client.set_read_timeout(Some(left.min(POLL))).ok()?;
        match client.read(&mut buffer) {
            Ok(0) => break,
            Ok(received) => request.extend_from_slice(&buffer[..received]),
            Err(error)
                if matches!(
                    error.kind(),
                    ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
                ) => {}
            Err(_) => break,
        }
    }
    Some(request)
}

fn ends_head(request: &[u8]) -> bool {
    request.windows(4).any(|window| window == b"\r\n\r\n") || request.windows(2).any(|window| window == b"\n\n")
}

fn answer(mut client: TcpStream, body: &dyn Fn() -> String, stopping: &AtomicBool) {
    // A client of a non-blocking listener may inherit its mode.
    if client.set_nonblocking(false).is_err() {
        return;
    }
    let Some(request) = read_head(&mut client, stopping) else {
        return;
    };
    let request = String::from_utf8_lossy(&request);
    let line = request.split(['\r', '\n']).next().unwrap_or_default();
    let Some((method, rest)) = line.split_once(' ') else {
        return;
    };
    let target = rest.split(' ').next().unwrap_or_default();
    let target = target.split('?').next().unwrap_or_default();
    let text = if method != "GET" && method != "HEAD" {
        response("405 Method Not Allowed", PLAIN_TYPE, "GET only\n")
    } else if target != "/metrics" {
        response("404 Not Found", PLAIN_TYPE, "the metrics are at /metrics\n")
    } else {
        let mut text = response("200 OK", CONTENT_TYPE, &body());
        if method == "HEAD" {
            text.truncate(text.find("\r\n\r\n").map_or(text.len(), |end| end + 4));
        }
        text
    };
    let _ = client.set_write_timeout(Some(REQUEST_TIMEOUT));
    let _ = client.write_all(text.as_bytes());
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpStream;

    use super::*;

    fn connect(port: u16) -> TcpStream {
        TcpStream::connect(("127.0.0.1", port)).unwrap()
    }

    /// Sends the request and reads the answer until the server closes the connection.
    fn exchange(port: u16, request: &str) -> String {
        let mut client = connect(port);
        client.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        client.write_all(request.as_bytes()).unwrap();
        let mut bytes = Vec::new();
        let _ = client.read_to_end(&mut bytes);
        String::from_utf8_lossy(&bytes).into_owned()
    }

    fn server() -> MetricsServer {
        MetricsServer::start("127.0.0.1", 0, || "nettest_test 1\n".to_owned()).unwrap()
    }

    /// True when the server closed the connection within the time: a read gives the end or an error.
    fn closed_within(client: &mut TcpStream, time: Duration) -> bool {
        client.set_read_timeout(Some(time)).unwrap();
        let mut buffer = [0_u8; 256];
        match client.read(&mut buffer) {
            Ok(0) => true,
            Ok(_) => false,
            Err(error) => !matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut),
        }
    }

    #[test]
    fn the_responder_serves_get_with_the_content_type_and_the_length() {
        let server = server();
        assert!(server.port() > 0);
        let metrics = exchange(server.port(), "GET /metrics HTTP/1.0\r\nHost: localhost\r\n\r\n");
        assert!(metrics.starts_with("HTTP/1.0 200 OK\r\n"), "{metrics}");
        assert!(
            metrics.contains("\r\nContent-Type: text/plain; version=0.0.4; charset=utf-8\r\n"),
            "{metrics}"
        );
        assert!(metrics.contains("\r\nContent-Length: 15\r\n"), "{metrics}");
        assert!(metrics.ends_with("\r\n\r\nnettest_test 1\n"), "{metrics}");
        // A query string does not matter, and a head that ends with two line feeds is a head.
        assert!(exchange(server.port(), "GET /metrics?x=1 HTTP/1.1\n\n").starts_with("HTTP/1.0 200 OK\r\n"));
    }

    #[test]
    fn head_has_the_headers_of_get_and_no_body() {
        let server = server();
        let head = exchange(server.port(), "HEAD /metrics HTTP/1.0\r\n\r\n");
        assert!(head.starts_with("HTTP/1.0 200 OK\r\n"), "{head}");
        assert!(head.contains("Content-Length: 15\r\n"), "{head}");
        assert!(head.ends_with("\r\n\r\n"), "{head}");
        assert!(!head.contains("nettest_test"), "{head}");
    }

    #[test]
    fn other_paths_are_404_and_other_methods_405() {
        let server = server();
        let missing = exchange(server.port(), "GET /other HTTP/1.0\r\n\r\n");
        assert!(missing.starts_with("HTTP/1.0 404 Not Found\r\n"), "{missing}");
        assert!(
            missing.contains("Content-Type: text/plain; charset=utf-8\r\n"),
            "{missing}"
        );
        assert!(missing.ends_with("\r\n\r\nthe metrics are at /metrics\n"), "{missing}");
        let method = exchange(server.port(), "POST /metrics HTTP/1.0\r\n\r\n");
        assert!(method.starts_with("HTTP/1.0 405 Method Not Allowed\r\n"), "{method}");
        assert!(method.ends_with("\r\n\r\nGET only\n"), "{method}");
    }

    #[test]
    fn a_port_in_use_cannot_be_bound() {
        let server = server();
        let busy = MetricsServer::start("127.0.0.1", server.port(), String::new);
        let error = busy.err().expect("the port is taken");
        assert_eq!(
            error,
            format!(
                "cannot listen on 127.0.0.1:{} for /metrics (is the port in use?)",
                server.port()
            )
        );
    }

    #[test]
    fn a_host_that_does_not_resolve_is_an_error() {
        let error = MetricsServer::start("no-such-host.invalid", 0, String::new)
            .err()
            .unwrap();
        assert_eq!(
            error,
            "cannot resolve the host 'no-such-host.invalid' of --metrics-port"
        );
    }

    #[test]
    fn a_client_that_sends_its_request_a_byte_at_a_time_is_closed_at_the_deadline_of_the_whole_request() {
        let server = server();
        let mut client = connect(server.port());
        // A byte every 500 ms keeps a timeout per read alive forever; the deadline of the request closes it.
        let request = b"GET /metrics HTTP/1.0\r\n";
        let start = Instant::now();
        let mut closed = false;
        for byte in request {
            if client.write_all(&[*byte]).is_err() {
                closed = true;
                break;
            }
            closed = closed_within(&mut client, Duration::from_millis(500));
            if closed {
                break;
            }
        }
        assert!(closed, "the server kept the slow client");
        assert!(start.elapsed() < Duration::from_secs(5), "{:?}", start.elapsed());
    }

    #[test]
    fn the_stop_does_not_wait_for_a_client_that_sends_nothing() {
        let mut server = server();
        let mut client = connect(server.port());
        // The responder has accepted the connection and waits for its request.
        std::thread::sleep(Duration::from_millis(300));
        let start = Instant::now();
        server.stop();
        let elapsed = start.elapsed();
        assert!(closed_within(&mut client, Duration::from_secs(1)));
        // The deadline of the request is 2 s; the stop is seen within a poll.
        assert!(elapsed < Duration::from_millis(1000), "{elapsed:?}");
    }

    #[test]
    fn the_stop_is_prompt_without_clients_and_repeatable() {
        let mut server = server();
        let start = Instant::now();
        server.stop();
        server.stop();
        assert!(start.elapsed() < Duration::from_millis(1000), "{:?}", start.elapsed());
        assert!(TcpStream::connect(("127.0.0.1", server.port())).is_err());
    }
}
