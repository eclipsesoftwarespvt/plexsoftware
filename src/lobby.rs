use std::io::ErrorKind;
use std::ops::Deref;
use std::sync::Arc;
use std::time::Duration;

use bytes::BytesMut;
use futures::FutureExt;
use minecraft_protocol::decoder::Decoder;
use minecraft_protocol::version::v1_14_4::login::{
    LoginPluginRequest, LoginPluginResponse, LoginStart, LoginSuccess, SetCompression,
};
use tokio::io::AsyncWriteExt;
use tokio::net::tcp::{ReadHalf, WriteHalf};
use tokio::net::TcpStream;
use tokio::select;
use tokio::time;

use crate::config::*;
use crate::forge;
use crate::mc::uuid;
use crate::net;
use crate::proto;
use crate::proto::client::{Client, ClientInfo, ClientState};
use crate::proto::packets::play::join_game::JoinGameData;
use crate::proto::{packet, packets};
use crate::proxy;
use crate::server::{Server, State};

pub const KEEP_ALIVE_INTERVAL: Duration = Duration::from_secs(10);

const SERVER_CONNECT_TIMEOUT: Duration = Duration::from_secs(2 * 60);

const SERVER_JOIN_GAME_TIMEOUT: Duration = Duration::from_secs(20);

const SERVER_WARMUP: Duration = Duration::from_secs(1);

pub async fn serve(
    client: &Client,
    client_info: ClientInfo,
    mut inbound: TcpStream,
    config: Arc<Config>,
    server: Arc<Server>,
    queue: BytesMut,
) -> Result<(), ()> {
    let (mut reader, mut writer) = inbound.split();

    if client.state() != ClientState::Login {
        error!(target: "plexpaper::lobby", "Client reached lobby service with invalid state: {:?}", client.state());
        return Err(());
    }

    if client_info.username.is_none() {
        error!(target: "plexpaper::lobby", "Client username is unknown, closing connection");
        return Err(());
    }

    let mut inbound_buf = queue;

    loop {

        let (packet, _raw) = match packet::read_packet(client, &mut inbound_buf, &mut reader).await
        {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            Err(_) => {
                error!(target: "plexpaper", "Closing connection, error occurred");
                break;
            }
        };

        let client_state = client.state();

        if client_state == ClientState::Login && packet.id == packets::login::SERVER_LOGIN_START {

            let login_start = LoginStart::decode(&mut packet.data.as_slice()).map_err(|_| ())?;

            debug!(target: "plexpaper::lobby", "Login on lobby server (user: {})", login_start.name);

            if config.server.forge {
                forge::replay_login_payload(client, &mut inbound, server.clone(), &mut inbound_buf)
                    .await?;
                let (_returned_reader, returned_writer) = inbound.split();
                writer = returned_writer;
            }

            if proto::COMPRESSION_THRESHOLD >= 0 {
                trace!(target: "plexpaper::lobby", "Enabling compression for lobby client because server has it enabled (threshold: {})", proto::COMPRESSION_THRESHOLD);
                respond_set_compression(client, &mut writer, proto::COMPRESSION_THRESHOLD).await?;
                client.set_compression(proto::COMPRESSION_THRESHOLD);
            }

            respond_login_success(client, &mut writer, &login_start).await?;
            client.set_state(ClientState::Play);

            trace!(target: "plexpaper::lobby", "Client login success, sending required play packets for lobby world");

            send_lobby_play_packets(client, &client_info, &mut writer, &server).await?;

            stage_wait(client, &client_info, &server, &config, &mut writer).await?;

            let server_client_info = client_info.clone();
            let (server_client, mut outbound, mut server_buf) =
                connect_to_server(&server_client_info, &inbound, &config).await?;
            let (returned_reader, returned_writer) = inbound.split();
            reader = returned_reader;
            writer = returned_writer;

            let join_game_data = wait_for_server_join_game(
                &server_client,
                &server_client_info,
                &mut outbound,
                &mut server_buf,
            )
            .await?;

            packets::play::title::send(client, &client_info, &mut writer, "").await?;

            play_lobby_ready_sound(client, &client_info, &mut writer, &config).await?;

            trace!(target: "plexpaper::lobby", "Waiting a second before relaying client connection...");
            time::sleep(SERVER_WARMUP).await;

            packets::play::respawn::lobby_send(client, &client_info, &mut writer, join_game_data)
                .await?;

            trace!(target: "plexpaper::lobby", "Voiding remaining incoming lobby client data before relay to real server");
            drain_stream(&mut reader).await?;

            debug!(target: "plexpaper::lobby", "Server connection ready, relaying lobby client to proxy");
            route_proxy(inbound, outbound, server_buf);

            return Ok(());
        }

        debug!(target: "plexpaper", "Got unhandled packet:");
        debug!(target: "plexpaper", "- State: {:?}", client_state);
        debug!(target: "plexpaper", "- Packet ID: 0x{:02X} ({})", packet.id, packet.id);
    }

    net::close_tcp_stream(inbound).await.map_err(|_| ())?;

    Ok(())
}

