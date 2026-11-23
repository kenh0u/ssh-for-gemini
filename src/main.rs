use std::collections::HashMap;
use std::env;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const MAX_TX_BODY: usize = 1024 * 1024;
const RX_CHUNK_SIZE: usize = 65536;
const RX_COALESCE_MS: u64 = 5;

#[derive(Clone, Debug)]
struct Config {
    listen: String,
    port: u16,
    target_host: String,
    target_port: u16,
    max_sessions: usize,
    idle_timeout: Duration,
    rx_wait: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen: "127.0.0.1".to_string(),
            port: 28787,
            target_host: "127.0.0.1".to_string(),
            target_port: 22,
            max_sessions: 16,
            idle_timeout: Duration::from_secs(180),
            rx_wait: Duration::from_secs(15),
        }
    }
}

struct Session {
    rx_stream: Mutex<TcpStream>,
    tx_stream: Mutex<TcpStream>,
    ctrl_stream: TcpStream,
    last_active: Mutex<Instant>,
    closed: AtomicBool,
}

impl Session {
    fn new(stream: TcpStream) -> io::Result<Self> {
        stream.set_nodelay(true)?;
        let rx_stream = stream.try_clone()?;
        let tx_stream = stream.try_clone()?;
        Ok(Self {
            rx_stream: Mutex::new(rx_stream),
            tx_stream: Mutex::new(tx_stream),
            ctrl_stream: stream,
            last_active: Mutex::new(Instant::now()),
            closed: AtomicBool::new(false),
        })
    }

    fn touch(&self) {
        if let Ok(mut guard) = self.last_active.lock() {
            *guard = Instant::now();
        }
    }

    fn is_stale(&self, idle_timeout: Duration) -> bool {
        if self.closed.load(Ordering::Acquire) {
            return true;
        }
        self.last_active
            .lock()
            .map(|t| t.elapsed() > idle_timeout)
            .unwrap_or(true)
    }

    fn close(&self) {
        if !self.closed.swap(true, Ordering::AcqRel) {
            let _ = self.ctrl_stream.shutdown(Shutdown::Both);
        }
    }
}

struct State {
    config: Config,
    sessions: Mutex<HashMap<String, Arc<Session>>>,
}

impl State {
    fn new(config: Config) -> Self {
        Self {
            config,
            sessions: Mutex::new(HashMap::new()),
        }
    }

    fn get_session(&self, sid: &str) -> Option<Arc<Session>> {
        self.sessions.lock().ok()?.get(sid).cloned()
    }

    fn remove_session(&self, sid: &str) -> Option<Arc<Session>> {
        self.sessions.lock().ok()?.remove(sid)
    }
}

fn random_sid() -> io::Result<String> {
    let mut buf = [0u8; 16];
    let mut f = File::open("/dev/urandom")?;
    f.read_exact(&mut buf)?;
    let mut hex = String::with_capacity(32);
    for b in buf {
        hex.push_str(&format!("{:02x}", b));
    }
    Ok(hex)
}

fn write_response(
    stream: &mut TcpStream,
    status: u16,
    reason: &str,
    content_type: &str,
    body: &[u8],
    keep_alive: bool,
) -> io::Result<()> {
    let conn_hdr = if keep_alive { "keep-alive" } else { "close" };
    let header = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: {conn_hdr}\r\n\r\n",
        body.len()
    );
    stream.write_all(header.as_bytes())?;
    if !body.is_empty() {
        stream.write_all(body)?;
    }
    stream.flush()
}

