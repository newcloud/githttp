//! githttp integration tests (native + cgi backends).
//!
//! Starts a real HTTP server and runs `git` client commands against it.
//! Both backends share `/*path` route, selected via `backend` config field.
//! Requires `git` in PATH. Run with:
//!   cargo test --test workflow -- --test-threads=1

use std::io::Write;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use std::{env, fs};

// ── helpers ──────────────────────────────────────────────────────

static SERIAL: Mutex<()> = Mutex::new(());
const PORT: u16 = 18113;
const AUTH: &str = "testuser:pass";

fn repo_url(repo: &str) -> String {
    format!("http://{}@127.0.0.1:{PORT}/{repo}", AUTH)
}

fn fail_url(repo: &str) -> String {
    format!("http://testuser:wrong@127.0.0.1:{PORT}/{repo}")
}

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_githttp"))
}

fn git<I, S>(args: I) -> Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    Command::new("git")
        .args(args)
        .output()
        .expect("git command failed")
}

fn git_cwd<I, S>(dir: &Path, args: I) -> Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git command failed")
}

fn mkfile(dir: &Path, name: &str, content: &str) {
    fs::write(dir.join(name), content).unwrap();
}

fn init_bare(path: &Path) {
    let out = git(["init", "--bare", &path.to_string_lossy()]);
    assert!(
        out.status.success(),
        "init --bare: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn init_repo_with(name: &str, dir: &Path, files: &[(&str, &str)]) {
    let out = git(["init", &dir.to_string_lossy()]);
    assert!(
        out.status.success(),
        "init {}: {}",
        name,
        String::from_utf8_lossy(&out.stderr)
    );
    for (f, c) in files {
        mkfile(dir, f, c);
    }
    let out = git_cwd(dir, ["add", "."]);
    assert!(
        out.status.success(),
        "add {}: {}",
        name,
        String::from_utf8_lossy(&out.stderr)
    );
    git_commit(dir, name);
}

fn git_commit(dir: &Path, msg: &str) {
    let out = Command::new("git")
        .args(["-c", "user.name=Test", "-c", "user.email=t@t.com"])
        .arg("commit")
        .args(["-m", msg])
        .current_dir(dir)
        .output()
        .expect("commit failed");
    assert!(
        out.status.success(),
        "commit '{}': {}",
        msg,
        String::from_utf8_lossy(&out.stderr)
    );
}

fn push_to_bare(src: &Path, bare: &Path) {
    let out = git_cwd(src, ["remote", "add", "origin", &bare.to_string_lossy()]);
    assert!(
        out.status.success(),
        "remote add: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = git_cwd(src, ["push", "-u", "origin", "master"]);
    assert!(
        out.status.success(),
        "push: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn push_branch(src: &Path, branch: &str) {
    let out = git_cwd(src, ["push", "-u", "origin", branch]);
    assert!(
        out.status.success(),
        "push branch {}: {}",
        branch,
        String::from_utf8_lossy(&out.stderr)
    );
}

fn wait_for_server(timeout: Duration) {
    let start = Instant::now();
    loop {
        if TcpStream::connect(("127.0.0.1", PORT)).is_ok() {
            return;
        }
        if start.elapsed() > timeout {
            panic!("Server not ready within {timeout:?}");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn yaml_escape(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "\\\\")
}

fn detect_git_backend() -> String {
    if let Ok(out) = Command::new("git").args(["--exec-path"]).output()
        && out.status.success()
    {
        let exec = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let mut pb = PathBuf::from(&exec);
        pb.push("git-http-backend");
        if cfg!(windows) {
            pb.set_extension("exe");
        }
        if pb.exists() {
            return yaml_escape(&pb);
        }
    }
    let fallback = if cfg!(windows) {
        r"C:\Program Files\Git\mingw64\libexec\git-core\git-http-backend.exe"
    } else {
        "/usr/lib/git-core/git-http-backend"
    };
    yaml_escape(&PathBuf::from(fallback))
}

// ── test env ──────────────────────────────────────────────────────

struct TestEnv {
    root: PathBuf,
    server: Option<Child>,
    config_native: PathBuf,
    config_cgi: PathBuf,
}

impl TestEnv {
    fn new() -> Self {
        let root = {
            let mut p =
                PathBuf::from(env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| "target".into()));
            p.push("test-workflow");
            if !p.is_absolute() {
                p = env::current_dir().unwrap().join(&p);
            }
            p
        };
        if root.exists() {
            fs::remove_dir_all(&root).ok();
        }
        fs::create_dir_all(root.join("repos")).unwrap();
        fs::create_dir_all(root.join("tmp")).unwrap();

        let git_backend = detect_git_backend();
        let repos = root.join("repos");

        // native config: no git_http_backend needed
        let config_native = root.join("config-native.yaml");
        let native_yaml = format!(
            "listen_addr: \"127.0.0.1:{PORT}\"\ngit_project_root: \"{root}\"\nusers: {{}}\nbackend: \"native\"\nlogging:\n  file_enabled: false\n  log_dir: \"logs\"\n",
            root = yaml_escape(&repos),
        );
        fs::write(&config_native, native_yaml).unwrap();

        // cgi config: needs git_http_backend
        let config_cgi = root.join("config-cgi.yaml");
        let cgi_yaml = format!(
            "listen_addr: \"127.0.0.1:{PORT}\"\ngit_project_root: \"{root}\"\ngit_http_backend: \"{backend}\"\nusers: {{}}\nbackend: \"cgi\"\nlogging:\n  file_enabled: false\n  log_dir: \"logs\"\n",
            root = yaml_escape(&repos),
            backend = git_backend,
        );
        fs::write(&config_cgi, cgi_yaml).unwrap();

        Self {
            root,
            server: None,
            config_native,
            config_cgi,
        }
    }

    fn repo(&self) -> PathBuf {
        self.root.join("repos").join("test.git")
    }

    fn tmp(&self, name: &str) -> PathBuf {
        self.root.join("tmp").join(name)
    }

    fn stop_server(&mut self) {
        if let Some(mut c) = self.server.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }

    fn start_server(&mut self, backend: &str) {
        let config = match backend {
            "native" => &self.config_native,
            "cgi" => &self.config_cgi,
            _ => panic!("unknown backend: {}", backend),
        };

        // Add a real user so auth enforces credentials
        let mut child = Command::new(bin())
            .arg("adduser")
            .arg("testuser")
            .arg(config)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("adduser");
        {
            let mut stdin = child.stdin.take().expect("adduser stdin");
            stdin.write_all(b"pass\npass\n").expect("write password");
        }
        let status = child.wait().expect("wait adduser");
        assert!(status.success(), "adduser failed");

        let child = Command::new(bin())
            .arg("-c")
            .arg(config)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start server");
        self.server = Some(child);
        wait_for_server(Duration::from_secs(10));
    }

    fn has_file(&self, name: &str, file: &str) -> bool {
        self.tmp(name).join(file).exists()
    }
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        if let Some(mut c) = self.server.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

// ── test helpers ──────────────────────────────────────────────────

struct TestCtx<'a> {
    env: &'a mut TestEnv,
    url: String,
    backend_label: &'static str,
}

impl<'a> TestCtx<'a> {
    fn run_native_scenarios(env: &'a mut TestEnv) {
        let url = repo_url("test.git");
        let ctx = TestCtx {
            env,
            url,
            backend_label: "native",
        };

        // 1. Clone
        let c1 = ctx.clone("clone1");
        assert!(ctx.has("clone1", "readme.txt"), "T1 cloned file exists");

        // 2. Push
        mkfile(&c1, "hello.txt", "world");
        git_cwd(&c1, ["add", "."]);
        git_commit(&c1, "add hello.txt");
        let out = git_cwd(&c1, ["push"]);
        assert!(
            out.status.success(),
            "T2 push: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let _c1v = ctx.clone("clone1-verify");
        assert!(ctx.has("clone1-verify", "hello.txt"), "T2 pushed file exists");

        // 3. Pull (seed push → c1 pull)
        let seed = ctx.env.tmp("seed");
        let out = git_cwd(&seed, ["pull", "--rebase"]);
        assert!(
            out.status.success(),
            "T3 seed rebase: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        mkfile(&seed, "from-seed.txt", "seed content");
        git_cwd(&seed, ["add", "."]);
        git_commit(&seed, "from seed");
        git_cwd(&seed, ["push"]);
        let out = git_cwd(&c1, ["pull"]);
        assert!(
            out.status.success(),
            "T3 pull: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(c1.join("from-seed.txt").exists(), "T3 pulled file exists");

        // 4. Create & push branch
        git_cwd(&c1, ["checkout", "-b", "feature"]);
        mkfile(&c1, "feature.txt", "feat");
        git_cwd(&c1, ["add", "."]);
        git_commit(&c1, "feature branch");
        push_branch(&c1, "feature");

        // 5. Clone specific branch
        let cfeat = ctx.clone_branch("clone-feature", "feature");
        assert!(cfeat.join("feature.txt").exists(), "T5 branch file exists");

        // 6. Checkout / switch branches
        git_cwd(&c1, ["checkout", "master"]);
        assert!(c1.join("readme.txt").exists(), "T6 master file exists");
        assert!(!c1.join("feature.txt").exists(), "T6 master lacks feature file");
        git_cwd(&c1, ["checkout", "feature"]);
        assert!(c1.join("feature.txt").exists(), "T6 feature file exists");

        // 7. Fetch + merge
        mkfile(&seed, "merge-me.txt", "merge content");
        git_cwd(&seed, ["add", "."]);
        git_commit(&seed, "prepare merge");
        git_cwd(&seed, ["push", "origin", "master"]);
        git_cwd(&c1, ["checkout", "master"]);
        git_cwd(&c1, ["pull"]);
        assert!(c1.join("merge-me.txt").exists(), "T7 pulled merge file");
        git_cwd(&c1, ["checkout", "feature"]);
        git_cwd(&c1, ["merge", "master"]);
        assert!(c1.join("merge-me.txt").exists(), "T7 merged file exists");
        git_cwd(&c1, ["push"]);

        // 8. Protocol v2 explicit
        let cp2 = ctx.clone_proto2("clone-proto2");
        assert!(cp2.join("readme.txt").exists(), "T8 cloned file exists");
        assert!(cp2.join("hello.txt").exists(), "T8 pushed file exists");

        // 9. Auth failure
        ctx.clone_should_fail("clone-auth-fail", "test.git");

        // 10. Nonexistent repo
        ctx.clone_should_fail_repo("clone-nonexistent", "nonexistent.git");

        // 11. Delete remote branch
        git_cwd(&c1, ["push", "origin", "--delete", "feature"]);
        let cdel = ctx.clone("clone-after-delete");
        let out = git_cwd(&cdel, ["ls-remote", "--heads", "origin", "feature"]);
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(!stdout.contains("feature"), "T11 feature branch deleted");
    }

    fn run_cgi_scenarios(env: &'a mut TestEnv) {
        let url = repo_url("test.git");
        let ctx = TestCtx {
            env,
            url,
            backend_label: "cgi",
        };

        // Basic clone
        let c1 = ctx.clone("clone-cgi");
        assert!(ctx.has("clone-cgi", "readme.txt"), "T12 cgi clone file exists");

        // Basic push
        mkfile(&c1, "cgi-test.txt", "cgi push test");
        git_cwd(&c1, ["add", "."]);
        git_commit(&c1, "cgi push");
        let out = git_cwd(&c1, ["push"]);
        assert!(
            out.status.success(),
            "T13 cgi push: {}",
            String::from_utf8_lossy(&out.stderr)
        );

        // Verify push via fresh clone
        let _cv = ctx.clone("clone-cgi-verify");
        assert!(ctx.has("clone-cgi-verify", "cgi-test.txt"), "T14 cgi verify");

        // CGI auth failure
        ctx.clone_should_fail("clone-cgi-auth-fail", "test.git");
    }

    fn clone(&self, name: &str) -> PathBuf {
        let dst = self.env.tmp(name);
        let dst_str = dst.to_string_lossy().into_owned();
        let out = git(["clone", &self.url, &dst_str]);
        assert!(
            out.status.success(),
            "{} clone {}: {}",
            self.backend_label,
            name,
            String::from_utf8_lossy(&out.stderr)
        );
        dst
    }

    fn clone_branch(&self, name: &str, branch: &str) -> PathBuf {
        let dst = self.env.tmp(name);
        let dst_str = dst.to_string_lossy().into_owned();
        let out = git([
            "clone",
            "-b",
            branch,
            "--single-branch",
            &self.url,
            &dst_str,
        ]);
        assert!(
            out.status.success(),
            "{} clone {} branch {}: {}",
            self.backend_label,
            name,
            branch,
            String::from_utf8_lossy(&out.stderr)
        );
        dst
    }

    fn clone_proto2(&self, name: &str) -> PathBuf {
        let dst = self.env.tmp(name);
        let dst_str = dst.to_string_lossy().into_owned();
        let out = Command::new("git")
            .arg("-c")
            .arg("protocol.version=2")
            .arg("clone")
            .arg(&self.url)
            .arg(&dst_str)
            .output()
            .expect("protocol v2 clone");
        assert!(
            out.status.success(),
            "{} protocol v2 clone {}: {}",
            self.backend_label,
            name,
            String::from_utf8_lossy(&out.stderr)
        );
        dst
    }

    fn clone_should_fail(&self, name: &str, repo: &str) {
        let dst = self.env.tmp(name);
        let dst_str = dst.to_string_lossy().into_owned();
        let out = git(["clone", &fail_url(repo), &dst_str]);
        assert!(!out.status.success(), "{} auth should fail", self.backend_label);
    }

    fn clone_should_fail_repo(&self, name: &str, repo: &str) {
        let dst = self.env.tmp(name);
        let dst_str = dst.to_string_lossy().into_owned();
        let out = git(["clone", &repo_url(repo), &dst_str]);
        assert!(
            !out.status.success(),
            "{} nonexistent repo should fail",
            self.backend_label
        );
    }

    fn has(&self, name: &str, file: &str) -> bool {
        self.env.has_file(name, file)
    }
}

// ── test entry ────────────────────────────────────────────────────

/// Full integration test: native then cgi backend.
/// Run with: `cargo test --test workflow -- --test-threads=1`
///
/// Scenarios:
///   native: clone, push, pull, branch create/clone/switch, fetch+merge, protocol v2,
///           auth failure, nonexistent repo, delete remote branch
///   cgi:    clone, push, verify, auth failure
#[test]
fn test_full_workflow() {
    let _lock = SERIAL.lock().unwrap();
    let mut env = TestEnv::new();

    // Setup: create bare repo with initial content
    let bare = env.repo();
    init_bare(&bare);
    let seed = env.tmp("seed");
    init_repo_with("seed", &seed, &[("readme.txt", "hello")]);
    push_to_bare(&seed, &bare);

    // ── Native backend ───────────────────────────────────────────
    env.start_server("native");
    TestCtx::run_native_scenarios(&mut env);
    env.stop_server();

    // ── CGI backend ──────────────────────────────────────────────
    env.start_server("cgi");
    TestCtx::run_cgi_scenarios(&mut env);
    env.stop_server();

    eprintln!("All integration scenarios PASSED (native + cgi)");
}
