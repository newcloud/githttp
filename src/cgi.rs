use axum::{
    body::Body,
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode},
    response::Response,
};
use std::str::FromStr;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio_util::io::ReaderStream;
use tracing::warn;

/// Parses CGI response headers from a buffered reader.
///
/// Reads lines until an empty line (header/body separator) is found.
/// Returns the parsed headers and the remaining BufReader (positioned at body start).
pub async fn parse_cgi_headers<R>(
    mut reader: BufReader<R>,
) -> Result<(HeaderMap, BufReader<R>), StatusCode>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut headers = HeaderMap::new();
    let mut line = String::new();

    loop {
        line.clear();
        let bytes_read = reader.read_line(&mut line).await.map_err(|e| {
            warn!("Failed to read CGI header line: {}", e);
            StatusCode::BAD_GATEWAY
        })?;

        if bytes_read == 0 {
            break;
        }

        let trimmed = line.trim_end_matches(['\r', '\n']);

        if trimmed.is_empty() {
            break;
        }

        if let Some((key, value)) = trimmed.split_once(':') {
            let key = key.trim();
            let value = value.trim();

            if key.eq_ignore_ascii_case("Status") {
                let status_code = value.split_whitespace().next().unwrap_or("200");
                let code = status_code.parse::<u16>().unwrap_or(200);
                headers.insert(
                    HeaderName::from_static("x-cgi-status"),
                    HeaderValue::from_str(&code.to_string()).unwrap(),
                );
            } else if !key.eq_ignore_ascii_case("Content-Length")
                && let (Ok(name), Ok(val)) =
                    (HeaderName::from_str(key), HeaderValue::from_str(value))
            {
                headers.insert(name, val);
            }
        }
    }

    Ok((headers, reader))
}

/// Converts a BufReader (positioned at body start) into an axum Response with streaming body.
#[allow(dead_code)]
pub fn streaming_response(
    headers: HeaderMap,
    reader: impl tokio::io::AsyncRead + Unpin + Send + 'static,
) -> Result<Response, StatusCode> {
    let status = if let Some(val) = headers.get("x-cgi-status") {
        let code_str = val.to_str().unwrap_or("200");
        StatusCode::from_u16(code_str.parse().unwrap_or(200)).unwrap_or(StatusCode::OK)
    } else {
        StatusCode::OK
    };

    let mut response_builder = Response::builder().status(status);

    for (name, value) in headers.iter() {
        if name != "x-cgi-status" {
            response_builder = response_builder.header(name, value);
        }
    }

    let stream = ReaderStream::new(reader);
    let body = Body::from_stream(stream);

    response_builder.body(body).map_err(|e| {
        warn!("Failed to build response: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })
}
