# noxcode 项目构建计划

> 生成时间：2026-09-02
> 参考项目：`/Users/wenyuantc/IdeaProjects/my/codex-ai`（已逐文件读取验证，非命名推断）
> 目标目录：`/Users/wenyuantc/IdeaProjects/my/noxcode`

---

## 0. 前置澄清与现状核实

### 0.1 ZCode 实测结论（`/Applications/ZCode.app`）

ZCode 是**智谱（Z.ai）出品的闭源商业桌面应用**，本机装的是 `v3.10.1`（build `5712af2e`，2026-08-28）。
**只有打包产物，没有源码**（`out/` 下是 esbuild bundle 后的单文件，`index.js` 达 2.27MB，已 minify）。
所以下面是**黑盒架构观察**，可以作为设计参考，但无法像 codex-ai 那样逐行搬运。

#### 技术栈（`Contents/Resources/app.asar` 内 `package.json` 实测）

```
name: @zcode/desktop   version: 3.10.1   bundleId: dev.zcode.app
框架：Electron（electron-builder 26.8.1）+ React 19.2
SSH：ssh2 ^1.16.0          ← Node 纯协议实现，不调系统 ssh 命令
终端：node-pty ^1.0.0       ← 交互式 PTY
浏览器自动化：playwright-core 1.59.1
图像：sharp 0.34.5   传输：undici / ws   埋点：OpenTelemetry
自研 workspace 包：@zcode/{client,server,services,shared,ui,rpc,contracts,core}
```

#### 进程模型（`out/` 目录实测）

| 进程 | 产物 | 推测职责 |
| --- | --- | --- |
| main | `out/main/index.js`（1.49MB） | Electron 主进程 |
| **host** | `out/host/index.js`（2.27MB） | Agent 运行时宿主（最大的一块） |
| **scheduler** | `out/scheduler/index.js`（1.33MB） | 任务调度 |
| preload | `out/preload/*.cjs` × 6 | 含 `codingPlanWebview` / `cuaPermissionPanel` / `processMonitor` / `embeddedBrowserJavaScriptDialog` |
| renderer | `out/renderer/index.html` + assets | React 前端 |

`glm/zcode.cjs` 里能读到 `cliProcessTitle = "zcode-cli"` —— **桌面端内置了一个 CLI**，GUI 是 CLI 的外壳。
这与 codex-ai「Rust 进程内跑 agent」是两条不同路线。

#### 扩展体系（`Contents/Resources/glm/packages/` 实测，8 个内置插件）

```
android-emulator-plugin / browser-use-plugin / document-skills-plugin / ios-simulator-plugin
restore-legacy-sessions-plugin / skill-creator-plugin / zcode-cua-plugin / zcode-guide-plugin
```

每个插件的统一形态：

```
<plugin>/
├── package.json          # @zcode/xxx-plugin
├── skills/<name>/SKILL.md    # 技能定义（与 codex-ai native/skills.rs 同构）
├── commands/<name>.md        # 斜杠命令
├── hooks/hooks.json          # 生命周期钩子
└── dist/mcp/server.js        # MCP server（bin 入口）
```

**即 Claude Code 那套 skills + commands + hooks + MCP 的四件套。**
codex-ai 的 native 已有 `skills.rs`（发现 `.agents/skills` / `.claude/skills` / `$APPCONFIG/native-skills` 下的 `SKILL.md`）
和 `hooks.rs`（Pre/Post 钩子），但**没有 commands 和插件打包机制**。

#### 模型渠道设计（`Resources/model-providers/models_catalog_china_llm_zcode_2026-06-03.json` 实测）

schema 是 `zcode.model-providers.v1`，结构比 codex-ai 的 `model_catalog.json` 更强：

```json
{ "schemaVersion": "zcode.model-providers.v1",
  "providers": [{
    "id": "moonshot-kimi", "name": "Moonshot AI / Kimi",
    "endpoints": { "baseURL": "https://api.moonshot.cn",
      "paths": { "anthropic": "/anthropic/v1/messages",
                 "openai-compatible": "/v1/chat/completions" } },
    "defaultKind": "anthropic",
    "models": [{ "id": "kimi-k3",
      "kinds": ["anthropic", "openai-compatible"],
      "modalities": { "input": ["text","image","video"], "output": ["text"] },
      "contextWindow": 1048576, "maxOutputTokens": 131072,
      "reasoning": { "defaultLevel": "max", "levels": {
        "low":  { "anthropic": { "set": [{"path":["output_config","effort"],"value":"low"}] },
                  "openai-compatible": { "set": [{"path":["reasoningEffort"],"value":"low"}] } },
        "high": { ... }, "max": { ... } } } }] }] }
```

**三个明显优于 codex-ai 的设计点**（见 §1 决策 4）：
1. **provider 层**：一个厂商一条记录，同时挂 anthropic 与 openai-compatible 两个端点路径，用户不必为同一厂商建两条渠道
2. **思考等级用 JSON path 声明式映射**：`{"path":["output_config","effort"],"value":"low"}` —— 新增厂商只改 JSON，不改 Rust 代码。
   codex-ai 是硬编码在 `model/{openai,anthropic,responses}.rs` 里的
3. **modalities 显式声明**输入/输出模态，UI 可据此禁用图片上传

#### 其他实测细节

- 内置原生二进制工具：`Resources/tools/{ripgrep/rg, bfs/bfs, ugrep/ugrep}` —— agent 的 Grep/Glob 用编译好的二进制，不是纯 JS/Rust 遍历
- `app-update.yml` 指向 `http://localhost:8081`（这份是内测/本地构建版）
- URL scheme：`zcode://`；`NSAppleEventsUsageDescription` 说明用 Apple Events 做桌面自动化（对应 `zcode-cua-plugin`，CUA = Computer Use Agent）
- ATS 放开了 `localhost` / `127.0.0.1` 的明文 HTTP（本地 server 通信）

### 0.2 noxcode 目录现状

```
/Users/wenyuantc/IdeaProjects/my/noxcode/
├── .git/          # 干净的空仓库
└── plan.md        # 本文件
```

实测（2026-09-02 14:33，在 noxcode 目录内执行）：

```
$ git status
On branch main
No commits yet

$ git remote -v
（无输出）

$ cat .git/config      # 无 [remote] 段
$ du -sh .git
 76K
```

**结论：这是一个全新的空仓库，可以直接开始。** 无 remote、无历史、无需任何前置清理。

> 订正记录：本文件早前版本曾写「noxcode 是 codex-ai 的 clone，remote 指向 wenyuantc/codex-ai，
> 存在误 push 污染上游的风险」——那是误读。当时那条命令的 `git remote -v` 实际在 codex-ai 目录下执行，
> 读到的是 codex-ai 自己的 remote。**该风险不存在**，相关的 §7 问题 1 与 §5.2 风险条目已一并删除。

### 0.3 需求映射

| 你的需求 | codex-ai 对应实现（已验证） |
| --- | --- |
| 1. native-agent 功能 | `src-tauri/src/native/`（20 个文件，含 `agent/loop.rs` 97KB、`session.rs` 95KB、`model/client.rs` 80KB） |
| 2. AI 渠道配置 | `native/channels.rs` + `native/protocol.rs` + `native/model_catalog.rs` + `src/components/settings/AiChannelsSettingsTab.tsx` + `ChannelModelsEditor.tsx` + 表 `ai_channels` |
| 3. SSH 功能 | `app/remote.rs`（92KB，13 个命令）+ `native/tools/ssh.rs` + `engine/context.rs` + `SshSettingsTab.tsx` + 表 `ssh_configs`。<br>⚠️ 按 §1 决策 5 改用 `russh` 重写，仅约 8KB 可直接搬 |


### 0.4 三方架构对照（这是选型的关键）

| 维度 | ZCode 3.10.1 | codex-ai 0.7.0 | noxcode 建议 |
| --- | --- | --- | --- |
| 桌面框架 | Electron + electron-builder | **Tauri 2** | **Tauri 2**（理由见下） |
| 前端 | React 19.2 | React 19.1 + Vite 7 + Tailwind 4 | 同 codex-ai |
| Agent 运行时 | Node（`out/host` 独立进程 + `zcode-cli`） | **Rust 进程内**（`native/`，HTTP 直连渠道） | 同 codex-ai |
| IPC | 自研 `@zcode/rpc` | Tauri `invoke` + `emit` | 同 codex-ai |
| 存储 | 未探明（asar 已 bundle） | SQLite + SQLx 编译期校验 | 同 codex-ai |
| **SSH** | **`ssh2` Node 库**（纯协议实现） | 系统 `ssh` 子进程 + ControlMaster 多路复用 | **`russh` 0.63 库实现**（对齐 ZCode，见 §1 决策 5） |
| **Git** | **直接调系统 git** + `porcelain=v2` + `GIT_INDEX_FILE` 临时索引 + plumbing checkpoint | Node bridge（`git_bridge.mjs` 45KB）+ simple-git，`porcelain=v1`，直接 `git add` | **对齐 ZCode**（见 §1 决策 7） |
| 终端 | `node-pty`（交互式 PTY） | 无 PTY，只有 `bash -lc` 一次性执行 | 第一版不做，但 russh 自带 `request_pty`，v2 成本很低 |
| 模型目录 | `zcode.model-providers.v1`（provider 分层 + JSON path 声明式思考等级） | 扁平 `model_catalog.json` + 协议硬编码 | **借鉴 ZCode 的 schema**（见 §1 决策 4） |
| **UI 形态** | 单主界面 + 左侧两级树 + ⌘K 命令面板；权限/模型/思考等级/上下文用量在输入框内；设置为全屏页（三组导航 + 卡片） | 8 条路由多页面；模型绑在员工上，权限埋在设置页；设置为平铺 Tab | **布局学 ZCode，组件用 codex-ai**（§3 P5，6 张截图实测） |
| 扩展体系 | skills + commands + hooks + MCP，可打包成插件 | skills + hooks + MCP，**无 commands、无插件打包** | 第一版对齐 codex-ai，commands 列入 backlog |
| 搜索工具 | 内置 ripgrep / bfs / ugrep 二进制 | Rust 自实现遍历 + 子串匹配 | 第一版用 codex-ai 的实现 |
| 浏览器/CUA | playwright-core + Apple Events | 无 | 不做 |
| 埋点 | OpenTelemetry | 无 | 不做 |

**为什么 noxcode 建议走 Tauri（codex-ai 路线）而不是 Electron（ZCode 路线）**：

1. 你的三条需求（native-agent / AI 渠道 / SSH）**全部指向 codex-ai 的现成 Rust 实现**，累计约 640KB 可搬运源码。
   走 Electron 意味着这 640KB 要用 TypeScript **从零重写**，工作量约为搬运方案的 8–10 倍
2. ZCode 是闭源商业产品，**拿不到源码**，能借鉴的只有架构形态和数据结构设计
3. Tauri 包体积约 10–20MB，ZCode 的 asar 单文件就 **307MB**
4. ZCode 值得借鉴的部分（provider catalog schema、插件四件套、PTY）**与框架无关**，在 Tauri 下同样能实现

> 若你确实要 Electron 形态，这份计划需要整体重写，请先告诉我。

---

## 1. 核心技术决策（先看这一节）

### 决策 1：不能"整目录复制"，必须解耦

我统计了 `native/session.rs` 的耦合度：

```bash
grep -c "employee"                        src-tauri/src/native/session.rs   # 136 处
grep -c "task_id\|task_automation\|run_queue"  src-tauri/src/native/session.rs   # 109 处
```

`native/session.rs` 的 import 直接依赖：`fetch_employee_by_id`、`fetch_task_by_id`、`save_task_plan_content`、
`crate::codex::mcp::resolve_effective_mcp_for_task`、`crate::run_queue`、`crate::task_automation` 等。

**结论**：codex-ai 的 native agent 是深度长在「项目 / 员工 / 任务 / 看板 / Git 工作流 / 任务自动化」这套业务骨架上的。
直接 copy 会连带拖入 30 张表、57 个迁移、260 个 Tauri 命令中的大半。

**noxcode 采用「保留内核 + 替换外壳」策略**：

| 层 | 处理方式 |
| --- | --- |
| `native/model/`（协议 + HTTP 客户端） | **近乎原样搬运**，几乎无业务耦合 |
| `native/tools/`（工具运行时） | **原样搬运**，仅改 `ssh.rs` 的执行入口引用路径 |
| `native/agent/`（loop / compact / truncate / subagent） | **原样搬运**，仅改 settings 读取路径 + 一处 ~20 行改动：把 `ContextWindow` 用量通过 `on_event` 发前端（§3 P5.2 的上下文用量显示需要） |
| `native/channels.rs`、`protocol.rs`、`model_catalog.rs`、`secret_store.rs`、`settings.rs`、`skills.rs`、`transcript.rs`、`images.rs`、`api_logs.rs` | **原样搬运** |
| `native/session.rs`、`manager.rs` | **重写**：`employee` → `agent_profile`，删除 `task_id` / `run_queue` / `task_automation` / review 相关路径 |
| `app/remote.rs` | **重写为 `app/ssh/`**（§1 决策 5）：CRUD 的 DB 逻辑 + shell 转义 + 密钥存储可搬（~8KB）；执行通道与连接复用改用 `russh` 新写（~20KB） |
| `git_workflow/`(50 命令)、`git_runtime.rs`、`git_bridge.mjs` | **不搬运**，改为按 ZCode 方式新写 `git/` 模块（§1 决策 7、§3 P2.5） |
| `app/{tasks,templates,delivery,review}.rs`、`task_automation/`、`run_queue.rs`、`codex/`、`claude/`、`grok/`、`opencode/` | **不搬运** |

### 决策 2：数据表重新设计，不继承历史包袱

codex-ai 有 30 张表、57 个迁移，且存在历史命名问题（CLAUDE.md 原文："All engines share `codex_sessions` / `codex_session_events` regardless of provider; **the table names are historical**"）。

noxcode 从 **1 个 baseline 迁移**开始，共 **9 张表**：

| 表名 | 来源 | 说明 |
| --- | --- | --- |
| `ssh_configs` | 照搬 codex-ai v23 | 字段完全一致 |
| `ai_channels` | 照搬 codex-ai v48 + v50 | 直接带上 `api_key` 列，不要 `api_key_ref` 历史迁移逻辑 |
| `agent_profiles` | **新建**（替代 `employees`） | 只保留 agent 运行必需字段 |
| `workspaces` | **新建**（精简自 `projects`） | 本地/SSH 工作目录绑定 |
| `agent_sessions` | 重命名自 `codex_sessions` | 去掉 `task_id`，保留 `workspace_id` / `profile_id` |
| `agent_session_events` | 重命名自 `codex_session_events` | 字段一致 |
| `native_api_call_logs` | 照搬 codex-ai v51 | 模型调用日志 |
| `native_session_transcripts` | 照搬 codex-ai v53 | 会话上下文续聊 |
| `git_checkpoints` | **新建**（ZCode 无对应表，是本计划的设计） | AI 修改快照与回滚，见 §3 P2.5.3 |

> `codex_session_file_changes` / `file_change_details` 属于「任务执行文件变更基线」，服务于 Git 工作流与代码评审，noxcode 第一版不做，因此不建表。

