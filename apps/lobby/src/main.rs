use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::Context;
use clap::Parser;
use tokio::net::TcpListener;
use tracing::info;
use tracing_subscriber::EnvFilter;

/// Lists gameservers and relays WebRTC signaling between browsers and gameservers.
#[derive(Debug, Parser)]
#[command(version)]
struct Args {
    /// Address to listen on for HTTP and WebSocket connections.
    #[arg(long, env = "LOBBY_LISTEN", default_value = "127.0.0.1:3000")]
    listen: SocketAddr,
    /// Secret gameservers must present to register. Required in production.
    #[arg(long, env = "GAMESERVER_TOKEN", hide_env_values = true)]
    gameserver_token: Option<String>,
    /// Directory with the built client (apps/client/dist) to serve on non-API paths.
    #[arg(long, env = "STATIC_DIR")]
    static_dir: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
    let args = Args::parse();

    if args.gameserver_token.is_none() {
        info!("GAMESERVER_TOKEN is not set: any gameserver may register");
    }
    let listener = TcpListener::bind(args.listen)
        .await
        .with_context(|| format!("binding {}", args.listen))?;
    info!("lobby listening on http://{}", listener.local_addr()?);

    let config = lobby::Config {
        gameserver_token: args.gameserver_token,
        static_dir: args.static_dir,
        ..Default::default()
    };
    lobby::serve(listener, config, async {
        let _ = tokio::signal::ctrl_c().await;
        info!("shutting down");
    })
    .await?;
    Ok(())
}
