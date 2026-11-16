use std::net::{IpAddr, SocketAddr};

use clap::Parser;
use tracing::info;
use tracing_subscriber::EnvFilter;

/// Authoritative shooter2d gameserver: WebRTC data channels plus a fixed-tick simulation.
#[derive(Debug, Parser)]
#[command(version)]
struct Args {
    /// Name shown in the server list.
    #[arg(long, env = "GAMESERVER_NAME", default_value = "shooter2d")]
    name: String,
    /// Most players connected at once.
    #[arg(long, env = "SLOTS", default_value_t = 8)]
    slots: u16,
    /// The lobby's WebSocket endpoint for gameservers.
    #[arg(
        long,
        env = "LOBBY_URL",
        default_value = "ws://127.0.0.1:3000/api/gameservers"
    )]
    lobby_url: String,
    /// Secret the lobby requires from gameservers.
    #[arg(long, env = "GAMESERVER_TOKEN", hide_env_values = true)]
    lobby_token: Option<String>,
    /// UDP address for WebRTC traffic. Avoid ports browsers block (e.g. 6665-6669).
    #[arg(long, env = "BIND", default_value = "0.0.0.0:7777")]
    bind: SocketAddr,
    /// IP clients send WebRTC traffic to; set it when the server is behind 1:1 NAT.
    /// Defaults to the IP of the default route.
    #[arg(long, env = "PUBLIC_IP")]
    public_ip: Option<IpAddr>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,str0m=warn,dimpl=warn")),
        )
        .init();
    let args = Args::parse();
    let config = gameserver::Config {
        name: args.name,
        slots: args.slots,
        lobby_url: args.lobby_url,
        lobby_token: args.lobby_token,
        bind: args.bind,
        public_ip: args.public_ip,
    };
    gameserver::run(config, async {
        let _ = tokio::signal::ctrl_c().await;
        info!("received Ctrl+C");
    })
    .await
}