### 决策 3：技术栈完全对齐 codex-ai（已验证的版本）

```
前端：React 19.1 + TypeScript 5.8 + Vite 7 + TailwindCSS 4.2 + zustand 5 + react-router-dom 7
      + i18next 26 / react-i18next 17 + lucide-react + @base-ui/react + react-hotkeys-hook 5
后端：Tauri 2.11.5 + Rust 2021 + Tokio + SQLx 0.8.6(sqlite) + reqwest 0.12(rustls) + keyring 3
插件：sql / shell / dialog / opener / notification / process / updater
SSH：russh 0.63（ring 后端）+ ssh2-config 0.8    （§1 决策 5/6）
Git：无任何 git 库 —— 直接 spawn 系统 `git`      （§1 决策 7）
```

#### 版本实测（2026-09-02，非推断）

建了一个包含**全部计划依赖**的测试工程跑 `cargo check`，**编译通过**（746 个包，无冲突）。

| 组件 | 实测锁定版本 | codex-ai 现用 | 结论 |
| --- | --- | --- | --- |
| `tauri` | **2.11.5** | 2.10.3 | ✅ 可用。rust-version 要求 1.77.2，本机 rustc 1.94.1 满足 |
| `tauri-build` | 2.6.3 | 2.5.6 | ✅ |
| `tauri-runtime` / `-wry` | 2.11.3 / 2.11.4 | 2.10.1 | ✅ 随 tauri 自动对齐 |
| `tauri-plugin-sql` | 2.4.1 | 2.4.0 | ✅ |
| `tauri-plugin-shell` | 2.3.6 | 2.3.5 | ✅ |
| `tauri-plugin-dialog` | 2.7.3 | 2.7.0 | ✅ |
| `tauri-plugin-opener` | 2.5.5 | 2.5.3 | ✅ |
| `tauri-plugin-notification` | 2.4.0 | 2.3.3 | ✅ |
| `tauri-plugin-updater` | 2.11.0 | 2.10.1 | ✅ |
| `tauri-plugin-process` | 2.3.1 | 2.3.1 | ✅ 相同 |

**npm 侧版本号对不上是正常的**（Tauri 的 npm 包与 Rust crate 独立发版）：

```
$ npx tauri info
- tauri 🦀:              =2.11.5     ← Rust crate
- @tauri-apps/api  ⱼₛ:   2.11.1      ← npm 最新只到 2.11.1
- @tauri-apps/cli  ⱼₛ:   2.11.4      ← npm 最新只到 2.11.4
（无任何版本不匹配警告，7 个插件的 🦀/ⱼₛ 两侧全部正常识别）
```

**Cargo.toml 写法建议**：写 `tauri = { version = "2.11.5", ... }`（caret 语义，允许 2.11.5 ≤ v < 3.0），
靠提交 `Cargo.lock` 保证可重现构建。只有需要严格钉死时才用 `=2.11.5`。

#### ⚠️ 实测中发现的两个真问题（与版本升级无关，但必须处理）

**问题 1：`russh` 默认引入 `aws-lc-rs`（C/汇编库），与「无 C 依赖」的前提冲突**

`cargo tree -i aws-lc-rs` 追出来：`aws-lc-rs v1.18.1 ← russh v0.63.1`。
russh 的 `default = [flate2, aws-lc-rs, rsa]`——这会把 AWS 的 libcrypto 编进来，**Windows/交叉编译要 C 工具链**，
直接推翻 §1 决策 5 里「无 C 依赖，交叉编译友好」的论据。

**解法（已实测验证）**：切到 `ring` 后端。

```toml
russh = { version = "0.63", default-features = false, features = ["ring", "flate2", "rsa"] }
```

实测结果：`aws-lc-rs` / `aws-lc-sys` **全部消失**，只剩 `ring 0.17.14`。
而 `ring` 本来就被 `reqwest` 的 `rustls-tls` 引入了 → **零新增依赖**。
`rustls` 全工程只有一个版本 `0.23.43`，russh 与 reqwest 共用，无冲突。

**问题 2：`reqwest` 出现两个版本**

`cargo tree -i reqwest@0.13.4` 追出来：`reqwest v0.13.4 ← tauri-plugin-updater v2.11.0`，
而我们自己用 `0.12.28`。两个版本可共存（编译通过），代价是二进制体积增大。

处理：**接受**。要消除只能把自己的 reqwest 升到 0.13，但 codex-ai 的 `native/model/client.rs`(80KB) 是照着
0.12 的 API 写的，升级要改搬运代码，得不偿失。在 P6 打包阶段量一下实际体积影响即可。

**运行时外部依赖只有一个：系统 `git`（≥ 2.11，2016 年发布，为 `--porcelain=v2`）。**
本地端与 SSH 远端都只要求有 `git`，**不要求 Node**。
> 明确排除：codex-ai `devDependencies` 里的 `simple-git` **不引入**；`git_bridge.mjs` 那套 Node 桥接**不移植**。
> P0 抄 `package.json` 时要把 `simple-git` 剔掉。

**数据流铁律（照搬 codex-ai 约定）**：
```
React (UI) → Tauri IPC commands → Rust service layer → SQLite
```
前端永不直接读写 SQLite：`src/lib/database.ts` 写成 hard-fail stub，`capabilities/default.json` **不授予** `sql:allow-select` / `sql:allow-execute`。


### 决策 4：模型目录采用 ZCode 的 provider 分层 schema（唯一一处不照抄 codex-ai）

codex-ai 的 `model_catalog.json` 是**扁平模型列表**，思考等级怎么映射到各协议字段是**硬编码在 Rust 里**的
（`model/openai.rs::normalize_effort`、`model/anthropic.rs::thinking_budget_tokens`、`model/responses.rs`）。
新增一个厂商就要改 Rust 代码。

noxcode 的 `model_catalog.json` 改用 ZCode 的结构（schema 名换成 `noxcode.model-providers.v1`）：

```
providers[]
├── id / name
├── endpoints.baseURL
├── endpoints.paths.{ anthropic | openai-compatible | responses }   ← 一个厂商挂多个协议端点
├── defaultKind
└── models[]
    ├── id / name / kinds[]
    ├── modalities.input[] / output[]        ← UI 据此禁用图片上传
    ├── contextWindow / maxOutputTokens
    └── reasoning.defaultLevel / levels.<level>.<kind>.set[{path,value}]
                                              ← 声明式：新增厂商只改 JSON，不改 Rust
```

对应改造：
- `native/model_catalog.rs`：`lookup_catalog` 增加 provider 维度；`fill_from_catalog` 从 provider→model 两级取默认值
- `native/model/{openai,anthropic,responses}.rs`：思考等级写入改为**读 catalog 的 `set[{path,value}]` 按 JSON path 注入 body**，
  保留原硬编码逻辑作为 catalog 未命中时的 fallback（不能直接删，很多网关模型不在目录里）
- 渠道创建 UI 增加「从 provider 目录快速填充」：选厂商 → 自动带出 baseURL、协议、模型列表

> 这是全计划**唯一**建议偏离 codex-ai 的设计点。如果你想先跑通、后优化，可以第一版直接照抄 codex-ai 的扁平 catalog，把本决策挪到 v2。

### 决策 5：SSH 改用 `russh` 库实现，不调系统 ssh 命令（对齐 ZCode）

ZCode 用 Node `ssh2@1.17.0`——**纯 JS 实现的 SSH 协议**（依赖只有 `asn1` + `bcrypt-pbkdf`，另带一个可选的
native crypto 加速模块 `sshcrypto.node`），不调用系统 `ssh` 命令。

Rust 侧有两个都叫「ssh2」的东西，必须区分清楚：

| | `ssh2` crate 0.9.6 | **`russh` 0.63.1（选用）** |
| --- | --- | --- |
| 实质 | libssh2 的 **C 绑定** | **纯 Rust** 实现 SSH 协议 |
| 与 ZCode 的 Node ssh2 同性质 | ❌ 不同（那边是纯 JS 实现） | ✅ 同性质 |
| API | 同步阻塞，Tauri 里每次都要包 `spawn_blocking` | async/tokio 原生 |
| 构建依赖 | 需 C 工具链 + OpenSSL/libssh2 交叉编译（Windows 打包麻烦） | **默认会拉 `aws-lc-rs`（C/汇编）**，必须切 `ring` feature 才真正无新增 C 依赖（§1 决策 3 已实测） |

**选定 `russh 0.63.1`**。已通过官方文档验证的能力：

```rust
// 连接 + 认证（password / publickey / certificate 三种都支持）
let tcp = TcpStream::connect("host:22").await?;
let mut client = russh::client::connect(config, tcp, handler).await?;
client.authenticate_password(user, pass).await?;
// 或：client.authenticate_publickey(user, &PrivateKeyWithHashAlg { key, hash_alg }).await?;

// 执行命令 + 流式读取
let ch = client.channel_open_session().await?;
client.exec(ch, "uname -a").await?;
while let Some(msg) = receiver.wait().await {
    match msg {
        ChannelMsg::Data { data } => stdout.extend(data),
        ChannelMsg::ExtendedData { data, ext } if ext == 1 => stderr.extend(data),
        ChannelMsg::Close => break,
        _ => {}
    }
}

// known_hosts 三态判定（比 codex-ai 的 StrictHostKeyChecking 字符串模式更精确）
match russh::keys::check_known_hosts(host, "22", key) {
    Ok(true)                => 已知且匹配，放行,
    Ok(false)               => 新主机，弹 UI 让用户确认后写入 known_hosts,
    Err(Error::KeyChanged { line }) => 主机密钥变更，MITM 告警，拒绝连接,
}

// 交互式 PTY（codex-ai 没有，ZCode 有）
client.request_pty(ch, "xterm-256color", cols, rows, 0, 0, &[]).await?;
client.shell(ch).await?;

// 保活
config.keepalive_interval = Some(Duration::from_secs(30));
config.keepalive_max = 5;
```

**这个改动的代价**（必须说清楚）：

| 影响 | 说明 |
| --- | --- |
| codex-ai 的 `remote.rs` 大部分作废 | 原计划搬运 ~35KB，现在只剩约 8KB 可用（见 §3 P2） |
| 连接复用要自己写 | ControlMaster 是 OpenSSH 的机制，russh 下改为**自维护连接池**（`HashMap<ssh_config_id, Handle>` + keepalive + 空闲回收） |
| `~/.ssh/config` 不再自动生效 | 由 `ssh2-config` 补，见下 |
| askpass 脚本机制作废 | russh 直接传密码字符串，不需要 `SSH_ASKPASS` + 临时脚本这套绕路，**这是简化** |
| shell 转义仍需保留 | `exec` 的命令字符串仍在远端 shell 里执行，`shell_escape_single_quoted` 照搬不误 |

**净收益**：不依赖机器上有 `ssh` 命令、Windows 行为一致、PTY 变得容易、known_hosts 判定更精确、错误信息是结构化的 Rust 错误而不是解析 stderr 文本。

### 决策 6：用 `ssh2-config` 解析 `~/.ssh/config`

换成库实现后会丢掉「系统 ssh 自动读 `~/.ssh/config`」这个能力。用 `ssh2-config = "0.8"` 补回来：

```
Host prod                          第一版支持范围
    HostName 10.0.1.5              ├─ ✅ Host 别名解析（UI 里填 "prod" 即可）
    Port 2222                      ├─ ✅ HostName / Port / User
    User deploy                    ├─ ✅ IdentityFile
    IdentityFile ~/.ssh/prod_ed25519  └─ ⚠️ ProxyJump 放 v2
    ProxyJump bastion
```

- 落地方式：SSH 配置表单里加一个「从 ~/.ssh/config 导入」按钮，选 Host 别名 → 自动填充 host/port/user/private_key_path
- `ProxyJump`（跳板机）需要用 russh 的 `direct-tcpip` 转发自建跳板链，**第一版不做**，但 UI 要在检测到 ProxyJump 时明确提示「暂不支持，请手动配置直连参数」，不能静默忽略

### 决策 7：Git 实现方式对齐 ZCode（直接调 git 命令 + 临时索引 + checkpoint）

> 这是需求变更：原 §1 决策 1 把 `git_workflow/` 列为「不搬运」，现按你的要求**新增 Git 能力，且按 ZCode 的方式做**。

#### 实测对比（ZCode `out/host/index.js` 与 codex-ai 源码逐条比对）

| 维度 | codex-ai | **ZCode（noxcode 采用）** |
| --- | --- | --- |
| 实现方式 | `git_bridge.mjs`（45KB Node 脚本）+ **simple-git 库**，Rust spawn 它 | **直接调系统 `git` 命令**，无库、无 bridge |
| Node 依赖 | **必需**（本地要 Node，SSH 远端也要装 —— 这正是 `install_remote_codex_sdk` / `ensure_supported_node_version` 存在的原因） | **无** |
| status 格式 | `--porcelain=v1` | **`--porcelain=v2 --branch -z`**（结构化更强，NUL 分隔避免文件名歧义） |
| 暂存操作 | 直接 `git add`（**会污染用户当前暂存区**） | **`GIT_INDEX_FILE` 临时索引** + `update-index`，**完全不碰 `.git/index`** |
| 快照/回滚 | ❌ 无 | ✅ **checkpoint 机制**（见下） |
| 重命名检测 | 未启用 | `--find-renames` |
| diff 稳定性 | 未见处理 | `--no-ext-diff --no-color --binary`（禁外部 diff 工具与颜色，保证可解析） |
| worktree | ✅ 有（`git worktree list/add/remove`，50 个命令里占一块） | ❌ 无（只有 `restore --worktree` 参数，不是 worktree 子命令） |

#### ZCode 实测到的 git 命令全集

```
路径探测   rev-parse --show-toplevel --show-prefix --absolute-git-dir --git-common-dir   ← 一次调用取全
           rev-parse --git-path index / --verify HEAD / --abbrev-ref HEAD / HEAD
状态       status --porcelain=v2 --branch --untracked-files=<mode> -z
           status --porcelain=v2 -z -- <paths>
           ls-files --cached --others --exclude-standard -z
           ls-files --stage -z -- <paths>
差异       diff --numstat -z --find-renames --
           diff --cached --numstat -z --find-renames --
           diff --numstat -z --find-renames <upstream>...HEAD --
           diff --no-ext-diff --no-color --binary [--cached] -- <path>
           diff --no-index --no-ext-diff --no-color --binary <a> <b>
           diff --name-status --find-renames -z <fromOid> <toOid> -- <path>
暂存       add -- <paths> / add -A -- <path>
           update-index（add / remove selected paths，配 GIT_INDEX_FILE）
           restore --staged -- / restore --worktree -- / restore --source=HEAD --staged --worktree --
           reset --quiet HEAD -- <paths>
提交       commit -m <msg> [-- <paths>]        ← 只提交选中路径
推送       push / push --set-upstream <remote> <branch>
分支/远端  for-each-ref refs/heads / remote / check-ref-format --branch
其他       show <rev>:<path> / log -1 --format=%ct / config --get <key> / check-ignore
checkpoint hash-object / write-tree / commit-tree / read-tree / update-ref / update-ref -d / ls-tree
```

