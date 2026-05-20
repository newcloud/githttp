mod auth;
mod cgi;
mod config;
mod git_cgi;
mod git_native;
mod quickstart;
mod users;

use axum::{Router, middleware::from_fn, routing::any};
use axum::body::Body;
use axum::http::Request;
use axum::middleware::Next;
use axum::response::Response;
use std::{env, path::PathBuf, sync::Arc};
use tracing::{info, warn};
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

use config::Config;
use git_cgi::AppState;

/// Newtype for authenticated user, stored in request extensions.
#[derive(Clone)]
pub struct AuthenticatedUser(pub String);

async fn auth_middleware(
    state: Arc<AppState>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let method = req.method().to_string();
    let uri = req.uri().to_string();

    match auth::verify_auth(&req, &state.config.users) {
        Some(user) => {
            info!("Auth success for user: {}", user);
            let (mut parts, body) = req.into_parts();
            parts.extensions.insert(AuthenticatedUser(user));
            let req = Request::from_parts(parts, body);
            next.run(req).await
        }
        None => {
            warn!("Auth failed for request: {} {}", method, uri);
            auth::unauthorized_response()
        }
    }
}

enum CliCommand {
    Server {
        config_path: String,
        quiet: bool,
    },
    Quickstart {
        config_path: String,
    },
    AddUser {
        username: String,
        config_path: String,
    },
    SetPassword {
        username: String,
        config_path: String,
    },
    DelUser {
        username: String,
        config_path: String,
    },
}

fn parse_args() -> CliCommand {
    let raw_args: Vec<String> = env::args().collect();
    let default_config = "config.toml".to_string();
    let mut config_path = default_config.clone();
    let mut quiet = false;
    let mut positional: Vec<String> = vec![raw_args[0].clone()];

    let mut i = 1;
    while i < raw_args.len() {
        match raw_args[i].as_str() {
            "-c" | "--config" => {
                i += 1;
                if i < raw_args.len() {
                    config_path = raw_args[i].clone();
                } else {
                    eprintln!("Error: --config requires a path argument");
                    std::process::exit(1);
                }
            }
            "-q" | "--quiet" => {
                quiet = true;
            }
            arg if arg.starts_with("--config=") => {
                config_path = arg[9..].to_string();
            }
            arg if arg.starts_with("-c=") => {
                config_path = arg[3..].to_string();
            }
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            }
            other => {
                if other.starts_with('-') {
                    eprintln!("Error: Unknown flag '{}'", other);
                    print_help();
                    std::process::exit(1);
                }
                positional.push(other.to_string());
            }
        }
        i += 1;
    }

    if positional.len() < 2 {
        return CliCommand::Server { config_path, quiet };
    }

    match positional[1].as_str() {
        "adduser" => {
            if positional.len() < 3 {
                eprintln!("Usage: githttp adduser <username> [config.toml]");
                std::process::exit(1);
            }
            if positional.len() > 3 {
                config_path = positional[3].clone();
            }
            CliCommand::AddUser {
                username: positional[2].clone(),
                config_path,
            }
        }
        "setpassword" => {
            if positional.len() < 3 {
                eprintln!("Usage: githttp setpassword <username> [config.toml]");
                std::process::exit(1);
            }
            if positional.len() > 3 {
                config_path = positional[3].clone();
            }
            CliCommand::SetPassword {
                username: positional[2].clone(),
                config_path,
            }
        }
        "deluser" => {
            if positional.len() < 3 {
                eprintln!("Usage: githttp deluser <username> [config.toml]");
                std::process::exit(1);
            }
            if positional.len() > 3 {
                config_path = positional[3].clone();
            }
            CliCommand::DelUser {
                username: positional[2].clone(),
                config_path,
            }
        }
        "quickstart" => CliCommand::Quickstart {
            config_path: default_config.clone(),
        },
        _ => CliCommand::Server {
            config_path: positional[1].clone(),
            quiet,
        },
    }
}

fn print_help() {
    eprintln!("githttp — Git HTTP server");
    eprintln!();
    eprintln!("Usage:");
    eprintln!("  githttp [OPTIONS]                    Start server (reads config.toml)");
    eprintln!("  githttp [OPTIONS] <config.toml>      Start server with config file");
    eprintln!();
    eprintln!("User management:");
    eprintln!("  githttp adduser <username> [config.toml]");
    eprintln!("  githttp setpassword <username> [config.toml]");
    eprintln!("  githttp deluser <username> [config.toml]");
    eprintln!("  githttp quickstart                   Interactive setup wizard");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  -c, --config <path>    Config file path");
    eprintln!("  --config=<path>        Equivalent");
    eprintln!("  -c=<path>              Equivalent");
    eprintln!("  -q, --quiet            Suppress terminal log output");
    eprintln!("  -h, --help             Show this help");
}

