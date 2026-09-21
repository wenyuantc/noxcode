use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use super::dispatch::ToolCtx;
use super::web_access::{authorize_hop, parse_url, sanitized_url, FetchSettings, NetworkBudget};
use futures_util::{Stream, StreamExt};
use serde_json::Value;

const FETCH_TIMEOUT: Duration = Duration::from_secs(30);
const FETCH_BODY_LIMIT: usize = 512 * 1024;
const FETCH_MODEL_CHARS: usize = 80_000;
const SEARCH_MODEL_CHARS: usize = 20_000;
/// WebFetch 内存缓存：同一 URL 15 分钟内复用，总量不超过 50 MB。
const FETCH_CACHE_TTL: Duration = Duration::from_secs(15 * 60);
const FETCH_CACHE_MAX_BYTES: usize = 50 * 1024 * 1024;

#[derive(Clone)]
struct CachedFetch {
    chain: Vec<reqwest::Url>,
    accounted_bytes: usize,
    status: u16,
    text: String,
    fetched_at: Instant,
}

struct FetchCache {
    entries: HashMap<String, CachedFetch>,
    total_bytes: usize,
}

impl FetchCache {
    fn new() -> Self {
        Self {
            entries: HashMap::new(),
            total_bytes: 0,
        }
    }

    fn get(&mut self, url: &str, now: Instant) -> Option<(u16, String)> {
        let expired = self
            .entries
            .get(url)
            .is_some_and(|entry| now.duration_since(entry.fetched_at) > FETCH_CACHE_TTL);
        if expired {
            self.remove(url);
            return None;
        }
        self.entries
            .get(url)
            .map(|entry| (entry.status, entry.text.clone()))
    }

    fn remove(&mut self, url: &str) {
        if let Some(entry) = self.entries.remove(url) {
            self.total_bytes = self.total_bytes.saturating_sub(entry.accounted_bytes);
        }
    }

    fn put(&mut self, url: &str, status: u16, text: &str, now: Instant) {
        self.put_chain(url, status, text, Vec::new(), now);
    }

    fn put_chain(
        &mut self,
        url: &str,
        status: u16,
        text: &str,
        chain: Vec<reqwest::Url>,
        now: Instant,
    ) {
        // Charge keys, redirect URLs and entry storage as well as body text.
        // A nonzero minimum prevents unlimited empty-response cache entries.
        let accounted_bytes = url.len()
            + text.len()
            + std::mem::size_of::<CachedFetch>()
            + std::mem::size_of::<String>()
            + chain.capacity() * std::mem::size_of::<reqwest::Url>()
            + chain.iter().map(|hop| hop.as_str().len()).sum::<usize>()
            + 128;
        if accounted_bytes > FETCH_CACHE_MAX_BYTES {
            return;
        }
        self.remove(url);
        // 先清过期，再按最早抓取顺序淘汰直到放得下。
        let expired: Vec<String> = self
            .entries
            .iter()
            .filter(|(_, entry)| now.duration_since(entry.fetched_at) > FETCH_CACHE_TTL)
            .map(|(key, _)| key.clone())
            .collect();
        for key in expired {
            self.remove(&key);
        }
        while self.total_bytes + accounted_bytes > FETCH_CACHE_MAX_BYTES {
            let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.fetched_at)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            self.remove(&oldest);
        }
        self.total_bytes += accounted_bytes;
        self.entries.insert(
            url.to_string(),
            CachedFetch {
                chain,
                accounted_bytes,
                status,
                text: text.to_string(),
                fetched_at: now,
            },
        );
    }
}

static FETCH_CACHE: LazyLock<Mutex<FetchCache>> = LazyLock::new(|| Mutex::new(FetchCache::new()));

fn cached_fetch(key: &str) -> Option<CachedFetch> {
    let mut cache = FETCH_CACHE.lock().ok()?;
    cache.get(key, Instant::now())?;
    cache.entries.get(key).cloned()
}

