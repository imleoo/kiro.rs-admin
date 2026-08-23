# PDF / 文档附件支持实现方案

现状：客户端发来的 Anthropic `document` content block（PDF 等）在
`src/anthropic/converter.rs:947` 的分发 `match` 里落进 `_ => {}`，**被静默丢弃**，模型看不到附件，
客户端和用户都不会收到任何报错提示。
适用版本：本仓库 `imleoo/kiro.rs-admin` master（2026-08-23）
文档日期：2026-08-23

---

## 一、协议依据（实测，非推测）

方法：本机正版 `Kiro.app` v1.0.337 用 `--proxy-server=127.0.0.1:8888 --ignore-certificate-errors`
（Chromium 层）+ `NODE_EXTRA_CA_CERTS`（扩展宿主是独立 Node 进程，证书链不受前者影响，两者都要加）
重新拉起，挂本地 `mitmdump` 中间人抓包，在聊天面板真实附加 PDF 并三轮追问，抓到 3 次独立请求，
结构完全一致。调试进程与含真实文件内容的抓包文件已在验证后清理，未改动系统信任库/系统代理设置。

| 项 | 实测值 |
|---|---|
| 请求 | `POST https://runtime.us-east-1.kiro.dev/`（**根路径**，非 `/generateAssistantResponse`）+ `x-amz-target: KiroRuntimeService.GenerateAssistantResponse` 头路由 |
| `documents` 字段位置 | `conversationState.currentMessage.userInputMessage.documents`，与 `images` 同级，**不在** `userInputMessageContext` 里 |
| `documents[].format` | `"pdf"`（小写短 token，只验证了这一个值） |
| `documents[].source.bytes` | base64 编码的**原始文件字节**（解码后是 `%PDF-1.7...`），后端不做本地文本提取/OCR |
| `documents[].name` | 文件名，去掉了扩展名 |
| `userInputMessage.origin` | `"AI_EDITOR"`（不是 `KIRO_CLI`） |
| 额外请求头 | `x-amzn-kiro-client-attribution: kiro-ide`（本仓库当前从未发送过，作用未知） |
| 历史消息 | `history[]` 里的 `userInputMessage`/`assistantResponseMessage` 都**没有** `documents` 字段——文档只随"当前这一条" user turn 发送，不进历史 |

```json
"documents": [
  {
    "name": "dzfp-26112000003182441701-20260731105238",
    "format": "pdf",
    "source": { "bytes": "<base64>" }
  }
]
```

