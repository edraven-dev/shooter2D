//! End to end over real sockets: the lobby and a gameserver run in-process, and str0m clients
//! play the browsers (offer through the lobby, DTLS, SCTP, both data channels).

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use protocol::game::{
    Buttons, CONTROL_CHANNEL, ClientMessage, INPUT_REDUNDANCY, PROTOCOL_VERSION, PlayerId,
    PlayerState, STATE_CHANNEL, ServerMessage, Snapshot, decode, encode,
};
use protocol::lobby::{ServerSummary, SessionDescription};
use str0m::change::SdpAnswer;
use str0m::channel::{ChannelConfig, ChannelId, Reliability};
use str0m::net::{Protocol, Receive};
use str0m::{Candidate, Event, Input, Output, Rtc};
use tokio::net::{TcpListener, UdpSocket};

const TIMEOUT: Duration = Duration::from_secs(10);

struct Harness {
    lobby: SocketAddr,
    server_id: String,
}

async fn start(slots: u16) -> Harness {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let lobby = listener.local_addr().unwrap();
    tokio::spawn(lobby::serve(
        listener,
        lobby::Config::default(),
        std::future::pending(),
    ));
    let config = gameserver::Config {
        name: "E2E".into(),
        slots,
        lobby_url: format!("ws://{lobby}/api/gameservers"),
        lobby_token: None,
        bind: "127.0.0.1:0".parse().unwrap(),
        public_ip: None,
    };
    tokio::spawn(gameserver::run(config, std::future::pending()));
    let servers = wait_for_servers(lobby, |servers| !servers.is_empty()).await;
    Harness {
        lobby,
        server_id: servers[0].id.clone(),
    }
}