fn store_fetch(key: &str, status: u16, text: &str, chain: Vec<reqwest::Url>) {
    if let Ok(mut cache) = FETCH_CACHE.lock() {
        cache.put_chain(key, status, text, chain, Instant::now());
    }
}

async fn read_bounded_body<S, B, E>(stream: S) -> Result<(Vec<u8>, bool), String>
where
    S: Stream<Item = Result<B, E>>,
    B: AsRef<[u8]>,
    E: std::fmt::Display,
{
    futures_util::pin_mut!(stream);
    let mut body = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| format!("读取网页失败: {error}"))?;
        let bytes = chunk.as_ref();
        let keep = bytes.len().min(FETCH_BODY_LIMIT - body.len());
        body.extend_from_slice(&bytes[..keep]);
        if body.len() == FETCH_BODY_LIMIT {
            // Drop the response stream here. Reading a whole response and
            // truncating afterwards does not bound network or memory use.
            return Ok((body, true));
        }
    }
    Ok((body, false))
}

async fn fetch_page(
    ctx: &ToolCtx,
    settings: &FetchSettings,
    initial_url: reqwest::Url,
    initial_client: reqwest::Client,
    budget: &mut NetworkBudget,
) -> Result<(u16, String, Vec<reqwest::Url>), String> {
    let mut url = initial_url;
    let mut client = initial_client;
    let mut chain = Vec::new();
    loop {
        chain.push(url.clone());
        let response = budget
            .run(ctx, async {
                client
                    .get(url.clone())
                    .header("user-agent", "noxcode-native/0.1 (+coding-agent)")
                    .send()
                    .await
                    .map_err(|error| format!("WebFetch 请求失败: {}", error.without_url()))
            })
            .await?;
        if matches!(response.status().as_u16(), 301 | 302 | 303 | 307 | 308) {
            if let Some(location) = response.headers().get(reqwest::header::LOCATION) {
                if chain.len() > 5 {
                    return Err("WebFetch 重定向超过 5 次".into());
                }
                let location = location.to_str().map_err(|_| "WebFetch 重定向地址无效")?;
                let mut next = parse_url(
                    url.join(location)
                        .map_err(|_| "WebFetch 重定向地址无效")?
                        .as_str(),
                )?;
                // A redirect is a fresh unauthenticated request. Never forward
                // URL userinfo or allow a Location header to introduce it.
                let _ = next.set_username("");
                let _ = next.set_password(None);
                if chain.contains(&next) {
                    return Err("WebFetch 检测到重定向循环".into());
                }
                drop(response);
                client = authorize_hop(ctx, settings, &next, budget).await?;
                url = next;
                continue;
            }
        }
        let status = response.status().as_u16();
        let (bytes, limited) = budget
            .run(ctx, read_bounded_body(response.bytes_stream()))
            .await?;
        let raw = String::from_utf8_lossy(&bytes);
        let text = truncate_chars(&strip_tags(&raw), FETCH_MODEL_CHARS);
        return Ok((
            status,
            if limited {
                format!("[响应正文达到读取上限，以下内容可能不完整]\n{text}")
            } else {
                text
            },
            chain,
        ));
    }
}

