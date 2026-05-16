# CLI 回归复审报告

日期：2026-05-16

基线报告：`/testresult.md`（2026-05-14）

分支：`codex-test-automation-just`

## 结论

本次 codebase 更新后，上一版 `testresult.md` 中列出的若干关键问题已经被修复，但整体结论仍然不能从 `FAIL` 提升到 `PASS`。

当前状态更准确地说是：`明显改进，但仍未完成`.

主要原因有三类：

1. 旧问题中仍有数项没有修复，尤其是 `-D` 命令旗标与文档不一致、`-Qo` 仍非真实文件归属查询、`-U` 的 CLI 形式仍与文档不一致。
2. 个别旧问题只修了一半，例如非托管包的 `install_path` 语义在部分路径修正了，但 `-Syu` 升级路径仍然保留不一致。
3. 自动化测试入口出现了新的回归：`just test-offline` 当前失败，原因是测试代码没有跟上 `archive::install_asset` 的新参数签名。

## 本次实际执行

本次复审实际执行了以下命令：

```powershell
cargo run -- --help
cargo run -- -Sh
cargo run -- -Qh
cargo run -- -D --clean
cargo run -- -D --db-clean
just test-offline
```

执行结果摘要：

| 命令 | 结果 | 备注 |
|---|---|---|
| `cargo run -- --help` | 成功 | 暴露出的真实 CLI 面仍与 `docs/COMMANDS_DESIGN.md` 有差异 |
| `cargo run -- -Sh` | 成功 | 但输出仍是全局帮助，不是 `-S` 专属 scoped help |
| `cargo run -- -Qh` | 成功 | 同样仍是全局帮助 |
| `cargo run -- -D --clean` | 成功退出，但未触发数据库清理 | 仍输出 `No database operation specified. Use -Dh for help.` |
| `cargo run -- -D --db-clean` | 成功 | 现在会清理 orphan package、ETag cache、DNS cache |
| `just test-offline` | 失败 | `crates/grel-network/src/archive.rs` 的测试调用未补 `overwrite: bool` 参数 |

## 对照 `testresult.md` 的问题复核

### 1. 非托管包卸载会删掉整个下载目录

旧结论：`FAILED`

当前结论：`PARTIAL`

现状：

- `src/commands/sync.rs:361-364` 已把普通安装路径中的非托管包 `install_path` 改成具体文件路径 `archive_path`。
- `src/commands/upgrade.rs:175-178` 也对本地文件安装路径做了同样修正。
- 但 `src/commands/sync.rs` 的 `upgrade_single_package()` 仍沿用旧语义，在非托管升级时把 `updated_pkg.install_path` 设成目录而不是具体文件。

证据：

- 普通安装路径：`src/commands/sync.rs:361-364`
- 本地文件安装路径：`src/commands/upgrade.rs:175-178`
- 升级路径仍未完全修正：`src/commands/sync.rs:1464` 起的 `upgrade_single_package()` 逻辑
- 删除逻辑仍按 `install_path` 是否为目录决定 `remove_dir_all`：`src/commands/remove.rs:223-235`

判断：

- 这个问题不再像上一版那样“所有非托管路径都危险”，但也不能判定为完全修复。

### 2. `--asdeps` / 非显式安装状态不持久化

旧结论：`FAILED`

当前结论：`FIXED`

证据：

- `crates/grel-cache/src/database.rs:294-307` 现在已把 `is_explicit` 放进 `INSERT INTO installed`
- `crates/grel-cache/src/database.rs:298-307` 的 `ON CONFLICT` 分支也会更新 `is_explicit`

判断：

- 这一项已经实质修复。

### 3. `-Qt` 被错误实现为“repo unreachable orphan”，而不是“依赖孤儿”

旧结论：`FAILED`

当前结论：`FIXED`

证据：

