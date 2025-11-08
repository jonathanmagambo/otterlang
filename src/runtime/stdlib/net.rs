use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::raw::c_char;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use once_cell::sync::Lazy;
use parking_lot::{Mutex, RwLock};

use crate::runtime::symbol_registry::{FfiFunction, FfiSignature, FfiType, SymbolRegistry};

type HandleId = u64;
static NEXT_HANDLE_ID: AtomicU64 = AtomicU64::new(1);

fn next_handle_id() -> HandleId {
    NEXT_HANDLE_ID.fetch_add(1, Ordering::SeqCst)
}

struct Connection {
    stream: Mutex<TcpStream>,
}

struct HttpResponse {
    status: i32,
    body: String,
}

static CONNECTIONS: Lazy<RwLock<HashMap<HandleId, Connection>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));

static LISTENERS: Lazy<RwLock<HashMap<HandleId, Mutex<TcpListener>>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));

static HTTP_RESPONSES: Lazy<RwLock<HashMap<HandleId, HttpResponse>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));

fn parse_http_url(url: &str) -> Option<(String, u16, String)> {
    let trimmed = url.trim();
    let rest = trimmed.strip_prefix("http://")?;
    let (authority, path_part) = match rest.split_once('/') {
        Some((host_port, path)) => (host_port, format!("/{}", path)),
        None => (rest, "/".to_string()),
    };

    if authority.is_empty() {
        return None;
    }

    let mut host_split = authority.splitn(2, ':');
    let host = host_split.next()?.to_string();
    let port = host_split
        .next()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(80);

    Some((host, port, path_part))
}

fn run_http_request(method: &str, url: &str, body: Option<&str>) -> HttpResponse {
    let Some((host, port, path)) = parse_http_url(url) else {
        return HttpResponse {
            status: 400,
            body: format!("unsupported url: {}", url),
        };
    };

    match TcpStream::connect((host.as_str(), port)) {
        Ok(mut stream) => {
            let timeout = Some(Duration::from_secs(5));
            let _ = stream.set_read_timeout(timeout);
            let _ = stream.set_write_timeout(timeout);

            let request = match body {
                Some(payload) => format!(
                    concat!(
                        "{method} {path} HTTP/1.1\\r\\n",
                        "Host: {host}\\r\\n",
                        "Content-Length: {len}\\r\\n",
                        "Connection: close\\r\\n",
                        "Accept: */*\\r\\n",
                        "\\r\\n",
                        "{payload}"
                    ),
                    method = method,
                    path = path,
                    host = host,
                    len = payload.as_bytes().len(),
                    payload = payload
                ),
                None => format!(
                    concat!(
                        "{method} {path} HTTP/1.1\\r\\n",
                        "Host: {host}\\r\\n",
                        "Connection: close\\r\\n",
                        "Accept: */*\\r\\n",
                        "\\r\\n"
                    ),
                    method = method,
                    path = path,
                    host = host
                ),
            };

            if let Err(err) = stream.write_all(request.as_bytes()) {
                return HttpResponse {
                    status: 502,
                    body: format!("request failed: {}", err),
                };
            }

            let mut buffer = Vec::new();
            match stream.read_to_end(&mut buffer) {
                Ok(_) => {
                    let response_text = String::from_utf8_lossy(&buffer);
                    let mut sections = response_text.splitn(2, "\\r\\n\\r\\n");
                    let header_block = sections.next().unwrap_or("");
                    let body_block = sections.next().unwrap_or("");

                    let mut header_lines = header_block.lines();
                    let status_line = header_lines.next().unwrap_or("");
                    let status = status_line
                        .split_whitespace()
                        .nth(1)
                        .and_then(|code| code.parse::<i32>().ok())
                        .unwrap_or(500);

                    HttpResponse {
                        status,
                        body: body_block.to_string(),
                    }
                }
                Err(err) => HttpResponse {
                    status: 504,
                    body: format!("response read failed: {}", err),
                },
            }
        }
        Err(err) => HttpResponse {
            status: 503,
            body: format!("connection failed: {}", err),
        },
    }
}

#[no_mangle]
pub unsafe extern "C" fn otter_std_net_listen(addr: *const c_char) -> u64 {
    if addr.is_null() {
        return 0;
    }

    let Ok(address) = (unsafe { CStr::from_ptr(addr).to_str() }) else {
        return 0;
    };

    match TcpListener::bind(address) {
        Ok(listener) => {
            let id = next_handle_id();
            LISTENERS.write().insert(id, Mutex::new(listener));
            id
        }
        Err(_) => 0,
    }
}

#[no_mangle]
pub unsafe extern "C" fn otter_std_net_dial(addr: *const c_char) -> u64 {
    if addr.is_null() {
        return 0;
    }

    let Ok(address) = (unsafe { CStr::from_ptr(addr).to_str() }) else {
        return 0;
    };

    match TcpStream::connect(address) {
        Ok(stream) => {
            let _ = stream.set_nonblocking(true);
            let id = next_handle_id();
            CONNECTIONS.write().insert(
                id,
                Connection {
                    stream: Mutex::new(stream),
                },
            );
            id
        }
        Err(_) => 0,
    }
}

