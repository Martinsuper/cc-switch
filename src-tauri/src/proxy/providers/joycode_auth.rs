//! Joycode Auth Token Reader
//!
//! 从 `~/.joycode/auth.json` 读取 access_token 和 base_url，
//! 使用 mtime 缓存机制：joycode-cli 刷新 token 时会重写文件，
//! 按 mtime 失效自动重读。

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use crate::proxy::error::ProxyError;

use crate::provider::Provider;

/// 检测是否为 Joycode Provider
///
/// 统一判断逻辑，覆盖所有配置格式：
/// 1. meta.provider_type == "joycode"
/// 2. Claude 格式：env.ANTHROPIC_BASE_URL 包含 joycode-api-inner
/// 3. Codex 格式：base_url / baseURL / config TOML base_url 包含 joycode-api-inner
pub fn is_joycode_provider(provider: &Provider) -> bool {
    // 1. meta.provider_type
    if provider
        .meta
        .as_ref()
        .and_then(|m| m.provider_type.as_deref())
        == Some("joycode")
    {
        return true;
    }
    // 2. Claude 格式：env.ANTHROPIC_BASE_URL 包含 joycode-api-inner
    if provider
        .settings_config
        .pointer("/env/ANTHROPIC_BASE_URL")
        .and_then(|v| v.as_str())
        .map(|url| url.contains("joycode-api-inner"))
        .unwrap_or(false)
    {
        return true;
    }
    // 3. Codex 格式：base_url / baseURL / config TOML base_url 包含 joycode-api-inner
    let has_joycode_base = provider
        .settings_config
        .get("base_url")
        .and_then(|v| v.as_str())
        .or_else(|| provider.settings_config.get("baseURL").and_then(|v| v.as_str()))
        .map(|url| url.contains("joycode-api-inner"))
        .unwrap_or(false)
        || provider
            .settings_config
            .get("config")
            .and_then(|v| v.as_str())
            .and_then(|s| {
                if let Some(start) = s.find("base_url = \"") {
                    let rest = &s[start + 12..];
                    rest.find('"').map(|end| rest[..end].to_string())
                } else {
                    None
                }
            })
            .map(|url| url.contains("joycode-api-inner"))
            .unwrap_or(false);
    has_joycode_base
}


/// Joycode auth.json 的缓存条目
struct AuthCache {
    /// 缓存的文件路径
    path: PathBuf,
    /// 文件的 mtime（毫秒级时间戳）
    mtime_ms: u64,
    /// 缓存的 token
    token: String,
    /// 缓存的 base_url
    base_url: String,
}

/// 全局 mtime 缓存（Mutex 保证线程安全）
static AUTH_CACHE: Mutex<Option<AuthCache>> = Mutex::new(None);

/// Joycode auth.json 的结构
#[derive(serde::Deserialize)]
struct JoycodeAuthFile {
    tokens: JoycodeTokens,
    base_url: String,
}

#[derive(serde::Deserialize)]
struct JoycodeTokens {
    access_token: String,
}

/// 获取默认 auth.json 路径
fn default_auth_path() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| "~".to_string());
    PathBuf::from(home).join(".joycode").join("auth.json")
}

/// 读取 Joycode auth.json，提取 access_token 和 base_url
///
/// 使用 mtime 缓存：如果文件未修改则直接返回缓存值。
/// joycode-cli 刷新 token 时会重写 auth.json 文件，
/// mtime 变化后缓存自动失效重读。
pub fn read_joycode_auth(auth_path: Option<&Path>) -> Result<(String, String), ProxyError> {
    let path = auth_path
        .map(PathBuf::from)
        .unwrap_or_else(default_auth_path);

    // 获取文件 mtime
    let st = std::fs::metadata(&path).map_err(|err| ProxyError::UpstreamError {
        status: 502,
        body: Some(format!(
            "读取 auth.json 失败 ({})): {}. 请先运行 `joycode-cli login`",
            path.display(),
            err
        )),
    })?;
    let mtime = st
        .modified()
        .unwrap_or(SystemTime::UNIX_EPOCH)
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    // 检查缓存
    let cache = AUTH_CACHE.lock().unwrap();
    if let Some(ref cached) = *cache {
        if cached.path == path && cached.mtime_ms == mtime {
            return Ok((cached.token.clone(), cached.base_url.clone()));
        }
    }
    // 缓存未命中或失效，释放锁后读取文件
    drop(cache);

    // 读取并解析 auth.json
    let raw = std::fs::read_to_string(&path).map_err(|err| ProxyError::UpstreamError {
        status: 502,
        body: Some(format!("读取 auth.json 失败: {}", err)),
    })?;
    let parsed: JoycodeAuthFile =
        serde_json::from_str(&raw).map_err(|err| ProxyError::UpstreamError {
            status: 502,
            body: Some(format!("auth.json 不是合法 JSON: {}", err)),
        })?;

    if parsed.tokens.access_token.is_empty() || parsed.base_url.is_empty() {
        return Err(ProxyError::AuthError(
            "auth.json 缺少 tokens.access_token 或 base_url，请重新跑 `joycode-cli login`"
                .to_string(),
        ));
    }

    let token = parsed.tokens.access_token;
    let base_url = parsed.base_url.trim_end_matches('/').to_string();

    // 更新缓存
    let mut cache = AUTH_CACHE.lock().unwrap();
    *cache = Some(AuthCache {
        path,
        mtime_ms: mtime,
        token: token.clone(),
        base_url: base_url.clone(),
    });

    Ok((token, base_url))
}

