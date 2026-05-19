use serde::{Deserialize, Serialize};
use std::{fs, path::{Path, PathBuf}};

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
pub enum Backend {
    #[serde(rename = "cgi")]
    Cgi,
    #[serde(rename = "native")]
    Native,
}

#[derive(Deserialize, Serialize, Clone, PartialEq)]
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
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub git_http_backend: Option<PathBuf>,
    pub listen_addr: String,
    #[serde(skip_serializing_if = "std::collections::HashMap::is_empty", default)]
    pub users: std::collections::HashMap<String, String>,
    #[serde(skip_serializing_if = "is_default_backend", default = "default_backend")]
    pub backend: Backend,
    #[serde(skip_serializing_if = "is_default_logging", default)]
    pub logging: LoggingConfig,
}

fn is_default_backend(b: &Backend) -> bool {
    *b == default_backend()
}

fn is_default_logging(l: &LoggingConfig) -> bool {
    *l == LoggingConfig::default()
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
        toml::from_str(&content).ok()
    }

    pub fn save(&self, path: &str) -> Result<(), Box<dyn std::error::Error>> {
        let content = toml::to_string_pretty(self)?;
        fs::write(path, content)?;
        Ok(())
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            git_project_root: dirs::home_dir()
                .unwrap_or_else(|| {
                    if cfg!(windows) {
                        PathBuf::from(r"C:\Users\Default")
                    } else {
                        PathBuf::from("/home/user")
                    }
                })
                .join("repos"),
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

pub fn scan_git_repos(path: &Path) -> Vec<String> {
    std::fs::read_dir(path)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter(|e| {
                    e.file_type().map(|t| t.is_dir()).unwrap_or(false)
                        && e.file_name().to_string_lossy().ends_with(".git")
                })
                .map(|e| e.file_name().to_string_lossy().to_string())
                .collect()
        })
        .unwrap_or_default()
}

pub fn resolve_display_host(listen_addr: &str) -> String {
    let host = listen_addr.rsplit(':').nth(1).unwrap_or("0.0.0.0");

    let lower = host.to_lowercase();
    if lower == "127.0.0.1" || lower == "localhost" {
        return "127.0.0.1".to_string();
    }

    // 绑定到具体地址时直接用，不做扫描
    if host != "0.0.0.0" && host != "[::]" {
        return host.to_string();
    }

    // 优先物理网卡 IP，过滤虚拟适配器
    if let Ok(netifs) = local_ip_address::list_afinet_netifas() {
        // 优先级: 私有 IPv4 > 任意非回环非 APIPA IPv4 > 回退
        let physical = netifs.iter().find(|(name, ip)| {
            ip.is_ipv4()
                && !ip.is_loopback()
                && !is_apipa(ip)
                && is_private_ipv4(ip)
                && {
                    let lower = name.to_lowercase();
                    !lower.contains("vmware")
                        && !lower.contains("virtualbox")
                        && !lower.contains("docker")
                        && !lower.contains("vethernet")
                        && !lower.contains("hyper-v")
                        && !lower.contains("wsl")
                        && !lower.contains("tunnel")
                        && !lower.contains("teredo")
                        && !lower.contains("isatap")
                        && !lower.contains("bluetooth")
                        && !lower.contains("vpn")
                }
        });

        if let Some((_name, ip)) = physical {
            return ip.to_string();
        }
    }

    // 回退：先用 list_afinet_netifas 取任意非回环 IPv4，再回退到 local_ip
    if let Ok(netifs) = local_ip_address::list_afinet_netifas()
        && let Some((_name, ip)) = netifs.iter().find(|(_name, ip)| {
            ip.is_ipv4() && !ip.is_loopback() && !is_apipa(ip)
        })
    {
        return ip.to_string();
    }

    local_ip_address::local_ip()
        .map(|ip| ip.to_string())
        .unwrap_or_else(|_| host.to_string())
}

fn is_apipa(ip: &std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            let octets = v4.octets();
            octets[0] == 169 && octets[1] == 254
        }
        std::net::IpAddr::V6(_) => false,
    }
}

fn is_private_ipv4(ip: &std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            let octets = v4.octets();
            // 10.0.0.0/8
            (octets[0] == 10)
            // 172.16.0.0/12
            || (octets[0] == 172 && octets[1] >= 16 && octets[1] <= 31)
            // 192.168.0.0/16
            || (octets[0] == 192 && octets[1] == 168)
        }
        std::net::IpAddr::V6(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_scan_git_repos_empty() {
        let temp = std::env::temp_dir().join("githttp_scan_empty");
        fs::create_dir_all(&temp).unwrap();
        let repos = scan_git_repos(&temp);
        assert!(repos.is_empty());
        fs::remove_dir(&temp).ok();
    }

    #[test]
    fn test_scan_git_repos_finds_git_dirs() {
        let temp = std::env::temp_dir().join("githttp_scan_test");
        fs::create_dir_all(temp.join("hello.git")).unwrap();
        fs::create_dir_all(temp.join("world.git")).unwrap();
        fs::create_dir_all(temp.join("not-repo")).unwrap();
        let repos = scan_git_repos(&temp);
        assert_eq!(repos.len(), 2);
        assert!(repos.contains(&"hello.git".to_string()));
        assert!(repos.contains(&"world.git".to_string()));
        assert!(!repos.contains(&"not-repo".to_string()));
        fs::remove_dir_all(&temp).ok();
    }
}