- `src/main.rs:85-88` 现在把 `cli.unrequired` 和 `cli.orphans` 拆开路由
- `src/commands/query.rs:217-249` 新增 `cmd_list_unrequired()`，基于依赖图计算 unrequired packages
- `src/commands/query.rs:152-214` 的 `cmd_list_orphans()` 仍保留为 repo-unreachable 状态查询

判断：

- 这一项已经按语义拆正了，属于明确修复。

### 4. 文档中的 `-D --clean/--check/--dump` 没有真正接线

旧结论：`FAILED`

当前结论：`NOT FIXED`

证据：

- 运行 `cargo run -- -D --clean` 仍输出：`No database operation specified. Use -Dh for help.`
- clap 字段仍是 `db_clean/db_check/db_dump`：`crates/grel-cli/src/commands.rs:251-257`
- 实际工作命令仍是 `--db-clean/--db-check/--db-dump`

判断：

- 这一项完全没有修掉，只是实现内容增强了，但对外 CLI 名字还是不对。

### 5. `--verify-signatures` 只有开关，没有真实签名校验

旧结论：`FAILED`

当前结论：`PARTIAL`

现状：

- `src/commands/sync.rs:749-760` 现在至少会在开启验证时检查 release 里是否存在同名 `.sig` 或 `.asc` 文件，没有则跳过安装。
- 但这仍不是“验证签名”，只是“检测 sidecar 文件是否存在”。
- 没有看到对签名内容、校验链或 checksum 文件本体的加密验证。

判断：

- 相比上一版有进步，但仍不满足 `grel-config` 里“会拒绝安装无有效签名/校验的资产”的承诺。

### 6. `-Qo` / `--owns` 不是实际文件归属查询

旧结论：`FAILED`

当前结论：`NOT FIXED`

证据：

- `src/commands/query.rs:352-359` 仍然只是：
  - `install_path.contains(path)`
  - `asset_filename.contains(path)`

判断：

- 仍不是基于实际 extracted files / linked binaries 的 ownership lookup。

### 7. `--proxy` CLI 旗标未接线

旧结论：`FAILED`

当前结论：`FIXED`

证据：

- `src/main.rs:52-54` 现在会把 `cli.proxy` 写回 `config.general.proxy`
- `crates/grel-network/src/client.rs:29-36` 的 HTTP client 构造逻辑会消费这个值

### 8. `--allow-format` 暴露但未生效

旧结论：`FAILED`

当前结论：`FIXED`

证据：

- `src/commands/sync.rs:549-552` 会把指定格式从 `ignore_formats` 中移除，再构造 `ResolverConfig`

限制：

- 当前实现是“按字符串精确移除配置中的忽略项”，不是更强的模式扩展，但已经不再是未接线状态。

### 9. `--overwrite` 暴露但未生效

旧结论：`FAILED`

当前结论：`FIXED`

证据：

- `src/commands/sync.rs:311-316`、`809-814`、`1511-1516`
- `src/commands/upgrade.rs:129-134`
- `crates/grel-network/src/archive.rs:363-365`

判断：

- 覆盖逻辑已经进入 archive/link 安装路径。

### 10. `-R --nosave` 是 no-op

旧结论：`FAILED`

当前结论：`PARTIAL`

现状：

- `src/commands/remove.rs:223-235` 现在在 `nosave = false` 时会先保留配置文件。
- `src/commands/remove.rs:241-278` 新增 `preserve_config_files()`，会根据扩展名复制到 `.grelnew` 目录。

限制：

- 这是简化版实现，按扩展名猜测配置文件，不是 pacman 那种基于 package metadata 的精确保留。

### 11. `-U` 的 CLI 形式与文档不一致

旧结论：`FAILED`

当前结论：`NOT FIXED`

证据：

- 文档 `docs/COMMANDS_DESIGN.md` 仍写 `-U --asset <path>`
- 实际 CLI 字段仍是 `local_asset`：`crates/grel-cli/src/commands.rs:262-263`
- 真实实现仍读 `ctx.cli.local_asset`：`src/commands/upgrade.rs:14`、`40`

