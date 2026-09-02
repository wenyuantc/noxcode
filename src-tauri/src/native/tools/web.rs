use std::time::Duration;

use serde_json::Value;

const FETCH_TIMEOUT: Duration = Duration::from_secs(30);
const FETCH_BODY_LIMIT: usize = 512 * 1024;
const FETCH_MODEL_CHARS: usize = 80_000;
const SEARCH_MODEL_CHARS: usize = 20_000;

pub async fn web_fetch(arguments: &str) -> Result<String, String> {
    let args = parse_args(arguments)?;
    let url = string_arg(&args, "url")?;
    let prompt = args
        .get("prompt")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    ensure_http_url(&url)?;
    let client = reqwest::Client::builder()
        .timeout(FETCH_TIMEOUT)
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .map_err(|error| format!("创建 HTTP 客户端失败: {error}"))?;
    let response = client
        .get(&url)
        .header("user-agent", "codex-ai-native/0.1")
        .send()
        .await
        .map_err(|error| format!("WebFetch 失败: {error}"))?;
    let status = response.status().as_u16();
    let bytes = response
        .bytes()
        .await
        .map_err(|error| format!("读取网页失败: {error}"))?;
    let raw = String::from_utf8_lossy(&bytes[..bytes.len().min(FETCH_BODY_LIMIT)]);
    let text = truncate_chars(&strip_tags(&raw), FETCH_MODEL_CHARS);
    if prompt.is_empty() {
        Ok(format!("URL: {url}\nStatus: {status}\n\n{text}"))
    } else {
        Ok(format!(
            "URL: {url}\nStatus: {status}\nPrompt: {prompt}\n\nContent:\n{text}"
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
    fn parses_duckduckgo_anchors() {
        let html = r#"<a class="result__a" href="https://example.com">Hello</a>"#;
        let hits = parse_ddg_results(html, 3);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0, "Hello");
        assert_eq!(hits[0].1, "https://example.com");
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
        let output = web_fetch(&format!(r#"{{"url":"http://{addr}/"}}"#))
            .await
            .expect("fetch");
        assert!(output.contains("Status: 200"));
        assert!(output.contains("Hi"));
        assert!(output.contains("there"));
    }
}
