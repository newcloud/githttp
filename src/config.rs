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
    #[serde(skip_serializing_if = "Option::is_none")]
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
        let mut content = String::new();
        content.push_str("# ============================================================\n");
        content.push_str("#  githttp Configuration\n");
        content.push_str("# ============================================================\n\n");
        content.push_str("# --- Repository Settings ---\n");
        content.push_str("# Root directory for Git bare repositories.\n");
        content.push_str("# Example: C:\\Users\\...\\repos   or   /home/.../repos\n");
        let root = self.git_project_root.display().to_string();
        content.push_str(&format!("git_project_root: {}\n\n", yaml_str(&root)));
        content.push_str("# --- Server Settings ---\n");
        content.push_str("# Listen address and port.\n");
        content.push_str("# Example: 0.0.0.0:18011\n");
        content.push_str(&format!("listen_addr: \"{}\"\n\n", self.listen_addr));
        if self.backend == Backend::Cgi {
            if let Some(ref backend_path) = self.git_http_backend {
                content.push_str("# Path to git-http-backend executable.\n");
                let bp = backend_path.display().to_string();
                content.push_str(&format!("git_http_backend: {}\n\n", yaml_str(&bp)));
            }
            content.push_str("# --- Backend ---\n");
            content.push_str("# Git backend mode: \"native\" or \"cgi\"\n");
            content.push_str("#   native - spawns git processes directly\n");
            content.push_str("#   cgi    - proxies through git-http-backend (CGI)\n");
            content.push_str("backend: cgi\n\n");
        }
        if !self.users.is_empty() {
            content.push_str("# --- User Accounts ---\n");
            content.push_str("# Username and hashed password for Git authentication.\n");
            content.push_str("users:\n");
            for (user, hash) in &self.users {
                content.push_str(&format!("  {}: \"{}\"\n", user, hash));
            }
            content.push('\n');
        }
        content.push_str("# --- Logging ---\n");
        content.push_str("logging:\n");
        content.push_str("  # Write access logs to file.\n");
        content.push_str("  # Example: true  or  false\n");
        content.push_str(&format!("  file_enabled: {}\n", self.logging.file_enabled));
        content.push_str("  # Directory for log files.\n");
        content.push_str("  # Example: logs\n");
        let log_dir = self.logging.log_dir.display().to_string();
        content.push_str(&format!("  log_dir: {}\n", yaml_str(&log_dir)));
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
fn yaml_str(s: &str) -> String {
    if s.is_empty() || s.contains('\\') || s.contains('"') || s.contains(':') || s.contains('#') {
        let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
        format!("\"{}\"", escaped)
    } else {
        s.to_string()
    }
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
