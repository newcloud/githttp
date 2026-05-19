use std::io::{IsTerminal, Read, Write};

use crate::auth::hash_password;
use crate::config::Config;

pub fn load_config_or_exit(config_path: &str) -> Config {
    Config::from_file(config_path).unwrap_or_else(|| {
        eprintln!("Config file not found at '{}'", config_path);
        eprintln!("Copy config.example.yaml to config.yaml and try again");
        std::process::exit(1);
    })
}

fn read_password_secure(prompt: &str) -> String {
    print!("{}", prompt);
    std::io::stdout().flush().unwrap();

    let stdin_is_tty = std::io::stdin().is_terminal();

    if stdin_is_tty && let Ok(p) = rpassword::read_password() {
        return p;
    }

    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .expect("Failed to read password");
    line.trim_end_matches(&['\r', '\n'][..]).to_string()
}

fn read_password_pair() -> (String, String) {
    let stdin_is_tty = std::io::stdin().is_terminal();

    if stdin_is_tty {
        let p = read_password_secure("Enter password: ");
        let c = read_password_secure("Confirm password: ");
        (p, c)
    } else {
        let mut lines = String::new();
        std::io::stdin()
            .read_to_string(&mut lines)
            .expect("Failed to read input");
        let mut iter = lines.lines();
        let p = iter.next().unwrap_or("").trim().to_string();
        let c = iter.next().unwrap_or("").trim().to_string();
        (p, c)
    }
}

fn read_new_password_pair() -> (String, String) {
    let stdin_is_tty = std::io::stdin().is_terminal();

    if stdin_is_tty {
        let p = read_password_secure("Enter new password: ");
        let c = read_password_secure("Confirm new password: ");
        (p, c)
    } else {
        let mut lines = String::new();
        std::io::stdin()
            .read_to_string(&mut lines)
            .expect("Failed to read input");
        let mut iter = lines.lines();
        let p = iter.next().unwrap_or("").trim().to_string();
        let c = iter.next().unwrap_or("").trim().to_string();
        (p, c)
    }
}

pub fn cmd_add_user(username: &str, config_path: &str) {
    let (password, confirm) = read_password_pair();

    if password != confirm {
        eprintln!("Passwords do not match");
        std::process::exit(1);
    }

    if password.is_empty() {
        eprintln!("Password cannot be empty");
        std::process::exit(1);
    }

    let mut config = load_config_or_exit(config_path);

    if config.users.contains_key(username) {
        eprintln!(
            "User '{}' already exists. Use 'setpassword' to change password",
            username
        );
        std::process::exit(1);
    }

    let hashed = hash_password(&password).expect("Failed to hash password");
    config.users.insert(username.to_string(), hashed);
    config.save(config_path).expect("Failed to save config");

    println!("User '{}' added successfully", username);
}

pub fn cmd_set_password(username: &str, config_path: &str) {
    let mut config = load_config_or_exit(config_path);

    if !config.users.contains_key(username) {
        eprintln!("User '{}' not found", username);
        std::process::exit(1);
    }

    let (password, confirm) = read_new_password_pair();

    if password != confirm {
        eprintln!("Passwords do not match");
        std::process::exit(1);
    }

    if password.is_empty() {
        eprintln!("Password cannot be empty");
        std::process::exit(1);
    }

    let hashed = hash_password(&password).expect("Failed to hash password");
    config.users.insert(username.to_string(), hashed);
    config.save(config_path).expect("Failed to save config");

    println!("Password for '{}' updated successfully", username);
}

pub fn cmd_del_user(username: &str, config_path: &str) {
    let mut config = load_config_or_exit(config_path);

    if !config.users.contains_key(username) {
        eprintln!("User '{}' not found", username);
        std::process::exit(1);
    }

    config.users.remove(username);
    config.save(config_path).expect("Failed to save config");

    println!("User '{}' deleted successfully", username);
}
