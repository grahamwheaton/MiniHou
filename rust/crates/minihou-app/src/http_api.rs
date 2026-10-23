//! Loopback HTTP transport. Socket threads never own or mutate the editor document.
use crate::Result;
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    time::Duration,
};

pub struct Request {
    pub method: String,
    pub path: String,
    pub body: Value,
    pub reply: mpsc::Sender<Response>,
}
pub struct Response {
    pub status: u16,
    pub body: Value,
}
impl Response {
    pub fn ok(body: Value) -> Self {
        Self { status: 200, body }
    }
    pub fn error(status: u16, message: impl ToString) -> Self {
        Self {
            status,
            body: json!({"error":message.to_string()}),
        }
    }
}
pub struct HttpServer {
    pub address: SocketAddr,
    requests: mpsc::Receiver<Request>,
    stop: Arc<AtomicBool>,
}
impl HttpServer {
    pub fn start(
        address: SocketAddr,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> std::io::Result<Self> {
        if !address.ip().is_loopback() {
            return Err(std::io::Error::other("MCP HTTP must bind to loopback"));
        }
        let listener = TcpListener::bind(address)?;
        let address = listener.local_addr()?;
        listener.set_nonblocking(true)?;
        let (tx, requests) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let wake = Arc::new(wake);
        // A bounded pool prevents stalled local clients from creating unbounded threads.
        let (sockets_tx, sockets_rx) = mpsc::sync_channel::<TcpStream>(16);
        let sockets_rx = Arc::new(std::sync::Mutex::new(sockets_rx));
        for i in 0..4 {
            let (rx, tx, wake, stop) = (sockets_rx.clone(), tx.clone(), wake.clone(), stop.clone());
            std::thread::Builder::new()
                .name(format!("minihou-http-{i}"))
                .spawn(move || loop {
                    let socket = rx.lock().unwrap().recv_timeout(Duration::from_millis(100));
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                    if let Ok(socket) = socket {
                        serve(socket, &tx, &*wake, &stop);
                    } else if matches!(socket, Err(mpsc::RecvTimeoutError::Disconnected)) {
                        break;
                    }
                })?;
        }
        std::thread::Builder::new()
            .name("minihou-http-listen".into())
            .spawn(move || {
                while !stopped.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((socket, _)) => {
                            // Accepted sockets can inherit the listener's nonblocking mode on Windows.
                            if socket.set_nonblocking(false).is_ok() {
                                let _ = sockets_tx.try_send(socket);
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(10))
                        }
                        Err(_) => break,
                    }
                }
            })?;
        Ok(Self {
            address,
            requests,
            stop,
        })
    }
    pub fn poll(&self) -> Vec<Request> {
        self.requests.try_iter().collect()
    }
}
impl Drop for HttpServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}
fn parse(socket: &mut TcpStream) -> Result<(String, String, Value)> {
    socket.set_read_timeout(Some(Duration::from_secs(5)))?;
    socket.set_write_timeout(Some(Duration::from_secs(5)))?;
    let mut reader = BufReader::new(socket);
    let mut line = String::new();
    reader.by_ref().take(8193).read_line(&mut line)?;
    if line.len() > 8192 {
        return Err("request line too long".into());
    }
    let parts = line.split_whitespace().collect::<Vec<_>>();
    if parts.len() != 3 || !parts[2].starts_with("HTTP/1.") {
        return Err("invalid HTTP request".into());
    }
    let (method, path) = (parts[0].to_owned(), parts[1].to_owned());
    let (mut length, mut total, mut has_length, mut host) = (0usize, 0usize, false, false);
    loop {
        line.clear();
        let n = reader.by_ref().take(16385).read_line(&mut line)?;
        total += n;
        if total > 16384 || n == 0 {
            return Err("invalid or oversized HTTP headers".into());
        }
        if line == "\r\n" || line == "\n" {
            break;
        }
        let (k, v) = line.split_once(':').ok_or("invalid HTTP header")?;
        let v = v.trim();
        if k.eq_ignore_ascii_case("content-length") {
            if has_length {
                return Err("duplicate Content-Length".into());
            }
            has_length = true;
            length = v.parse()?;
        }
        if k.eq_ignore_ascii_case("transfer-encoding") {
            return Err("Transfer-Encoding is not supported; use Content-Length".into());
        }
        if k.eq_ignore_ascii_case("host") {
            let h = v.split(':').next().unwrap_or("");
            if !["127.0.0.1", "localhost"].contains(&h) {
                return Err("Host must be localhost or 127.0.0.1".into());
            }
            host = true;
        }
        if k.eq_ignore_ascii_case("origin") {
            let authority = v
                .strip_prefix("http://")
                .ok_or("cross-origin request rejected")?
                .split('/')
                .next()
                .unwrap_or("");
            if !["127.0.0.1", "localhost"].contains(&authority.split(':').next().unwrap_or("")) {
                return Err("cross-origin request rejected".into());
            }
        }
    }
    if !host {
        return Err("missing Host".into());
    }
    if length > 8 * 1024 * 1024 {
        return Err("request body exceeds 8 MiB".into());
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    let body = if body.is_empty() {
        json!({})
    } else {
        serde_json::from_slice(&body)?
    };
    if !body.is_object() {
        return Err("request JSON must be an object".into());
    }
    Ok((method, path, body))
}
fn serve(mut socket: TcpStream, tx: &mpsc::Sender<Request>, wake: &dyn Fn(), stop: &AtomicBool) {
    let response = match parse(&mut socket) {
        Err(e) => Response::error(400, e),
        Ok((method, path, body)) => {
            let (reply, rx) = mpsc::channel();
            if tx
                .send(Request {
                    method,
                    path,
                    body,
                    reply,
                })
                .is_err()
            {
                Response::error(503, "editor closed")
            } else {
                wake();
                let deadline = std::time::Instant::now() + Duration::from_secs(120);
                loop {
                    match rx.recv_timeout(Duration::from_millis(100)) {
                        Ok(r) => break r,
                        Err(mpsc::RecvTimeoutError::Disconnected) => {
                            break Response::error(503, "editor closed")
                        }
                        _ if stop.load(Ordering::Relaxed) => {
                            break Response::error(503, "editor closed")
                        }
                        _ if std::time::Instant::now() >= deadline => {
                            break Response::error(504, "editor did not answer within 120 s")
                        }
                        _ => {}
                    }
                }
            }
        }
    };
    let body = response.body.to_string();
    let reason = match response.status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        409 => "Conflict",
        501 => "Not Implemented",
        _ => "Service Unavailable",
    };
    let _=write!(socket,"HTTP/1.1 {} {}\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\n\r\n{}",response.status,reason,body.len(),body);
}

