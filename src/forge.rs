#[cfg(feature = "lobby")]
use std::sync::Arc;
#[cfg(feature = "lobby")]
use std::time::Duration;

#[cfg(feature = "lobby")]
use bytes::BytesMut;
use minecraft_protocol::decoder::Decoder;
use minecraft_protocol::encoder::Encoder;
use minecraft_protocol::version::forge_v1_13::login::{Acknowledgement, LoginWrapper, ModList};
use minecraft_protocol::version::v1_14_4::login::{LoginPluginRequest, LoginPluginResponse};
use minecraft_protocol::version::PacketId;
#[cfg(feature = "lobby")]
use tokio::io::AsyncWriteExt;
use tokio::net::tcp::WriteHalf;
#[cfg(feature = "lobby")]
use tokio::net::TcpStream;
#[cfg(feature = "lobby")]
use tokio::time;

use crate::forge;
use crate::proto::client::Client;
#[cfg(feature = "lobby")]
use crate::proto::client::ClientState;
use crate::proto::packet;
use crate::proto::packet::RawPacket;
#[cfg(feature = "lobby")]
use crate::proto::packets;
#[cfg(feature = "lobby")]
use crate::server::Server;

pub const STATUS_MAGIC: &str = "\0FML2\0";

pub const CHANNEL_LOGIN_WRAPPER: &str = "fml:loginwrapper";

pub const CHANNEL_HANDSHAKE: &str = "fml:handshake";

#[cfg(feature = "lobby")]
const CLIENT_DRAIN_FORGE_TIMEOUT: Duration = Duration::from_secs(5);

pub async fn respond_forge_login_packet(
    client: &Client,
    writer: &mut WriteHalf<'_>,
    message_id: i32,
    forge_channel: String,
    forge_packet: impl PacketId + Encoder,
) -> Result<(), ()> {

    let mut forge_data = Vec::new();
    forge_packet.encode(&mut forge_data).map_err(|_| ())?;

    let forge_payload =
        RawPacket::new(forge_packet.packet_id(), forge_data).encode_without_len(client)?;

    let mut payload = Vec::new();
    let packet = LoginWrapper {
        channel: forge_channel,
        packet: forge_payload,
    };
    packet.encode(&mut payload).map_err(|_| ())?;

    packet::write_packet(
        LoginPluginResponse {
            message_id,
            successful: true,
            data: payload,
        },
        client,
        writer,
    )
    .await
}

pub async fn respond_login_plugin_request(
    client: &Client,
    packet: LoginPluginRequest,
    writer: &mut WriteHalf<'_>,
) -> Result<(), ()> {

    let (message_id, login_wrapper, packet) =
        forge::decode_forge_login_packet(client, packet).await?;

    let is_unknown_header = login_wrapper.channel != forge::CHANNEL_HANDSHAKE;
    let is_mod_list = !is_unknown_header && packet.id == ModList::PACKET_ID;

    if !is_mod_list {
        trace!(target: "plexpaper::forge", "Acknowledging login plugin request");
        forge::respond_forge_login_packet(
            client,
            writer,
            message_id,
            login_wrapper.channel,
            Acknowledgement {},
        )
        .await
        .map_err(|_| {
            error!(target: "plexpaper::forge", "Failed to send Forge login plugin request acknowledgement");
        })?;
        return Ok(());
    }

    trace!(target: "plexpaper::forge", "Sending mod list reply to server with same contents");

    let mod_list = ModList::decode(&mut packet.data.as_slice()).map_err(|err| {
        error!(target: "plexpaper::forge", "Failed to decode Forge mod list: {:?}", err);
    })?;
    let mod_list_reply = mod_list.into_reply();

    forge::respond_forge_login_packet(
        client,
        writer,
        message_id,
        login_wrapper.channel,
        mod_list_reply,
    )
    .await
    .map_err(|_| {
        error!(target: "plexpaper::forge", "Failed to send Forge login plugin mod list reply");
    })?;

    Ok(())
}

pub async fn decode_forge_login_packet(
    client: &Client,
    plugin_request: LoginPluginRequest,
) -> Result<(i32, LoginWrapper, RawPacket), ()> {

    assert_eq!(plugin_request.channel, CHANNEL_LOGIN_WRAPPER);

    let login_wrapper =
        LoginWrapper::decode(&mut plugin_request.data.as_slice()).map_err(|err| {
            error!(target: "plexpaper::forge", "Failed to decode Forge LoginWrapper packet: {:?}", err);
        })?;

    let packet = RawPacket::decode_without_len(client, &login_wrapper.packet).map_err(|err| {
        error!(target: "plexpaper::forge", "Failed to decode Forge LoginWrapper packet contents: {:?}", err);
    })?;

    Ok((plugin_request.message_id, login_wrapper, packet))
}

#[cfg(feature = "lobby")]
pub async fn replay_login_payload(
    client: &Client,
    inbound: &mut TcpStream,
    server: Arc<Server>,
    inbound_buf: &mut BytesMut,
) -> Result<(), ()> {
    debug!(target: "plexpaper::lobby", "Replaying Forge login procedure for lobby client...");

    for packet in server.forge_payload.read().await.as_slice() {
        inbound.write_all(packet).await.map_err(|err| {
            error!(target: "plexpaper::lobby", "Failed to send Forge join payload to lobby client, will likely cause issues: {}", err);
        })?;
    }

    let count = server.forge_payload.read().await.len();
    drain_forge_responses(client, inbound, inbound_buf, count).await?;

    trace!(target: "plexpaper::lobby", "Forge join payload replayed");

    Ok(())
}

#[cfg(feature = "lobby")]
async fn drain_forge_responses(
    client: &Client,
    inbound: &mut TcpStream,
    buf: &mut BytesMut,
    mut count: usize,
) -> Result<(), ()> {
    let (mut reader, mut _writer) = inbound.split();

    loop {

        if count == 0 {
            trace!(target: "plexpaper::forge", "Drained all plugin responses from client");
            return Ok(());
        }

        let read_packet_task = packet::read_packet(client, buf, &mut reader);
        let timeout = time::timeout(CLIENT_DRAIN_FORGE_TIMEOUT, read_packet_task).await;
        let read_packet_task = match timeout {
            Ok(result) => result,
            Err(_) => {
                error!(target: "plexpaper::forge", "Expected more plugin responses from client, but didn't receive anything in a while, may be problematic");
                return Ok(());
            }
        };

        let (packet, _raw) = match read_packet_task {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            Err(_) => {
                error!(target: "plexpaper::forge", "Closing connection, error occurred");
                break;
            }
        };

        let client_state = client.state();

        if client_state == ClientState::Login
            && packet.id == packets::login::SERVER_LOGIN_PLUGIN_RESPONSE
        {
            trace!(target: "plexpaper::forge", "Voiding plugin response from client");
            count -= 1;
            continue;
        }

        debug!(target: "plexpaper::forge", "Got unhandled packet from server in record_forge_response:");
        debug!(target: "plexpaper::forge", "- State: {:?}", client_state);
        debug!(target: "plexpaper::forge", "- Packet ID: 0x{:02X} ({})", packet.id, packet.id);
    }

    Err(())
}
