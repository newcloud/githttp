# AGENTS.md — githttp

## Project

Single-binary Rust HTTP server that serves Git repositories over HTTP.

- **cgi backend:** proxies to `git-http-backend` (CGI)
- **native backend:** calls `git upload-pack` / `git receive-pack` directly — no CGI
- Both share `/*path` route, selected via `backend` config field

## Commands

```
cargo build          # dev
cargo build --release
cargo run            # start server (reads config.yaml from cwd)
cargo run -- -c /path/to/config.yaml   # custom config with --config/-c flag
cargo run -- -q -c myconfig.yaml       # quiet mode (no terminal logs)

# User management (CLI subcommands)
cargo run -- adduser <username> [config.yaml]       # add user (prompts for password)
cargo run -- setpassword <username> [config.yaml]   # change password
cargo run -- deluser <username> [config.yaml]       # delete user

# CLI flags (server mode)
#   -c / --config <path>    config file path
#   --config=<path>         equivalent
#   -c=<path>               equivalent
#   -q / --quiet            suppress terminal log output
```

## Architecture

- **Entry point:** `src/main.rs` — CLI parsing, config loading, logging init, module assembly
- **Modules:**
  - `src/config.rs` — Config struct, `LoggingConfig`, path detection, serialization
  - `src/auth.rs` — Basic Auth verification, SHA-256 password hashing
  - `src/git_cgi.rs` — git-http-backend CGI proxy handler, streaming I/O orchestration
  - `src/git_native.rs` — Native handler: calls `git upload-pack` / `git receive-pack` directly
  - `src/cgi.rs` — Streaming CGI response header parser + body Response construction
  - `src/users.rs` — CLI user management commands (add/setpassword/del)
- **Flow (cgi):** HTTP request → Basic Auth verify (SHA-256) → spawn `git-http-backend` → spawn stdin writer (DataStream chunks → child stdin) → stream stdout (BufReader → read_line header parse → ReaderStream → Body::from_stream) → return streaming Response
- **Flow (native):** HTTP request → Basic Auth verify (SHA-256) → spawn `git upload-pack` / `git receive-pack` directly with `--advertise-refs` or `--stateless-rpc` → stream stdin/stdout (no CGI header parsing) → return streaming Response
- **Config:** `config.yaml` (serde_yaml). See `Config` struct in `config.rs` for schema.
  - `git_http_backend` is `Option<PathBuf>` in config. When `None` and `backend=cgi`, auto-detect via `resolve_git_http_backend()`. When `backend=native`, it's ignored (returns empty `PathBuf`).
  - `logging.file_enabled: bool` (default `false`) — whether to write logs to file.
  - `logging.log_dir: PathBuf` (default `"logs"`) — log directory.
- **Config fallback:** `Config::from_file()` returns `Option<Self>`. Missing/invalid file → auto-detected defaults.
- **CLI parsing:** Manual arg parsing in `parse_args()` in `main.rs`. Flags: `-q`/`--quiet`, `-c`/`--config` (with `=` syntax). Subcommands: `adduser`, `setpassword`, `deluser`, or default server mode.
- **Logging init:** Config is loaded *before* `tracing_subscriber::init()`. Terminal layer is skipped when `--quiet`; file layer is only added when `logging.file_enabled=true`.

## Key gotchas

- **FULL STREAMING** — no `axum::body::to_bytes()` or `wait_with_output()`. Request body chunks flow through `tokio::spawn` → stdin; stdout flows through `BufReader` → `ReaderStream` → axum Body. Memory stays at ~KB regardless of file size.
- **`CONTENT_TYPE` must be forwarded from the incoming request**, not hardcoded. Hardcoding caused 415 errors on push.
 - **`users` empty = authentication denied** — you must add at least one user with `adduser`.