async fn respond_set_compression(
    client: &Client,
    writer: &mut WriteHalf<'_>,
    threshold: i32,
) -> Result<(), ()> {
    packet::write_packet(SetCompression { threshold }, client, writer).await
}

async fn respond_login_success(
    client: &Client,
    writer: &mut WriteHalf<'_>,
    login_start: &LoginStart,
) -> Result<(), ()> {
    packet::write_packet(
        LoginSuccess {
            uuid: uuid::offline_player_uuid(&login_start.name),
            username: login_start.name.clone(),
        },
        client,
        writer,
    )
    .await
}

async fn play_lobby_ready_sound(
    client: &Client,
    client_info: &ClientInfo,
    writer: &mut WriteHalf<'_>,
    config: &Config,
) -> Result<(), ()> {
    if let Some(sound_name) = config.join.lobby.ready_sound.as_ref() {

        if sound_name.trim().is_empty() {
            warn!(target: "plexpaper::lobby", "Lobby ready sound effect is an empty string, you should remove the configuration item instead");
            return Ok(());
        }

        packets::play::player_pos::send(client, client_info, writer).await?;
        packets::play::sound::send(client, client_info, writer, sound_name).await?;
    }

    Ok(())
}

async fn send_lobby_play_packets(
    client: &Client,
    client_info: &ClientInfo,
    writer: &mut WriteHalf<'_>,
    server: &Server,
) -> Result<(), ()> {

    packets::play::join_game::lobby_send(client, client_info, writer, server).await?;

    packets::play::server_brand::send(client, client_info, writer).await?;

    packets::play::player_pos::send(client, client_info, writer).await?;

    packets::play::time_update::send(client, client_info, writer).await?;

    Ok(())
}

async fn keep_alive_loop(
    client: &Client,
    client_info: &ClientInfo,
    writer: &mut WriteHalf<'_>,
    config: &Config,
) -> Result<(), ()> {
    let mut interval = time::interval(KEEP_ALIVE_INTERVAL);

    loop {
        interval.tick().await;

        trace!(target: "plexpaper::lobby", "Sending keep-alive sequence to lobby client");

        packets::play::keep_alive::send(client, client_info, writer).await?;
        packets::play::title::send(client, client_info, writer, &config.join.lobby.message).await?;

    }
}

async fn stage_wait(
    client: &Client,
    client_info: &ClientInfo,
    server: &Server,
    config: &Config,
    writer: &mut WriteHalf<'_>,
) -> Result<(), ()> {
    select! {
        a = keep_alive_loop(client, client_info, writer, config) => a,
        b = wait_for_server(server, config) => b,
    }
}

async fn wait_for_server(server: &Server, config: &Config) -> Result<(), ()> {
    debug!(target: "plexpaper::lobby", "Waiting on server to come online...");

    let task_wait = async {
        let mut state = server.state_receiver();
        loop {

            state.changed().await.unwrap();

            match state.borrow().deref() {

                State::Starting => {
                    trace!(target: "plexpaper::lobby", "Server not ready, holding client for longer");
                    continue;
                }

                State::Started => {
                    break true;
                }

                State::Stopping | State::Stopped => {
                    break false;
                }
            }
        }
    };

    let timeout = Duration::from_secs(config.join.lobby.timeout as u64);
    match time::timeout(timeout, task_wait).await {

        Ok(true) => {
            debug!(target: "plexpaper::lobby", "Server ready for lobby client");
            return Ok(());
        }

        Ok(false) => {}

        Err(_) => {
            warn!(target: "plexpaper::lobby", "Lobby client waiting for server to come online reached timeout of {}s", timeout.as_secs());
        }
    }

    Err(())
}

