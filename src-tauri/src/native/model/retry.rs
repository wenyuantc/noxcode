use std::time::Duration;

#[derive(Debug, Clone, Copy)]
pub struct RetryConfig {
    pub max_retries: u32,
    pub base_delay_ms: u64,
    pub max_delay_ms: u64,
    /// 指数退避的底数：第 n 次重试等待 `base * factor^n`，上限 `max_delay_ms`。
    pub backoff_factor: f64,
    /// 打开后在 `[delay/2, delay]` 内随机取值，避免多个会话同时撞限流。
    pub jitter: bool,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 6,
            base_delay_ms: 1_000,
            max_delay_ms: 30_000,
            backoff_factor: 2.0,
            jitter: true,
        }
    }
}

impl RetryConfig {
    pub fn none() -> Self {
        Self {
            max_retries: 0,
            base_delay_ms: 1,
            max_delay_ms: 1,
            backoff_factor: 1.0,
            jitter: false,
        }
    }

    /// 固定间隔、无抖动（测试与需要可预测节奏的场景）。
    pub fn fixed(max_retries: u32, delay_ms: u64) -> Self {
        Self {
            max_retries,
            base_delay_ms: delay_ms,
            max_delay_ms: delay_ms,
            backoff_factor: 1.0,
            jitter: false,
        }
    }

    /// 第 `attempt`（从 0 起）次失败后的等待时间；`retry_after` 是服务端给的
    /// `Retry-After`，有则优先但不超过 `max_delay_ms`。
    pub fn delay_for_attempt(self, attempt: u32) -> Duration {
        self.delay_for_attempt_with_hint(attempt, None)
    }

    pub fn delay_for_attempt_with_hint(
        self,
        attempt: u32,
        retry_after: Option<Duration>,
    ) -> Duration {
        let max_delay = self.max_delay_ms.max(1);
        if let Some(hint) = retry_after {
            let hinted = hint.as_millis().min(u128::from(max_delay)) as u64;
            return Duration::from_millis(hinted.max(1));
        }
        let factor = if self.backoff_factor.is_finite() && self.backoff_factor >= 1.0 {
            self.backoff_factor
        } else {
            1.0
        };
        let scaled = (self.base_delay_ms as f64) * factor.powi(attempt.min(16) as i32);
        let mut delay = if scaled.is_finite() {
            scaled.min(max_delay as f64) as u64
        } else {
            max_delay
        };
        if self.jitter && delay > 1 {
            let half = delay / 2;
            let span = delay - half;
            delay = half + pseudo_random_below(span.max(1));
        }
        Duration::from_millis(delay.clamp(1, max_delay))
    }
}

/// 无外部依赖的伪随机：基于当前纳秒时间取模，足够打散多会话的重试节奏。
fn pseudo_random_below(bound: u64) -> u64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|item| item.subsec_nanos() as u64)
        .unwrap_or(0);
    let mixed = nanos ^ (nanos >> 13) ^ 0x9E37_79B9;
    mixed % bound.max(1)
}

/// 解析 `Retry-After` 头：支持秒数与 HTTP 日期。
pub fn parse_retry_after(value: &str) -> Option<Duration> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(seconds) = trimmed.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    if let Ok(seconds) = trimmed.parse::<f64>() {
        if seconds.is_finite() && seconds >= 0.0 {
            return Some(Duration::from_millis((seconds * 1000.0) as u64));
        }
    }
    let target = chrono::DateTime::parse_from_rfc2822(trimmed).ok()?;
    let now = chrono::Utc::now();
    let delta = target.with_timezone(&chrono::Utc) - now;
    delta.to_std().ok()
}

pub fn is_retryable_status(status: u16) -> bool {
    matches!(status, 408 | 409 | 429) || status >= 500
}

pub fn is_retryable_error(status: Option<u16>, message: &str) -> bool {
    if let Some(status) = status {
        if matches!(status, 401 | 403 | 404) {
            return false;
        }
        if is_retryable_status(status) {
            return true;
        }
        if (400..500).contains(&status) {
            return false;
        }
    }
    if is_non_retryable_message(message) {
        return false;
    }
    is_transient_message(message)
}

pub fn format_retry_line(error: &str, attempt: u32, max_retries: u32, delay: Duration) -> String {
    let secs = delay.as_secs().max(1);
    format!("[重试] {error}，{secs} 秒后进行第 {attempt}/{max_retries} 次重试")
}

fn is_non_retryable_message(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("insufficient quota")
        || lower.contains("invalid api key")
        || lower.contains("invalid_api_key")
        || lower.contains("unauthorized")
        || lower.contains("authentication")
}

fn is_transient_message(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    // 流中断（读到一半连接断开）视为可恢复：重试会丢弃半截内容重新生成。
    lower.contains("读取模型响应失败")
        || lower.contains("timeout")
        || lower.contains("timed out")
        || lower.contains("overloaded")
        || lower.contains("rate limit")
        || lower.contains("too many requests")
        || lower.contains("try again")
        || lower.contains("temporarily")
        || lower.contains("connection")
        || lower.contains("connect")
        || lower.contains("broken pipe")
        || lower.contains("reset")
        || lower.contains("eof")
        || lower.contains("空响应")
        || lower.contains("模型返回错误")
        || lower.contains("模型请求失败")
        || lower.contains("502")
        || lower.contains("503")
        || lower.contains("504")
        || lower.contains("529")
}

