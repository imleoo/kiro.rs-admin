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

## 合并进度追踪

### 2026-09-02：upstream-kiro 362d543..v0.8.0（45 commits）—— 仅合并 3 个独立 bug 修复，其余搁置

`upstream-admin` 无新提交（最新仍是 2026-07-22 的 `e00bc27`，均已在此前记录/评估）。
`upstream-kiro` 45 个新提交与本仓库 4 个自定义功能域产生真实架构重叠，规模过大不适合
一次性全量合并，按用户决策分阶段处理，**本次只 cherry-pick 了 3 个真正独立、无重叠的
后端 bug 修复**：

- `f45bf05`（对应上游 `20161ae`）：`getUsageLimits`/`ListAvailableModels` 补发真实
  `profileArn`，修复 Enterprise/IdC 账号 403。同步修了本仓库自有的 API Key 区域自动探测
  （`probe_api_key_usage_limits_one_region`）调用点，跟随 `usage_limits_url` 签名变化。
- `17c002b`（对应上游 `9140b11`）：`setUserPreference` 同样补发真实 `profileArn`。
- `3bbdd0d`（对应上游 `e3866d9`）：空字符串 `proxy_url`/`proxyUrl` 视为未配置代理，回退到
  全局代理，而不是启动失败。

以下 4 类改动**故意搁置未合并**，需要单独开分支评估，合并时先读对应章节：

1. **凭据选择"严格按优先级"重写**（`b0d3926`）：重写了 `acquire_context` 的 priority
   模式选择逻辑（不再复用 `current_id` 快速路径，改为每次请求重新按 `(priority, id)` 排序
   选择），同时改了多处 `min_by_key(|e| e.credentials.priority)` 为
   `(priority, id)` 兜底破同分。这段代码与本仓库 RPM 限流、`least_conn` 负载均衡模式、
   自愈（self-heal）逻辑深度纠缠（同一批函数），不是可以孤立摘取的小修复，需要人工对照
   现有 `select_next_credential`/`try_self_heal` 实现评估行为差异后再决定是否采纳。
2. **自定义模型管理新实现**（`8e54496` 等）：upstream 新增了一套运行时可热更新的模型
   注册表 + Admin API（`GET/PUT /api/admin/config/custom-models`）+ 设置页「模型」分区，
   与本仓库既有的"自定义模型映射"（见下文第 5 节，`src/model/custom_models.rs` +
   `src/admin/model_mapping.rs` + `model-mappings-dialog.tsx`）是同一能力的重复实现。需要
   评估是否合并为一套，而非并存两个模型别名管理入口。
3. **代理治理增强**：`7b911c8`（全局代理独立用户名密码）、`8e54496` 中的"专属代理故障
   自动切换"、`c86ce81`（proxy+model 设置加固），与本仓库代理池（负载策略/自动停用/
   直连兜底）部分重叠，需要对照评估避免功能重复或行为冲突。
4. **凭据元数据 schema + admin-ui 大改版**（约 19 个 commit：`8e8bae0`…`18d694c`，另加
   `c4e2919`/`def8929`/`64af2af`/`482946f` 等控制台改版提交）：upstream 新增了一套面向
   凭据转售场景的元数据体系（`type`/`saleStatus`/`salePrice`），以及配套的凭据卡片重设计、
   Metadata 设置页、多主题选择器等。这套业务语义（卖号）是否适用本部署待确认；且改版会与
   本仓库已深度定制的 `credential-card.tsx`（隐私模式）、`batch-import-dialog.tsx`（纯文本
   批量导入）等产生大量冲突，需要专门评估。
5. **`/v1/responses` 流式与 Codex 兼容性重写**（`de53acc`，单 commit 2776 行）：大幅重写了
   `src/anthropic/responses.rs`/`handlers.rs`/`stream.rs`/`websearch_loop.rs`，改善流式增量
   转换、WebSearch 保活、取消传播、中断后 usage/trace 结算。本仓库该能力落在
   `src/openai/*`（见下文"OpenAI 兼容层"架构分歧），**不能直接拿文件**，需要把
   upstream 这次修的具体 bug 逐条对照移植到 `src/openai/handlers.rs`。搁置原因是工作量大
   （移植 + 验证成本高），不是内容不重要——如果用户反馈 `/v1/responses` 或 Codex CLI
   连接不稳定，应优先翻这个 commit。

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

