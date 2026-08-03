# 自定义功能与上游差异

本文档记录本仓库（`imleoo/kiro.rs-admin`）相对上游的自定义功能，供日后合并上游代码、
或排查"这段逻辑是我们自己加的还是上游带来的"时参考。**每次合并上游后应更新本文档**，
尤其是新增/调整了下文列出的功能域时。

## 上游关系

本仓库同时跟踪两个上游，历史上交替合并：

| Remote 名 | 地址 | 关系 |
|---|---|---|
| `upstream-kiro` | https://github.com/ZyphrZero/kiro.rs.git | **真正的上游**，本仓库最初基于它的二次开发（README 中的"长秋佬 kiro-rs-admin"一脉），带 Admin 面板。合并时应优先全量同步。 |
| `upstream-admin` | https://github.com/doitcan-oiu/kiro.rs-admin.git | `upstream-kiro` 的另一个平行 fork，两者共同祖先在 `ad257d2`（v0.7.1 附近）。**不应无脑全量合并**——它有时会选择与本仓库不同的架构路线（见下文"已知架构分歧"），需要逐 commit 甄别取舍。 |

两个 remote 已配置在本地 `.git/config`，可用 `git fetch upstream-kiro` / `git fetch upstream-admin` 拉取最新。

## 已知架构分歧（合并时需要人工决策，不能自动合并）

### 429 / 账号级风控冷却 vs 无退避多端点顺序重试

`upstream-admin`（doitcan-oiu）在其独有提交 `51dc019`/`e00bc27` 中选择了一条不同的重试路线：
彻底移除账号级 429 冷却/锁定，改为同一凭据沿端点链**无退避**顺序重试，靠"跑满预算"兜底。

本仓库保留并持续打磨了自己的方案（见下文"多端点降级链 + 账号级风控冷却"），**更精细**：
区分"账号级风控（响应体含 `suspicious activity`）"与"普通瞬态错误（429/408/5xx）"，前者才冷却
换号，后者先走同凭据多端点降级链。2026-08 合并时评估后**决定不采纳** upstream-admin 这条路线，
只从其提交里摘取了一处无关联的独立 bug 修复（流结束时空参数 `tool_use` 误判为截断失败）。

**合并上游时的检查点**：如果未来 upstream-admin 或 upstream-kiro 再次改动
`token_manager.rs` / `provider.rs` 里的 429 处理逻辑，先确认是否会移除
`account_throttle_failover` / `account_throttle_cooldown_secs` / `throttled_until` /
`fallback_chain()` 这套机制，不要不假思索接受。

### OpenAI 兼容层：两条平行实现路径（曾产生孤儿文件）

本仓库很早就有自己的 OpenAI 兼容层，路径是顶层模块 **`src/openai/{mod,handlers,types}.rs`**
（`anthropic/router.rs` 里 `/v1/chat/completions`、`/v1/responses` 等路由实际调用的就是这套）。

`upstream-kiro` 后来也做了一套等价功能，但路径是 **`src/anthropic/openai.rs` +
`src/anthropic/responses.rs`**（挂在 `anthropic` 子模块下）。因为路径不同，2026-08 合并
upstream-kiro 时 Git 三方合并**没有检测到冲突**，两套代码同时落地；解决
`src/anthropic/mod.rs` 冲突时没有声明 `mod openai;`/`mod responses;`，
upstream 那套变成了完全未被引用、编译器也不会警告的**孤儿文件**（未在任何 `mod` 树中，
不产生 dead_code 警告），已在合并后清理删除。

**合并上游时的检查点**：如果 upstream 再新增/修改 `src/anthropic/openai.rs` 或
`src/anthropic/responses.rs`，注意这与本仓库的 `src/openai/*` 是同一能力的重复实现，
不要机械保留两份——功能应并入 `src/openai/*`，upstream 路径的文件应删除或不注册 `mod`。
且**合并后要检查一遍新增文件是否都被正确 `mod` 声明引用**（`cargo build` 对孤儿文件完全
沉默，不会报任何警告，必须手动核对，参考本次合并 commit `2480c14`）。

### 模型 ID 的 thinking 后缀规范化（合并时曾静默丢失）

`map_model`（`src/anthropic/converter.rs`）合并 upstream-kiro 时从"字符串包含匹配"改写为
"结构化 family+version 解析"（`normalize_claude_model`），过程中丢了两处 thinking 变体
后缀（`-thinking` / `.thinking`）的处理，且对应测试
`test_map_model_native_kiro_models_passthrough` 被一并删除，`cargo test` 因此没有报错：

1. `normalize_claude_model` 内部的后缀剥离循环原来只处理 `-thinking`，遗漏了本仓库一直
   支持的 `.thinking` 点号形式——`claude-opus-4.6.thinking` 这类输入会直接返回 `None`，
   带着未规范化的原始 ID 传给上游。
