# OCScale 产品需求文档（PRD）

> 本文档描述当前实现状态。项目前身为 TokenScope（监控 Claude CLI 的 JSONL 日志），
> 2026 年 7 月迁移到 OpenCode SQLite 数据源并更名 OCScale。数据源与配置路径以
> 本文档及 `AGENTS.md` 为准。

## 1. 产品概述

### 1.1 产品名称
OCScale —— macOS 菜单栏 / Windows 托盘 OpenCode CLI 用量仪表盘

### 1.2 一句话定位
一个常驻系统菜单栏 / 托盘的小工具，实时展示 OpenCode CLI 的 Token 用量、调用统计
和估算花费，让用户对自己的 AI 编码消耗「心中有数」。

### 1.3 目标用户
- 频繁使用 OpenCode CLI 的开发者
- 关心 Token 消耗、估算花费、工具使用习惯的个人用户
- 希望了解自己 AI 工作流中哪些 MCP / Skill 真正在被使用的人

### 1.4 解决的问题
- OpenCode 自带的查看方式无法纵向看每日 / 每周 / 每月趋势
- 不知道自己装的 MCP、Skill 哪些在用、哪些是「装了不用」
- 缺少按项目、按 Agent、按模型的消耗洞察
- 缺少常驻、随时可见的用量提醒入口

## 2. 核心功能

### 2.1 菜单栏 / 托盘常驻入口
- macOS：菜单栏图标 + 当日 Token 数（如「⬡ 14.00M」）
- Windows：托盘图标；托盘 API 无文字标签，同一数字通过悬停 tooltip 展示
- 点击托盘图标开 / 关浮窗面板；后台 30s 轮询 OpenCode 数据库，准实时更新
- 开机自启默认开启（偏好持久化于数据目录），可在面板内关闭
- 单实例：重复启动只唤起已运行实例并展开面板，不出现第二个图标

### 2.2 用量仪表盘（核心）
#### 时间维度
- 今日 / 本周 / 本月，各自对比上一周期（昨日 / 上周 / 上月）显示百分比增减

#### 核心指标
| 指标 | 说明 |
|------|------|
| 会话数 | 按 session 去重计数 |
| 请求数 | 每条 assistant 消息计 1（无模型事件不计） |
| Token 用量 | input / cache / output / reasoning 四类求和 |
| 估算花费 | 按公开价格表估算（USD）；未定价模型不计入，UI 标注「暂无定价」 |

#### 多维切片
- **按模型**：token 与花费分布（未识别模型仍计 token、标注 no price）
- **按项目**：token 与花费分布、会话数
- **按 Agent**：token、花费、请求数、会话数
- **按 MCP 调用**：用户安装的 server 各自调用次数
- **按 Skill 调用**：用户安装的 skill 各自调用次数

#### 可视化
- 时段堆叠柱状图（按小时 / 按天，含 reasoning / output / cache+input）
- 费用甜甜圈（hover 查看单项）
- 约 26 周每日活跃热力图（0–4 级强度）

### 2.3 工具调用统计（只展示用户自定义安装的）
> 仅统计用户自己安装的 MCP 和 Skill，OpenCode 内置工具一律过滤。

- MCP：`part` 表工具名形如 `{server}_{tool}`，且 server 在 `opencode.json` 的
  `mcp` 对象中
- Skill：`skill` 工具的 `state.input.name` 命中 `~/.config/opencode/skills/` 目录

### 2.4 附加功能
- 每 100M Token 里程碑触发全屏彩带庆祝（快照持久化，跨重启不重复触发；首启只
  基线不庆祝）
- 面板截图保存到桌面（DOM 截图，绕过 macOS 录屏权限）
- 深色 / 浅色 / 跟随系统主题（macOS 通过原生通知同步系统外观）
- EN / 中文 双语界面（localStorage 记住选择）

## 3. 数据来源与采集

### 3.1 主数据源（只读）
OpenCode SQLite 数据库，应用只做 SELECT 查询，绝不写入 / 修改：

- 路径：`$XDG_DATA_HOME/opencode/opencode.db` 或 `~/.local/share/opencode/opencode.db`
- 核心表：
  - `message` —— 每条 assistant 消息的 JSON `data`（role / tokens / modelID / cost /
    time.created / time.completed）
  - `session` —— 会话元数据（agent、project_id、title、time_created / time_updated、
    summary_*）
  - `project` —— 项目名称
  - `part` —— 消息内的 tool 调用（MCP / Skill 分类依据）

