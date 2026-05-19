use axum::{
    body::Body,
    extract::{Request, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use bytes::Bytes;
use futures::stream::{self, BoxStream, StreamExt};
use std::io;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::Stdio;
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::io::AsyncWriteExt;
use tokio::process::Command as TokioCommand;
use tokio_util::io::ReaderStream;
use tracing::{error, info, warn};

use crate::auth;
use crate::config::detect_git_executable;
use crate::git_cgi::AppState;

#[derive(Debug, Clone, Copy)]
enum GitService {
    UploadPack,
    ReceivePack,
}

#[derive(Debug)]
enum GitAction {
    /// GET /info/refs?service=git-xxx-pack → --advertise-refs
    AdvertiseRefs,
    /// POST /git-xxx-pack → --stateless-rpc
    Rpc(GitService),
}

#[derive(Debug)]
struct ParsedPath {
    repo_name: String,
    action: GitAction,
}

/// Parse the Git HTTP path.
/// Handles paths like `/repo.git/info/refs`, `/v2/repo.git/git-upload-pack`, etc.
/// Extracts the repo name (last path segment before the known suffix) and the action.
fn parse_git_path(path: &str) -> Option<ParsedPath> {
    let path = path.strip_suffix('/').unwrap_or(path);

    // Reject path traversal and double-slash (empty segment) early
    if path.contains("..") || path.contains("//") || path.contains('\\') {
        return None;
    }

    if let Some(rest) = path.strip_suffix("/info/refs") {
        let repo_name = rest.rsplit('/').find(|s| !s.is_empty())?.to_string();
        if repo_name.is_empty() {
            return None;
        }
        return Some(ParsedPath {
            repo_name,
            action: GitAction::AdvertiseRefs,
        });
    }

    let (rest, svc) = if let Some(r) = path.strip_suffix("/git-upload-pack") {
        (r, GitService::UploadPack)
    } else if let Some(r) = path.strip_suffix("/git-receive-pack") {
        (r, GitService::ReceivePack)
    } else {
        return None;
    };

    let repo_name = rest.rsplit('/').find(|s| !s.is_empty())?.to_string();
    if repo_name.is_empty() {
        return None;
    }

    Some(ParsedPath {
        repo_name,
        action: GitAction::Rpc(svc),
    })
}

fn verify_repo_path(root: &Path, repo_name: &str) -> Result<PathBuf, StatusCode> {
    use crate::config::verify_repo_path as config_verify_repo_path;

    config_verify_repo_path(root, repo_name).map_err(|e| {
        if e.kind() == std::io::ErrorKind::InvalidInput {
            warn!("native Path traversal attempt detected: {:?}", e);
            StatusCode::FORBIDDEN
        } else {
            error!("native Failed to verify repo path: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        }
    })
}

fn service_name(svc: &GitService) -> &'static str {
    match svc {
        GitService::UploadPack => "upload-pack",
        GitService::ReceivePack => "receive-pack",
    }
}

fn content_type_advertisement(svc: &GitService) -> String {
    format!("application/x-git-{}-advertisement", service_name(svc))
}

fn content_type_result(svc: &GitService) -> String {
    format!("application/x-git-{}-result", service_name(svc))
}

fn content_type_request(svc: &GitService) -> String {
    format!("application/x-git-{}-request", service_name(svc))
}

/// Protocol v2 不需要 `# service=git-xxx-pack` 前缀
fn is_protocol_v2(headers: &HeaderMap) -> bool {
    if let Some(gp) = headers.get("git-protocol")
        && let Ok(v) = gp.to_str()
    {
        return v.contains("version=2");
    }
    false
}

/// 构造 pkt-line 格式的 service header: `001e# service=git-upload-pack\n0000`
fn build_service_pkt_line(svc: &GitService) -> String {
    let payload = format!("# service=git-{}\n", service_name(svc));
    let total_len = payload.len() + 4; // 4 bytes pkt-len
    format!("{:04x}{}{:04x}", total_len, payload, 0)
}

fn build_body_stream(
    reader: impl tokio::io::AsyncRead + Send + 'static,
    prepend_header: Option<String>,
) -> BoxStream<'static, Result<Bytes, io::Error>> {
    if let Some(header) = prepend_header {
        let header_bytes = Bytes::from(header);
        let header_stream = stream::once(async move { Ok(header_bytes) });
        let stdout_stream = ReaderStream::new(reader);
        Box::pin(header_stream.chain(stdout_stream))
    } else {
        Box::pin(ReaderStream::new(reader))
    }
}

/// Stream wrapper that sends a kill signal when dropped.
/// Ensures the Git child process is terminated when the HTTP connection closes.
struct GuardedStream {
    inner: BoxStream<'static, Result<Bytes, io::Error>>,
    kill_tx: Option<tokio::sync::broadcast::Sender<()>>,
}

impl futures::Stream for GuardedStream {
    type Item = Result<Bytes, io::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.inner).poll_next(cx)
    }
}

