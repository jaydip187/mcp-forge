//! CLI definition and config resolution. Precedence: CLI flag > env var > default
//! (clap's `env` support handles the first two).

use std::net::SocketAddr;
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

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
    #[command(flatten)]
    pub common: CommonArgs,
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
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Cli {
        Cli::try_parse_from(args).unwrap()
    }

    #[test]
    fn serve_defaults() {
        let Command::Serve(s) = parse(&["mcp-forge", "serve"]).command else {
            panic!("expected serve");
        };
        assert_eq!(s.bind, DEFAULT_BIND.parse().unwrap());
    }

    #[test]
    fn serve_flags() {
        let Command::Serve(s) = parse(&[
            "mcp-forge",
            "serve",
            "--bind",
            "0.0.0.0:9000",
            "--data-dir",
            "/tmp/x",
        ])
        .command
        else {
            panic!("expected serve");
        };
        assert_eq!(s.bind, "0.0.0.0:9000".parse().unwrap());
        assert_eq!(
            s.common.resolve_data_dir().unwrap(),
            PathBuf::from("/tmp/x")
        );
    }

    #[test]
    fn stdio_requires_project() {
        assert!(Cli::try_parse_from(["mcp-forge", "stdio"]).is_err());
        let Command::Stdio(s) = parse(&["mcp-forge", "stdio", "--project", "demo"]).command else {
            panic!("expected stdio");
        };
        assert_eq!(s.project, "demo");
    }

    #[test]
    fn data_dir_defaults_under_home() {
        let args = CommonArgs { data_dir: None };
        if let Ok(dir) = args.resolve_data_dir() {
            assert!(dir.ends_with(DATA_DIR_NAME));
        }
    }
}