### 3.2 Assistant 消息核心字段
```json
{
  "role": "assistant",
  "modelID": "anthropic/claude-sonnet-4-6",
  "tokens": {
    "input": 1234,
    "output": 567,
    "reasoning": 89,
    "cache": { "write": 100, "read": 2000 }
  },
  "cost": 0.01234,
  "time": { "created": 1780000000000, "completed": 1780000001234 }
}
```

### 3.3 配置数据源（用户自定义过滤）
- `~/.config/opencode/opencode.json` → `mcp` 对象键 → MCP server 白名单
- `~/.config/opencode/skills/` 目录 → Skill 白名单
- 文件 / 目录不存在时回退为空白名单（面板不展示 MCP / Skill 区块）

### 3.4 数据采集策略
- 每次构建仪表盘直接查询 SQLite，无本地事件缓存文件；`store.rs` 只做内存级
  增量比对（`ingest()` 按值比较，无变化则跳过重算）
- 保留约 210 天窗口（热力图 26 周 + 余量），更早事件裁剪
- 30s 后台轮询 + 打开面板时即时刷新；不设文件监听器（本地 SQLite 查询成本低）
- 价格表：启动时后台线程加载，之后每 24h 刷新一次；`OnceLock<RwLock<Arc<Pricing>>>`
  进程级记忆化，`build_dashboard` 只做廉价 clone，锁内永不联网

## 4. 分类与过滤规则

### 4.1 工具调用分类逻辑
```
part.data.tool 判定：
  1. 在内置工具黑名单中 → 过滤
  2. 形如 "{server}_{tool}" 且 server 在用户 MCP 配置中 → 用户 MCP
  3. == "skill" 且 state.input.name 在用户 skills 目录中 → 用户 Skill
  4. 其他 → 过滤
```

### 4.2 内置工具黑名单（硬编码于 store.rs）
```
read, write, edit, bash, grep, glob, globb, task, todowrite,
question, webfetch, websearch_web_search_exa
```

### 4.3 花费计算
#### 价格数据源（分层，高优先级先命中）
1. models.dev（主源，官方裸模型名价格）
2. LiteLLM 在线表（补 models.dev 缺口）
3. 内置 LiteLLM 快照（离线兜底，`src-tauri/snapshots/litellm.json`）
4. 硬编码的少量 Anthropic 模型（最后兜底）

缓存：平台 cache 目录 `ocscale/`（macOS 为 `~/Library/Caches/ocscale/`），24h 有效；
仅通过结构校验的响应才写入缓存，防止错误响应毒化缓存。

#### 匹配规则
精确名 → 归一化名（去厂商前缀、`.`↔`p` 统一）。匹配不到的模型：token 照常统计，
先按 OpenCode 自带 `cost` 字段兜底，仍无则标记「无定价」，不计入花费。

#### 计算公式（reasoning 按 output 单价）
```
cost = input × p.input
     + cache.write × p.cache_create
     + cache.read × p.cache_read
     + output × p.output
     + reasoning × p.output
```

#### 估算性质
花费为「按公开价格估算」，订阅用户应理解为「等效消费价值」而非真实账单；
UI 始终标注「预估费用」。

## 5. 数据精度说明

| 指标 | 精度 |
|------|------|
| 会话数 | ✅ 精确（session 去重） |
| 请求数 | ✅ 精确（每条 assistant 消息 = 1，无模型事件不计） |
| Token 用量 | ✅ 精确（来自 OpenCode 记录的 tokens） |
| 用户 MCP / Skill 调用次数 | ✅ 精确（part 表 + 配置白名单） |
| 模型 / 项目 / Agent 分布 | ✅ 精确 |
| **花费（USD）** | ✅ 按公开价格精确计算；无定价模型不计入（估算） |

## 6. 技术方案

### 6.1 技术栈（已定稿）
Tauri 2 + Rust + React 18 (TS) + Vite。理由：安装包小、常驻内存低、跨平台，
UI 复用系统 WebView（macOS 为 WKWebView）；Rust 负责 SQLite 查询、聚合、价格加载。

- 无图表库：全部图表为手写 SVG（`charts.tsx`）
- 无 lint / formatter 配置（无 ESLint、Prettier、pre-commit hooks）
- 固定 400×660 不可缩放面板；macOS 转非激活 NSPanel（level 25），可浮于全屏
  Space，失焦 / 切 Space / 切 App 自动隐藏