2. `map_model` 的兜底透传分支（非 Claude 原生模型，如 deepseek/minimax/glm/qwen）完全不
   剥离 thinking 后缀——`deepseek-3.2-thinking` 会被原样传给 Kiro，产生无效模型 ID。

已于 2026-08 合并同一批次修复（commit `b39562c`），并恢复/补充了对应测试。**这是"合并冲突
解决时不知不觉丢掉一个 fallback/规范化分支，编译和已有测试都不会报错"的第二个真实案例**
（第一个见下文 `get_context_window_size`）——本次是被更高强度（`model_reasoning_effort=high`）
的 `codex review` 抓到的，第一次用默认 low effort 跑同一批 diff 时没发现。**合并后如果只用
默认低强度跑一次 review 就认为万事大吉，是不够的**；涉及大范围手工合并冲突时，应该针对
高风险文件（尤其是这种"从字符串匹配重写成结构化解析"的函数）单独提高 review 强度或让人工
逐条对照旧实现的分支覆盖面。

## 自定义功能域

以下按功能域列出相对 `upstream-kiro/master`（原始上游）的主要差异，涉及的关键文件供合并冲突时定位。

### 1. 企业 SSO / 多种登录方式

- 企业 SSO（Microsoft Entra ID / Azure AD）全流程登录：无需本机监听回调端口，可网页获取
  第二段 IdP 登录链接，或手动粘贴 Kiro 中间链接生成登录链接。
- 关键文件：`src/kiro/auth/social.rs`（认证流程）、`src/kiro/model/credentials.rs`
  （`external_idp` 相关字段：`token_endpoint`/`scopes`/`issuer` 等）、
  `admin-ui/src/components/social-login-dialog.tsx`、`relogin-dialog.tsx`。

### 2. 凭据导入导出增强

- 兼容 Kiro Account Manager 1.1.2/1.8.3、Kiro-Go、CLiProxyAPIPlus 等导出格式；
  导入时自动归一化 `external_idp` 字段，从 JWT 补全邮箱/scopes/issuer/token endpoint。
- 批量导入与 Account Manager 导入入口已合并进同一个对话框，支持导入时统一配置代理和 RPM；
  支持导出为 Account Manager 嵌套格式或通用 JSON。
- 支持纯文本「每行一个 `ksk_` API Key」粘贴批量导入（JSON 解析失败且非 JSON 时的回退路径）。
- API Key 凭据支持区域自动探测（`detect_api_key_region`，`token_manager.rs`
  里 `api_key_region_candidates` / `probe_api_key_usage_limits_one_region`），
  添加/单条导入/批量导入/粘贴导入均覆盖；导出支持 API Key（`ExportedCredentials.kiro_api_key`
  + `authMethod=api_key`，不再因缺 `refreshToken` 被跳过）。
- 关键文件：`src/admin/service.rs`（`credential_to_export_account` 等）、
  `src/admin/types.rs`、`src/kiro/token_manager.rs`、
  `admin-ui/src/components/batch-import-dialog.tsx`（已删除的旧
  `kam-import-dialog.tsx` 功能已并入此文件）。

### 3. 多端点降级链 + 账号级风控冷却（重试体系核心）

这是本仓库投入最多、迭代最多轮的部分,**合并上游 429/重试相关改动时优先复查这里**。

- **多端点降级链**（换桶不换号）：同一凭据沿 `fallback_chain()` 在 IDE 协议族
  （`ide`/`runtime`/`codewhisperer`/`amazonq`，见 `src/kiro/endpoint/*.rs`）和
  CLI 协议族（`cli`/`runtime_cli`）内部依次重试，命中即返回；不计入 attempt、不退避。
  必须在"账号级风控"分类**之前**执行（否则账号级 429 会抢先冷却换号，永远轮不到备用桶）。
- **账号级风控识别与冷却**：精确匹配响应体 `suspicious activity` 文案（而非仅凭状态码），
  由 `account_throttle_failover` 开关控制是否冷却当前凭据
  `account_throttle_cooldown_secs` 秒（默认 1800）并换号。
- **可配置 429 重试策略**：`retry_mode`（`failover`/`turbo`/`fast`/`balanced`/`steady`/
  `polite`/`custom`，见 `RetryMode`）+ `retry_policy`（`custom` 模式下的自定义参数），
  可在 Admin 面板配置；合并了 GreyGunG/Kiro-RS-Tool 的策略设计。
- **账号自愈**（此项本身来自 upstream-kiro 的 `self_heal_*` 系统，非本仓库独有，
  仅记录于此便于区分）：403 封号检测（`suspended_detection_enabled`）与自愈节流
  （`self_heal_enabled`/`self_heal_min_interval_secs`/`self_heal_max_consecutive_rounds`）。
