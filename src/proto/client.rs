use std::net::SocketAddr;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Mutex;

use minecraft_protocol::version::v1_14_4::handshake::Handshake;

#[derive(Debug)]
pub struct Client {

    pub peer: SocketAddr,

    pub state: Mutex<ClientState>,

    pub compression: AtomicI32,
}

impl Client {

    pub fn new(peer: SocketAddr) -> Self {
        Self {
            peer,
            state: Default::default(),
            compression: AtomicI32::new(-1),
        }
    }

    pub fn dummy() -> Self {
        Self::new("0.0.0.0:0".parse().unwrap())
    }

    pub fn state(&self) -> ClientState {
        *self.state.lock().unwrap()
    }

    pub fn set_state(&self, state: ClientState) {
        *self.state.lock().unwrap() = state;
    }

    pub fn compressed(&self) -> i32 {
        self.compression.load(Ordering::Relaxed)
    }

    pub fn is_compressed(&self) -> bool {
        self.compressed() >= 0
    }

    #[allow(unused)]
    pub fn set_compression(&self, threshold: i32) {
        trace!(target: "plexpaper", "Client now uses compression threshold of {}", threshold);
        self.compression.store(threshold, Ordering::Relaxed);
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum ClientState {

    Handshake,

    Status,

    Login,

    #[allow(unused)]
    Play,
}

impl ClientState {

    pub fn from_id(id: i32) -> Option<Self> {
        match id {
            0 => Some(Self::Handshake),
            1 => Some(Self::Status),
            2 => Some(Self::Login),
            _ => None,
        }
    }

    pub fn to_id(self) -> i32 {
        match self {
            Self::Handshake => 0,
            Self::Status => 1,
            Self::Login => 2,
            Self::Play => -1,
        }
    }
}

impl Default for ClientState {
    fn default() -> Self {
        Self::Handshake
    }
}

#[derive(Debug, Clone, Default)]
pub struct ClientInfo {

    pub protocol: Option<u32>,

    pub handshake: Option<Handshake>,

    pub username: Option<String>,
}

impl ClientInfo {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn protocol(&self) -> Option<u32> {
        self.protocol
            .or_else(|| self.handshake.as_ref().map(|h| h.protocol_version as u32))
    }
}
