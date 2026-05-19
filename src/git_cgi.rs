use axum::{
    body::Body,
    extract::{Request, State},
    http::StatusCode,
    response::Response,
};
use bytes::Bytes;
use futures_core::Stream;
use tokio_stream::StreamExt;

type BoxStream<'a, T> = Pin<Box<dyn Stream<Item = T> + Send + 'a>>;

use std::io;
use std::pin::Pin;
use std::process::Stdio;
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::io::AsyncWriteExt;
use tokio::process::Command as TokioCommand;
use tokio_util::io::ReaderStream;
use tracing::{error, info, warn};

use crate::auth;
use crate::cgi;
use crate::config::Config;

pub struct AppState {
    pub config: Config,
}

struct GuardedStream {
    inner: BoxStream<'static, Result<Bytes, io::Error>>,
    kill_tx: Option<tokio::sync::broadcast::Sender<()>>,
}

impl Stream for GuardedStream {
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

pub async fn git_handler_cgi(
    State(state): State<Arc<AppState>>,
    req: Request,
) -> Result<Response, StatusCode> {
    let method = req.method().to_string();
    let uri = req.uri().to_string();
    let remote_addr = req
        .extensions()
        .get::<std::net::SocketAddr>()
        .map(|a| a.to_string())
        .unwrap_or_else(|| "unknown".to_string());

    info!("Request: {} {} from {}", method, uri, remote_addr);

    let remote_user = match auth::verify_auth(&req, &state.config.users) {
        Some(user) => {
            info!("Auth success for user: {}", user);
            user
        }
        None => {
            warn!("Auth failed for request: {} {}", method, uri);
            return Ok(auth::unauthorized_response());
        }
    };

    let backend_path = state.config.resolve_git_http_backend();
    let backend_path = backend_path.to_str().ok_or_else(|| {
        error!("Invalid backend path: {:?}", backend_path);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let root_path_str = state.config.git_project_root.to_str().ok_or_else(|| {
        error!(
            "Invalid project root path: {:?}",
            state.config.git_project_root
        );
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let path = req.uri().path();
    if path.contains("..") || path.contains("//") || path.contains('\\') {
        warn!("cgi Path traversal attempt in request: {}", path);
        return Err(StatusCode::FORBIDDEN);
    }

    let mut cmd = TokioCommand::new(backend_path);
    cmd.env("GIT_PROJECT_ROOT", root_path_str)
        .env("GIT_HTTP_EXPORT_ALL", "")
        .env("PATH_INFO", req.uri().path())
        .env("REQUEST_METHOD", req.method().as_str())
        .env("QUERY_STRING", req.uri().query().unwrap_or(""))
        .env("REMOTE_USER", &remote_user)
        .env("SERVER_PROTOCOL", "HTTP/1.1")
        .env("GATEWAY_INTERFACE", "CGI/1.1");

    if let Some(content_type) = req.headers().get("Content-Type")
        && let Ok(ct) = content_type.to_str()
    {
        cmd.env("CONTENT_TYPE", ct);
    }

    if let Some(content_length) = req.headers().get("Content-Length")
        && let Ok(len) = content_length.to_str()
    {
        cmd.env("CONTENT_LENGTH", len);
    }

    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());

    let mut child = cmd.spawn().map_err(|e| {
        error!("Failed to spawn git-http-backend: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    info!("Spawned git-http-backend for: {}", uri);

    // === Cancellation channel: ties child lifecycle to response stream ===
    let (cancel_tx, _) = tokio::sync::broadcast::channel::<()>(1);
    let mut cancel_stdin = cancel_tx.subscribe();
    let mut cancel_child = cancel_tx.subscribe();

    // === INBOUND STREAMING: Request body → child stdin ===
    let mut stdin = child.stdin.take().ok_or_else(|| {
        error!("Failed to take stdin");
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
                                    warn!("Error streaming request body to git-http-backend: {}", e);
                                }
                                return;
                            }
                        }
                        Some(Err(e)) => {
                            warn!("Error reading request body: {}", e);
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

    // === OUTBOUND STREAMING: Child stdout → Response body ===
    let stdout = child.stdout.take().ok_or_else(|| {
        error!("Failed to take stdout");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let buf_reader = tokio::io::BufReader::new(stdout);
    let (headers, remaining_reader) = cgi::parse_cgi_headers(buf_reader).await?;

    // Wait child + kill on connection close
    let mut child_for_wait = child;
    tokio::spawn(async move {
        tokio::select! {
            biased;
            _ = cancel_child.recv() => {
                let _ = child_for_wait.start_kill();
                match child_for_wait.wait().await {
                    Ok(s) => info!("git-http-backend killed due to connection close, exit: {}", s),
                    Err(e) => warn!("git-http-backend killed due to connection close: {}", e),
                }
            }
            status = child_for_wait.wait() => {
                let _ = stdin_task.await;
                match status {
                    Ok(s) if !s.success() => {
                        warn!("git-http-backend exited with status: {}", s);
                    }
                    Err(e) => {
                        error!("Failed to wait for git-http-backend: {}", e);
                    }
                    _ => {}
                }
            }
        }
    });

    // === Build response with GuardedStream ===
    info!("Starting streaming response for: {}", uri);

    let stream = ReaderStream::new(remaining_reader);
    let guarded_stream = GuardedStream {
        inner: Box::pin(stream),
        kill_tx: Some(cancel_tx),
    };
    let body = Body::from_stream(guarded_stream);

    let status = headers
        .get("x-cgi-status")
        .and_then(|v| v.to_str().ok()?.parse::<u16>().ok())
        .map(|c| StatusCode::from_u16(c).unwrap_or(StatusCode::OK))
        .unwrap_or(StatusCode::OK);

    let mut response_builder = Response::builder().status(status);
    for (name, value) in headers.iter() {
        if name != "x-cgi-status" {
            response_builder = response_builder.header(name, value);
        }
    }

    response_builder.body(body).map_err(|e| {
        warn!("Failed to build response: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })
}