#### checkpoint 机制（ZCode 最值得抄的设计，codex-ai 完全没有）

从 bundle 里提取到的关键证据：`{ GIT_INDEX_FILE: e, GIT_AUTHOR_NAME: ... }` 与 `a.checkpoint.refName`。

原理——**用 git 底层 plumbing 给 AI 的每次修改打游离快照，不产生任何分支提交、不碰用户暂存区**：

```
1. 建临时索引        export GIT_INDEX_FILE=<tmp>
2. 把当前工作区写进去  git add -A  （作用在临时索引上，用户的 .git/index 毫发无损）
3. 生成 tree         git write-tree                    → <tree-oid>
4. 生成游离提交       git commit-tree <tree-oid> -p HEAD -m "checkpoint"  → <commit-oid>
5. 存到自定义 ref     git update-ref refs/zcode/checkpoints/<id> <commit-oid>
                                    ↑ 不在 refs/heads 下。实测：git log / git branch 看不见，
                                      但 git log --all 看得见（详见 §8.1.6）
6. 需要回滚时         git restore --source=<commit-oid> --worktree -- <paths>
7. 清理              git update-ref -d refs/<...>
```

**noxcode 落地**：ref 命名空间用 `refs/noxcode/checkpoints/<session_id>/<seq>`。
每次 AI 会话开始前打一个 checkpoint，会话中每次写文件后可选再打一个，UI 提供「回滚到某个检查点」。
这直接替代了 codex-ai 那套 `codex_session_file_changes` + `file_change_details`（用数据库存 diff 文本），**更可靠且几乎零存储成本**。

#### 为什么这套对 noxcode 是净收益

1. **砍掉 Node 依赖链**：codex-ai 的 SSH 远端要先装 Node 才能跑 git bridge；ZCode 方式下，远端只要有 `git` 就行 —— 配合 §1 决策 5 的 russh，SSH 侧变成 `russh exec "git ..."`，链路极短
2. **不污染用户暂存区**：`GIT_INDEX_FILE` 这招让 AI 的操作和用户手上的 `git add` 完全隔离
3. **AI 改错了能回滚**：checkpoint 是 AI coding 工具的安全网，codex-ai 缺这个
4. **少写 45KB Node 代码**


---

## 2. 目标目录结构

```
noxcode/
├── package.json                    # name: noxcode
├── vite.config.ts / tsconfig.json / eslint.config.js / vitest.config.ts
├── index.html
├── src/
│   ├── App.tsx                     # 4 条路由（对比 codex-ai 的 8 条）
│   ├── main.tsx / index.css
│   ├── components/
│   │   ├── layout/                 # AppShell / SidebarTree / SidebarCommands
│   │   │                           # SidebarFooter / SshTrustBanner
│   │   ├── settings/
│   │   │   ├── SettingsLayout.tsx / SettingCard.tsx   ← 新写（ZCode 卡片式）
│   │   │   ├── AiChannelsSettingsTab.tsx    ← 需求 2
│   │   │   ├── ChannelModelsEditor.tsx      ← 需求 2
│   │   │   ├── SshSettingsTab.tsx           ← 需求 3
│   │   │   ├── NativeRuntimeSettingsTab.tsx ← 需求 1（从 RuntimeSettingsTab 85KB 中裁剪）
│   │   │   ├── NativeHooksSettingsCard.tsx
│   │   │   ├── NativeSkillsSettingsCard.tsx
│   │   │   └── McpSettingsTab.tsx
│   │   ├── home/                   # HomeEmptyState（问候语+输入卡片+预设胶囊）
│   │   ├── command/                # CommandPalette（⌘K）
│   │   ├── session/                # SessionHeader / Composer / ToolCallLine
│   │   │                           # WorkspacePicker / BranchPicker
│   │   │                           # WorkSummaryBar / 权限确认 / 计划提问弹窗
│   │   ├── workspace/              # 工作区 CRUD
│   │   ├── profile/                # Agent 档案 CRUD
│   │   └── ui/                     # 基础组件（照搬 codex-ai src/components/ui）
│   ├── pages/
│   │   ├── WorkspacePage.tsx        # 主界面（左树 + 空态/会话流）
│   │   ├── SettingsPage.tsx         # 全屏独立页（左导航三组 + 右卡片）
│   │   └── ApiCallLogsPage.tsx
│   ├── lib/
│   │   ├── backend.ts              # 全部 Tauri invoke 封装（唯一出口）
│   │   ├── database.ts             # hard-fail stub
│   │   ├── native.ts / types.ts / modelCatalog.ts / theme.ts / utils.ts
│   │   └── i18n/
│   ├── stores/                     # zustand: sessionStore / workspaceStore / profileStore / logStore
│   └── locales/                    # zh-CN / en
└── src-tauri/
    ├── Cargo.toml / tauri.conf.json / build.rs
    ├── capabilities/default.json
    └── src/
        ├── main.rs / lib.rs        # 约 45 个 Tauri 命令（对比 codex-ai 的 260 个）
        ├── db/
        │   ├── mod.rs / migrations.rs   # 1 个 baseline 迁移，8 张表
        │   └── models.rs
        ├── app/
        │   ├── mod.rs / shared.rs
        │   ├── database.rs         # 健康检查 / 备份 / 恢复
        │   ├── workspaces.rs
        │   ├── profiles.rs
        │   ├── sessions.rs
        │   ├── ssh/                ← 需求 3（russh 重写，见 §3 P2）
        │   │   ├── mod.rs / client.rs / pool.rs
        │   │   └── exec.rs / known_hosts.rs / config_file.rs
        │   └── notifications.rs
        ├── git/                    ← 按 §1 决策 7 新写（ZCode 方式）
        │   ├── mod.rs / runner.rs / repo.rs / status.rs
        │   └── diff.rs / stage.rs / commit.rs / checkpoint.rs
        ├── engine/
        │   ├── mod.rs / context.rs # ExecutionContext（local | ssh）
        │   └── usage.rs            # UsageDelta
        ├── native/                 ← 需求 1 + 需求 2（主体搬运）
        │   ├── mod.rs / manager.rs / session.rs      # 重写
        │   ├── channels.rs / protocol.rs / secret_store.rs
        │   ├── model_catalog.rs / model_catalog.json
        │   ├── settings.rs / skills.rs / subagents.rs
        │   ├── transcript.rs / images.rs / api_logs.rs
        │   ├── prompt/{mod.rs, identity.md}
        │   ├── agent/{mod.rs, loop.rs, subagent.rs, compact.rs, truncate.rs}
        │   ├── model/{mod.rs, client.rs, types.rs, sse.rs, retry.rs, usage.rs,
        │   │          openai.rs, anthropic.rs, responses.rs, call_log.rs}
        │   └── tools/{mod.rs, catalog.rs, dispatch.rs, permission.rs, patch.rs,
        │              hooks.rs, question.rs, cancel.rs, local.rs, ssh.rs,
        │              mcp.rs, web.rs, glob.rs, paths.rs}
        ├── process_spawn.rs
        ├── tray.rs / window_state.rs / window_event.rs
        └── notifications.rs        # 裁剪版
```

---

## 3. 分阶段实施计划

每个阶段都有明确的**验证标准**，未通过不进入下一阶段。

### P0 — 仓库与脚手架

| 步骤 | 动作 | 验证 |
| --- | --- | --- |
| 1 | ~~处理 git remote~~ —— **已确认无需处理**（§0.2：空仓库，无 remote 无历史）。需要推远端时再 `git remote add origin <noxcode 仓库>` | — |
| 2 | 初始化 `package.json`（name=noxcode），依赖版本对齐 codex-ai 已验证版本。**剔除 `simple-git`**（§1 决策 7 不用它） | `npm install` 成功；`grep simple-git package.json` 无输出 |
| 3 | 复制并改名配置：`vite.config.ts`（端口 1420、`@/*` 别名）、`tsconfig.json`、`eslint.config.js`、`.prettierrc`、`vitest.config.ts`、`index.html` | `npm run lint` 通过 |
| 4 | `cargo init` src-tauri。`Cargo.toml`：`tauri = { version = "2.11.5", features = ["protocol-asset","tray-icon"] }` + 7 个插件 + **`russh = { version = "0.63", default-features = false, features = ["ring","flate2","rsa"] }`**（§1 决策 3 问题 1）+ `ssh2-config = "0.8"`；含 `[lib] name = "noxcode_lib"`、`crate-type = ["staticlib","cdylib","rlib"]` | `cargo check` 通过；`cargo tree -i aws-lc-rs` **无输出**（确认 ring 后端生效） |
| 5 | `tauri.conf.json`：`productName=noxcode`、`identifier=com.wenyuan.noxcode`、`sql.preload=["sqlite:noxcode.db"]`；**先移除 updater 段**（pubkey 是 codex-ai 的，不能复用）。⚠️ 用了 `protocol-asset` feature 就**必须**同时写 `app.security.assetProtocol`，否则 `tauri-build` 直接报 allowlist 不匹配（实测踩过） | `npx tauri info` 无版本警告；`npm run tauri:dev` 能起白屏窗口 |
| 6 | `capabilities/default.json`：`core/opener/sql:default/dialog/notification/process:allow-restart`，**不含 `sql:allow-select`、`sql:allow-execute`** | 前端调 `select()` 报权限错误 |
| 7 | 建 `CLAUDE.md` + `AGENTS.md`，写明数据流铁律和"迁移版本必须连续"约束 | 文件存在 |
| 8 | 启动时探测系统 `git --version`，低于 2.11 直接报错（不静默降级）；SSH 工作区首次连接时同样探测远端 git | 无 git 的机器上启动会看到明确的中文错误提示 |

**P0 出口标准**：`npm run tauri:dev` 打开空白窗口，`npm run lint` + `cargo clippy --all-targets -- -D warnings` 全绿。

---

### P1 — 数据层

| 步骤 | 动作 | 说明 |
| --- | --- | --- |
| 1 | `db/migrations.rs` 写 **version 1 baseline**，包含 8 张表 DDL | 见 §1 决策 2 |
| 2 | 保留 codex-ai 的 `migration_versions_are_contiguous` 单测（版本号必须 1..N 连续） | 防止后续迁移插队 |
| 3 | `db/models.rs` 定义 SQLx 结构体：`SshConfig` / `SshConfigRecord` / `CreateSshConfig` / `UpdateSshConfig` / `AiChannel` / `ChannelModelConfig` / `AgentProfile` / `Workspace` / `AgentSession` 等 | `SshConfig` DTO 字段照抄 codex-ai `src/lib/types.ts:37`（含 `password_configured` / `passphrase_configured` / `password_probe_status` / `password_execution_allowed` 等派生字段） |
| 4 | `app/shared.rs`：`sqlite_pool()` / `database_path()` / `new_id()` / `now_sqlite()` / `normalize_optional_text()` / `EXECUTION_TARGET_{LOCAL,SSH}` / `PROJECT_TYPE_{LOCAL,SSH}` | 从 codex-ai `app/shared.rs` 裁剪 |
| 5 | `app/database.rs`：`health_check` / `backup_database` / `restore_database` / `open_database_folder` | 裁剪自 codex-ai（93KB → 预计 15KB） |

**关键 DDL（照抄 codex-ai v23，字段一字不改）**：

```sql
CREATE TABLE ssh_configs (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    host TEXT NOT NULL,
    port INTEGER NOT NULL DEFAULT 22,
    username TEXT NOT NULL,
    auth_type TEXT NOT NULL DEFAULT 'key' CHECK (auth_type IN ('key', 'password')),
    private_key_path TEXT,
    password_ref TEXT,
    passphrase_ref TEXT,
    known_hosts_mode TEXT NOT NULL DEFAULT 'accept-new',
    last_checked_at TEXT, last_check_status TEXT, last_check_message TEXT,
    password_probe_checked_at TEXT, password_probe_status TEXT, password_probe_message TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE TABLE ai_channels (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    protocol TEXT NOT NULL,          -- openai | anthropic | codex
    base_url TEXT NOT NULL,
    api_key TEXT,                    -- 直接落 sqlite（codex-ai v50 的最终形态）
    extra_headers_json TEXT,
    models_json TEXT NOT NULL DEFAULT '[]',
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
```

**P1 出口标准**：`cargo test` 中迁移连续性测试 + 建表冒烟测试通过；应用启动后 `$APPCONFIG/noxcode.db` 生成 8 张表。

---

### P2 — SSH 功能（需求 3，改用 `russh` 重写）

> ⚠️ 因 §1 决策 5 改用 `russh`，本阶段从「搬运 codex-ai」变成「**重写为主 + 少量搬运**」。
> codex-ai `app/remote.rs`（92KB）里只有约 8KB 仍可直接用。

#### 2.1 从 codex-ai 仍可搬运的部分（约 8KB）

| 能力 | codex-ai 位置 | 说明 |
| --- | --- | --- |
| `shell_escape_single_quoted` / `shell_escape_double_quoted` | `remote.rs:120` 及附近 | **安全关键**。russh 的 `exec` 仍是把命令字符串丢给远端 shell 执行，转义一点不能少 |
| `redact_secret_text` | `remote.rs` | 日志/错误里抹掉密码 |
| `remote_shell_bootstrap` / `remote_node_bin_dir_expression` | `remote.rs:170` / `:160` | 远端 PATH 引导，native agent 的 SSH bash 工具需要 |
| `remote_path_join` / `resolve_under_workspace_posix` | `remote.rs` / `native/tools/paths.rs` | POSIX 路径拼接与越界防护 |
| SSH 配置 CRUD 的 SQL 与校验逻辑 | `remote.rs:1058-1333` | 表结构不变，DB 读写部分可直接用，只把「执行/探测」的实现换掉 |
| 密钥存储（keyring） | `codex/secret_store.rs:471` | `store/resolve/delete_secret_value` + `sweep_orphan_secret_refs`，原样搬运 |

#### 2.2 需要新写的部分（`app/ssh/` 模块，预计 ~20KB）

```
src-tauri/src/app/ssh/
├── mod.rs          # 对外命令 + 类型
├── client.rs       # russh Handler 实现、连接建立、认证分发
├── pool.rs         # 连接池（替代 codex-ai 的 ControlMaster）
├── exec.rs         # exec / exec_with_input / 流式 spawn
├── known_hosts.rs  # check_known_hosts 三态 + 用户确认写入
└── config_file.rs  # ssh2-config 解析 ~/.ssh/config
```

**`client.rs` — Handler 与认证**
- 实现 `russh::client::Handler`，`check_server_key` 接到 `known_hosts.rs` 的三态判定
- 认证分发：`auth_type = key` → `authenticate_publickey`（读 `private_key_path`，passphrase 从 keyring 取）；
  `auth_type = password` → `authenticate_password`（密码从 keyring 取）
- `Config { keepalive_interval: 30s, keepalive_max: 5 }`

**`pool.rs` — 连接池（替代 ControlMaster）**
- `HashMap<ssh_config_id, PooledConn>`，`PooledConn` 持 `russh::client::Handle` + 最后使用时间
- 取连接时校验存活（发一次 keepalive），失效则重连
- 空闲超过 N 分钟回收；应用退出时全部关闭
- **这是本阶段最容易出 bug 的地方**，需要专门的并发测试

