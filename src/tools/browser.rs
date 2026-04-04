/// Browser automation tools via adk-browser (feature-gated: `browser`).
///
/// Requires a WebDriver server (ChromeDriver, geckodriver, or Selenium).
/// Default: http://localhost:4444
use adk_rust::prelude::*;
use std::sync::Arc;
use tokio::sync::OnceCell;

static BROWSER: OnceCell<Arc<adk_browser::BrowserSession>> = OnceCell::const_new();

/// Get or start the shared browser session (lazy, headless).
pub async fn get_browser() -> anyhow::Result<Arc<adk_browser::BrowserSession>> {
    BROWSER
        .get_or_try_init(|| async {
            let config = adk_browser::BrowserConfig::new().headless(true);
            let session = Arc::new(adk_browser::BrowserSession::new(config));
            session.start().await.map_err(|e| anyhow::anyhow!("browser start failed: {e}"))?;
            tracing::info!("Browser session started (headless)");
            Ok(session)
        })
        .await
        .cloned()
}

/// Build all browser tools (lazy session init on first use).
pub fn build_browser_tools(session: Arc<adk_browser::BrowserSession>) -> Vec<Arc<dyn Tool>> {
    let toolset = adk_browser::BrowserToolset::new(session);
    toolset.all_tools()
}

/// Cleanup browser session on exit.
pub async fn cleanup_browser() {
    if let Some(session) = BROWSER.get() {
        if let Err(e) = session.stop().await {
            tracing::warn!("Browser cleanup failed: {e}");
        }
    }
}
