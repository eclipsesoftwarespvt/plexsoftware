use md5::{Digest, Md5};
use uuid::Uuid;

const OFFLINE_PLAYER_NAMESPACE: &str = "OfflinePlayer:";

fn player_uuid(username: &str) -> Uuid {
    java_name_uuid_from_bytes(username.as_bytes())
}

pub fn offline_player_uuid(username: &str) -> Uuid {
    player_uuid(&format!("{OFFLINE_PLAYER_NAMESPACE}{username}"))
}

fn java_name_uuid_from_bytes(data: &[u8]) -> Uuid {
    let mut hasher = Md5::new();
    hasher.update(data);
    let mut md5: [u8; 16] = hasher.finalize().into();

    md5[6] &= 0x0f;
    md5[6] |= 0x30;
    md5[8] &= 0x3f;
    md5[8] |= 0x80;

    Uuid::from_bytes(md5)
}
