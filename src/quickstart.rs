use std::collections::HashMap;
use std::io::{self, Write};
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;
use std::fs;
use colored::Colorize;

use crate::auth::hash_password;
use crate::config::Config;
use crate::config::LoggingConfig;
use crate::users::read_password_secure;

fn read_line() -> String {
    let mut input = String::new();
    io::stdin().read_line(&mut input).expect("Failed to read input");
    input.trim().to_string()
}

fn parse_yes_no(input: &str, default_yes: bool) -> bool {
    let trimmed = input.trim().to_lowercase();
    if trimmed.is_empty() {
        return default_yes;
    }
    trimmed.starts_with('y')
}

#[allow(dead_code)]
fn validate_repos_root(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Err("Path does not exist.".to_string());
    }
    if !path.is_dir() {
        return Err("Path is not a directory.".to_string());
    }
    Ok(())
}

fn validate_username(name: &str, users: &HashMap<String, String>) -> Result<(), String> {
    if name.is_empty() {
        return Err("Username cannot be empty.".to_string());
    }
    if users.contains_key(name) {
        return Err(format!("User '{}' already exists.", name));
    }
    Ok(())
}

fn validate_password_pair(password: &str, confirm: &str) -> Result<(), String> {
    if password.is_empty() {
        return Err("Password cannot be empty.".to_string());
    }
    if password != confirm {
        return Err("Passwords do not match.".to_string());
    }
    Ok(())
}

fn ask_yes_no(prompt: &str, default_yes: bool) -> bool {
    let hint = if default_yes { "Y/n" } else { "y/N" };
    print!("{} ({}) ", prompt, hint);
    io::stdout().flush().unwrap();
    parse_yes_no(&read_line(), default_yes)
}

fn print_banner() {
    println!("{}", "  ◆  githttp — Git HTTP Server  ◆".bold().bright_white());
    println!();
    println!("This wizard will help you set up a Git HTTP server.");
    println!("You'll configure:");
    println!("  • Where your Git repositories are stored");
    println!("  • A user account for Git access (clone/push/pull)");
    println!();
    println!("{}", "Let's get started!".bold());
    println!();
}

fn step1_repos_root(current: Option<PathBuf>, from_config: bool) -> PathBuf {
    loop {
        println!();
        println!("{}", "Step 1: Git Repository Root".bold().underline());
        println!();
        println!("This is the folder where your Git bare repos will live.");
        println!("A \"bare repo\" is a server-side Git repo (no working files),");
        println!("it's the .git folder you get from: git init --bare <name>.git");
        println!();
        if let Some(ref current_path) = current {
            if from_config {
                println!("  {} {}", "Source: config.yaml →".dimmed(), current_path.display().to_string().bright_white());
            } else {
                println!("  Default: {}", current_path.display().to_string().bright_white());
            }
            println!("  (Press Enter to keep, or type a new path)");
            println!();
        }
        print!("Enter path: ");
        io::stdout().flush().unwrap();
        let input = read_line();
        let path = if input.is_empty() {
            match current {
                Some(ref p) => p.clone(),
                None => {
                    println!();
                    println!("{}", "Path cannot be empty.".red());
                    println!();
                    continue;
                }
            }
        } else {
            PathBuf::from(input)
        };

        if !path.exists() {
            if let Err(e) = fs::create_dir_all(&path) {
                println!();
                println!("{}", format!("Cannot create directory: {}", e).red().bold());
                println!();
                continue;
            }
            println!("  {}", format!("Created: {}", path.display()).dimmed());
        }

        if !path.is_dir() {
            println!();
            println!("{}", "Path is not a directory.".red().bold());
            println!();
            continue;
        }

        println!();
        let repos = crate::config::scan_git_repos(&path);
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
        println!();
        return path;
    }
}

fn step_listen_addr(current: &str) -> String {
    println!();
    println!("The address and port the server will listen on.");
    println!();
    println!("Current: {}", current);
    println!("(Press Enter to keep, or type a new address)");
    println!();
    print!("Enter listen address: ");
    io::stdout().flush().unwrap();
    let input = read_line();

    if input.is_empty() {
        current.to_string()
    } else {
        input
    }
}

