use std::sync::Arc;

use bytes::BytesMut;
use minecraft_protocol::data::server_status::{OnlinePlayers, ServerVersion};
use minecraft_protocol::decoder::Decoder;
use minecraft_protocol::encoder::Encoder;
use minecraft_protocol::version::v1_14_4::handshake::Handshake;
use minecraft_protocol::version::v1_14_4::login::LoginStart;
use minecraft_protocol::version::v1_20_3::status::{ServerStatus, StatusResponse};
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;

use crate::config::{Config, Server as ConfigServer};
use crate::join;
use crate::mc::favicon;
use crate::proto::action;
use crate::proto::client::{Client, ClientInfo, ClientState};
use crate::proto::packet::{self, RawPacket};
use crate::proto::packets;
use crate::server::{self, Server};

const BAN_MESSAGE_PREFIX: &str = "Your IP address is banned from this server.\nReason: ";

const DEFAULT_BAN_REASON: &str = "Banned by an operator.";

const WHITELIST_MESSAGE: &str = "You are not white-listed on this server!";

const SERVER_ICON_FILE: &str = "server-icon.png";

pub async fn serve(
    client: Client,
    mut inbound: TcpStream,
    config: Arc<Config>,
    server: Arc<Server>,
) -> Result<(), ()> {
    let (mut reader, mut writer) = inbound.split();

    let mut buf = BytesMut::new();

    let mut inbound_history = BytesMut::new();
    let mut client_info = ClientInfo::empty();

    loop {

        let (packet, raw) = match packet::read_packet(&client, &mut buf, &mut reader).await {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            Err(_) => {
                error!(target: "plexpaper", "Closing connection, error occurred");
                break;
            }
        };

        let client_state = client.state();

        if client_state == ClientState::Handshake
            && packet.id == packets::handshake::SERVER_HANDSHAKE
        {

            let handshake = match Handshake::decode(&mut packet.data.as_slice()) {
                Ok(handshake) => handshake,
                Err(err) => {
                    debug!(target: "plexpaper", "Got malformed handshake from client, disconnecting: {err:?}");
                    break;
                }
            };

            let new_state = match ClientState::from_id(handshake.next_state) {
                Some(state) => state,
                None => {
                    error!(target: "plexpaper", "Client tried to switch into unknown protcol state ({}), disconnecting", handshake.next_state);
                    break;
                }
            };

            client_info
                .protocol
                .replace(handshake.protocol_version as u32);
            client_info.handshake.replace(handshake);
            client.set_state(new_state);

            if new_state == ClientState::Login {
                inbound_history.extend(raw);
            }

            continue;
        }

        if client_state == ClientState::Status && packet.id == packets::status::SERVER_STATUS {
            let server_status = server_status(&client_info, &config, &server).await;
            let packet = StatusResponse { server_status };

            let mut data = Vec::new();
            packet.encode(&mut data).map_err(|_| ())?;

            let response = RawPacket::new(0, data).encode_with_len(&client)?;
            writer.write_all(&response).await.map_err(|_| ())?;

            continue;
        }

        if client_state == ClientState::Status && packet.id == packets::status::SERVER_PING {
            writer.write_all(&raw).await.map_err(|_| ())?;
            continue;
        }

        if client_state == ClientState::Login && packet.id == packets::login::SERVER_LOGIN_START {

            let username = LoginStart::decode(&mut packet.data.as_slice())
                .ok()
                .map(|p| p.name);
            client_info.username = username.clone();

            if config.lockout.enabled {
                match username {
                    Some(username) => {
                        info!(target: "plexpaper", "Kicked '{}' because lockout is enabled", username)
                    }
                    None => info!(target: "plexpaper", "Kicked player because lockout is enabled"),
                }
                action::kick(&client, &config.lockout.message, &mut writer).await?;
                break;
            }

            if let Some(ban) = server.ban_entry(&client.peer.ip()).await {
                if ban.is_banned() {
                    let msg = if let Some(reason) = ban.reason {
                        info!(target: "plexpaper", "Login from banned IP {} ({}), disconnecting", client.peer.ip(), &reason);
                        reason.to_string()
                    } else {
                        info!(target: "plexpaper", "Login from banned IP {}, disconnecting", client.peer.ip());
                        DEFAULT_BAN_REASON.to_string()
                    };
                    action::kick(&client, &format!("{BAN_MESSAGE_PREFIX}{msg}"), &mut writer)
                        .await?;
                    break;
                }
            }

            if let Some(ref username) = username {
                if !server.is_whitelisted(username).await {
                    info!(target: "plexpaper", "User '{}' tried to wake server but is not whitelisted, disconnecting", username);
                    action::kick(&client, WHITELIST_MESSAGE, &mut writer).await?;
                    break;
                }
            }

            Server::start(config.clone(), server.clone(), username).await;

            inbound_history.extend(&raw);
            inbound_history.extend(&buf);

            let mut login_queue = BytesMut::with_capacity(raw.len() + buf.len());
            login_queue.extend(&raw);
            login_queue.extend(&buf);

            buf.clear();

            join::occupy(
                client,
                client_info,
                config,
                server,
                inbound,
                inbound_history,
                login_queue,
            )
            .await?;
            return Ok(());
        }

        debug!(target: "plexpaper", "Got unhandled packet:");
        debug!(target: "plexpaper", "- State: {:?}", client_state);
        debug!(target: "plexpaper", "- Packet ID: {}", packet.id);
    }

    Ok(())
}

async fn server_status(client_info: &ClientInfo, config: &Config, server: &Server) -> ServerStatus {
    let status = server.status().await;
    let server_state = server.state();

    if server_state == server::State::Started && status.is_some() {
        return status.as_ref().unwrap().clone();
    }

    let (version, max) = match status.as_ref() {
        Some(status) => (status.version.clone(), status.players.max),
        None => (
            ServerVersion {
                name: config.public.version.clone(),
                protocol: config.public.protocol,
            },
            0,
        ),
    };

    let description = {
        if config.motd.from_server && status.is_some() {
            status.as_ref().unwrap().description.clone()
        } else {
            match server_state {
                server::State::Stopped | server::State::Started => config.motd.sleeping.clone(),
                server::State::Starting => config.motd.starting.clone(),
                server::State::Stopping => config.motd.stopping.clone(),
            }
        }
    };

    let mut favicon = None;
    if favicon::supports_favicon(client_info) {
        if config.motd.from_server && status.is_some() {
            favicon = status.as_ref().unwrap().favicon.clone()
        }
        if favicon.is_none() {
            favicon = Some(server_favicon(config).await);
        }
    }

    ServerStatus {
        version,
        description,
        players: OnlinePlayers {
            online: 0,
            max,
            sample: vec![],
        },
        favicon,
    }
}

async fn server_favicon(config: &Config) -> String {

    let dir = match ConfigServer::server_directory(config) {
        Some(dir) => dir,
        None => return favicon::default_favicon(),
    };

    let path = dir.join(SERVER_ICON_FILE);
    if !path.is_file() {
        return favicon::default_favicon();
    }

    let data = fs::read(path).await.unwrap_or_else(|err| {
        error!(target: "plexpaper::status", "Failed to read favicon from {}, using default: {err}", SERVER_ICON_FILE);
        favicon::default_favicon().into_bytes()
    });

    favicon::encode_favicon(&data)
}