- 关键文件：`src/kiro/provider.rs`（重试主循环）、`src/kiro/token_manager.rs`
  （凭据选择、冷却状态）、`src/model/config.rs`（上述配置项）、
  `src/kiro/endpoint/{amazonq,codewhisperer,runtime,runtime_cli}.rs`（本仓库自建，
  upstream-kiro 原生只有 `ide`/`cli`）。

### 4. 代理池与负载均衡

- 全局和单凭据均可配置多个代理；三种代理负载策略：粘性会话 / 轮询 / 最小负载
  （`proxy_balancing_mode`）。代理失效时自动切换、自动停用、直连兜底。
- 凭据负载均衡模式：`priority` / `balanced` / `least_conn`（`load_balancing_mode`）。
- 关键文件：`src/admin/proxy_pool.rs`、`src/kiro/token_manager.rs`（选凭据逻辑）、
  `admin-ui/src/components/proxy-pool-dialog.tsx`。

### 5. 自定义模型映射

- 支持在 Admin 面板配置自定义模型别名 → 后端模型 ID 映射（含 `context_window`/
  `max_tokens`/`supports_reasoning`/`owned_by` 等覆盖字段），优先于内置映射表。
- 关键文件：`src/model/custom_models.rs`、`src/admin/model_mapping.rs`（CRUD API）、
  `admin-ui/src/components/model-mappings-dialog.tsx`、
  `admin-ui/src/hooks/use-model-mappings.ts`。
- 动态模型兼容：从 Kiro 拉取账号可用模型列表，对 `auto`/DeepSeek/MiniMax/GLM/Qwen 等
  Kiro 原生模型族做透传兼容（`src/anthropic/converter.rs` 的 `map_model`/
  `get_context_window_size`，含"未来版本号动态推断"fallback，见上文"已知架构分歧"
  相关的合并教训）。

### 6. Trace 请求日志与可观测性

- 记录每次请求的完整重试链路（凭据、端点、HTTP 状态、失败分类、耗时），SQLite 持久化。
- 各处理阶段耗时打点（convert/serialize/acquire/profile_arn/execute/metering/decode），
  链路详情逐步展示。
- 一键清空所有请求日志（`DELETE /api/admin/traces`）；按凭据聚合失败次数统计
  （鉴权/账号风控/其他三类）。
- 关键文件：`src/admin/trace_db.rs`、`src/admin/handlers.rs`（`clear_traces`/
  `trace_failure_stats`）、`admin-ui/src/components/trace-log-page.tsx`。

### 7. 凭据管理页增强

- 隐私模式：默认隐藏邮箱完整信息。
- 凭据测试响应：默认用 `claude-sonnet-4-6` 发送 `hello`，也可先拉取模型列表后按模型
  单选/多选批量测试，展示可读的响应片段和耗时。
- 企业凭据余额与模型查询（历史 bug 修复，见 commit `ccdcffd`）。
- 关键文件：`src/admin/service.rs`、`admin-ui/src/components/credential-card.tsx`、
  `admin-ui/src/components/credential-response-test-dialog.tsx`。

### 8. Claude Code / 工具调用兼容加固

- 内置工具双向字段映射；流式工具参数完整收到后才输出，减少 `Invalid tool parameters`
  和半截 JSON 导致的中断。
- 过滤 Kiro 工具 XML 泄漏（`<invoke>` 文本误入正文）；官方 `thinking` 参数与
  `thinking` block 转换合并处理。
- 流干净结束时，空参数（0 字节）的 `tool_use` 缓冲不再误判为截断失败，而是按空参数
  `{}` 补发为完整调用（2026-08 从 upstream-admin 摘取的独立修复，commit `d60d21e`）。
- 关键文件：`src/anthropic/stream.rs`（`ToolJsonAccumulator`）、
  `src/anthropic/handlers.rs`、`src/anthropic/websearch_loop.rs`
  （web_search 内部 agentic loop）。

### 9. 上游限流全链路传播

- 上游 429 的 `Retry-After` 等限流信息完整传播给客户端，而不是在中间层被吞掉或改写
  （历史修复见 commit `326a2c7`）。
- 关键文件：`src/kiro/provider.rs`、`src/anthropic/handlers.rs`。

## 维护建议

- 合并上游前先跑一遍本文档，确认待合并的上游 commit 是否触及上述任一功能域；
  触及时把该 commit 的意图和本仓库现状对照着看，而不是直接接受上游改动。
- 合并完成后，除了 `cargo build`/`cargo test`，建议额外检查：
  1. 是否有新文件未被对应的 `mod.rs` 正确声明引用（孤儿文件，编译器不会提示）；
  2. 涉及 fallback / 未来版本号推断 一类"默认分支兜底逻辑"的函数是否被合并整体替换掉
     （这类回归编译器也发现不了，只能靠测试或人工比对）；
  3. 更新本文档，补充/调整对应功能域的描述。