**`known_hosts.rs` — 三态处理**
| `check_known_hosts` 返回 | 处理 |
| --- | --- |
| `Ok(true)` | 放行 |
| `Ok(false)`（新主机） | 按 `known_hosts_mode` 分流：`accept-new` → 自动写入并放行；`strict` → 拒绝；UI 模式 → 弹指纹确认框（复用 codex-ai 的 `SshTrustBanner.tsx`） |
| `Err(KeyChanged { line })` | **一律拒绝**，报 MITM 告警并给出 known_hosts 行号 |

> 这比 codex-ai 传 `StrictHostKeyChecking=accept-new` 字符串然后解析 stderr 精确得多。
> 表里的 `known_hosts_mode` 列保持不变，语义映射到上面三态。

**`exec.rs` — 三种执行形态**（对齐 codex-ai 的三个函数，签名尽量保持一致，好让 `native/tools/ssh.rs` 少改）
- `execute_ssh_command(app, ssh_config_id, cmd) -> (stdout, stderr, exit_code)`
- `execute_ssh_command_with_input(app, ssh_config_id, cmd, stdin)`
- `spawn_ssh_stream(app, ssh_config_id, cmd) -> impl Stream<Item = ChannelMsg>`（替代 `SshStdioProcess`，用于长任务流式输出）

#### 2.3 Tauri 命令（7 个）

`list_ssh_configs` / `get_ssh_config` / `create_ssh_config` / `update_ssh_config` / `delete_ssh_config` /
`probe_ssh_password_auth` / `test_ssh_connection`

CRUD 四个的 DB 逻辑搬 codex-ai，`probe_ssh_password_auth` 与 `test_ssh_connection` 用 russh 重写
（探测方式：建连 → 认证 → `exec("echo ok && uname -a && pwd")` → 回写 `last_check_*` / `password_probe_*` 字段）。

**新增 2 个**（因 §1 决策 6）：
- `list_ssh_config_file_hosts` —— 用 `ssh2-config` 列出 `~/.ssh/config` 里的 Host 别名
- `import_ssh_config_file_host` —— 选中别名后返回解析出的 host/port/user/identity_file，供表单填充；
  若该 Host 带 `ProxyJump`，返回里带 `proxy_jump_unsupported: true`，UI 明确提示不支持

#### 2.4 执行上下文抽象

同原计划：搬运 `engine/context.rs` 的 `ExecutionContext`，改 `resolve_project_*` → `resolve_workspace_*`，
删 `resolve_task_project_execution_context`。**内部调用从「拼 ssh 命令」换成「从连接池取 russh 连接」。**

#### 2.5 前端

- `SshSettingsTab.tsx` 搬运，去掉远端 CLI 引擎区块，**新增「从 ~/.ssh/config 导入」按钮**
- `SshTrustBanner.tsx` 搬运，接到新的 known_hosts 三态（新增 `KeyChanged` 的红色告警态）
- `backend.ts` 增加 9 个 invoke 封装

#### 2.6 Cargo 依赖

```toml
russh = "0.63"
russh-keys = "0.63"        # load_secret_key / check_known_hosts / PrivateKeyWithHashAlg
ssh2-config = "0.8"        # ~/.ssh/config 解析
# 移除：不再需要通过 tauri-plugin-shell 或 Command 调系统 ssh
```

**P2 出口标准**：
1. key 认证、password 认证各建一条配置，「测试连接」返回远端 `uname -a`
2. 连接池生效：连续执行 3 条命令只建立 1 次 TCP 连接（用 `RUST_LOG` 或计数器验证）
3. known_hosts 三态各测一次：新主机弹确认框 / 已知主机直接放行 / **手动改 known_hosts 制造 KeyChanged，必须拒绝并告警**
4. `~/.ssh/config` 里配一个 Host 别名，能导入并连通；配一个带 `ProxyJump` 的，UI 明确提示不支持
5. 删除配置后 keyring 中的 password/passphrase 一并清理
6. `cargo test`：shell 转义、known_hosts 三态分流、连接池并发取用/失效重连的单测通过

### P2.5 — Git 集成（按 §1 决策 7，ZCode 方式）

**不搬运 codex-ai 的 `git_workflow/`（50 命令）、`git_runtime.rs`、`git_bridge.mjs`（45KB Node + simple-git）。**
新写 Rust 模块 `src-tauri/src/git/`，直接 spawn 系统 `git`；SSH 场景走 P2 的 `russh exec`。

#### 2.5.1 模块结构（预计 ~28KB）

```
src-tauri/src/git/
├── mod.rs          # Tauri 命令 + 类型
├── runner.rs       # git 进程执行抽象：本地 Command / 远端 russh exec 同一套接口
├── repo.rs         # rev-parse --show-toplevel --show-prefix --absolute-git-dir --git-common-dir
├── status.rs       # status --porcelain=v2 --branch -z 解析
├── diff.rs         # numstat / name-status / 单文件 diff
├── stage.rs        # GIT_INDEX_FILE 临时索引 + update-index 选择性暂存
├── commit.rs       # commit -m [-- paths] / push / push --set-upstream
└── checkpoint.rs   # write-tree + commit-tree + update-ref 快照与回滚
```

**`runner.rs` 是关键抽象**——本地与 SSH 共用同一接口，让上层逻辑写一遍：

```rust
enum GitTarget { Local(PathBuf), Ssh { config_id: String, repo_path: String } }

async fn git(target: &GitTarget, args: &[&str], env: &[(&str,&str)]) -> Result<GitOutput>;
// Local → tokio::process::Command，配 configure_std_command（Windows 隐藏 CMD 窗口）
// Ssh   → app::ssh::exec::execute_ssh_command，命令经 shell_escape_single_quoted 转义
```

#### 2.5.2 必须落实的 ZCode 细节（不能简化）

| 细节 | 原因 |
| --- | --- |
| `status --porcelain=v2 --branch -z` | v2 给出 XY 状态位、mode、oid、重命名分数；`-z` 用 NUL 分隔，**含空格/换行/中文的文件名不会解析错**。v1 做不到 |
| `--find-renames` | 不加的话重命名会显示成「删一个 + 加一个」，diff 噪音大 |
| `--no-ext-diff --no-color --binary` | 用户若配了 `diff.external` 或 `color.ui=always`，不加这三个参数解析必然崩 |
| `GIT_INDEX_FILE` 临时索引 | **绝对不能省**。省了就等于 AI 的 `git add` 会把用户手上正在准备的暂存区搅乱 |
| `GIT_AUTHOR_NAME` / `GIT_AUTHOR_EMAIL` | checkpoint 提交要标注为工具生成，与用户提交区分 |
| `check-ref-format --branch` | 创建分支前校验名称合法性，别等 git 报错 |
| `rev-parse` 四参数一次调用 | 少 3 次进程启动，SSH 下差异明显 |

#### 2.5.3 checkpoint 表

baseline 迁移新增第 9 张表：

```sql
CREATE TABLE git_checkpoints (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES agent_sessions(id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    seq INTEGER NOT NULL,               -- 会话内序号
    ref_name TEXT NOT NULL,             -- refs/noxcode/checkpoints/<session_id>/<seq>
    commit_oid TEXT NOT NULL,
    parent_oid TEXT,                    -- 打点时的 HEAD
    label TEXT,                         -- "会话开始" / "第 3 轮工具调用后"
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX idx_git_checkpoints_session ON git_checkpoints(session_id, seq);
```

**生命周期**：会话开始打第 0 号；会话中每次写文件类工具成功后打一个（可在设置里关）；
会话删除时级联删表行，**同时 `git update-ref -d` 清理 ref**（否则会变成仓库里的垃圾对象，需要在删除路径里显式处理）。

#### 2.5.4 Tauri 命令（12 个）

| 命令 | 说明 |
| --- | --- |
| `get_git_repo_info` | rev-parse 四参数 + 当前分支 + upstream |
| `get_git_status` | porcelain=v2 解析后的结构化状态 |
| `get_git_file_diff` | 单文件 diff（工作区 / 暂存区 / 两个 commit 之间） |
| `get_git_numstat` | 变更行数统计（工作区 / 暂存 / vs upstream） |
| `stage_git_paths` / `unstage_git_paths` | 走临时索引 |
| `restore_git_paths` | 丢弃工作区改动 |
| `commit_git_changes` | `commit -m <msg> [-- paths]` |
| `push_git_branch` | 含 `--set-upstream` 分支 |
| `list_git_branches` | `for-each-ref refs/heads` |
| `create_git_checkpoint` | 打快照 |
| `preview_git_checkpoint_restore` | 回滚影响面预览（§8.2.4 三类文件），确认框依赖它 |
| `list_git_checkpoints` / `restore_git_checkpoint` | 列出 / 回滚 |

#### 2.5.5 与 native agent 的接线

- `native/prompt/mod.rs` 的 `detect_local_git` / `detect_ssh_git`（组装系统提示词里的 Git 上下文）**改为调本模块**，
  不再各写一套 —— codex-ai 里这两个函数是独立实现的，noxcode 统一到 `git/`
- 写文件类工具（Write / Edit / ApplyPatch）成功后触发 `create_git_checkpoint`（异步，失败只警告不阻断）

#### 2.5.6 前端

- `components/git/GitPanel.tsx`：状态列表（分组：已暂存 / 未暂存 / 未跟踪）+ 单文件 diff 查看 + 勾选暂存/提交
- `components/git/CheckpointTimeline.tsx`：会话检查点时间线 + 「回滚到这里」按钮（**回滚前必须二次确认**，这是破坏性操作）
- diff 渲染复用 codex-ai `components/git/` 里的组件

> 两个 🔴 高危项的完整设计见 **§8**（含实测依据），P2.5 实现时以 §8 为准。

**P2.5 出口标准**：
1. 本地仓库：status / diff / 暂存 / 提交 / push 全通
2. **在有暂存内容的仓库里跑一次 AI 暂存操作，用户原有的 `git status` 输出必须完全不变**（验证 `GIT_INDEX_FILE` 隔离生效）
3. 文件名含空格、中文、换行的用例能正确解析（验证 `-z`）
4. 重命名一个文件，diff 显示为 rename 而非 delete+add（验证 `--find-renames`）
5. 打 checkpoint → 让 AI 改文件 → 回滚 → 文件恢复原状；`git log`（不带 `--all`）/ `git branch` 看不到 checkpoint
6. **回滚后，checkpoint 之后新建的未跟踪文件必须被正确处理**（实测陷阱，见 §8.2.4）
7. 删除会话后 `git for-each-ref refs/noxcode` 为空，且 `git gc --prune=now` 后无残留对象
8. SSH 工作区重复 1–7

### P3 — AI 渠道配置（需求 2）

**来源**：`native/channels.rs`(17KB) + `native/protocol.rs`(14KB) + `native/model_catalog.rs`(12KB) + `native/model_catalog.json`(26KB)

#### 3.1 后端搬运（几乎零改动）

| 文件 | 内容 |
| --- | --- |
| `channels.rs` | 6 个 Tauri 命令：`list_ai_channels`(:241) / `create_ai_channel`(:258) / `update_ai_channel`(:308) / `delete_ai_channel`(:384) / `test_ai_channel`(:423) / `list_ai_channel_models`(:466)。<br>**改动**：`delete_ai_channel` 的「被员工引用时拒绝删除」改为「被 agent_profile 引用时拒绝删除」；删除 `api_key_ref → api_key` 的历史迁移逻辑（`hydrate_channel_record`），新库直接用 `api_key` 列 |
| `protocol.rs` | 协议归一化（`openai` chat/completions、`anthropic` messages、`codex` OpenAI Responses 三套 + 别名）、`channel_chat_url` / `channel_models_url` / `normalize_base_url` / `normalize_extra_headers_json` / `parse_model_list_json` / `model_list_next_page` / `parse_channel_models_json` / `serialize_channel_models` / `record_to_channel`。**原样搬运** |
| `model_catalog.rs` + `.json` | `include_str!` 内嵌模型目录（openai/anthropic/deepseek/minimax/glm/kimi/doubao/hunyuan/gemini/mimo/qwen）。`lookup_catalog`（精确 ID / 别名 / 归一化 / 前缀模糊）、`fill_from_catalog`、`normalize_channel_model_config`、`resolve_runtime_reasoning_effort`、`apply_catalog_defaults`、命令 `list_model_catalog`。**原样搬运** |
| `secret_store.rs` | keyring 服务名从 `codex-ai-channel` 改为 `noxcode-channel`。**注意**：codex-ai 现在渠道 key 存 sqlite `api_key` 列，keyring 只留给历史迁移，noxcode 保持一致（渠道 key 落库，SSH 密码走 keyring） |

#### 3.2 需要保留的关键语义（易踩坑，源码已确认）

- `thinking_levels` 仅在为 `None` 时采用目录全集（未知模型回退 `low/medium/high`）；用户已保存的显式子集或空数组**不会**被目录新增等级覆盖
- 开启思考时写入路径**拒绝空集合**；关闭思考时清空 `thinking_level`
- 运行时 `thinking_level` 必须属于已选集合，越界回退默认等级（`resolve_runtime_reasoning_effort`）
- `list_ai_channel_models` 分页最多 500 条 / 20 页，打满仍有下一页时返回 `truncated=true`
- `extra_headers` **禁止覆盖** `authorization` / `x-api-key`

#### 3.3 前端搬运

| 文件 | 大小 | 说明 |
| --- | --- | --- |
| `AiChannelsSettingsTab.tsx` | 18.3KB | 渠道列表 + 新建/编辑弹窗 + 协议选择 + base_url + API Key（可见性切换）+ 额外请求头 JSON + 「测试连通」+「拉取模型列表」 |
| `ChannelModelsEditor.tsx` | 10KB | 模型条目编辑：`context_tokens` / `max_output_tokens` / `thinking_enabled` / `thinking_level` / `thinking_levels`，配合 `list_model_catalog` 做默认值回填 |
| `src/lib/types.ts` | — | 搬运 `AiChannelProtocol` / `AiChannelModel` / `ModelCatalogEntry` / `AiChannel`（`types.ts:75-107`） |

**P3 出口标准**：
1. 新建一条 OpenAI 兼容渠道，「测试连通」返回成功
2. 「拉取模型列表」能列出模型并写入 `models_json`
3. 分别建 anthropic 协议、codex(Responses) 协议各一条，测试连通均成功
4. 删除被 agent_profile 引用的渠道时被正确拒绝
5. `cargo test`：protocol URL 构造、模型列表解析分页、thinking 等级归一化单测通过

---

### P4 — native agent 运行时（需求 1，工作量最大）

分 5 个子阶段，每个子阶段独立可测。

#### P4.1 model 层（约 175KB 源码，**近乎零改动搬运**）

`native/model/`：`client.rs`(80KB) / `types.rs` / `sse.rs` / `retry.rs` / `usage.rs` / `openai.rs`(21KB) / `anthropic.rs`(16KB) / `responses.rs`(20KB) / `call_log.rs`(24KB)

