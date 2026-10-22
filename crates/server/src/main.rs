use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use clap::Parser;
use forge_exec_http::DefaultSecretResolver;
use forge_store::Store;
use mcp_forge::config::{Cli, Command, ExportArgs, ImportArgs, OnConflict, ServeArgs, StdioArgs};
use mcp_forge::AppState;
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();
    match Cli::parse().command {
        Command::Serve(args) => serve(args).await,
        Command::Stdio(args) => stdio(args).await,
        Command::Export(args) => export(args).await,
        Command::Import(args) => import(args).await,
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
    let security = args.security()?;
    let data_dir = args.common.resolve_data_dir()?;
    let store = Store::open(&data_dir)
        .await
        .with_context(|| format!("opening store in {}", data_dir.display()))?;
    let listener = TcpListener::bind(args.bind)
        .await
        .with_context(|| format!("binding {}", args.bind))?;
    let addr = listener.local_addr()?;
    tracing::info!(%addr, data_dir = %data_dir.display(), "mcp-forge listening");
    if addr.ip().is_loopback() {
        tracing::info!("open http://{addr}/ in your browser");
    }
    if security.admin_token.is_some() {
        tracing::info!("admin token required on /api and /mcp");
    }
    let state = AppState::new(store, Arc::new(DefaultSecretResolver))
        .with_security(security)
        .with_data_dir(data_dir);
    mcp_forge::serve(listener, state, shutdown_signal()).await?;
    tracing::info!("shut down");
    Ok(())
}

async fn stdio(args: StdioArgs) -> anyhow::Result<()> {
    let data_dir = args.common.resolve_data_dir()?;
    let store = Store::open(&data_dir)
        .await
        .with_context(|| format!("opening store in {}", data_dir.display()))?;
    let exec = forge_mcp::Executors::new(Arc::new(DefaultSecretResolver));
    forge_mcp::serve_stdio(store, exec, &args.project, STDIO_POLL).await?;
    Ok(())
}

async fn open_store(common: &mcp_forge::config::CommonArgs) -> anyhow::Result<Store> {
    let data_dir = common.resolve_data_dir()?;
    Store::open(&data_dir)
        .await
        .with_context(|| format!("opening store in {}", data_dir.display()))
}

async fn export(args: ExportArgs) -> anyhow::Result<()> {
    let store = open_store(&args.common).await?;
    let project = store
        .get_project_by_slug(&args.project)
        .await
        .with_context(|| format!("project `{}`", args.project))?;
    let yaml = mcp_forge::bundle::export_project(&store, project.id).await?;
    match args.out {
        Some(path) => {
            std::fs::write(&path, yaml).with_context(|| format!("writing {}", path.display()))?;
            eprintln!("Exported `{}` to {}", project.slug, path.display());
        }
        None => print!("{yaml}"),
    }
    Ok(())
}

async fn import(args: ImportArgs) -> anyhow::Result<()> {
    use std::io::Read;
    let text = match args.file.as_deref() {
        None => {
            let mut s = String::new();
            std::io::stdin()
                .read_to_string(&mut s)
                .context("reading stdin")?;
            s
        }
        Some(p) if p.as_os_str() == "-" => {
            let mut s = String::new();
            std::io::stdin()
                .read_to_string(&mut s)
                .context("reading stdin")?;
            s
        }
        Some(p) => {
            std::fs::read_to_string(p).with_context(|| format!("reading {}", p.display()))?
        }
    };
    let bundle = mcp_forge::bundle::parse_bundle(&text).map_err(|e| anyhow::anyhow!("{e}"))?;
    let store = open_store(&args.common).await?;
    let project = match store.get_project_by_slug(&args.project).await {
        Ok(p) => p,
        Err(forge_store::StoreError::NotFound(_)) => {
            let name = bundle
                .project
                .as_ref()
                .map_or_else(|| args.project.clone(), |p| p.name.clone());
            let p = store
                .create_project(&forge_core::ProjectInput {
                    name,
                    slug: Some(args.project.clone()),
                })
                .await?;
            eprintln!("Created project `{}`", p.slug);
            p
        }
        Err(e) => return Err(e.into()),
    };
    let policy = match args.on_conflict {
        OnConflict::Fail => forge_store::ConflictPolicy::Fail,
        OnConflict::Skip => forge_store::ConflictPolicy::Skip,
        OnConflict::Replace => forge_store::ConflictPolicy::Replace,
    };
    mcp_forge::bundle::check_sql_tools(&store, project.id, &bundle).await?;
    let r = store
        .import_bundle(
            project.id,
            &bundle.auth_configs,
            &bundle.db_connections,
            &bundle.tools,
            policy,
        )
        .await?;
    eprintln!(
        "Imported into `{}`: tools {} created, {} updated, {} skipped; auth configs {} created, {} updated, {} kept; connections {} created, {} updated, {} kept",
        project.slug,
        r.tools_created.len(),
        r.tools_updated.len(),
        r.tools_skipped.len(),
        r.auth_created.len(),
        r.auth_updated.len(),
        r.auth_kept.len(),
        r.connections_created.len(),
        r.connections_updated.len(),
        r.connections_kept.len()
    );
    Ok(())
}

/// How often stdio mode checks the store for tool edits made by other processes.
const STDIO_POLL: Duration = Duration::from_secs(2);

async fn shutdown_signal() {
    if let Err(e) = tokio::signal::ctrl_c().await {
        tracing::error!(error = %e, "failed to listen for ctrl-c");
        std::future::pending::<()>().await;
    }
}