#[no_mangle]
pub unsafe extern "C" fn otter_std_net_send(conn: u64, data: *const c_char) -> i32 {
    if data.is_null() {
        return 0;
    }

    let Ok(message) = (unsafe { CStr::from_ptr(data).to_str() }) else {
        return 0;
    };

    let connections = CONNECTIONS.read();
    if let Some(connection) = connections.get(&conn) {
        let mut stream = connection.stream.lock();
        if stream.write_all(message.as_bytes()).is_ok() && stream.flush().is_ok() {
            1
        } else {
            0
        }
    } else {
        0
    }
}

#[no_mangle]
pub extern "C" fn otter_std_net_recv(conn: u64) -> *mut c_char {
    let connections = CONNECTIONS.read();
    let Some(connection) = connections.get(&conn) else {
        return std::ptr::null_mut();
    };

    let mut stream = connection.stream.lock();
    let mut buffer = vec![0u8; 4096];
    match stream.read(&mut buffer) {
        Ok(0) => {
            drop(stream);
            drop(connections);
            CONNECTIONS.write().remove(&conn);
            std::ptr::null_mut()
        }
        Ok(n) => {
            let text = String::from_utf8_lossy(&buffer[..n]).to_string();
            CString::new(text)
                .ok()
                .map(CString::into_raw)
                .unwrap_or(std::ptr::null_mut())
        }
        Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => CString::new("")
            .ok()
            .map(CString::into_raw)
            .unwrap_or(std::ptr::null_mut()),
        Err(_) => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "C" fn otter_std_net_close(conn: u64) {
    CONNECTIONS.write().remove(&conn);
}

#[no_mangle]
pub unsafe extern "C" fn otter_std_net_http_get(url: *const c_char) -> u64 {
    if url.is_null() {
        return 0;
    }

    let Ok(url_str) = (unsafe { CStr::from_ptr(url).to_str() }) else {
        return 0;
    };

    let id = next_handle_id();
    let response = run_http_request("GET", url_str, None);
    HTTP_RESPONSES.write().insert(id, response);
    id
}

#[no_mangle]
pub unsafe extern "C" fn otter_std_net_http_post(url: *const c_char, body: *const c_char) -> u64 {
    if url.is_null() {
        return 0;
    }

    let Ok(url_str) = (unsafe { CStr::from_ptr(url).to_str() }) else {
        return 0;
    };

    let body_str = if body.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(body).to_string_lossy().into_owned() }
    };

    let id = next_handle_id();
    let response = run_http_request("POST", url_str, Some(&body_str));
    HTTP_RESPONSES.write().insert(id, response);
    id
}

#[no_mangle]
pub extern "C" fn otter_std_net_response_status(response: u64) -> i32 {
    let responses = HTTP_RESPONSES.read();
    responses
        .get(&response)
        .map(|resp| resp.status)
        .unwrap_or(0)
}

#[no_mangle]
pub extern "C" fn otter_std_net_response_body(response: u64) -> *mut c_char {
    let responses = HTTP_RESPONSES.read();
    if let Some(resp) = responses.get(&response) {
        CString::new(resp.body.clone())
            .ok()
            .map(CString::into_raw)
            .unwrap_or(std::ptr::null_mut())
    } else {
        std::ptr::null_mut()
    }
}

fn register_std_net_symbols(registry: &SymbolRegistry) {
    registry.register(FfiFunction {
        name: "net.listen".into(),
        symbol: "otter_std_net_listen".into(),
        signature: FfiSignature::new(vec![FfiType::Str], FfiType::Opaque),
    });

    registry.register(FfiFunction {
        name: "net.dial".into(),
        symbol: "otter_std_net_dial".into(),
        signature: FfiSignature::new(vec![FfiType::Str], FfiType::Opaque),
    });

    registry.register(FfiFunction {
        name: "net.send".into(),
        symbol: "otter_std_net_send".into(),
        signature: FfiSignature::new(vec![FfiType::Opaque, FfiType::Str], FfiType::I32),
    });

    registry.register(FfiFunction {
        name: "net.recv".into(),
        symbol: "otter_std_net_recv".into(),
        signature: FfiSignature::new(vec![FfiType::Opaque], FfiType::Str),
    });

    registry.register(FfiFunction {
        name: "net.close".into(),
        symbol: "otter_std_net_close".into(),
        signature: FfiSignature::new(vec![FfiType::Opaque], FfiType::Unit),
    });

    registry.register(FfiFunction {
        name: "net.http_get".into(),
        symbol: "otter_std_net_http_get".into(),
        signature: FfiSignature::new(vec![FfiType::Str], FfiType::Opaque),
    });

    registry.register(FfiFunction {
        name: "net.http_post".into(),
        symbol: "otter_std_net_http_post".into(),
        signature: FfiSignature::new(vec![FfiType::Str, FfiType::Str], FfiType::Opaque),
    });

    registry.register(FfiFunction {
        name: "net.response.status".into(),
        symbol: "otter_std_net_response_status".into(),
        signature: FfiSignature::new(vec![FfiType::Opaque], FfiType::I32),
    });

    registry.register(FfiFunction {
        name: "net.response.body".into(),
        symbol: "otter_std_net_response_body".into(),
        signature: FfiSignature::new(vec![FfiType::Opaque], FfiType::Str),
    });
}

inventory::submit! {
    crate::runtime::ffi::SymbolProvider {
        register: register_std_net_symbols,
    }
}
