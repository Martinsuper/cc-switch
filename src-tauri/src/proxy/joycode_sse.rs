//! Joycode Gateway SSE Repackaging and Business Error Detection
//!
//! Joycode 上游网关的 SSE 流有两个怪异行为需要处理：
//!
//! 1. **双层 data: 包装**：每个事件被拆成两个独立块：
//!    ```
//!    data: event: message_start
//!
//!    data: data: {"type":"message_start",...}
//!
//!    ```
//!    标准格式应该是：
//!    ```
//!    event: message_start
//!    data: {"type":"message_start",...}
//!    ```
//!
//! 2. **网关业务错误**：HTTP 200 响应体包含 `{"code":401,...}` 这样的业务错误，
//!    需要识别并转换为真实的 HTTP 错误码。

use crate::proxy::error::ProxyError;
use crate::proxy::sse::{append_utf8_safe, take_sse_block};
use bytes::Bytes;
use futures::stream::{Stream, StreamExt};

/// 解析后的 SSE 块
#[derive(Debug)]
pub enum ParsedJoycodeBlock {
    /// 事件名（如 message_start）
    Event { name: String },
    /// 数据负载
    Data { data: serde_json::Value },
    /// 流结束标记
    Done,
}

/// 解析 Joycode 网关的双层 data: 包装 SSE 块
///
/// 上游格式（每个事件被前缀 "data: "）：
///   块1: `data: event: message_start\n\n` → 事件名
///   块2: `data: data: {"type":"message_start",...}\n\n` → 数据
///
/// 也会处理非双层包装的标准 SSE 格式（兼容直连上游的场景）。
pub fn parse_joycode_sse_block(block: &str) -> Option<ParsedJoycodeBlock> {
    let block = block.trim();
    if block.is_empty() {
        return None;
    }

    let mut event_name = None;
    let mut data_lines = Vec::new();

    for line in block.lines() {
        let mut l = line;
        // 剥外层 "data: " 前缀
        if l.starts_with("data: ") {
            l = &l[6..];
        } else if l.starts_with("data:") {
            l = &l[5..];
        }
        let l = l.trim();

        // 内层可能是 "event: xxx" 或 "data: {...}" 或 "[DONE]"
        if l.starts_with("event: ") {
            event_name = Some(l[7..].trim().to_string());
        } else if l.starts_with("event:") {
            event_name = Some(l[6..].trim().to_string());
        } else if l.starts_with("data: ") {
            data_lines.push(&l[6..]);
        } else if l.starts_with("data:") {
            data_lines.push(&l[5..]);
        } else if l == "[DONE]" {
            return Some(ParsedJoycodeBlock::Done);
        }
    }

    // 纯事件块（只有 event 名没有 data）
    if let Some(name) = event_name {
        if data_lines.is_empty() {
            return Some(ParsedJoycodeBlock::Event { name });
        }
    }

    // 数据块
    let data_str = data_lines.join("").trim().to_string();
    if data_str.is_empty() {
        return None;
    }
    if data_str == "[DONE]" {
        return Some(ParsedJoycodeBlock::Done);
    }

    match serde_json::from_str::<serde_json::Value>(&data_str) {
        Ok(data) => Some(ParsedJoycodeBlock::Data { data }),
        Err(_) => None, // 解析失败丢弃，避免污染客户端
    }
}

/// 检测 Joycode 网关业务错误
///
/// Joycode 上游在 HTTP 200 响应中返回业务错误，格式有三种：
/// - 扁平数字：`{"code":401,"msg":"..."}`
/// - 扁平字符串：`{"code":"1050","msg":"..."}`
/// - 嵌套：`{"error":{"code":1050,"message":"..."}}`
pub fn detect_joycode_business_error(body: &str) -> Option<ProxyError> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return None;
    }

    // 纯 JSON 格式
    if trimmed.starts_with('{') {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(trimmed) {
            return extract_business_error(&json);
        }
    }

    // SSE 流中嵌入的业务错误（可能带 `data: ` 前缀）
    if let Some(pos) = trimmed.find('{') {
        // 从匹配位置开始找配平的 }
        let mut depth = 0;
        let mut end = 0;
        for (i, ch) in trimmed[pos..].char_indices() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = pos + i + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        if end > 0 {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&trimmed[pos..end]) {
                return extract_business_error(&json);
            }
        }
    }

    None
}