async fn connect_to_server(
    client_info: &ClientInfo,
    inbound: &TcpStream,
    config: &Config,
) -> Result<(Client, TcpStream, BytesMut), ()> {
    time::timeout(
        SERVER_CONNECT_TIMEOUT,
        connect_to_server_no_timeout(client_info, inbound, config),
    )
    .await
    .map_err(|_| {
        error!(target: "plexpaper::lobby", "Creating new server connection for lobby client timed out after {}s", SERVER_CONNECT_TIMEOUT.as_secs());
    })?
}

async fn connect_to_server_no_timeout(
    client_info: &ClientInfo,
    inbound: &TcpStream,
    config: &Config,
) -> Result<(Client, TcpStream, BytesMut), ()> {

    let mut outbound = TcpStream::connect(config.server.address)
        .await
        .map_err(|_| ())?;

    if config.server.send_proxy_v2 {
        trace!(target: "plexpaper::lobby", "Sending client proxy header for server connection");
        outbound
            .write_all(&proxy::stream_proxy_header(inbound).map_err(|_| ())?)
            .await
            .map_err(|_| ())?;
    }

    let tmp_client = match outbound.local_addr() {
        Ok(addr) => Client::new(addr),
        Err(_) => Client::dummy(),
    };
    tmp_client.set_state(ClientState::Login);

    let (mut reader, mut writer) = outbound.split();

    assert_eq!(
        client_info.handshake.as_ref().unwrap().next_state,
        ClientState::Login.to_id(),
        "Client handshake should have login as next state"
    );
    packet::write_packet(
        client_info.handshake.clone().unwrap(),
        &tmp_client,
        &mut writer,
    )
    .await?;

    packet::write_packet(
        LoginStart {
            name: client_info.username.clone().ok_or(())?,
        },
        &tmp_client,
        &mut writer,
    )
    .await?;

    let mut buf = BytesMut::new();

    loop {

        let (packet, _raw) = match packet::read_packet(&tmp_client, &mut buf, &mut reader).await {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            Err(_) => {
                error!(target: "plexpaper::lobby", "Closing connection, error occurred");
                break;
            }
        };

        let client_state = tmp_client.state();

        if client_state == ClientState::Login && packet.id == packets::login::CLIENT_SET_COMPRESSION
        {

            let set_compression =
                SetCompression::decode(&mut packet.data.as_slice()).map_err(|_| ())?;

            if set_compression.threshold != proto::COMPRESSION_THRESHOLD {
                error!(
                    target: "plexpaper::lobby",
                    "Compression threshold sent to lobby client does not match threshold from server, this may cause errors (client: {}, server: {})",
                    proto::COMPRESSION_THRESHOLD,
                    set_compression.threshold
                );
            }

            tmp_client.set_compression(set_compression.threshold);
            continue;
        }

        if client_state == ClientState::Login
            && packet.id == packets::login::CLIENT_ENCRYPTION_REQUEST
        {
            error!(
                target: "plexpaper::lobby",
                "Got encryption request from server, this is unsupported. Server must be in offline mode to use lobby.",
            );

            break;
        }

        if client_state == ClientState::Login
            && packet.id == packets::login::CLIENT_LOGIN_PLUGIN_REQUEST
        {

            let plugin_request =
                LoginPluginRequest::decode(&mut packet.data.as_slice()).map_err(|err| {
                    dbg!(err);
                })?;

            if config.server.forge {
                trace!(target: "plexpaper::lobby", "Got login plugin request from server, responding with Forge reply");

                forge::respond_login_plugin_request(&tmp_client, plugin_request, &mut writer)
                    .await?;

                continue;
            }

            warn!(target: "plexpaper::lobby", "Got unexpected login plugin request from server, you may need to enable Forge support");

            packet::write_packet(
                LoginPluginResponse {
                    message_id: plugin_request.message_id,
                    successful: false,
                    data: vec![],
                },
                &tmp_client,
                &mut writer,
            )
            .await?;

            continue;
        }

        if client_state == ClientState::Login && packet.id == packets::login::CLIENT_LOGIN_SUCCESS {
            trace!(target: "plexpaper::lobby", "Got login success from server connection, change to play mode");

            tmp_client.set_state(ClientState::Play);

            if tmp_client.is_compressed() != (proto::COMPRESSION_THRESHOLD >= 0) {
                error!(target: "plexpaper::lobby", "Compression enabled for lobby client while the server did not, this will cause errors");
            }

            return Ok((tmp_client, outbound, buf));
        }

        if client_state == ClientState::Login && packet.id == packets::login::CLIENT_DISCONNECT {
            error!(target: "plexpaper::lobby", "Got disconnect from server connection");

            break;
        }

        debug!(target: "plexpaper::lobby", "Got unhandled packet from server in connect_to_server:");
        debug!(target: "plexpaper::lobby", "- State: {:?}", client_state);
        debug!(target: "plexpaper::lobby", "- Packet ID: 0x{:02X} ({})", packet.id, packet.id);
    }

    net::close_tcp_stream(outbound).await.map_err(|_| ())?;

    Err(())
}