**外部研究结论（[kirodotdev/Kiro#6458](https://github.com/kirodotdev/Kiro/issues/6458)、
[kiro.dev/changelog/ide/0-11](https://kiro.dev/changelog/ide/0-11/)）**：

- Kiro CLI（终端工具本身）**不支持** PDF 附件，`@path` 遇二进制文件直接报错，是仍未关闭的 feature request。
- Kiro IDE 自 0.11 起支持，格式为 `PDF, CSV, DOC, DOCX, XLS, XLSX, HTML, TXT, Markdown`，单条消息最多 5 个，官方原话
  "sent to the model as native document blocks"——与实测的"透传原始字节、后端不解析"一致。
- 这 8 种格式与 AWS Bedrock Converse API 的 `document` block 支持格式清单完全吻合，故推断其余 7 种的
  `format` token 分别为 `csv`/`doc`/`docx`/`xls`/`xlsx`/`html`/`txt`/`md`——**这是推断，未逐个抓包验证**。

## 二、风险点（决定"发不发 documents 字段"要看 origin）

| 风险 | 说明 | 应对 |
|---|---|---|
| CLI 协议端点可能不认 `documents` | 真实 Kiro CLI 本身都不支持文档附件，`cli.rs`/`runtime_cli.rs`（origin=`KIRO_CLI`）走这套协议，后端是否按 origin 做门禁未知 | 只在 IDE 系端点（origin=`AI_EDITOR`）发送；CLI 系端点收到 document block 时走本地兜底（见下） |
| `x-amzn-kiro-client-attribution` 头缺失 | 本仓库现有 IDE 系端点都没发这个头，不确定是不是 documents 被接受的必要条件 | 先按现状（不加）实现，若联调发现 documents 被忽略，再补这个头验证 |
| 非 PDF 格式的 `format` token 未验证 | csv/docx 等 7 种格式的具体字符串是推断值 | 首版只落地 `application/pdf`，其余格式返回明确的"暂不支持"提示，不静默丢弃 |

## 三、代码改动清单

### 1. `src/kiro/model/requests/conversation.rs`

新增 `KiroDocument`/`KiroDocumentSource`（结构照抄 `KiroImage`/`KiroImageSource`，多一个 `name`）：

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KiroDocument {
    pub name: String,
    pub format: String,
    pub source: KiroDocumentSource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KiroDocumentSource {
    pub bytes: String,
}
```

`UserInputMessage`（currentMessage 用）加一个字段，仿照现有 `images`：

```rust
#[serde(default, skip_serializing_if = "Vec::is_empty")]
pub documents: Vec<KiroDocument>,
```

并补一个 `with_documents(mut self, documents: Vec<KiroDocument>) -> Self` builder（仿照 `with_images`）。

> **与最初方案的偏差**：最初设想 `UserMessage`（历史消息结构体）也照抄加一份 `documents` 字段
> "为了结构完整"。实现时改变主意——抓包已证实历史消息**从不**携带 `documents`，加一个永远不会被
> 写入的字段属于纯粹的推测性代码（YAGNI），故 `UserMessage` **没有加** `documents` 字段；
> `process_message_content_dedup` 改用 `keep_documents: bool` 参数区分"当前消息"（生成真正的
> `KiroDocument`）与"历史消息"（只留一句占位文本），历史路径彻底不产生文档对象，从类型层面
> 保证不会误发。

### 2. `src/anthropic/converter.rs`（实际实现，含 Codex 一轮审查后的修正）

`process_message_content_dedup` 签名加了 `keep_documents: bool` 参数（当前消息 `true`，历史消息
`false`），返回值多带一路 `Vec<KiroDocument>`。`match block.block_type.as_str()` 新增 `"document"`
分支：`source.type=="text"` 直接拼进正文（不占文档配额）；其余交给 `extract_kiro_document`——
按 `keep_documents`/超过 5 个/`source.type!="base64"`/media_type 不支持/空数据/超 45MB 依次判定，
命中任一条件都返回占位文本而不是静默丢弃，通过校验才 `move`（非 clone）进 `KiroDocument`。
`documents[].name` 会剥离与 `format` 匹配的扩展名，对齐抓包实测的官方行为。

### 3. `src/kiro/endpoint/cli.rs`

IDE 系端点（`ide`/`runtime`/`codewhisperer`/`amazonq`，origin=`AI_EDITOR`）不用改，转换结果直接带
`documents`。CLI 系端点（`cli`/`runtime_cli`，origin=`KIRO_CLI`）在 `set_origin_kiro_cli` 里
用 `serde_json::Value` 精确删除 `conversationState.currentMessage.userInputMessage.documents`
（不是字符串正则，不会误删同名字段），并在 `content` 末尾追加一句提示；`runtime_cli.rs` 复用同一
函数，两个端点一次改动同时生效。

### 4. Codex 一轮独立审查后修正的问题

首版实现（仅按上文方案落地）过审前，用 `codex:codex-rescue` 跑了一轮独立审查（性能/功能/安全/
鲁棒性四维度），发现并修正：

| 严重程度 | 问题 | 修正 |
|---|---|---|
| High | `tracing::debug!` 会把完整 Kiro 请求体（含 PDF 原始字节，可能是发票等敏感内容）明文打进日志，三处调用点（`handlers.rs` x2、`provider.rs` x1） | 新增 `redact_binary_payloads_for_log`，DEBUG 日志前统一脱敏 `images`/`documents` 里的 `source.bytes`（只在 DEBUG 真正开启时才求值，不影响正常路径性能） |
| Medium | `documents[].name` 没有去掉扩展名，与实测协议不符 | 加 `strip_matching_extension`，按 `format` 剥离扩展名 |
| Medium | `source.type=="text"` 的 document block 被误判为"不支持"，正文丢失 | 拆分出独立分支，直接拼进 `content` |
| Medium | 未限制单条消息文档数量，官方 changelog 明确最多 5 个 | 加 `MAX_DOCUMENTS_PER_MESSAGE=5`，第 6 个起返回占位文本 |
| Medium | `extract_kiro_document` 对已经 clone 过一次的 `source.data` 又 clone 一次，PDF 场景下是双倍的大段内存拷贝 | 改成按值接收 `ImageSource`，直接 `move` 进 `KiroDocument`，去掉多余的一次 clone |

**有意保留、未修正的一项**（决策记录，避免下一轮审查重复提出）：

- **不做客户端侧 base64 合法性全量解码校验**。`image_resize.rs` 里 `maybe_shrink_image` 的既有
  哲学是"永不因数据可疑就丢弃，最多打 warning 让上游去判"（函数文档原话："Never panics and never
  drops an image... passes through original"）。为文档单独引入"解码校验失败就拒绝"会既不一致，
  又要为最大 45MB 的 base64 多做一次全量解码（与"减少大段拷贝"的修复直接冲突）。只加了零成本的
  空字符串兜底（`source.data.is_empty()`）。

### 5. Codex 第二轮独立审查后修正的问题（对抗式复核第一轮修复）

第二轮明确裁定第一轮"`ContentBlock` 反序列化失败静默吞掉不算新问题"这条理由不成立——URL/`file_id`
等合法 Anthropic document source 会因为 `ImageSource` 强制要求 `media_type`/`data` 而反序列化失败，
导致本功能自己的"仅支持 base64"占位提示实际不可达，这已经是本功能自身的行为缺陷，不再是纯粹的
既有共用技术债。第二轮发现并修正：

| 严重程度 | 问题 | 修正 |
|---|---|---|
| Medium | URL/`file_id` 等合法 document source 因 `ImageSource.media_type`/`data` 必填而反序列化失败，被 `if let Ok` 静默吞掉，"仅支持 base64" 提示不可达 | `ImageSource` 的 `media_type`/`data` 加 `#[serde(default)]`，缺失时反序列化仍成功（空串），走到已有的"文档暂不支持"占位分支 |
| Medium | Anthropic document block 的可选 `context` 字段（给模型的补充说明）完全没建模，文本型 document 的正文以外部分也没保留 | `ContentBlock` 加 `context: Option<String>`，非空时统一拼一句 `[文档说明: ...]` 到正文，text/base64 两条路径都覆盖 |
| Medium | 本地 `/count_tokens` 只读 block 顶层 `text` 字段，PDF-only 请求返回接近 0~1 token 的严重低估 | `token.rs` 新增 `estimate_document_tokens`（按解码字节数粗估，1 页~100 页封顶），`count_all_tokens_local` 按 `source.type` 分流：text 型走 `count_tokens`，base64 型走新函数 |
| Medium | `cache_metering.rs` 的通用 block 哈希只取 `type/text/thinking`，document block 没有这些字段，任意两份不同 PDF 会哈希成同一个签名，造成假缓存命中/低估 token 分母 | 加专门的 document 分支：哈希纳入 `source.type + media_type + data`，token 估算复用上面的 `estimate_document_tokens`（text 型复用既有 `estimate_tokens`） |
| Low | `documents.len() >= 5` 的数量上限检查在类型/格式/大小校验之前，会把"第 6 个文档本身格式就不支持"误报成"超过 5 个"，掩盖真实原因 | 调整检查顺序：先校验类型/格式/空/超限大小，数量上限放最后，只对"本该被接受"的文档计数 |
| Low | `strip_matching_extension` 对标题恰好等于 `.pdf`（纯扩展名、无主文件名）不会剥离，因为判断条件是 `len() > suffix.len()` 而非 `>=` | 改成 `>=`，剥离后若结果为空字符串会被既有的空值回退逻辑接管，落回默认名 `"document"` |

**第二轮同样明确裁定"不做 base64 全量解码校验"这条理由成立**（与 `image_resize.rs` 既有哲学一致、
且会和"减少大段拷贝"的修复冲突），未改动。

### 6. Codex 第三轮独立审查：确认 3 处第二轮修复自己引入的回归

第三轮专门对抗式复核第二轮的 5 处修复，确认其中 2 处修复本身引入了新问题、1 处是遗漏的连带影响，
均已修正：

| 严重程度 | 问题 | 修正 |
|---|---|---|
| Medium | `ImageSource.media_type`/`data` 加 `#[serde(default)]` 是为了让 document 的 url/file_id source 能优雅降级，但这个字段是 `image`/`document` 两种 block 共用的——一个缺 `data` 的畸形 **image** block 现在也能反序列化成功（`data=""`），并被继续构造成 `bytes=""` 的 `KiroImage` 发给上游，把"静默跳过这张图"的旧行为变成了"整个请求可能因空图片数据被上游拒绝" | `extract_kiro_image` 加 `source.data.is_empty()` 显式拦截，跳过而不生成空字节图片；一个函数改动同时覆盖顶层图片和 tool_result 里嵌套的图片（两处调用同一个函数） |
| Medium | `count_all_tokens_local` 先判断"有没有顶层 `text` 字段"再判断 block type——请求体是未做 schema 校验的原始 JSON，一个 `{"type":"document","text":"",...}` 会先命中空字符串 text 分支，把整份 PDF 的本地 token 估算短路成 ~0，document 分支永远走不到 | 调整判断顺序：先判断 `type=="document"`，命中就不再看顶层 `text`；其余 block 类型维持原有的"有 text 字段就计入"通用逻辑不变 |
| Medium | 第二轮加的 `context`（拼进正文）和 `title`（变成 `KiroDocument.name`）都会改变真实发给 Kiro 的请求内容，但 `cache_metering.rs` 的 document 哈希只喂了 `source.type/media_type/data`，同一份 PDF 换个 title 或 context，会被误判成命中旧缓存前缀；`context` 也没有计入本地 token 估算，与"真的会被发送"的内容量不一致 | 哈希输入补上 `title`/`context`；`context` 非空时按文本估算累加进 token；`count_all_tokens_local` 同步补上 `context` 的计数 |

第三轮同时质疑了 `estimate_document_tokens` 按 base64 长度线性折算 token 数的公式，建议改成明确的
"未知"语义。**当时评估后判断不采纳，但第四轮独立审查重新挑战了这个判断，复核后认为第四轮是对的，
已经改掉**——见下一节。

### 7. Codex 第四轮独立审查：推翻了第三轮里"维持不改"这个判断

第四轮专门复核第三轮的 3 处修复本身（`extract_kiro_image` 空数据检查、`count_all_tokens_local` 判断
顺序、`cache_metering.rs` 哈希补字段），确认均已修正到位，只留了一条非阻塞建议（哈希用裸 `|`
拼接 `title`/`context`/`source.type`/`media_type`/`data` 五段，理论上存在"不同字段组合拼出相同字节
序列"的确定性歧义，但只影响本地会话缓存模拟，不影响真实发出去的请求，暂不处理）。

**真正的分歧在 `estimate_document_tokens` 该不该维持"按字节数线性折算"**——第四轮重新审查后认定这是
阻塞项，并指出了本文档第三轮判断里两个站不住脚的前提：

1. **"数量级正确、不精确"这个自我评估是错的**：一份 3MB 的单页扫描件（图片型 PDF 很常见），按字节数
   公式会被高估上百倍（算出 20 万+ token，封顶 15 万，真实约 1500 token/页）——这不是"近似有误差"，
   是页数这个核心变量被文件大小完全掩盖，字节数和真实 token 成本之间根本没有稳定的比例关系。
2. **"只在没配远程 API 时才用"这个前提本身不成立**：`token.rs::count_all_tokens` 确实优先调远程 API，
   但 `cache_metering.rs::compute_cache_usage` 是纯同步函数（见其签名，没有 `async`/`tokio::` 调用），
   无论有没有配置远程 count_tokens API 都无条件跑本地估算——这条估算的实际影响面比文档第三轮版本
   描述的更大，它会直接进最终返回给客户端的 `usage` 统计和管理面板的用量记账。

复核后认可这两点，第三轮"维持不改"的决定被推翻，实际修正：

| 严重程度 | 问题 | 修正 |
|---|---|---|
| Medium（升级为阻塞项后修复） | `estimate_document_tokens` 按 base64 字节数线性折算，与真实 PDF 页数/token 成本无稳定映射，图片型 PDF 可高估上百倍 | 优先解码 base64、扫描 PDF 页树根对象声明的 `/Count N`（页数，PDF 规范要求页树根节点必须声明），乘以 Anthropic 官方"每页约 1500 token"的量级；多个 `/Count` 命中（增量更新）取最大值；只有解不出页数（加密 PDF、`/Pages` 字典被压缩进 `/ObjStm` 对象流、非法 base64/非 PDF 内容）才退回原来的字节数估算兜底 |

不引入新依赖（PDF 页数扫描是手写的字节级 `/Count` 模式匹配，不是完整 PDF 解析器，仅用已有的
`base64` crate 做一次解码）。用抓包拿到的真实发票 PDF 反查过：其 `/Pages` 对象
（`<</Count 1/Kids [5 0 R]/Type /Pages>>`）确实是明文可扫描的，未被对象流压缩——这类简单办公软件
产出的 PDF 是最常见的场景，新逻辑对它们有效；复杂的加密/流压缩 PDF 会退回旧的字节数兜底，不会比
改动前更差。

### 8. Codex 第五轮独立审查：新写的字节扫描逻辑本身没有 panic/DoS 问题，但暴露了 3 处新的准确性缺口

第五轮专门审查第四轮新写的 `find_pdf_declared_page_count` 手写字节扫描——逐处切片/索引边界、
循环终止条件、base64 解码的复杂度都独立复核过，**没有发现 panic、越界或退化成平方级复杂度的
问题**。但指出了三处新问题，前两处判定为阻塞：

| 严重程度 | 问题 | 处理 |
|---|---|---|
| Medium（已修复） | `find_pdf_declared_page_count` 只裸搜字节流里的 `/Count` 字符串，不验证是不是真的在 PDF 页树字典里——任意二进制数据里恰好出现 `/Count 42` 都会被当成页数采信，且"多命中取最大值"在增量更新场景会采到过期的旧版本页数 | 加两层邻近性启发式：① 必须先在开头一段范围内扫到 `%PDF-` 文件头，不是 PDF 直接放弃；② `/Count` 命中点前后 200 字节窗口内必须同时出现 `/Type` 和 `/Pages`，孤立的裸 `/Count` 不采信。仍然不是真正的 PDF 字典/对象图解析，但把"任意字节流误判"的假阳性面收窄到了"看起来像页树字典附近"这个更可信的范围 |
| Medium（已修复） | `count_all_tokens_local`（`token.rs`）和缓存计量（`cache_metering.rs`）对所有二进制 document block 一律按真实数据估算 token/哈希，没有感知 converter.rs 会把历史消息里的文档、非 PDF 类型、空数据、超 45MB、单条消息第 6 个及以后的文档全部降级成短占位文本——比如一份 40MB 的 `text/csv` document，转换器实际发给 Kiro 的是十几个字的占位提示，但本地估算此前会按最高 15 万 token 计 | 新增 `token::document_would_be_sent_as_real_content`（media_type 必须是 `application/pdf`、数据非空、不超过 45MB）与 `is_current_message`/`accepted_doc_count`（对齐 `MAX_DOCUMENTS_PER_MESSAGE=5`）两道判断，`token.rs`/`cache_metering.rs` 都改成：符合这些条件才按真实内容估算，否则按 `PLACEHOLDER_DOCUMENT_TOKENS`（占位文本量级，10 token）计 |
| Low（未处理，Codex 原话为非阻塞） | `cache_metering.rs` 的 document 哈希喂的是未规范化的原始 `title`/`context`（转换器会 trim + 去扩展名），语义相同的请求可能因为大小写/首尾空白差异产生不同哈希，造成"确定性假 miss"（不是安全问题，只是缓存模拟偶尔少算命中） | 维持现状，成本收益不划算：这条路径只影响管理面板的缓存命中率展示，不影响真实发给 Kiro 的内容 |

**关于"纯 WebSearch 快路径会静默丢弃文档"这条，评估后判定不是本次改动引入的问题，不在这次修复范围内**：
`websearch::extract_search_query`（`websearch.rs:149`）只读第一条消息第一个 content block 的
`text` 字段——这个限制对 **image block 早就是一样的**（同样会被静默忽略，不是文档特有），是"纯
WebSearch 单工具快路径只提取一句查询词、根本不把消息内容发给聊天模型"这个设计本身的固有特征，不是
这次接入文档支持引入的回归。真要让文档在这条路径生效，需要重新设计"纯 WebSearch 请求要不要因为带
了文档就改走完整聊天路径"这个路由规则，影响面覆盖所有"web_search 单工具 + 图片/文档"组合，
超出"接入文档协议支持"这个任务的范围，留作独立技术债，与图片那条一起处理更合适。

第六轮独立复核认同了这个论点，判定 WebSearch 快路径这条非阻塞，与作者结论一致。

### 9. Codex 第六轮独立审查：/Count 启发式仍可被构造性绕过、两处判断条件与 converter.rs 未完全对齐

| 严重程度 | 问题 | 处理 |
|---|---|---|
| Medium（已修复） | `/Count` 邻近性启发式只要求 `/Type`/`/Pages` 分别出现在同一窗口内，可以被"`%PDF-` 头 + 不相关位置塞 `/Type` 和 `/Pages`"这类构造绕过 | 收紧成要求连续子串 `/Type/Pages` 或 `/Type /Pages`（PDF 字典语法里 `/Type` 键的值就是 `/Pages`，两者中间不会插别的 key，真实页树字典恒是这个连续形态）——仍不是完整字典解析，但比"两个词分别在附近"明显更难伪造 |
| Medium（已修复） | `document_would_be_sent_as_real_content` 没检查 `source.type=="base64"`，一个 `source.type=="url"` 但把 `media_type` 伪造成 `"application/pdf"` 的 document block 会被误判成真实文档、误占用数量配额 | 加上 `source_type == "base64"` 判断，与 converter.rs 的 `extract_kiro_document` 判断条件对齐 |
| Medium（已修复） | `token.rs`/`cache_metering.rs` 判断"是不是当前消息"直接用 `messages.len() - 1`，没有复现 converter.rs 的 assistant prefill 截断规则（结尾若是 assistant 消息，真正的当前消息是往前找的最后一条 user 消息）——`[user(PDF), assistant(prefill)]` 这种结尾场景下，真实会被当作当前消息发送的 PDF 会被误判成历史消息，低估其 token | 新增 `token::effective_current_message_index`，复现 converter.rs 同一条判断规则；`count_all_tokens_local` 和 `cache_metering.rs` 的 document 分支都改用它，不再简单取最后一条下标（`cache_metering.rs` 原有的、给所有 block 类型共用的缓存段切分逻辑 `idx != last_idx` 未改动，只用这个新判断驱动 document 是否按真实内容估算，避免把变更面扩大到与文档无关的既有行为） |

第六轮同时指出一条问题：CLI/`runtime_cli` 端点会在真正发出请求前（`cli.rs::set_origin_kiro_cli`）
把 `documents` 剥离替换成占位文本，但 token/cache 计量运行在完全不同的时机和数据源上——原本没有
任何渠道知道这次请求最终走了哪个端点族。当时（第六轮）评估后判定"不在本次改动里解决"，理由是
"需要架构级改动、风险大于收益"；**第七轮独立复核认为这个判断不成立**，指出计量闭包实际调用时机
是在 `provider.call_api[_stream]` **返回之后**（"计量延后到上游返回后再算"本来就是既有设计），
此时 `KiroCallResult` 已经带着真正命中的端点名，只是这个信息此前没有从 `KiroCallResult` 传给计量
闭包——不需要重排请求处理时序，只需要把已经存在的信息接上。复核后认可这个具体方案风险可控，已实现：

### 10. Codex 第七轮独立审查：/Type/Pages 邻近性校验漏判换行/多空格排版、prefill 截断规则未同步到缓存段切分边界，以及 CLI 计量时序问题重新评估

| 严重程度 | 问题 | 处理 |
|---|---|---|
| Medium（已修复） | `/Type/Pages` 邻近性校验只认单个空格或无空格，PDF 词法允许名称对象间用任意空白（含换行/Tab/多空格）分隔，格式规范但排版稀疏的合法 PDF 会被漏判，退回精度更低的字节数估算 | 改成 `contains_type_then_pages`：找到 `/Type` 后跳过任意数量 ASCII 空白，检查是否紧跟 `/Pages`，不再要求字面量子串完全匹配 |
| Medium（已修复） | `effective_current_message_index` 只用来判断"文档是否按真实内容估算"，`cache_metering.rs` 原有的缓存段切分边界（`idx != last_idx`）仍用物理最后一条下标，两个不同的"当前消息"判断并存——`[user(PDF), assistant(prefill)]` 场景下，物理最后一条（assistant）从未被真正发送却仍参与哈希，而真正的当前消息（user）反而被错误提交成"可复用历史前缀" | 统一成一个下标：`extract_segments` 改用 `effective_current_message_index` 截断消息数组（`&req.messages[..=idx]`），被丢弃的尾部完全不参与处理，缓存段切分和文档判断共用同一条规则 |
| Medium（已修复，此前判定"不在本次范围"的决策被推翻） | token/cache 计量的闭包捕获原始 `payload`，无法知道请求最终会不会走会剥离 `documents` 的 CLI 协议族端点，导致 CLI+文档场景下计量数字偏高 | `KiroCallResult` 新增 `endpoint_name` 字段（两处构造点：主端点成功、429 降级到备用端点成功，各自带上真正命中的端点名）；新增 `kiro::endpoint::is_cli_family_endpoint_name` 判断端点名是否属于 `cli`/`runtime_cli`；`token::count_all_tokens_with_transport`/`cache_metering::compute_cache_usage_with_transport` 是原函数的新增变体（多一个 `is_cli_transport` 参数，原函数保留、等价于传 `false`，不用改动 35+ 处既有测试调用点）；`handlers.rs` 里两个 `compute_metering` 闭包定义、三个消费它的 handler 函数、共 6 个调用点全部改成在拿到 `call_result` 后传入真实 `is_cli_transport`，上游整链失败（没有任何端点成功）时保守传 `false`（现状不变，不会更差） |

修复 `token_count_excludes_signature_noise` 这个既有测试时发现一个值得记录的观察：`[user, assistant]`
两条消息收尾，在真实 Anthropic API 语义下就是 assistant prefill 请求（模型被要求接着这段
assistant 文本续写），不是"两条普通对话消息"——这个测试原本恰好用了这个结构，只是因为它早于
prefill 截断逻辑存在而没暴露问题；改成三条消息（`[user, assistant(空), user]`）后语义清晰，也不
再触发 prefill 截断。

### 11. Codex 第八轮独立审查：tool_result 里嵌套的 document 被静默丢弃、PDF 空白字符集不完整

第八轮对第七轮的三处修复逐项复核（`contains_type_then_pages` 边界安全性、`extract_segments` 截断
是否影响普通请求、`KiroCallResult` 的两处构造点、`handlers.rs` 六个调用点的取值时机），四项全部
确认无误。同时对累计 8 轮的完整 diff 做了一次全量走查，发现两个新问题：

| 严重程度 | 问题 | 处理 |
|---|---|---|
| Medium（已修复） | `extract_tool_result_content`（`converter.rs`）只处理 tool_result 内嵌的 `text`/`image` block，没有 `document` 分支——一个"读文件"工具把 PDF 内容作为 `tool_result.content` 里的 document block 返回（Anthropic 官方合法用法），会因为没有匹配分支被完全跳过，PDF 内容静默消失，这正是本次改动最初要解决的那类 bug，只是换了个此前没覆盖到的位置 | 加 `document` 分支，复用已有的 `extract_kiro_document`（同一个 `documents` vec、同样受 `keep_documents`/数量上限约束，与顶层 document block 走一模一样的判断链）；只有 images/documents 而无文本时的占位文案从单一的 `"[image attached]"` 拆成按实际内容区分的三种（`image`/`document`/两者都有），不影响原有 image-only 场景的文案 |
| Low（已修复） | `/Type`→`/Pages` 邻近性校验和 `/Count` 后的空白跳过都只用 Rust `is_ascii_whitespace()`，PDF 规范里 NUL（`0x00`）也是合法空白字符，但不在这个集合里，会漏判 | 加 `is_pdf_whitespace`（`= b == 0 || is_ascii_whitespace()`），两处判断统一改用它 |

**第八轮同时指出一条问题，评估后认为是这套本地估算机制的固有边界，不再继续修**：当管理员配置了
远程 `count_tokens_api_url` 时，`count_all_tokens_with_transport` 会优先把原始（未经 Kiro 协议
转换的）`messages` 发给远程服务计数并直接返回，不会走到本地的 `is_cli_transport` 感知路径——这个
观察是对的，但remote-API 优先这条路径从一开始就只是把客户端原始请求转发给第三方 tokenizer，
对 Kiro 协议转换层做的**任何**改写（不只是 CLI 端点剥离文档，还包括工具名截断、图片压缩重编码、
模型 ID 映射等）都天然不在远程服务的感知范围内——这是"接远程精确计数服务"这个既有设计从一开始
就有的特征，不是这次文档改动引入或能够解决的问题；要修就要么放弃远程精确计数的优势、要么让远程
服务理解 Kiro 协议的所有改写细节，两者都不现实。本地兜底路径（未配置远程 API 或远程调用失败时）
已经正确感知 CLI 剥离，是这条本地估算链路唯一能够合理承诺的范围。

### 12. 最终验收：fable5 独立终验代理复核 8 轮 Codex 审查

8 轮 Codex 对抗审查结束后，用一个完全独立、此前未参与过这项工作的 fable5 子代理重新通读全部代码、
自己跑 `cargo build`/`cargo test`（不采信本文档里的数字），对协议依据、实现质量、以及每一条
"评估后判定不修"的决策逐条复核。结论：**可以交付**，同时指出并已修正两处小问题：

- 文档此前把测试增量写成 59，fable5 自己 stash 回退到改动前重新跑测试算出真实增量是 47（改动前
  671 个，改动后 718 个）——已更正为准确数字。
- `extract_tool_result_content`（tool_result 内嵌 block 的处理）判断顺序是先查顶层 `text` 字段
  再查 `type`，与第三轮在 `token.rs` 修过的同一类"畸形 document block 带杂散 text 字段会绕过提取"
  问题同型，只是这个位置当时没对齐——已修正为 `document` 类型优先判断，并补了对应回归测试
  （`test_tool_result_document_with_stray_text_field_not_bypassed`）。

fable5 独立确认的其余关键结论：抓包协议依据可信（结构与 AWS Bedrock Converse API 的 `DocumentBlock`
公开结构吻合，且文档诚实标注了"实测 vs 推断"的边界，最后一项真实上游联调也如实留空未勾，没有
自欺欺人的迹象）；6 个端点的 `fallback_chain()` 降级链不会跨协议族，即使降级到 CLI 族也会正确剥离
文档并把生效端点名传给计量逻辑，`KiroCallResult.endpoint_name` 这条链路是编译期保证不会漏传的；
遍历新写的字节扫描逐处切片边界，未发现 panic 风险。同时指出 tool_result 内嵌内容完全不参与本地
token 计量是一处既有结构性缺口（图片同样如此，非本次引入），与"远程计数 API 不感知 CLI 剥离"
一起归为本次改动之外的独立技术债。

## 四、验收标准

- [x] `cargo build && cargo test` 通过（719 个测试全绿，比改动前的 671 个净增 48 个）
- [x] Anthropic 客户端发送 `{"type":"document","source":{"type":"base64","media_type":"application/pdf",...}}`，
      走 IDE 系端点（默认路由）时，Kiro 请求体里能看到 `documents` 字段且 `format=="pdf"`
- [x] 同样的请求走 CLI 系端点（`KIRO_CLI` origin）时不产生 `documents` 字段，且给出明确的降级提示文本（不是静默丢弃）
- [x] 非 PDF 的 `document` block（如 `media_type=="text/csv"`、`source.type=="url"`）返回明确的
      "暂不支持"占位文本，不 panic、不裸吞
- [x] 多轮对话场景下，`history[]` 里不携带 `documents`（对齐实测的官方行为，避免历史越滚越大）
- [x] `docs/CUSTOM_FEATURES.md` 补一节记录这是自定义功能域（上游 kiro.rs / doitcan fork 均无此能力）
- [x] 本地 `/count_tokens` 与内部缓存命中估算能正确识别 document block，不再把 PDF 算成 0~1 token
- [x] tool_result 内嵌的 document block（如"读文件"工具返回 PDF）能正确提升到顶层 `documents`，
      不会被静默丢弃
- [ ] 真实联调：用带 PDF 附件的 Anthropic 客户端（如 Claude Code）打一次本仓库代理，确认 Kiro
      后端真的接受 `documents` 字段并返回了基于文档内容的回答（目前只做到"协议按抓包结构拼装
      正确"，还没有对着真实上游账号跑通这最后一环）
