//! Token 计算模块
//!
//! 提供文本 token 数量计算功能。
//!
//! # 计算规则
//! - 非西文字符：每个计 4.5 个字符单位
//! - 西文字符：每个计 1 个字符单位
//! - 4 个字符单位 = 1 token（四舍五入）

use std::sync::OnceLock;

use base64::{Engine, engine::general_purpose::STANDARD as BASE64};

use crate::anthropic::types::{
    CountTokensRequest, CountTokensResponse, Message, SystemMessage, Tool,
};
use crate::http_client::{ProxyConfig, build_client};
use crate::model::config::TlsBackend;

/// Count Tokens API 配置
#[derive(Clone, Default)]
pub struct CountTokensConfig {
    /// 外部 count_tokens API 地址
    pub api_url: Option<String>,
    /// count_tokens API 密钥
    pub api_key: Option<String>,
    /// count_tokens API 认证类型（"x-api-key" 或 "bearer"）
    pub auth_type: String,
    /// 代理配置
    pub proxy: Option<ProxyConfig>,

    pub tls_backend: TlsBackend,
}

/// 全局配置存储
static COUNT_TOKENS_CONFIG: OnceLock<CountTokensConfig> = OnceLock::new();

/// 初始化 count_tokens 配置
///
/// 应在应用启动时调用一次
pub fn init_config(config: CountTokensConfig) {
    let _ = COUNT_TOKENS_CONFIG.set(config);
}

/// 获取配置
fn get_config() -> Option<&'static CountTokensConfig> {
    COUNT_TOKENS_CONFIG.get()
}

/// 判断字符是否为非西文字符
///
/// 西文字符包括：
/// - ASCII 字符 (U+0000..U+007F)
/// - 拉丁字母扩展 (U+0080..U+024F)
/// - 拉丁字母扩展附加 (U+1E00..U+1EFF)
///
/// 返回 true 表示该字符是非西文字符（如中文、日文、韩文、阿拉伯文等）
fn is_non_western_char(c: char) -> bool {
    !matches!(c,
        // 基本 ASCII
        '\u{0000}'..='\u{007F}' |
        // 拉丁字母扩展-A (Latin Extended-A)
        '\u{0080}'..='\u{00FF}' |
        // 拉丁字母扩展-B (Latin Extended-B)
        '\u{0100}'..='\u{024F}' |
        // 拉丁字母扩展附加 (Latin Extended Additional)
        '\u{1E00}'..='\u{1EFF}' |
        // 拉丁字母扩展-C/D/E
        '\u{2C60}'..='\u{2C7F}' |
        '\u{A720}'..='\u{A7FF}' |
        '\u{AB30}'..='\u{AB6F}'
    )
}

/// 计算文本的 token 数量
///
/// # 计算规则
/// - 非西文字符：每个计 4.5 个字符单位
/// - 西文字符：每个计 1 个字符单位
/// - 4 个字符单位 = 1 token（四舍五入）
/// ```
pub fn count_tokens(text: &str) -> u64 {
    // println!("text: {}", text);

    let char_units: f64 = text
        .chars()
        .map(|c| if is_non_western_char(c) { 4.0 } else { 1.0 })
        .sum();

    let tokens = char_units / 4.0;

    let acc_token = if tokens < 100.0 {
        tokens * 1.5
    } else if tokens < 200.0 {
        tokens * 1.3
    } else if tokens < 300.0 {
        tokens * 1.25
    } else if tokens < 800.0 {
        tokens * 1.2
    } else {
        tokens * 1.0
    } as u64;

    // println!("tokens: {}, acc_tokens: {}", tokens, acc_token);
    acc_token
}

const TOKENS_PER_PDF_PAGE: u64 = 1500; // Anthropic 官方给出的 PDF 每页粗略 token 量级
const MAX_PDF_PAGES: u32 = 100; // Bedrock PDF 页数上限

/// 与 `converter.rs::MAX_DOCUMENT_BASE64_LEN` 保持一致——两处各自独立维护，
/// 因为跨模块共享私有常量会引入不必要的耦合，这个数字本身很稳定，改动概率低。
const DOCUMENT_BASE64_LEN_LIMIT: usize = 45_000_000;

/// 与 `converter.rs::MAX_DOCUMENTS_PER_MESSAGE` 保持一致，理由同上。
pub(crate) const MAX_DOCUMENTS_PER_MESSAGE_LOCAL: u32 = 5;

/// 一段典型占位文本（"[文档已省略/暂不支持/已跳过...]"）大致的 token 量级，用于历史消息、
/// 不支持类型、空数据、超限大小这几种会被 converter.rs 降级成短占位文本的场景。
pub(crate) const PLACEHOLDER_DOCUMENT_TOKENS: u64 = 10;

