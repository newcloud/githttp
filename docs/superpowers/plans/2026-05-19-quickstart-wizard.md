# githttp quickstart Wizard — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `githttp quickstart` interactive wizard for first-time setup (repos root, user creation, auto-generate config.yaml).

**Architecture:** New `src/quickstart.rs` module with `run_quickstart()` that drives stdin/stdout prompts. Returns `Option<Config>` — `Some` means user wants to start server, `None` means exit. CLI parsing in `main.rs` gets a new `quickstart` positional subcommand.

**Tech Stack:** Rust (edition 2024), std::io for prompts, rpassword for hidden password input, serde_yaml for config save.

---

### Task 1: Expose `read_password_secure` for reuse

**Files:**
- Modify: `src/users.rs`

- [ ] **Step 1: Make `read_password_secure` public**

```rust
// Change:
fn read_password_secure(prompt: &str) -> String {
// To:
pub fn read_password_secure(prompt: &str) -> String {
```

- [ ] **Step 2: Build check**

Run: `cargo build`
Expected: compiles without errors (no other code depends on this yet)

---

### Task 2: Create `quickstart.bat` and `quickstart.sh`

**Files:**
- Create: `quickstart.bat`
- Create: `quickstart.sh`

- [ ] **Step 1: Create `quickstart.bat`**

```bat
@echo off
githttp quickstart
```

- [ ] **Step 2: Create `quickstart.sh`**

```bash
#!/bin/sh
exec githttp quickstart
```

- [ ] **Step 3: Make `quickstart.sh` executable (Linux/macOS)**

Run: `git update-index --chmod=+x quickstart.sh`

---

### Task 3: Create `src/quickstart.rs` — wizard module

**Files:**
- Create: `src/quickstart.rs`

- [ ] **Step 1: Write the full module**

```rust
use std::io::{self, Write};
use std::path::PathBuf;

use crate::auth::hash_password;
use crate::config::Config;
use crate::users::read_password_secure;

fn read_line() -> String {
    let mut input = String::new();
    io::stdin().read_line(&mut input).expect("Failed to read input");
    input.trim().to_string()
}

fn ask_yes_no(prompt: &str, default_yes: bool) -> bool {
    let hint = if default_yes { "Y/n" } else { "y/N" };
    print!("{} ({}) ", prompt, hint);
    io::stdout().flush().unwrap();
    let answer = read_line().to_lowercase();
    if answer.is_empty() {
        return default_yes;
    }
    answer.starts_with('y')
}

fn print_banner() {
    println!("═══════════════════════════════════════════");
    println!("  Welcome to githttp - Git HTTP Server");
    println!("═══════════════════════════════════════════");
    println!();
    println!("This wizard will help you set up a Git HTTP server.");
    println!("You'll configure:");
    println!("  • Where your Git repositories are stored");
    println!("  • A user account for Git access (clone/push/pull)");
    println!();
    println!("Let's get started!");
    println!();
}

fn step1_repos_root() -> PathBuf {
    loop {
        println!("─────────────────────────────────────────────");
        println!("  Step 1: Git Repository Root Directory");
        println!("─────────────────────────────────────────────");
        println!();
        println!("This is the folder where your Git bare repos will live.");
        println!("A \"bare repo\" is a server-side Git repo (no working files),");
        println!("it's the .git folder you get from: git init --bare <name>.git");
        println!();
        println!("Example: C:\\code\\repos");
        println!("         /home/user/git");
        println!();
        print!("Enter path: ");
        io::stdout().flush().unwrap();
        let path = PathBuf::from(read_line());

        if !path.exists() {
            println!();
            println!("Path does not exist.");
            println!();
            continue;
        }
        if !path.is_dir() {
            println!();
            println!("Path is not a directory.");
            println!();
            continue;
        }

        println!();
        if let Ok(entries) = std::fs::read_dir(&path) {
            let repos: Vec<String> = entries
                .filter_map(|e| e.ok())
                .filter(|e| {
                    e.file_type().map(|t| t.is_dir()).unwrap_or(false)
                        && e.file_name().to_string_lossy().ends_with(".git")
                })
                .map(|e| e.file_name().to_string_lossy().to_string())
                .collect();

            if repos.is_empty() {
                println!("No .git repositories found yet. You can create them later:");
                println!();
                println!("  cd {}", path.display());
                println!("  git init --bare <name>.git");
            } else {
                println!("Found {} repositories:", repos.len());
                for repo in &repos {
                    println!("  • {}", repo);
                }
            }
        }
        println!();
        return path;
    }
}

fn step2_add_user(config: &mut Config) {
    loop {
        println!("─────────────────────────────────────────────");
        println!("  Step 2: Create a User");
        println!("─────────────────────────────────────────────");
        println!();
        println!("This username and password are used with git clone/push.");
        println!();

        print!("Enter username: ");
        io::stdout().flush().unwrap();
        let username = read_line();

        if username.is_empty() {
            println!();
            println!("Username cannot be empty.");
            println!();
            continue;
        }

        if config.users.contains_key(&username) {
            println!();
            println!("User '{}' already exists. Choose another name.", username);
            println!();
            continue;
        }

        let password = read_password_secure("Enter password: ");
        if password.is_empty() {
            println!();
            println!("Password cannot be empty.");
            println!();
            continue;
        }

        let confirm = read_password_secure("Confirm password: ");
        if password != confirm {
            println!();
            println!("Passwords do not match.");
            println!();
            continue;
        }

        let hashed = hash_password(&password).expect("Failed to hash password");
        config.users.insert(username.clone(), hashed);

        println!();
        println!("✓ User '{}' added.", username);
        println!("  Usage: http://{}:<password>@127.0.0.1:18011/repo.git", username);
        println!();
        return;
    }
}

fn step4_summary(config: &Config, config_path: &str) {
    println!("─────────────────────────────────────────────");
    println!("  Setup Complete!");
    println!("─────────────────────────────────────────────");
    println!();
    println!("  Config file     {}", config_path);
    println!("  Repos directory {}", config.git_project_root.display());
    println!("  Listen address  {}", config.listen_addr);

    let user_list: Vec<String> = config.users.keys().cloned().collect();
    if user_list.len() == 1 {
        println!("  Users           1: {}", user_list[0]);
    } else {
        println!("  Users           {}: {}", user_list.len(), user_list.join(", "));
    }
    println!();
    println!("───────────────");
    println!("  How to use:");
    println!("───────────────");
    println!();
    println!("  Create a bare repo:");
    println!("    git init --bare {}\\hello.git", config.git_project_root.display());
    println!();
    if let Some(user) = user_list.first() {
        println!("  Clone:");
        println!("    git clone http://{}:<password>@127.0.0.1:18011/hello.git", user);
        println!();
    }
    println!("  Push:");
    println!("    cd hello");
    println!("    git push");
    println!();
}

pub fn run_quickstart(config_path: &str) -> Option<Config> {
    print_banner();

    let repos_root = step1_repos_root();

    let mut config = Config::default();
    config.git_project_root = repos_root;

    config.save(config_path).expect("Failed to save config");

    println!("✓ Config file created: {}", config_path);
    println!();
    println!("Now let's add a user account for Git access.");
    println!();

    loop {
        step2_add_user(&mut config);
        config.save(config_path).expect("Failed to save config");

        println!("─────────────────────────────────────────────");
        println!("  More Users?");
        println!("─────────────────────────────────────────────");
        println!();
        println!("You can add multiple users. Each user can access all repos.");

        if !ask_yes_no("Add another user?", false) {
            break;
        }
        println!();
    }

    step4_summary(&config, config_path);

    println!("══════════════════════════════════════════════════════");
    println!();

    if ask_yes_no("Start the server now?", true) {
        Some(config)
    } else {
        None
    }
}
```

