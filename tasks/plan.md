# P1/P2 浏览器自动化实施计划

状态：已批准；实现按 B01a → B01b → B02 → B03 → B04 → B05 顺序验收。唯一完成状态以 `../ZcodeTodos.md` 的 B01–B05 为准，不因代码存在而提前勾选。

## 已确认基线

- 应用是 Tauri 2.11.5 + React；MCP 会话支持本机与 SSH stdio。改动前设置页 Playwright 预设为 `npx -y @playwright/mcp@latest`，连接测试只在本机握手/列工具；B01a 已开始修正此行为。
- `npm view @playwright/mcp@0.0.82` 显示最低 Node 18，CLI 为 `playwright-mcp`；本机 Node 22.22.2、npm 10.9.7。该版本 CLI 的 `--isolated`、`--output-dir`、`--image-responses` 等参数已实测；独立夹具与后续真实 macOS Tauri Agent 会话均完成页面操作和截图。
- 官方 [Playwright MCP 文档](https://github.com/microsoft/playwright-mcp/blob/main/README.md) 明确默认持久配置目录不能供并发实例共用；`--isolated` 不保存浏览器状态，截图可作为 image 响应；浏览器网络 allowlist 不构成安全边界。`browser_run_code_unsafe` 等价于服务端远程代码执行，默认不暴露/不调用。
- 改动前 MCP `format_tool_result` 会把图片当文字输出，调用只有超时而未接会话取消，且关闭事件会隐藏副窗口；这些风险已在当前代码中分别加入图片附件、取消路径和主窗口判断，仍须按下面的运行时门槛验收。
- 开始实施时 Agent loop 拆分是其他人的未提交改动；在随后的检查中工作树已恢复干净。后续仍以每次编辑前的最新文件布局为准，不覆盖并发变更。

## 执行切片与验收门槛

1. **B01a 固定版本与按需供给。** 先对版本 0.0.82 做本地表单夹具 MCP 握手/核心工具实测；通过后让新预设固定版本、默认不触发 npm 自动安装，给用户明确的安装/升级按钮与 Node、包、浏览器状态。未安装时仍可启动基础应用，已有自定义 MCP 配置不自动覆盖。连接测试读取保存的服务器与选定工作区，在本机或 SSH 真实执行主机握手并列工具；诊断无副作用，安装须用户点击。必须有未安装、旧预设、SSH、离线测试。
2. **B01b 图片与取消。** MCP image 类型按 MIME、base64 与字节数限额转为 `ToolOutput.images`，通过受管附件/产物保存可恢复引用；禁止把 base64 直接交给模型/日志。会话取消和超时能中止活动请求并回收托管进程。固定表单夹具验证导航、DOM、输入、点击、截图、审批拒绝及中断恢复。
3. **B02 随包技能。** 增加低优先级随包技能源和浏览器/GUI 测试技能，覆盖观察、操作、断言、错误恢复；技能只引用实际 MCP 工具，不以 `allowed-tools` 代替权限；页面内容不可信。测试发现、同名覆盖、禁用与依赖缺失。
4. **B03 生命周期。** 每个运行实例独立托管浏览器、配置目录和下载目录。取消、停止、崩溃只清理自身进程与临时配置；用户已有浏览器从不隐式接管/关闭。下载保留并显示宿主，本机与 SSH 路径不得混淆；验证多会话及异常清理。
5. **B04 受限 JS。** 选可设置 CPU/时间/内存上限且默认无宿主 I/O 的独立 JS 内核；脚本只能通过权限桥调用浏览器操作。不得使用 Node vm 伪装安全沙箱。无限循环、取消、跨会话状态与宿主 API 越权测试不能通过则保持禁用，记为阻断。
6. **B05 同页内嵌。** 先逐平台证明 Tauri WebView 能同时接受人类交互与 Agent 对同一页面的 DOM 操作、截图、上传下载，且远程网站无主窗口 IPC 权限。通过的平台才接入应用 UI；未通过的给明确外部浏览器回退。SSH 只在远端执行，不本机镜像。不得用一个独立浏览器控制实例加另一个 WebView 显示来冒充同页。

## 每片检查与完成定义

- 逻辑变更先补失败测试；每次代码改动后运行 `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings` 和 `npm run format:check`；按范围再跑 `cargo test --manifest-path src-tauri/Cargo.toml`、`npm run test:ci`、`npm run lint`、`npm run build`。
- 加依赖时验证 `cargo tree -i aws-lc-rs` 无匹配，保证基础应用只依赖系统 Git。
- B01–B03 须真实 Tauri 会话 + 本机表单夹具通过；SSH 无测试环境时标明未验证。B05 Windows/Linux 无运行环境时不得宣称已支持，保持外部回退。只在证据齐全时更新 `../ZcodeTodos.md`。


## 本轮验收证据与剩余门槛

- 固定版本、离线启动的 Playwright MCP 通过独立确定性夹具：初始化、列工具、导航、DOM 快照、输入、点击、可见 `Alice` 断言和 `image/png` 截图。
- 2026-09-28 的 macOS Tauri 开发版真实 Agent 会话完成同一表单流程；服务级授权得到批准，曾有无关 Context7 权限请求被拒绝，但 **Playwright 权限拒绝尚未验证**。保存的截图在全新应用进程重新打开旧会话后显示为图片。此前前端历史事件解析遗漏 `attachment_id`，失败回归证明并修复了这一缺陷。
- 官方 Playwright 服务器原始列表包含 `browser_run_code_unsafe`。现在 Agent 目录、工具契约和内部名称调用均过滤此项；用户自配普通 MCP 不受影响。在全新 Tauri 进程用已保存配置及本机工作区按“测试”得到“连接成功：24 个工具”，原始列表先前为 25 个。
- 数据库及代码测试覆盖已保存工作区作用域、组件参数、随包技能优先级/禁用、MCP 图片大小/MIME 限制、事件截图引用、取消、隔离输出参数、SSH 环境透传和副窗口关闭行为。受管实例使用隔离配置和独立目录，拒绝附着已有浏览器。此次串行 Rust 1,014 条通过、3 条忽略；前端 767 条通过，Clippy、Rustfmt、ESLint、Prettier、TypeScript 构建通过。
- MCP 0.0.82 的 `--output-dir` 仅覆盖自动命名输出；显式命名文件相对工作区解析。连接摘要标注执行主机，SSH 文件不自动复制本机；MCP 明确返回的图片块存成本机受管附件。请求的稳定版 `playwright@1.63.0` 不是 `@playwright/mcp` 的版本：后者无 1.63.0 发行，当前 0.0.82 的依赖为 1.64.0-alpha。用户未选择降级到其他 alpha 版本，因此未擅改版本。
- B01–B03 仍需真实 Agent 权限拒绝、取消和连接恢复、并发及异常清理；实际 SSH 主机与 Windows/Linux 运行时未测。仅验证 SSH 命令构造不能证明远端浏览器进程树清理。技能发现/覆盖有自动化测试，按技能驱动的真实 Agent 会话未测。
- B04 尚无受资源硬限制的独立 JS 工作进程，也无逐次权限检查的浏览器桥；普通 Node、Node vm 或 MCP 的危险脚本都不能作为替代。未通过无限循环、取消、跨会话状态和宿主 API 越权测试前保持禁用。
- B05 尚未证明 Tauri WebView 与 Playwright 控制的是同一实时页面。现有 WebView 创建和主帧 JS 回调接口不等于截图、可信输入、跨域框架、上传/下载和弹窗可靠可控。macOS、Windows、Linux 均不能据此启用同页嵌入；保留外部浏览器入口，SSH 继续远端执行，不本机镜像。上述缺口未补齐前，B01–B05 主项保持未勾选。