**2026-08-18 复现一次**：合并 upstream-kiro v0.7.6（PR #64，修复 GPT-5.6 effort 与
OpenAI 会话缓存）时，`git merge` 对这两个文件报了 `modify/delete` 冲突（本仓库已删除，
upstream 又改了），这次冲突标记本身就提示了问题，`git rm` 二者即可，未再变成静默孤儿文件。
但该 PR 里的实际功能改动（`resolve_session_metadata`：从 OpenAI 请求体 `prompt_cache_key`
或 `x-session-affinity`/`x-client-request-id`/`session_id` 请求头推导会话 UUID，写入 Kiro
`metadata.user_id` 提升上游 prompt cache 命中率）**必须手工移植**到
`src/openai/{types,handlers}.rs`——已在本次合并中完成：`ChatCompletionRequest`/
`ResponsesRequest` 新增 `prompt_cache_key` 字段，`chat_to_anthropic` 新增 `metadata`
参数（原先硬编码 `None`，即本仓库 OpenAI 兼容层此前从未支持会话亲和/缓存命中优化），
`post_chat_completions`/`post_responses` 新增 `resolve_session_metadata` 调用。

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

### RPM 限流：per-account 可配置 vs 全局单一开关

本仓库在更早的 `merge/upstream-2026-08-03` 合并里，已经独立实现了一套账号级 RPM（每分钟
请求数）主动限流：`KiroCredentials.rpm_limit` 是**每个凭据自己的字段**（随凭据编辑/导入
设置，`0` = 不限速），调度侧用 `is_rpm_exceeded`/`rpm_window_count`（`token_manager.rs`
自由函数）判断，`record_request` 由 `provider.rs` 在真正发起上游请求时调用。这套设计当时
**没有被记录进本文档**，导致 2026-08 合并 upstream-kiro 时才发现是一处未文档化的架构分歧。

`upstream-kiro` 后来（commit `baa60e9`）也做了等价功能，但形状不同：`account_rpm_limit_enabled`/
`account_rpm_limit` 是**全局单一开关 + 单一上限**，对所有账号一视同仁，且额外走 Admin API
（`GET|PUT /api/admin/config/account-rpm-limit`）+ 顶栏独立 UI 卡片。2026-08 合并时评估后
**决定不采纳**上游这条全局路线——本仓库的"每账号可配置"设计严格更灵活（能模拟全局限速，
也能单独放开某个账号），upstream 的全局开关是它的子集。合并时把 `token_manager.rs`（12 处
冲突）、`model/config.rs`、`admin/{handlers,router,service,types}.rs`、
`topbar-tools.tsx`、`use-credentials.ts`、`admin-ui/src/api/credentials.ts` 里 upstream
一侧的 RPM 改动全部丢弃，只保留本地实现；upstream 在 `topbar-tools.tsx` 里新增的
`AccountRpmLimitButton`/`AccountRpmLimitPanel`/`AccountRpmLimitCompactItems` 等组件因为
只在被丢弃的这条路径里被引用，作为孤儿代码一并删除。

upstream 同批次的后续提交 `da4829d`（"fix: enforce RPM limits and preserve failed
deletions"）修了它自己那套全局实现里的一个真实竞态 bug：`rpm_exceeded` 检查和
`record_request` 记账不是原子的，并发请求可能一起穿过检查后一起记账导致短时超过上限；
修法是把 `record_request` 改成返回 `bool`、在拿到 token 成功后在同一把锁内原子重检，
失败则重新选号；额度耗尽时返回带 `Retry-After` 的类型化 429，而不是裸 `bail!`。

**本仓库的 per-account 实现目前存在同一类问题**（`is_rpm_exceeded` 检查与
`provider.rs` 里的 `record_request` 调用点是分离的两步，非原子），且全账号 RPM 耗尽时
的错误路径未确认是否已是类型化 429 + `Retry-After`。这不是这次合并引入的回归（本地设计
一直如此），不在"合并上游"范围内顺手改掉，但记在这里作为后续独立待办：
1. 评估 `is_rpm_exceeded` 检查 + `record_request` 记账之间的 TOCTOU 窗口是否需要收紧
   为原子操作（可参考 upstream `da4829d` 的思路，但要适配本地 per-credential 而非全局的
   数据结构）。
2. 确认全账号 RPM 耗尽时返回的错误是否已经是类型化限流错误（`UpstreamRateLimitError` 或
   等价类型）并带 `Retry-After`，而不是被吞成普通 5xx/无 Retry-After 的 429。

**合并上游时的检查点**：如果未来 upstream-kiro 再次改动 `account_rpm_limit_enabled`/
`account_rpm_limit`/`rpm_window`/`rpm_exceeded` 这套全局 RPM 逻辑，先确认本地
`rpm_limit`/`is_rpm_exceeded`/`rpm_window_count`/`recent_requests` 这套 per-account
逻辑是否已被顺手替换掉——不要不假思索接受全局开关这条路线。

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
- 支持纯文本「每行一个 `ksk_` API Key」粘贴批量导入（JSON 解析失败且非 JSON 时的回退路径，
  `parseImportInput`/`parsePlainTextApiKeys`）。
