# OCScale

[English](README.md) · **中文**

![License: MIT](https://img.shields.io/badge/license-MIT-green.svg)
![Platform: macOS / Windows](https://img.shields.io/badge/platform-macOS%20%7C%20Windows-lightgrey.svg)

**macOS 菜单栏 / Windows 系统托盘工具**，展示 OpenCode、Codex 与 DeepSeek Harness 的
**每日 Token 用量与估算花费**，支持按模型 / 项目 / Agent / MCP / Skill 的统计，
面板里还能直接看到 **DeepSeek 账户余额**。

技术栈：**Tauri 2 + React + TypeScript**（前端）/ **Rust**（数据层）。

![OCScale 面板](docs/screenshot.png)

> 100% 本地、只读。应用从不写入你的数据，没有云组件、没有遥测、不需要账号——
> **纯单机运行，默认私密**。

## 功能

- **菜单栏标签**：图标旁显示当日 Token 数（如 `⬡ 14.00M`）；Windows 托盘 API 无
  文字标签，同一数字通过悬停 tooltip 展示
- **标签模式切换**：在「当日 Token」与「DeepSeek 余额」（如 `¥12.34`）之间切换，
  菜单栏标题与 tooltip 同步跟随
- **面板**（点击托盘图标开合）：今日 / 本周 / 本月，各自对比上一周期并显示百分比
  增减；周 / 月支持 ‹ › 翻页
- **核心指标**：总 Token（input / cache / output / reasoning）、估算花费、
  请求数 / 会话数、缓存命中率与单请求花费走势
- **多维分布**：按**模型** / **项目** / **Agent** / **MCP 调用** / **Skill 调用**，
  附费用甜甜圈（hover 查看单项）与约 26 周活跃热力图
- **DeepSeek 余额**：总览页在估算费用旁显示「当前余额」；在面板里输入一次
  DeepSeek API Key 即可——本地存储（`0600`）、绝不打印日志，仅用于查询余额接口
- **日界模式**：Day 视图 / 托盘「今日」可在**本地自然日**与 DeepSeek 平台使用的
  **UTC 平台日**之间切换
- 三个页签：**概览 / Agent / 会话**
- **只统计你自己安装的 MCP / Skill**，OpenCode 内置工具一律过滤
- 附加功能：每 100M Token 里程碑彩带庆祝、面板截图保存到桌面、开机自启偏好、
  深色 / 浅色 / 跟随系统主题、EN / 中文 双语界面
- macOS：面板是浮动 NSPanel，可在全屏应用之上显示且不抢焦点

## 数据来源（零侵入，只读）

应用只**读取**本地数据，绝不写入或修改。

| 用途 | 来源 |
|------|------|
| OpenCode 消息（Token / 模型 / 工具调用） | OpenCode SQLite 数据库 —— `$XDG_DATA_HOME/opencode/opencode.db` 或 `~/.local/share/opencode/opencode.db` |
| Codex 消息（Token / MCP） | `~/.codex/sessions/**` + `~/.codex/archived_sessions/**` JSONL 会话记录 |
| DeepSeek Harness 消息（Token / MCP） | `~/.dsh/sessions/**/session.jsonl[.zstd]`（或 `$DSH_HOME/sessions`），zstd 解压 |
| 用户 MCP 白名单 | `~/.config/opencode/opencode.json` → `mcp` 对象键 |
| 用户 Skill 白名单 | `~/.config/opencode/skills/` 目录 |
| 模型价格 | **主**：[models.dev](https://models.dev/api.json) → **兜底**：[LiteLLM](https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json) → 内置快照 + 外部定价文件。缓存于 `~/Library/Caches/ocscale/`（平台 cache 目录），每 24h 刷新，离线回退 |
| DeepSeek 余额 | `GET https://api.deepseek.com/user/balance` —— Key 在面板输入，缓存 5 分钟 |

### 价格

- 价格按精确模型名匹配，再做归一化（去厂商前缀 + `.`↔`p`，如 `glm-5.1`⇄`glm-5p1`）；
  官方裸名价格优先
- **外部定价文件**（`pricing.rs` 不再写死价格）：官方 DeepSeek 单价与少量美元
  兜底模型都在用户可编辑的 `data_dir/ocscale/pricing.json`（macOS：
  `~/Library/Application Support/ocscale/pricing.json`），首次运行自动从
  `core/snapshots/pricing.json` 复制默认文件；**以后价格有变化只需改这个文件，
  无需重新构建**。文件内含**北京时间峰谷分时**（高峰 = 09:00–12:00 与
  14:00–18:00，UTC+8，无夏令时）与周末规则（2026-08-23 起周末全天按低谷价）：

  | 模型 | 低谷（未命中 / 命中 / 输出） | 高峰（未命中 / 命中 / 输出） |
  |------|------------------------------|------------------------------|
  | `deepseek-v4-flash` | ¥1.5 / ¥0.05 / ¥4.5 每 1M | ¥3.0 / ¥0.10 / ¥9.0 每 1M |
  | `deepseek-v4-pro`   | ¥4.5 / ¥0.15 / ¥13.5 每 1M | ¥9.0 / ¥0.30 / ¥27.0 每 1M |
  | `deepseek-v4-flash-vision-exp` | ¥1.5 / ¥0.05 / ¥4.5 每 1M | ¥3.0 / ¥0.10 / ¥9.0 每 1M |

  按每条事件的时戳取对应单价；缓存写入按未命中 input 单价计费。数值按中文界面
  固定 7.2 汇率折算为 USD 存储，`cost × 7.2` 即精确人民币。
- 任何来源都查不到的模型**照常统计 Token**、UI 标注「暂无定价」；OpenCode 自带
  的每条消息 `cost` 字段作为未知模型的兜底。
- 花费为按公开价格的**估算**——可理解为「等效消费价值」。

### 关键处理

- **每条 assistant 消息 = 一个事件**：按消息时间戳做小时级图表，会话级元数据
  （agent、项目、标题）来自 `session` / `project` 表
- Token 拆分：`input`（未缓存）/ `cache`（写入 + 读取）/ `output` / `reasoning`；
  UI 默认把 cache 并入「In」显示，并单列「cached %」
- Codex：每轮 Token 取 `token_count`（会话级 `total_token_usage` 是累计值 →
  用差值）；`input_tokens` 已包含 `cached_input_tokens`，未缓存 input = input − cached
- DeepSeek Harness：`usage` 直接映射（其 `inputTokens` 本就是未缓存值——无需相减），
  另有 `cr` / `cc` / `reasoning` 字段
- 工具分类：形如 `{server}_{tool}` 且前缀在 OpenCode 配置中的 → MCP；
  `skill` 工具的 `state.input.name` 命中 skills 目录 → Skill；
  内置工具（`read` / `write` / `edit` / `bash` / `grep` / `glob` / `task` /
  `todowrite` / `question` / `webfetch` 等）一律忽略

#### 四类 Token 与计价公式

每条 assistant 消息的 `tokens` 对象包含互斥的计数：

| 阶段 | 数据库字段 | 含义 |
|------|-----------|------|
| **Input**（未缓存） | `tokens.input` | 本轮新发送的提示词 Token |
| **Cache 写入** | `tokens.cache.write` | 写入提示缓存的上下文 |
| **Cache 命中**（读） | `tokens.cache.read` | 从缓存重放的上下文 |
| **Output** | `tokens.output` | 模型生成的 Token |
| **Reasoning** | `tokens.reasoning` | 推理 Token，按 output 单价计费 |

```
total = input + cache.write + cache.read + output + reasoning

cost  = input       × p.input
      + cache.write × p.cache_create
      + cache.read  × p.cache_read    # 缓存命中按折扣后的 read 单价计费
      + output      × p.output
      + reasoning   × p.output
```

缓存命中**不会**按普通 input 计费，而是用专门（更便宜）的 `cache_read` 单价——
这就是重度缓存场景下 Token 量很大、花费却不高的原因。

## 安装

从 [GitHub Releases](https://github.com/Cherno76/ocscale/releases) 下载最新版本
（macOS 为 `.dmg`，Windows 为 NSIS `.exe`），或按下方说明自行构建。

因为是**未签名 / 未公证**构建：

- **macOS**：首次打开会被 Gatekeeper 拦截 —— 右键 App →「打开」→ 再次确认，
  或执行一次 `xattr -cr /Applications/OCScale.app`
- **Windows**：首次运行有 SmartScreen 提示 —— 点「更多信息 → 仍要运行」

应用按当前用户安装、自动注册开机自启，启动后仅出现在菜单栏 / 托盘
（无 Dock 图标、无启动窗口）。

## 开发

```bash
pnpm install
pnpm tauri dev         # 启动桌面 App（需要 Rust 工具链）
```

仅预览前端（使用真实数据快照 `public/dev-dashboard.json`，gitignored、随机器而异）：

```bash
pnpm dev               # http://localhost:1420
# 刷新快照：
cd src-tauri && cargo run --example dump > ../public/dev-dashboard.json
```

Rust 单元测试：

```bash
cargo test -p ocscale
```

## 构建

```bash
pnpm tauri build       # macOS 产出 .app / .dmg，Windows 产出 .exe (NSIS)
```

产物位于 `src-tauri/target/release/bundle/`。CI 在 `git push --tags`（`v*` tag）时
构建，macOS leg 同时更新 Homebrew Cask tap。

## 结构

```
core/                 ocscale-core crate — 共享聚合核心（RawEvent → Dashboard）
  store.rs            OpenCode SQLite → RawEvent（+ 工具调用分类）
  store_codex.rs      Codex transcripts → RawEvent
  store_dsh.rs        DeepSeek Harness 会话日志 → RawEvent
  parser.rs           聚合（Day/Week/Month + 热力图）
  pricing.rs          models.dev / LiteLLM 价格加载与计价
  config.rs           用户 MCP / Skill 白名单
  model.rs            返回给前端的数据结构
src/                  React 前端（5 个文件）
  data.ts             类型 + Tauri 桥 + 主题 + 格式化
  charts.tsx          图表原语（柱状 / 甜甜圈 / sparkline / 热力图 / 分段控件）
  App.tsx             主面板
  i18n.ts             EN/ZH 词典
  main.tsx            入口

src-tauri/src/        Rust App 后端
  lib.rs              Tauri 命令 + 菜单栏托盘 + NSPanel
  balance.rs          DeepSeek 余额
  main.rs             入口
```

## Bug 记录

开发过程中遇到的典型 bug（现象、根因、解决办法）汇总在
[docs/BUGFIXES.md](docs/BUGFIXES.md)。

## 许可证

[MIT](LICENSE) © 2026 HduSy、Cherno76