pub async fn web_fetch(ctx: &ToolCtx, arguments: &str) -> Result<String, String> {
    let args = parse_args(arguments)?;
    let url = parse_url(&string_arg(&args, "url")?)?;
    let prompt = args
        .get("prompt")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let mut budget = NetworkBudget::new(FETCH_TIMEOUT);
    let settings = budget.run(ctx, FetchSettings::load(ctx)).await?;
    let client = authorize_hop(ctx, &settings, &url, &mut budget).await?;
    let key = format!(
        "{}:{}:{}",
        ctx.web_network.cache_scope, settings.identity, url
    );
    let (status, text) = match cached_fetch(&key) {
        Some(hit) => {
            // Cached redirects retain their full access chain. Re-resolve and
            // re-authorize every destination with current rules and grants.
            for hop in hit.chain.iter().skip(1) {
                authorize_hop(ctx, &settings, hop, &mut budget).await?;
            }
            (hit.status, hit.text)
        }
        None => {
            let (status, text, chain) =
                fetch_page(ctx, &settings, url.clone(), client, &mut budget).await?;
            if (200..300).contains(&status) {
                store_fetch(&key, status, &text, chain);
            }
            (status, text)
        }
    };
    let display = sanitized_url(&url);
    if prompt.is_empty() {
        Ok(format!("URL: {display}\nStatus: {status}\n\n{text}"))
    } else {
        Ok(format!(
            "URL: {display}\nStatus: {status}\nPrompt: {prompt}\n\nContent:\n{text}"
        ))
    }
}

pub async fn web_search(arguments: &str) -> Result<String, String> {
    let args = parse_args(arguments)?;
    let query = string_arg(&args, "query")?;
    let num = args
        .get("num_results")
        .or_else(|| args.get("numResults"))
        .and_then(Value::as_i64)
        .unwrap_or(10)
        .clamp(1, 10) as usize;
    let text = if let Ok(key) = std::env::var("EXA_API_KEY") {
        let trimmed = key.trim();
        if trimmed.is_empty() {
            duckduckgo_search(&query, num).await?
        } else {
            exa_search(&query, num, trimmed).await?
        }
    } else {
        duckduckgo_search(&query, num).await?
    };
    Ok(truncate_chars(&text, SEARCH_MODEL_CHARS))
}

fn parse_args(arguments: &str) -> Result<Value, String> {
    if arguments.trim().is_empty() {
        return Ok(Value::Object(serde_json::Map::new()));
    }
    serde_json::from_str(arguments).map_err(|error| format!("工具参数不是合法 JSON: {error}"))
}

fn string_arg(args: &Value, key: &str) -> Result<String, String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("{key} 不能为空"))
}

pub fn ensure_http_url(url: &str) -> Result<(), String> {
    let parsed = reqwest::Url::parse(url).map_err(|_| "url 不是合法地址".to_string())?;
    match parsed.scheme() {
        "http" | "https" => Ok(()),
        other => Err(format!("WebFetch 仅允许 http/https，收到 {other}")),
    }
}

fn strip_tags(input: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for ch in input.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let prefix: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{prefix}…")
}

async fn duckduckgo_search(query: &str, num: usize) -> Result<String, String> {
    let encoded = urlencoding_query(query);
    let url = format!("https://html.duckduckgo.com/html/?q={encoded}");
    let client = reqwest::Client::builder()
        .timeout(FETCH_TIMEOUT)
        .build()
        .map_err(|error| format!("创建 HTTP 客户端失败: {error}"))?;
    let html = client
        .get(&url)
        .header("user-agent", "codex-ai-native/0.1")
        .send()
        .await
        .map_err(|error| format!("WebSearch 失败: {error}"))?
        .text()
        .await
        .map_err(|error| format!("读取搜索结果失败: {error}"))?;
    Ok(format_search_hits(query, parse_ddg_results(&html, num)))
}

fn parse_ddg_results(html: &str, num: usize) -> Vec<(String, String)> {
    let mut hits = Vec::new();
    let mut rest = html;
    while hits.len() < num {
        let Some(anchor) = rest.find("class=\"result__a\"") else {
            break;
        };
        rest = &rest[anchor..];
        let href = attr_after(rest, "href=\"").unwrap_or_default();
        let title = {
            let start = rest.find('>').map(|index| index + 1).unwrap_or(0);
            let end = rest[start..].find("</a>").unwrap_or(0);
            strip_tags(&rest[start..start + end])
        };
        if let Some(end) = rest.find("</a>") {
            rest = &rest[end + 4..];
        } else {
            break;
        }
        if title.is_empty() && href.is_empty() {
            continue;
        }
        hits.push((title, href));
    }
    hits
}

