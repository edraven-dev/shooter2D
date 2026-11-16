//! The game loop: one task owns the UDP socket, every peer's str0m `Rtc` and the world.
//!
//! str0m is sans-IO, so this loop feeds it datagrams and timeouts and sends what it outputs.
//! All peers share one UDP socket; an incoming datagram belongs to the `Rtc` that `accepts` it.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use anyhow::Context;
use protocol::game::{
    Buttons, CONTROL_CHANNEL, ClientMessage, INPUT_REDUNDANCY, MAX_PLAYER_NAME_LEN,
    PROTOCOL_VERSION, PlayerId, PlayerInfo, STATE_CHANNEL, ServerMessage, Welcome, decode, encode,
};
use sim::{Event as GameEvent, Level, TICK_RATE, World};
use str0m::change::SdpOffer;
use str0m::channel::ChannelId;
use str0m::net::{Protocol, Receive};
use str0m::{Candidate, Event, IceConnectionState, Input, Output, Rtc};
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, watch};
use tracing::{debug, info, warn};

use crate::lobby_client::OfferRequest;

/// Snapshots go out every this many ticks (30 per second at 60 Hz).
const SNAPSHOT_INTERVAL: u32 = 2;
/// A connection must say hello within this time or it is dropped.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// Rejected peers get this long to receive the rejection before being disconnected.
const REJECT_LINGER: Duration = Duration::from_secs(1);
/// A peer that sent nothing for this long is gone (closed tab, lost network). Browsers send
/// ICE consent checks every few seconds even from background tabs.
const IDLE_TIMEOUT: Duration = Duration::from_secs(10);
/// Inputs buffered per player; older ones are dropped beyond this.
const MAX_QUEUED_INPUTS: usize = 32;
/// With more inputs than this queued, a player catches up by applying two per tick.
const CATCH_UP_THRESHOLD: usize = 3;

pub(crate) struct Server {
    socket: UdpSocket,
    /// Our address as advertised in the ICE candidate; reported to str0m as the destination
    /// of every datagram, so it matches even behind 1:1 NAT.
    public_addr: SocketAddr,
    slots: u16,
    peers: Vec<Peer>,
    world: World,
    next_player_id: PlayerId,
    player_count: watch::Sender<u16>,
}

struct Peer {
    rtc: Rtc,
    control: Option<ChannelId>,
    state: Option<ChannelId>,
    player: Option<PlayerId>,
    name: String,
    /// Inputs received but not applied yet, by sequence number.
    inputs: BTreeMap<u32, Buttons>,
    /// Sequence number of the last input applied.
    last_seq: u32,
    connected_at: Instant,
    last_seen: Instant,
    disconnect_at: Option<Instant>,
}

impl Server {
    pub(crate) fn new(
        socket: UdpSocket,
        public_addr: SocketAddr,
        slots: u16,
        player_count: watch::Sender<u16>,
    ) -> Self {
        Self {
            socket,
            public_addr,
            slots,
            peers: Vec::new(),
            world: World::new(Level::arena()),
            next_player_id: 1,
            player_count,
        }
    }

    pub(crate) async fn run(
        mut self,
        mut offers: mpsc::Receiver<OfferRequest>,
        shutdown: impl Future<Output = ()>,
    ) -> anyhow::Result<()> {
        tokio::pin!(shutdown);
        let tick_duration = Duration::from_secs(1) / u32::from(TICK_RATE);
        let mut next_tick = Instant::now() + tick_duration;
        let mut buf = vec![0; 2000];

        loop {
            let wake_at = self.poll_peers(next_tick).await;
            self.reap_peers();

            tokio::select! {
                received = self.socket.recv_from(&mut buf) => {
                    let (len, source) = received.context("receiving from the UDP socket")?;
                    self.handle_datagram(&buf[..len], source);
                }
                Some(offer) = offers.recv() => self.accept_offer(offer),
                () = tokio::time::sleep_until(wake_at.into()) => {}
                () = &mut shutdown => break,
            }

            let now = Instant::now();
            for peer in &mut self.peers {
                peer.handle_input(Input::Timeout(now));
            }
            if now >= next_tick {
                self.tick();
                next_tick += tick_duration;
                if now > next_tick + tick_duration * 5 {
                    // Far behind (e.g. the process was suspended): skip ahead instead of spinning.
                    next_tick = now + tick_duration;
                }
            }
        }

        info!("shutting down");
        for peer in &mut self.peers {
            peer.rtc.disconnect();
        }
        Ok(())
    }