必须保住的行为（源码已确认）：
- `chat`：按协议构造 body → `post_stream` 流式 → `parse_success_body`（先 SSE，失败再整体 JSON）
- Responses 协议在历史前缀未变时带 `previous_response_id`，只发增量；压缩或网关拒绝时失效并回退完整历史
- 网关报 `max_tokens` 超限时，自动解析上限并**最多连续下调 3 次**（`parse_max_output_token_limit`）
- HTTP 200 但 body 含 `error` 对象 → 报「模型返回错误」，**不得**报成空响应
- 重试：默认最多 10 次、固定 3s、无抖动；408/409/429/5xx 可重试，401/4xx/配额不重试；等待期可响应 `CancelFlag`
- 错误脱敏：`redact_secrets` 抹掉 `Bearer` 与 `sk-` 令牌
- 鉴权：anthropic 用 `x-api-key` + `anthropic-version`，其他用 `Authorization: Bearer`
- SSE 解析兼容「不按 spec 分帧」的中转（完整 JSON / `[DONE]` 的 `data:` 行即使无空行也单独成事件）

> `call_log.rs` 配合表 `native_api_call_logs`，是排查渠道问题的关键，一并搬运。

**验证**：写一个 `#[cfg(test)]` 用真实渠道跑通三套协议的 chat + list_models；SSE 分帧、重试判定、usage 解析的纯函数单测全绿。

#### P4.2 tools 层（约 160KB，**原样搬运，仅改 ssh.rs 的引用路径**）

`native/tools/`：`catalog.rs` / `dispatch.rs`(30KB) / `permission.rs`(22KB) / `patch.rs`(17KB) / `hooks.rs` / `question.rs` / `cancel.rs` / `local.rs`(13KB) / `ssh.rs` / `mcp.rs`(22KB) / `web.rs` / `glob.rs` / `paths.rs`

工具清单（`catalog.rs::tool_specs()`）：`Read`、`Write`、`Edit`、`ApplyPatch`、`Bash`、`Glob`、`Grep`、`TodoRead`、`TodoWrite`、`WebFetch`、`WebSearch`、`Skill`、`Agent`

必须保住的安全边界：
- `paths.rs::resolve_under_workspace` / `resolve_under_workspace_posix`：所有文件工具禁止逃逸工作区
- 写/编辑类工具要求目标文件**已先被 Read**（`ApplyPatch` 例外）
- `permission.rs`：高风险（overwrite / delete / push / force_git / mcp / opaque）需用户确认，超时可配
- `hooks.rs`：Pre 钩子退出码 2 阻断，Post 失败仅警告
- `local.rs::bash`：`bash -lc`，默认 120s / 上限 600s，支持取消与 `kill_on_drop`，输出截断 3 万字符保留尾部
- `ssh.rs::SshToolRuntime`：**唯一改动点** —— 把 `crate::app::execute_ssh_command` 指向 P2 新写的 `app::ssh::exec::execute_ssh_command`。<br>P2 刻意保持了函数签名一致（`(app, ssh_config_id, cmd) -> (stdout, stderr, exit_code)`），所以这里只改 import 路径。<br>远端命令映射不变：`cat`(read) / `mkdir -p && cat >`(write) / `rm -f`(delete) / `find . -type f | head -n 500`(glob) / `rg -n \|\| grep -R -n`(grep) / `cd <root> && bash -lc`(bash)
- `mcp.rs`：stdio MCP 客户端（本地 spawn / SSH 远端 spawn），失败跳过不回退

**验证**：本地工作区跑通 Read/Write/Edit/Glob/Grep/Bash；SSH 工作区跑通同一组；路径逃逸用例（`../../etc/passwd`）被拒绝。

#### P4.3 agent 循环层（约 155KB，**原样搬运，仅改 settings 读取路径**）

`native/agent/`：`loop.rs`(97KB) / `subagent.rs` / `compact.rs`(28KB) / `truncate.rs`(28KB)

`AgentRunner` 必须保住的行为：
- `prepare_model_call`：取消检查 → 轮次上限 → 截断/压缩 → 最后一轮移除 tools 并追加「停止调用工具」提醒
- `consume_assistant`：剥空工具名；planned last_turn 强制清空工具调用；若本轮请求带了 tools，即使预算耗尽也执行这批调用，下一轮再收尾
- 防死循环：同一工具 + 相同参数**连续调用 3 次**（`REPEAT_TOOL_LIMIT`）后直接拒绝
- 子 Agent：连续 `Agent` 调用走 `JoinSet`；同批共享**一个** `ChildQuota` 池（`remaining × subagent_budget_share_percent`），**不是**每个 child 各拿一份；子循环用 `max_subagent_turns`（与父 `max_turns` 独立）；高风险确认 FIFO；MCP 经 `SharedMcp` Mutex 共享
- `compact.rs`：`ContextWindow` 到 85% 阈值时优先用当前模型做**无工具结构化摘要**（校验标题，失败纠偏一次），再失败回退本地摘要/窗口重置；`RolloutBudget` 父子共享、原子预留结算
- `truncate.rs`：模型历史中每条工具结果默认限 4096 token，保留头尾并附「路径/offset 继续读取」提示；head 截到完整行
- 事件行前缀：`[思考]` / `[读取]` / `[命令]` / `[工具结果]` / `[子 Agent]` / `[重试]`；`[工具结果]` 保留完整输出（一条事件可含换行），UI 侧超 2000 行或 65536 字才截断

**改动点**：settings 从 `$APPCONFIG/native-settings.json` 读取的路径改为 noxcode 的 APPCONFIG。

**验证**：用 `run_scripted`（预置回复序列，无需真实模型）跑通轮次控制、重复工具拒绝、子 Agent 配额、压缩触发、截断边界；这批是 codex-ai 测试最密集的区域，测试一并搬运。

#### P4.4 session 层（**重写，本计划最核心的改造**）

原 `native/session.rs` 95KB / 136 处 employee / 109 处 task。noxcode 重写为约 30KB。

| codex-ai 原逻辑 | noxcode 处理 |
| --- | --- |
| `start_native_session` 先过 `run_queue` 任务级门控 | **删除**（无任务并发队列） |
| `fetch_employee_by_id` → 员工模型/思考等级/system_prompt | 改为 `fetch_agent_profile_by_id` |
| `fetch_task_by_id` / `save_task_plan_content` / review 相关 | **删除** |
| `resolve_effective_mcp_for_task` | 改为 `resolve_effective_mcp_for_workspace` |
| `capture_native_execution_change_baseline` 文件变更基线 | **删除**（不做 Git 工作流） |
| `handle_session_exit_blocking` 任务自动化触发 | **删除** |
| 会话记录写 `codex_sessions` | 改写 `agent_sessions`（去 `task_id`） |
| 事件写 `codex_session_events` + 广播 `native-stdout` | 改写 `agent_session_events`，事件名保持 `native-stdout` / `native-exit` / `native-session` |
| `run_native_one_shot` 协调器计划/测试验收 | **保留**（一次性调用有独立价值：思考模型只回 `reasoning_content` 时的兜底、DeepSeek V4 必须显式 `thinking.type=disabled`），但去掉任务上下文参数 |

保留的 Tauri 命令（9 个）：
`start_native_session` / `stop_native_session` / `stop_native` / `restart_native_session` / `resume_native_session` /
`send_native_input` / `finish_native_input` / `resolve_native_tool_permission` / `answer_native_plan_question`

`manager.rs` 同步改造：键从 `session_record_id` 保持不变，`NativeLiveSession` 去掉 task 维度查询（`get_task_process_any`），保留 `has_profile_processes` / `get_profile_processes`。

`prompt/mod.rs`：`compose_system` 保留组装顺序 —— identity.md → 环境块（工作目录/平台/日期/权限/模型名）→ Git 上下文（`detect_local_git` / `detect_ssh_git`）→ 全局提示词模板 → 项目指令（`AGENTS.md` / `Agents.md` / `CLAUDE.md`，本地或 SSH 读取，单文件上限 32KB 去重）→ profile 设定。
**改动**：「员工设定」改为「agent profile 设定」；「AI 提示词模板库」若不搬运 `codex/prompt_templates.rs`，则改为从 `native-settings.json` 读一个全局模板字符串。

`transcript.rs` / `settings.rs` / `skills.rs` / `subagents.rs` / `images.rs` / `api_logs.rs`：**原样搬运**，仅改表名与配置路径。

**验证**：
1. 建一个本地工作区 + 一条渠道 + 一个 profile，启动会话，输入「读一下 README.md 并总结」→ 能看到 `[读取]` 事件与最终文本
2. 输入触发写文件 → 弹出高风险确认，允许/拒绝均正确
3. 停止后 `resume_native_session` 能从 `native_session_transcripts` 恢复上下文继续对话
4. 把工作区切到 SSH，重复 1-3

#### P4.5 native 设置与子 Agent 管理

- `settings.rs`：`$APPCONFIG/native-settings.json`，字段与默认值照搬 —— `max_turns`(40, 0=不限, 上限 500) / `max_subagent_turns`(20) / 高风险确认 / `permission_timeout_secs`(300, 0=不超时) / `max_concurrent_subagents`(1, 1-16) / `subagent_policy`(conservative) / `subagent_budget_share_percent`(40, 5-100) / `context_window_tokens`(128000, 8000-1000000) / `rollout_token_budget`(10000000, 0=不限) / `max_tool_output_tokens`(4096, 256-65536) / `hooks[]`。旧 JSON 缺字段按保守默认归一化
- `skills.rs`：发现工作区 `.agents/skills` / `.claude/skills` 与全局 `$APPCONFIG/native-skills` 下的 `SKILL.md`
- `subagents.rs`(26KB)：自定义子 Agent CRUD（`list/create/update/delete_native_subagent`）
- `api_logs.rs`(31KB)：`list_native_api_call_logs` / `get_native_api_call_log`

**P4 出口标准**：三套协议 × 本地/SSH 两种工作区 = 6 条链路全部能完成一次「读文件 → 改文件 → 汇报」的完整循环。

---

### P5 — 前端 UI（布局学 ZCode，组件用 codex-ai）

#### 5.0 ZCode 界面实测（v3.10.1，6 张实际截图 + orca 可访问性树）

##### (1) 首页空态

```
┌────────────────────────┬──────────────────────────────────────────────────────┐
│ ●●● ⊟ ← → [更新]       │                                        ? ⊟          │
│ ⊕ 新建任务       ⌘N    │                                                      │
│ 🔍 搜索          ⌘K    │                                                      │
│ ⏱ 自动化               │           下午好呀，接下来交给我吧      ← 时段问候   │
│ ⊞ 插件市场             │                                                      │
│ ───────────────        │   ┌────────────────────────────────────────────┐    │
│ [# 分组][📁 项目] ⤢ ≡ ⊟│   │ [📁 codex-ai ⌄] [⑂ main ⌄]   ← 卡片顶部     │    │
│ 项目                   │   ├────────────────────────────────────────────┤    │
│  📁 item-center        │   │ 向 ZCode 提问，使用 @ 添加上下文，          │    │
│  📁 codex-ai      ▼    │   │ 使用 / 选择命令或能力                       │    │
│    分析项目        4分 │   │                                            │    │
│    你好          23小时│   │ + │⚠完全访问⌄│  ollama/…⌄ │⊙最高⌄│ ↑ │   │    │
│    原生Agent…      1天 │   └────────────────────────────────────────────┘    │
│    显示更多            │                                                      │
│  📁 proxy-pool-system  │   [⊙周报总结] [※报错修复] [🖵PPT制作] [☾闲时任务]   │
│  …                     │        ↑ 预设 prompt 胶囊，一键起会话               │
│ ───────────────        │                                                      │
│ 👤 eztballs      📱 ⚙  │                                                      │
└────────────────────────┴──────────────────────────────────────────────────────┘
```

关键点：
- **时段问候语**（下午好呀…），空态不是空白页
- **工作区选择器 + 分支选择器在输入卡片顶部**，不是只在会话页顶栏 —— 开始对话前就能定好上下文
- placeholder 明示两个输入语法：**`@` 添加上下文**、**`/` 选择命令或能力**
- **4 个预设 prompt 胶囊**：周报总结 / 报错修复 / PPT 制作 / 闲时任务
- 首页**不显示**上下文用量（还没会话），进入会话后才出现

##### (2) 工作区选择器下拉（点 `📁 codex-ai ⌄`）

```
┌─────────────────────────────┐
│ 🔍 搜索工作区               │ ← 顶部搜索
├─────────────────────────────┤
│ 📁 item-center              │
│ 📁 codex-ai              ✓  │ ← 当前项打勾
│ 📁 proxy-pool-system        │
│ 📁 trade-center             │
│ 📁 admin-service            │
├─────────────────────────────┤
│ ⊞ 打开文件夹                │ ← 加本地工作区
│ ☁ 远程连接                  │ ← ★ SSH 入口就在这里
│ 💬 不在项目中工作            │ ← 无工作区模式
└─────────────────────────────┘
```

**「远程连接」是 SSH 的入口**——这对需求 3 很关键：ZCode 把 SSH 当作"另一种工作区来源"，
和"打开文件夹"平级，而不是像 codex-ai 那样塞进设置页的一个 Tab。
选中后 chip 变为 `✕ codex-ai ⌄`（带清除按钮）。

##### (3) 分支选择器下拉（点 `⑂ main ⌄`）

```
┌─────────────────────────────────────┐
│ 🔍 搜索分支                         │
├─────────────────────────────────────┤
│ 分支                                │
│ ⑂ main                           ✓  │
│ ⑂ ai-workflow/20260902-api-c…       │ ← 长分支名截断
│ ⑂ cursor/p3-send-input-planning     │
├─────────────────────────────────────┤
│ + 创建并检出新分支…                 │
│ ⑂ Git 图谱                          │
└─────────────────────────────────────┘
```

对应 §3 P2.5 的 `list_git_branches` / `create_git_branch`。「Git 图谱」是可视化提交图，属 backlog。

##### (4) ⌘K 命令面板（全屏遮罩 + 背景虚化）

```
┌──────────────────────────────────────────────┐
│ 🔍 搜索操作、任务或文件                      │
│ [≡ 全部][🚀 操作][💬 任务][📄 文件]  ← 类型过滤│
├──────────────────────────────────────────────┤
│ 最近任务                                     │
│  💬 分析项目                            6分  │
│  💬 你好                                1天  │
│  💬 原生Agent实现分析                   1天  │
│ 建议                                         │
│  ⊕ 新任务                              ⌘N   │
│  📁 打开工作区                          ⌘O   │
│  ⚙ 设置                                     │
│ 面板                                         │
│  ⊟ 切换侧边栏                          ⌘B   │
│  ⊡ 切换终端                            ⌘J   │
│  🌐 切换预览                                 │
└──────────────────────────────────────────────┘
```

这是**统一命令面板**（操作 + 任务 + 文件三合一 + 分组结果 + 右侧快捷键提示），
不是 codex-ai 那种只搜数据的搜索框。

##### (5) 设置：**全屏独立页面，不是 Dialog**（订正上一版）

