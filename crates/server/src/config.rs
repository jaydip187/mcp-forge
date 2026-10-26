//! CLI definition and config resolution. Precedence: CLI flag > env var > default
//! (clap's `env` support handles the first two).

use std::net::SocketAddr;
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

use crate::security::{Security, MIN_TOKEN_LEN};

pub const DEFAULT_BIND: &str = "127.0.0.1:7410";
pub const DATA_DIR_NAME: &str = ".mcp-forge";

#[derive(Debug, Parser)]
#[command(
    name = "mcp-forge",
    version,
    about = "Turn HTTP APIs and SQL databases into MCP tools"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Run the web UI, REST API and MCP streamable-HTTP endpoints.
    Serve(ServeArgs),
    /// Serve one project's tools over MCP stdio.
    Stdio(StdioArgs),
    /// Write a project's auth configs and tools as YAML (secrets stay references).
    Export(ExportArgs),
    /// Load auth configs and tools from a YAML file into a project.
    Import(ImportArgs),
    /// Manage accounts for the web UI (works on the data directory directly,
    /// so it also recovers a forgotten password).
    User(UserArgs),
}

#[derive(Debug, Args)]
pub struct UserArgs {
    #[command(subcommand)]
    pub action: UserAction,
    #[command(flatten)]
    pub common: CommonArgs,
}

#[derive(Debug, Subcommand)]
pub enum UserAction {
    /// Create an account.
    Add {
        username: String,
        /// Read the password from the first line of stdin instead of prompting.
        #[arg(long)]
        password_stdin: bool,
    },
    /// Set a new password; signs the account out everywhere.
    Passwd {
        username: String,
        /// Read the password from the first line of stdin instead of prompting.
        #[arg(long)]
        password_stdin: bool,
    },
    /// List accounts.
    List,
    /// Delete an account and end its sessions.
    Remove { username: String },
}

#[derive(Debug, Args)]
pub struct ExportArgs {
    /// Slug of the project to export.
    #[arg(long)]
    pub project: String,
    /// Write to this file instead of stdout.
    #[arg(long, short)]
    pub out: Option<PathBuf>,
    #[command(flatten)]
    pub common: CommonArgs,
}

#[derive(Debug, Args)]
pub struct ImportArgs {
    /// Slug of the project to import into; created if it doesn't exist.
    #[arg(long)]
    pub project: String,
    /// YAML file to read; `-` or omitted reads stdin.
    pub file: Option<PathBuf>,
    /// What to do when a tool or auth config with the same name exists.
    #[arg(long, value_enum, default_value_t = OnConflict::Fail)]
    pub on_conflict: OnConflict,
    /// Allow importing database connections with `full_access` (agents may
    /// change and delete data with SQL they write).
    #[arg(long)]
    pub confirm_full_access: bool,
    #[command(flatten)]
    pub common: CommonArgs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum OnConflict {
    /// Abort without changing anything.
    Fail,
    /// Keep what's there.
    Skip,
    /// Overwrite with the file's version.
    Replace,
}

#[derive(Debug, Args)]
pub struct CommonArgs {
    /// Directory holding the internal database [default: ~/.mcp-forge]
    #[arg(long, env = "MCP_FORGE_DATA_DIR", global = true)]
    pub data_dir: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct ServeArgs {
    /// Address to listen on.
    #[arg(long, env = "MCP_FORGE_BIND", default_value = DEFAULT_BIND)]
    pub bind: SocketAddr,
    /// Bearer token required on /api and /mcp. Mandatory when --bind is not a
    /// loopback address. Prefer the env var: flags show up in process lists.
    #[arg(long, env = "MCP_FORGE_ADMIN_TOKEN", hide_env_values = true)]
    pub admin_token: Option<String>,
    #[command(flatten)]
    pub common: CommonArgs,
}

impl ServeArgs {
    /// Security settings for this bind, or an error if they'd be unsafe.
    pub fn security(&self) -> anyhow::Result<Security> {
        let loopback = self.bind.ip().is_loopback();
        if let Some(t) = &self.admin_token {
            if t.len() < MIN_TOKEN_LEN {
                anyhow::bail!("--admin-token must be at least {MIN_TOKEN_LEN} characters");
            }
        } else if !loopback {
            anyhow::bail!(
                "binding to non-loopback address {} requires --admin-token (or MCP_FORGE_ADMIN_TOKEN)",
                self.bind
            );
        }
        Ok(Security {
            admin_token: self.admin_token.as_deref().map(Into::into),
            loopback_hosts_only: loopback,
            require_login: true,
        })
    }
}

#[derive(Debug, Args)]
pub struct StdioArgs {
    /// Slug of the project whose tools to expose.
    #[arg(long, env = "MCP_FORGE_PROJECT")]
    pub project: String,
    #[command(flatten)]
    pub common: CommonArgs,
}

impl CommonArgs {
    /// Resolves the data directory, falling back to `~/.mcp-forge`.
    pub fn resolve_data_dir(&self) -> anyhow::Result<PathBuf> {
        if let Some(dir) = &self.data_dir {
            return Ok(dir.clone());
        }
        home_dir()
            .map(|h| h.join(DATA_DIR_NAME))
            .ok_or_else(|| anyhow::anyhow!("cannot determine home directory; pass --data-dir"))
    }
}

fn home_dir() -> Option<PathBuf> {
    ["HOME", "USERPROFILE"]
        .iter()
        .filter_map(std::env::var_os)
        .find(|v| !v.is_empty())
        .map(PathBuf::from)
}

#[cfg(test)]
#[path = "../tests/unit/config.rs"]
mod tests;