fn attr_after(html: &str, key: &str) -> Option<String> {
    let start = html.find(key)? + key.len();
    let end = html[start..].find('"')?;
    Some(html[start..start + end].to_string())
}

fn urlencoding_query(query: &str) -> String {
    let mut out = String::new();
    for ch in query.chars() {
        match ch {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => out.push(ch),
            ' ' => out.push('+'),
            _ => {
                for byte in ch.to_string().as_bytes() {
                    out.push_str(&format!("%{byte:02X}"));
                }
            }
        }
    }
    out
}

async fn exa_search(query: &str, num: usize, api_key: &str) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(FETCH_TIMEOUT)
        .build()
        .map_err(|error| format!("创建 HTTP 客户端失败: {error}"))?;
    let body = serde_json::json!({
        "query": query,
        "numResults": num,
        "type": "auto",
        "contents": {"text": {"maxCharacters": 800}},
    });
    let response = client
        .post("https://api.exa.ai/search")
        .header("content-type", "application/json")
        .header("x-api-key", api_key)
        .json(&body)
        .send()
        .await
        .map_err(|error| format!("Exa 搜索失败: {error}"))?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("Exa 搜索失败: HTTP {}", status.as_u16()));
    }
    let parsed: Value =
        serde_json::from_str(&text).map_err(|error| format!("解析 Exa 结果失败: {error}"))?;
    let mut hits = Vec::new();
    if let Some(results) = parsed.get("results").and_then(Value::as_array) {
        for item in results.iter().take(num) {
            let title = item
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let url = item
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            hits.push((title, url));
        }
    }
    Ok(format_search_hits(query, hits))
}