fn step2_add_user(config: &mut Config) {
    loop {
        println!();
        println!("{}", "Create a User".bold().underline());
        println!();
        println!("This username and password are used with git clone/push.");
        println!();

        print!("Enter username: ");
        io::stdout().flush().unwrap();
        let username = read_line();

        if let Err(err) = validate_username(&username, &config.users) {
            println!();
            println!("{}", err.red().bold());
            println!();
            continue;
        }

        let password = read_password_secure("Enter password: ");

        let confirm = read_password_secure("Confirm password: ");

        if let Err(err) = validate_password_pair(&password, &confirm) {
            println!();
            println!("{}", err.red().bold());
            println!();
            continue;
        }

        let hashed = hash_password(&password).expect("Failed to hash password");
        config.users.insert(username.clone(), hashed);

        println!();
        std::thread::sleep(Duration::from_millis(600));
        println!("{}", format!("✓ Great! User '{}' has been created.", username).green().bold());
        println!("  {}", format!("Usage: http://{}:<password>@127.0.0.1:18011/repo.git", username).dimmed());
        println!();
        return;
    }
}
fn step_logging(current: &LoggingConfig) -> LoggingConfig {
    println!();
    println!("{}", "Logging".bold().underline());
    println!();
    println!("Server can write access logs to a file.");
    println!("This helps you track who accessed your repos.");
    println!();
    let resolved = std::env::current_dir()
        .map(|d| d.join(&current.log_dir))
        .unwrap_or_else(|_| current.log_dir.clone());
    println!("  Currently: {}", if current.file_enabled { "On" } else { "Off" });
    println!("    Log file: {}", resolved.join("githttp.log").display());
    println!();
    let file_enabled = ask_yes_no("Enable file logging?", current.file_enabled);
    let log_dir = if file_enabled {
        let sep = if cfg!(windows) { "\\" } else { "/" };
        let rel = format!(".{}logs{}", sep, sep);
        println!();
        println!("Log file location [default {}]:", rel);
        println!("  eg: relative {}  or  absolute {}", rel, resolved.display());
        println!("  (Press Enter for default, or type a custom path)");
        print!("Enter path: ");
        io::stdout().flush().unwrap();
        let input = read_line();
        if input.is_empty() {
            current.log_dir.clone()
        } else {
            PathBuf::from(input)
        }
    } else {
        current.log_dir.clone()
    };
    println!();
    LoggingConfig { file_enabled, log_dir }
}

fn step4_summary(config: &Config, config_path: &str) {
    println!();
    println!("{}", "Setup Complete!".bold().green());
    println!();
    println!("  Config file     {}", config_path.yellow());
    println!("  Repos directory {}", config.git_project_root.display().to_string().bright_white());
    println!("  Listen address  {}", config.listen_addr.bright_white());

    let user_list: Vec<String> = config.users.keys().cloned().collect();
    if user_list.len() == 1 {
        println!("  Users           1: {}", user_list[0].cyan());
    } else {
        println!("  Users           {}: {}", user_list.len(), user_list.join(", ").cyan());
    }
    println!();
    println!("{}", "How to use:".bold().underline());
    println!();
    println!("  Create a bare repo:");
    println!("    {}", format!("git init --bare {}\\hello.git", config.git_project_root.display()).dimmed());
    println!();
    if let Some(user) = user_list.first() {
        println!("  Clone:");
        println!("    {}", format!("git clone http://{}:<password>@127.0.0.1:18011/hello.git", user).dimmed());
        println!();
    }
    println!("  Push:");
    println!("    {}", "cd hello".dimmed());
    println!("    {}", "git push".dimmed());
    println!();
}