/// 判断一个二进制型 document block 是否会被 converter.rs 真正当作文档内容发送
/// （即 `extract_kiro_document` 会把它塞进 `documents[]`，而不是降级成占位文本）。
///
/// 与 converter.rs 的 `get_document_format`/`extract_kiro_document` 保持同样的判断条件
/// （source.type 必须是 "base64"、media_type 仅 "application/pdf"、数据非空、大小不超过
/// 上限）——两处独立维护，是因为 converter.rs 那边的返回值还带着具体的占位提示文案，
/// 没有必要为了一个布尔判断跨模块抽出公共函数。这里只关心"值不值得按真实内容估算
/// token"这一件事。source_type 检查不能漏：否则一个 `source.type=="url"` 但把
/// `media_type` 伪造成 `"application/pdf"` 的 document block 会被误判成真实文档。
pub(crate) fn document_would_be_sent_as_real_content(
    source_type: &str,
    media_type: &str,
    data: &str,
) -> bool {
    source_type == "base64"
        && media_type == "application/pdf"
        && !data.is_empty()
        && data.len() <= DOCUMENT_BASE64_LEN_LIMIT
}

/// 复现 converter.rs（`convert_request_with_mode`）的 assistant prefill 截断规则，
/// 判断"当前消息"（会被塞进 `currentMessage` 那一条）在原始 `messages` 数组里的下标。
///
/// Claude 4.x 已弃用 assistant prefill，Kiro API 也不支持——如果客户端请求结尾是
/// assistant 消息，converter.rs 会静默丢弃它并截断到最后一条 user 消息为止，那条 user
/// 消息才是真正的"当前消息"，而不是字面意义的"最后一条"。本地 token/cache 估算如果
/// 简单地拿 `messages.len() - 1` 当当前消息下标，遇到这种结尾场景会把真正会被当作
/// 当前消息发送的文档误判成历史消息，反而低估其 token。
pub(crate) fn effective_current_message_index(messages: &[Message]) -> Option<usize> {
    if messages.is_empty() {
        return None;
    }
    if messages.last().is_some_and(|m| m.role != "user") {
        messages.iter().rposition(|m| m.role == "user")
    } else {
        Some(messages.len() - 1)
    }
}

/// PDF 等二进制文档内容的 token 估算。
///
/// 优先解码 base64、扫描 PDF 页树根对象声明的 `/Count N`（页数），乘以每页约 1500 token
/// 换算——这比"完全不解析、直接拿文件字节数除以常数"准得多：字节数和页数/token 成本
/// 没有稳定映射（一份 3MB 的单页扫描件按字节数会被高估上百倍，真实约 1500 token/页；
/// 高压缩率的密集文字 PDF 又可能被字节数公式严重低估），按 PDF 自己声明的页数走能把
/// 估算拉回正确量级。这不是严格的 PDF 语法解析——加密 PDF、把 `/Pages` 字典塞进压缩
/// 对象流（`/ObjStm`，PDF 1.5+ 常见）的场景，明文扫描可能找不到 `/Count`，这时才退回
/// 按字节数粗估的兜底（同样明确知道不准，只保证不再是接近 0 的离谱低估）。
///
/// 这条估算会被 `/v1/messages/count_tokens` 直接返回给客户端，也会无条件参与
/// `cache_metering.rs` 的 usage/cache 记账（该模块是纯本地同步计算，不受是否配置远程
/// count_tokens API 影响）——不是"无关痛痒的内部近似值"，误差量级需要认真对待。
pub fn estimate_document_tokens(data_base64: &str) -> u64 {
    if data_base64.is_empty() {
        return 0;
    }
    if let Ok(decoded) = BASE64.decode(data_base64)
        && let Some(pages) = find_pdf_declared_page_count(&decoded)
    {
        return (pages.min(MAX_PDF_PAGES) as u64) * TOKENS_PER_PDF_PAGE;
    }
    estimate_document_tokens_by_byte_size(data_base64.len())
}

/// 兜底：解不出声明页数时，按（估算的）解码后字节数粗略折算，上下界仍卡在
/// 1~100 页对应的 token 量级，不追求精确，只保证下限不再是接近 0 的离谱低估。
fn estimate_document_tokens_by_byte_size(base64_len: usize) -> u64 {
    const BYTES_PER_TOKEN_ESTIMATE: u64 = 15;
    let decoded_bytes_estimate = base64_len as u64 * 3 / 4;
    (decoded_bytes_estimate / BYTES_PER_TOKEN_ESTIMATE)
        .clamp(TOKENS_PER_PDF_PAGE, TOKENS_PER_PDF_PAGE * MAX_PDF_PAGES as u64)
}

/// PDF 词法定义的空白字符集：空格/Tab/CR/LF/FF，加上 Rust `is_ascii_whitespace()`
/// 不认的 NUL（PDF 规范里 NUL 也是合法空白，`is_ascii_whitespace()` 只覆盖常见的
/// 那几个，两者集合不完全一样）。
fn is_pdf_whitespace(b: u8) -> bool {
    b == 0 || b.is_ascii_whitespace()
}

