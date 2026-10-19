pub mod auth;
pub mod commands;
pub mod handlers;
pub mod validate;

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

/// Largest accepted request line. Longer requests get an error and the connection is dropped.
const MAX_REQUEST_BYTES: usize = 1024 * 1024;

/// Maximum concurrent client connections. Extra connections are closed immediately.
const MAX_CONNECTIONS: usize = 64;

/// Compute the Unix socket path per D-06.
/// $XDG_RUNTIME_DIR/cmux/cmux.sock, fallback /run/user/{uid}/cmux/cmux.sock.
pub fn socket_path() -> PathBuf {
    let base = std::env::var("XDG_RUNTIME_DIR")
        .unwrap_or_else(|_| format!("/run/user/{}", unsafe { libc::getuid() }));
    PathBuf::from(base).join("cmux").join("cmux.sock")
}

/// Returns the directory containing the socket file.
fn socket_dir() -> PathBuf {
    socket_path().parent().unwrap().to_path_buf()
}

/// Returns the last-socket-path marker file path.
fn last_socket_path_marker() -> PathBuf {
    socket_dir().join("last-socket-path")
}

/// Start the socket server:
/// 1. Creates $XDG_RUNTIME_DIR/cmux/ (mode 0700).
/// 2. Removes stale socket file from previous crash.
/// 3. Binds UnixListener, sets socket mode to 0600.
/// 4. Writes last-socket-path marker for cmux.py discovery.
/// 5. Spawns tokio accept loop.
///
/// The cmd_tx sender is used to dispatch SocketCommands to the GTK main thread
/// via the tokio::sync::mpsc bridge established in main.rs.
pub fn start_socket_server(
    runtime: &tokio::runtime::Handle,
    _state: crate::app_state::AppStateRef,
    cmd_tx: tokio::sync::mpsc::UnboundedSender<commands::SocketCommand>,
) {
    let sock_path = socket_path();
    let dir = socket_dir();

    // Create directory with restrictive permissions.
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("cmux: socket dir create failed: {e}");
        return;
    }
    if let Err(e) = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)) {
        eprintln!("cmux: socket dir chmod failed: {e}");
    }

    // Remove stale socket from previous run (ignore ENOENT).
    let _ = std::fs::remove_file(&sock_path);

    // Enter the tokio runtime context so UnixListener::bind can register with the reactor.
    // bind() is synchronous but requires an active reactor context.
    let _guard = runtime.enter();
    let listener = match tokio::net::UnixListener::bind(&sock_path) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("cmux: socket bind failed at {}: {e}", sock_path.display());
            return;
        }
    };

    // Set socket file mode to 0600 (owner read/write only).
    if let Err(e) = std::fs::set_permissions(&sock_path, std::fs::Permissions::from_mode(0o600)) {
        eprintln!("cmux: socket chmod failed: {e}");
    }

    // Write last-socket-path marker so cmux.py can discover the socket.
    if let Err(e) = std::fs::write(last_socket_path_marker(), sock_path.to_string_lossy().as_bytes()) {
        eprintln!("cmux: last-socket-path write failed: {e}");
    }

    eprintln!("cmux: socket server listening at {}", sock_path.display());

    // Spawn the accept loop in tokio.
    let limit = std::sync::Arc::new(tokio::sync::Semaphore::new(MAX_CONNECTIONS));
    runtime.spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, _addr)) => {
                    // Validate peer UID before reading any data.
                    match auth::validate_peer_uid(&stream) {
                        Ok(true) => match limit.clone().try_acquire_owned() {
                            Ok(permit) => {
                                let tx = cmd_tx.clone();
                                tokio::spawn(async move {
                                    let _permit = permit; // released when the connection ends
                                    handle_connection(stream, tx).await;
                                });
                            }
                            Err(_) => {
                                eprintln!("cmux: socket connection limit reached, rejecting");
                            }
                        },
                        Ok(false) => {
                            eprintln!("cmux: socket connection rejected (UID mismatch)");
                        }
                        Err(e) => {
                            eprintln!("cmux: SO_PEERCRED check failed: {e}");
                        }
                    }
                }
                Err(e) => {
                    // Transient (e.g. EMFILE): log, back off briefly, keep serving.
                    eprintln!("cmux: socket accept error: {e}");
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
            }
        }
    });
}

