# CLAUDE.md — grel

## 项目概述

**grel** 是一个仿 pacman 风格的二进制包管理器，用于从 Git Forge（GitHub、GitLab、Gitea、Codeberg）下载、安装和管理预编译二进制文件。

- **语言:** Rust（edition 2024，MSRV 1.85）
- **架构:** Cargo workspace，6 个专用 crate

---

## 代码库结构

```
grel/
├── src/main.rs                  # CLI 主入口 & 命令路由
├── crates/
│   ├── grel-cli/                # CLI 解析 & 用户交互（clap derive）
│   │   ├── src/commands.rs      # Pacman 风格命令定义
│   │   └── src/prompt.rs        # 终端交互 prompt
│   ├── grel-core/               # 核心解析逻辑
│   │   ├── src/asset.rs         # Asset 文件名 tokenization
│   │   ├── src/platform.rs      # OS/Arch 枚举及别名映射
│   │   ├── src/resolver.rs      # 过滤/排序 pipeline
│   │   └── src/package_ref.rs   # PackageRef 解析（owner/repo 格式）
│   ├── grel-config/             # 配置管理（figment: TOML + env vars）
│   │   ├── src/config.rs        # TOML 配置结构体
│   │   └── src/paths.rs         # 路径展开（~ 支持）
│   ├── grel-cache/              # SQLite 状态管理（sqlx async）
│   │   ├── src/database.rs      # DB 操作
│   │   └── src/models.rs        # 数据模型
│   ├── grel-network/            # HTTP 客户端 & 下载
│   │   ├── src/download.rs      # 并行下载（JoinSet）
│   │   ├── src/archive.rs       # 归档解压（tar/zip/zstd/xz/bz2）
│   │   ├── src/client.rs        # reqwest client（rustls-tls，SOCKS 代理）
│   │   └── src/dns_cache.rs     # DNS 缓存（缓存至 SQLite）
│   └── grel-providers/          # Forge API 适配器
│       ├── src/github.rs        # GitHub API 实现
│       ├── src/trait_def.rs     # ReleaseProvider 异步 trait
│       └── src/registry.rs      # Provider 注册表
├── tests/integration.rs         # 集成测试占位
├── docs/
│   ├── TECHNICAL_DESIGN.md      # 架构设计文档
│   └── COMMANDS_DESIGN.md       # CLI 命令规范
├── config.example.toml          # 配置文件示例
├── .rustfmt.toml                # 格式化规则（max_width = 100）
└── .github/workflows/rust.yml   # CI/CD（Ubuntu + Windows）
```

---

## 构建与测试

```bash
# 调试构建
cargo build --workspace

# 发布构建
cargo build --release
# 产物: target/release/grel

# 运行所有测试
cargo test --workspace

# 代码检查（CI 强制）
cargo fmt --check
cargo clippy -- -D warnings
```

---

## 常用命令

```bash
grel -S owner/repo          # 安装包（最新版本）
grel -Syu                   # 刷新并升级所有包
grel -Ss pattern            # 搜索包
grel -Ql                    # 列出已安装包
grel -Qk                    # 校验 SHA-256 checksums
grel -R owner/repo          # 卸载包
grel -D --migrate old new   # 迁移包（项目重命名）
```

---

## 核心架构

### 命令路由（src/main.rs）

`main.rs` 是主要编排层：初始化 tracing/config/database/HTTP client，然后根据 CLI operation 分发到对应命令函数：

| Operation    | 命令函数                                            |
|--------------|-----------------------------------------------------|
| `-S`（Sync） | `cmd_sync`, `cmd_search`, `cmd_upgrade`             |
| `-Q`（Query）| `cmd_list`, `cmd_info_local`, `cmd_check_checksums` |
| `-R`（Remove）| `cmd_remove`                                       |
| `-D`（Database）| `cmd_db_clean`, `cmd_migrate`, `cmd_as_explicit` |
| `-F`（Files）| `cmd_file_search`, `cmd_reindex`                    |

### Asset 解析 Pipeline（grel-core/resolver.rs）

**严格确定性**：无模糊评分，仅用显式规则。

