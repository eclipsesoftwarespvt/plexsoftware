use std::ops::Deref;
use std::sync::Arc;
use std::time::Duration;

use bytes::BytesMut;
use minecraft_protocol::decoder::Decoder;
use minecraft_protocol::version::v1_14_4::handshake::Handshake;
use minecraft_protocol::version::v1_14_4::login::{
    LoginPluginRequest, LoginPluginResponse, LoginStart, SetCompression,
};
use tokio::net::TcpStream;
use tokio::time;

use crate::config::Config;
use crate::forge;
use crate::net;
use crate::proto::client::{Client, ClientInfo, ClientState};
use crate::proto::packets::play::join_game::JoinGameData;
use crate::proto::{self, packet, packets};
use crate::server::{Server, State};

const PROBE_USER: &str = "_plexpaper_probe";

const PROBE_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

const PROBE_ONLINE_TIMEOUT: Duration = Duration::from_secs(10 * 60);

const PROBE_JOIN_GAME_TIMEOUT: Duration = Duration::from_secs(20);

pub async fn probe(config: Arc<Config>, server: Arc<Server>) -> Result<(), ()> {
    debug!(target: "plexpaper::probe", "Starting server probe...");

    if Server::start(config.clone(), server.clone(), None).await {
        info!(target: "plexpaper::probe", "Starting server to probe...");
    }

    if !wait_until_online(&server).await? {
        warn!(target: "plexpaper::probe", "Couldn't probe server, failed to wait for server to come online");
        return Err(());
    }

    debug!(target: "plexpaper::probe", "Connecting to server to probe details...");

    let forge_payload = connect_to_server(&config, &server).await?;
    *server.forge_payload.write().await = forge_payload;

    Ok(())
}

async fn wait_until_online<'a>(server: &Server) -> Result<bool, ()> {
    trace!(target: "plexpaper::probe", "Waiting for server to come online...");

    let task_wait = async {
        let mut state = server.state_receiver();
        loop {

            state.changed().await.unwrap();

            match state.borrow().deref() {

                State::Starting => {
                    continue;
                }

                State::Started => {
                    break true;
                }

                State::Stopping => {
                    warn!(target: "plexpaper::probe", "Server stopping while trying to probe, skipping");
                    break false;
                }

                State::Stopped => {
                    error!(target: "plexpaper::probe", "Server stopped while trying to probe, skipping");
                    break false;
                }
            }
        }
    };

    match time::timeout(PROBE_ONLINE_TIMEOUT, task_wait).await {
        Ok(online) => Ok(online),

        Err(_) => {
            warn!(target: "plexpaper::probe", "Probe waited for server to come online but timed out after {}s", PROBE_ONLINE_TIMEOUT.as_secs());
            Ok(false)
        }
    }
}

async fn connect_to_server(config: &Config, server: &Server) -> Result<Vec<Vec<u8>>, ()> {
    time::timeout(
        PROBE_CONNECT_TIMEOUT,
        connect_to_server_no_timeout(config, server),
    )
    .await
    .map_err(|_| {
        error!(target: "plexpaper::probe", "Probe tried to connect to server but timed out after {}s", PROBE_CONNECT_TIMEOUT.as_secs());
    })?
}

async fn connect_to_server_no_timeout(
    config: &Config,
    server: &Server,
) -> Result<Vec<Vec<u8>>, ()> {

    let mut outbound = TcpStream::connect(config.server.address)
        .await
        .map_err(|_| ())?;

    let tmp_client = match outbound.local_addr() {
        Ok(addr) => Client::new(addr),
        Err(_) => Client::dummy(),
    };
    tmp_client.set_state(ClientState::Login);

    let mut tmp_client_info = ClientInfo::empty();
    tmp_client_info.protocol.replace(config.public.protocol);

    let (mut reader, mut writer) = outbound.split();

    let server_addr = if config.server.forge {
        format!("{}{}", config.server.address.ip(), forge::STATUS_MAGIC)
    } else {
        config.server.address.ip().to_string()
    };

    packet::write_packet(
        Handshake {
            protocol_version: config.public.protocol as i32,
            server_addr,
            server_port: config.server.address.port(),
            next_state: ClientState::Login.to_id(),
        },
        &tmp_client,
        &mut writer,
    )
    .await?;

    packet::write_packet(
        LoginStart {
            name: PROBE_USER.into(),
        },
        &tmp_client,
        &mut writer,
    )
    .await?;

    let mut buf = BytesMut::new();
    let mut forge_payload = Vec::new();

    loop {

        let (packet, raw) = match packet::read_packet(&tmp_client, &mut buf, &mut reader).await {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            Err(_) => {
                error!(target: "plexpaper::forge", "Closing connection, error occurred");
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
                    target: "plexpaper::forge",
                    "Compression threshold sent to lobby client does not match threshold from server, this may cause errors (client: {}, server: {})",
                    proto::COMPRESSION_THRESHOLD,
                    set_compression.threshold
                );
            }

            tmp_client.set_compression(set_compression.threshold);
            continue;
        }

        if client_state == ClientState::Login
            && packet.id == packets::login::CLIENT_LOGIN_PLUGIN_REQUEST
        {

            let plugin_request = LoginPluginRequest::decode(&mut packet.data.as_slice()).map_err(|err| {
                error!(target: "plexpaper::probe", "Failed to decode login plugin request from server, cannot respond properly: {:?}", err);
            })?;

            if config.server.forge {

                forge_payload.push(raw);

                forge::respond_login_plugin_request(&tmp_client, plugin_request, &mut writer)
                    .await?;
                continue;
            }

            warn!(target: "plexpaper::probe", "Got unexpected login plugin request, responding with error");

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
            trace!(target: "plexpaper::probe", "Got login success from server connection, change to play mode");

            tmp_client.set_state(ClientState::Play);

            let join_game_data =
                wait_for_server_join_game(&tmp_client, &tmp_client_info, &mut outbound, &mut buf)
                    .await?;
            server
                .probed_join_game
                .write()
                .await
                .replace(join_game_data);

            let _ = net::close_tcp_stream(outbound).await;

            return Ok(forge_payload);
        }

        debug!(target: "plexpaper::forge", "Got unhandled packet from server in connect_to_server:");
        debug!(target: "plexpaper::forge", "- State: {:?}", client_state);
        debug!(target: "plexpaper::forge", "- Packet ID: 0x{:02X} ({})", packet.id, packet.id);
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
        PROBE_JOIN_GAME_TIMEOUT,
        wait_for_server_join_game_no_timeout(client, client_info, outbound, buf),
    )
    .await
    .map_err(|_| {
        error!(target: "plexpaper::probe", "Waiting for for game data from server for probe client timed out after {}s", PROBE_JOIN_GAME_TIMEOUT.as_secs());
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
                error!(target: "plexpaper::probe", "Closing connection, error occurred");
                break;
            }
        };

        if packets::play::join_game::is_packet(client_info, packet.id) {

            let join_game_data = JoinGameData::from_packet(client_info, packet).map_err(|err| {
                warn!(target: "plexpaper::probe", "Failed to parse join game packet: {:?}", err);
            })?;

            return Ok(join_game_data);
        }

        debug!(target: "plexpaper::probe", "Got unhandled packet from server in wait_for_server_join_game:");
        debug!(target: "plexpaper::probe", "- Packet ID: 0x{:02X} ({})", packet.id, packet.id);
    }

    net::close_tcp_stream_ref(outbound).await.map_err(|_| ())?;

    Err(())
}
