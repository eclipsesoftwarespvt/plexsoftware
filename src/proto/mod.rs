pub mod action;
pub mod client;
pub mod packet;
pub mod packets;

pub const PROTO_DEFAULT_VERSION: &str = "1.21.4";

pub const PROTO_DEFAULT_PROTOCOL: u32 = 769;

pub const COMPRESSION_THRESHOLD: i32 = 256;

pub(super) const BUF_SIZE: usize = 8 * 1024;
