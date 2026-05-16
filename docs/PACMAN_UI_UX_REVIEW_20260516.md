# 当前 CLI 的 pacman 命令风格与 UI/UX 评审

日期：2026-05-16

分支：`codex-test-automation-just`

评审目标：判断当前 `grel` CLI 在“命令形态、帮助信息、交互风格、输出风格”上，是否已经接近 pacman 用户的预期。

## 总结结论

结论：`部分接近 pacman，但整体仍不够像 pacman`

当前实现已经具备 pacman 风格的骨架：

- 使用 `-S/-Q/-R/-D/-U/-F` 作为 operation
- 支持 flag 叠加，如 `-Syu`
- 大部分基础动作都围绕 pacman 的命令语义来设计

但在 CLI 细节和 UX 上仍有多处会打断 pacman 用户的肌肉记忆，尤其是：

- 帮助系统不是 operation-scoped
- 多个 long option 名称与 pacman 风格不一致
- 某些真实 flag 和帮助文本自相矛盾
- 帮助输出中存在大量空白描述或占位式描述
- 输出文案更像“普通工具提示”，不像 pacman 那种统一、克制、可预测的终端语气

## 1. 命令形态是否像 pacman

### 做得比较像的部分

| 项目 | 评价 |
|---|---|
| `-S/-Q/-R/-D/-U/-F` | 方向正确，符合 pacman 用户预期 |
| `-Syu` 这类 flag stacking | 符合 pacman 肌肉记忆 |
| `-Qe/-Qd/-Qt` 这类 query 语义 | 方向正确 |
| `-R` 下的 `-c/-n/-u` | 命名方向基本正确 |

### 不像 pacman 的部分

| 问题 | 证据 | 影响 |
|---|---|---|
| 递归删除用的是 `-r`，不是文档和 pacman 习惯中的 `-s` | `crates/grel-cli/src/commands.rs:223-228` | 直接打断 muscle memory |
| long option `--deps-filter` 不像 pacman 的 `--deps` | `crates/grel-cli/src/commands.rs:189` | 风格不自然，也与文档不一致 |
| `-D` 实际是 `--db-clean/--db-check/--db-dump`，而不是 `--clean/--check/--dump` | `crates/grel-cli/src/commands.rs:251-257` | 用户需要记一套“非 pacman 式”的额外名字 |
| `-U` 的额外路径参数是 `--local-asset`，不是文档和 pacman 语境更自然的 `--asset <path>` | `crates/grel-cli/src/commands.rs:263` | 命名显得偏实现导向，而不是用户导向 |

## 2. 帮助系统是否像 pacman

结论：`不像`

### 2.1 `-Sh` / `-Qh` 没有提供 scoped help

实跑结果：

- `cargo run -- -Sh`
- `cargo run -- -Qh`

这两个命令都只返回了同一份全局 help，而不是某个 operation 的局部帮助。

为什么这点重要：

- pacman 用户预期 `-Sh` 看 sync 帮助，`-Qh` 看 query 帮助
- 当前 `grel` 还在程序提示中鼓励用户这样做，但真实行为并没有兑现

### 2.2 帮助输出过于“平铺”

`cargo run -- --help` 暴露的是一整张全局选项表，把所有 operation 的 flag 摊平列出。

这会带来两个问题：

1. 用户很难快速建立“这个 flag 属于哪个 operation”的边界。
2. 一些同名短旗标在不同 operation 中承担不同语义时，帮助信息看起来会更混乱。

pacman 的体验通常更偏向：

- 全局入口非常短
- operation 级帮助更清楚
- 不需要用户从一张长表里自己拆语义

## 3. 帮助文本质量评估

结论：`当前帮助文本质量偏低`

### 3.1 多个 flag 没有真正的用户描述

`--help` 实际输出中，以下项目表现为“空描述”或“把 flag 名自己重复一遍”：

- `--db-clean`
- `--db-check`
- `--db-dump`
- `--local-asset`
- `--dry-run`
- `--noconfirm`
- `--overwrite`
- `--asset`
- `--platform`
- `--exclude-keywords`
- `--allow-keyword`
- `--allow-format`
- `--proxy`

这类帮助文本对于最终用户几乎没有信息价值。

### 3.2 同一份帮助里存在自相矛盾

证据：

- `crates/grel-cli/src/commands.rs:73-75` 的 `-R` long help 仍写着 `-s (recursive)`
- 但真实 flag 是 `-r, --recursive`

影响：