- **Passwords stored as SHA-256 hashes** — format: `$sha256$<salt_hex>$<hash_hex>`
- **`Content-Length` header is stripped from CGI output** — axum sets it automatically for streaming bodies.
- **Edition is 2024** — do not downgrade.
- **`config.yaml` is in `.gitignore`** — users copy `config.example.yaml` to create it.
- **Password input**: interactive mode uses `rpassword` (hidden), piped mode reads from stdin (two lines: password + confirm).
- **BrokenPipe on stdin** is expected and silently handled — child may exit early before all body is sent.
- **`CGI parser`** uses `BufReader::read_line()` not `windows(4)`. Handles both `\r\n` and `\n` by `trim_end_matches`.
- **Route** is `/*path`. No other paths.
- **v2 Content-Type** is set by code, not parsed from CGI. For ref advertisement: `application/x-git-*-advertisement`. For RPC result: `application/x-git-*-result`.
- **v2 protocol v2** (`Git-Protocol: version=2` header) skips the `# service=git-xxx-pack` prefix and bypasses service header entirely.
- **`parse_git_path`**: extracts repo name via `rsplit('/')` — path-prefix agnostic (handles `/v2/repo.git` and `/repo.git` equally). Rejects `..`, `//`, `\\` early for security.
- **`GuardedStream`**: wraps response body, sends kill signal on Drop (connection close). Kill log level is conditional: `INFO` if child already exited normally (expected for GET /info/refs, small repos), `WARN` only if child was forcibly killed with non-zero exit.
- **`verify_repo_path`**: shared between native and CGI backends. Uses canonicalize to verify the resolved path stays within `git_project_root`. Returns original path (not canonicalized) for compatibility with git command on Windows.
- **`detect_git_executable`**: auto-detects git.exe on Windows by scanning common paths (`D:\Program Files\Git\cmd`, `D:\Program Files\Git\bin`, etc.). Falls back to "git" if not found.

## Testing

Unit tests (26 in `src/git_native.rs`) + integration test (`tests/workflow.rs`):

```bash
cargo test
cargo clippy --all-targets -- -D warnings
```

End-to-end functional test with real git-http-backend:

```bash
# Start server, add user, test clone/push, test auth failure
git init --bare /path/to/repos/test.git
./target/release/githttp -c config.yaml &
echo -e "pass\npass" | ./target/release/githttp adduser user config.yaml
kill %1 && ./target/release/githttp -c config.yaml &
git clone http://user:pass@127.0.0.1:18011/test.git
cd test && touch f && git add . && git commit -m "x" && git push origin master
cd .. && git clone http://user:pass@127.0.0.1:18011/test.git test2
git clone http://user:wrong@127.0.0.1:18011/test.git fail  # expect auth failure
kill %1
```

### v2 end-to-end test plan

Run v1 tests first, then v2 on the same infrastructure:

**1. Basic clone/fetch (upload-pack)**

```bash
git clone http://user:pass@127.0.0.1:18011/test.git test-v2-clone
ls test-v2-clone/f
```

**2. Push (receive-pack)**

```bash
cd test-v2-clone
echo "v2 content" > v2file
git add v2file && git commit -m "v2 push test"
git push origin master
cd ..
git clone http://user:pass@127.0.0.1:18011/test.git test-v2-verify
ls test-v2-verify/v2file
```

**3. Auth failure**

```bash
git clone http://user:wrong@127.0.0.1:18011/test.git v2-fail
# expect: fatal: Authentication failed
```

**4. Protocol v2 test**

```bash
git -c protocol.version=2 clone http://user:pass@127.0.0.1:18011/test.git test-v2-proto
```

**5. Non-existent repo**

```bash
git clone http://user:pass@127.0.0.1:18011/nonexistent.git v2-nonexistent
# expect: fatal: repository not found
```

**6. curl ref advertisement**

```bash
curl -s http://user:pass@127.0.0.1:18011/test.git/info/refs?service=git-upload-pack | head -c 100
# expect: 001e# service=git-upload-pack

curl -s -H "Git-Protocol: version=2" http://user:pass@127.0.0.1:18011/test.git/info/refs?service=git-upload-pack | head -c 100
# expect: no # service= prefix, direct capability advertisement
```

**7. Cross-protocol: v1 and v2 on same repo**

