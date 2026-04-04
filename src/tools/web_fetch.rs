use std::net::IpAddr;
use std::time::Instant;

use serde_json::{Value, json};

const MAX_CONTENT_BYTES: usize = 100 * 1024; // 100 KB
const MAX_REDIRECTS: usize = 5;
const TIMEOUT_SECS: u64 = 30;

const BLOCKED_HOSTS: &[&str] = &[
    "localhost",
    "127.0.0.1",
    "0.0.0.0",
    "::1",
    "169.254.169.254",           // AWS metadata
    "metadata.google.internal",  // GCP metadata
];

pub async fn web_fetch_tool_response(args: &Value) -> Value {
    let url = match args.get("url").and_then(Value::as_str).map(str::trim) {
        Some(u) if !u.is_empty() => u,
        _ => return error("invalid_args", "'url' is required"),
    };
    let prompt = args
        .get("prompt")
        .and_then(Value::as_str)
        .unwrap_or("Extract the main content");

    // Validate URL
    let parsed = match reqwest::Url::parse(url) {
        Ok(u) => u,
        Err(_) => return error("invalid_url", format!("invalid URL: {}", url)),
    };

    if !matches!(parsed.scheme(), "http" | "https") {
        return error("invalid_url", "only http and https URLs are supported");
    }

    if let Err(msg) = check_blocked_host(&parsed) {
        return error("blocked_domain", msg);
    }

    let start = Instant::now();

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(TIMEOUT_SECS))
        .redirect(reqwest::redirect::Policy::limited(MAX_REDIRECTS))
        .user_agent(format!("zavora-cli/{}", env!("CARGO_PKG_VERSION")))
        .build()
    {
        Ok(c) => c,
        Err(e) => return error("http_error", format!("failed to build HTTP client: {}", e)),
    };

    let response = match client.get(url).send().await {
        Ok(r) => r,
        Err(e) => return error("http_error", format!("request failed: {}", e)),
    };

    let status = response.status();
    let code = status.as_u16();
    let code_text = status.canonical_reason().unwrap_or("Unknown").to_string();
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let final_url = response.url().to_string();

    // Check final redirect destination isn't blocked
    if let Ok(final_parsed) = reqwest::Url::parse(&final_url) {
        if let Err(msg) = check_blocked_host(&final_parsed) {
            return error("blocked_domain", format!("redirect to blocked host: {}", msg));
        }
    }

    let body_bytes = match response.bytes().await {
        Ok(b) => b,
        Err(e) => return error("http_error", format!("failed to read body: {}", e)),
    };

    let bytes = body_bytes.len();
    let body = String::from_utf8_lossy(&body_bytes[..bytes.min(MAX_CONTENT_BYTES)]);

    // Convert based on content type
    let result = if content_type.contains("text/html") || content_type.contains("application/xhtml") {
        html_to_markdown(&body)
    } else if content_type.contains("application/json") {
        // Pretty-print JSON
        match serde_json::from_str::<Value>(&body) {
            Ok(v) => serde_json::to_string_pretty(&v).unwrap_or_else(|_| body.to_string()),
            Err(_) => body.to_string(),
        }
    } else {
        body.to_string()
    };

    // Truncate result
    let result = if result.len() > MAX_CONTENT_BYTES {
        format!("{}...\n[truncated at {}KB]", &result[..MAX_CONTENT_BYTES], MAX_CONTENT_BYTES / 1024)
    } else {
        result
    };

    json!({
        "url": final_url,
        "code": code,
        "codeText": code_text,
        "bytes": bytes,
        "result": result,
        "prompt": prompt,
        "durationMs": start.elapsed().as_millis() as u64,
    })
}

fn check_blocked_host(url: &reqwest::Url) -> Result<(), String> {
    let host = url.host_str().unwrap_or("");

    if BLOCKED_HOSTS.iter().any(|b| host.eq_ignore_ascii_case(b)) {
        return Err(format!("host '{}' is blocked", host));
    }

    // Block private IP ranges
    if let Ok(ip) = host.parse::<IpAddr>() {
        let is_private = match ip {
            IpAddr::V4(v4) => v4.is_private() || v4.is_loopback() || v4.is_link_local(),
            IpAddr::V6(v6) => v6.is_loopback(),
        };
        if is_private {
            return Err(format!("private/loopback IP '{}' is blocked", ip));
        }
    }

    Ok(())
}

fn html_to_markdown(html: &str) -> String {
    htmd::convert(html).unwrap_or_else(|_| html.to_string())
}

fn error(code: &str, message: impl Into<String>) -> Value {
    json!({ "status": "error", "code": code, "error": message.into() })
}