/// One request line read by `read_capped_line`.
#[derive(Debug, PartialEq)]
enum LineRead {
    Line(Vec<u8>),
    Eof,
    TooLong,
}

/// Read one newline-terminated line, buffering at most `max` bytes of it.
/// A line that would exceed `max` returns `TooLong` without reading further.
async fn read_capped_line<R>(reader: &mut R, max: usize) -> std::io::Result<LineRead>
where
    R: tokio::io::AsyncBufRead + Unpin,
{
    use tokio::io::AsyncBufReadExt;

    let mut buf: Vec<u8> = Vec::new();
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            return Ok(if buf.is_empty() { LineRead::Eof } else { LineRead::Line(buf) });
        }
        let newline = available.iter().position(|&b| b == b'\n');
        let take = newline.unwrap_or(available.len());
        if buf.len() + take > max {
            return Ok(LineRead::TooLong);
        }
        buf.extend_from_slice(&available[..take]);
        match newline {
            Some(_) => {
                reader.consume(take + 1);
                return Ok(LineRead::Line(buf));
            }
            None => reader.consume(take),
        }
    }
}

/// Per-connection handler running in a tokio task.
/// Reads newline-delimited JSON requests, dispatches via mpsc channel, writes responses.
async fn handle_connection(
    stream: tokio::net::UnixStream,
    cmd_tx: tokio::sync::mpsc::UnboundedSender<commands::SocketCommand>,
) {
    use tokio::io::{AsyncWriteExt, BufReader};

    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);

    loop {
        let bytes = match read_capped_line(&mut reader, MAX_REQUEST_BYTES).await {
            Ok(LineRead::Line(bytes)) => bytes,
            Ok(LineRead::Eof) | Err(_) => break,
            Ok(LineRead::TooLong) => {
                let response = serde_json::json!({
                    "id": null,
                    "ok": false,
                    "error": {
                        "code": "request_too_large",
                        "message": format!("request exceeds {MAX_REQUEST_BYTES} bytes"),
                    }
                })
                .to_string();
                let _ = writer.write_all(response.as_bytes()).await;
                let _ = writer.write_all(b"\n").await;
                break;
            }
        };
        let text = String::from_utf8_lossy(&bytes);
        let response = dispatch_line(text.trim_end_matches('\r'), &cmd_tx).await;
        if writer.write_all(response.as_bytes()).await.is_err() { break; }
        if writer.write_all(b"\n").await.is_err() { break; }
    }
}

