//! Drives the lobby over real sockets: reqwest plays the browser, tokio-tungstenite the gameserver.

use std::net::SocketAddr;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use protocol::lobby::{
    ApiError, GameserverMessage, LobbyMessage, ServerSummary, SessionDescription,
};
use reqwest::StatusCode;
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

async fn start(config: lobby::Config) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(lobby::serve(listener, config, std::future::pending()));
    addr
}

async fn connect_gameserver(
    addr: SocketAddr,
    token: Option<&str>,
) -> Result<Socket, tokio_tungstenite::tungstenite::Error> {
    let mut request = format!("ws://{addr}/api/gameservers")
        .into_client_request()
        .unwrap();
    if let Some(token) = token {
        request
            .headers_mut()
            .insert("authorization", format!("Bearer {token}").parse().unwrap());
    }
    Ok(tokio_tungstenite::connect_async(request).await?.0)
}

async fn send(socket: &mut Socket, message: &GameserverMessage) {
    socket
        .send(Message::text(serde_json::to_string(message).unwrap()))
        .await
        .unwrap();
}

async fn receive(socket: &mut Socket) -> LobbyMessage {
    loop {
        match tokio::time::timeout(Duration::from_secs(5), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
        {
            Message::Text(text) => return serde_json::from_str(text.as_str()).unwrap(),
            _ => continue,
        }
    }
}

/// Connects and registers a gameserver, returning its socket and lobby id.
async fn register(addr: SocketAddr, name: &str, slots: u16) -> (Socket, String) {
    let mut socket = connect_gameserver(addr, None).await.unwrap();
    send(
        &mut socket,
        &GameserverMessage::Register {
            name: name.into(),
            slots,
        },
    )
    .await;
    let LobbyMessage::Registered { id } = receive(&mut socket).await else {
        panic!("expected Registered")
    };
    (socket, id)
}

async fn servers(addr: SocketAddr) -> Vec<ServerSummary> {
    reqwest::get(format!("http://{addr}/api/servers"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

/// Polls the server list until `check` passes (status updates are asynchronous).
async fn wait_for_servers(
    addr: SocketAddr,
    check: impl Fn(&[ServerSummary]) -> bool,
) -> Vec<ServerSummary> {
    for _ in 0..100 {
        let list = servers(addr).await;
        if check(&list) {
            return list;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!(
        "the server list never matched; last: {:?}",
        servers(addr).await
    );
}

async fn post_offer(addr: SocketAddr, id: &str, sdp: &str) -> reqwest::Response {
    reqwest::Client::new()
        .post(format!("http://{addr}/api/servers/{id}/connect"))
        .json(&SessionDescription { sdp: sdp.into() })
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn lists_registered_gameservers_sorted_by_name() {
    let addr = start(lobby::Config::default()).await;
    assert!(servers(addr).await.is_empty());

    let (mut zulu, zulu_id) = register(addr, "Zulu", 8).await;
    let (_alpha, alpha_id) = register(addr, "  Alpha  ", 1000).await;
    send(&mut zulu, &GameserverMessage::Status { players: 3 }).await;

    let list = wait_for_servers(addr, |list| list.len() == 2 && list[1].players == 3).await;
    assert_eq!(
        list,
        vec![
            ServerSummary {
                id: alpha_id,
                name: "Alpha".into(),
                players: 0,
                slots: 64
            },
            ServerSummary {
                id: zulu_id,
                name: "Zulu".into(),
                players: 3,
                slots: 8
            },
        ]
    );
}

#[tokio::test]
async fn relays_offers_and_answers() {
    let addr = start(lobby::Config::default()).await;
    let (mut gameserver, id) = register(addr, "Relay", 8).await;

    let answering = tokio::spawn(async move {
        let LobbyMessage::Offer { request_id, sdp } = receive(&mut gameserver).await else {
            panic!("expected Offer")
        };
        assert_eq!(sdp, "offer-sdp");
        send(
            &mut gameserver,
            &GameserverMessage::Answer {
                request_id,
                sdp: "answer-sdp".into(),
            },
        )
        .await;
        gameserver
    });

    let response = post_offer(addr, &id, "offer-sdp").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.json::<SessionDescription>().await.unwrap().sdp,
        "answer-sdp"
    );
    answering.await.unwrap();
}

#[tokio::test]
async fn reports_rejections() {
    let addr = start(lobby::Config::default()).await;
    let (mut gameserver, id) = register(addr, "Picky", 8).await;
    tokio::spawn(async move {
        let LobbyMessage::Offer { request_id, .. } = receive(&mut gameserver).await else {
            panic!("expected Offer")
        };
        send(
            &mut gameserver,
            &GameserverMessage::Reject {
                request_id,
                reason: "invalid offer".into(),
            },
        )
        .await;
        gameserver
    });

    let response = post_offer(addr, &id, "garbage").await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(
        response.json::<ApiError>().await.unwrap().error,
        "invalid offer"
    );
}

#[tokio::test]
async fn refuses_unknown_and_full_servers() {
    let addr = start(lobby::Config::default()).await;
    assert_eq!(
        post_offer(addr, "nope", "sdp").await.status(),
        StatusCode::NOT_FOUND
    );

    let (mut gameserver, id) = register(addr, "Tiny", 1).await;
    send(&mut gameserver, &GameserverMessage::Status { players: 1 }).await;
    wait_for_servers(addr, |list| {
        list.first().is_some_and(|server| server.players == 1)
    })
    .await;
    let response = post_offer(addr, &id, "sdp").await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(
        response.json::<ApiError>().await.unwrap().error,
        "the server is full"
    );
}

#[tokio::test]
async fn times_out_when_the_gameserver_does_not_answer() {
    let config = lobby::Config {
        answer_timeout: Duration::from_millis(200),
        ..Default::default()
    };
    let addr = start(config).await;
    let (_silent, id) = register(addr, "Silent", 8).await;
    assert_eq!(
        post_offer(addr, &id, "sdp").await.status(),
        StatusCode::GATEWAY_TIMEOUT
    );
}

#[tokio::test]
async fn forgets_gameservers_that_disconnect() {
    let addr = start(lobby::Config::default()).await;
    let (gameserver, id) = register(addr, "Leaving", 8).await;
    wait_for_servers(addr, |list| list.len() == 1).await;

    // A browser waiting for an answer gets an error instead of hanging.
    let waiting = tokio::spawn(async move { post_offer(addr, &id, "sdp").await.status() });
    tokio::time::sleep(Duration::from_millis(100)).await;
    drop(gameserver);

    assert_eq!(waiting.await.unwrap(), StatusCode::BAD_GATEWAY);
    wait_for_servers(addr, <[ServerSummary]>::is_empty).await;
}

#[tokio::test]
async fn requires_the_gameserver_token_when_configured() {
    let config = lobby::Config {
        gameserver_token: Some("s3cret".into()),
        ..Default::default()
    };
    let addr = start(config).await;

    let refused = connect_gameserver(addr, None).await.unwrap_err();
    assert!(refused.to_string().contains("401"), "{refused}");
    assert!(connect_gameserver(addr, Some("wrong")).await.is_err());

    let mut socket = connect_gameserver(addr, Some("s3cret")).await.unwrap();
    send(
        &mut socket,
        &GameserverMessage::Register {
            name: "Trusted".into(),
            slots: 8,
        },
    )
    .await;
    assert!(matches!(
        receive(&mut socket).await,
        LobbyMessage::Registered { .. }
    ));
}