async fn wait_for_servers(
    lobby: SocketAddr,
    check: impl Fn(&[ServerSummary]) -> bool,
) -> Vec<ServerSummary> {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let servers: Vec<ServerSummary> = reqwest::get(format!("http://{lobby}/api/servers"))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if check(&servers) {
            return servers;
        }
        assert!(
            Instant::now() < deadline,
            "server list never matched: {servers:?}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// A browser stand-in: offers two data channels through the lobby and speaks the game protocol.
struct Client {
    name: String,
    rtc: Rtc,
    socket: UdpSocket,
    local: SocketAddr,
    control: ChannelId,
    state: ChannelId,
    state_open: bool,
    player_id: Option<PlayerId>,
    messages: Vec<ServerMessage>,
    snapshot: Option<Snapshot>,
    inputs: Vec<Buttons>,
}

impl Client {
    async fn connect(harness: &Harness, name: &str) -> Self {
        str0m::crypto::from_feature_flags().install_process_default();
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let local = socket.local_addr().unwrap();
        let mut rtc = Rtc::builder().build(Instant::now());
        rtc.add_local_candidate(Candidate::host(local, "udp").unwrap());

        let mut changes = rtc.sdp_api();
        let control = changes.add_channel(CONTROL_CHANNEL.into());
        let state = changes.add_channel_with_config(ChannelConfig {
            label: STATE_CHANNEL.into(),
            ordered: false,
            reliability: Reliability::MaxRetransmits { retransmits: 0 },
            ..ChannelConfig::default()
        });
        let (offer, pending) = changes.apply().unwrap();

        let response = reqwest::Client::new()
            .post(format!(
                "http://{}/api/servers/{}/connect",
                harness.lobby, harness.server_id
            ))
            .json(&SessionDescription {
                sdp: offer.to_sdp_string(),
            })
            .send()
            .await
            .unwrap();
        assert!(
            response.status().is_success(),
            "connect failed: {}",
            response.text().await.unwrap()
        );
        let answer: SessionDescription = response.json().await.unwrap();
        rtc.sdp_api()
            .accept_answer(pending, SdpAnswer::from_sdp_string(&answer.sdp).unwrap())
            .unwrap();

        Self {
            name: name.into(),
            rtc,
            socket,
            local,
            control,
            state,
            state_open: false,
            player_id: None,
            messages: Vec::new(),
            snapshot: None,
            inputs: Vec::new(),
        }
    }

    /// Runs the connection until `done` holds.
    async fn run_until(&mut self, mut done: impl FnMut(&Self) -> bool) {
        let deadline = Instant::now() + TIMEOUT;
        let mut buf = vec![0; 2000];
        loop {
            let wake_at = self.flush().await;
            if done(self) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "{} timed out; control messages: {:?}",
                self.name,
                self.messages
            );
            tokio::select! {
                received = self.socket.recv_from(&mut buf) => {
                    let (len, source) = received.unwrap();
                    let contents = buf[..len].try_into().unwrap();
                    let receive = Receive { proto: Protocol::Udp, source, destination: self.local, contents };
                    self.rtc.handle_input(Input::Receive(Instant::now(), receive)).unwrap();
                }
                () = tokio::time::sleep_until(wake_at.min(deadline).into()) => {
                    self.rtc.handle_input(Input::Timeout(Instant::now())).unwrap();
                }
            }
        }
    }

    /// Sends everything str0m wants to send and handles its events; returns its next timeout.
    async fn flush(&mut self) -> Instant {
        loop {
            match self.rtc.poll_output().unwrap() {
                Output::Timeout(at) => return at,
                Output::Transmit(transmit) => {
                    self.socket
                        .send_to(&transmit.contents, transmit.destination)
                        .await
                        .unwrap();
                }
                Output::Event(Event::ChannelOpen(id, _)) if id == self.control => {
                    let hello = ClientMessage::Hello {
                        protocol: PROTOCOL_VERSION,
                        name: self.name.clone(),
                    };
                    self.write(self.control, &encode(&hello));
                }
                Output::Event(Event::ChannelOpen(id, _)) if id == self.state => {
                    self.state_open = true
                }
                Output::Event(Event::ChannelData(data)) => {
                    match decode::<ServerMessage>(&data.data).unwrap() {
                        ServerMessage::Snapshot(snapshot) => self.snapshot = Some(snapshot),
                        message => {
                            if let ServerMessage::Welcome(welcome) = &message {
                                self.player_id = Some(welcome.player_id);
                            }
                            self.messages.push(message);
                        }
                    }
                }
                Output::Event(_) => {}
            }
        }
    }

    fn write(&mut self, channel: ChannelId, bytes: &[u8]) {
        assert!(
            self.rtc
                .channel(channel)
                .unwrap()
                .write(true, bytes)
                .unwrap(),
            "send buffer full"
        );
    }

    /// Holds `buttons` for `ticks` ticks, sending inputs the way the browser client does.
    async fn hold(&mut self, buttons: Buttons, ticks: usize) {
        for _ in 0..ticks {
            self.inputs.push(buttons);
            let last_seq = u32::try_from(self.inputs.len()).unwrap();
            let first_seq = last_seq
                .saturating_sub(u32::try_from(INPUT_REDUNDANCY).unwrap() - 1)
                .max(1);
            let recent = self.inputs[first_seq as usize - 1..].to_vec();
            self.write(
                self.state,
                &encode(&ClientMessage::Inputs {
                    first_seq,
                    buttons: recent,
                }),
            );
            let next = Instant::now() + Duration::from_millis(16);
            self.run_until(|_| Instant::now() >= next).await;
        }
    }

    async fn join(&mut self) -> PlayerId {
        self.run_until(|client| {
            client.player_id.is_some() && client.state_open && client.snapshot.is_some()
        })
        .await;
        self.player_id.unwrap()
    }

    fn player(&self, id: PlayerId) -> PlayerState {
        *self
            .snapshot
            .as_ref()
            .unwrap()
            .players
            .iter()
            .find(|player| player.id == id)
            .unwrap()
    }
}

#[tokio::test]
async fn server_movement_matches_client_prediction_bit_for_bit() {
    let harness = start(4).await;
    let mut alice = Client::connect(&harness, "Alice").await;
    let id = alice.join().await;
    let spawn = alice.player(id);
    assert_eq!(alice.snapshot.as_ref().unwrap().ack_seq, 0);

    let inputs = [
        Buttons::FORWARD,
        Buttons::FORWARD | Buttons::TURN_RIGHT,
        Buttons::STRAFE_LEFT,
    ];
    for buttons in inputs {
        alice.hold(buttons, 20).await;
    }
    alice
        .run_until(|client| {
            client
                .snapshot
                .as_ref()
                .is_some_and(|snapshot| snapshot.ack_seq == 60)
        })
        .await;

    // Replay the acknowledged inputs locally, as client-side prediction does.
    let level = sim::Level::arena();
    let mut body = sim::Body {
        position: sim::Vec2::new(spawn.x, spawn.y),
        rotation: spawn.rotation,
    };
    for &buttons in &alice.inputs {
        body = sim::move_body(&level, body, buttons).0;
    }
    let server = alice.player(id);
    assert_ne!(
        (server.x, server.y),
        (spawn.x, spawn.y),
        "the player never moved"
    );
    assert_eq!(
        (
            server.x.to_bits(),
            server.y.to_bits(),
            server.rotation.to_bits()
        ),
        (
            body.position.x.to_bits(),
            body.position.y.to_bits(),
            body.rotation.to_bits()
        ),
        "server {server:?} vs predicted {body:?}"
    );
}

#[tokio::test]
async fn players_see_each_other_join_and_leave() {
    let harness = start(4).await;
    let mut alice = Client::connect(&harness, "Alice").await;
    let alice_id = alice.join().await;
    let mut bob = Client::connect(&harness, "  Bob  ").await;
    let bob_id = bob.join().await;

    let Some(ServerMessage::Welcome(welcome)) = bob.messages.first() else {
        panic!("expected a welcome")
    };
    let mut roster: Vec<_> = welcome
        .players
        .iter()
        .map(|player| (player.id, player.name.as_str()))
        .collect();
    roster.sort_unstable();
    assert_eq!(roster, [(alice_id, "Alice"), (bob_id, "Bob")]);

    alice
        .run_until(|client| {
            client.messages.iter().any(|message| matches!(message, ServerMessage::PlayerJoined(player) if player.id == bob_id))
                && client.snapshot.as_ref().is_some_and(|snapshot| snapshot.players.len() == 2)
        })
        .await;
    wait_for_servers(harness.lobby, |servers| servers[0].players == 2).await;

    // Closing the control channel is how a client says goodbye (browsers do it on `pc.close()`).
    // The server must notice right away, not only after its idle timeout.
    let closed_at = Instant::now();
    bob.rtc.direct_api().close_data_channel(bob.control);
    tokio::select! {
        () = alice.run_until(|client| {
            client.messages.iter().any(|message| matches!(message, ServerMessage::PlayerLeft { id } if *id == bob_id))
        }) => {}
        () = bob.run_until(|_| false) => unreachable!(),
    }
    assert!(
        closed_at.elapsed() < Duration::from_secs(3),
        "leaving took {:?}",
        closed_at.elapsed()
    );
    wait_for_servers(harness.lobby, |servers| servers[0].players == 1).await;
}

#[tokio::test]
async fn full_servers_reject_offers() {
    let harness = start(1).await;
    let mut alice = Client::connect(&harness, "Alice").await;
    alice.join().await;

    let response = reqwest::Client::new()
        .post(format!(
            "http://{}/api/servers/{}/connect",
            harness.lobby, harness.server_id
        ))
        .json(&SessionDescription { sdp: "v=0".into() })
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::CONFLICT);
}
