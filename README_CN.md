# Git HTTP Server

> **警告：请勿用于生产环境！仅适用于本地安全环境下使用。**

基于 Axum 的轻量级 Git HTTP 服务器，支持两种后端模式，全链路流式传输。

## 后端模式

| 模式 | 配置值 | 说明 |
|------|--------|------|
| **Native**（默认）| `backend: "native"` | 直接调用 `git upload-pack` / `git receive-pack`，无 CGI 依赖 |
| **CGI** | `backend: "cgi"` | 通过 `git-http-backend` CGI 程序代理请求 |


## 快速开始

### 编译

```bash
cargo build --release
```

### 配置

```bash
cp config.example.yaml config.yaml
```

编辑 `config.yaml`：

```yaml
git_project_root: "/path/to/git/repos"
listen_addr: "0.0.0.0:18011"
users: {}
backend: "native"

logging:
  file_enabled: false
  log_dir: "logs"
```


### 用户管理

```bash
# 添加用户
githttp adduser admin

# 修改密码
githttp setpassword admin

# 删除用户
githttp deluser admin

# 指定配置文件
githttp adduser admin config.yaml
```


### 启动

```bash
# 默认读取 config.yaml
githttp

# 指定配置文件
githttp -c config.yaml

# 安静模式（不输出终端日志）
githttp -q -c config.yaml
```

## CLI 参数

| 参数 | 说明 |
|------|------|
| `-c`, `--config <path>` | 指定配置文件路径（默认 config.yaml） |
| `-q`, `--quiet` | 安静模式，不输出终端日志 |
| `adduser <username>` | 添加用户 |
| `setpassword <username>` | 修改密码 |
| `deluser <username>` | 删除用户 |

## 使用示例

```bash
# 创建裸仓库
cd /path/to/git/repos
git init --bare my-project.git

# 启动服务器
githttp -c config.yaml

# clone
git clone http://user:pass@localhost:18011/my-project.git

# push
cd my-project
echo "# README" > README.md
git add . && git commit -m "init"
git push origin master

git clone http://user:pass@localhost:18011/my-project.git
```

## 配置说明

| 字段 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `git_project_root` | path | `C:\git\repos` (Win) `/mnt/git` (Linux) | 裸仓库根目录 |
| `git_http_backend` | path? | 无 | git-http-backend 路径，仅 cgi 模式需要，不填自动检测 |
| `listen_addr` | string | `0.0.0.0:18011` | 监听地址和端口 |
| `users` | map | `{}` | 用户名 → SHA-256 哈希密码（通过 `githttp adduser` 生成） |
| `backend` | string | `native` | `"native"` 或 `"cgi"` |
| `logging.file_enabled` | bool | `false` | 是否写入日志文件 |
| `logging.log_dir` | path | `"logs"` | 日志文件目录 |

## 日志

- **终端输出**（带颜色）：默认开启，`-q`/`--quiet` 关闭
- **文件输出**（无 ANSI）：默认关闭，`logging.file_enabled: true` 开启
- 文件按日滚动：`logs/githttp.log.YYYY-MM-DD`
- 日志级别通过 `RUST_LOG` 环境变量控制（默认 `githttp=info`）

## 测试

```bash
# 单元测试（26 个）
cargo test --bin githttp

# 集成测试
cargo test --test workflow
```

## 架构

```
Client ←→ Axum HTTP ←→ 后端处理器
                         ├─ Native: git upload-pack / receive-pack（直接调用）
                         └─ CGI:    git-http-backend（CGI 代理）

入向流：Request Body → DataStream → tokio::spawn → 子进程 stdin
出向流：子进程 stdout → ReaderStream → Body::from_stream → Response
```

全链路流式传输：请求体数据块到达即写入子进程，响应体实时读取返回，内存常驻 ~KB，处理数 GB 的 push/clone 不 OOM。

## 模块结构

| 模块 | 文件 | 职责 |
|------|------|------|
| `config` | `src/config.rs` | 配置解析、后端检测、`detect_git_executable`、`verify_repo_path` |
| `auth` | `src/auth.rs` | Basic Auth 验证、SHA-256 密码哈希 |
| `git_cgi` | `src/git_cgi.rs` | CGI 后端：代理 git-http-backend |
| `git_native` | `src/git_native.rs` | Native 后端：直接调用 git 命令 |
| `cgi` | `src/cgi.rs` | 流式 CGI 响应头部解析与构造 |
| `users` | `src/users.rs` | CLI 用户管理命令 |

## 技术栈

- `axum` — HTTP 框架
- `tokio` — 异步运行时 + 子进程管理
- `tokio-util` — `ReaderStream` 流式转换
- `sha2` — 密码哈希 (SHA-256)
- `tracing` + `tracing-subscriber` + `tracing-appender` — 日志
- `serde` + `serde_yaml` — 配置序列化

## 开发

本项目使用 [OpenCode](https://opencode.ai) 辅助开发，模型为 DeepSeek V4、Qwen3.6 Plus。
