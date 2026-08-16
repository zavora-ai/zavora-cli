use std::path::Path;

use std::collections::BTreeSet;

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
    McpCatalogEntry {
        id: "mcp-flowchart",
        name: "Diagrams",
        description: "Generate SVG, Mermaid, and PlantUML diagrams from a description.",
        category: CapabilityCategory::Productivity,
        command: "adk-rust-mcp-diagrams",
        args: &[],
        install: "cargo install adk-rust-mcp-diagrams",
    },
    McpCatalogEntry {
        id: "mcp-slack",
        name: "Slack",
        description: "Send messages, manage channels, search, react, and upload files.",
        category: CapabilityCategory::Productivity,
        command: "mcp-slack",
        args: &[],
        install: "cargo install mcp-slack",
    },
    McpCatalogEntry {
        id: "mcp-messaging",
        name: "Messaging",
        description: "Send SMS, push, webhook, and in-app messages through configured providers.",
        category: CapabilityCategory::Productivity,
        command: "mcp-messaging",
        args: &[],
        install: "cargo install mcp-messaging",
    },
    McpCatalogEntry {
        id: "mcp-notifications",
        name: "Notifications",
        description: "Deliver push, SMS, email, and in-app notifications with templates and tracking.",
        category: CapabilityCategory::Productivity,
        command: "mcp-notifications",
        args: &[],
        install: "cargo install mcp-notifications",
    },
    McpCatalogEntry {
        id: "mcp-task",
        name: "Tasks",
        description: "Track task lists, status workflow, priorities, and assignments.",
        category: CapabilityCategory::Productivity,
        command: "mcp-task",
        args: &[],
        install: "cargo install mcp-task",
    },
    McpCatalogEntry {
        id: "mcp-project",
        name: "Projects",
        description: "Manage projects, work items, sprints, milestones, dependencies, and time.",
        category: CapabilityCategory::Productivity,
        command: "mcp-project",
        args: &[],
        install: "cargo install mcp-project",
    },
    McpCatalogEntry {
        id: "mcp-scheduling",
        name: "Scheduling",
        description: "Book appointments, shifts, and resources with availability and recurrence.",
        category: CapabilityCategory::Productivity,
        command: "mcp-scheduling",
        args: &[],
        install: "cargo install mcp-scheduling",
    },
    McpCatalogEntry {
        id: "mcp-workflow",
        name: "Workflows",
        description: "Run state machines, approvals, case routing, and work orders.",
        category: CapabilityCategory::Productivity,
        command: "mcp-workflow",
        args: &[],
        install: "cargo install mcp-workflow",
    },
    McpCatalogEntry {
        id: "mcp-forms",
        name: "Forms",
        description: "Build forms with validation and conditional logic, and read submissions.",
        category: CapabilityCategory::Productivity,
        command: "mcp-forms",
        args: &[],
        install: "cargo install mcp-forms",
    },
    McpCatalogEntry {
        id: "mcp-survey",
        name: "Surveys",
        description: "Author, distribute, and analyse surveys with branching questions.",
        category: CapabilityCategory::Productivity,
        command: "mcp-survey",
        args: &[],
        install: "cargo install mcp-survey",
    },
    McpCatalogEntry {
        id: "mcp-github",
        name: "GitHub",
        description: "Interact with GitHub repositories, issues, and pull requests.",
        category: CapabilityCategory::Development,
        command: "mcp-github",
        args: &[],
        install: "cargo install mcp-github",
    },
    McpCatalogEntry {
        id: "mcp-package-registry",
        name: "Package registries",
        description: "Look up dependency metadata, versions, advisories, and changelogs.",
        category: CapabilityCategory::Development,
        command: "mcp-package-registry",
        args: &[],
        install: "cargo install mcp-package-registry",
    },
    McpCatalogEntry {
        id: "mcp-cicd",
        name: "CI/CD",
        description: "Inspect pipeline status, logs, and artifacts, and trigger reruns and deployments.",
        category: CapabilityCategory::Development,
        command: "mcp-cicd",
        args: &[],
        install: "cargo install mcp-cicd",
    },
    McpCatalogEntry {
        id: "mcp-containers",
        name: "Containers",
        description: "Manage Docker containers, images, volumes, networks, exec, and logs.",
        category: CapabilityCategory::Development,
        command: "mcp-containers",
        args: &[],
        install: "cargo install mcp-containers",
    },
    McpCatalogEntry {
        id: "mcp-database",
        name: "Databases",
        description: "Query, inspect schema, run migrations, and read explain plans.",
        category: CapabilityCategory::Development,
        command: "mcp-database",
        args: &[],
        install: "cargo install mcp-database",
    },
    McpCatalogEntry {
        id: "mcp-infrastructure",
        name: "Infrastructure",
        description: "Deploy and scale cloud resources, DNS, serverless, and secrets.",
        category: CapabilityCategory::Development,
        command: "mcp-infrastructure",
        args: &[],
        install: "cargo install mcp-infrastructure",
    },
    McpCatalogEntry {
        id: "mcp-environment",
        name: "Environments",
        description: "Track environment registry, deployments, and runtime configuration.",
        category: CapabilityCategory::Development,
        command: "mcp-environment",
        args: &[],
        install: "cargo install mcp-environment",
    },
    McpCatalogEntry {
        id: "mcp-observability",
        name: "Observability",
        description: "Read logs, traces, metrics, alerts, dashboards, incidents, and SLOs.",
        category: CapabilityCategory::Development,
        command: "mcp-observability",
        args: &[],
        install: "cargo install mcp-observability",
    },
    McpCatalogEntry {
        id: "mcp-news",
        name: "News",
        description: "Read real-time articles, trending topics, and sentiment.",
        category: CapabilityCategory::Research,
        command: "mcp-news",
        args: &[],
        install: "cargo install mcp-news",
    },
    McpCatalogEntry {
        id: "mcp-knowledge-base",
        name: "Knowledge base",
        description: "Search articles, policies, and known issues, and record feedback.",
        category: CapabilityCategory::Research,
        command: "mcp-knowledge-base",
        args: &[],
        install: "cargo install mcp-knowledge-base",
    },
    McpCatalogEntry {
        id: "mcp-maps",
        name: "Maps",
        description: "Geocode, reverse geocode, route, and search points of interest.",
        category: CapabilityCategory::Research,
        command: "mcp-maps",
        args: &[],
        install: "cargo install mcp-maps",
    },
    McpCatalogEntry {
        id: "mcp-weather",
        name: "Weather",
        description: "Read forecasts, history, air quality, and marine conditions.",
        category: CapabilityCategory::Research,
        command: "mcp-weather",
        args: &[],
        install: "cargo install mcp-weather",
    },
    McpCatalogEntry {
        id: "mcp-market-data",
        name: "Market data",
        description: "Read instruments, quotes, and historical bars.",
        category: CapabilityCategory::Research,
        command: "mcp-market-data",
        args: &[],
        install: "cargo install mcp-market-data",
    },
    McpCatalogEntry {
        id: "mcp-legal",
        name: "Legal reference",
        description: "Search case law, legislation, regulations, and sanctions lists.",
        category: CapabilityCategory::Research,
        command: "mcp-legal",
        args: &[],
        install: "cargo install mcp-legal",
    },
    McpCatalogEntry {
        id: "mcp-medical",
        name: "Clinical reference",
        description: "Search PubMed and WHO health statistics and clinical references.",
        category: CapabilityCategory::Research,
        command: "mcp-medical",
        args: &[],
        install: "cargo install mcp-medical",
    },
    McpCatalogEntry {
        id: "mcp-pharmacy",
        name: "Drug reference",
        description: "Search drug labels, RxNorm, PubChem, and regulatory sources.",
        category: CapabilityCategory::Research,
        command: "mcp-pharmacy",
        args: &[],
        install: "cargo install mcp-pharmacy",
    },
    McpCatalogEntry {
        id: "mcp-real-estate",
        name: "Property data",
        description: "Read property transactions, valuations, and market data.",
        category: CapabilityCategory::Research,
        command: "mcp-real-estate",
        args: &[],
        install: "cargo install mcp-real-estate",
    },
    McpCatalogEntry {
        id: "mcp-regulatory",
        name: "Regulatory",
        description: "Track requirements, compliance reviews, submissions, and filings.",
        category: CapabilityCategory::Research,
        command: "mcp-regulatory",
        args: &[],
        install: "cargo install mcp-regulatory",
    },
    McpCatalogEntry {
        id: "mcp-itsm",
        name: "IT service management",
        description: "Manage tickets, incidents, change requests, SLAs, and service catalog.",
        category: CapabilityCategory::Operations,
        command: "mcp-itsm",
        args: &[],
        install: "cargo install mcp-itsm",
    },
    McpCatalogEntry {
        id: "mcp-iot",
        name: "IoT fleets",
        description: "Read telemetry and send commands to devices, with twins and OTA updates.",
        category: CapabilityCategory::Operations,
        command: "mcp-iot",
        args: &[],
        install: "cargo install mcp-iot",
    },
    McpCatalogEntry {
        id: "mcp-crm",
        name: "CRM",
        description: "Read and update contacts, accounts, and deals across CRM backends.",
        category: CapabilityCategory::Operations,
        command: "mcp-crm",
        args: &[],
        install: "cargo install mcp-crm",
    },
    McpCatalogEntry {
        id: "mcp-erp",
        name: "ERP",
        description: "Read and update ERP records across SAP, NetSuite, Odoo, and Dynamics.",
        category: CapabilityCategory::Operations,
        command: "mcp-erp",
        args: &[],
        install: "cargo install mcp-erp",
    },
    McpCatalogEntry {
        id: "mcp-finance",
        name: "Finance",
        description: "Manage invoices, expenses, accounts, journal entries, and reconciliation.",
        category: CapabilityCategory::Operations,
        command: "mcp-finance",
        args: &[],
        install: "cargo install mcp-finance",
    },
    McpCatalogEntry {
        id: "mcp-sales",
        name: "Sales",
        description: "Manage proposals, quotes, sequences, meetings, and forecasts.",
        category: CapabilityCategory::Operations,
        command: "mcp-sales",
        args: &[],
        install: "cargo install mcp-sales",
    },
    McpCatalogEntry {
        id: "mcp-marketing",
        name: "Marketing",
        description: "Manage campaigns, audiences, content, ads, and performance.",
        category: CapabilityCategory::Operations,
        command: "mcp-marketing",
        args: &[],
        install: "cargo install mcp-marketing",
    },
    McpCatalogEntry {
        id: "mcp-hris",
        name: "HR systems",
        description: "Read employees, departments, time off, payroll, and org chart.",
        category: CapabilityCategory::Operations,
        command: "mcp-hris",
        args: &[],
        install: "cargo install mcp-hris",
    },
    McpCatalogEntry {
        id: "mcp-procurement",
        name: "Procurement",
        description: "Manage purchase orders, RFQs, suppliers, approvals, and receiving.",
        category: CapabilityCategory::Operations,
        command: "mcp-procurement",
        args: &[],
        install: "cargo install mcp-procurement",
    },
    McpCatalogEntry {
        id: "mcp-logistics",
        name: "Logistics",
        description: "Track shipments, carriers, routes, and warehouse movements.",
        category: CapabilityCategory::Operations,
        command: "mcp-logistics",
        args: &[],
        install: "cargo install mcp-logistics",
    },
    McpCatalogEntry {
        id: "mcp-credentials-vault",
        name: "Credentials vault",
        description: "Grant scoped, auditable access to stored credentials.",
        category: CapabilityCategory::Platform,
        command: "mcp-credentials-vault",
        args: &[],
        install: "cargo install mcp-credentials-vault",
    },
    McpCatalogEntry {
        id: "mcp-approval",
        name: "Approvals",
        description: "Run multi-stage approval workflows with configurable gates.",
        category: CapabilityCategory::Platform,
        command: "mcp-approval",
        args: &[],
        install: "cargo install mcp-approval",
    },
    McpCatalogEntry {
        id: "mcp-governance-policy",
        name: "Governance policy",
        description: "Evaluate and simulate policy, and record approvals.",
        category: CapabilityCategory::Platform,
        command: "mcp-governance-policy",
        args: &[],
        install: "cargo install mcp-governance-policy",
    },
    McpCatalogEntry {
        id: "mcp-identity",
        name: "Identity",
        description: "Look up users, groups, MFA, entitlements, and access requests.",
        category: CapabilityCategory::Platform,
        command: "mcp-identity",
        args: &[],
        install: "cargo install mcp-identity",
    },
    McpCatalogEntry {
        id: "mcp-artifact-store",
        name: "Artifact store",
        description: "Store and retrieve governed artifacts with provenance.",
        category: CapabilityCategory::Platform,
        command: "mcp-artifact-store",
        args: &[],
        install: "cargo install mcp-artifact-store",
    },
    McpCatalogEntry {
        id: "mcp-session-memory",
        name: "Session memory",
        description: "Read and write typed session state and scoped memory.",
        category: CapabilityCategory::Platform,
        command: "mcp-session-memory",
        args: &[],
        install: "cargo install mcp-session-memory",
    },
    McpCatalogEntry {
        id: "mcp-a2a",
        name: "Remote agents",
        description: "Discover agent cards and delegate tasks over A2A.",
        category: CapabilityCategory::Platform,
        command: "mcp-a2a",
        args: &[],
        install: "cargo install mcp-a2a",
    },
    McpCatalogEntry {
        id: "mcp-acp-workspace",
        name: "ACP workspace",
        description: "Manage ACP coding delegates with permission gates.",
        category: CapabilityCategory::Platform,
        command: "mcp-acp-workspace",
        args: &[],
        install: "cargo install mcp-acp-workspace",
    },
];

pub fn entries() -> &'static [McpCatalogEntry] {
    CATALOG
}

/// Every curated MCP server entry.
pub fn catalog_entries() -> &'static [McpCatalogEntry] {
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

/// Ids of the MCP servers already configured for a profile.
///
/// A missing or unreadable config means "nothing configured" rather than an
/// error: this is used to describe a proposed change, and a config that cannot be
/// parsed will surface when the change is applied.
pub fn configured_server_ids(path: &Path, profile: &str) -> Result<BTreeSet<String>> {
    if !path.exists() {
        return Ok(BTreeSet::new());
    }
    let document = read_document(path)?;
    let Some(servers) = document
        .get("profiles")
        .and_then(Item::as_table)
        .and_then(|profiles| profiles.get(profile))
        .and_then(Item::as_table)
        .and_then(|table| table.get("mcp_servers"))
        .and_then(Item::as_array_of_tables)
    else {
        return Ok(BTreeSet::new());
    };
    Ok(servers
        .iter()
        .filter_map(|table| table.get("name").and_then(Item::as_str))
        .map(str::to_string)
        .collect())
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