/// 在字节窗口里找 "/Type" 后面（跳过任意数量的 PDF 空白）紧跟 "/Pages" 的相邻关系。
/// PDF 词法允许名称对象之间用任意空白分隔，不能只认单个空格。
fn contains_type_then_pages(nearby: &[u8]) -> bool {
    const TYPE: &[u8] = b"/Type";
    const PAGES: &[u8] = b"/Pages";
    let mut start = 0usize;
    while start < nearby.len() {
        let Some(rel) = nearby[start..].windows(TYPE.len()).position(|w| w == TYPE) else {
            break;
        };
        let after_type = start + rel + TYPE.len();
        let mut j = after_type;
        while j < nearby.len() && is_pdf_whitespace(nearby[j]) {
            j += 1;
        }
        if nearby[j..].starts_with(PAGES) {
            return true;
        }
        start = after_type;
    }
    false
}

/// 扫描解码后的 PDF 字节，找 `/Type /Pages` 页树字典里的 `/Count N`（PDF 规范要求页树
/// 根节点必须声明总页数）。多个 `/Count` 命中（增量更新/多版本）时取最大值。找不到
/// （加密、对象流压缩、非 PDF 内容、base64 解码后不是合法 PDF）返回 `None`，由调用方
/// 回落到字节数估算。
fn find_pdf_declared_page_count(decoded: &[u8]) -> Option<u32> {
    // 起码要有 PDF 文件头才继续扫描——这不是完整解析，但能挡掉"任意字节流里凑巧出现
    // /Count"这种最粗暴的假阳性（比如一份根本不是 PDF 的二进制数据里偶然出现这几个
    // 字符）。规范允许文件头前有少量垃圾字节，实测只在开头一小段范围内找。
    const PDF_HEADER: &[u8] = b"%PDF-";
    let header_scan_len = decoded.len().min(1024);
    if decoded.len() < PDF_HEADER.len()
        || !decoded[..header_scan_len]
            .windows(PDF_HEADER.len())
            .any(|w| w == PDF_HEADER)
    {
        return None;
    }

    const NEEDLE: &[u8] = b"/Count";
    // PDF 字典的 /Type 键值对里，"/Pages" 就是 /Type 键的值本身，两者中间不会插别的
    // key——真实页树根字典写出来恒是 "/Type" 紧跟 "/Pages"，中间最多隔着 PDF 词法允许的
    // 任意空白（空格/Tab/CR/LF，不止一个），不是"/Type 和 /Pages 分别出现在同一段范围内"
    // 这么松散。要求这个（容忍空白的）相邻关系在 /Count 命中点前后一段窗口内出现，
    // 比"两个词分别在附近"更难被无关字节偶然凑出来（也更难被刻意构造的假 PDF 绕过，
    // 虽然仍不是真正的字典语法解析）。
    const PROXIMITY_WINDOW: usize = 200;

    let mut best: Option<u32> = None;
    let mut offset = 0usize;
    while offset < decoded.len() {
        let Some(rel) = decoded[offset..]
            .windows(NEEDLE.len())
            .position(|w| w == NEEDLE)
        else {
            break;
        };
        let match_start = offset + rel;
        let after_needle = match_start + NEEDLE.len();

        let window_start = match_start.saturating_sub(PROXIMITY_WINDOW);
        let window_end = (after_needle + PROXIMITY_WINDOW).min(decoded.len());
        let nearby = &decoded[window_start..window_end];
        let looks_like_pages_dict = contains_type_then_pages(nearby);

        if looks_like_pages_dict {
            let mut j = after_needle;
            while j < decoded.len() && is_pdf_whitespace(decoded[j]) {
                j += 1;
            }
            let digits_start = j;
            while j < decoded.len() && decoded[j].is_ascii_digit() {
                j += 1;
            }
            if j > digits_start
                && let Ok(s) = std::str::from_utf8(&decoded[digits_start..j])
                && let Ok(n) = s.parse::<u32>()
                && n > 0
                && n <= 100_000 // 防止扫到垃圾数字产生离谱大数
            {
                best = Some(best.map_or(n, |b| b.max(n)));
            }
        }
        offset = after_needle;
    }
    best
}

/// 估算请求的输入 tokens
///
/// 优先调用远程 API，失败时回退到本地计算。
///
/// 全部按引用收参：本函数在首字关键路径上每轮被调用一次，按值收参会导致每轮把
/// 整段会话（messages/system/tools）深拷贝一份，随对话轮数线性放大首字延迟。
pub(crate) fn count_all_tokens(
    model: &str,
    system: Option<&[SystemMessage]>,
    messages: &[Message],
    tools: Option<&[Tool]>,
) -> u64 {
    count_all_tokens_with_transport(model, system, messages, tools, false)
}