fn handle_client(mut writer: TcpStream, state: Arc<State>) {
    let _ = writer.set_nodelay(true);
    let _ = writer.set_read_timeout(Some(Duration::from_secs(60)));
    let Ok(reader_sock) = writer.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(reader_sock);

    loop {
        let mut req_line = String::new();
        match reader.read_line(&mut req_line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(_) => break,
        }
        let trimmed = req_line.trim_end_matches(&['\r', '\n'][..]);
        if trimmed.is_empty() {
            continue;
        }
        let mut parts = trimmed.split_whitespace();
        let (Some(method), Some(path), Some(version)) =
            (parts.next(), parts.next(), parts.next())
        else {
            let _ = write_response(&mut writer, 400, "Bad Request", "text/plain", b"bad request\n", false);
            break;
        };
        let method = method.to_string();
        let path = path.to_string();
        let mut keep_alive = version != "HTTP/1.0";
        let mut content_length: usize = 0;

        let mut header_line = String::new();
        let mut header_err = false;
        loop {
            header_line.clear();
            match reader.read_line(&mut header_line) {
                Ok(0) => {
                    header_err = true;
                    break;
                }
                Ok(_) => {}
                Err(_) => {
                    header_err = true;
                    break;
                }
            }
            let line = header_line.trim_end_matches(&['\r', '\n'][..]);
            if line.is_empty() {
                break;
            }
            if let Some((k, v)) = line.split_once(':') {
                let k = k.trim();
                let v = v.trim();
                if k.eq_ignore_ascii_case("content-length") {
                    if let Ok(len) = v.parse::<usize>() {
                        content_length = len;
                    }
                } else if k.eq_ignore_ascii_case("connection") {
                    if v.eq_ignore_ascii_case("close") {
                        keep_alive = false;
                    } else if v.eq_ignore_ascii_case("keep-alive") {
                        keep_alive = true;
                    }
                }
            }
        }
        if header_err {
            break;
        }
        if content_length > MAX_TX_BODY {
            let _ = write_response(
                &mut writer,
                413,
                "Payload Too Large",
                "text/plain",
                b"payload too large\n",
                false,
            );
            break;
        }

        let mut body = vec![0u8; content_length];
        if content_length > 0 && reader.read_exact(&mut body).is_err() {
            break;
        }

        let res = dispatch(&method, &path, &body, &state, &mut writer, keep_alive);
        if res.is_err() || !keep_alive {
            break;
        }
    }
}

fn dispatch(
    method: &str,
    path: &str,
    body: &[u8],
    state: &State,
    writer: &mut TcpStream,
    keep_alive: bool,
) -> io::Result<()> {
    match (method, path) {
        ("GET", "/healthz") => {
            write_response(writer, 200, "OK", "text/plain", b"ok\n", keep_alive)
        }
        ("POST", "/open") => {
            {
                let map = state.sessions.lock().unwrap();
                if map.len() >= state.config.max_sessions {
                    return write_response(
                        writer,
                        503,
                        "Service Unavailable",
                        "text/plain",
                        b"too many sessions\n",
                        keep_alive,
                    );
                }
            }
            let addr_str = format!("{}:{}", state.config.target_host, state.config.target_port);
            let sock = match addr_str
                .to_socket_addrs()
                .ok()
                .and_then(|mut it| it.next())
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid target addr"))
                .and_then(|sa| TcpStream::connect_timeout(&sa, Duration::from_secs(5)))
            {
                Ok(s) => s,
                Err(e) => {
                    let msg = format!("connect failed: {e}\n");
                    return write_response(
                        writer,
                        502,
                        "Bad Gateway",
                        "text/plain",
                        msg.as_bytes(),
                        keep_alive,
                    );
                }
            };

            let sess = match Session::new(sock) {
                Ok(s) => Arc::new(s),
                Err(e) => {
                    let msg = format!("session init failed: {e}\n");
                    return write_response(
                        writer,
                        500,
                        "Internal Server Error",
                        "text/plain",
                        msg.as_bytes(),
                        keep_alive,
                    );
                }
            };
            let sid = random_sid()?;
            state.sessions.lock().unwrap().insert(sid.clone(), sess);
            write_response(writer, 200, "OK", "text/plain", sid.as_bytes(), keep_alive)
        }
        ("POST", p) if p.starts_with("/tx/") => {
            let sid = &p[4..];
            let Some(sess) = state.get_session(sid) else {
                return write_response(writer, 410, "Gone", "text/plain", b"gone\n", keep_alive);
            };
            if sess.closed.load(Ordering::Acquire) {
                return write_response(writer, 410, "Gone", "text/plain", b"gone\n", keep_alive);
            }
            sess.touch();
            let write_res = {
                let mut tx = sess.tx_stream.lock().unwrap();
                tx.write_all(body)
            };
            if write_res.is_err() {
                sess.close();
                state.remove_session(sid);
                return write_response(writer, 410, "Gone", "text/plain", b"closed\n", keep_alive);
            }
            write_response(writer, 204, "No Content", "application/octet-stream", b"", keep_alive)
        }
        ("GET", p) if p.starts_with("/rx/") => {
            let sid = &p[4..];
            let Some(sess) = state.get_session(sid) else {
                return write_response(writer, 410, "Gone", "text/plain", b"gone\n", keep_alive);
            };
            if sess.closed.load(Ordering::Acquire) {
                return write_response(writer, 410, "Gone", "text/plain", b"gone\n", keep_alive);
            }
            sess.touch();

            let mut rx = sess.rx_stream.lock().unwrap();
            let mut out = Vec::new();
            let mut tmp = [0u8; 16384];

            let _ = rx.set_read_timeout(Some(state.config.rx_wait));
            match rx.read(&mut tmp) {
                Ok(0) => {
                    drop(rx);
                    sess.close();
                    state.remove_session(sid);
                    return write_response(writer, 410, "Gone", "text/plain", b"eof\n", keep_alive);
                }
                Ok(n) => {
                    out.extend_from_slice(&tmp[..n]);
                }
                Err(e)
                    if e.kind() == io::ErrorKind::WouldBlock
                        || e.kind() == io::ErrorKind::TimedOut =>
                {
                    sess.touch();
                    return write_response(
                        writer,
                        204,
                        "No Content",
                        "application/octet-stream",
                        b"",
                        keep_alive,
                    );
                }
                Err(_) => {
                    drop(rx);
                    sess.close();
                    state.remove_session(sid);
                    return write_response(writer, 410, "Gone", "text/plain", b"gone\n", keep_alive);
                }
            }

            // Brief coalesce for back-to-back SSH packets
            let _ = rx.set_read_timeout(Some(Duration::from_millis(RX_COALESCE_MS)));
            while out.len() < RX_CHUNK_SIZE {
                let cap = (RX_CHUNK_SIZE - out.len()).min(tmp.len());
                match rx.read(&mut tmp[..cap]) {
                    Ok(0) => break,
                    Ok(n) => out.extend_from_slice(&tmp[..n]),
                    Err(_) => break,
                }
            }

            sess.touch();
            write_response(writer, 200, "OK", "application/octet-stream", &out, keep_alive)
        }
        ("POST", p) if p.starts_with("/close/") => {
            let sid = &p[7..];
            if let Some(sess) = state.remove_session(sid) {
                sess.close();
            }
            write_response(writer, 204, "No Content", "application/octet-stream", b"", keep_alive)
        }
        _ => write_response(writer, 404, "Not Found", "text/plain", b"not found\n", keep_alive),
    }
}

