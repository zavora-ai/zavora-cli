//! First-party capabilities that ship with Zavora or have a managed companion.
//!
//! A catalogue recipe is not an installation. This module is the distribution
//! boundary: built-ins are linked into the Zavora executable, while companions
//! must resolve to a concrete executable before they are reported as installed.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{Context, Result, bail};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum EssentialDelivery {
    BuiltIn,
    ManagedCompanion,
}

impl std::fmt::Display for EssentialDelivery {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::BuiltIn => "built-in",
            Self::ManagedCompanion => "managed-companion",
        })
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct EssentialCapability {
    pub id: &'static str,
    pub name: &'static str,
    pub delivery: EssentialDelivery,
    pub executable: &'static str,
    pub package: &'static str,
    pub version: &'static str,
    pub requires_authorization: bool,
}

const ESSENTIALS: &[EssentialCapability] = &[
    EssentialCapability {
        id: "docx-mcp",
        name: "Word documents",
        delivery: EssentialDelivery::BuiltIn,
        executable: "docx-mcp-server",
        package: "docx-mcp-server",
        version: "2.2.0",
        requires_authorization: false,
    },
    EssentialCapability {
        id: "mcp-slides",
        name: "PowerPoint slides",
        delivery: EssentialDelivery::BuiltIn,
        executable: "slides-mcp-server",
        package: "slides-mcp-server",
        version: "0.1.0",
        requires_authorization: false,
    },
    EssentialCapability {
        id: "worksheet-mcp",
        name: "Excel workbooks",
        delivery: EssentialDelivery::BuiltIn,
        executable: "excel-mcp-server",
        package: "excel-mcp-server",
        version: "0.2.2",
        requires_authorization: false,
    },
    EssentialCapability {
        id: "mcp-pdf",
        name: "PDF operations",
        delivery: EssentialDelivery::BuiltIn,
        executable: "mcp-pdf",
        package: "mcp-pdf",
        version: "3.1.0",
        requires_authorization: false,
    },
    EssentialCapability {
        id: "computer-use-mcp",
        name: "Computer use",
        delivery: EssentialDelivery::ManagedCompanion,
        executable: "computer-use-mcp",
        package: "@zavora-ai/computer-use-mcp",
        version: "7.1.0",
        requires_authorization: true,
    },
    EssentialCapability {
        id: "mcp-device-management",
        name: "Device management",
        delivery: EssentialDelivery::ManagedCompanion,
        executable: "mcp-device-management",
        package: "mcp-device-management",
        version: "1.7.0",
        requires_authorization: true,
    },
];

#[derive(Debug, Clone, Serialize)]
pub struct EssentialStatus {
    pub id: &'static str,
    pub name: &'static str,
    pub delivery: EssentialDelivery,
    pub installed: bool,
    pub resolved_path: Option<PathBuf>,
    pub configured: bool,
    /// Authorization is intentionally unknown until the live server proves it.
    pub requires_authorization: bool,
}

pub fn entries() -> &'static [EssentialCapability] {
    ESSENTIALS
}

pub fn find(id: &str) -> Option<&'static EssentialCapability> {
    ESSENTIALS.iter().find(|entry| entry.id == id)
}

pub fn is_essential(id: &str) -> bool {
    find(id).is_some()
}

pub fn is_installed(id: &str) -> bool {
    find(id).is_some_and(|entry| match entry.delivery {
        EssentialDelivery::BuiltIn => cfg!(feature = "bundled-artifacts"),
        EssentialDelivery::ManagedCompanion => resolve_companion(entry).is_some(),
    })
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

pub fn managed_root() -> PathBuf {
    std::env::var_os("ZAVORA_ESSENTIALS_DIR")
        .map(PathBuf::from)
        .or_else(|| home_dir().map(|home| home.join(".zavora/essentials")))
        .unwrap_or_else(|| PathBuf::from(".zavora/essentials"))
}

fn executable_on_path(name: &str) -> Option<PathBuf> {
    if name.contains(std::path::MAIN_SEPARATOR) {
        return Path::new(name).is_file().then(|| PathBuf::from(name));
    }
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths).find_map(|directory| {
            let candidate = directory.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
            #[cfg(windows)]
            for extension in ["exe", "cmd", "bat"] {
                let candidate = candidate.with_extension(extension);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
            None
        })
    })
}

fn release_companion(entry: &EssentialCapability) -> Option<PathBuf> {
    let executable = std::env::current_exe().ok()?;
    let parent = executable.parent()?;
    let package_root = parent.parent();
    let candidates = [
        parent.join("libexec/zavora-cli").join(entry.executable),
        parent
            .join("libexec/zavora-cli/node_modules/.bin")
            .join(entry.executable),
        parent.join("essentials").join(entry.executable),
        parent.join(entry.executable),
        package_root
            .map(|root| root.join("node_modules/.bin").join(entry.executable))
            .unwrap_or_default(),
    ];
    candidates.into_iter().find(|candidate| candidate.is_file())
}

fn managed_companion(entry: &EssentialCapability) -> Option<PathBuf> {
    let root = managed_root();
    let candidates = [
        root.join("bin").join(entry.executable),
        root.join("node_modules/.bin").join(entry.executable),
        root.join(entry.executable),
    ];
    candidates.into_iter().find(|candidate| candidate.is_file())
}

/// Resolve a companion from a release payload, Zavora's managed prefix, or PATH.
/// The returned path is runtime evidence of installation; the package-manager
/// executable itself is never accepted as evidence for the package.
pub fn resolve_companion(entry: &EssentialCapability) -> Option<PathBuf> {
    release_companion(entry)
        .or_else(|| managed_companion(entry))
        .or_else(|| executable_on_path(entry.executable))
}