pub fn run_quickstart(config_path: &str) -> Option<Config> {
    print_banner();

    let existing_config = Config::from_file(config_path);
    let has_existing = existing_config.is_some();

    let mut config = existing_config.unwrap_or_default();

    if has_existing {
        println!("{} {}", "Config file found:".bold(), config_path.yellow());
        println!("  {}", "It will be updated. Press Enter to keep current values.".dimmed());
        println!();
    }

    let repos_root = step1_repos_root(Some(config.git_project_root.clone()), has_existing);
    config.git_project_root = repos_root;
    config.save(config_path).expect("Failed to save config");

    let listen_addr = step_listen_addr(&config.listen_addr);
    config.listen_addr = listen_addr;
    config.save(config_path).expect("Failed to save config");

    let logging = step_logging(&config.logging);
    config.logging = logging;
    config.save(config_path).expect("Failed to save config");

    println!();
    println!("Now let's add a user account for Git access.");
    println!();

    let should_add_users = if !config.users.is_empty() {
        let existing: Vec<String> = config.users.keys().cloned().collect();
        println!("{} {}", "Existing users:".bold(), existing.join(", ").cyan());
        ask_yes_no("Add another user?", false)
    } else {
        true
    };

    if should_add_users {
        loop {
            step2_add_user(&mut config);
            config.save(config_path).expect("Failed to save config");

            if !ask_yes_no("Add another user?", false) {
                break;
            }
            println!();
        }
    }

    step4_summary(&config, config_path);

    println!();

    if ask_yes_no("All set! Launch the server now?", true) {
        return Some(config);
    }

    println!();
    println!(
        "{}  {} {}",
        "No problem! Start it anytime:".dimmed(),
        "githttp -c".yellow(),
        config_path.yellow()
    );
    println!();
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_parse_yes_no_empty_defaults() {
        assert!(parse_yes_no("", true));
        assert!(!parse_yes_no("", false));
    }

    #[test]
    fn test_parse_yes_no_explicit() {
        assert!(parse_yes_no("y", true));
        assert!(parse_yes_no("Y", false));
        assert!(parse_yes_no("yes", true));
        assert!(parse_yes_no("YES", false));
    }

    #[test]
    fn test_parse_yes_no_no() {
        assert!(!parse_yes_no("n", true));
        assert!(!parse_yes_no("N", false));
        assert!(!parse_yes_no("no", true));
        assert!(!parse_yes_no("No", false));
        assert!(!parse_yes_no("abc", true));
    }

    #[test]
    fn test_validate_repos_root_nonexistent() {
        let path = Path::new("/tmp/githttp_nonexistent_test_dir_xyz");
        let err = validate_repos_root(path).unwrap_err();
        assert!(err.contains("does not exist"));
    }

    #[test]
    fn test_validate_repos_root_file_not_dir() {
        let temp = std::env::temp_dir();
        let file = temp.join("githttp_test_file.txt");
        fs::write(&file, "x").unwrap();
        let err = validate_repos_root(&file).unwrap_err();
        assert!(err.contains("not a directory"));
        fs::remove_file(&file).ok();
    }

    #[test]
    fn test_validate_repos_root_valid() {
        let temp = std::env::temp_dir();
        assert!(validate_repos_root(&temp).is_ok());
    }

    #[test]
    fn test_validate_username_empty() {
        let users = HashMap::new();
        let err = validate_username("", &users).unwrap_err();
        assert!(err.contains("cannot be empty"));
    }

    #[test]
    fn test_validate_username_duplicate() {
        let mut users = HashMap::new();
        users.insert("bob".to_string(), "hash".to_string());
        let err = validate_username("bob", &users).unwrap_err();
        assert!(err.contains("already exists"));
    }

    #[test]
    fn test_validate_username_valid() {
        let users = HashMap::new();
        assert!(validate_username("alice", &users).is_ok());
    }

    #[test]
    fn test_validate_password_pair_empty() {
        let err = validate_password_pair("", "").unwrap_err();
        assert!(err.contains("cannot be empty"));
    }

    #[test]
    fn test_validate_password_pair_mismatch() {
        let err = validate_password_pair("abc", "xyz").unwrap_err();
        assert!(err.contains("do not match"));
    }

    #[test]
    fn test_validate_password_pair_valid() {
        assert!(validate_password_pair("pass123", "pass123").is_ok());
    }

}