fn spawn_sweeper(state: Arc<State>) {
    thread::spawn(move || loop {
        thread::sleep(Duration::from_secs(30));
        let stale: Vec<Arc<Session>> = {
            let mut map = state.sessions.lock().unwrap();
            let keys: Vec<String> = map
                .iter()
                .filter(|(_, s)| s.is_stale(state.config.idle_timeout))
                .map(|(k, _)| k.clone())
                .collect();
            keys.into_iter().filter_map(|k| map.remove(&k)).collect()
        };
        for s in stale {
            s.close();
        }
    });
}

fn serve_listener(listener: TcpListener, state: Arc<State>) -> io::Result<()> {
    spawn_sweeper(Arc::clone(&state));
    for conn in listener.incoming() {
        match conn {
            Ok(stream) => {
                let st = Arc::clone(&state);
                thread::spawn(move || handle_client(stream, st));
            }
            Err(e) => {
                eprintln!("accept error: {e}");
            }
        }
    }
    Ok(())
}

fn parse_args() -> Result<Config, String> {
    let mut cfg = Config::default();
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--listen" => {
                cfg.listen = args.next().ok_or("--listen requires a value")?;
            }
            "--port" => {
                let v = args.next().ok_or("--port requires a value")?;
                cfg.port = v.parse().map_err(|_| format!("invalid --port: {v}"))?;
            }
            "--target-host" => {
                cfg.target_host = args.next().ok_or("--target-host requires a value")?;
            }
            "--target-port" => {
                let v = args.next().ok_or("--target-port requires a value")?;
                cfg.target_port = v.parse().map_err(|_| format!("invalid --target-port: {v}"))?;
            }
            "--max-sessions" => {
                let v = args.next().ok_or("--max-sessions requires a value")?;
                cfg.max_sessions = v.parse().map_err(|_| format!("invalid --max-sessions: {v}"))?;
            }
            "--idle-timeout" => {
                let v = args.next().ok_or("--idle-timeout requires a value")?;
                let secs: u64 = v.parse().map_err(|_| format!("invalid --idle-timeout: {v}"))?;
                cfg.idle_timeout = Duration::from_secs(secs);
            }
            "--rx-wait" => {
                let v = args.next().ok_or("--rx-wait requires a value")?;
                let secs: u64 = v.parse().map_err(|_| format!("invalid --rx-wait: {v}"))?;
                cfg.rx_wait = Duration::from_secs(secs);
            }
            "-h" | "--help" => {
                println!(
                    "ssh-for-gemini {}\n\nOPTIONS:\n  --listen <addr>        Bind address (default: 127.0.0.1)\n  --port <port>          Bind port (default: 28787)\n  --target-host <host>   Target TCP host (default: 127.0.0.1)\n  --target-port <port>   Target TCP port (default: 22)\n  --max-sessions <n>     Max concurrent sessions (default: 16)\n  --idle-timeout <sec>   Idle session timeout (default: 180)\n  --rx-wait <sec>        Long-poll RX wait seconds (default: 15)",
                    env!("CARGO_PKG_VERSION")
                );
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(cfg)
}

