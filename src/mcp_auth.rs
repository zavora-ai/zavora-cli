//! MCP OAuth 2.0 with PKCE for authenticated MCP servers.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// OAuth config for an MCP server.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct McpOAuthConfig {
    pub client_id: Option<String>,
    pub callback_port: Option<u16>,
    pub auth_server_metadata_url: Option<String>,
}

/// Stored OAuth tokens.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct OAuthTokens {
    access_token: String,
    refresh_token: Option<String>,
    expires_at: Option<u64>,
}

/// Get a valid access token for an MCP server, refreshing if needed.
#[cfg(feature = "oauth")]
pub async fn get_access_token(server_name: &str, oauth: &McpOAuthConfig) -> Result<String> {
    // Try cached token first
    if let Some(tokens) = load_tokens(server_name) {
        if !is_expired(&tokens) {
            return Ok(tokens.access_token);
        }
        // Try refresh
        if let Some(ref refresh) = tokens.refresh_token {
            if let Ok(new_tokens) = refresh_token(oauth, refresh).await {
                save_tokens(server_name, &new_tokens);
                return Ok(new_tokens.access_token);
            }
        }
    }

    // Full auth flow
    let tokens = run_auth_flow(oauth).await?;
    save_tokens(server_name, &tokens);
    Ok(tokens.access_token)
}

#[cfg(feature = "oauth")]
fn is_expired(tokens: &OAuthTokens) -> bool {
    let Some(expires_at) = tokens.expires_at else {
        return false;
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // Refresh 5 minutes before expiry
    now + 300 >= expires_at
}

// ---------------------------------------------------------------------------
// PKCE auth flow
// ---------------------------------------------------------------------------

#[cfg(feature = "oauth")]
async fn run_auth_flow(oauth: &McpOAuthConfig) -> Result<OAuthTokens> {
    use rand::Rng;
    use sha2::{Digest, Sha256};

    // Discover auth server metadata
    let metadata = discover_metadata(oauth).await?;

    // Generate PKCE
    let verifier: String = rand::rng()
        .sample_iter(&rand::distr::Alphanumeric)
        .take(64)
        .map(char::from)
        .collect();
    let challenge = {
        let hash = Sha256::digest(verifier.as_bytes());
        base64_url_encode(&hash)
    };

    let port = oauth.callback_port.unwrap_or(8912);
    let redirect_uri = format!("http://localhost:{}/callback", port);
    let client_id = oauth.client_id.as_deref().unwrap_or("zavora-cli");

    // Build authorization URL
    let auth_url = format!(
        "{}?response_type=code&client_id={}&redirect_uri={}&code_challenge={}&code_challenge_method=S256",
        metadata.authorization_endpoint, client_id, redirect_uri, challenge
    );

    // Open browser
    println!("Opening browser for authorization...");
    let _ = open::that(&auth_url);
    println!("If browser didn't open, visit:\n{}\n", auth_url);

    // Listen for callback
    let code = listen_for_callback(port).await?;

    // Exchange code for tokens
    let client = reqwest::Client::new();
    let resp = client
        .post(&metadata.token_endpoint)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("redirect_uri", &redirect_uri),
            ("client_id", client_id),
            ("code_verifier", &verifier),
        ])
        .send()
        .await
        .context("token exchange failed")?;

    let body: serde_json::Value = resp.json().await.context("invalid token response")?;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    Ok(OAuthTokens {
        access_token: body["access_token"]
            .as_str()
            .context("missing access_token")?
            .to_string(),
        refresh_token: body["refresh_token"].as_str().map(String::from),
        expires_at: body["expires_in"].as_u64().map(|secs| now + secs),
    })
}

