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
    "169.254.169.254",          // AWS metadata
    "metadata.google.internal", // GCP metadata
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
        // Every hop is revalidated, not just the first and last. A redirect
        // chain that passes through a link-local or metadata address is the
        // classic SSRF path, and `Policy::limited` gives no hook to see it.
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= MAX_REDIRECTS {
                return attempt.error("too many redirects");
            }
            match check_blocked_host(attempt.url()) {
                Ok(()) => attempt.follow(),
                Err(reason) => attempt.error(format!("redirect to blocked host: {reason}")),
            }
        }))
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
    if let Ok(final_parsed) = reqwest::Url::parse(&final_url)
        && let Err(msg) = check_blocked_host(&final_parsed)
    {
        return error(
            "blocked_domain",
            format!("redirect to blocked host: {}", msg),
        );
    }

    let body_bytes = match response.bytes().await {
        Ok(b) => b,
        Err(e) => return error("http_error", format!("failed to read body: {}", e)),
    };

    let bytes = body_bytes.len();
    let body = String::from_utf8_lossy(&body_bytes[..bytes.min(MAX_CONTENT_BYTES)]);

    // Convert based on content type
    let result = if content_type.contains("text/html") || content_type.contains("application/xhtml")
    {
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
    let result = crate::text::truncate(
        &result,
        MAX_CONTENT_BYTES,
        &format!("...\n[truncated at {}KB]", MAX_CONTENT_BYTES / 1024),
    );

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
    let raw_host = url.host_str().unwrap_or("");

    // `host_str` keeps the brackets on an IPv6 literal, so `"[::1]".parse()`
    // fails and the address check was silently skipped for every IPv6 target.
    let host = raw_host
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .unwrap_or(raw_host);

    if BLOCKED_HOSTS.iter().any(|b| host.eq_ignore_ascii_case(b)) {
        return Err(format!("host '{}' is blocked", host));
    }

    // Block addresses that are not routable on the public internet. The v4 set
    // adds shared address space (CGNAT) and the v6 set adds unique-local and
    // link-local, all of which reach infrastructure a fetch tool must not.
    if let Ok(ip) = host.parse::<IpAddr>() {
        if is_non_public_address(ip) {
            return Err(format!("private/loopback IP '{}' is blocked", ip));
        }
    }

    Ok(())
}

/// True when an address is not routable on the public internet.
fn is_non_public_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let octets = v4.octets();
            v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_unspecified()
                || v4.is_documentation()
                || v4.is_multicast()
                // 100.64.0.0/10 — carrier-grade NAT shared address space.
                || (octets[0] == 100 && (64..128).contains(&octets[1]))
                // 192.0.0.0/24 — IETF protocol assignments.
                || (octets[0] == 192 && octets[1] == 0 && octets[2] == 0)
                // 198.18.0.0/15 — benchmarking.
                || (octets[0] == 198 && (18..20).contains(&octets[1]))
        }
        IpAddr::V6(v6) => {
            let segments = v6.segments();
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                // fc00::/7 — unique local addresses.
                || (segments[0] & 0xfe00) == 0xfc00
                // fe80::/10 — link-local.
                || (segments[0] & 0xffc0) == 0xfe80
                // IPv4-mapped addresses inherit the v4 verdict.
                || v6
                    .to_ipv4_mapped()
                    .is_some_and(|mapped| is_non_public_address(IpAddr::V4(mapped)))
        }
    }
}

fn html_to_markdown(html: &str) -> String {
    htmd::convert(html).unwrap_or_else(|_| html.to_string())
}

fn error(code: &str, message: impl Into<String>) -> Value {
    json!({ "status": "error", "code": code, "error": message.into() })
}

#[cfg(test)]
mod ssrf_tests {
    use super::*;

    fn blocked(host: &str) -> bool {
        let url = reqwest::Url::parse(&format!("http://{host}/")).expect("valid url");
        check_blocked_host(&url).is_err()
    }

    #[test]
    fn named_metadata_and_loopback_hosts_are_blocked() {
        for host in BLOCKED_HOSTS {
            // A bare IPv6 literal needs brackets to be a valid URL authority.
            let authority = if host.contains(':') && !host.starts_with('[') {
                format!("[{host}]")
            } else {
                (*host).to_string()
            };
            assert!(blocked(&authority), "'{host}' should be blocked");
        }
    }

    #[test]
    fn private_and_loopback_ipv4_is_blocked() {
        for host in [
            "127.0.0.1",
            "10.0.0.1",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.169.254", // cloud metadata
            "0.0.0.0",
        ] {
            assert!(blocked(host), "'{host}' should be blocked");
        }
    }

    /// Ranges the original lexical check missed.
    #[test]
    fn shared_and_reserved_ranges_are_blocked() {
        for host in [
            "100.64.0.1", // CGNAT
            "192.0.0.1",  // IETF protocol assignments
            "198.18.0.1", // benchmarking
            "224.0.0.1",  // multicast
        ] {
            assert!(blocked(host), "'{host}' should be blocked");
        }
    }

    #[test]
    fn non_public_ipv6_is_blocked() {
        for host in [
            "[::1]",              // loopback
            "[fc00::1]",          // unique local
            "[fe80::1]",          // link local
            "[::]",               // unspecified
            "[::ffff:127.0.0.1]", // IPv4-mapped loopback
        ] {
            assert!(blocked(host), "'{host}' should be blocked");
        }
    }

    #[test]
    fn public_addresses_are_allowed() {
        for host in ["example.com", "93.184.216.34", "[2606:2800:220:1::1]"] {
            assert!(!blocked(host), "'{host}' should be allowed");
        }
    }
}
