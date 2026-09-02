use std::time::Duration;

#[derive(Debug, Clone, Copy)]
pub struct RetryConfig {
    pub max_retries: u32,
    pub base_delay_ms: u64,
    pub max_delay_ms: u64,
    pub jitter: bool,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 10,
            base_delay_ms: 3_000,
            max_delay_ms: 3_000,
            jitter: false,
        }
    }
}

impl RetryConfig {
    pub fn none() -> Self {
        Self {
            max_retries: 0,
            base_delay_ms: 1,
            max_delay_ms: 1,
            jitter: false,
        }
    }

    pub fn delay_for_attempt(self, attempt: u32) -> Duration {
        let factor = 1u64.checked_shl(attempt.min(16)).unwrap_or(u64::MAX);
        let mut delay = self.base_delay_ms.saturating_mul(factor);
        if delay > self.max_delay_ms {
            delay = self.max_delay_ms;
        }
        if self.jitter && delay > 1 {
            let jittered = delay / 2 + (delay % 7) + 1;
            delay = jittered.min(self.max_delay_ms);
        }
        Duration::from_millis(delay.max(1))
    }
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
    lower.contains("timeout")
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
    fn default_delay_is_fixed_three_seconds() {
        let retry = RetryConfig::default();
        assert_eq!(retry.max_retries, 10);
        assert_eq!(retry.delay_for_attempt(0), Duration::from_millis(3_000));
        assert_eq!(retry.delay_for_attempt(9), Duration::from_millis(3_000));
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
