use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;

use crate::proto;

pub const PUBLIC_PORT_DEFAULT: u16 = 25565;
pub const INTERNAL_PORT_PRIMARY: u16 = 25566;
pub const INTERNAL_PORT_FALLBACK: u16 = 25567;

const SLEEP_AFTER: u32 = 60;
const MIN_ONLINE_TIME: u32 = 60;
const START_TIMEOUT: u32 = 120;
const STOP_TIMEOUT: u32 = 60;
const HOLD_TIMEOUT: u32 = 25;
const LOBBY_TIMEOUT: u32 = 600;

const JAVA_BIN: &str = "java";
const JAR_DEFAULT: &str = "server.jar";
const JAVA_ARGS_DEFAULT: [&str; 5] = [
    "-Xms128M",
    "-XX:MaxRAMPercentage=95.0",
    "-Dterminal.jline=false",
    "-Dterminal.ansi=true",
    "-jar",
];

#[derive(Debug)]
pub struct Config {
    pub public: Public,
    pub server: Server,
    pub time: Time,
    pub motd: Motd,
    pub join: Join,
    pub lockout: Lockout,
    pub rcon: Rcon,
    pub advanced: Advanced,
}

impl Config {
    pub fn runtime(java_args: &[String]) -> Self {
        let public_port = env_u16("SERVER_PORT").unwrap_or(PUBLIC_PORT_DEFAULT);
        let internal_port = if public_port == INTERNAL_PORT_PRIMARY {
            INTERNAL_PORT_FALLBACK
        } else {
            INTERNAL_PORT_PRIMARY
        };

        Config {
            public: Public {
                address: SocketAddr::from(([0, 0, 0, 0], public_port)),
                version: env_string("MC_VERSION")
                    .unwrap_or_else(|| proto::PROTO_DEFAULT_VERSION.to_string()),
                protocol: env_u32("PROTOCOL_NUMBER").unwrap_or(proto::PROTO_DEFAULT_PROTOCOL),
            },
            server: Server {
                directory: Some(PathBuf::from(".")),
                command: build_command(java_args),
                address: SocketAddr::from(([127, 0, 0, 1], internal_port)),
                ..Default::default()
            },
            time: Time::default(),
            motd: Motd::default(),
            join: Join::default(),
            lockout: Lockout::default(),
            rcon: Rcon::default(),
            advanced: Advanced::default(),
        }
    }
}

fn env_string(key: &str) -> Option<String> {
    env::var(key).ok().map(|v| v.trim().to_owned()).filter(|v| !v.is_empty())
}

fn env_u16(key: &str) -> Option<u16> {
    env_string(key).and_then(|v| v.parse().ok())
}

fn env_u32(key: &str) -> Option<u32> {
    env_string(key).and_then(|v| v.parse().ok())
}

fn build_command(java_args: &[String]) -> String {
    let mut args: Vec<String> = java_args
        .iter()
        .map(|arg| arg.trim().to_owned())
        .filter(|arg| !arg.is_empty())
        .collect();

    if args.is_empty() {
        args = JAVA_ARGS_DEFAULT.iter().map(|a| (*a).to_owned()).collect();
    }

    if !args.iter().any(|arg| arg == "-jar") {
        args.push("-jar".to_owned());
    }

    if args.last().map(|a| a == "-jar").unwrap_or(false) {
        args.push(jar_file());
    }

    if !args.iter().any(|arg| arg == "--nogui") {
        args.push("--nogui".to_owned());
    }

    let mut parts = vec![JAVA_BIN.to_owned()];
    parts.extend(args);

    parts
        .iter()
        .map(|part| shlex::try_quote(part).map(|p| p.into_owned()).unwrap_or_else(|_| part.clone()))
        .collect::<Vec<_>>()
        .join(" ")
}

fn jar_file() -> String {
    env_string("SERVER_JARFILE").unwrap_or_else(|| JAR_DEFAULT.to_owned())
}

#[derive(Debug)]
pub struct Public {
    pub address: SocketAddr,
    pub version: String,
    pub protocol: u32,
}

#[derive(Debug)]
pub struct Server {
    directory: Option<PathBuf>,
    pub command: String,
    pub address: SocketAddr,
    pub freeze_process: bool,
    pub wake_on_start: bool,
    pub wake_on_crash: bool,
    pub probe_on_start: bool,
    pub forge: bool,
    pub start_timeout: u32,
    pub stop_timeout: u32,
    pub wake_whitelist: bool,
    pub block_banned_ips: bool,
    pub drop_banned_ips: bool,
    pub send_proxy_v2: bool,
}

