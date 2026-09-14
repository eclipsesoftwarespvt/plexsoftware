use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::BytesMut;
use minecraft_protocol::decoder::Decoder;
use minecraft_protocol::version::v1_14_4::handshake::Handshake;
use minecraft_protocol::version::v1_20_3::status::{
    PingRequest, PingResponse, ServerStatus, StatusRequest, StatusResponse,
};
use rand::Rng;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::time;

use crate::config::Config;
use crate::proto::client::{Client, ClientState};
use crate::proto::{packet, packets};
use crate::proxy;
use crate::server::{Server, State};

const MONITOR_POLL_INTERVAL: Duration = Duration::from_secs(2);

const STATUS_TIMEOUT: u64 = 20;

const PING_TIMEOUT: u64 = 10;

pub async fn monitor_server(config: Arc<Config>, server: Arc<Server>) {

    let addr = config.server.address;

    let mut poll_interval = time::interval(MONITOR_POLL_INTERVAL);

    loop {
        poll_interval.tick().await;

        trace!(target: "plexpaper::monitor", "Fetching status for {} ... ", addr);
        let status = poll_server(&config, &server, addr).await;
        match status {

            Ok(Some(status)) => server.update_status(&config, Some(status)).await,

            Err(_) => server.update_status(&config, None).await,

            Ok(None) => {
                warn!(target: "plexpaper::monitor", "Failed to poll server status, ping fallback succeeded");
            }
        }

        if server.should_sleep(&config).await {
            info!(target: "plexpaper::monitor", "Server has been idle, sleeping...");
            server.stop(&config).await;
        }

        if server.should_kill().await {
            error!(target: "plexpaper::monitor", "Force killing server, took too long to start or stop");
            if !server.force_kill().await {
                warn!(target: "plexpaper", "Failed to force kill server");
            }
        }
    }
}

pub async fn poll_server(
    config: &Config,
    server: &Server,
    addr: SocketAddr,
) -> Result<Option<ServerStatus>, ()> {

    if let Ok(status) = fetch_status(config, addr).await {
        return Ok(Some(status));
    }

    if server.state() == State::Started {
        debug!(target: "plexpaper::monitor", "Failed to get status from started server, trying ping...");
        do_ping(config, addr).await?;
    }

    Err(())
}

async fn fetch_status(config: &Config, addr: SocketAddr) -> Result<ServerStatus, ()> {
    let mut stream = TcpStream::connect(addr).await.map_err(|_| ())?;

    if config.server.send_proxy_v2 {
        trace!(target: "plexpaper::monitor", "Sending local proxy header for server connection");
        stream
            .write_all(&proxy::local_proxy_header().map_err(|_| ())?)
            .await
            .map_err(|_| ())?;
    }

    let client = Client::dummy();

    send_handshake(&client, &mut stream, config, addr).await?;
    request_status(&client, &mut stream).await?;
    wait_for_status_timeout(&client, &mut stream).await
}

async fn do_ping(config: &Config, addr: SocketAddr) -> Result<(), ()> {
    let mut stream = TcpStream::connect(addr).await.map_err(|_| ())?;

    if config.server.send_proxy_v2 {
        trace!(target: "plexpaper::monitor", "Sending local proxy header for server connection");
        stream
            .write_all(&proxy::local_proxy_header().map_err(|_| ())?)
            .await
            .map_err(|_| ())?;
    }

    let client = Client::dummy();

    send_handshake(&client, &mut stream, config, addr).await?;
    let token = send_ping(&client, &mut stream).await?;
    wait_for_ping_timeout(&client, &mut stream, token).await
}

async fn send_handshake(
    client: &Client,
    stream: &mut TcpStream,
    config: &Config,
    addr: SocketAddr,
) -> Result<(), ()> {
    packet::write_packet(
        Handshake {
            protocol_version: config.public.protocol as i32,
            server_addr: addr.ip().to_string(),
            server_port: addr.port(),
            next_state: ClientState::Status.to_id(),
        },
        client,
        &mut stream.split().1,
    )
    .await
}

async fn request_status(client: &Client, stream: &mut TcpStream) -> Result<(), ()> {
    packet::write_packet(StatusRequest {}, client, &mut stream.split().1).await
}

async fn send_ping(client: &Client, stream: &mut TcpStream) -> Result<u64, ()> {
    let token = rand::thread_rng().gen();
    packet::write_packet(PingRequest { time: token }, client, &mut stream.split().1).await?;
    Ok(token)
}

async fn wait_for_status(client: &Client, stream: &mut TcpStream) -> Result<ServerStatus, ()> {

    let (mut reader, mut _writer) = stream.split();
    let mut buf = BytesMut::new();

    loop {

        let (packet, _raw) = match packet::read_packet(client, &mut buf, &mut reader).await {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            Err(_) => continue,
        };

        if packet.id == packets::status::CLIENT_STATUS {
            let status = StatusResponse::decode(&mut packet.data.as_slice()).map_err(|err| {
                debug!(target: "plexpaper", "Failed to decode status response from backend server, ignoring: {err:?}");
            })?;
            return Ok(status.server_status);
        }
    }

    Err(())
}

async fn wait_for_status_timeout(
    client: &Client,
    stream: &mut TcpStream,
) -> Result<ServerStatus, ()> {
    let status = wait_for_status(client, stream);
    tokio::time::timeout(Duration::from_secs(STATUS_TIMEOUT), status)
        .await
        .map_err(|_| ())?
}

async fn wait_for_ping(client: &Client, stream: &mut TcpStream, token: u64) -> Result<(), ()> {

    let (mut reader, mut _writer) = stream.split();
    let mut buf = BytesMut::new();

    loop {

        let (packet, _raw) = match packet::read_packet(client, &mut buf, &mut reader).await {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            Err(_) => continue,
        };

        if packet.id == packets::status::CLIENT_PING {
            let ping = PingResponse::decode(&mut packet.data.as_slice()).map_err(|_| ())?;

            if ping.time == token {
                return Ok(());
            } else {
                debug!(target: "plexpaper", "Got unmatched ping response when polling server status by ping");
            }
        }
    }

    Err(())
}

async fn wait_for_ping_timeout(
    client: &Client,
    stream: &mut TcpStream,
    token: u64,
) -> Result<(), ()> {
    let status = wait_for_ping(client, stream, token);
    tokio::time::timeout(Duration::from_secs(PING_TIMEOUT), status)
        .await
        .map_err(|_| ())?
}