fn is_token_boundary(text: &str, index: usize) -> bool {
    if index == 0 {
        return true;
    }
    text[..index]
        .chars()
        .next_back()
        .is_none_or(|ch| !(ch.is_ascii_alphanumeric() || ch == '-' || ch == '_'))
}

pub fn redact_secrets(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let lower = text.to_ascii_lowercase();
    let mut i = 0;
    while i < text.len() {
        if is_token_boundary(text, i) && lower[i..].starts_with("bearer ") {
            out.push_str("[redacted]");
            i += "bearer ".len();
            i = skip_token(text, i);
            continue;
        }
        if is_token_boundary(text, i)
            && (lower[i..].starts_with("sk-") || lower[i..].starts_with("sk_"))
        {
            out.push_str("[redacted]");
            i = skip_token(text, i);
            continue;
        }
        let next = text[i..].chars().next().unwrap_or(' ');
        out.push(next);
        i += next.len_utf8();
    }
    out
}

fn skip_token(text: &str, mut i: usize) -> usize {
    while i < text.len() {
        let Some(ch) = text[i..].chars().next() else {
            break;
        };
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            i += ch.len_utf8();
        } else {
            break;
        }
    }
    i
}

pub fn format_http_error(status: u16, url: &str, body: &str) -> String {
    let snippet: String = redact_secrets(body)
        .chars()
        .filter(|ch| *ch != '\n' && *ch != '\r')
        .take(180)
        .collect();
    let host = redact_secrets(url.split('?').next().unwrap_or(url));
    if snippet.trim().is_empty() {
        format!("模型请求失败（HTTP {status}）: {host}")
    } else {
        format!("模型请求失败（HTTP {status}）: {snippet}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retries_rate_limits_and_server_errors() {
        assert!(is_retryable_status(429));
        assert!(is_retryable_status(503));
        assert!(!is_retryable_status(400));
        assert!(!is_retryable_status(401));
    }

    #[test]
    fn default_delay_backs_off_exponentially_with_jitter() {
        let retry = RetryConfig::default();
        assert_eq!(retry.max_retries, 6);
        let first = retry.delay_for_attempt(0);
        assert!(first >= Duration::from_millis(500) && first <= Duration::from_millis(1_000));
        let third = retry.delay_for_attempt(2);
        assert!(third >= Duration::from_millis(2_000) && third <= Duration::from_millis(4_000));
        let capped = retry.delay_for_attempt(12);
        assert!(capped <= Duration::from_millis(30_000));
        assert!(capped >= Duration::from_millis(15_000));
    }

    #[test]
    fn fixed_config_and_retry_after_hint() {
        let retry = RetryConfig::fixed(3, 3_000);
        assert_eq!(retry.delay_for_attempt(0), Duration::from_millis(3_000));
        assert_eq!(retry.delay_for_attempt(5), Duration::from_millis(3_000));
        let hinted = retry.delay_for_attempt_with_hint(0, Some(Duration::from_secs(2)));
        assert_eq!(hinted, Duration::from_millis(2_000));
        // Retry-After 超过上限时被截到 max_delay_ms。
        let capped = retry.delay_for_attempt_with_hint(0, Some(Duration::from_secs(90)));
        assert_eq!(capped, Duration::from_millis(3_000));
        assert_eq!(parse_retry_after("7"), Some(Duration::from_secs(7)));
        assert_eq!(parse_retry_after("1.5"), Some(Duration::from_millis(1_500)));
        assert_eq!(parse_retry_after(""), None);
        assert_eq!(parse_retry_after("garbage"), None);
    }

    #[test]
    fn stream_interruption_is_retryable() {
        assert!(is_retryable_error(
            None,
            "读取模型响应失败: error decoding response body"
        ));
    }

    #[test]
    fn retryable_error_covers_transient_and_auth() {
        assert!(is_retryable_error(Some(503), "模型请求失败（HTTP 503）"));
        assert!(is_retryable_error(Some(200), "模型返回错误：overloaded"));
        assert!(is_retryable_error(Some(200), "模型返回空响应：正文为空"));
        assert!(is_retryable_error(None, "模型请求失败: connection reset"));
        assert!(!is_retryable_error(Some(401), "模型请求失败（HTTP 401）"));
        assert!(!is_retryable_error(Some(403), "模型请求失败（HTTP 403）"));
        assert!(!is_retryable_error(Some(404), "模型请求失败（HTTP 404）"));
        assert!(!is_retryable_error(Some(400), "max_tokens is too large"));
        assert!(!is_retryable_error(
            Some(200),
            "模型返回错误：insufficient quota"
        ));
        assert!(!is_retryable_error(
            Some(200),
            "模型返回错误：invalid api key"
        ));
        assert!(!is_retryable_error(Some(200), "模型返回错误：unauthorized"));
    }

    #[test]
    fn retry_line_includes_attempt() {
        let line = format_retry_line(
            "模型请求失败（HTTP 503）: gateway",
            1,
            10,
            Duration::from_secs(3),
        );
        assert!(line.starts_with("[重试] "));
        assert!(line.contains("第 1/10 次重试"));
        assert!(line.contains("3 秒后"));
    }

    #[test]
    fn http_error_does_not_echo_authorization() {
        let message = format_http_error(
            401,
            "https://api.example.com/v1/chat/completions",
            "Authorization: Bearer sk-secret-key invalid",
        );
        assert!(message.contains("HTTP 401"));
        assert!(!message.contains("sk-secret-key"));
        assert!(!message.to_ascii_lowercase().contains("bearer sk"));
    }
}
