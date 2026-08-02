# OCScale

[English](README.md) · **中文**

**macOS 菜单栏 / Windows 系统托盘工具**，展示 OpenCode CLI 的 **每日 Token 用量、
估算花费、按模型 / 项目 / Agent / MCP / Skill 的调用统计**。

技术栈：**Tauri 2 + React + TypeScript**（前端）/ **Rust**（数据层）。

![OCScale 面板](docs/screenshot.png)

## 它做什么

- 菜单栏图标旁显示当日 Token 数（如 `⬡ 14.00M`）；Windows 托盘 API 无文字标签，
  同一数字通过悬停 tooltip 展示
- 点击托盘图标开关面板：今日 / 本周 / 本月，各自对比上一周期并显示百分比增减
- 核心指标：总 Token（input / cache / output / reasoning）、估算花费、请求数 / 会话数
- 多维分布：**按模型** / **按项目** / **按 Agent** / **按 MCP 调用** / **按 Skill 调用**，
  附费用甜甜圈（hover 查看单项）与约 26 周活跃热力图
- 三个页签：**概览 / Agent / 会话**（Code 页签已移除——OpenCode 数据库从不填充代码统计）
- **只统计用户自己安装的 MCP / Skill**，OpenCode 内置工具一律过滤
- 附加功能：每 100M Token 里程碑彩带庆祝、截图保存到桌面、开机自启偏好、
  深色 / 浅色 / 跟随系统主题、EN / 中文 双语界面

## 数据来源（零侵入，只读）

应用对 OpenCode 的数据**只读不写**，绝不修改。

| 用途 | 来源 |
|------|------|
| 消息（Token / 模型 / 工具调用） | OpenCode SQLite 数据库 —— `$XDG_DATA_HOME/opencode/opencode.db` 或 `~/.local/share/opencode/opencode.db` |
| 用户 MCP 白名单 | `~/.config/opencode/opencode.json` → `mcp` 对象键 |
| 用户 Skill 白名单 | `~/.config/opencode/skills/` 目录 |
| 模型价格 | **主**：[models.dev](https://models.dev/api.json) → **兜底**：[LiteLLM](https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json) → 内置快照。缓存于 `~/Library/Caches/ocscale/`（平台 cache 目录），每 24h 刷新，离线回退 |

### 关键处理

- **每条 assistant 消息 = 一个事件**：按消息时间戳做小时级图表，会话级元数据
  （agent、项目、标题）来自 `session` / `project` 表
- Token 拆分：`input`（未缓存）/ `cache`（写入 + 读取）/ `output` / `reasoning`；
  UI 默认把 cache 并入「In」显示，并单列「cached %」
- 价格匹配：精确名 → 归一化名（去厂商前缀 + `.`↔`p`，如 `glm-5.1`⇄`glm-5p1`）；
  models.dev 官方裸名价格优先
- 成本按各 Token 类型分别计价；模型带 `priced` 标记，**任何来源都查不到的模型
  照常统计 Token、UI 标注「暂无定价」**。OpenCode 自带的每条消息 `cost` 字段
  作为未知模型的兜底
- 工具分类：形如 `{server}_{tool}` 且前缀在 OpenCode 配置中的 → MCP；
  `skill` 工具的 `state.input.name` 命中 skills 目录 → Skill；
  内置工具（`read` / `write` / `edit` / `bash` / `grep` / `glob` / `task` /
  `todowrite` / `question` / `webfetch` 等）一律忽略

> 花费为按公开价格的**估算**；订阅用户应理解为「等效消费价值」。

### 四类 Token 与计价公式

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

cost  = input      × p.input
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

Rust 单元测试（里程碑逻辑）：

```bash
cargo test -p ocscale
```

## 构建

```bash
pnpm tauri build       # macOS 产出 .app / .dmg，Windows 产出 .exe (NSIS)
```

产物位于 `src-tauri/target/release/bundle/`。CI 在 `git push --tags`（`v*` tag）时
构建，macOS leg 同时更新 Homebrew Cask tap。版本规则见 `AGENTS.md`（每次代码变更
PATCH +1，三个版本文件保持同步）。

## 结构

```
src/                  React 前端（5 个文件）
  data.ts             类型 + Tauri 桥 + 主题 + 格式化
  charts.tsx          图表原语（柱状 / 甜甜圈 / sparkline / 热力图 / 分段控件）
  App.tsx             主面板
  i18n.ts             EN/ZH 词典
  main.tsx            入口

src-tauri/src/        Rust 后端（7 个文件）
  store.rs            OpenCode SQLite → RawEvent（+ 工具调用分类）
  parser.rs           聚合（Day/Week/Month + 热力图）
  pricing.rs          models.dev / LiteLLM 价格加载与计价
  config.rs           用户 MCP / Skill 白名单
  model.rs            返回给前端的数据结构
  lib.rs              Tauri 命令 + 菜单栏托盘 + NSPanel
  main.rs             入口
```

## Bug 记录

开发过程中遇到的典型 bug（现象、根因、解决办法）汇总在
[docs/BUGFIXES.md](docs/BUGFIXES.md)。