- API Key 凭据支持区域自动探测（`detect_api_key_region`，`token_manager.rs` 里
  `api_key_region_candidates` / `probe_api_key_usage_limits_one_region`，依次探测
  `us-east-1`/`eu-central-1` 的 `getUsageLimits`，首个 200 即命中），添加/单条导入/
  批量导入/粘贴导入均在 `add_credential_inner` 里统一覆盖（仅当未显式指定任何区域时
  触发，探测失败按 401/402/403 判定为致命错误并拒绝添加，其余状态回退默认区域）；
  导出支持 API Key（`ExportedCredentials.kiro_api_key` + `authMethod=api_key`，
  不再因缺 `refreshToken` 被跳过）。
- 2026-08-18 更正记录：以上区域自动探测 + 导出两点此前曾被错误记录为"已实现"，实测
  代码中当时并不存在（只存在于 `upstream-admin` 未合并的提交 `73a1987`）；本次已按
  该提交手工移植落地（`src/admin/service.rs`/`types.rs`、`src/kiro/token_manager.rs`、
  `admin-ui/src/components/batch-import-dialog.tsx` 的纯文本回退），未采用其
  `kam-import-dialog.tsx` 改动（该文件在本仓库已删除，功能并入 `batch-import-dialog.tsx`）。
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
- 2026-08-18 合并 upstream-kiro PR #66 起，`KiroProvider::call_mcp_with_retry`
  （web_search 内部 MCP 调用，`src/kiro/provider.rs`）补齐了与 `call_api_with_retry`
  同等粒度的逐 attempt `emit_attempt` 记录；此前混合工具调用（web_search + 其它 tool）
  请求里的 MCP 子调用不出现在 trace 链路中。合并时保留了本仓库
  `call_mcp_with_retry` 自身的代理池故障转移（`execute_mcp_request_with_proxy_failover`）
  和可配置 `retry_mode`/`retry_policy`（`effective_retry_policy`），仅叠加 trace 埋点，
  未采用 upstream 的固定重试次数公式（`(总凭据数 × 每凭据重试数).min(总上限)`）。

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

### 10. PDF / 文档附件支持

- 支持 Anthropic `document` content block（目前仅 `application/pdf`），转换为 Kiro 协议的
  `conversationState.currentMessage.userInputMessage.documents` 字段，原始文件字节透传，
  不做本地解析——协议字段是抓包官方 Kiro IDE v1.0.337 实测得来（详见
  `docs/PDF_DOCUMENT_SUPPORT.md`），上游 kiro.rs / doitcan-oiu fork 均无此能力。
- 只在 IDE 系端点（`ide`/`runtime`/`codewhisperer`/`amazonq`，origin=`AI_EDITOR`）发送；
  CLI 系端点（`cli`/`runtime_cli`，origin=`KIRO_CLI`）会在 `set_origin_kiro_cli` 里剥离
  `documents` 并回落成文本提示——真实 Kiro CLI 本身都不支持文档附件，未验证过该字段在
  CLI 协议下是否被后端接受，保守起见不发。
- `documents` 字段只在 currentMessage 出现，历史消息（`history[]`）不携带——同一份文档不会
  随对话轮次不断重复膨胀；不支持的文档类型 / 超过 45MB base64 上限的文档会退化为文本占位提示，
  不静默丢弃。tool_result 内嵌的 document block（如"读文件"工具返回 PDF）也会走同一条链路提升到
  顶层 `documents`，不局限于用户消息里的顶层 document block。
- token/cache 计量（`/count_tokens` 本地兜底、内部缓存命中模拟）会感知 CLI 端点会剥离
  `documents` 这件事，按真实会发送的内容估算，不按客户端原始请求里的文档字节估算——但仅限本地
  估算路径；配置了远程 count_tokens API 时该服务收到的是协议转换前的原始请求，天然感知不到任何
  Kiro 协议层面的改写（不止文档剥离，工具名截断/图片压缩/模型映射等都一样），这是"接远程精确计数
  服务"这个既有设计的固有特征。
- 关键文件：`src/anthropic/converter.rs`（`extract_kiro_document`/`get_document_format`/
  `extract_tool_result_content`）、`src/kiro/model/requests/conversation.rs`
  （`KiroDocument`/`KiroDocumentSource`）、`src/kiro/endpoint/cli.rs`（`set_origin_kiro_cli`
  剥离逻辑）、`src/token.rs`（`estimate_document_tokens` 的 PDF 页数扫描、CLI 剥离感知）、
  `src/anthropic/cache_metering.rs`（缓存命中模拟对 document block 的哈希/token 处理）。

## 维护建议

- 合并上游前先跑一遍本文档，确认待合并的上游 commit 是否触及上述任一功能域；
  触及时把该 commit 的意图和本仓库现状对照着看，而不是直接接受上游改动。
- 合并完成后，除了 `cargo build`/`cargo test`，建议额外检查：
  1. 是否有新文件未被对应的 `mod.rs` 正确声明引用（孤儿文件，编译器不会提示）；
  2. 涉及 fallback / 未来版本号推断 一类"默认分支兜底逻辑"的函数是否被合并整体替换掉
     （这类回归编译器也发现不了，只能靠测试或人工比对）；
  3. 更新本文档，补充/调整对应功能域的描述。
