use minecraft_protocol::version::{v1_16_3, v1_17};
use tokio::net::tcp::WriteHalf;

use crate::proto::client::{Client, ClientInfo};
use crate::proto::packet;

const CHANNEL: &str = "minecraft:brand";

const SERVER_BRAND: &[u8] = b"plexpaper";

pub async fn send(
    client: &Client,
    client_info: &ClientInfo,
    writer: &mut WriteHalf<'_>,
) -> Result<(), ()> {
    match client_info.protocol() {
        Some(p) if p < v1_17::PROTOCOL => {
            packet::write_packet(
                v1_16_3::game::ClientBoundPluginMessage {
                    channel: CHANNEL.into(),
                    data: SERVER_BRAND.into(),
                },
                client,
                writer,
            )
            .await
        }
        _ => {
            packet::write_packet(
                v1_17::game::ClientBoundPluginMessage {
                    channel: CHANNEL.into(),
                    data: SERVER_BRAND.into(),
                },
                client,
                writer,
            )
            .await
        }
    }
}