fn format_search_hits(query: &str, hits: Vec<(String, String)>) -> String {
    if hits.is_empty() {
        return format!("Query: {query}\nNo results");
    }
    let mut out = format!("Query: {query}\nAfter answering, list Sources as markdown links.\n");
    for (index, (title, url)) in hits.iter().enumerate() {
        out.push_str(&format!("{}. {title}\n   {url}\n", index + 1));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn bounded_body_stops_before_an_unbounded_tail() {
        let stream = futures_util::stream::iter([
            Ok(vec![b'a'; FETCH_BODY_LIMIT]),
            Err("the tail must never be polled"),
        ]);
        let (body, limited) = read_bounded_body(stream).await.unwrap();
        assert_eq!(body.len(), FETCH_BODY_LIMIT);
        assert!(limited);
    }

    #[tokio::test]
    async fn bounded_body_caps_a_large_chunk_and_joins_small_chunks() {
        let large =
            futures_util::stream::iter([Ok::<_, String>(vec![b'x'; FETCH_BODY_LIMIT + 100])]);
        let (body, limited) = read_bounded_body(large).await.unwrap();
        assert_eq!(body, vec![b'x'; FETCH_BODY_LIMIT]);
        assert!(limited);
        let short =
            futures_util::stream::iter([Ok::<_, String>(b"hel".to_vec()), Ok(b"lo".to_vec())]);
        assert_eq!(
            read_bounded_body(short).await.unwrap(),
            (b"hello".to_vec(), false)
        );
    }

    #[tokio::test]
    async fn bounded_body_propagates_errors_before_the_limit() {
        let stream = futures_util::stream::iter([Ok(b"partial".to_vec()), Err("connection lost")]);
        assert!(read_bounded_body(stream)
            .await
            .unwrap_err()
            .contains("connection lost"));
    }

    #[test]
    fn rejects_file_scheme() {
        let error = ensure_http_url("file:///etc/passwd").unwrap_err();
        assert!(error.contains("http/https"));
    }

    #[test]
    fn accepts_https() {
        ensure_http_url("https://example.com/a").expect("https ok");
    }

    #[test]
    fn empty_body_cache_accounts_for_keys_and_entry_metadata() {
        let mut cache = FetchCache::new();
        let key = "long-key".repeat(1024);
        cache.put(&key, 200, "", Instant::now());
        assert!(cache.total_bytes >= key.len() + std::mem::size_of::<CachedFetch>());
    }

    #[test]
    fn fetch_cache_expires_and_evicts_oldest() {
        let mut cache = FetchCache::new();
        let start = Instant::now();
        cache.put("https://a", 200, "aaa", start);
        assert_eq!(
            cache.get("https://a", start),
            Some((200, "aaa".to_string()))
        );
        let later = start + FETCH_CACHE_TTL + Duration::from_secs(1);
        assert_eq!(cache.get("https://a", later), None);
        assert_eq!(cache.total_bytes, 0);
        cache.put("https://b", 200, "bbb", start);
        cache.put("https://c", 200, "ccc", start + Duration::from_secs(1));
        assert!(cache.total_bytes > 6);
        assert!(cache.total_bytes <= FETCH_CACHE_MAX_BYTES);
        let huge = "x".repeat(FETCH_CACHE_MAX_BYTES + 1);
        cache.put("https://huge", 200, &huge, start);
        assert!(cache.get("https://huge", start).is_none());
    }

    #[test]
    fn cache_evicts_oldest_and_charges_redirect_provenance() {
        let mut cache = FetchCache::new();
        let now = Instant::now();
        let body = "x".repeat(FETCH_CACHE_MAX_BYTES / 2);
        cache.put("first", 200, &body, now);
        cache.put_chain(
            "second",
            200,
            &body,
            vec![parse_url("https://example.com/redirect").unwrap()],
            now + Duration::from_secs(1),
        );
        assert!(cache.get("first", now + Duration::from_secs(1)).is_none());
        assert!(cache.get("second", now + Duration::from_secs(1)).is_some());
        assert!(cache.total_bytes <= FETCH_CACHE_MAX_BYTES);
        let with_chain = cache.total_bytes;
        cache.put("second", 200, &body, now + Duration::from_secs(2));
        assert!(cache.total_bytes < with_chain);
    }

    #[test]
    fn parses_duckduckgo_anchors() {
        let html = r#"<a class="result__a" href="https://example.com">Hello</a>"#;
        let hits = parse_ddg_results(html, 3);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0, "Hello");
        assert_eq!(hits[0].1, "https://example.com");
    }

    #[tokio::test]
    async fn private_fetch_without_permission_channel_never_connects() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let ctx = super::super::dispatch::ToolCtx::new(super::super::LocalWorkspace::new(
            std::env::temp_dir(),
        ));
        let arguments = serde_json::json!({"url": url}).to_string();
        let fetch = super::super::dispatch::execute_tool(&ctx, "WebFetch", &arguments);
        tokio::pin!(fetch);
        tokio::select! {
            result = &mut fetch => assert!(result.unwrap_err().contains("权限确认")),
            _ = listener.accept() => panic!("private network contacted before authorization"),
        }
    }

    #[tokio::test]
    async fn fetch_reads_mock_http() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let mut buf = vec![0u8; 1024];
            let _ = stream.read(&mut buf).await;
            let body = "<html><body><h1>Hi</h1><p>there</p></body></html>";
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(header.as_bytes()).await;
            let _ = stream.write_all(body.as_bytes()).await;
        });
        let ctx = ToolCtx::new(super::super::LocalWorkspace::new(std::env::temp_dir()));
        ctx.allow_all_high_risk
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let output = web_fetch(&ctx, &format!(r#"{{"url":"http://{addr}/"}}"#))
            .await
            .expect("fetch");
        assert!(output.contains("Status: 200"));
        assert!(output.contains("Hi"));
        assert!(output.contains("there"));
    }
}
