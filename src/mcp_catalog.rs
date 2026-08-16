use std::path::Path;

use anyhow::{Context, Result};
use serde::Serialize;
use toml_edit::{Array, ArrayOfTables, DocumentMut, Item, Table, value};

use crate::capabilities::CapabilityCategory;

#[derive(Debug, Clone, Serialize)]
pub struct McpCatalogEntry {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub category: CapabilityCategory,
    pub command: &'static str,
    pub args: &'static [&'static str],
    pub install: &'static str,
}

const CATALOG: &[McpCatalogEntry] = &[
    McpCatalogEntry {
        id: "docx-mcp",
        name: "Word documents",
        description: "Create, read, edit, template, and export DOCX documents.",
        category: CapabilityCategory::Productivity,
        command: "docx-mcp-server",
        args: &[],
        install: "cargo install docx-mcp-server",
    },
    McpCatalogEntry {
        id: "mcp-slides",
        name: "PowerPoint slides",
        description: "Create, edit, inspect, and export PPTX presentations.",
        category: CapabilityCategory::Productivity,
        command: "slides-mcp-server",
        args: &[],
        install: "cargo install slides-mcp-server",
    },
    McpCatalogEntry {
        id: "worksheet-mcp",
        name: "Excel workbooks",
        description: "Create, edit, calculate, and inspect XLSX workbooks.",
        category: CapabilityCategory::Productivity,
        command: "excel-mcp-server",
        args: &[],
        install: "cargo install excel-mcp-server",
    },
    McpCatalogEntry {
        id: "mcp-pdf",
        name: "PDF operations",
        description: "Inspect, extract, create, convert, secure, and fill PDFs.",
        category: CapabilityCategory::Productivity,
        command: "mcp-pdf",
        args: &[],
        install: "cargo install mcp-pdf",
    },
    McpCatalogEntry {
        id: "mcp-email",
        name: "Email",
        description: "Read and send email through IMAP, SMTP, SES, or SendGrid.",
        category: CapabilityCategory::Productivity,
        command: "mcp-email",
        args: &[],
        install: "cargo install mcp-email",
    },
    McpCatalogEntry {
        id: "mcp-calendar",
        name: "Calendar",
        description: "Read and manage calendar events through configured backends.",
        category: CapabilityCategory::Productivity,
        command: "mcp-calendar",
        args: &[],
        install: "cargo install mcp-calendar",
    },
    McpCatalogEntry {
        id: "mcp-search",
        name: "Web search",
        description: "Search current web sources through configured providers.",
        category: CapabilityCategory::Research,
        command: "mcp-search",
        args: &[],
        install: "cargo install mcp-search",
    },
    McpCatalogEntry {
        id: "mcp-browser",
        name: "Browser",
        description: "Navigate and extract web content through an MCP browser.",
        category: CapabilityCategory::Research,
        command: "mcp-browser",
        args: &[],
        install: "cargo install mcp-browser",
    },
    McpCatalogEntry {
        id: "mcp-code-search",
        name: "Code search",
        description: "Search repositories and inspect dependency graphs.",
        category: CapabilityCategory::Development,
        command: "mcp-code-search",
        args: &[],
        install: "cargo install mcp-code-search",
    },
    McpCatalogEntry {
        id: "mcp-test-runner",
        name: "Test runner",
        description: "Discover, execute, and report project tests.",
        category: CapabilityCategory::Development,
        command: "mcp-test-runner",
        args: &[],
        install: "cargo install mcp-test-runner",
    },
    McpCatalogEntry {
        id: "mcp-security-advisory",
        name: "Security advisories",
        description: "Inspect dependency and ecosystem security advisories.",
        category: CapabilityCategory::Development,
        command: "mcp-security-advisory",
        args: &[],
        install: "cargo install mcp-security-advisory",
    },
    McpCatalogEntry {
        id: "computer-use-mcp",
        name: "Computer use",
        description: "Automate desktop applications with native cross-platform controls.",
        category: CapabilityCategory::Operations,
        command: "npx",
        args: &["--yes", "--prefer-offline", "@zavora-ai/computer-use-mcp"],
        install: "npm install --global @zavora-ai/computer-use-mcp",
    },
    McpCatalogEntry {
        id: "mcp-device-management",
        name: "Device management",
        description: "Inspect health, services, processes, packages, and updates on devices.",
        category: CapabilityCategory::Operations,
        command: "mcp-device-management",
        args: &[],
        install: "cargo install mcp-device-management",
    },
    McpCatalogEntry {
        id: "mcp-registry",
        name: "MCP registry",
        description: "Register, discover, health-check, allow-list, and audit MCP servers.",
        category: CapabilityCategory::Platform,
        command: "mcp-registry",
        args: &[],
        install: "cargo install mcp-registry",
    },
];

