//! An authoritative gameserver: browsers connect over WebRTC data channels (str0m), and a
//! fixed-tick loop runs the shared simulation.
//!
//! The gameserver dials out to the lobby over a WebSocket, registers, and answers the SDP
//! offers the lobby relays from browsers. It needs no HTTP endpoint or TLS certificate: only
//! its UDP port has to be reachable at the advertised address.
#![forbid(unsafe_code)]

mod lobby_client;
mod server;

use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket as StdUdpSocket};
use std::sync::Arc;

use anyhow::{Context, bail};
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, watch};
use tracing::info;

/// Ports browsers will not send traffic to, WebRTC included: Chromium's restricted-port list
/// (`kRestrictedPorts` in `net/base/port_util.cc`). Traffic to them is dropped silently, so a
/// gameserver listening on one would look up but never accept a single browser.
const BROWSER_BLOCKED_PORTS: &[u16] = &[
    1, 7, 9, 11, 13, 15, 17, 19, 20, 21, 22, 23, 25, 37, 42, 43, 53, 69, 77, 79, 87, 95, 101, 102,
    103, 104, 109, 110, 111, 113, 115, 117, 119, 123, 135, 137, 139, 143, 161, 179, 389, 427, 465,
    512, 513, 514, 515, 526, 530, 531, 532, 540, 548, 554, 556, 563, 587, 601, 636, 989, 990, 993,
    995, 1719, 1720, 1723, 2049, 3659, 4045, 4190, 5060, 5061, 6000, 6566, 6665, 6666, 6667, 6668,
    6669, 6679, 6697, 10080,
];

#[derive(Debug, Clone)]
pub struct Config {
    /// Name shown in the server list.
    pub name: String,
    /// Most players connected at once.
    pub slots: u16,
    /// The lobby's gameserver WebSocket, e.g. `ws://127.0.0.1:3000/api/gameservers`.
    pub lobby_url: String,
    /// Secret the lobby requires from gameservers, if any.
    pub lobby_token: Option<String>,
    /// UDP address to receive WebRTC traffic on.
    pub bind: SocketAddr,
    /// IP clients should send WebRTC traffic to. Defaults to the IP of `bind`, or, when that is
    /// unspecified (`0.0.0.0`), to the IP of the machine's default route. Set it to the public IP
    /// when the server sits behind 1:1 NAT (e.g. most cloud VMs).
    pub public_ip: Option<IpAddr>,
}

/// Runs the gameserver until `shutdown` completes.
///
/// # Errors
/// Fails when the port is one browsers refuse to use, the UDP socket cannot be bound or the
/// game loop hits an I/O error.
pub async fn run(config: Config, shutdown: impl Future<Output = ()>) -> anyhow::Result<()> {
    str0m::crypto::from_feature_flags().install_process_default();

    if BROWSER_BLOCKED_PORTS.contains(&config.bind.port()) {
        bail!(
            "browsers refuse to send traffic to port {}; bind the gameserver to another UDP port",
            config.bind.port()
        );
    }
    let socket = UdpSocket::bind(config.bind)
        .await
        .with_context(|| format!("binding UDP {}", config.bind))?;
    let local_addr = socket.local_addr()?;
    let public_addr = SocketAddr::new(
        advertised_ip(config.public_ip, local_addr.ip()),
        local_addr.port(),
    );
    info!(%local_addr, %public_addr, "listening for WebRTC");

    let (offers_tx, offers_rx) = mpsc::channel(16);
    let (players_tx, players_rx) = watch::channel(0);
    let config = Arc::new(config);
    let lobby = tokio::spawn(lobby_client::run(
        Arc::clone(&config),
        offers_tx,
        players_rx,
    ));

    let result = server::Server::new(socket, public_addr, config.slots, players_tx)
        .run(offers_rx, shutdown)
        .await;
    lobby.abort();
    result
}

fn advertised_ip(public_ip: Option<IpAddr>, bound_ip: IpAddr) -> IpAddr {
    public_ip.unwrap_or_else(|| {
        if bound_ip.is_unspecified() {
            default_route_ip().unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST))
        } else {
            bound_ip
        }
    })
}

/// The local IP the OS would use to reach the internet. `connect` on UDP sends nothing.
fn default_route_ip() -> Option<IpAddr> {
    let socket = StdUdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket.connect((Ipv4Addr::new(192, 0, 2, 1), 9)).ok()?; // TEST-NET-1, never routed
    let ip = socket.local_addr().ok()?.ip();
    (!ip.is_unspecified()).then_some(ip)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advertises_the_configured_ip_first() {
        let public: IpAddr = "203.0.113.7".parse().unwrap();
        assert_eq!(
            advertised_ip(Some(public), Ipv4Addr::UNSPECIFIED.into()),
            public
        );
    }

    #[test]
    fn advertises_a_specific_bound_ip() {
        assert_eq!(
            advertised_ip(None, Ipv4Addr::LOCALHOST.into()),
            IpAddr::from(Ipv4Addr::LOCALHOST)
        );
    }

    #[tokio::test]
    async fn refuses_ports_browsers_block() {
        let config = Config {
            name: "IRC".into(),
            slots: 1,
            lobby_url: "ws://127.0.0.1:1/api/gameservers".into(),
            lobby_token: None,
            bind: "127.0.0.1:6666".parse().unwrap(),
            public_ip: None,
        };
        let error = run(config, async {}).await.unwrap_err();
        assert!(error.to_string().contains("browsers refuse"), "{error}");
    }

    #[test]
    fn replaces_an_unspecified_bound_ip() {
        assert!(!advertised_ip(None, Ipv4Addr::UNSPECIFIED.into()).is_unspecified());
    }
}
