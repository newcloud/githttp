use serde::{Deserialize, Serialize};
use std::{fs, path::{Path, PathBuf}};

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
pub enum Backend {
    #[serde(rename = "cgi")]
    Cgi,
    #[serde(rename = "native")]
    Native,
}

#[derive(Deserialize, Serialize, Clone)]
pub struct LoggingConfig {
    #[serde(default)]
    pub file_enabled: bool,
    #[serde(default = "default_log_dir")]
    pub log_dir: PathBuf,
}

fn default_log_dir() -> PathBuf {
    PathBuf::from("logs")
}

impl Default for LoggingConfig {
    fn default() -> Self {
        LoggingConfig {
            file_enabled: false,
            log_dir: default_log_dir(),
        }
    }
}

#[derive(Deserialize, Serialize, Clone)]
pub struct Config {
    pub git_project_root: PathBuf,
    pub git_http_backend: Option<PathBuf>,
    pub listen_addr: String,
    pub users: std::collections::HashMap<String, String>,
    #[serde(default = "default_backend")]
    pub backend: Backend,
    #[serde(default)]
    pub logging: LoggingConfig,
}

/// 自动检测 git-http-backend 路径
pub fn detect_git_http_backend() -> PathBuf {
    let windows_paths = [
        r"C:\Program Files\Git\mingw64\libexec\git-core\git-http-backend.exe",
        r"C:\Program Files (x86)\Git\mingw64\libexec\git-core\git-http-backend.exe",
    ];

    let linux_paths = [
        "/usr/libexec/git-core/git-http-backend",
        "/usr/lib/git-core/git-http-backend",
        "/usr/local/libexec/git-core/git-http-backend",
    ];

    let paths: &[&str] = if cfg!(windows) {
        &windows_paths
    } else {
        &linux_paths
    };

    for path in paths {
        let p = PathBuf::from(path);
        if p.exists() {
            return p;
        }
    }

    PathBuf::from(paths[0])
}

impl Config {
    pub fn resolve_git_http_backend(&self) -> PathBuf {
        if let Some(ref path) = self.git_http_backend {
            return path.clone();
        }
        if self.backend == Backend::Cgi {
            detect_git_http_backend()
        } else {
            PathBuf::new()
        }
    }

    pub fn from_file(path: &str) -> Option<Self> {
        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return None,
        };
        if content.is_empty() {
            return None;
        }
        serde_yaml::from_str(&content).ok()
    }

    pub fn save(&self, path: &str) -> Result<(), Box<dyn std::error::Error>> {
        let content = serde_yaml::to_string(self)?;
        fs::write(path, content)?;
        Ok(())
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            git_project_root: if cfg!(windows) {
                PathBuf::from(r"C:\git")
            } else {
                PathBuf::from("/mnt/git")
            },
            git_http_backend: None,
            listen_addr: "0.0.0.0:18011".to_string(),
            users: std::collections::HashMap::new(),
            backend: default_backend(),
            logging: LoggingConfig::default(),
        }
    }
}

fn default_backend() -> Backend {
    Backend::Native
}

pub fn detect_git_executable() -> PathBuf {
    if cfg!(windows) {
        let paths = [
            r"D:\Program Files\Git\cmd\git.exe",
            r"D:\Program Files\Git\bin\git.exe",
            r"C:\Program Files\Git\cmd\git.exe",
            r"C:\Program Files\Git\bin\git.exe",
        ];
        for p in &paths {
            if std::path::Path::new(p).exists() {
                return PathBuf::from(p);
            }
        }
    }
    PathBuf::from("git")
}

pub fn verify_repo_path(root: &Path, repo_name: &str) -> Result<PathBuf, std::io::Error> {
    let full_path = root.join(repo_name);

    let root_canon = root.canonicalize()?;
    let full_canon = full_path.canonicalize().map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("Repository not found: {}", e),
        )
    })?;

    if !full_canon.starts_with(&root_canon) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Path traversal attempt",
        ));
    }

    Ok(full_path)
}