pub fn entries() -> &'static [McpCatalogEntry] {
    CATALOG
}

pub fn find_entry(id: &str) -> Option<&'static McpCatalogEntry> {
    CATALOG.iter().find(|entry| entry.id == id)
}

fn command_available(command: &str) -> bool {
    let path = Path::new(command);
    if path.components().count() > 1 {
        return path.is_file();
    }
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|directory| {
            let candidate = directory.join(command);
            if candidate.is_file() {
                return true;
            }
            #[cfg(windows)]
            {
                return ["exe", "cmd", "bat"]
                    .iter()
                    .any(|extension| candidate.with_extension(extension).is_file());
            }
            #[cfg(not(windows))]
            false
        })
    })
}

fn read_document(path: &Path) -> Result<DocumentMut> {
    if !path.exists() {
        return Ok(DocumentMut::new());
    }
    std::fs::read_to_string(path)
        .with_context(|| format!("failed to read '{}'", path.display()))?
        .parse::<DocumentMut>()
        .with_context(|| format!("invalid TOML in '{}'", path.display()))
}

fn server_tables_mut<'a>(
    document: &'a mut DocumentMut,
    profile: &str,
) -> Result<&'a mut ArrayOfTables> {
    let profiles = document
        .entry("profiles")
        .or_insert(Item::Table(Table::new()))
        .as_table_mut()
        .context("'profiles' must be a TOML table")?;
    let profile_table = profiles
        .entry(profile)
        .or_insert(Item::Table(Table::new()))
        .as_table_mut()
        .with_context(|| format!("profile '{profile}' must be a TOML table"))?;
    profile_table
        .entry("mcp_servers")
        .or_insert(Item::ArrayOfTables(ArrayOfTables::new()))
        .as_array_of_tables_mut()
        .with_context(|| format!("profiles.{profile}.mcp_servers must be an array of tables"))
}

fn write_document(path: &Path, document: &DocumentMut) -> Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create '{}'", parent.display()))?;
    }
    std::fs::write(path, document.to_string())
        .with_context(|| format!("failed to write '{}'", path.display()))
}

pub fn add_server(path: &Path, profile: &str, id: &str) -> Result<bool> {
    let entry = find_entry(id).ok_or_else(|| {
        anyhow::anyhow!(
            "MCP server '{id}' is not in the curated catalog. Run 'zavora-cli mcp catalog'."
        )
    })?;
    let mut document = read_document(path)?;
    let servers = server_tables_mut(&mut document, profile)?;
    if servers
        .iter()
        .any(|table| table.get("name").and_then(Item::as_str) == Some(id))
    {
        return Ok(false);
    }

    let mut table = Table::new();
    table.insert("name", value(entry.id));
    table.insert("command", value(entry.command));
    if !entry.args.is_empty() {
        let mut args = Array::new();
        for argument in entry.args {
            args.push(*argument);
        }
        table.insert("args", value(args));
    }
    table.insert("enabled", value(true));
    servers.push(table);
    write_document(path, &document)?;
    Ok(true)
}

pub fn remove_server(path: &Path, profile: &str, id: &str) -> Result<bool> {
    let mut document = read_document(path)?;
    let servers = server_tables_mut(&mut document, profile)?;
    let position = servers
        .iter()
        .position(|table| table.get("name").and_then(Item::as_str) == Some(id));
    let Some(position) = position else {
        return Ok(false);
    };
    servers.remove(position);
    write_document(path, &document)?;
    Ok(true)
}

pub fn set_server_enabled(path: &Path, profile: &str, id: &str, enabled: bool) -> Result<bool> {
    let mut document = read_document(path)?;
    let servers = server_tables_mut(&mut document, profile)?;
    let server = servers
        .iter_mut()
        .find(|table| table.get("name").and_then(Item::as_str) == Some(id))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "MCP server '{id}' is not configured for profile '{profile}'. Run 'zavora-cli mcp add {id}' when it is in the catalog."
            )
        })?;
    let previous = server
        .get("enabled")
        .and_then(Item::as_bool)
        .unwrap_or(true);
    if previous == enabled {
        return Ok(false);
    }
    server.insert("enabled", value(enabled));
    write_document(path, &document)?;
    Ok(true)
}