#[tokio::main]
async fn main() {
    let cmd = parse_args();

    match cmd {
        CliCommand::AddUser {
            username,
            config_path,
        } => {
            users::cmd_add_user(&username, &config_path);
            return;
        }
        CliCommand::SetPassword {
            username,
            config_path,
        } => {
            users::cmd_set_password(&username, &config_path);
            return;
        }
        CliCommand::DelUser {
            username,
            config_path,
        } => {
            users::cmd_del_user(&username, &config_path);
            return;
        }
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
                let _guard =
                    init_logging(config.logging.file_enabled, &config.logging.log_dir, false);
                run_server(config).await;
            }
            return;
        }
        CliCommand::Server { config_path, quiet } => {
            let config = Config::from_file(&config_path).unwrap_or_else(|| {
                eprintln!("No config file found at '{}'", config_path);
                eprintln!("Run 'githttp quickstart' for interactive setup.");
                std::process::exit(1);
            });
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
            if config.backend == config::Backend::Native
                && config.resolve_git_executable().is_none()
            {
                eprintln!("Error: git executable not found. Set git_path in config.toml.");
                std::process::exit(1);
            }
            let _guard = init_logging(config.logging.file_enabled, &config.logging.log_dir, quiet);
            info!("Git project root: {:?}", config.git_project_root);

            info!("User count: {}", config.users.len());
            if config.backend == config::Backend::Cgi {
                info!("Backend: CGI");
                info!("Git HTTP backend: {:?}", config.resolve_git_http_backend());
            }

            info!(
                "Log file: {}",
                if config.logging.file_enabled {
                    "enabled"
                } else {
                    "disabled"
                }
            );
            run_server(config).await;
        }
    }
}

fn init_logging(
    file_enabled: bool,
    log_dir: &PathBuf,
    quiet: bool,
) -> Option<tracing_appender::non_blocking::WorkerGuard> {
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| "githttp=info".into());

    let mut layers: Vec<Box<dyn tracing_subscriber::layer::Layer<_> + Send + Sync>> = Vec::new();

    if !quiet {
        layers.push(Box::new(tracing_subscriber::fmt::layer()));
    }

    let guard = if file_enabled {
        std::fs::create_dir_all(log_dir).expect("Failed to create log directory");
        let file_appender = RollingFileAppender::new(Rotation::DAILY, log_dir, "githttp.log");
        let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
        let layer = tracing_subscriber::fmt::layer()
            .with_writer(non_blocking)
            .with_ansi(false);
        layers.push(Box::new(layer));
        Some(guard)
    } else {
        None
    };

    tracing_subscriber::registry()
        .with(env_filter)
        .with(layers)
        .init();

    guard
}

async fn run_server(config: Config) {
    let listen_addr = config.listen_addr.clone();

    let repos = config::scan_git_repos(&config.git_project_root);
    let host = config::resolve_display_host(&listen_addr);
    let port = listen_addr.rsplit(':').next().unwrap_or("18011");
    let repo_urls: Vec<String> = repos
        .iter()
        .map(|r| format!("http://<user>:<password>@{}:{}/{}", host, port, r))
        .collect();

    let project_root = config.git_project_root.clone();
    let state = Arc::new(AppState { config });
    let auth_state = state.clone();

    let app = Router::new()
        .route(
            "/{*path}",
            match state.config.backend {
                config::Backend::Native => any(git_native::git_handler_native),
                config::Backend::Cgi => any(git_cgi::git_handler_cgi),
            },
        )
        .route_layer(from_fn(move |req, next| {
            let state = auth_state.clone();
            auth_middleware(state, req, next)
        }))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&listen_addr)
        .await
        .expect("Failed to bind address");
    info!("Git server listening on http://{}", listen_addr);

    if !repo_urls.is_empty() {
        info!("Available repositories:");
        for url in &repo_urls {
            info!("  {}", url);
        }
    } else {
        info!("No repositories found in {}", project_root.display());
        info!("To get started, create a bare Git repository:");
        info!("  cd {}", project_root.display());
        info!("  git init --bare demo.git");
        info!("  Then clone it: git clone http://<user>:<password>@{}:{}/demo.git", host, port);
    }

    axum::serve(listener, app.into_make_service_with_connect_info::<std::net::SocketAddr>())
        .with_graceful_shutdown(async {
            tokio::signal::ctrl_c().await.expect("Ctrl+C handler");
        })
        .await
        .unwrap();
}
