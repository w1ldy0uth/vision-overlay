//! Loopback-only, read-only panel. The capture writes JSONL; this process tails it.
#[path = "../event_format.rs"]
mod event_format;
#[path = "../live_panel.rs"]
mod live_panel;
use anyhow::{Context, Result, bail};
use clap::Parser;
use serde_json::Value;
use std::{
    fs::File,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Parser)]
#[command(about = "Serve a read-only live inventory panel on 127.0.0.1")]
struct Args {
    input: PathBuf,
    #[arg(long, default_value = "local-data/materials.ru.json")]
    catalog: PathBuf,
    #[arg(long,default_value_t=18787,value_parser=clap::value_parser!(u16).range(1..))]
    port: u16,
    #[arg(long, default_value = "standalone")]
    instance: String,
    #[arg(long, default_value = "local-data/avatars.ru.json")]
    avatar_catalog: PathBuf,
    #[arg(long, default_value = "local-data/achievements.ru.json")]
    achievement_catalog: PathBuf,
}

fn response(stream: &mut TcpStream, status: &str, kind: &str, body: &[u8]) -> Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nContent-Security-Policy: default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; connect-src 'self'; frame-ancestors 'none'\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)?;
    Ok(())
}
fn serve(
    mut stream: TcpStream,
    panel: &Mutex<live_panel::Panel>,
    port: u16,
    instance: &str,
) -> Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(1)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 1024];
    while !bytes.windows(4).any(|w| w == b"\r\n\r\n") {
        let len = stream.read(&mut buffer)?;
        if len == 0 {
            return Ok(());
        }
        bytes.extend_from_slice(&buffer[..len]);
        if bytes.len() > 8192 {
            return response(
                &mut stream,
                "431 Request Header Fields Too Large",
                "text/plain",
                b"Headers too large",
            );
        }
    }
    let request = std::str::from_utf8(&bytes)?;
    let expected = format!("127.0.0.1:{port}");
    let host = request.lines().skip(1).find_map(|line| {
        line.split_once(':')
            .filter(|(name, _)| name.eq_ignore_ascii_case("host"))
            .map(|(_, value)| value.trim())
    });
    if host != Some(expected.as_str()) {
        return response(
            &mut stream,
            "403 Forbidden",
            "text/plain",
            b"Loopback host required",
        );
    }
    let first = request.lines().next().unwrap_or("");
    let mut parts = first.split_whitespace();
    if parts.next() != Some("GET") {
        return response(
            &mut stream,
            "405 Method Not Allowed",
            "text/plain",
            b"Read only",
        );
    }
    let path = parts.next().unwrap_or("");
    match path {
        "/" => response(
            &mut stream,
            "200 OK",
            "text/html; charset=utf-8",
            include_bytes!("../../assets/live-panel.html"),
        ),
        "/panel.js" => response(
            &mut stream,
            "200 OK",
            "text/javascript; charset=utf-8",
            include_bytes!("../../assets/live-panel.js"),
        ),
        _ if path == "/api/state" || path.starts_with("/api/state?after=") => {
            let state = panel.lock().unwrap();
            let after = path
                .strip_prefix("/api/state?after=")
                .and_then(|v| v.parse::<u64>().ok());
            if after == Some(state.revision) {
                return response(&mut stream, "204 No Content", "application/json", b"");
            }
            let mut report = state.report();
            report["instance"] = serde_json::json!(instance);
            let body = serde_json::to_vec(&report)?;
            drop(state);
            response(
                &mut stream,
                "200 OK",
                "application/json; charset=utf-8",
                &body,
            )
        }
        _ => response(&mut stream, "404 Not Found", "text/plain", b"Not found"),
    }
}