pub fn run_catalog(query: &str, json: bool) -> Result<()> {
    let terms = query
        .split_whitespace()
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>();
    let matches = CATALOG
        .iter()
        .filter(|entry| {
            let haystack = format!(
                "{} {} {} {}",
                entry.id, entry.name, entry.description, entry.category
            )
            .to_ascii_lowercase();
            terms.iter().all(|term| {
                haystack.contains(term)
                    || (term == "office" && entry.category == CapabilityCategory::Productivity)
            })
        })
        .collect::<Vec<_>>();
    if json {
        let payload = matches
            .iter()
            .map(|entry| {
                serde_json::json!({
                    "id": entry.id,
                    "name": entry.name,
                    "description": entry.description,
                    "category": entry.category,
                    "command": entry.command,
                    "args": entry.args,
                    "installed": command_available(entry.command),
                    "install": entry.install,
                })
            })
            .collect::<Vec<_>>();
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(());
    }
    if matches.is_empty() {
        println!("No curated MCP servers matched '{query}'.");
        return Ok(());
    }
    for category in CapabilityCategory::ALL {
        let category_entries = matches
            .iter()
            .filter(|entry| entry.category == category)
            .collect::<Vec<_>>();
        if category_entries.is_empty() {
            continue;
        }
        println!("{}:", category.label());
        for entry in category_entries {
            println!(
                "  {} {:<24} {} — {}",
                if command_available(entry.command) {
                    "✓"
                } else {
                    "·"
                },
                entry.id,
                entry.name,
                entry.description
            );
            if !command_available(entry.command) {
                println!("      install: {}", entry.install);
            }
        }
    }
    println!("\n✓ command available · not found on PATH");
    Ok(())
}

pub fn run_add(path: &Path, profile: &str, id: &str) -> Result<()> {
    let entry = find_entry(id).ok_or_else(|| {
        anyhow::anyhow!(
            "MCP server '{id}' is not in the curated catalog. Run 'zavora-cli mcp catalog'."
        )
    })?;
    if add_server(path, profile, id)? {
        println!("Added '{id}' to profile '{profile}' in {}.", path.display());
    } else {
        println!("MCP server '{id}' is already configured for profile '{profile}'.");
    }
    if !command_available(entry.command) {
        println!(
            "The server is configured but not installed. Run: {}",
            entry.install
        );
    }
    println!("Verify with: zavora-cli mcp doctor --server {id}");
    Ok(())
}

pub fn run_remove(path: &Path, profile: &str, id: &str) -> Result<()> {
    if remove_server(path, profile, id)? {
        println!(
            "Removed '{id}' from profile '{profile}' in {}.",
            path.display()
        );
    } else {
        println!("MCP server '{id}' is not configured for profile '{profile}'.");
    }
    Ok(())
}

pub fn run_set_enabled(path: &Path, profile: &str, id: &str, enabled: bool) -> Result<()> {
    let changed = set_server_enabled(path, profile, id, enabled)?;
    let state = if enabled { "enabled" } else { "disabled" };
    if changed {
        println!(
            "MCP server '{id}' {state} for profile '{profile}' in {}.",
            path.display()
        );
    } else {
        println!("MCP server '{id}' was already {state} for profile '{profile}'.");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn catalog_covers_the_requested_work_capabilities() {
        for id in [
            "docx-mcp",
            "mcp-slides",
            "worksheet-mcp",
            "mcp-pdf",
            "mcp-email",
            "computer-use-mcp",
            "mcp-device-management",
        ] {
            assert!(find_entry(id).is_some(), "missing catalog entry {id}");
        }
    }

    #[test]
    fn add_and_remove_preserve_other_profile_content() {
        let temp = tempfile::tempdir().expect("temp directory");
        let path: PathBuf = temp.path().join("config.toml");
        std::fs::write(
            &path,
            "# keep this comment\n[profiles.default]\nworker_model = \"test-model\"\n",
        )
        .expect("seed config");

        assert!(add_server(&path, "default", "docx-mcp").expect("add"));
        assert!(!add_server(&path, "default", "docx-mcp").expect("idempotent add"));
        let configured = std::fs::read_to_string(&path).expect("read config");
        assert!(configured.contains("# keep this comment"));
        assert!(configured.contains("worker_model = \"test-model\""));
        assert!(configured.contains("command = \"docx-mcp-server\""));

        assert!(set_server_enabled(&path, "default", "docx-mcp", false).expect("disable"));
        assert!(!set_server_enabled(&path, "default", "docx-mcp", false).expect("idempotent"));
        assert!(set_server_enabled(&path, "default", "docx-mcp", true).expect("enable"));

        assert!(remove_server(&path, "default", "docx-mcp").expect("remove"));
        assert!(!remove_server(&path, "default", "docx-mcp").expect("idempotent remove"));
    }
}
