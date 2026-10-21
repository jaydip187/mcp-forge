use anyhow::Context;
use clap::Parser;
use forge_store::Store;
use mcp_forge::config::{Cli, Command, ServeArgs, StdioArgs};
use mcp_forge::AppState;
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();
    match Cli::parse().command {
        Command::Serve(args) => serve(args).await,
        Command::Stdio(args) => stdio(args).await,
    }
}

/// Logs always go to stderr so stdout stays clean for the MCP stdio transport.
fn init_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .with_writer(std::io::stderr)
        .init();
}

async fn serve(args: ServeArgs) -> anyhow::Result<()> {
    let data_dir = args.common.resolve_data_dir()?;
    let store = Store::open(&data_dir)
        .await
        .with_context(|| format!("opening store in {}", data_dir.display()))?;
    let listener = TcpListener::bind(args.bind)
        .await
        .with_context(|| format!("binding {}", args.bind))?;
    tracing::info!(addr = %listener.local_addr()?, data_dir = %data_dir.display(), "mcp-forge listening");
    mcp_forge::serve(listener, AppState { store }, shutdown_signal()).await?;
    tracing::info!("shut down");
    Ok(())
}

async fn stdio(args: StdioArgs) -> anyhow::Result<()> {
    let data_dir = args.common.resolve_data_dir()?;
    let _store = Store::open(&data_dir)
        .await
        .with_context(|| format!("opening store in {}", data_dir.display()))?;
    anyhow::bail!(
        "stdio transport for project '{}' is not implemented yet (Stage 2)",
        args.project
    )
}

async fn shutdown_signal() {
    if let Err(e) = tokio::signal::ctrl_c().await {
        tracing::error!(error = %e, "failed to listen for ctrl-c");
        std::future::pending::<()>().await;
    }
}