```
┌────────────────────────┬─────────────────────────────────────────────────┐
│ ← 返回工作区           │  常规                                       ?   │
│                        │  [中文简体]  ← 当前值 badge                     │
│ 基础设置               │                                                 │
│  ⚙ 常规          ←选中│  ┌───────────────────────────────────────────┐  │
│  🎨 外观               │  │ 界面语言                    [中文简体 ⌄]  │  │
│  📦 模型设置           │  │ 选择应用 UI 的显示语言。                  │  │
│  🌐 浏览器控制         │  └───────────────────────────────────────────┘  │
│  🖥 电脑控制           │  ┌───────────────────────────────────────────┐  │
│                        │  │ 继承系统终端 Profile              [ ●]   │  │
│ Agent 能力             │  │ 启动内置终端时尽量继承登录 shell 环境…    │  │
│  🧠 记忆               │  ├───────────────────────────────────────────┤  │
│  🤖 子智能体           │  │ 终端字体                        [保存]    │  │
│  ⊞ 插件                │  │ 留空时自动探测系统终端配置；…             │  │
│  🔌 MCP 服务器         │  │ [留空自动继承，例如 MesloLGS NF…       ]  │  │
│  ✦ 技能                │  ├───────────────────────────────────────────┤  │
│  >_ 命令               │  │ 增强 Find 和 Grep                 [ ●]   │  │
│  ⚓ 钩子               │  └───────────────────────────────────────────┘  │
│                        │  ┌───────────────────────────────────────────┐  │
│ 数据与统计             │  │ HTTP 代理                       [保存]    │  │
│  🛡 索引库             │  ├───────────────────────────────────────────┤  │
│  📊 使用统计           │  │ 不使用代理的地址                [保存]    │  │
│                        │  ├───────────────────────────────────────────┤  │
│  🚀 引导               │  │ 自定义证书                      [保存]    │  │
│                        │  └───────────────────────────────────────────┘  │
│ 👤 eztballs         ⚙  │  ┌ Chrome 硬件加速                  [ ●]  ┐  │
└────────────────────────┴─────────────────────────────────────────────────┘
```

**四点值得抄的设置页设计**：

| # | 设计 | 对比 codex-ai |
| --- | --- | --- |
| 1 | 导航按**语义分组**：基础设置 / Agent 能力 / 数据与统计 | codex-ai 是一排平铺的 Tab，8 个并列没有层次 |
| 2 | **相关项合并进同一张卡片**（终端 Profile + 终端字体 + Find/Grep 一张；HTTP 代理 + 不代理地址 + 自定义证书 一张） | codex-ai 每项一个区块，长页面很散 |
| 3 | 每项统一结构：**标题 + 一句话描述 + 右侧控件**；文本输入配独立「保存」按钮，开关即时生效 | 需要抽一个 `SettingCard` 组件统一 |
| 4 | 标题下方有**当前值 badge**（「中文简体」） | 一眼看到关键项现值 |

**ZCode 设置项里暴露的能力线索**：
- 「增强 Find 和 Grep」→ 对应 `Resources/tools/{rg,bfs,ugrep}` 三个内置二进制（§6 backlog 已列）
- 「HTTP 代理 / 不使用代理的地址 / 自定义证书」→ 企业内网场景，证书注入 `NODE_EXTRA_CA_CERTS`。
  **noxcode 应该抄这三项**：模型请求（reqwest）、MCP、命令工具都要能走代理，否则内网用户直接不可用
- 「记忆」「命令」→ codex-ai 都没有，属 §6 backlog
- 「索引库」「使用统计」→ 代码索引与 token 统计

#### 5.1 路由结构（按截图订正）

```tsx
<Route path="/"          element={<WorkspacePage />} />   // 主界面：左树 + 空态/会话流
<Route path="/settings"  element={<SettingsPage />} />    // ★ 全屏独立页（左导航+右卡片）
<Route path="/settings/:section" element={<SettingsPage />} />  // 深链到具体分节
<Route path="/api-logs"  element={<ApiCallLogsPage />} /> // 模型调用日志
```

> **订正**：上一版我写「设置走全屏 Dialog」是错的。实测截图里设置有「← 返回工作区」，
> 是一个占满窗口的独立页面（左侧仍保留用户/设置底栏），所以做成路由而非 Dialog。

**快捷键**（实测自命令面板，集中声明在 `src/lib/shortcuts.ts`）：

| 键 | 动作 |
| --- | --- |
| ⌘N | 新建会话 |
| ⌘K | 命令面板 |
| ⌘O | 打开工作区 |
| ⌘B | 切换侧边栏 |
| ⌘J | 切换终端（第一版无终端，占位不注册） |

#### 5.2 需要新写的组件

| 组件 | 估算 | 说明 |
| --- | --- | --- |
| `layout/AppShell.tsx` | 3KB | 左栏 + splitter + 主区，侧栏宽度持久化 |
| `layout/SidebarTree.tsx` | 3KB | 工作区→会话两级树，相对时间、选中高亮、「显示更多」 |
| `layout/SidebarCommands.tsx` | 1KB | 顶部命令区，快捷键右对齐灰字 |
| `layout/SidebarFooter.tsx` | 1KB | 当前 profile + 设置入口 |
| `home/HomeEmptyState.tsx` | 4KB | 时段问候语 + 居中输入卡片 + 预设 prompt 胶囊 |
| `session/WorkspacePicker.tsx` | 5KB | 下拉 + 搜索 + **打开文件夹 / 远程连接(SSH) / 不在项目中工作** |
| `session/BranchPicker.tsx` | 4KB | 下拉 + 搜索 + 创建并检出新分支 |
| `session/Composer.tsx` | 8KB | 输入区：`@` 上下文、`/` 命令、四个控件（见下） |
| `session/ToolCallLine.tsx` | 2KB | 工具调用单行摘要 + 展开详情 |
| `session/WorkSummaryBar.tsx` | 1KB | 「已工作 N 秒 ⌄」折叠条 |
| `session/SessionHeader.tsx` | 2KB | 会话标题 + 工作区 chip + 分支 chip + 更多 |
| `command/CommandPalette.tsx` | 8KB | ⌘K：操作/任务/文件三类过滤 + 分组结果 + 快捷键提示 |
| `settings/SettingsLayout.tsx` | 3KB | 左导航（三组）+ 右内容区 + 返回工作区 |
| `settings/SettingCard.tsx` | 3KB | 统一卡片：标题 + 描述 + 控件（开关即时 / 输入配保存按钮） |
| **合计** | **~48KB** | |

> ⚠️ **工作量订正**：上一版按单张截图估的 +15KB 不够。补全 6 张截图后是 **~48KB**。
> 多出来的主要是命令面板(8KB)、工作区/分支选择器(9KB)、首页空态(4KB)、设置布局(6KB)——
> 这些在上一版根本没看到。前端新写总量从 ~38KB 升到 **~86KB**。

**`Composer.tsx` 要接的能力**：

| UI 元素 | 数据来源 | 状态 |
| --- | --- | --- |
| `@` 添加上下文 | 文件/目录选择器，插入路径引用 | 新写 |
| `/` 命令 | §6 backlog 的斜杠命令；**第一版只做 `/` 触发技能列表**（`skills.rs` 已有） | 降级实现 |
| `⚠ 完全访问 ⌄` | `tools/permission.rs`，第一版两档：每次确认 / 会话内允许 | 已有后端 |
| `44,933 / 384,000` | `agent/compact.rs` 的 `ContextWindow` | **需后端改 ~20 行**（§1 决策 1 已标注） |
| 模型选择器 | `channels.rs` 的 `models_json` | 已有 |
| `最高 ⌄` 思考等级 | `model_catalog.rs::resolve_runtime_reasoning_effort` | 已有 |

#### 5.2.1 设置页分节映射（左导航按 ZCode 分组，内容用 codex-ai 组件）

| ZCode 分组 | ZCode 分节 | noxcode 第一版 | 内容来源 |
| --- | --- | --- | --- |
| 基础设置 | 常规 | ✅ 常规 | 新写：语言 + **HTTP 代理 / 不代理地址 / 自定义证书**（企业内网必需） |
| | 外观 | ✅ 外观 | `lib/theme.ts` |
| | 模型设置 | ✅ **AI 渠道** | 照搬 `AiChannelsSettingsTab` + `ChannelModelsEditor`（需求 2） |
| | 浏览器控制 / 电脑控制 | ❌ 不做 | — |
| | — | ✅ **SSH 连接** | 照搬 `SshSettingsTab`（需求 3）。ZCode 把 SSH 放在工作区下拉里，noxcode **两个入口都留**：工作区下拉「远程连接」快速新建 + 设置页集中管理 |
| Agent 能力 | 子智能体 | ✅ 子智能体 | `subagents.rs` 已有 |
| | MCP 服务器 | ✅ MCP | 照搬 `McpSettingsTab` |
| | 技能 | ✅ 技能 | 照搬 `NativeSkillsSettingsCard` |
| | 钩子 | ✅ 钩子 | 照搬 `NativeHooksSettingsCard` |
| | — | ✅ **内置 Agent 运行时** | 从 `RuntimeSettingsTab`(85KB) 裁剪 native 区块（轮次/上下文/预算/权限超时） |
| | 记忆 / 插件 / 命令 | ❌ 放 §6 backlog | — |
| 数据与统计 | 使用统计 | ✅ 使用统计 | 复用 `native_api_call_logs` 聚合 |
| | 索引库 | ❌ 不做 | — |
| — | — | ✅ 数据库维护 | `DatabaseSettingsTab` 裁剪 |
| — | — | ✅ 关于 | `AboutUpdateSection` |

> **新增需求**：ZCode 的「HTTP 代理 / 不使用代理的地址 / 自定义证书」三项要抄。
> 落到 noxcode 是：`ModelClient`(reqwest) 走代理、MCP 子进程注入代理环境变量、
> 自定义 CA 证书注入 reqwest 的 `add_root_certificate`。这是**后端新增工作**，约 3KB，记在 P3。

#### 5.3 直接照搬 codex-ai 的部分（不动）

| 来源 | 说明 |
| --- | --- |
| `components/ui/*` | shadcn 底座。**实测 ZCode 也是 shadcn + Tailwind + lucide-react**，底座通用 |
| `AiChannelsSettingsTab.tsx`(18KB) + `ChannelModelsEditor.tsx`(10KB) | 需求 2，照搬 |
| `SshSettingsTab.tsx`(16KB) | 需求 3，去掉远端 CLI 区块 + 加「从 ~/.ssh/config 导入」 |
| `NativeHooksSettingsCard` / `NativeSkillsSettingsCard` / `McpSettingsTab` | 照搬 |
| 从 `RuntimeSettingsTab.tsx`(85KB) 裁剪出的 native 设置区块 | 只挑 native 部分 |
| `DatabaseSettingsTab.tsx` 裁剪 | 备份/恢复 |
| 事件流渲染 + `@tanstack/react-virtual` 虚拟滚动 | 长会话必需 |
| 权限确认弹窗 / 计划提问弹窗 | 对接 `onNativePermissionRequest` / `onNativePlanQuestion` |
| `components/git/` 的 diff 渲染组件 | 供 §3 P2.5 的 GitPanel 复用 |

上述设置类 Tab 全部塞进一个全屏 Dialog 的左侧 tab 列表里（对齐 ZCode 的设置形态），不做独立路由。

#### 5.4 lib 与 store（同原计划）

- `lib/backend.ts` 唯一 invoke 出口；`lib/database.ts` hard-fail stub
- `lib/native.ts` 搬运事件监听封装与类型
- `stores/`：`sessionStore` / `workspaceStore` / `profileStore` / `logStore`
- `locales/`：zh-CN 为主，en 骨架

#### 5.5 明确不抄 ZCode 的部分

| ZCode 有 | 为什么不做 |
| --- | --- |
| 自动化面板、插件市场（左栏两项） | 属 §6 backlog |
| 浏览器控制 / 电脑控制（CUA）| 与三条核心需求无关，且依赖 playwright + Apple Events |
| xlsx/docx/pdf/pptx 预览（4 个 wasm 共 ~7MB） | 无关 |
| 记忆、索引库 | 需要向量库/嵌入模型，独立子系统 |
| 移动端远程控制、Git 图谱、切换预览 | backlog |
| 👍/👎 反馈 | 需要上报链路，本地工具无收益 |
| 分组视图（`# 分组` tab） | 第一版只做「项目」视图 |
| 「引导」新手引导流程 | 第一版不做 |

#### 5.6 预设 prompt 胶囊（首页那 4 个）

ZCode 给的是「周报总结 / 报错修复 / PPT 制作 / 闲时任务」——偏办公场景。
noxcode 面向编码，改为四个更贴合的（内容存 `$APPCONFIG/quick-prompts.json`，用户可改）：

| 胶囊 | 预填 prompt |
| --- | --- |
| 🔍 解读代码库 | 分析当前工作区的架构与模块职责，输出结构化报告 |
| 🐛 修复报错 | 我遇到这个报错：（粘贴），定位原因并修复 |
| ✅ 补测试 | 为（文件/函数）补充测试用例并跑通 |
| 📝 写提交信息 | 看当前 git 变更，生成 Conventional Commit 信息 |

**P5 出口标准**：
1. `npm run build`（tsc + vite）通过
2. **首页空态**：问候语 + 输入卡片 + 4 个胶囊，点胶囊能预填并起会话
3. **工作区选择器**：搜索 / 切换 / 打开文件夹 / **远程连接（新建 SSH 配置）** / 不在项目中工作，五条路径全通
4. **分支选择器**：搜索 / 切换 / 创建并检出新分支
5. **⌘K 命令面板**：三类过滤 + 最近会话 + 建议 + 面板三组结果，快捷键提示正确
6. **设置全屏页**：三组导航可跳转，深链 `/settings/channels` 可用；`SettingCard` 的开关即时生效、输入框走「保存」
7. 输入框内四个控件全部可用且实时反映后端状态
8. 工具调用单行摘要可展开
9. 侧栏宽度可拖拽并持久化，⌘B 可折叠
10. 完整走通「配渠道 → 配 SSH → 选工作区 → 起会话 → 对话 → 停止 → 恢复」

### P6 — 打包与质量门禁

| 项 | 动作 |
| --- | --- |
| updater 签名 | `npm run tauri signer generate` **生成 noxcode 自己的密钥对**，替换 `tauri.conf.json` 的 `pubkey` 与 `endpoints`（当前 codex-ai 的 pubkey 与 GitHub endpoint **绝对不能复用**） |
| 图标 | 用 `tauri icon` 生成 noxcode 自己的图标集 |
| 版本同步脚本 | 搬运 `scripts/bump-version.mjs`（同步 package.json / Cargo.toml / tauri.conf.json） |
| 托盘与窗口 | 搬运 `tray.rs` / `window_state.rs` / `window_event.rs`（窗口尺寸持久化、关闭到托盘） |
| Windows 兼容 | 搬运 `process_spawn.rs::configure_std_command`（codex-ai commit `9d0a148` 修的「隐藏本地子进程 CMD 窗口」，别漏） |
| CI | 搬运 `.github/workflows/lint.yml` |
| 打包 | `npm run tauri:dmg:no-sign` / `tauri:windows` / `tauri:linux` |

**质量门禁（每次提交前必须全绿）**：
```bash
npm run lint
npm run format:check
npm run test:ci
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
```