/// 与 [`count_all_tokens`] 相同，多一个 `is_cli_transport` 参数：调用方如果已经知道
/// 这次请求最终会不会走 CLI 协议端点（`cli`/`runtime_cli`，发送前会把
/// `documents` 整个剥离替换成占位文本，见 `kiro::endpoint::cli::set_origin_kiro_cli`），
/// 传 `true` 能让本地兜底估算对齐真实会发送的内容，不按从未真正发出去的原始 PDF
/// 字节估算。
///
/// 只有"上游调用已经完成、已知最终成功端点"的场景才应该传 `true`——`/count_tokens`
/// 独立端点、WebSearch 循环里预先算的兜底估算都发生在选定端点之前，天然不知道会走
/// 哪个协议族，继续用 [`count_all_tokens`]（等价于传 `false`，按 IDE 协议假设，也是
/// 目前占绝大多数的真实场景）即可。
pub(crate) fn count_all_tokens_with_transport(
    model: &str,
    system: Option<&[SystemMessage]>,
    messages: &[Message],
    tools: Option<&[Tool]>,
    is_cli_transport: bool,
) -> u64 {
    // 检查是否配置了远程 API
    if let Some(config) = get_config() {
        if let Some(api_url) = &config.api_url {
            // 尝试调用远程 API
            let result = tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(call_remote_count_tokens(
                    api_url, config, model, system, messages, tools,
                ))
            });

            match result {
                Ok(tokens) => {
                    tracing::debug!("远程 count_tokens API 返回: {}", tokens);
                    return tokens;
                }
                Err(e) => {
                    tracing::warn!("远程 count_tokens API 调用失败，回退到本地计算: {}", e);
                }
            }
        }
    }

    // 本地计算
    count_all_tokens_local(system, messages, tools, is_cli_transport)
}

/// 调用远程 count_tokens API
async fn call_remote_count_tokens(
    api_url: &str,
    config: &CountTokensConfig,
    model: &str,
    system: Option<&[SystemMessage]>,
    messages: &[Message],
    tools: Option<&[Tool]>,
) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
    let client = build_client(config.proxy.as_ref(), 300, config.tls_backend)?;

    // 构建请求体（远程 API 需要拥有所有权的请求体，仅此分支付一次 clone；
    // 未配置远程 API 的默认路径不进入此函数，无 clone 开销）。
    let request = CountTokensRequest {
        model: model.to_string(),
        messages: messages.to_vec(),
        system: system.map(|s| s.to_vec()),
        tools: tools.map(|t| t.to_vec()),
    };

    // 构建请求
    let mut req_builder = client.post(api_url);

    // 设置认证头
    if let Some(api_key) = &config.api_key {
        if config.auth_type == "bearer" {
            req_builder = req_builder.header("Authorization", format!("Bearer {}", api_key));
        } else {
            req_builder = req_builder.header("x-api-key", api_key);
        }
    }

    // 发送请求
    let response = req_builder
        .header("Content-Type", "application/json")
        .json(&request)
        .send()
        .await?;

    if !response.status().is_success() {
        return Err(format!("API 返回错误状态: {}", response.status()).into());
    }

    let result: CountTokensResponse = response.json().await?;
    Ok(result.input_tokens as u64)
}