    /// Flushes every peer's pending output and handles their events until none are left.
    /// Returns when the loop next has to wake up (at the latest, `deadline`).
    async fn poll_peers(&mut self, deadline: Instant) -> Instant {
        loop {
            let mut wake_at = deadline;
            let mut events = Vec::new();
            for (index, peer) in self.peers.iter_mut().enumerate() {
                while peer.rtc.is_alive() {
                    match peer.rtc.poll_output() {
                        Ok(Output::Timeout(at)) => {
                            wake_at = wake_at.min(at);
                            break;
                        }
                        Ok(Output::Transmit(transmit)) => {
                            if let Err(error) = self
                                .socket
                                .send_to(&transmit.contents, transmit.destination)
                                .await
                            {
                                debug!(%error, destination = %transmit.destination, "dropping a datagram");
                            }
                        }
                        Ok(Output::Event(event)) => events.push((index, event)),
                        Err(error) => {
                            warn!(%error, "peer failed");
                            peer.rtc.disconnect();
                        }
                    }
                }
            }
            if events.is_empty() {
                return wake_at;
            }
            for (index, event) in events {
                self.handle_event(index, event);
            }
        }
    }

    fn handle_datagram(&mut self, data: &[u8], source: SocketAddr) {
        let Ok(contents) = data.try_into() else {
            debug!(%source, len = data.len(), "dropping an unrecognized datagram");
            return;
        };
        let input = Input::Receive(
            Instant::now(),
            Receive {
                proto: Protocol::Udp,
                source,
                destination: self.public_addr,
                contents,
            },
        );
        match self.peers.iter_mut().find(|peer| peer.rtc.accepts(&input)) {
            Some(peer) => {
                peer.last_seen = Instant::now();
                peer.handle_input(input);
            }
            None => debug!(%source, "datagram for no peer"),
        }
    }

    fn accept_offer(&mut self, OfferRequest { sdp, reply }: OfferRequest) {
        if self.peers.len() >= usize::from(self.slots) {
            let _ = reply.send(Err("the server is full".into()));
            return;
        }
        let answered = (|| {
            let offer = SdpOffer::from_sdp_string(&sdp)
                .map_err(|error| format!("invalid offer: {error}"))?;
            let mut rtc = Rtc::builder().set_ice_lite(true).build(Instant::now());
            let candidate =
                Candidate::host(self.public_addr, "udp").map_err(|error| error.to_string())?;
            rtc.add_local_candidate(candidate);
            let answer = rtc
                .sdp_api()
                .accept_offer(offer)
                .map_err(|error| format!("cannot accept the offer: {error}"))?;
            Ok::<_, String>((rtc, answer.to_sdp_string()))
        })();
        match answered {
            Ok((rtc, answer)) => {
                self.peers.push(Peer::new(rtc));
                let _ = reply.send(Ok(answer));
            }
            Err(reason) => {
                warn!(reason, "refused an offer");
                let _ = reply.send(Err(reason));
            }
        }
    }