impl Default for Server {
    fn default() -> Self {
        Self {
            directory: Some(PathBuf::from(".")),
            command: build_command(&[]),
            address: SocketAddr::from(([127, 0, 0, 1], INTERNAL_PORT_PRIMARY)),
            freeze_process: false,
            wake_on_start: false,
            wake_on_crash: false,
            probe_on_start: false,
            forge: false,
            start_timeout: START_TIMEOUT,
            stop_timeout: STOP_TIMEOUT,
            wake_whitelist: true,
            block_banned_ips: true,
            drop_banned_ips: false,
            send_proxy_v2: false,
        }
    }
}

impl Server {
    pub fn server_directory(config: &Config) -> Option<PathBuf> {
        config.server.directory.clone()
    }
}

#[derive(Debug)]
pub struct Time {
    pub sleep_after: u32,
    pub min_online_time: u32,
}

impl Default for Time {
    fn default() -> Self {
        Self {
            sleep_after: SLEEP_AFTER,
            min_online_time: MIN_ONLINE_TIME,
        }
    }
}

#[derive(Debug)]
pub struct Motd {
    pub sleeping: String,
    pub starting: String,
    pub stopping: String,
    pub from_server: bool,
}

impl Default for Motd {
    fn default() -> Self {
        Self {
            sleeping: "§bPlexScale§f - Server is sleeping, join to wake it up\n§fThis server is powered by PlexScale.com - discord.gg/plexscale".into(),
            starting: "§bPlexScale§f - Server is starting, please wait...\n§fThis server is powered by PlexScale.com - discord.gg/plexscale".into(),
            stopping: "§bPlexScale§f - Server is going to sleep...\n§fThis server is powered by PlexScale.com - discord.gg/plexscale".into(),
            from_server: false,
        }
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum Method {
    Kick,
    Hold,
    Forward,
    Lobby,
}

#[derive(Debug)]
pub struct Join {
    pub methods: Vec<Method>,
    pub kick: JoinKick,
    pub hold: JoinHold,
    pub forward: JoinForward,
    pub lobby: JoinLobby,
}

impl Default for Join {
    fn default() -> Self {
        Self {
            methods: vec![Method::Hold, Method::Kick],
            kick: Default::default(),
            hold: Default::default(),
            forward: Default::default(),
            lobby: Default::default(),
        }
    }
}

#[derive(Debug)]
pub struct JoinKick {
    pub starting: String,
    pub stopping: String,
}

impl Default for JoinKick {
    fn default() -> Self {
        Self {
            starting: "§bPlexScale§f\n\nThe server is starting up.\n\nPlease reconnect in a moment.".into(),
            stopping: "§bPlexScale§f\n\nThe server is going to sleep.\n\nReconnect to wake it up again.".into(),
        }
    }
}

#[derive(Debug)]
pub struct JoinHold {
    pub timeout: u32,
}

impl Default for JoinHold {
    fn default() -> Self {
        Self {
            timeout: HOLD_TIMEOUT,
        }
    }
}

#[derive(Debug)]
pub struct JoinForward {
    pub address: SocketAddr,
    pub send_proxy_v2: bool,
}

impl Default for JoinForward {
    fn default() -> Self {
        Self {
            address: SocketAddr::from(([127, 0, 0, 1], PUBLIC_PORT_DEFAULT)),
            send_proxy_v2: false,
        }
    }
}

#[derive(Debug)]
pub struct JoinLobby {
    pub timeout: u32,
    pub message: String,
    pub ready_sound: Option<String>,
}

impl Default for JoinLobby {
    fn default() -> Self {
        Self {
            timeout: LOBBY_TIMEOUT,
            message: "§bPlexScale§f\n§fServer is starting, please wait...".into(),
            ready_sound: Some("block.note_block.chime".into()),
        }
    }
}

#[derive(Debug)]
pub struct Lockout {
    pub enabled: bool,
    pub message: String,
}

impl Default for Lockout {
    fn default() -> Self {
        Self {
            enabled: false,
            message: "§bPlexScale§f\n\nThis server is currently closed.".into(),
        }
    }
}

#[derive(Debug)]
pub struct Rcon {
    pub enabled: bool,
    pub port: u16,
    pub password: String,
    pub randomize_password: bool,
    pub send_proxy_v2: bool,
}

impl Default for Rcon {
    fn default() -> Self {
        Self {
            enabled: cfg!(windows),
            port: 25575,
            password: String::new(),
            randomize_password: true,
            send_proxy_v2: false,
        }
    }
}

#[derive(Debug)]
pub struct Advanced {
    pub rewrite_server_properties: bool,
}

impl Default for Advanced {
    fn default() -> Self {
        Self {
            rewrite_server_properties: true,
        }
    }
}