- [ ] **Step 2: Build check**

Run: `cargo build`
Expected: warning about unused module (no `mod quickstart` yet), but quickstart.rs itself compiles

---

### Task 4: Modify `src/main.rs` — add `quickstart` subcommand

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Add module declaration at top**

```rust
// Add after "mod auth;":
mod quickstart;
```

- [ ] **Step 2: Add `Quickstart` variant to `CliCommand` enum**

```rust
// Add after Server variant:
    Quickstart {
        config_path: String,
    },
```

- [ ] **Step 3: Add `quickstart` match arm in positional parsing**

In `parse_args()`, add before the `_ =>` arm in the `match positional[1].as_str()` block:

```rust
        "quickstart" => CliCommand::Quickstart {
            config_path: default_config.clone(),
        },
```

- [ ] **Step 4: Add `Quickstart` handler in `main()`**

Add after the `DelUser` handler block:

```rust
        CliCommand::Quickstart { config_path } => {
            if let Some(config) = quickstart::run_quickstart(&config_path) {
                if !config.git_project_root.exists() {
                    eprintln!(
                        "Error: git_project_root {:?} does not exist",
                        config.git_project_root
                    );
                    std::process::exit(1);
                }
                if !config.git_project_root.is_dir() {
                    eprintln!(
                        "Error: git_project_root {:?} is not a directory",
                        config.git_project_root
                    );
                    std::process::exit(1);
                }
                let _guard = init_logging(
                    config.logging.file_enabled,
                    &config.logging.log_dir,
                    false,
                );
                run_server(config).await;
            }
            return;
        }
```

- [ ] **Step 5: Update `print_help()` to include `quickstart`**

```rust
// Add after the User management block:
    eprintln!("  githttp quickstart                   Interactive setup wizard");
```

- [ ] **Step 6: Build check**

Run: `cargo build`
Expected: clean compile

---

### Task 5: Modify `.github/workflows/build.yml` — package scripts

**Files:**
- Modify: `.github/workflows/build.yml`

- [ ] **Step 1: Update Windows packaging step**

```yaml
# Change:
          Copy-Item config.example.yaml pkg/
# To:
          Copy-Item quickstart.bat pkg/
```

- [ ] **Step 2: Update Linux packaging step**

```yaml
# Change:
          cp target/x86_64-unknown-linux-musl/release/githttp .
          tar -czf githttp-x86_64-unknown-linux-musl.tar.gz githttp config.example.yaml
# To:
          cp target/x86_64-unknown-linux-musl/release/githttp .
          chmod +x quickstart.sh
          tar -czf githttp-x86_64-unknown-linux-musl.tar.gz githttp quickstart.sh
```

---

### Task 6: Verify — build, lint, test

- [ ] **Step 1: Build release**

Run: `cargo build`
Expected: clean compile

- [ ] **Step 2: Clippy**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: no warnings

- [ ] **Step 3: Run existing tests**

Run: `cargo test`
Expected: all 26 tests pass

- [ ] **Step 4: Quick manual smoke test of wizard**

Run: `echo "" | cargo run -- quickstart`
Expected: wizard starts, banner prints, exits on empty path input

---

### Task 7: Commit

- [ ] **Step 1: Commit all changes**

```bash
git add src/quickstart.rs src/main.rs src/users.rs quickstart.bat quickstart.sh .github/workflows/build.yml docs/superpowers/
git commit -m "feat: add quickstart interactive setup wizard"
```