impl Drop for GuardedStream {
    fn drop(&mut self) {
        if let Some(ref tx) = self.kill_tx {
            let _ = tx.send(());
        }
    }
}

pub async fn git_handler_native(
    State(state): State<Arc<AppState>>,
    req: Request,
) -> Result<Response, StatusCode> {
    let remote_addr = req
        .extensions()
        .get::<std::net::SocketAddr>()
        .map(|a| a.to_string())
        .unwrap_or_else(|| "unknown".to_string());

    info!("native {} {} from {}", req.method(), req.uri(), remote_addr);

    // --- Auth ---
    let remote_user = match auth::verify_auth(&req, &state.config.users) {
        Some(user) => {
            info!("native Auth success for user: {}", user);
            user
        }
        None => {
            warn!(
                "native Auth failed for request: {} {}",
                req.method(),
                req.uri()
            );
            return Ok(auth::unauthorized_response());
        }
    };

    // --- URL parsing ---
    let path = req.uri().path();
    let parsed = parse_git_path(path).ok_or_else(|| {
        warn!("native Invalid request path: {}", path);
        StatusCode::BAD_REQUEST
    })?;

    // --- Resolve service ---
    let svc = match &parsed.action {
        GitAction::AdvertiseRefs => {
            let qs = req.uri().query().unwrap_or("");
            let svc_name = qs
                .split('&')
                .find_map(|p| p.strip_prefix("service="))
                .ok_or_else(|| {
                    warn!("native Missing service parameter in info/refs");
                    StatusCode::BAD_REQUEST
                })?;
            match svc_name {
                "git-upload-pack" => GitService::UploadPack,
                "git-receive-pack" => GitService::ReceivePack,
                _ => {
                    warn!("native Unsupported service: {}", svc_name);
                    return Err(StatusCode::BAD_REQUEST);
                }
            }
        }
        GitAction::Rpc(svc) => *svc,
    };

    let is_ref_advertisement = matches!(parsed.action, GitAction::AdvertiseRefs);

    // --- repo path (with security check) ---
    let repo_path = verify_repo_path(&state.config.git_project_root, &parsed.repo_name)?;

    // --- check content-type for POST ---
    if matches!(parsed.action, GitAction::Rpc(_)) {
        let expected_ct = content_type_request(&svc);
        if let Some(ct) = req.headers().get("Content-Type")
            && let Ok(ct_str) = ct.to_str()
            && ct_str != expected_ct
        {
            warn!(
                "native Unsupported Media Type: {}, expected {}",
                ct_str, expected_ct
            );
            return Ok((
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                format!(
                    "Expected POST with Content-Type '{}', but received '{}' instead.",
                    expected_ct, ct_str
                ),
            )
                .into_response());
        }
    }

    // --- build command ---
    let is_v2 = is_protocol_v2(req.headers());

    let content_type = if is_ref_advertisement {
        content_type_advertisement(&svc)
    } else {
        content_type_result(&svc)
    };

    let git_path = detect_git_executable();
    let mut cmd = TokioCommand::new(&git_path);
    cmd.arg(service_name(&svc));

    if is_ref_advertisement {
        cmd.arg("--advertise-refs");
    } else {
        cmd.arg("--stateless-rpc");
    }
    cmd.arg(&repo_path);

    // --- env ---
    cmd.env("REMOTE_USER", &remote_user);
    if let Some(ct) = req.headers().get("Content-Type")
        && let Ok(v) = ct.to_str()
    {
        cmd.env("CONTENT_TYPE", v);
    }
    if let Some(cl) = req.headers().get("Content-Length")
        && let Ok(v) = cl.to_str()
    {
        cmd.env("CONTENT_LENGTH", v);
    }
    if let Some(gp) = req.headers().get("git-protocol")
        && let Ok(v) = gp.to_str()
    {
        cmd.env("GIT_PROTOCOL", v);
    }

    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());

    let mut child = cmd.spawn().map_err(|e| {
        error!("native Failed to spawn git {}: {}", service_name(&svc), e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    info!(
        "native Spawned git {} for repo {:?}",
        service_name(&svc),
        repo_path
    );

    // Save URI before consuming body
    let request_uri = req.uri().to_string();

    // --- Cancellation: ties child lifecycle to response stream ---
    let (cancel_tx, _) = tokio::sync::broadcast::channel::<()>(1);
    let mut cancel_stdin = cancel_tx.subscribe();
    let mut cancel_child = cancel_tx.subscribe();

    // --- stream stdin ---
    let mut stdin = child.stdin.take().ok_or_else(|| {
        error!("native Failed to take stdin");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let mut body_stream = req.into_body().into_data_stream();

    let stdin_task = tokio::spawn(async move {
        loop {
            tokio::select! {
                biased;
                _ = cancel_stdin.recv() => {
                    return;
                }
                chunk = body_stream.next() => {
                    match chunk {
                        Some(Ok(data)) => {
                            if let Err(e) = stdin.write_all(&data).await {
                                if e.kind() != io::ErrorKind::BrokenPipe {
                                    warn!("native Error streaming request body: {}", e);
                                }
                                return;
                            }
                        }
                        Some(Err(e)) => {
                            warn!("native Error reading request body: {}", e);
                            return;
                        }
                        None => {
                            let _ = stdin.shutdown().await;
                            return;
                        }
                    }
                }
            }
        }
    });

    // --- stream stdout ---
    let stdout = child.stdout.take().ok_or_else(|| {
        error!("native Failed to take stdout");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let need_service_header = is_ref_advertisement && !is_v2;
    let prepend_header = if need_service_header {
        Some(build_service_pkt_line(&svc))
    } else {
        None
    };

    // --- child wait task (kills child on connection close) ---
    let svc_name = service_name(&svc).to_string();
    tokio::spawn(async move {
        tokio::select! {
            biased;
            _ = cancel_child.recv() => {
                let _ = child.start_kill();
                match child.wait().await {
                    Ok(s) if s.success() => {
                        // Client disconnected but child already exited normally.
                        // This is expected for GET /info/refs (client closes after
                        // receiving ref advertisement) and small/fast repos.
                        info!("native git {} completed before connection close", svc_name);
                    }
                    Ok(s) => {
                        warn!("native git {} killed due to connection close, exit status: {}", svc_name, s);
                    }
                    Err(e) => {
                        warn!("native git {} killed due to connection close: {}", svc_name, e);
                    }
                }
            }
            status = child.wait() => {
                let _ = stdin_task.await;
                match status {
                    Ok(s) if !s.success() => {
                        warn!("native git {} exited with status: {}", svc_name, s);
                    }
                    Err(e) => {
                        error!("native Failed to wait for git {}: {}", svc_name, e);
                    }
                    _ => {}
                }
            }
        }
    });

    // --- response ---
    info!("native Starting streaming response for: {}", request_uri);

    let body_stream = build_body_stream(stdout, prepend_header);
    let guarded_stream = GuardedStream {
        inner: body_stream,
        kill_tx: Some(cancel_tx),
    };
    let body = Body::from_stream(guarded_stream);

    let response = Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", &content_type)
        .header("Expires", "Fri, 01 Jan 1980 00:00:00 GMT")
        .header("Pragma", "no-cache")
        .header("Cache-Control", "no-cache, max-age=0, must-revalidate")
        .body(body)
        .map_err(|e| {
            warn!("native Failed to build response: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderMap;

    // ── parse_git_path ──────────────────────────────────────────────

    #[test]
    fn test_parse_path_info_refs() {
        let r = parse_git_path("/v2/repo.git/info/refs").unwrap();
        assert_eq!(r.repo_name, "repo.git");
        assert!(matches!(r.action, GitAction::AdvertiseRefs));
    }

    #[test]
    fn test_parse_path_upload_pack() {
        let r = parse_git_path("/v2/repo.git/git-upload-pack").unwrap();
        assert_eq!(r.repo_name, "repo.git");
        assert!(matches!(r.action, GitAction::Rpc(GitService::UploadPack)));
    }

    #[test]
    fn test_parse_path_receive_pack() {
        let r = parse_git_path("/v2/repo.git/git-receive-pack").unwrap();
        assert_eq!(r.repo_name, "repo.git");
        assert!(matches!(r.action, GitAction::Rpc(GitService::ReceivePack)));
    }

    #[test]
    fn test_parse_path_no_v2_prefix() {
        let r = parse_git_path("/repo.git/info/refs").unwrap();
        assert_eq!(r.repo_name, "repo.git");
    }

    #[test]
    fn test_parse_path_traversal_blocked() {
        assert!(parse_git_path("/v2/../repo.git/info/refs").is_none());
        assert!(parse_git_path("/v2/a/../../b/info/refs").is_none());
    }

    #[test]
    fn test_parse_path_illegal_traversal() {
        assert!(parse_git_path("/../etc/passwd/info/refs").is_none());
        assert!(parse_git_path("/../etc/passwd/git-upload-pack").is_none());
        assert!(parse_git_path("/../etc/passwd/git-receive-pack").is_none());
        assert!(parse_git_path("/etc/passwd/../repo.git/info/refs").is_none());
        assert!(parse_git_path("/../../../etc/shadow/info/refs").is_none());
        assert!(parse_git_path("/repo.git/../../etc/passwd/info/refs").is_none());
        assert!(parse_git_path("/..%2f..%2fetc/passwd/info/refs").is_none());
    }

    #[test]
    fn test_parse_path_windows_traversal() {
        assert!(parse_git_path("/..\\..\\windows\\system32/info/refs").is_none());
        assert!(parse_git_path("/..\\etc\\passwd/git-upload-pack").is_none());
    }

    #[test]
    fn test_parse_path_empty_repo() {
        assert!(parse_git_path("/v2//info/refs").is_none());
    }

    #[test]
    fn test_parse_path_unknown_suffix() {
        assert!(parse_git_path("/v2/repo.git/unknown").is_none());
        assert!(parse_git_path("/v2/repo.git/").is_none());
    }

    // ── verify_repo_path ────────────────────────────────────────────

    #[test]
    fn test_verify_repo_path_valid() {
        let root = std::env::temp_dir().join("githttp_test_root");
        std::fs::create_dir_all(&root).unwrap();

        let repo_dir = root.join("test.git");
        std::fs::create_dir_all(&repo_dir).unwrap();

        let result = verify_repo_path(&root, "test.git");
        assert!(result.is_ok());
        let result_path = result.unwrap();
        assert_eq!(result_path, root.join("test.git"));

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn test_verify_repo_path_nonexistent_repo() {
        let root = std::env::temp_dir().join("githttp_test_root2");
        std::fs::create_dir_all(&root).unwrap();

        let result = verify_repo_path(&root, "new.git");
        assert!(result.is_err());

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn test_verify_repo_path_traversal_escaped() {
        let root = std::env::temp_dir().join("githttp_test_root3");
        std::fs::create_dir_all(&root).unwrap();

        // Try to escape via .. (should be caught by canonicalize check)
        let result = verify_repo_path(&root, "../escape.git");
        assert!(result.is_err());

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn test_verify_repo_path_nonexistent_root() {
        let root = std::env::temp_dir().join("githttp_nonexistent_root");
        // Don't create the root
        let result = verify_repo_path(&root, "test.git");
        assert!(result.is_err());
    }

    #[test]
    fn test_verify_repo_path_absolute_escape() {
        let root = std::env::temp_dir().join("githttp_test_root4");
        std::fs::create_dir_all(&root).unwrap();

        // Absolute path attempt
        let abs_path = if cfg!(windows) {
            "C:\\Windows\\System32"
        } else {
            "/etc/passwd"
        };
        let result = verify_repo_path(&root, abs_path);
        assert!(result.is_err());

        std::fs::remove_dir_all(&root).ok();
    }

    // ── service_name ────────────────────────────────────────────────

    #[test]
    fn test_service_name_upload_pack() {
        assert_eq!(service_name(&GitService::UploadPack), "upload-pack");
    }

    #[test]
    fn test_service_name_receive_pack() {
        assert_eq!(service_name(&GitService::ReceivePack), "receive-pack");
    }

    // ── content_type helpers ────────────────────────────────────────

    #[test]
    fn test_content_type_advertisement() {
        assert_eq!(
            content_type_advertisement(&GitService::UploadPack),
            "application/x-git-upload-pack-advertisement"
        );
        assert_eq!(
            content_type_advertisement(&GitService::ReceivePack),
            "application/x-git-receive-pack-advertisement"
        );
    }

    #[test]
    fn test_content_type_result() {
        assert_eq!(
            content_type_result(&GitService::UploadPack),
            "application/x-git-upload-pack-result"
        );
    }

    #[test]
    fn test_content_type_request() {
        assert_eq!(
            content_type_request(&GitService::UploadPack),
            "application/x-git-upload-pack-request"
        );
    }

    // ── is_protocol_v2 ──────────────────────────────────────────────

    #[test]
    fn test_is_protocol_v2_with_header() {
        let mut h = HeaderMap::new();
        h.insert("Git-Protocol", "version=2".parse().unwrap());
        assert!(is_protocol_v2(&h));
    }

    #[test]
    fn test_is_protocol_v2_no_header() {
        let h = HeaderMap::new();
        assert!(!is_protocol_v2(&h));
    }

    #[test]
    fn test_is_protocol_v2_wrong_version() {
        let mut h = HeaderMap::new();
        h.insert("Git-Protocol", "version=1".parse().unwrap());
        assert!(!is_protocol_v2(&h));
    }

    #[test]
    fn test_is_protocol_v2_case_insensitive_header() {
        let mut h = HeaderMap::new();
        h.insert("git-protocol", "version=2".parse().unwrap());
        assert!(is_protocol_v2(&h));
    }

    // ── build_service_pkt_line ──────────────────────────────────────

    #[test]
    fn test_service_pkt_line_upload_pack() {
        let line = build_service_pkt_line(&GitService::UploadPack);
        assert_eq!(line, "001e# service=git-upload-pack\n0000");
    }

    #[test]
    fn test_service_pkt_line_receive_pack() {
        let line = build_service_pkt_line(&GitService::ReceivePack);
        assert_eq!(line, "001f# service=git-receive-pack\n0000");
    }

    #[test]
    fn test_service_pkt_line_length_prefix() {
        let line = build_service_pkt_line(&GitService::UploadPack);
        let pkt_len = u32::from_str_radix(&line[..4], 16).unwrap() as usize;
        assert!(pkt_len >= 4, "pkt-len must be at least 4");
        let pkt = &line[..pkt_len];
        assert_eq!(pkt.len(), pkt_len, "packet size matches pkt-len header");
        let flush = &line[pkt_len..];
        assert_eq!(flush, "0000", "flush packet follows data pkt");
    }
}