/// 从 Provider 的 settings_config 中获取 JOYCODE_AUTH_PATH
pub fn get_joycode_auth_path_from_provider(
    provider: &crate::provider::Provider,
) -> Option<PathBuf> {
    provider
        .settings_config
        .pointer("/env/JOYCODE_AUTH_PATH")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| {
            // 展开 ~ 为 HOME 目录
            if s.starts_with('~') {
                let home = std::env::var("HOME")
                    .or_else(|_| std::env::var("USERPROFILE"))
                    .unwrap_or_else(|_| "~".to_string());
                PathBuf::from(s.replacen('~', &home, 1))
            } else {
                PathBuf::from(s)
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn create_test_auth_file(dir: &Path, token: &str, base_url: &str) -> PathBuf {
        let auth_json = serde_json::json!({
            "tokens": { "access_token": token },
            "base_url": base_url
        });
        let path = dir.join("auth.json");
        fs::write(&path, serde_json::to_string(&auth_json).unwrap()).unwrap();
        path
    }

    #[test]
    fn test_read_joycode_auth_valid() {
        let dir = tempfile::tempdir().unwrap();
        let path = create_test_auth_file(
            dir.path(),
            "test-token-123",
            "https://joycode-api-inner.jd.com",
        );

        let (token, base_url) = read_joycode_auth(Some(&path)).unwrap();
        assert_eq!(token, "test-token-123");
        assert_eq!(base_url, "https://joycode-api-inner.jd.com"); // 末尾 / 已去除
    }

    #[test]
    fn test_read_joycode_auth_missing_file() {
        let result = read_joycode_auth(Some(Path::new("/nonexistent/auth.json")));
        assert!(result.is_err());
    }

    #[test]
    fn test_read_joycode_auth_invalid_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = PathBuf::from(dir.path()).join("auth.json");
        fs::write(&path, "not json").unwrap();

        let result = read_joycode_auth(Some(&path));
        assert!(result.is_err());
    }

    #[test]
    fn test_read_joycode_auth_empty_token() {
        let dir = tempfile::tempdir().unwrap();
        let auth_json = serde_json::json!({
            "tokens": { "access_token": "" },
            "base_url": "https://joycode-api-inner.jd.com"
        });
        let path = PathBuf::from(dir.path()).join("auth.json");
        fs::write(&path, serde_json::to_string(&auth_json).unwrap()).unwrap();

        let result = read_joycode_auth(Some(&path));
        assert!(result.is_err());
    }

    #[test]
    fn test_mtime_cache_invalidation() {
        let dir = tempfile::tempdir().unwrap();
        let path = create_test_auth_file(dir.path(), "old-token", "https://old.example.com");

        // First read
        let (token1, _) = read_joycode_auth(Some(&path)).unwrap();
        assert_eq!(token1, "old-token");

        // Wait for mtime granularity (filesystem mtime is second-level on many platforms)
        std::thread::sleep(std::time::Duration::from_millis(10));

        // Update file (changes mtime)
        let new_auth_json = serde_json::json!({
            "tokens": { "access_token": "new-token" },
            "base_url": "https://new.example.com"
        });
        fs::write(&path, serde_json::to_string(&new_auth_json).unwrap()).unwrap();

        // Second read should get new value (mtime changed)
        let (token2, base_url2) = read_joycode_auth(Some(&path)).unwrap();
        assert_eq!(token2, "new-token");
        assert_eq!(base_url2, "https://new.example.com");
    }

    #[test]
    fn test_base_url_trailing_slash_removed() {
        let dir = tempfile::tempdir().unwrap();
        let path = create_test_auth_file(dir.path(), "token", "https://joycode-api-inner.jd.com/");

        let (_, base_url) = read_joycode_auth(Some(&path)).unwrap();
        assert_eq!(base_url, "https://joycode-api-inner.jd.com");
    }
}