---

## 4. Tauri 命令清单（约 60 个，对比 codex-ai 的 260 个）

| 模块 | 数量 | 命令 |
| --- | --- | --- |
| `app::ssh`（SSH） | 9 | `list/get/create/update/delete_ssh_config`、`probe_ssh_password_auth`、`test_ssh_connection`、`list_ssh_config_file_hosts`、`import_ssh_config_file_host` |
| `native::channels`（渠道） | 6 | `list/create/update/delete/test_ai_channel`、`list_ai_channel_models` |
| `native::model_catalog` | 1 | `list_model_catalog` |
| `native::session` | 9 | `start/stop/restart/resume_native_session`、`stop_native`、`send/finish_native_input`、`resolve_native_tool_permission`、`answer_native_plan_question` |
| `native::settings` | 2 | `get/update_native_settings` |
| `native::skills` | 2 | `list_native_global_skills`、`open_native_skills_dir` |
| `native::subagents` | 4 | `list/create/update/delete_native_subagent` |
| `native::api_logs` | 2 | `list_native_api_call_logs`、`get_native_api_call_log` |
| `git`（§1 决策 7） | 13 | `get_git_repo_info`、`get_git_status`、`get_git_file_diff`、`get_git_numstat`、`stage/unstage_git_paths`、`restore_git_paths`、`commit_git_changes`、`push_git_branch`、`list_git_branches`、`create/list/restore_git_checkpoint` |
| `app::workspaces` | 5 | `list/create/update/delete_workspace`、`check_workspace_health` |
| `app::profiles` | 4 | `list/create/update/delete_agent_profile` |
| `app::sessions` | 3 | `list_agent_sessions`、`get_agent_session_log_lines`、`prepare_agent_session_resume` |
| `app::database` | 4 | `health_check`、`backup_database`、`restore_database`、`open_database_folder` |
| `mcp`（若搬运） | 3 | `get/update/reset_mcp_servers` |
| `tray` | 1 | `show_main_window` |

---

## 5. 工作量与风险

### 5.1 代码量估算（基于实际文件大小）

| 阶段 | 搬运（低风险） | 重写（高风险） | 备注 |
| --- | --- | --- | --- |
| P0 脚手架 | — | ~1k 行 | 配置为主 |
| P1 数据层 | — | ~2k 行 | DDL + models |
| P2 SSH | ~8KB | **~20KB** | 改用 russh 后由搬运变重写（§1 决策 5） |
| **P2.5 Git** | 0 | **~28KB** | 全新写（§1 决策 7）。对比：codex-ai 该能力是 45KB Node + 34KB Rust |
| P3 渠道 | ~70KB | ~5KB | channels/protocol/catalog 近乎原样 + **HTTP 代理/自定义证书 ~3KB**（§3 P5.2.1） |
| P4.1 model | ~175KB | 0 | 零改动 |
| P4.2 tools | ~160KB | ~1KB | 只改 ssh.rs 引用 |
| P4.3 agent | ~155KB | ~1KB | 只改 settings 路径 |
| P4.4 session | ~60KB | ~30KB | **核心改造区** |
| P4.5 设置/子Agent | ~85KB | ~3KB | — |
| P5 前端 | ~120KB | **~86KB** | 裁剪 RuntimeSettingsTab + GitPanel/CheckpointTimeline + **ZCode 布局层 ~48KB**（§3 P5.2，按 6 张截图订正） |
| P6 打包 | ~10KB | ~1KB | — |

### 5.2 风险清单

| 风险 | 等级 | 应对 |
| --- | --- | --- |
| `session.rs` 解耦时漏掉隐式依赖，编译一路报错 | 🔴 高 | 按 P4.1→4.5 顺序自底向上搬运，每层 `cargo check` 通过再进下一层 |
| SQLx 用 `query_as!` **编译期校验**，表结构必须先建好 | 🟡 中 | P1 必须完整完成，且本地要有一份已迁移的 db 供编译期校验；或统一改用运行时 `query_as` |
| updater pubkey 复用 codex-ai 导致签名校验失败/安全问题 | 🔴 高 | P0 先删 updater 段，P6 生成自己的密钥 |
| `native/tools/mcp.rs` 与 `permission.rs` 在 codex-ai 中权限位是 `-rw-------` | 🟡 中 | 搬运后确认文件权限与 git 是否正常跟踪 |
| SSH 密码认证探测 | 🟢 低 | russh 直接传密码，askpass 那套绕路作废；保留 `probe_ssh_password_auth` 预探测 + `password_execution_allowed` 门控即可 |
| **连接池实现**（替代 ControlMaster）并发取用/失效重连出错 | 🔴 高 | P2 最易出 bug 处，专门写并发单测；先做「每次新建连接」的朴素版跑通功能，再加池化 |
| russh 误用默认 features 拉进 `aws-lc-rs`，Windows 打包需 C 工具链 | 🟡 中 | P0 步骤 4 已固化 `default-features = false` + `ring`；CI 加断言 `cargo tree -i aws-lc-rs` 必须无输出 |
| russh 与部分老旧 SSH 服务器的算法协商失败（老 KEX / 老 HostKey 算法） | 🟡 中 | russh 默认算法集偏现代。预留 `Config` 里的算法列表可配；测试要覆盖一台老服务器 |
| `~/.ssh/config` 的 ProxyJump 用户实际在用，第一版不支持 | 🟡 中 | 导入时显式提示不支持（不静默忽略）；v2 用 russh `direct-tcpip` 实现跳板链 |
| **checkpoint ref 泄漏**：会话删了但 `refs/noxcode/*` 没清，仓库里堆垃圾对象 | 🟡 中 | 删除路径显式 `update-ref -d`；再加一个启动时的孤儿 ref 清扫（对照 `git_checkpoints` 表） |
| `GIT_INDEX_FILE` 用错导致污染用户暂存区 | 🔴 高 | **完整方案见 §8.1**：三类操作分流 + IndexMode 类型强制 + runner 运行期守卫 + CI grep 断言 + `--no-optional-locks`（实测 status 会偷写 index）+ index 字节级 sha256 验证 |
| `restore_git_checkpoint` 是破坏性操作，误触会丢用户未保存的改动 | 🔴 高 | **完整方案见 §8.2**：四步流程（前置校验 → 影响面预览 → 自动 pre-restore checkpoint → 分半执行）。**实测发现 `restore --worktree` 不删 checkpoint 之后新建的文件**，必须显式处理 |
| 用户 git 版本过低不支持 `--porcelain=v2`（需 git ≥ 2.11，2016 年） | 🟢 低 | 启动时 `git --version` 探测，低于 2.11 明确报错而非静默降级 |
| 前端 `RuntimeSettingsTab.tsx` 85KB 强耦合四个 CLI 引擎 | 🟡 中 | 不整文件搬，只挑 native 区块重写 |
| ZCode 无源码可搬，只能黑盒借鉴设计 | 🟢 低 | 已在 §0.4 / §1 决策 4-5 / §6 消化完毕，不构成返工风险 |
| 决策 4（provider 分层 catalog）偏离 codex-ai，无现成代码 | 🟡 中 | 保留原硬编码逻辑作 fallback；也可先照抄扁平 catalog，v2 再改 |

### 5.3 建议执行顺序

```
P0 (仓库+脚手架) → P1 (数据层) → P2 (SSH) → P2.5 (Git) → P3 (渠道)
                                                              ↓
P6 (打包)  ←  P5 (前端)  ←  P4.5 ← P4.4 ← P4.3 ← P4.2 ← P4.1
```

P2 必须在 P4.2 之前完成（`tools/ssh.rs` 依赖 SSH 执行通道）；
P3 必须在 P4.1 之前完成（`model/client.rs` 需要渠道配置才能测通）；
P2.5 必须在 P2 之后（Git 的 SSH 执行走 russh 通道），且在 P4.4 之前（`prompt/mod.rs` 的 Git 上下文与 checkpoint 接线依赖它）。

---

## 6. 从 ZCode 借鉴、但不进第一版的清单（backlog）

按优先级排序，都是 ZCode 有、codex-ai 没有、且值得做的：

| 优先级 | 能力 | ZCode 实现 | noxcode 落地方式 |
| --- | --- | --- | --- |
| P1 | **斜杠命令（commands）** | `<plugin>/commands/<name>.md` | 与 `skills.rs` 同构：扫 `.agents/commands` / `$APPCONFIG/native-commands` 下的 `.md`，前端输入框 `/` 触发 |
| P1 | **插件打包机制** | `glm/packages/<plugin>/` = skills + commands + hooks + MCP 四件套 | 定义 `noxcode-plugin.json` 清单，一个目录同时注册四类扩展，替代现在 skills/hooks/MCP 三处分散配置 |
| **P1↑** | **交互式终端（PTY）** | `node-pty` | 远端已白送：russh 的 `request_pty` + `shell` 直接可用（§1 决策 5 已验证）。本地端补 `portable-pty` 即可。**改用 russh 后这项成本大幅下降，建议提前** |
| P2 | **内置 ripgrep 二进制** | `Resources/tools/{rg,bfs,ugrep}` | 大仓库下 Grep 工具性能会明显好于 codex-ai 的逐行子串匹配；作为 sidecar 打包 |
| P2 | **git worktree 隔离** | ZCode **没有**（这项是 codex-ai 独有） | 让每个会话在独立 worktree 里跑，互不干扰。codex-ai `git_workflow/worktree.rs` 有现成实现可参考 |
| P3 | **进程监控面板** | `out/preload/processMonitor.cjs` + `process-monitor.html` | 展示当前会话派生的所有子进程（bash / MCP / SSH），可单独 kill |
| P3 | **计划视图 webview** | `out/preload/codingPlanWebview.cjs` | codex-ai 的计划模式只有文本，可做成结构化可勾选清单 |
| P2 | **斜杠命令完整版** | `/` 选择命令或能力（ZCode 首页 placeholder 明示） | 第一版只做 `/` 触发技能列表；完整版需 `commands/*.md` 发现机制 |
| P3 | **Agent 记忆** | 设置页「记忆」分节 | 跨会话长期记忆，需独立存储与召回策略 |
| P3 | **代码索引库** | 设置页「索引库」分节 | 需嵌入模型 + 向量库 |
| P3 | **Git 图谱** | 分支下拉里的「Git 图谱」 | 提交图可视化 |
| P4 | 浏览器自动化 / CUA | playwright-core + Apple Events | 与三条核心需求无关，暂不考虑 |
| P4 | OpenTelemetry 埋点 | `@opentelemetry/*` | 本地工具，暂不需要 |

---

## 7. 决策记录与待确认项

### 已确认

| 项 | 决定 | 出处 |
| --- | --- | --- |
| SSH 实现 | `russh 0.63.1`（纯 Rust），不用系统 ssh 子进程、不用 ssh2 crate | §1 决策 5 |
| `~/.ssh/config` | 用 `ssh2-config 0.8` 解析，ProxyJump 放 v2 | §1 决策 6 |
| Git 实现 | 直接调系统 `git`，不引入 simple-git / Node bridge | §1 决策 7 |
| 业务外壳范围 | **最小外壳**：工作区 + Agent 档案 + 会话。不做项目/员工/看板/任务自动化 | 本计划全文按此设计 |
| git remote | 无需处理，noxcode 本就是空仓库 | §0.2（订正） |
| 前端形态 | **布局学 ZCode**（单主界面 + 左侧两级树 + ⌘K 命令面板 + 输入框内控件 + 全屏设置页），**组件照搬 codex-ai** | §3 P5（6 张截图实测，新写量 ~48KB） |

### 待确认

暂无阻塞项。计划可按 §3 的 P0 → P6 顺序开始执行。

若后续需要，这两项可随时提出：
- **完整业务外壳**（项目/员工/看板/Git 工作流/任务自动化）：工作量约翻 3 倍，需要多搬 `git_workflow/` 50 命令、`task_automation` 12 命令、`app/tasks.rs` 27 命令与 22 张表
- **§6 backlog 里的能力**提前到第一版（斜杠命令、插件打包、PTY、内置 ripgrep 等）

---

## 8. 两个 🔴 高危项的专项设计

> 本节的每条结论都在 `/tmp/gitprobe` 的真实仓库上实测过（git 2.39.5），实测命令与输出附在各小节。
> P2.5 实现时以本节为准。

---

## 8.1 高危项 A：`GIT_INDEX_FILE` 用错污染用户暂存区

### 8.1.1 先厘清：不是"所有操作都用临时索引"

这是最容易搞错的地方。git 操作要分**三类**，各自的 index 策略不同：

| 类别 | 典型操作 | index 策略 | 理由 |
| --- | --- | --- | --- |
| **A 只读** | status / diff / ls-files / log / rev-parse / show | 不写 index，且**必须加 `--no-optional-locks`** | 见 8.1.2，不加会偷偷写 |
| **B 用户显式暂存** | 用户在 GitPanel 点「暂存这个文件」/「取消暂存」 | **写真实 `.git/index`** | 这是用户意图，写真实暂存区才是正确行为，不算污染 |
| **C 工具内部操作** | checkpoint 快照、只提交选中文件、AI 触发的任何 git 写操作 | **必须 `GIT_INDEX_FILE` 临时索引** | 用户完全没有感知，绝不能碰真实暂存区 |

> 把 B 也塞进临时索引是错的——用户点了「暂存」结果暂存区没变，会让人以为按钮坏了。

### 8.1.2 实测依据：`git status` 会偷偷写 `.git/index`

```
$ touch a.txt b.txt          # 只改 mtime，内容不变
$ shasum .git/index → 1c5d0f71fa950a3c
$ git status --porcelain=v2 -z > /dev/null
$ shasum .git/index → 1f6df3e190dba5fe    ⚠️ 变了（刷新 stat 缓存）

$ touch a.txt b.txt
$ shasum .git/index → 1f6df3e190dba5fe
$ git --no-optional-locks status --porcelain=v2 -z > /dev/null
$ shasum .git/index → 1f6df3e190dba5fe    ✅ 未变
```

**后果（不加 `--no-optional-locks` 的话）**：
1. 会去抢 `.git/index.lock`，用户此刻正在终端里 `git add` 就会报 `Unable to create index.lock`
2. 8.1.6 的字节级验证永远过不了，等于失去自动化防线

**规定：所有 A 类只读命令一律加 `--no-optional-locks`，无例外。**

### 8.1.3 结构性防护：让"忘记加"在编译期/运行期就炸

不能靠开发者记性。`runner.rs` 里做三层：

```rust
// 第 1 层：类型强制。IndexMode 是 git() 的必填参数，没有默认值，想省略都不行
pub enum IndexMode {
    /// A 类：只读。runner 自动注入 --no-optional-locks
    ReadOnly,
    /// B 类：写真实 .git/index。构造函数是 pub(in crate::git::stage)，
    ///       只有 stage.rs 能造出来，别的模块编译期就拿不到
    UserIndex(UserIndexToken),
    /// C 类：临时索引。RAII，Drop 时清理
    Scratch(ScratchIndex),
}

pub async fn git(
    target: &GitTarget,
    args: &[&str],
    mode: IndexMode,        // ← 必填，无 Default
) -> Result<GitOutput> { ... }
```