```bash
cd test-v2-clone
echo "cross-protocol" > cross
git add cross && git commit -m "cross protocol test"
git push origin master
cd ..
git clone http://user:pass@127.0.0.1:18011/test.git test-v1-after-v2
ls test-v1-after-v2/cross
```

## Config & paths

- **Use `PathBuf` + `.join()` for all path construction** — never string concatenation.
- `Config.git_http_backend` is `Option<PathBuf>` (not auto-detected by serde default). Call `config.resolve_git_http_backend()` to get the resolved path: returns the configured path, or auto-detects when `backend=cgi` and field is `None`, or returns empty when `backend=native`.
- `detect_git_http_backend()` in `config.rs` auto-detects by scanning common install paths:
  - Windows: `C:\Program Files\Git\mingw64\libexec\git-core\git-http-backend.exe`, `C:\Program Files (x86)\...`
  - Linux: `/usr/libexec/git-core/`, `/usr/lib/git-core/`, `/usr/local/libexec/git-core/`
  - Falls back to first path if none exist.
- `detect_git_executable()` in `config.rs` auto-detects git executable on Windows:
  - Scans: `D:\Program Files\Git\cmd\git.exe`, `D:\Program Files\Git\bin\git.exe`, etc.
  - Falls back to "git" if not found.
- `verify_repo_path()` in `config.rs` validates repo path stays within `git_project_root`:
  - Uses canonicalize to check for symlink/junction escape attempts.
  - Returns original path (not canonicalized) for git command compatibility on Windows.

## Logging

- **CLI flags:** `-q` / `--quiet` suppresses terminal output.
- **Config fields:**
  - `logging.file_enabled: bool` (default `false`) — controls file log output.
  - `logging.log_dir: PathBuf` (default `"logs"`) — log directory, auto-created if file logging is enabled.
- **Terminal:** colored output (skipped when `--quiet`).
- **File:** no ANSI, rolling daily (`githttp.log.YYYY-MM-DD`).
- Uses `tracing` + `tracing-subscriber` + `tracing-appender`.
- `RUST_LOG` env var controls log level.
- Config is loaded before logging init, so `logging` settings are applied correctly.

## Config schema (config.example.yaml)

```yaml
git_project_root: "/path/to/git/repos"
git_http_backend: "/usr/local/libexec/git-core/git-http-backend"  # optional, only for cgi
listen_addr: "0.0.0.0:18011"
users: {}
backend: "native"   # "cgi" | "native"

logging:
  file_enabled: false
  log_dir: "logs"
```

## Git repos

All repos must be **bare** (`git init --bare`) and located under `git_project_root`.

## Release & CI

### GitHub Actions workflow

Workflow file: `.github/workflows/build.yml`

**Triggers:**
- `push` to `master`/`main` — build only
- `push` tag `v*` — build + create GitHub Release
- `pull_request` to `master`/`main` — build only

**Release artifacts:**

| Platform | File |
|----------|------|
| Windows x86_64 | `githttp-x86_64-pc-windows-msvc.zip` |
| Linux x86_64 musl | `githttp-x86_64-unknown-linux-musl.tar.gz` |

Each archive contains: binary + `config.example.yaml`

### How to release

```bash
git tag v0.1.2
git push origin v0.1.2
```

Pushing a `v*` tag triggers the release job automatically. Check progress at `https://github.com/newcloud/githttp/actions`.

### Release profile

`Cargo.toml` has `[profile.release]` optimized for binary size:

```toml
opt-level = "z"
lto = true
codegen-units = 1
strip = true
```

### Key gotchas

- **403 on release**: `release` job needs `permissions: contents: write`. Already configured.
- **Re-tagging**: to re-trigger a failed release, delete and re-create the tag:
  ```bash
  git tag -d vX.Y.Z && git push origin :vX.Y.Z && git tag vX.Y.Z && git push origin vX.Y.Z
  ```
- **Actions versions**: `checkout@v6`, `upload-artifact@v7`, `download-artifact@v5`, `action-gh-release@v3`. Watch for deprecation warnings.