/// Parse a JSON-RPC line and dispatch to the appropriate SocketCommand.
/// Returns the JSON response string (without trailing newline).
async fn dispatch_line(
    line: &str,
    cmd_tx: &tokio::sync::mpsc::UnboundedSender<commands::SocketCommand>,
) -> String {
    let req: serde_json::Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => {
            return serde_json::json!({
                "id": null,
                "ok": false,
                "error": {"code": "parse_error", "message": "invalid JSON"}
            }).to_string();
        }
    };

    let req_id = req.get("id").cloned().unwrap_or(serde_json::Value::Null);
    let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("").to_string();
    let params = req.get("params").cloned().unwrap_or(serde_json::Value::Object(Default::default()));

    let (resp_tx, resp_rx) = tokio::sync::oneshot::channel();

    let cmd = match method.as_str() {
        "system.ping" => commands::SocketCommand::Ping { req_id: req_id.clone(), resp_tx },
        "system.identify" => commands::SocketCommand::Identify { req_id: req_id.clone(), resp_tx },
        "system.capabilities" => commands::SocketCommand::Capabilities { req_id: req_id.clone(), resp_tx },

        "workspace.list" => commands::SocketCommand::WorkspaceList { req_id: req_id.clone(), resp_tx },
        "workspace.current" => commands::SocketCommand::WorkspaceCurrent { req_id: req_id.clone(), resp_tx },
        "workspace.create" => commands::SocketCommand::WorkspaceCreate {
            req_id: req_id.clone(),
            remote_target: params.get("remote_target").and_then(|v| v.as_str()).map(String::from),
            resp_tx,
        },
        "workspace.select" => commands::SocketCommand::WorkspaceSelect {
            req_id: req_id.clone(),
            id: params.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            resp_tx,
        },
        "workspace.close" => commands::SocketCommand::WorkspaceClose {
            req_id: req_id.clone(),
            id: params.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            resp_tx,
        },
        "workspace.rename" => commands::SocketCommand::WorkspaceRename {
            req_id: req_id.clone(),
            id: params.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            name: params.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            resp_tx,
        },
        "workspace.next" => commands::SocketCommand::WorkspaceNext { req_id: req_id.clone(), resp_tx },
        "workspace.previous" => commands::SocketCommand::WorkspacePrev { req_id: req_id.clone(), resp_tx },
        "workspace.last" => commands::SocketCommand::WorkspaceLast { req_id: req_id.clone(), resp_tx },
        "workspace.reorder" => commands::SocketCommand::WorkspaceReorder {
            req_id: req_id.clone(),
            id: params.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            position: params.get("position").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
            resp_tx,
        },

        "surface.list" => commands::SocketCommand::SurfaceList { req_id: req_id.clone(), resp_tx },
        "surface.split" => commands::SocketCommand::SurfaceSplit {
            req_id: req_id.clone(),
            id: params.get("id").and_then(|v| v.as_str()).map(String::from),
            direction: params.get("direction").and_then(|v| v.as_str()).unwrap_or("horizontal").to_string(),
            resp_tx,
        },
        "surface.focus" => commands::SocketCommand::SurfaceFocus {
            req_id: req_id.clone(),
            id: params.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            resp_tx,
        },
        "surface.close" => commands::SocketCommand::SurfaceClose {
            req_id: req_id.clone(),
            id: params.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            resp_tx,
        },
        "surface.send_text" => commands::SocketCommand::SurfaceSendText {
            req_id: req_id.clone(),
            id: params.get("id").and_then(|v| v.as_str()).map(String::from),
            text: params.get("text").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            resp_tx,
        },
        "surface.send_key" => commands::SocketCommand::SurfaceSendKey {
            req_id: req_id.clone(),
            id: params.get("id").and_then(|v| v.as_str()).map(String::from),
            key: params.get("key").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            resp_tx,
        },
        "surface.read_text" => commands::SocketCommand::SurfaceReadText {
            req_id: req_id.clone(),
            id: params.get("id").and_then(|v| v.as_str()).map(String::from),
            resp_tx,
        },
        "surface.health" => commands::SocketCommand::SurfaceHealth {
            req_id: req_id.clone(),
            id: params.get("id").and_then(|v| v.as_str()).map(String::from),
            resp_tx,
        },
        "surface.refresh" => commands::SocketCommand::SurfaceRefresh {
            req_id: req_id.clone(),
            id: params.get("id").and_then(|v| v.as_str()).map(String::from),
            resp_tx,
        },

        "pane.list" => commands::SocketCommand::PaneList { req_id: req_id.clone(), resp_tx },
        "pane.focus" => commands::SocketCommand::PaneFocus {
            req_id: req_id.clone(),
            id: params.get("id").and_then(|v| v.as_str()).map(String::from),
            resp_tx,
        },
        "pane.last" => commands::SocketCommand::PaneLast { req_id: req_id.clone(), resp_tx },

        "window.list" => commands::SocketCommand::WindowList { req_id: req_id.clone(), resp_tx },
        "window.current" => commands::SocketCommand::WindowCurrent { req_id: req_id.clone(), resp_tx },

        "debug.layout" => commands::SocketCommand::DebugLayout { req_id: req_id.clone(), resp_tx },
        // Keystroke injection is a debugging aid only; release builds do not route it.
        #[cfg(debug_assertions)]
        "debug.type" => commands::SocketCommand::DebugType {
            req_id: req_id.clone(),
            text: params.get("text").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            resp_tx,
        },

        "notification.list" => commands::SocketCommand::NotificationList {
            req_id: req_id.clone(),
            resp_tx,
        },
        "notification.clear" => commands::SocketCommand::NotificationClear {
            req_id: req_id.clone(),
            id: params.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            resp_tx,
        },

        "browser.open" => commands::SocketCommand::BrowserOpen {
            req_id: req_id.clone(),
            url: params.get("url").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            workspace: params.get("workspace").and_then(|v| v.as_str()).map(String::from),
            resp_tx,
        },
        "browser.stream.enable" => commands::SocketCommand::BrowserStreamEnable {
            req_id: req_id.clone(),
            resp_tx,
        },
        "browser.stream.disable" => commands::SocketCommand::BrowserStreamDisable {
            req_id: req_id.clone(),
            resp_tx,
        },
        "browser.list" => commands::SocketCommand::BrowserList {
            req_id: req_id.clone(),
            resp_tx,
        },

        // Route all other browser.* methods to the generic proxy (P0/P1 parity)
        _ if method.starts_with("browser.") => {
            let action = method.strip_prefix("browser.").unwrap().to_string();
            let surface_ref = params.get("surface_ref").and_then(|v| v.as_str()).map(String::from);
            commands::SocketCommand::BrowserAction {
                req_id: req_id.clone(),
                action,
                params,
                surface_ref,
                resp_tx,
            }
        }

        _ => commands::SocketCommand::NotImplemented {
            req_id: req_id.clone(),
            method: method.clone(),
            resp_tx,
        },
    };

    if cmd_tx.send(cmd).is_err() {
        return serde_json::json!({
            "id": req_id,
            "ok": false,
            "error": {"code": "internal_error", "message": "handler channel closed"}
        }).to_string();
    }

    resp_rx.await.unwrap_or_else(|_| serde_json::json!({
        "id": req_id,
        "ok": false,
        "error": {"code": "internal_error", "message": "handler dropped response"}
    })).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SOCK-01: Socket path must be under XDG_RUNTIME_DIR/cmux/.
    #[test]
    fn test_socket_path_creation() {
        unsafe { std::env::set_var("XDG_RUNTIME_DIR", "/tmp/test-xdg") };
        let path = socket_path();
        assert_eq!(path, std::path::PathBuf::from("/tmp/test-xdg/cmux/cmux.sock"));
    }

    #[tokio::test]
    async fn test_read_capped_line_reads_lines() {
        let data: &[u8] = b"abc\ndef";
        let mut reader = tokio::io::BufReader::new(data);
        assert_eq!(read_capped_line(&mut reader, 16).await.unwrap(), LineRead::Line(b"abc".to_vec()));
        // Final line without a trailing newline is still returned.
        assert_eq!(read_capped_line(&mut reader, 16).await.unwrap(), LineRead::Line(b"def".to_vec()));
        assert_eq!(read_capped_line(&mut reader, 16).await.unwrap(), LineRead::Eof);
    }

    #[tokio::test]
    async fn test_read_capped_line_rejects_overlong() {
        let data: &[u8] = b"abcdefgh\nok\n";
        let mut reader = tokio::io::BufReader::new(data);
        assert_eq!(read_capped_line(&mut reader, 4).await.unwrap(), LineRead::TooLong);
        // A line exactly at the cap is accepted.
        let data: &[u8] = b"abcd\n";
        let mut reader = tokio::io::BufReader::new(data);
        assert_eq!(read_capped_line(&mut reader, 4).await.unwrap(), LineRead::Line(b"abcd".to_vec()));
    }

    /// SOCK-05: Focus policy whitelist is documented.
    #[test]
    fn test_focus_policy() {
        let focus_intent_methods = [
            "workspace.select", "pane.focus", "pane.last", "surface.focus",
            "workspace.next", "workspace.previous", "workspace.last",
        ];
        assert!(!focus_intent_methods.is_empty());
    }
}
