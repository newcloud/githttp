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

    let mut config = Config {
        git_project_root: repos_root,
        ..Config::default()
    };

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