#[cfg(feature = "oauth")]
async fn refresh_token(oauth: &McpOAuthConfig, refresh: &str) -> Result<OAuthTokens> {
    let metadata = discover_metadata(oauth).await?;
    let client_id = oauth.client_id.as_deref().unwrap_or("zavora-cli");

    let client = reqwest::Client::new();
    let resp = client
        .post(&metadata.token_endpoint)
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh),
            ("client_id", client_id),
        ])
        .send()
        .await?;

    let body: serde_json::Value = resp.json().await?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    Ok(OAuthTokens {
        access_token: body["access_token"]
            .as_str()
            .context("missing access_token")?
            .to_string(),
        refresh_token: body["refresh_token"]
            .as_str()
            .map(String::from)
            .or_else(|| Some(refresh.to_string())),
        expires_at: body["expires_in"].as_u64().map(|secs| now + secs),
    })
}

// ---------------------------------------------------------------------------
// Auth server metadata discovery
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct AuthMetadata {
    authorization_endpoint: String,
    token_endpoint: String,
}

#[cfg(feature = "oauth")]
async fn discover_metadata(oauth: &McpOAuthConfig) -> Result<AuthMetadata> {
    let url = oauth
        .auth_server_metadata_url
        .as_deref()
        .context("auth_server_metadata_url is required for OAuth")?;

    let resp = reqwest::get(url).await.context("metadata discovery failed")?;
    resp.json::<AuthMetadata>()
        .await
        .context("invalid auth server metadata")
}

// ---------------------------------------------------------------------------
// Callback listener (temporary HTTP server on localhost)
// ---------------------------------------------------------------------------

#[cfg(feature = "oauth")]
async fn listen_for_callback(port: u16) -> Result<String> {
    use tokio::net::TcpListener;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = TcpListener::bind(format!("127.0.0.1:{}", port))
        .await
        .with_context(|| format!("failed to bind callback listener on port {}", port))?;

    println!("Waiting for authorization callback on port {}...", port);

    let (mut stream, _) = tokio::time::timeout(
        std::time::Duration::from_secs(120),
        listener.accept(),
    )
    .await
    .context("authorization timed out (2 minutes)")?
    .context("accept failed")?;

    let mut buf = vec![0u8; 4096];
    let n = stream.read(&mut buf).await?;
    let request = String::from_utf8_lossy(&buf[..n]);

    // Extract code from GET /callback?code=XXX
    let code = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|path| {
            url::form_urlencoded::parse(path.split('?').nth(1).unwrap_or("").as_bytes())
                .find(|(k, _)| k == "code")
                .map(|(_, v)| v.to_string())
        })
        .context("no authorization code in callback")?;

    // Send response
    let response = "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n<html><body><h2>Authorization successful!</h2><p>You can close this tab.</p></body></html>";
    let _ = stream.write_all(response.as_bytes()).await;

    Ok(code)
}

// ---------------------------------------------------------------------------
// Token storage (OS keychain via keyring crate)
// ---------------------------------------------------------------------------

#[cfg(feature = "oauth")]
fn save_tokens(server_name: &str, tokens: &OAuthTokens) {
    let key = format!("zavora-mcp-{}", server_name);
    if let Ok(json) = serde_json::to_string(tokens) {
        match keyring::Entry::new(&key, "oauth-tokens") {
            Ok(entry) => { let _ = entry.set_password(&json); }
            Err(_) => {
                // Fallback: write to .zavora/tokens/<server>.json
                let _ = save_tokens_file(server_name, &json);
            }
        }
    }
}

#[cfg(feature = "oauth")]
fn load_tokens(server_name: &str) -> Option<OAuthTokens> {
    let key = format!("zavora-mcp-{}", server_name);
    let json = keyring::Entry::new(&key, "oauth-tokens")
        .ok()
        .and_then(|e| e.get_password().ok())
        .or_else(|| load_tokens_file(server_name))?;
    serde_json::from_str(&json).ok()
}

#[cfg(feature = "oauth")]
fn save_tokens_file(server_name: &str, json: &str) -> Result<()> {
    let dir = std::path::Path::new(".zavora/tokens");
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join(format!("{}.json", server_name)), json)?;
    Ok(())
}

#[cfg(feature = "oauth")]
fn load_tokens_file(server_name: &str) -> Option<String> {
    std::fs::read_to_string(format!(".zavora/tokens/{}.json", server_name)).ok()
}

#[cfg(feature = "oauth")]
fn base64_url_encode(data: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(data)
}