fn tail(path: PathBuf, panel: Arc<Mutex<live_panel::Panel>>) -> Result<()> {
    let mut file = loop {
        match File::open(&path) {
            Ok(file) => break file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                std::thread::sleep(Duration::from_millis(100))
            }
            Err(e) => return Err(e.into()),
        }
    };
    let mut pending = Vec::new();
    let mut buffer = [0u8; 65536];
    let mut last_event = Instant::now();
    loop {
        let len = file.read(&mut buffer)?;
        if len > 0 {
            pending.extend_from_slice(&buffer[..len]);
            while let Some(end) = pending.iter().position(|b| *b == b'\n') {
                if end > 4 * 1024 * 1024 {
                    bail!("Capture event too large");
                }
                let line: Vec<_> = pending.drain(..=end).collect();
                if line.iter().all(u8::is_ascii_whitespace) {
                    continue;
                }
                let event: Value = serde_json::from_slice(&line)?;
                panel.lock().unwrap().apply(&event)?;
                last_event = Instant::now();
            }
            if pending.len() > 4 * 1024 * 1024 {
                bail!("Unterminated capture event too large");
            }
        } else {
            let mut state = panel.lock().unwrap();
            if last_event.elapsed() > Duration::from_secs(8) {
                state.heartbeat_expired();
            }
            if state.stopped() {
                return Ok(());
            }
            drop(state);
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}
fn main() -> Result<()> {
    let args = Args::parse();
    let catalog: Value = serde_json::from_slice(
        &std::fs::read(&args.catalog).context("Cannot read material catalog")?,
    )?;
    let mut names: std::collections::HashMap<String, String> =
        serde_json::from_value(catalog["names"].clone())?;
    names.extend(serde_json::from_value::<
        std::collections::HashMap<String, String>,
    >(catalog["item_names"].clone())?);
    let mut skills = std::collections::HashMap::new();
    if args.avatar_catalog.exists() {
        let avatars: Value = serde_json::from_slice(&std::fs::read(&args.avatar_catalog)?)?;
        names.extend(serde_json::from_value::<
            std::collections::HashMap<String, String>,
        >(avatars["names"].clone())?);
        if avatars["skill_names"].is_object() {
            skills.extend(serde_json::from_value::<
                std::collections::HashMap<String, String>,
            >(avatars["skill_names"].clone())?);
        }
    }
    let panel = Arc::new(Mutex::new(
        live_panel::Panel::new(names)
            .with_skills(skills)
            .with_achievements(if args.achievement_catalog.exists() {
                let catalog: Value =
                    serde_json::from_slice(&std::fs::read(&args.achievement_catalog)?)?;
                serde_json::from_value(catalog["records"].clone())?
            } else {
                Default::default()
            }),
    ));
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, args.port))
        .context("Cannot open local panel port")?;
    println!("Live panel: http://127.0.0.1:{}/", args.port);
    let state = panel.clone();
    std::thread::spawn(move || {
        if let Err(error) = tail(args.input, state.clone()) {
            eprintln!("Panel reader: {error:#}");
            state
                .lock()
                .unwrap()
                .unavailable("error", "Ошибка чтения журнала. Проверьте файл захвата");
        }
    });
    let (sender, receiver) = std::sync::mpsc::sync_channel(16);
    let receiver = Arc::new(Mutex::new(receiver));
    for _ in 0..4 {
        let receiver = receiver.clone();
        let state = panel.clone();
        let port = args.port;
        let instance = args.instance.clone();
        std::thread::spawn(move || {
            loop {
                let Ok(stream) = receiver.lock().unwrap().recv() else {
                    break;
                };
                let _ = serve(stream, &state, port, &instance);
            }
        });
    }
    for stream in listener.incoming() {
        let stream = stream?;
        // A bounded worker pool prevents abandoned browser connections from
        // creating unbounded threads. Overload closes the connection for retry.
        let _ = sender.try_send(stream);
    }
    Ok(())
}

#[cfg(test)]
mod http_tests {
    use super::*;
    fn request(method: &str, path: &str, host: Option<&str>) -> String {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let worker = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let panel = Mutex::new(live_panel::Panel::new(std::collections::HashMap::new()));
            serve(stream, &panel, port, "test-instance").unwrap();
        });
        let mut stream = TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port)).unwrap();
        let host = host
            .map(str::to_owned)
            .unwrap_or_else(|| format!("127.0.0.1:{port}"));
        write!(
            stream,
            "{method} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        worker.join().unwrap();
        response
    }
    #[test]
    fn read_only_routes_revision_and_local_host_are_enforced() {
        let response = request("GET", "/api/state", None);
        assert!(response.starts_with("HTTP/1.1 200"));
        assert!(response.contains("test-instance"));
        assert!(request("GET", "/api/state?after=1", None).starts_with("HTTP/1.1 204"));
        assert!(request("POST", "/api/state", None).starts_with("HTTP/1.1 405"));
        assert!(request("GET", "/api/state", Some("foreign.example")).starts_with("HTTP/1.1 403"));
        assert!(request("GET", "/../local-data/gi.json", None).starts_with("HTTP/1.1 404"));
    }
}