fn main() {
    let cfg = match parse_args() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(2);
        }
    };
    let bind_addr: SocketAddr = format!("{}:{}", cfg.listen, cfg.port)
        .parse()
        .expect("invalid listen address");
    let listener = TcpListener::bind(bind_addr).expect("failed to bind listener");
    eprintln!(
        "ssh-for-gemini {} listening on http://{} -> {}:{}",
        env!("CARGO_PKG_VERSION"),
        bind_addr,
        cfg.target_host,
        cfg.target_port
    );
    let state = Arc::new(State::new(cfg));
    if let Err(e) = serve_listener(listener, state) {
        eprintln!("server error: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn http_req(
        stream: &mut TcpStream,
        method: &str,
        path: &str,
        body: &[u8],
    ) -> (u16, Vec<u8>) {
        let req = format!(
            "{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: keep-alive\r\nContent-Length: {}\r\n\r\n",
            body.len()
        );
        stream.write_all(req.as_bytes()).unwrap();
        if !body.is_empty() {
            stream.write_all(body).unwrap();
        }
        stream.flush().unwrap();

        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut status_line = String::new();
        reader.read_line(&mut status_line).unwrap();
        let status: u16 = status_line
            .split_whitespace()
            .nth(1)
            .unwrap()
            .parse()
            .unwrap();
        let mut cl = 0usize;
        let mut line = String::new();
        loop {
            line.clear();
            reader.read_line(&mut line).unwrap();
            let t = line.trim_end_matches(&['\r', '\n'][..]);
            if t.is_empty() {
                break;
            }
            if let Some((k, v)) = t.split_once(':') {
                if k.trim().eq_ignore_ascii_case("content-length") {
                    cl = v.trim().parse().unwrap();
                }
            }
        }
        let mut resp_body = vec![0u8; cl];
        if cl > 0 {
            reader.read_exact(&mut resp_body).unwrap();
        }
        (status, resp_body)
    }

    #[test]
    fn full_lifecycle_echo_and_eof() {
        let echo_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let echo_port = echo_listener.local_addr().unwrap().port();
        thread::spawn(move || {
            let (mut conn, _) = echo_listener.accept().unwrap();
            conn.write_all(b"SSH-2.0-RustBridgeTest\r\n").unwrap();
            let mut buf = [0u8; 1024];
            let n = conn.read(&mut buf).unwrap();
            let mut reply = b"ECHO:".to_vec();
            reply.extend_from_slice(&buf[..n]);
            conn.write_all(&reply).unwrap();
        });

        let bridge_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let bridge_addr = bridge_listener.local_addr().unwrap();
        let cfg = Config {
            target_port: echo_port,
            rx_wait: Duration::from_secs(1),
            ..Config::default()
        };
        let state = Arc::new(State::new(cfg));
        thread::spawn(move || {
            let _ = serve_listener(bridge_listener, state);
        });

        let mut client = TcpStream::connect(bridge_addr).unwrap();
        let (st, body) = http_req(&mut client, "GET", "/healthz", b"");
        assert_eq!(st, 200);
        assert_eq!(body, b"ok\n");

        let (st, sid_bytes) = http_req(&mut client, "POST", "/open", b"");
        assert_eq!(st, 200);
        let sid = String::from_utf8(sid_bytes).unwrap();
        assert_eq!(sid.len(), 32);

        let (st, banner) = http_req(&mut client, "GET", &format!("/rx/{sid}"), b"");
        assert_eq!(st, 200);
        assert_eq!(banner, b"SSH-2.0-RustBridgeTest\r\n");

        let (st, _) = http_req(&mut client, "POST", &format!("/tx/{sid}"), b"ping-from-client");
        assert_eq!(st, 204);

        let (st, echoed) = http_req(&mut client, "GET", &format!("/rx/{sid}"), b"");
        assert_eq!(st, 200);
        assert_eq!(echoed, b"ECHO:ping-from-client");

        // Next RX should observe EOF -> 410 Gone
        let (st, _) = http_req(&mut client, "GET", &format!("/rx/{sid}"), b"");
        assert_eq!(st, 410);
    }
}