### 12. 递归删除短旗标与文档不一致

旧结论：`MISMATCH`

当前结论：`NOT FIXED`

证据：

- 文档写的是 `-R -s`
- 实现仍是 `-r, --recursive`：`crates/grel-cli/src/commands.rs:223-228`
- 更糟的是，`long_help` 里仍写 `-s (recursive)`，与真实 flag 自相矛盾：`crates/grel-cli/src/commands.rs:73-75`

### 13. README 过时

旧结论：`MISMATCH`

当前结论：`PARTIAL`

现状：

- README 已从 `grel sync / grel list / grel remove` 改成 pacman 风格 `-S / -Q / -R`
- 但 README 现在又转而示例 `-D --db-clean / --db-check`，与 `docs/COMMANDS_DESIGN.md` 继续分叉

判断：

- README 比上一版更新了，但仓库内“文档一致性”仍未达成。

### 14. `cmd_db_clean` 只做了部分工作

旧结论：`PARTIAL`

当前结论：`FIXED`

证据：

- `src/commands/database.rs:82-90` 现在会调用：
  - `db.clean_etag_cache()`
  - `db.clean_dns_cache()`
- `crates/grel-cache/src/database.rs` 也补了对应实现

### 15. `cmd_db_check` 只是打印 package count，不是真 integrity check

旧结论：`PARTIAL`

当前结论：`FIXED`

证据：

- `src/commands/database.rs:98-117` 现在会调用 `db.check_integrity()`
- `crates/grel-cache/src/database.rs` 已实现 `PRAGMA integrity_check`

## 新发现的问题

### A. `just test-offline` 当前失败，测试基线回归

严重度：`HIGH`

现象：

- 本次执行 `just test-offline` 失败
- 错误位于 `crates/grel-network/src/archive.rs:707`
- 根因是测试代码仍按旧签名调用 `link_binaries(&install_dir, &bin_dir, "mytool.tar.gz")`
- 现在 `link_binaries()` 需要第 4 个参数 `overwrite: bool`

判断：

- 这不是产品逻辑 bug，而是测试基础设施回归。
- 但它会直接阻断当前分支的离线自动化验证，因此必须在合并前修复。

### B. `-Qk` 对非托管包的 checksum 校验路径语义仍不一致

严重度：`MEDIUM`

证据：

- `src/commands/query.rs:395` 固定按 `Path::new(&pkg.install_path).join(&pkg.asset_filename)` 计算 archive path
- 但新版本里部分非托管安装路径已把 `install_path` 直接设为文件路径

影响：

- 某些非托管包可能在 `-Qk` 中被误判为 `MISSING`

### C. `-Sh` / `-Qh` 实际并没有 scoped help

严重度：`LOW`

证据：

- 实跑 `cargo run -- -Sh`
- 实跑 `cargo run -- -Qh`
- 两者都返回同一份全局 help，而不是对应 operation 的局部帮助

影响：

- 与 pacman 用户的肌肉记忆不一致
- 也和程序自己打印的“Use `grel -Sh`, `grel -Qh`...”承诺不一致

## 最终判定

相对于 `testresult.md`，这次 codebase 更新的修复进展是明显可见的，尤其是以下几项：

- `--proxy` 已接线
- `--allow-format` 已接线
- `--overwrite` 已接线
- `is_explicit` 已持久化
- `-Qt` 已纠正为 dependency orphan 语义
- `cmd_db_clean` 和 `cmd_db_check` 已不再是 stub

但以下问题依旧阻止本轮评审给出 `PASS`：

- `-D --clean/--check/--dump` 的对外 CLI 仍与文档不一致
- `-Qo` 仍未真正实现
- `--verify-signatures` 仍只有“有无 sidecar 文件”检查，没有真实签名验证
- 非托管包路径语义在 `-Syu` 升级路径中仍不一致
- `just test-offline` 当前失败

综合判定：`FAIL（较上一版明显改善，但仍未达可合入的完整状态）`
