use base64::Engine;

use crate::proto::client::ClientInfo;

const FAVICON_PROTOCOL_VERSION: u32 = 4;

pub fn default_favicon() -> String {
    encode_favicon(include_bytes!("../../res/unknown_server_optimized.png"))
}

pub fn encode_favicon(data: &[u8]) -> String {
    format!(
        "{}{}",
        "data:image/png;base64,",
        base64::engine::general_purpose::STANDARD.encode(data)
    )
}

pub fn supports_favicon(client_info: &ClientInfo) -> bool {
    client_info
        .protocol
        .map(|p| p >= FAVICON_PROTOCOL_VERSION)
        .unwrap_or(true)
}