- `parser::build_dashboard()` 是唯一数据入口，由 `BUILD_LOCK` 串行化，在
  `spawn_blocking` 中调用，绝不在异步运行时内联执行
- 前端 `pnpm build` = `tsc && vite build`；`strictPort: true`（1420）

### 6.2 架构
```
OpenCode SQLite DB
    ↓ store.rs（SQL 查询 + part 表工具分类）
parser.rs::build_dashboard() ← BUILD_LOCK
    ├── config.rs（MCP / Skill 白名单）
    └── pricing.rs::Pricing::shared()（记忆化，后台线程 24h 刷新）
    ↓ model.rs → Dashboard JSON
Tauri command get_dashboard() / 30s 后台 refresh
    ↓
src/data.ts → App.tsx + charts.tsx
```

### 6.3 安装与分发
- 产物：macOS `.app` / `.dmg`，Windows NSIS `.exe`（`pnpm tauri build`）
- CI：`git push --tags`（`v*` tag）触发，`fail-fast: false`，macOS leg 同时更新
  Homebrew Cask tap
- 当前未签名 / 未公证：macOS 需右键 → 打开或 `xattr -cr`；Windows 有 SmartScreen
  提示。CI 的签名 Secret 已注释——无真实 Secret 时不要打开（Tauri 会把空证书
  当成「有证书」导致构建失败）

### 6.4 代码签名与公证（远期）
Developer ID 签名 + 公证是「双击直开」的唯一正解；Tauri 原生支持，配齐
`APPLE_CERTIFICATE` / `APPLE_ID` 等 Secret 后 `tauri build` 自动完成。当前版本保持
ad-hoc 签名，文档注明手动放行方式。

## 7. 非功能需求

- **性能**：常驻内存 < 100MB，空闲 CPU 低（30s 轮询，仅数据变化时重算）
- **隐私**：所有数据本地处理，不上传任何日志；仅价格表联网（models.dev / LiteLLM，
  每 24h）
- **响应**：轮询 30s + 打开面板即时刷新（未来可加文件监视器缩短到秒级）
- **稳定性**：DB 不存在 / 查询失败时返回空仪表盘，不崩溃；价格缓存损坏时回退
- **启动**：开机自启可选（默认开，偏好持久化于数据目录）

## 8. 范围与边界

### 8.1 已实现（v0.3.x）
- 菜单栏 / 托盘图标 + 今日用量速览
- 仪表盘：今日 / 本周 / 本月 + 对比、模型 / 项目 / Agent / MCP / Skill 分布、
  费用甜甜圈、26 周热力图
- Codex 数据源合并显示（OpenCode + Codex 同一管线，概览页按模型 / 项目 / Agent 切换）
- 100M 里程碑庆祝、截图、主题、双语、开机自启、单实例
- Windows 支持（托盘 tooltip、popover 拖拽 + 位置记忆）

### 8.2 不在范围
- ❌ 修改 OpenCode 数据 / 配置（只读）
- ❌ 跨设备同步、多用户聚合、Web 端
- ❌ 自定义价格表配置入口

### 8.3 后续可能扩展
- 文件监视器 / 更短刷新间隔（当前 30s）
- 预算预警（接近设定金额时通知）
- 月度报告导出（PDF / Markdown）
- 代码签名 + 公证（正式分发）
- 与其他 AI CLI 集成（数据源已抽象为 `store.rs → RawEvent`，可扩展新源）

## 9. 关键决策记录

1. **数据采集**：直接只读查询 OpenCode SQLite，不经过 hook / 代理，零侵入、零配置
2. **MCP / Skill 过滤**：仅展示用户自定义安装的，过滤全部内置工具，聚焦真正的
  用户行为
3. **花费定位**：标注「估算」，不承诺等同账单，避免与订阅制实际支出混淆
4. **价格分层**：models.dev 主源 → LiteLLM → 内置快照 → OpenCode cost 兜底；
   未知模型显示「无定价」而非 0 元
5. **缓存策略**：事件不落本地缓存（SQLite 即真源，210 天窗口内存裁剪）；仅价格表
   落盘缓存（24h，校验后写入）
6. **数据源合并**：默认合并 OpenCode + Codex（`store.rs` / `store_codex.rs` → 同一
   `RawEvent` 管线），概览页「模型 / 项目 / Agent」切换展示；项目按名称归并。