    fn handle_event(&mut self, index: usize, event: Event) {
        let peer = &mut self.peers[index];
        match event {
            Event::Connected => debug!("peer connected"),
            Event::IceConnectionStateChange(state) => {
                debug!(?state, "ICE state changed");
                if state == IceConnectionState::Disconnected {
                    peer.rtc.disconnect();
                }
            }
            Event::ChannelOpen(id, label) => match label.as_str() {
                CONTROL_CHANNEL => peer.control = Some(id),
                STATE_CHANNEL => peer.state = Some(id),
                _ => warn!(label, "unexpected data channel"),
            },
            Event::ChannelData(data) if Some(data.id) == peer.control => {
                self.handle_control(index, &data.data)
            }
            Event::ChannelData(data) if Some(data.id) == peer.state => {
                self.handle_state(index, &data.data)
            }
            Event::ChannelClose(id) if Some(id) == peer.control => peer.rtc.disconnect(),
            _ => {}
        }
    }

    fn handle_control(&mut self, index: usize, bytes: &[u8]) {
        let Ok(ClientMessage::Hello { protocol, name }) = decode(bytes) else {
            warn!("invalid message on the control channel");
            self.peers[index].rtc.disconnect();
            return;
        };
        if self.peers[index].player.is_some() {
            return;
        }
        if protocol != PROTOCOL_VERSION {
            let reason = format!(
                "this server speaks protocol version {PROTOCOL_VERSION}, the game sent {protocol}"
            );
            self.reject(index, reason);
            return;
        }
        if self.world.player_count() >= usize::from(self.slots) {
            self.reject(index, "the server is full".into());
            return;
        }

        let id = self.allocate_player_id();
        let name = sanitize_player_name(&name, id);
        self.world.spawn_player(id);
        let peer = &mut self.peers[index];
        peer.player = Some(id);
        peer.name.clone_from(&name);

        let players = self
            .peers
            .iter()
            .filter_map(|peer| {
                peer.player.map(|id| PlayerInfo {
                    id,
                    name: peer.name.clone(),
                })
            })
            .collect();
        self.peers[index].send_control(&ServerMessage::Welcome(Welcome {
            player_id: id,
            tick_rate: TICK_RATE,
            players,
        }));
        let joined = ServerMessage::PlayerJoined(PlayerInfo {
            id,
            name: name.clone(),
        });
        for (other, peer) in self.peers.iter_mut().enumerate() {
            if other != index && peer.player.is_some() {
                peer.send_control(&joined);
            }
        }
        self.publish_player_count();
        info!(id, name, "player joined");
    }

    fn handle_state(&mut self, index: usize, bytes: &[u8]) {
        // The state channel is unreliable; ignore whatever does not parse.
        let Ok(ClientMessage::Inputs { first_seq, buttons }) = decode(bytes) else {
            return;
        };
        let peer = &mut self.peers[index];
        if peer.player.is_none() || buttons.len() > INPUT_REDUNDANCY * 2 {
            return;
        }
        for (seq, buttons) in (first_seq..).zip(buttons) {
            if seq > peer.last_seq {
                peer.inputs.entry(seq).or_insert(buttons);
            }
        }
        while peer.inputs.len() > MAX_QUEUED_INPUTS {
            peer.inputs.pop_first();
        }
    }

    fn tick(&mut self) {
        for peer in &mut self.peers {
            let Some(id) = peer.player else { continue };
            // Normally one input per tick; two while catching up after a burst of late packets.
            let budget = if peer.inputs.len() > CATCH_UP_THRESHOLD {
                2
            } else {
                1
            };
            for _ in 0..budget {
                let Some((seq, buttons)) = peer.inputs.pop_first() else {
                    break;
                };
                self.world.apply_input(id, buttons);
                peer.last_seq = seq;
            }
        }
        self.world.step();

        for event in self.world.take_events() {
            let GameEvent::Kill { killer, victim } = event;
            self.broadcast_control(&ServerMessage::Kill { killer, victim });
        }

        if self.world.tick().is_multiple_of(SNAPSHOT_INTERVAL) {
            let mut message = ServerMessage::Snapshot(self.world.snapshot(0));
            for peer in &mut self.peers {
                let (Some(_), Some(channel)) = (peer.player, peer.state) else {
                    continue;
                };
                if let ServerMessage::Snapshot(snapshot) = &mut message {
                    snapshot.ack_seq = peer.last_seq;
                }
                if let Some(mut channel) = peer.rtc.channel(channel) {
                    // A full send buffer just drops this snapshot; the next one replaces it.
                    let _ = channel.write(true, &encode(&message));
                }
            }
        }
    }