pub fn node_types() -> Value {
    Value::Array(minihou_core::nodes::registry().iter().map(|s|{
        let params=s.params.iter().map(|p|{
            let mut m=crate::param_meta(s.name,p.name).cloned().unwrap_or_else(||json!({}));
            m["name"]=json!(p.name);m["default"]=(p.default)();m["supported"]=json!(p.supported);
            if m.get("kind").is_none(){m["kind"]=json!(match crate::param_kind(&m["default"]){crate::ParamKind::Bool=>"bool",crate::ParamKind::Integer=>"int",crate::ParamKind::Float=>"float",crate::ParamKind::Vec3=>"vec3",crate::ParamKind::String=>"str",_=>"json"});}m
        }).collect::<Vec<_>>();
        json!({"name":s.name,"category":"rust","doc":"Rust core node","inputs":s.inputs.iter().map(|i|json!({"name":i.name,"multi":i.multi,"optional":i.optional})).collect::<Vec<_>>(),"params":params,"actions":if s.name=="usd_export"{json!([{"name":"save","label":"Save to Disk","doc":"Write cooked geometry to USD"}])}else{json!([])}})
    }).collect())
}
pub fn error_detail(message: &str) -> Value {
    let node = message
        .split("node '")
        .nth(1)
        .and_then(|s| s.split('\'').next());
    let statement = message.split("statement ").nth(1)
        .and_then(|s| s.split(|c: char| !c.is_ascii_digit()).next())
        .and_then(|s| s.parse::<usize>().ok());
    let excerpt = message.split("; source: ").nth(1).map(str::trim);
    json!({"node":node,"message":message,"statement":statement,"excerpt":excerpt})
}
pub fn edit_status(body: &mut Value) {
    let status = body["status"].clone();
    body["edit_applied"] = body["changed"].clone();
    body["cook_ok"] = if status["busy"] == true { Value::Null } else { status["ok"].clone() };
    body["failed_node"] = status["error"]["node"].clone();
    body["error"] = status["error"].clone();
}

#[cfg(test)]
mod edit_status_tests {
    use super::*;
    #[test]
    fn edit_reply_exposes_wrangle_error_fields() {
        let detail = error_detail("node 'w' (wrangle): point 0, line 1, statement 2: @uv has existing width 2; requested width 3; source: v@uv = set(1, 2, 3)");
        assert_eq!(detail["node"], "w");
        assert_eq!(detail["statement"], 2);
        assert_eq!(detail["excerpt"], "v@uv = set(1, 2, 3)");
        let mut reply = json!({"changed":true,"status":{"busy":false,"ok":false,"error":detail}});
        edit_status(&mut reply);
        assert_eq!(reply["edit_applied"], true);
        assert_eq!(reply["cook_ok"], false);
        assert_eq!(reply["failed_node"], "w");
    }
}