1. **过滤（严格，按序）：**
   - OS 匹配（精确或 Unknown 回退）
   - Arch 匹配（含 32 位回退逻辑）
   - 排除关键字过滤（`setup`、`installer`、`bundle`、`nupkg`）
   - 忽略格式（`.deb`、`.rpm`、`.msi`、`.dmg`、`.AppImage` 等）

2. **排序（决定顺序）：**
   - Arch 优先级索引
   - 格式优先级（`.tar.gz > .tar.xz > .zip > .exe`）
   - 文件名字典序
   - 大小降序

3. **选择策略（tie-breaker）：**
   - `first`（默认）：取排序后第一个
   - `largest`：按大小降序取第一个

### 配置优先级（grel-config）

```
CLI 参数 > 环境变量（GREL_*） > TOML 配置文件 > 编译期默认值
```

认证 Token 优先通过环境变量传入（`GREL_GITHUB_TOKEN` 等），不写入配置文件。

### 状态持久化（grel-cache）

SQLite 数据库（`state.sqlite`）包含三张表：

- `installed` — 包元数据（forge、owner、repo、version、checksum、install_path 等）
- `etag_cache` — HTTP ETag 缓存（用于升级检测）
- `dns_cache` — DNS IP 缓存（CDN 路由优化）

包状态枚举：`active`、`orphaned`、`migrated`

---

## 关键数据结构

```rust
// grel-cache/models.rs
pub struct InstalledPackage {
    pub forge: String,
    pub owner: String,
    pub repo: String,
    pub version: String,
    pub asset_filename: String,
    pub checksum: Option<String>,
    pub install_path: String,
    pub installed_binaries: Vec<String>,
    pub is_managed: bool,
    pub status: PackageStatus,
}

// grel-core/asset.rs
pub struct AssetTokens {
    pub filename: String,
    pub os: Os,
    pub arch: Arch,
    pub format: String,
    pub version: Option<String>,
    pub is_musl: bool,
    pub is_static: bool,
}

// grel-core/package_ref.rs
pub struct PackageRef {
    pub forge: Forge,
    pub owner: String,
    pub repo: String,
}
```

---

## 开发规范

### 代码风格
- `max_width = 100`（见 `.rustfmt.toml`）
- 使用 `cargo fmt` 格式化，`cargo clippy --pedantic` 检查
- `#![warn(unsafe_code)]` — 避免使用 unsafe
- 错误处理：应用层用 `anyhow`（附带 `.context()`），库层用 `thiserror` 自定义错误类型
- 异步：全栈 tokio；不在 async 函数中调用阻塞操作

### Provider 扩展

新增 Forge 支持步骤：
1. 在 `grel-providers/src/` 下新增 `<forge>.rs`
2. 实现 `ReleaseProvider` async trait（`trait_def.rs`）
3. 在 `registry.rs` 注册新 provider

### 归档格式支持（grel-network/archive.rs）

当前支持：`tar.gz`、`tar.xz`、`tar.bz2`、`tar`、`zip`、单一二进制文件（直接复制）。

---

## CI/CD

GitHub Actions（`.github/workflows/rust.yml`）：

- **触发：** push to main、pull requests
- **矩阵：** Ubuntu + Windows
- **步骤：** `fmt --check` → `clippy -D warnings` → `build` → `test` → `build --release`

---

## 配置文件

参考 `config.example.toml`：

```toml
[general]
max_concurrent = 4
proxy = ""           # 留空则自动检测 $http_proxy/$all_proxy
keep_archives = true

[assets]
default_selection_policy = "first"
exclude_keywords = ["setup", "installer", "bundle", "nupkg"]
ignore_formats = ["*.deb", "*.rpm", "*.msi", "*.dmg", "*.AppImage"]
prefer_formats = ["*.tar.gz", "*.tar.xz", "*.zip", "*.exe"]

[paths]
install_root = "~/.local/share/grel"
bin_dir = "~/.local/share/grel/bin"
download_dir = "~/Downloads"

[auth]
github_token = ""    # 推荐使用 GREL_GITHUB_TOKEN 环境变量
```