- 用户连帮助都不能完全相信，这会明显伤害 CLI 的可信度。

## 4. 输出与交互语气是否像 pacman

结论：`只有部分像`

### 比较接近的部分

- 整体仍然是纯文本终端交互
- 使用确认提示而不是复杂 TUI
- 有“检查中 / 清理中 / 升级摘要”这类命令行工具常见结构

### 不像 pacman 的部分

| 项目 | 现状 | 为什么不像 pacman |
|---|---|---|
| 下载选择提示 | `Proceed with download? [1-n, s=skip]` | pacman 更偏统一、克制、少岔路的确认式提示 |
| 说明性文案 | `Other compatible assets:`、`Selected:`、`Install your first package with:` | 更像面向新手的 CLI onboarding，不像 pacman 的简练风格 |
| 警告文案 | 大量 `Warning:`、`Info:` 前缀 | pacman 更常见 `warning:` / `error:` / `::` 等固定风格，而不是应用层自定义句式 |
| 彩色/标签表现 | 当前列表里会打印字面量 `[green]` / `[yellow]` | 这更像未完成的 UI 占位，而不是成熟 CLI 风格 |

证据：

- `src/commands/query.rs:65-79` 中 `status_icon` 是字符串 `"green" / "yellow"`，最终直接格式化成 `[green]` 这类字面量

## 5. 当前 CLI 与 pacman 风格的核心差距

### 5.1 命令名看起来像 pacman，但细节不够“严格”

现在的 `grel` 最大的问题不是“完全不像 pacman”，而是：

- 外观上很像
- 细节上不够严格

这会导致更强的违和感，因为用户会默认它遵守 pacman 的规则，然后在细节上被绊住。

### 5.2 文档、help、真实实现三者还没有收拢

当前至少存在三套“事实来源”：

1. `docs/COMMANDS_DESIGN.md`
2. `cargo run -- --help`
3. 实际 clap/handler 路由

这三者还没有完全一致。对于一个想要提供 pacman 兼容 UX 的 CLI，这是非常伤的。

## 6. 我对当前风格的判定

如果用 4 档来判断：

- `A`：高度贴近 pacman，老用户几乎不需要重新学习
- `B`：大方向贴近 pacman，但存在少量不一致
- `C`：只是借用了 pacman 的操作字母，细节和体验还没跟上
- `D`：基本不是 pacman 风格

我会给当前版本：`C`

理由：

- 操作字母与若干基本组合已经像 pacman
- 但帮助系统、长旗标命名、操作级帮助、部分提示文案、文档一致性，还没达到 pacman 用户可以“无缝迁移”的程度

## 7. 优先级最高的改进建议

### 第一优先级：统一“对外命令面”

先收拢这三者：

- `docs/COMMANDS_DESIGN.md`
- clap 暴露出的 `--help`
- 实际 handler 路由

在这一步完成前，不建议继续扩张 CLI 功能面。

### 第二优先级：把帮助系统做成 operation-scoped

目标：

- `grel -Sh` 只看 sync 相关
- `grel -Qh` 只看 query 相关
- `grel -Rh`、`grel -Dh` 同理

### 第三优先级：把 flag 命名改回 pacman 用户更自然的形态

优先修正：

- `--db-clean` -> `--clean`
- `--db-check` -> `--check`
- `--db-dump` -> `--dump`
- `--deps-filter` -> `--deps`
- `--local-asset` -> 更贴近文档/语义的名字
- 递归 remove 的短旗标与帮助文本统一

### 第四优先级：清理帮助文本

目标：

- 每个选项都必须有真正的用户说明
- 不要出现“`--dry-run` 的描述就是 `--dry-run`”这类占位输出
- 去掉同文档自相矛盾的情况

### 第五优先级：统一输出语气

建议方向：

- 更少“教程式”句子
- 更统一的 `warning:` / `error:` / `::` 风格
- 避免字面量 `[green]` 这类调试残留
- 把交互提示压缩成更短、更稳定的终端话语

## 最终结论

当前 `grel` 已经有了 pacman 风格 CLI 的外壳，但还没有形成 pacman 风格 CLI 的“纪律”。

如果目标只是“看起来像 pacman”，它已经做到了大半。

如果目标是“让 pacman 用户用起来几乎不出戏”，那它还需要继续收紧以下几件事：

- 对外命令面一致
- operation-scoped 帮助
- flag 命名严格对齐
- help 文案完整且无冲突
- 输出语气与 pacman 的 CLI 习惯保持统一