/// 本地计算请求的输入 tokens
fn count_all_tokens_local(
    system: Option<&[SystemMessage]>,
    messages: &[Message],
    tools: Option<&[Tool]>,
    is_cli_transport: bool,
) -> u64 {
    let mut total = 0;

    // 系统消息
    if let Some(system) = system {
        for msg in system {
            total += count_tokens(&msg.text);
        }
    }

    // 用户消息
    let current_idx = effective_current_message_index(messages);
    for (idx, msg) in messages.iter().enumerate() {
        // converter.rs 只把 documents 真正塞进 currentMessage，历史消息里的 document
        // block 会被降级成一句占位文本（见 extract_kiro_document 的 keep_documents 参数）
        // ——本地估算得对齐这个"是不是当前消息"的判断，否则历史里随手带的 PDF 会被重复
        // 按整份文件估算 token，跟真实发送的内容量差好几个数量级。"当前消息"不是简单的
        // "最后一条"：结尾若是 assistant prefill，converter.rs 会截断到最后一条 user
        // 消息为止（见 effective_current_message_index），这里要对齐同一条规则。
        let is_current_message = Some(idx) == current_idx;
        // 与 converter.rs 的 MAX_DOCUMENTS_PER_MESSAGE 对齐：单条消息里第 6 个及以后的
        // 合法文档也会被降级成占位文本
        let mut accepted_doc_count = 0u32;
        if let serde_json::Value::String(s) = &msg.content {
            total += count_tokens(s);
        } else if let serde_json::Value::Array(arr) = &msg.content {
            for item in arr {
                // 先按 block type 判断，"document" 优先于通用的"有没有 text 字段"分支：
                // document block 本身不该有顶层 text 字段，但请求体是未做严格 schema
                // 校验的原始 JSON，一个畸形/构造出的 {"type":"document","text":"",...}
                // 如果落进下面的通用 text 分支，会让整份 PDF 的 token 估算被短路成 ~0。
                if item.get("type").and_then(|v| v.as_str()) == Some("document") {
                    if let Some(source) = item.get("source") {
                        let source_type =
                            source.get("type").and_then(|v| v.as_str()).unwrap_or("");
                        let media_type =
                            source.get("media_type").and_then(|v| v.as_str()).unwrap_or("");
                        let data = source.get("data").and_then(|v| v.as_str()).unwrap_or("");
                        total += if source_type == "text" {
                            // 文本型 document 不占用 documents 配额，无论是否当前消息都会被
                            // converter.rs 原样拼进正文（见 process_message_content_dedup）
                            count_tokens(data)
                        } else if !is_cli_transport
                            && is_current_message
                            && accepted_doc_count < MAX_DOCUMENTS_PER_MESSAGE_LOCAL
                            && document_would_be_sent_as_real_content(
                                source_type,
                                media_type,
                                data,
                            )
                        {
                            accepted_doc_count += 1;
                            estimate_document_tokens(data)
                        } else {
                            // 历史消息里的文档 / 不支持的类型 / 空数据 / 超限大小 / CLI
                            // 协议端点（会在发送前整个剥离 documents，见 cli.rs）：converter.rs
                            // 或端点层最终会把它降级成一句短占位文本，本地估算按占位文本的
                            // 量级算，不能按整份原始数据估算
                            PLACEHOLDER_DOCUMENT_TOKENS
                        };
                    }
                    // context 是会被实际拼进发给模型的正文的说明文字（见 converter.rs 的
                    // "[文档说明: ...]" 拼接逻辑），本地估算也要把它算进去，否则和真实发送
                    // 内容的 token 量不一致
                    if let Some(context) = item.get("context").and_then(|v| v.as_str()) {
                        total += count_tokens(context);
                    }
                } else if let Some(text) = item.get("text").and_then(|v| v.as_str()) {
                    total += count_tokens(text);
                }
            }
        }
    }

    // 工具定义
    if let Some(tools) = tools {
        for tool in tools {
            total += count_tokens(&tool.name);
            total += count_tokens(&tool.description);
            let input_schema_json = serde_json::to_string(&tool.input_schema).unwrap_or_default();
            total += count_tokens(&input_schema_json);
        }
    }

    total.max(1)
}