/// 从 JSON 对象中提取业务错误码
fn extract_business_error(json: &serde_json::Value) -> Option<ProxyError> {
    if !json.is_object() {
        return None;
    }

    let code = extract_error_code(json);
    if let Some(code) = code {
        if code >= 400 {
            let msg = extract_error_message(json);
            return Some(map_business_error(code, &msg));
        }
    }

    None
}

/// 提取错误码（支持扁平格式和嵌套 error 格式）
fn extract_error_code(json: &serde_json::Value) -> Option<i32> {
    // 扁平格式：{"code": 401, ...}
    if let Some(code) = json.get("code").and_then(normalize_code) {
        return Some(code);
    }

    // 嵌套格式：{"error": {"code": 1050, ...}}
    if let Some(error) = json.get("error") {
        if let Some(code) = error.get("code").and_then(normalize_code) {
            return Some(code);
        }
    }

    None
}

/// 将 code 值规范化为数字
fn normalize_code(v: &serde_json::Value) -> Option<i32> {
    match v {
        serde_json::Value::Number(n) => n.as_i64().map(|c| c as i32),
        serde_json::Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

/// 提取错误消息
fn extract_error_message(json: &serde_json::Value) -> String {
    // 嵌套格式优先
    if let Some(error) = json.get("error") {
        if let Some(msg) = error.get("message").and_then(|v| v.as_str()) {
            return msg.to_string();
        }
    }
    // 扁平格式
    if let Some(msg) = json
        .get("msg")
        .or_else(|| json.get("message"))
        .and_then(|v| v.as_str())
    {
        return msg.to_string();
    }
    String::new()
}

/// 映射业务错误码到 ProxyError
fn map_business_error(code: i32, msg: &str) -> ProxyError {
    let msg = if msg.is_empty() {
        format!("上游业务错误 (code {})", code)
    } else {
        msg.to_string()
    };

    match code {
        401 => ProxyError::AuthError(format!(
            "上游账号未登录或 token 过期（{}）。请重新运行 `joycode-cli login`",
            msg
        )),
        403 | 1050 => ProxyError::UpstreamError {
            status: 403,
            body: Some(format!("上游拒绝访问: {}", msg)),
        },
        400..=499 => ProxyError::UpstreamError {
            status: code as u16,
            body: Some(format!("上游错误 {}: {}", code, msg)),
        },
        _ => ProxyError::UpstreamError {
            status: 502,
            body: Some(format!("上游业务错误 {}: {}", code, msg)),
        },
    }
}

/// 创建 Joycode 双层 SSE 重打包流
///
/// 将 Joycode 网关的双层 `data:` 包装 SSE 转换为标准 SSE 格式，
/// 供下游的 `create_anthropic_sse_stream` 等标准 SSE 处理器消费。
///
/// 输入（Joycode 网关格式）：
/// ```text
/// data: event: message_start\n\n
/// data: data: {"type":"message_start",...}\n\n
/// ```
///
/// 输出（标准 SSE 格式）：
/// ```text
/// event: message_start\n
/// data: {"type":"message_start",...}\n\n
/// ```
pub fn create_joycode_repackaged_stream(
    stream: impl Stream<Item = Result<Bytes, std::io::Error>> + Send + 'static,
) -> impl Stream<Item = Result<Bytes, std::io::Error>> + Send {
    async_stream::stream! {
        let mut buffer = String::new();
        let mut utf8_remainder: Vec<u8> = Vec::new();
        let mut pending_event: Option<String> = None;

        tokio::pin!(stream);

        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(bytes) => {
                    append_utf8_safe(&mut buffer, &mut utf8_remainder, &bytes);

                    let mut repackaged = String::new();
                    while let Some(event_text) = take_sse_block(&mut buffer) {
                        if event_text.trim().is_empty() {
                            continue;
                        }

                        match parse_joycode_sse_block(&event_text) {
                            Some(ParsedJoycodeBlock::Event { name }) => {
                                pending_event = Some(name);
                            }
                            Some(ParsedJoycodeBlock::Data { data }) => {
                                if let Some(event_name) = pending_event.take() {
                                    repackaged.push_str(&format!("event: {}\n", event_name));
                                }
                                let data_str = serde_json::to_string(&data).unwrap_or_default();
                                repackaged.push_str(&format!("data: {}\n\n", data_str));
                            }
                            Some(ParsedJoycodeBlock::Done) => {
                                if let Some(event_name) = pending_event.take() {
                                    repackaged.push_str(&format!("event: {}\n", event_name));
                                }
                                repackaged.push_str("data: [DONE]\n\n");
                            }
                            None => {
                                // 解析失败，原样透传
                                repackaged.push_str(&event_text);
                                repackaged.push_str("\n\n");
                            }
                        }
                    }

                    if !repackaged.is_empty() {
                        yield Ok(Bytes::from(repackaged));
                    }
                }
                Err(e) => {
                    yield Err(e);
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_joycode_event_block() {
        let block = "data: event: message_start\n\n";
        let result = parse_joycode_sse_block(block);
        match result {
            Some(ParsedJoycodeBlock::Event { name }) => assert_eq!(name, "message_start"),
            other => panic!("Expected Event, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_joycode_data_block() {
        let block = "data: data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_123\"}}\n\n";
        let result = parse_joycode_sse_block(block);
        match result {
            Some(ParsedJoycodeBlock::Data { data }) => {
                assert_eq!(data["type"], "message_start");
            }
            other => panic!("Expected Data, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_joycode_done() {
        let block = "data: [DONE]\n\n";
        let result = parse_joycode_sse_block(block);
        assert!(matches!(result, Some(ParsedJoycodeBlock::Done)));
    }

    #[test]
    fn test_parse_joycode_empty() {
        assert!(parse_joycode_sse_block("").is_none());
        assert!(parse_joycode_sse_block("   ").is_none());
    }

    #[test]
    fn test_detect_business_error_flat_numeric_code() {
        let body = r#"{"code":401,"msg":"token expired"}"#;
        let err = detect_joycode_business_error(body);
        assert!(err.is_some());
        let msg = format!("{:?}", err.unwrap());
        assert!(msg.contains("token expired") || msg.contains("未登录"));
    }

    #[test]
    fn test_detect_business_error_flat_string_code() {
        let body = r#"{"code":"1050","msg":"quota exhausted"}"#;
        let err = detect_joycode_business_error(body);
        assert!(err.is_some());
    }

    #[test]
    fn test_detect_business_error_nested() {
        let body = r#"{"error":{"code":1050,"message":"rate limited"}}"#;
        let err = detect_joycode_business_error(body);
        assert!(err.is_some());
    }

    #[test]
    fn test_detect_business_error_success_response() {
        // 正常的 Anthropic 响应，不应被识别为错误
        let body = r#"{"id":"msg_123","type":"message","role":"assistant","content":[{"type":"text","text":"hello"}]}"#;
        let err = detect_joycode_business_error(body);
        assert!(err.is_none());
    }

    #[test]
    fn test_detect_business_error_empty() {
        assert!(detect_joycode_business_error("").is_none());
        assert!(detect_joycode_business_error("   ").is_none());
    }

    #[test]
    fn test_detect_business_error_sse_wrapped() {
        // 业务错误可能被 SSE data: 前缀包装
        let body = "data: {\"code\":401,\"msg\":\"unauthorized\"}\n\n";
        let err = detect_joycode_business_error(body);
        assert!(err.is_some());
    }

    #[test]
    fn test_map_business_error_401() {
        let err = map_business_error(401, "token expired");
        let msg = format!("{:?}", err);
        assert!(msg.contains("joycode-cli login"));
    }

    #[test]
    fn test_map_business_error_403() {
        let err = map_business_error(403, "forbidden");
        let msg = format!("{:?}", err);
        assert!(msg.contains("拒绝访问"));
    }

    #[test]
    fn test_map_business_error_1050() {
        let err = map_business_error(1050, "quota exhausted");
        let msg = format!("{:?}", err);
        assert!(msg.contains("拒绝访问"));
    }
}