pub fn statuses(configured_servers: &[String]) -> Vec<EssentialStatus> {
    ESSENTIALS
        .iter()
        .map(|entry| {
            let resolved_path = match entry.delivery {
                EssentialDelivery::BuiltIn if cfg!(feature = "bundled-artifacts") => {
                    std::env::current_exe().ok()
                }
                EssentialDelivery::BuiltIn => None,
                EssentialDelivery::ManagedCompanion => resolve_companion(entry),
            };
            EssentialStatus {
                id: entry.id,
                name: entry.name,
                delivery: entry.delivery,
                installed: resolved_path.is_some(),
                resolved_path,
                configured: configured_servers
                    .iter()
                    .any(|server| server.eq_ignore_ascii_case(entry.id)),
                requires_authorization: entry.requires_authorization,
            }
        })
        .collect()
}

pub fn run_status(configured_servers: &[String], json: bool) -> Result<()> {
    let statuses = statuses(configured_servers);
    if json {
        println!("{}", serde_json::to_string_pretty(&statuses)?);
        return Ok(());
    }
    print!("{}", format_status_markdown(configured_servers));
    Ok(())
}

pub fn format_status_markdown(configured_servers: &[String]) -> String {
    let statuses = statuses(configured_servers);
    let mut output = "## Essential capabilities\n\n".to_string();
    for status in statuses {
        output.push_str(&format!(
            "- {} **{}** (`{}`) — {} · installed={} · configured={}{}\n",
            if status.installed { "✓" } else { "·" },
            status.name,
            status.id,
            status.delivery,
            status.installed,
            status.configured,
            if status.requires_authorization {
                " · authorization requires live check"
            } else {
                ""
            }
        ));
    }
    output.push_str(
        "\n_Installed is verified locally. Configured is not connected or authorized; use `zavora-cli mcp doctor`._\n",
    );
    output
}

/// Command and arguments written into MCP configuration for an essential.
/// The current executable is used instead of PATH so npm, Homebrew, release,
/// and cargo installations all launch the same audited build.
pub fn launcher(id: &str) -> Result<(String, Vec<String>)> {
    if !is_essential(id) {
        bail!("'{id}' is not an essential capability");
    }
    let executable = std::env::current_exe().context("failed to resolve the Zavora executable")?;
    Ok((
        executable.display().to_string(),
        vec!["essentials".into(), "serve".into(), id.into()],
    ))
}

pub async fn serve(id: &str) -> Result<()> {
    let entry = find(id).with_context(|| format!("essential capability '{id}' was not found"))?;
    match entry.delivery {
        EssentialDelivery::BuiltIn => serve_builtin(id).await,
        EssentialDelivery::ManagedCompanion => {
            let executable = resolve_companion(entry).with_context(|| {
                format!(
                    "essential companion '{}' is not installed; inspect with `zavora-cli essentials status`",
                    entry.id
                )
            })?;
            let status = tokio::process::Command::new(&executable)
                .stdin(Stdio::inherit())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .status()
                .await
                .with_context(|| format!("failed to launch '{}'", executable.display()))?;
            if !status.success() {
                bail!("essential companion '{}' exited with {status}", entry.id);
            }
            Ok(())
        }
    }
}

#[cfg(feature = "bundled-artifacts")]
async fn serve_builtin(id: &str) -> Result<()> {
    use rmcp::{ServiceExt, transport::stdio};
    match id {
        "docx-mcp" => {
            docx_mcp_server::DocxServer::new()
                .serve(stdio())
                .await?
                .waiting()
                .await?;
        }
        "mcp-slides" => {
            slides_mcp_server::SlidesServer::new()
                .serve(stdio())
                .await?
                .waiting()
                .await?;
        }
        "worksheet-mcp" => {
            use std::sync::Arc;
            use tokio::sync::RwLock;

            let store = Arc::new(RwLock::new(excel_mcp_server::store::WorkbookStore::new()));
            excel_mcp_server::server::ExcelMcpServer::new(store)
                .serve(stdio())
                .await?
                .waiting()
                .await?;
        }
        "mcp-pdf" => {
            mcp_pdf::server::PdfServer
                .serve(stdio())
                .await?
                .waiting()
                .await?;
        }
        _ => bail!("'{id}' is not a built-in essential server"),
    }
    Ok(())
}

#[cfg(not(feature = "bundled-artifacts"))]
async fn serve_builtin(id: &str) -> Result<()> {
    bail!(
        "essential server '{id}' is not present because this build disabled the `bundled-artifacts` feature"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn essentials_have_unique_catalog_ids() {
        let ids = ESSENTIALS
            .iter()
            .map(|entry| entry.id)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(ids.len(), ESSENTIALS.len());
        assert!(
            ESSENTIALS
                .iter()
                .all(|entry| crate::mcp_catalog::find_entry(entry.id).is_some())
        );
    }

    #[test]
    fn builtin_installation_is_compile_time_truth() {
        for id in ["docx-mcp", "mcp-slides", "worksheet-mcp", "mcp-pdf"] {
            assert_eq!(is_installed(id), cfg!(feature = "bundled-artifacts"));
        }
    }

    #[test]
    fn authorization_is_not_inferred_from_installation() {
        let status = statuses(&[])
            .into_iter()
            .find(|status| status.id == "computer-use-mcp")
            .expect("computer use status");
        assert!(status.requires_authorization);
        assert!(!status.configured);
    }
}