/// 估算输出 tokens
pub(crate) fn estimate_output_tokens(content: &[serde_json::Value]) -> i32 {
    let mut total = 0;

    for block in content {
        if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
            total += count_tokens(text) as i32;
        }
        if let Some(thinking) = block.get("thinking").and_then(|v| v.as_str()) {
            total += count_tokens(thinking) as i32;
        }
        if block.get("type").and_then(|v| v.as_str()) == Some("redacted_thinking") {
            total += 8;
        }
        if block.get("type").and_then(|v| v.as_str()) == Some("tool_use") {
            // 工具调用开销
            if let Some(input) = block.get("input") {
                let input_str = serde_json::to_string(input).unwrap_or_default();
                total += count_tokens(&input_str) as i32;
            }
        }
    }

    total.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn estimate_document_tokens_empty_is_zero() {
        assert_eq!(estimate_document_tokens(""), 0);
    }

    #[test]
    fn estimate_document_tokens_prefers_declared_page_count() {
        let fake_pdf = b"%PDF-1.7\n1 0 obj<</Type/Pages/Count 42/Kids[]>>endobj\n%%EOF";
        let b64 = BASE64.encode(fake_pdf);
        assert_eq!(estimate_document_tokens(&b64), 42 * TOKENS_PER_PDF_PAGE);
    }

    #[test]
    fn estimate_document_tokens_takes_max_across_multiple_count_hits() {
        // 增量更新/多版本场景可能出现多个 /Count，取最大值更接近真实页数；
        // 每个 /Count 都要在 /Type/Pages 字典的邻近范围内才会被采信
        let fake_pdf = b"%PDF-1.7\n<</Type/Pages/Count 3>>\n<</Type/Pages/Count 10>>\n<</Type/Pages/Count 7>>\n%%EOF";
        let b64 = BASE64.encode(fake_pdf);
        assert_eq!(estimate_document_tokens(&b64), 10 * TOKENS_PER_PDF_PAGE);
    }

    #[test]
    fn estimate_document_tokens_declared_page_count_still_capped_at_max_pages() {
        let fake_pdf = b"%PDF-1.7\n<</Type/Pages/Count 99999>>";
        let b64 = BASE64.encode(fake_pdf);
        assert_eq!(
            estimate_document_tokens(&b64),
            MAX_PDF_PAGES as u64 * TOKENS_PER_PDF_PAGE
        );
    }

    #[test]
    fn estimate_document_tokens_falls_back_when_no_count_found() {
        // 合法 base64，但内容不含 /Count（比如加密 PDF、非 PDF 内容）——应退回字节数估算，
        // 而不是 panic 或者返回 0
        let no_pdf_signal = "A".repeat(20_000);
        let tokens = estimate_document_tokens(&no_pdf_signal);
        assert!(tokens >= TOKENS_PER_PDF_PAGE);
    }

    #[test]
    fn estimate_document_tokens_ignores_count_without_pdf_header() {
        // 没有 %PDF- 文件头：不是 PDF，即使字节流里恰好出现 /Count 也不该被当页数用
        let not_a_pdf = b"<</Type/Pages/Count 42/Kids[]>>endobj";
        let b64 = BASE64.encode(not_a_pdf);
        assert_ne!(
            estimate_document_tokens(&b64),
            42 * TOKENS_PER_PDF_PAGE,
            "没有 PDF 文件头不应该采信 /Count"
        );
    }

    #[test]
    fn estimate_document_tokens_ignores_count_far_from_pages_dict() {
        // 有 PDF 文件头，但 /Count 附近既没有 /Type 也没有 /Pages（离题万里的裸 /Count）
        // ——不应该被误判成页数声明
        let mut fake_pdf = b"%PDF-1.7\n".to_vec();
        fake_pdf.extend(std::iter::repeat_n(b'x', 500));
        fake_pdf.extend_from_slice(b"/Count 42");
        fake_pdf.extend(std::iter::repeat_n(b'x', 500));
        let b64 = BASE64.encode(&fake_pdf);
        assert_ne!(
            estimate_document_tokens(&b64),
            42 * TOKENS_PER_PDF_PAGE,
            "远离 /Type /Pages 的裸 /Count 不应该被采信"
        );
    }

    #[test]
    fn estimate_document_tokens_ignores_count_when_type_and_pages_not_adjacent() {
        // /Type 和 /Pages 都在附近，但不是紧挨着的 "/Type/Pages" / "/Type /Pages"——
        // 这类"两个词分别出现在同一段范围内但没有构成真正的键值对"不该被采信，
        // 否则一份精心构造的假 PDF（塞进不相关的 /Type ... /Pages ... /Count）就能
        // 伪造出任意页数
        let fake_pdf = b"%PDF-1.7\n<</Type /Font/Subtype /Pages/Count 42>>";
        let b64 = BASE64.encode(fake_pdf);
        assert_ne!(
            estimate_document_tokens(&b64),
            42 * TOKENS_PER_PDF_PAGE,
            "不相邻的 /Type ... /Pages 不该被当成页树字典"
        );
    }

    #[test]
    fn estimate_document_tokens_tolerates_whitespace_between_type_and_pages() {
        // PDF 词法允许名称对象之间用任意空白（含换行/多空格）分隔，不能只认单个空格；
        // "格式规范但排版稀疏"的合法 PDF 不应该被误判成找不到页数
        let fake_pdf = b"%PDF-1.7\n1 0 obj\n<<\n  /Type\n  /Pages\n  /Count 42\n>>\nendobj\n%%EOF";
        let b64 = BASE64.encode(fake_pdf);
        assert_eq!(estimate_document_tokens(&b64), 42 * TOKENS_PER_PDF_PAGE);
    }

    #[test]
    fn estimate_document_tokens_tolerates_nul_as_pdf_whitespace() {
        // PDF 规范里 NUL（0x00）也是合法空白字符，不止 Rust is_ascii_whitespace()
        // 认的那几个
        let mut fake_pdf = b"%PDF-1.7\n<</Type".to_vec();
        fake_pdf.push(0u8);
        fake_pdf.extend_from_slice(b"/Pages");
        fake_pdf.push(0u8);
        fake_pdf.extend_from_slice(b"/Count 42>>");
        let b64 = BASE64.encode(&fake_pdf);
        assert_eq!(estimate_document_tokens(&b64), 42 * TOKENS_PER_PDF_PAGE);
    }

    #[test]
    fn document_would_be_sent_as_real_content_rejects_spoofed_media_type() {
        // source.type=="url" 但把 media_type 伪造成 "application/pdf" 不应该被当成
        // 真实文档——converter.rs 只接受 source.type=="base64" 的文档
        assert!(!document_would_be_sent_as_real_content(
            "url",
            "application/pdf",
            "irrelevant",
        ));
        assert!(document_would_be_sent_as_real_content(
            "base64",
            "application/pdf",
            "AAAA",
        ));
    }

    #[test]
    fn effective_current_message_index_skips_trailing_assistant_prefill() {
        let messages = vec![
            Message {
                role: "user".to_string(),
                content: json!("带文档的一条"),
            },
            Message {
                role: "assistant".to_string(),
                content: json!("prefill"),
            },
        ];
        assert_eq!(effective_current_message_index(&messages), Some(0));

        let normal = vec![
            Message {
                role: "user".to_string(),
                content: json!("a"),
            },
            Message {
                role: "assistant".to_string(),
                content: json!("b"),
            },
            Message {
                role: "user".to_string(),
                content: json!("c"),
            },
        ];
        assert_eq!(effective_current_message_index(&normal), Some(2));

        assert_eq!(effective_current_message_index(&[]), None);
    }

    #[test]
    fn count_all_tokens_local_document_before_assistant_prefill_counts_as_current() {
        // 结尾是 assistant prefill 时，真正的"当前消息"是它之前最后一条 user 消息——
        // 那条消息里的文档应该按真实内容估算，不能因为它不是字面意义的"最后一条"而
        // 被误判成历史消息、降级成占位符量级
        let pdf_data = "A".repeat(200_000);
        let messages = vec![
            Message {
                role: "user".to_string(),
                content: json!([
                    {"type": "document", "source": {
                        "type": "base64", "media_type": "application/pdf", "data": pdf_data
                    }}
                ]),
            },
            Message {
                role: "assistant".to_string(),
                content: json!("prefill"),
            },
        ];
        let total = count_all_tokens_local(None, &messages, None, false);
        let full_estimate = estimate_document_tokens(&"A".repeat(200_000));
        assert!(
            total >= full_estimate,
            "assistant prefill 之前的 user 消息里的文档应该按真实内容估算: total={total}, full_estimate={full_estimate}"
        );
    }

    #[test]
    fn estimate_document_tokens_falls_back_on_invalid_base64() {
        let invalid = "not valid base64 !!! ===";
        let tokens = estimate_document_tokens(invalid);
        assert!(tokens >= TOKENS_PER_PDF_PAGE);
    }

    #[test]
    fn estimate_document_tokens_has_min_and_max_bounds() {
        // 极小文档也至少按 1 页估算
        assert_eq!(estimate_document_tokens("QQ=="), 1500);

        // 极大文档不应该无限线性放大，封顶在 100 页
        let huge = "A".repeat(60_000_000);
        assert_eq!(estimate_document_tokens(&huge), 1500 * 100);
    }

    #[test]
    fn estimate_document_tokens_scales_with_size_within_bounds() {
        let small = "A".repeat(20_000);
        let big = "A".repeat(200_000);
        assert!(estimate_document_tokens(&big) > estimate_document_tokens(&small));
    }

    #[test]
    fn count_all_tokens_local_counts_pdf_document_block() {
        let pdf_data = "A".repeat(200_000);
        let messages = vec![Message {
            role: "user".to_string(),
            content: json!([
                {"type": "document", "source": {
                    "type": "base64", "media_type": "application/pdf", "data": pdf_data
                }}
            ]),
        }];
        let total = count_all_tokens_local(None, &messages, None, false);
        assert!(
            total > 1000,
            "PDF-only 请求不应该只算出 0~1 个 token: total={total}"
        );
    }

    #[test]
    fn count_all_tokens_local_cli_transport_uses_placeholder_even_for_current_message() {
        // CLI 协议端点会在发送前把 documents 整个剥离替换成占位文本（见
        // cli.rs::set_origin_kiro_cli）——即使文档就在当前消息里、格式也完全合法，
        // is_cli_transport=true 时也不该按真实内容估算
        let pdf_data = "A".repeat(200_000);
        let messages = vec![Message {
            role: "user".to_string(),
            content: json!([
                {"type": "document", "source": {
                    "type": "base64", "media_type": "application/pdf", "data": pdf_data
                }}
            ]),
        }];
        let ide_total = count_all_tokens_local(None, &messages, None, false);
        let cli_total = count_all_tokens_local(None, &messages, None, true);
        assert!(
            cli_total < ide_total,
            "CLI 协议下文档已被剥离，估算不应该按真实内容算: cli={cli_total}, ide={ide_total}"
        );
    }

    #[test]
    fn count_all_tokens_local_document_block_not_bypassed_by_stray_text_field() {
        // 请求体是未做严格 schema 校验的原始 JSON，一个畸形的
        // {"type":"document","text":"",...} 不应该因为命中通用的"有没有 text 字段"分支
        // 而把整份 PDF 的 token 短路成 ~0
        let pdf_data = "A".repeat(200_000);
        let messages = vec![Message {
            role: "user".to_string(),
            content: json!([
                {"type": "document", "text": "", "source": {
                    "type": "base64", "media_type": "application/pdf", "data": pdf_data
                }}
            ]),
        }];
        let total = count_all_tokens_local(None, &messages, None, false);
        assert!(
            total > 1000,
            "带杂散 text 字段的 document block 不应该绕过文档 token 估算: total={total}"
        );
    }

    #[test]
    fn count_all_tokens_local_counts_document_context() {
        let with_context = {
            let messages = vec![Message {
                role: "user".to_string(),
                content: json!([
                    {"type": "document", "context": "重点关注第三页数据重点关注第三页数据重点关注第三页数据", "source": {
                        "type": "base64", "media_type": "application/pdf", "data": "A".repeat(1000)
                    }}
                ]),
            }];
            count_all_tokens_local(None, &messages, None, false)
        };
        let without_context = {
            let messages = vec![Message {
                role: "user".to_string(),
                content: json!([
                    {"type": "document", "source": {
                        "type": "base64", "media_type": "application/pdf", "data": "A".repeat(1000)
                    }}
                ]),
            }];
            count_all_tokens_local(None, &messages, None, false)
        };
        assert!(
            with_context > without_context,
            "context 说明文字应该计入本地 token 估算: with={with_context}, without={without_context}"
        );
    }

    #[test]
    fn count_all_tokens_local_history_document_uses_placeholder_not_full_estimate() {
        // converter.rs 只把 documents 塞进 currentMessage，历史消息里的 PDF 会被降级成
        // 一句占位文本——本地估算不该按整份文件估算，否则历史里的 PDF 会被重复计入巨量 token
        let pdf_data = "A".repeat(200_000);
        let messages = vec![
            Message {
                role: "user".to_string(),
                content: json!([
                    {"type": "document", "source": {
                        "type": "base64", "media_type": "application/pdf", "data": pdf_data
                    }}
                ]),
            },
            Message {
                role: "assistant".to_string(),
                content: json!("好的"),
            },
            Message {
                role: "user".to_string(),
                content: json!("金额是多少"),
            },
        ];
        let total = count_all_tokens_local(None, &messages, None, false);
        let full_estimate = estimate_document_tokens(&"A".repeat(200_000));
        assert!(
            total < full_estimate,
            "历史消息里的文档不应该按整份文件估算: total={total}, full_estimate={full_estimate}"
        );
    }

    #[test]
    fn count_all_tokens_local_unsupported_media_type_uses_placeholder() {
        // converter.rs 只接受 application/pdf，其余类型会降级成占位文本
        let big_csv = "A".repeat(200_000);
        let messages = vec![Message {
            role: "user".to_string(),
            content: json!([
                {"type": "document", "source": {
                    "type": "base64", "media_type": "text/csv", "data": big_csv
                }}
            ]),
        }];
        let total = count_all_tokens_local(None, &messages, None, false);
        let full_estimate = estimate_document_tokens(&"A".repeat(200_000));
        assert!(
            total < full_estimate,
            "不支持的文档类型不应该按整份文件估算: total={total}, full_estimate={full_estimate}"
        );
    }

    #[test]
    fn count_all_tokens_local_sixth_document_in_message_uses_placeholder() {
        let doc = |i: usize| {
            json!({"type": "document", "title": format!("d{i}.pdf"), "source": {
                "type": "base64", "media_type": "application/pdf", "data": "A".repeat(200_000)
            }})
        };
        let blocks: Vec<_> = (0..6).map(doc).collect();
        let messages = vec![Message {
            role: "user".to_string(),
            content: serde_json::Value::Array(blocks),
        }];
        let total = count_all_tokens_local(None, &messages, None, false);
        let one_doc_estimate = estimate_document_tokens(&"A".repeat(200_000));
        assert!(
            total < one_doc_estimate * 6,
            "第 6 个文档不应该按整份文件估算: total={total}, one_doc_estimate*6={}",
            one_doc_estimate * 6
        );
    }

    #[test]
    fn count_all_tokens_local_counts_text_document_block_as_text() {
        let messages = vec![Message {
            role: "user".to_string(),
            content: json!([
                {"type": "document", "source": {
                    "type": "text", "media_type": "text/plain", "data": "纯文本文档内容重复重复重复重复重复重复重复重复重复重复"
                }}
            ]),
        }];
        let total = count_all_tokens_local(None, &messages, None, false);
        assert!(total > 1);
    }

    #[test]
    fn estimate_output_tokens_counts_thinking_blocks() {
        let with_thinking = estimate_output_tokens(&[json!({
            "type": "thinking",
            "thinking": "需要计入输出 token"
        })]);
        let text_only = estimate_output_tokens(&[json!({
            "type": "text",
            "text": ""
        })]);

        assert!(with_thinking > text_only);
    }

    #[test]
    fn estimate_output_tokens_counts_redacted_thinking() {
        let tokens = estimate_output_tokens(&[json!({
            "type": "redacted_thinking",
            "data": "encrypted"
        })]);

        assert!(tokens >= 8);
    }
}
