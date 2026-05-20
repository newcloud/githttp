# Git HTTP Server

> **WARNING: Do not use in production environments! Intended for local, secure environments only.**

A lightweight Git HTTP server built on Axum, supporting two backend modes with full-pipeline streaming.

## Backend Modes

| Mode | Config Value | Description |
|------|-------------|-------------|
| **Native** (default) | `backend: "native"` | Directly invokes `git upload-pack` / `git receive-pack`, no CGI dependency |
| **CGI** | `backend: "cgi"` | Proxies requests through `git-http-backend` CGI program |


## Quick Start

### Build

```bash
cargo build --release
```

### Configuration

**Option 1: Quickstart wizard (recommended)**

Run the interactive setup wizard to generate `config.toml`:

```bash
# Windows
quickstart.bat

# Linux
./quickstart.sh
```

**Option 2: Manual configuration**

Copy and edit `config.example.toml`:

```bash
cp config.example.toml config.toml
```

Edit `config.toml`:

```yaml
git_project_root: "/path/to/git/repos"
listen_addr: "0.0.0.0:18011"
users: {}
backend: "native"

logging:
  file_enabled: false
  log_dir: "logs"
```


### User Management

```bash
# Add a user
githttp adduser admin

# Change password
githttp setpassword admin

# Delete a user
githttp deluser admin

# Specify config file
githttp adduser admin config.toml
```


### Start

```bash
# Reads config.toml by default
githttp

# Specify config file
githttp -c config.toml

# Quiet mode (suppress terminal log output)
githttp -q -c config.yaml
```

## CLI Arguments

| Argument | Description |
|----------|-------------|
| `-c`, `--config <path>` | Specify config file path (default: config.toml) |
| `-q`, `--quiet` | Quiet mode, suppress terminal log output |
| `adduser <username>` | Add a user |
| `setpassword <username>` | Change password |
| `deluser <username>` | Delete a user |

## Usage Examples

```bash
# Create a bare repository
cd /path/to/git/repos
git init --bare my-project.git

# Start the server
githttp -c config.toml

# Clone
git clone http://user:pass@localhost:18011/my-project.git

# Push
cd my-project
echo "# README" > README.md
git add . && git commit -m "init"
git push origin master

git clone http://user:pass@localhost:18011/my-project.git
```

## Config Reference

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `git_project_root` | path | `%USERPROFILE%\repos` (Win) `~/repos` (Linux) | Bare repository root directory |
| `git_http_backend` | path? | None | git-http-backend path, only needed for CGI mode; auto-detected if omitted |
| `git_path` | path? | None | git executable path, only needed for native mode; auto-detected from PATH if omitted |
| `listen_addr` | string | `0.0.0.0:18011` | Listen address and port |
| `users` | map | `{}` | Username → SHA-256 hashed password (generated via `githttp adduser`) |
| `backend` | string | `native` | `"native"` or `"cgi"` |
| `logging.file_enabled` | bool | `false` | Whether to write logs to file |
| `logging.log_dir` | path | `"logs"` | Log file directory |

## Logging

- **Terminal output** (colored): enabled by default, disabled with `-q`/`--quiet`
- **File output** (no ANSI): disabled by default, enabled with `logging.file_enabled: true`
- Daily rolling files: `logs/githttp.log.YYYY-MM-DD`
- Log level controlled via `RUST_LOG` env var (default: `githttp=info`)

## Testing

```bash
# Unit tests (26)
cargo test --bin githttp

# Integration tests
cargo test --test workflow
```

## Architecture

```
Client ←→ Axum HTTP ←→ Backend Handler
                         ├─ Native: git upload-pack / receive-pack (direct invocation)
                         └─ CGI:    git-http-backend (CGI proxy)

Inbound:  Request Body → DataStream → tokio::spawn → child stdin
Outbound: child stdout → ReaderStream → Body::from_stream → Response
```

Full-pipeline streaming: request body chunks are written to the child process as they arrive, and response body is read and returned in real time. Memory footprint stays at ~KB, handling multi-GB pushes and clones without OOM.

## Module Structure

| Module | File | Responsibility |
|--------|------|---------------|
| `config` | `src/config.rs` | Config parsing, backend detection, `detect_git_executable`, `verify_repo_path` |
| `auth` | `src/auth.rs` | Basic Auth verification, SHA-256 password hashing |
| `git_cgi` | `src/git_cgi.rs` | CGI backend: proxy git-http-backend |
| `git_native` | `src/git_native.rs` | Native backend: directly invoke git commands |
| `cgi` | `src/cgi.rs` | Streaming CGI response header parsing and construction |
| `users` | `src/users.rs` | CLI user management commands |

## Tech Stack

- `axum` — HTTP framework
- `tokio` — Async runtime + child process management
- `tokio-util` — `ReaderStream` streaming conversion
- `sha2` — Password hashing (SHA-256)
- `tracing` + `tracing-subscriber` + `tracing-appender` — Logging
- `serde` + `serde_yaml` — Config serialization

## Development

This project uses [OpenCode](https://opencode.ai) for AI-assisted development, with models DeepSeek V4 and Qwen3.6 Plus.