async fn wait_for_server_join_game(
    client: &Client,
    client_info: &ClientInfo,
    outbound: &mut TcpStream,
    buf: &mut BytesMut,
) -> Result<JoinGameData, ()> {
    time::timeout(
        SERVER_JOIN_GAME_TIMEOUT,
        wait_for_server_join_game_no_timeout(client, client_info, outbound, buf),
    )
    .await
    .map_err(|_| {
        error!(target: "plexpaper::lobby", "Waiting for for game data from server for lobby client timed out after {}s", SERVER_JOIN_GAME_TIMEOUT.as_secs());
    })?
}

async fn wait_for_server_join_game_no_timeout(
    client: &Client,
    client_info: &ClientInfo,
    outbound: &mut TcpStream,
    buf: &mut BytesMut,
) -> Result<JoinGameData, ()> {
    let (mut reader, mut _writer) = outbound.split();

    loop {

        let (packet, _raw) = match packet::read_packet(client, buf, &mut reader).await {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            Err(_) => {
                error!(target: "plexpaper::lobby", "Closing connection, error occurred");
                break;
            }
        };

        if packets::play::join_game::is_packet(client_info, packet.id) {

            let join_game_data = JoinGameData::from_packet(client_info, packet).map_err(|err| {
                warn!(target: "plexpaper::lobby", "Failed to parse join game packet: {:?}", err);
            })?;

            return Ok(join_game_data);
        }

        debug!(target: "plexpaper::lobby", "Got unhandled packet from server in wait_for_server_join_game:");
        debug!(target: "plexpaper::lobby", "- Packet ID: 0x{:02X} ({})", packet.id, packet.id);
    }

    net::close_tcp_stream_ref(outbound).await.map_err(|_| ())?;

    Err(())
}

#[inline]
pub fn route_proxy(inbound: TcpStream, outbound: TcpStream, inbound_queue: BytesMut) {

    let service = async move {
        proxy::proxy_inbound_outbound_with_queue(inbound, outbound, &inbound_queue, &[])
            .map(|r| {
                if let Err(err) = r {
                    warn!(target: "plexpaper", "Failed to proxy: {}", err);
                }
            })
            .await
    };

    tokio::spawn(service);
}

async fn drain_stream(reader: &mut ReadHalf<'_>) -> Result<(), ()> {
    let mut drain_buf = [0; 8 * 1024];
    loop {
        match reader.try_read(&mut drain_buf) {
            Ok(0) => return Ok(()),
            Err(err) if err.kind() == ErrorKind::WouldBlock => return Ok(()),
            Ok(_) => continue,
            Err(err) => {
                error!(target: "plexpaper::lobby", "Failed to drain lobby client connection before relaying to real server. Maybe already disconnected? Error: {:?}", err);
                return Ok(());
            }
        }
    }
}