```rust
// 第 2 层：运行期守卫。即便类型绕过了，这里也拦得住
const INDEX_WRITING_SUBCOMMANDS: &[&str] = &[
    "add", "rm", "mv", "update-index", "read-tree", "reset",
    "checkout", "switch", "stash", "commit", "apply", "am", "cherry-pick", "merge",
];

fn guard(args: &[&str], mode: &IndexMode) -> Result<()> {
    let sub = first_non_flag(args);                       // 跳过 -c/--git-dir 等全局参数
    let writes_index = INDEX_WRITING_SUBCOMMANDS.contains(&sub)
        || (sub == "restore" && args.contains(&"--staged"));   // restore 只有带 --staged 才写 index
    match (writes_index, mode) {
        (true, IndexMode::ReadOnly) =>
            Err("BUG: 写 index 的命令用了 ReadOnly 模式".into()),   // 直接失败，不降级
        (false, IndexMode::ReadOnly) => Ok(()),
        _ => Ok(()),
    }
}
```

```rust
// 第 3 层：环境变量只在 runner 内部注入，业务代码碰不到
match mode {
    IndexMode::ReadOnly    => cmd.arg("--no-optional-locks"),      // 自动加，业务不用管
    IndexMode::UserIndex(_) => { /* 不设 GIT_INDEX_FILE，走默认 */ }
    IndexMode::Scratch(ix)  => cmd.env("GIT_INDEX_FILE", ix.path()),
}
```

**再加一条 CI 级的死规矩**：全仓库禁止 `git` 二字出现在 `src-tauri/src/git/runner.rs` 之外的 `Command::new` / `execute_ssh_command` 调用里。用一条 grep 断言进 CI：

```bash
! grep -rn 'Command::new("git")\|execute_ssh_command.*"git ' src-tauri/src \
    --include=*.rs | grep -v 'src-tauri/src/git/runner.rs'
```

### 8.1.4 `ScratchIndex`：两种构造，一个是性能关键

```rust
impl ScratchIndex {
    /// 用于 checkpoint。复制用户 index → 保留 stat 缓存 → 后续 add -A 只 hash 变化的文件
    /// 大仓库上这是数量级差异：从空索引 add -A 要 hash 全部文件
    pub async fn from_user_index_copy(target: &GitTarget) -> Result<Self>;

    /// 用于「只提交选中文件」。read-tree HEAD 拿干净基线，再 update-index 加选中项
    pub async fn from_head(target: &GitTarget) -> Result<Self>;
}
```

> 复制 `.git/index` 是**读操作**，不污染。实测已验证（8.1.6）。

**Drop 语义**：
- 本地：`Drop` 里直接 `std::fs::remove_file`，同步删，没问题
- **SSH：`Drop` 不能 `await`**。方案是双保险：
  1. 正常路径显式 `cleanup().await` 发一条 `rm -f`
  2. `Drop` 里若未 cleanup，`tokio::spawn` 补发一条并打 warn 日志
  3. **兜底**：远端临时索引统一放 `~/.noxcode/tmp-index/`，应用启动时和每次会话结束时扫一遍删除 1 小时前的残留。即便前两条都漏了也不会永久堆积

### 8.1.5 并发：per-repo 串行锁

`.git/index.lock` 是排他的。两个 B/C 类操作并发 → `Unable to create index.lock`。

```rust
// key 用 rev-parse --absolute-git-dir 的结果，保证 worktree/软链等价路径归一到同一把锁
static REPO_LOCKS: Lazy<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> = ...;
```

- A 类只读**不加锁**（加了 `--no-optional-locks` 后本来就不写 index，让 status 能和写操作并行）
- B / C 类进锁

### 8.1.6 自动化验证（比"status 输出不变"严格得多）

出口标准第 2 条落成可跑的测试。**判据是 `.git/index` 的 sha256 字节级不变**——因为 status 输出相同不代表 index 没被刷新过 stat 缓存。

实测已跑通的完整用例：

```bash
# 构造：用户暂存了 b.txt，AI 改了 a.txt、新建了未跟踪的 c.txt
echo B_user > b.txt && git add b.txt
echo A_ai   > a.txt
echo C_ai   > c.txt
IDX_BEFORE=$(shasum -a 256 .git/index)          # 9ff94e8b422e5955

# AI 打 checkpoint：全程 GIT_INDEX_FILE 隔离
TMPIDX=$(mktemp -u); cp .git/index "$TMPIDX"
export GIT_INDEX_FILE="$TMPIDX"
git add -A
TREE=$(git write-tree)
COMMIT=$(GIT_AUTHOR_NAME=noxcode GIT_AUTHOR_EMAIL=noxcode@local \
         GIT_COMMITTER_NAME=noxcode GIT_COMMITTER_EMAIL=noxcode@local \
         git commit-tree "$TREE" -p HEAD -m "checkpoint#0")
unset GIT_INDEX_FILE; rm -f "$TMPIDX"
git update-ref refs/noxcode/checkpoints/sess1/0 "$COMMIT"

IDX_AFTER=$(shasum -a 256 .git/index)           # 9ff94e8b422e5955
[ "$IDX_BEFORE" = "$IDX_AFTER" ]                # ✅ 实测通过：字节级未变
```

实测输出：
```
--- 用户 status（checkpoint 前后完全一致）---
 M a.txt        ← AI 改的，未暂存
M  b.txt        ← 用户暂存的，纹丝不动
?? c.txt        ← AI 新建的未跟踪文件
checkpoint 内容: a.txt b.txt c.txt      ← 快照抓全了，包括未跟踪文件
```

**顺带实测到的 checkpoint 可见性真相**（我之前在 §1 决策 7 里写错了，已订正）：

| 命令 | 能否看到 checkpoint |
| --- | --- |
| `git log` | ❌ 看不到 |
| `git branch -a` | ❌ 看不到 |
| `git fsck` | ❌ 无悬空报告 |
| **`git log --all`** | ⚠️ **看得到** |
| `git log --all --exclude='refs/noxcode/*'` | ❌ 看不到 |

应对：
1. 首次创建 checkpoint 时，帮用户加一条仓库级配置（可在设置里关）：
   `git config --add log.excludeDecoration 'refs/noxcode/'`
2. checkpoint 的 author/committer 固定为 `noxcode <noxcode@local>`，`git log --all` 里一眼可辨
3. 设置页提供「清除本仓库所有 checkpoint」一键操作

**gc 行为也已实测**：
- 有 ref 指向时，`git gc --prune=now` **不会**回收 checkpoint 对象 ✅
- `git update-ref -d` 删掉 ref 后，`git gc --prune=now` **立即回收，零残留** ✅ → 8.2 的清理路径可靠

---

## 8.2 高危项 B：`restore_git_checkpoint` 的破坏性

### 8.2.1 实测：`git restore --worktree` 有一个致命盲区

```bash
# checkpoint 时刻只有 a.txt、b.txt
# 之后：改 a.txt、删 b.txt、新建 d.txt
$ git restore --source=<cp> --worktree -- .
$ ls
a.txt  b.txt  d.txt
```

| 变化类型 | 回滚结果 |
| --- | --- |
| 被修改的 `a.txt` | ✅ 还原 |
| 被删除的 `b.txt` | ✅ 恢复 |
| **新建的 `d.txt`** | ❌ **仍在** |

**这是最坑的一条**：用户点了「回滚」，以为回到了干净状态，实际 AI 新建的一堆文件全留着。
如果 AI 刚生成了 30 个文件，回滚后这 30 个还在，用户会以为回滚失败。

**`git restore` 只处理"checkpoint 里有的路径"，对"checkpoint 里没有、现在有"的路径完全不管。**

### 8.2.2 回滚必须是四步，不是一步

```
第 1 步  前置校验          ← 任一不通过则拒绝，不进入后续
第 2 步  算影响面 + 预览    ← 用户看到具体文件才能做决定
第 3 步  自动打 pre-restore checkpoint  ← 让回滚本身可回滚
第 4 步  执行（分两半：restore 已跟踪 + 处理新增文件）
```

### 8.2.3 第 1 步：前置校验（硬性拒绝条件）

| 校验 | 命令 | 不通过时 |
| --- | --- | --- |
| checkpoint 对象还在 | `git cat-file -e <oid>^{commit}` | 拒绝，提示"检查点已失效（可能被 `git gc` 清理或 ref 被手工删除）"，并把该行标记为失效 |
| ref 与数据库一致 | `git rev-parse <ref_name>` 结果 == 表里 `commit_oid` | 拒绝，提示数据不一致 |
| 仓库不在中间态 | 检查 `.git/MERGE_HEAD`、`REBASE_HEAD`、`CHERRY_PICK_HEAD`、`BISECT_LOG` 是否存在 | 拒绝，提示"请先完成或中止当前 merge/rebase" |
| 不在 detached HEAD 的意外状态 | `git symbolic-ref -q HEAD` | 仅警告，不拒绝 |

> ZCode bundle 里能看到 `"git hash-object checkpoint verify"` 字样，说明它也做校验，思路一致。

### 8.2.4 第 2 步：影响面预览（三类必须分开展示）

```rust
struct RestoreImpact {
    will_overwrite:   Vec<PathBuf>,  // checkpoint 有 + 现在有 + 内容不同 → 会被覆盖
    will_recreate:    Vec<PathBuf>,  // checkpoint 有 + 现在没有         → 会被重建
    wont_be_touched:  Vec<PathBuf>,  // checkpoint 没有 + 现在有         → 8.2.1 的盲区，必须单列
}
```

算法（都是只读，走 `IndexMode::ReadOnly`）：

```bash
# checkpoint 里的全部路径
git ls-tree -r --name-only -z <cp_oid>
# 当前工作区的全部路径（含未跟踪，排除 gitignore）
git --no-optional-locks ls-files --cached --others --exclude-standard -z
# 差异明细
git diff --name-status --find-renames -z <cp_oid> -- .
```

**UI 必须把第三类单独做成一个可勾选区块**，默认**不勾选**（删文件比留文件危险），文案写清楚：

```
⚠️ 以下 3 个文件是检查点之后新建的，回滚不会自动删除它们：
   ☐ src/generated/a.ts
   ☐ src/generated/b.ts
   ☐ debug.log
   [ ] 同时删除这些文件
```

对已被 gitignore 的文件**永远不删**，即使用户勾了——它们通常是 `node_modules`、构建产物、`.env`。

### 8.2.5 第 3 步：自动 pre-restore checkpoint

```rust
// kind 字段区分来源，UI 上用不同图标，且不计入"AI 检查点"计数
enum CheckpointKind { SessionStart, AfterToolCall, Manual, AutoPreRestore }
```

- 在第 4 步**任何写操作之前**创建，label 固定为 `回滚前自动快照（目标：<target_seq>）`
- 若这一步失败 → **整个回滚中止**，不允许"没有退路地回滚"
- `AutoPreRestore` 类型的 checkpoint 不参与自动清理策略（保留更久）

### 8.2.6 第 4 步：执行

```bash
# 4a. 还原 checkpoint 里有的路径（不加 --staged，不碰 index，不碰 HEAD）
git restore --source=<cp_oid> --worktree -- <paths...>

# 4b. 处理新增文件（仅当用户勾选，且逐个过 gitignore 白名单）
git check-ignore -q <path> || rm <path>
```

**三条硬约束**：

| 约束 | 原因 |
| --- | --- |
| **不加 `--staged`** | 回滚只改工作区。用户的暂存区是他自己的东西，回滚不该动——这是 8.1 原则的延续 |
| **不动 HEAD、不动分支** | checkpoint 是游离提交，回滚不是 `git reset` |
| 分批执行，收集失败清单 | `git restore` 是逐文件的，可能因权限/占用部分失败。失败的路径要明确列给用户，并提示"pre-restore 检查点仍可用" |

**副作用要提前告知用户**：回滚只改工作区不改 index，所以如果用户暂存区里有内容，回滚后 `git status`
会显示"暂存区与工作区不一致"。这是**正确行为**，但 UI 上要有一句解释，否则用户会以为出 bug 了。

### 8.2.7 确认对话框（必须展示影响面，不能只写"确定吗？"）

```
┌─ 回滚到检查点 #3「第 5 轮工具调用后」 ────────────┐
│  2026-09-02 14:26                                │
│                                                  │
│  将覆盖 4 个文件的当前内容                        │
│    src/main.rs  src/lib.rs  README.md  Cargo.toml│
│  将重建 1 个已删除的文件                          │
│    src/utils.rs                                  │
│  ⚠️ 3 个检查点之后新建的文件不会被自动删除         │
│    [ ] 同时删除（.gitignore 中的文件不会被删）    │
│                                                  │
│  ✓ 回滚前会自动创建一个检查点，此操作可撤销        │
│                                                  │
│              [取消]        [确认回滚]             │
└──────────────────────────────────────────────────┘
```

- 「确认回滚」按钮用危险色，**不做默认焦点**（避免回车误触）
- 影响文件超过 20 个时折叠展示，但**总数必须始终可见**

### 8.2.8 审计与可观测

- 每次 restore 写 `activity_logs`：目标 checkpoint、pre-restore checkpoint、影响文件数、删除文件数、失败清单
- restore 期间**冻结该 workspace 的 agent 会话**（不允许 AI 同时在写文件），用 8.1.5 的 per-repo 锁串起来

### 8.2.9 checkpoint 清理策略（防止仓库膨胀）

| 时机 | 动作 |
| --- | --- |
| 会话删除 | 删表行 + `git update-ref -d` 删 ref。实测：删 ref 后 `git gc --prune=now` 零残留 |
| 会话结束 N 天后 | 按设置项（默认 7 天）清理 `AfterToolCall` 类型，保留 `SessionStart` / `Manual` / `AutoPreRestore` |
| 应用启动 | 扫描 `git for-each-ref refs/noxcode/checkpoints`，对照 `git_checkpoints` 表删孤儿 ref |
| 设置页手动 | 「清除本仓库所有检查点」——批量 `update-ref -d`，并提示用户可自行跑 `git gc` 回收空间 |

> 单个 checkpoint 的增量成本很低：tree + commit 对象，未变化的 blob 全部复用已有对象。
> 主要成本在"AI 新建了大文件"的场景，清理策略覆盖得住。

### 8.2.10 验证清单（P2.5 出口标准的展开）

1. checkpoint → 改文件 → 回滚 → 内容还原
2. checkpoint → **删文件** → 回滚 → 文件重建
3. checkpoint → **新建文件** → 回滚 → 文件仍在 + UI 明确提示 + 勾选后能删掉
4. checkpoint → 新建一个 **gitignore 内**的文件 → 勾选删除 → **该文件不被删**
5. 用户暂存区有内容 → 回滚 → `.git/index` sha256 不变
6. 回滚后再回滚（用 pre-restore checkpoint）→ 回到回滚前状态
7. 制造 merge 中间态（`git merge --no-commit`）→ 回滚被拒绝并给出提示
8. 手工 `update-ref -d` 删掉 ref → 回滚被拒绝并标记该检查点失效
9. 文件设为只读制造部分失败 → 失败清单正确列出，pre-restore checkpoint 可用
10. 以上全部在 SSH 工作区重跑一遍