    /// Drops dead, idle and rejected peers, and peers that never said hello.
    fn reap_peers(&mut self) {
        let now = Instant::now();
        let mut left = Vec::new();
        self.peers.retain_mut(|peer| {
            let handshake_expired =
                peer.player.is_none() && now - peer.connected_at > HANDSHAKE_TIMEOUT;
            let idle = now - peer.last_seen > IDLE_TIMEOUT;
            if handshake_expired || idle || peer.disconnect_at.is_some_and(|at| now >= at) {
                peer.rtc.disconnect();
            }
            if peer.rtc.is_alive() {
                return true;
            }
            if let Some(id) = peer.player {
                info!(id, name = peer.name, "player left");
                left.push(id);
            }
            false
        });
        if left.is_empty() {
            return;
        }
        for id in left {
            self.world.remove_player(id);
            self.broadcast_control(&ServerMessage::PlayerLeft { id });
        }
        self.publish_player_count();
    }

    fn reject(&mut self, index: usize, reason: String) {
        info!(reason, "rejected a player");
        let peer = &mut self.peers[index];
        peer.send_control(&ServerMessage::Rejected { reason });
        peer.disconnect_at = Some(Instant::now() + REJECT_LINGER);
    }

    fn broadcast_control(&mut self, message: &ServerMessage) {
        for peer in &mut self.peers {
            if peer.player.is_some() {
                peer.send_control(message);
            }
        }
    }

    fn allocate_player_id(&mut self) -> PlayerId {
        loop {
            let id = self.next_player_id;
            self.next_player_id = id.checked_add(1).unwrap_or(1);
            if !self.peers.iter().any(|peer| peer.player == Some(id)) {
                return id;
            }
        }
    }

    fn publish_player_count(&self) {
        let count = self
            .peers
            .iter()
            .filter(|peer| peer.player.is_some())
            .count();
        self.player_count
            .send_replace(u16::try_from(count).unwrap_or(u16::MAX));
    }
}

impl Peer {
    fn new(rtc: Rtc) -> Self {
        Self {
            rtc,
            control: None,
            state: None,
            player: None,
            name: String::new(),
            inputs: BTreeMap::new(),
            last_seq: 0,
            connected_at: Instant::now(),
            last_seen: Instant::now(),
            disconnect_at: None,
        }
    }

    fn handle_input(&mut self, input: Input<'_>) {
        if !self.rtc.is_alive() {
            return;
        }
        if let Err(error) = self.rtc.handle_input(input) {
            warn!(%error, "peer failed");
            self.rtc.disconnect();
        }
    }

    fn send_control(&mut self, message: &ServerMessage) {
        let Some(mut channel) = self.control.and_then(|id| self.rtc.channel(id)) else {
            return;
        };
        if !matches!(channel.write(true, &encode(message)), Ok(true)) {
            warn!(
                player = self.player,
                "control channel is backed up; dropped a message"
            );
        }
    }
}

fn sanitize_player_name(name: &str, id: PlayerId) -> String {
    let name: String = name
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_PLAYER_NAME_LEN)
        .collect();
    if name.is_empty() {
        format!("Player {id}")
    } else {
        name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn player_names_are_cleaned_up() {
        assert_eq!(sanitize_player_name("  Ferret \n", 3), "Ferret");
        assert_eq!(sanitize_player_name("   ", 3), "Player 3");
        assert_eq!(
            sanitize_player_name(&"ö".repeat(40), 3).chars().count(),
            MAX_PLAYER_NAME_LEN
        );
    }
}
