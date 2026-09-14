use std::net::IpAddr;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::FutureExt;
use minecraft_protocol::version::v1_20_3::status::ServerStatus;
use tokio::process::Command;
use tokio::sync::watch;
#[cfg(feature = "rcon")]
use tokio::sync::Semaphore;
use tokio::sync::{Mutex, RwLock, RwLockReadGuard};
use tokio::time;

use crate::config::{Config, Server as ConfigServer};
use crate::mc::ban::{BannedIp, BannedIps};
use crate::mc::whitelist::Whitelist;
use crate::os;
use crate::proto::packets::play::join_game::JoinGameData;

const SERVER_QUIT_COOLDOWN: Duration = Duration::from_millis(2500);

#[cfg(feature = "rcon")]
const RCON_COOLDOWN: Duration = Duration::from_secs(15);

const ALLOWED_EXIT_CODES: [i32; 2] = [130, 143];

#[derive(Debug)]
pub struct Server {

    state: AtomicU8,

    state_watch_sender: watch::Sender<State>,

    state_watch_receiver: watch::Receiver<State>,

    pid: Mutex<Option<u32>>,

    status: RwLock<Option<ServerStatus>>,

    last_active: RwLock<Option<Instant>>,

    keep_online_until: RwLock<Option<Instant>>,

    kill_at: RwLock<Option<Instant>>,

    banned_ips: RwLock<BannedIps>,

    whitelist: RwLock<Option<Whitelist>>,

    #[cfg(feature = "rcon")]
    rcon_lock: Semaphore,

    #[cfg(feature = "rcon")]
    rcon_last_stop: Mutex<Option<Instant>>,

    pub probed_join_game: RwLock<Option<JoinGameData>>,

    pub forge_payload: RwLock<Vec<Vec<u8>>>,
}

impl Server {

    pub fn state(&self) -> State {
        State::from_u8(self.state.load(Ordering::Relaxed))
    }

    pub fn state_receiver(&self) -> watch::Receiver<State> {
        self.state_watch_receiver.clone()
    }

    async fn update_state(&self, state: State, config: &Config) -> bool {
        self.update_state_from(None, state, config).await
    }

    async fn update_state_from(&self, from: Option<State>, new: State, config: &Config) -> bool {

        let old = State::from_u8(match from {
            Some(from) => match self.state.compare_exchange(
                from.to_u8(),
                new.to_u8(),
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(old) => old,
                Err(_) => return false,
            },
            None => self.state.swap(new.to_u8(), Ordering::Relaxed),
        });

        if old == new {
            return false;
        }

        trace!("Change server state from {:?} to {:?}", old, new);

        let _ = self.state_watch_sender.send(new);

        *self.kill_at.write().await = match new {
            State::Starting if config.server.start_timeout > 0 => {
                Some(Instant::now() + Duration::from_secs(config.server.start_timeout as u64))
            }
            State::Stopping if config.server.stop_timeout > 0 => {
                Some(Instant::now() + Duration::from_secs(config.server.stop_timeout as u64))
            }
            _ => None,
        };

        match new {
            State::Started => info!(target: "plexpaper::monitor", "Server is now online"),
            State::Stopped => info!(target: "plexpaper::monitor", "Server is now sleeping"),
            _ => {}
        }

        if new == State::Started {
            self.update_last_active().await;
            self.keep_online_for(Some(config.time.min_online_time))
                .await;
        }

        true
    }

    pub async fn update_status(&self, config: &Config, status: Option<ServerStatus>) {

        match (self.state(), &status) {
            (State::Stopped | State::Starting, Some(_)) => {
                self.update_state(State::Started, config).await;
            }
            (State::Started, None) => {
                self.update_state(State::Stopped, config).await;
            }
            _ => {}
        }

        if let Some(status) = status {

            if status.players.online > 0 {
                self.update_last_active().await;
            }

            self.status.write().await.replace(status);
        }
    }

    pub async fn start(config: Arc<Config>, server: Arc<Server>, username: Option<String>) -> bool {

        if !server
            .update_state_from(Some(State::Stopped), State::Starting, &config)
            .await
        {
            return false;
        }

        match username {
            Some(username) => info!(target: "plexpaper", "Starting server for '{}'...", username),
            None => info!(target: "plexpaper", "Starting server..."),
        }

        #[cfg(unix)]
        if config.server.freeze_process && unfreeze_server_signal(&config, &server).await {
            return true;
        }

        Self::spawn_server_task(config, server);
        true
    }

    fn spawn_server_task(config: Arc<Config>, server: Arc<Server>) {
        tokio::spawn(invoke_server_cmd(config, server).map(|_| ()));
    }

    #[allow(unused_variables)]
    pub async fn stop(&self, config: &Config) -> bool {

        #[cfg(unix)]
        if config.server.freeze_process && freeze_server_signal(config, self).await {
            return true;
        }

        #[cfg(feature = "rcon")]
        if self.state() == State::Started && stop_server_rcon(config, self).await {
            return true;
        }

        #[cfg(unix)]
        if stop_server_signal(config, self).await {
            return true;
        }

        warn!(target: "plexpaper", "Failed to stop server, no more suitable stopping method to use");
        false
    }

    pub async fn force_kill(&self) -> bool {
        if let Some(pid) = *self.pid.lock().await {
            return os::force_kill(pid);
        }
        false
    }

    pub async fn should_sleep(&self, config: &Config) -> bool {

        if self.state() != State::Started {
            return false;
        }

        let players_online = self
            .status
            .read()
            .await
            .as_ref()
            .map(|status| status.players.online > 0)
            .unwrap_or(false);
        if players_online {
            trace!(target: "plexpaper", "Not sleeping because players are online");
            return false;
        }

        let keep_online = self
            .keep_online_until
            .read()
            .await
            .map(|i| i >= Instant::now())
            .unwrap_or(false);
        if keep_online {
            trace!(target: "plexpaper", "Not sleeping because of keep online");
            return false;
        }

        if let Some(last_idle) = self.last_active.read().await.as_ref() {
            return last_idle.elapsed() >= Duration::from_secs(config.time.sleep_after as u64);
        }

        false
    }

    pub async fn should_kill(&self) -> bool {
        self.kill_at
            .read()
            .await
            .map(|t| t <= Instant::now())
            .unwrap_or(false)
    }

    pub async fn status(&self) -> RwLockReadGuard<'_, Option<ServerStatus>> {
        self.status.read().await
    }

    async fn update_last_active(&self) {
        self.last_active.write().await.replace(Instant::now());
    }

    async fn keep_online_for(&self, duration: Option<u32>) {
        *self.keep_online_until.write().await = duration
            .filter(|d| *d > 0)
            .map(|d| Instant::now() + Duration::from_secs(d as u64));
    }

    pub async fn is_banned_ip(&self, ip: &IpAddr) -> bool {
        self.banned_ips.read().await.is_banned(ip)
    }

    pub async fn ban_entry(&self, ip: &IpAddr) -> Option<BannedIp> {
        self.banned_ips.read().await.get(ip)
    }

    pub fn is_banned_ip_blocking(&self, ip: &IpAddr) -> bool {
        futures::executor::block_on(async { self.is_banned_ip(ip).await })
    }

    pub async fn is_whitelisted(&self, username: &str) -> bool {
        self.whitelist
            .read()
            .await
            .as_ref()
            .map(|w| w.is_whitelisted(username))
            .unwrap_or(true)
    }

    pub async fn set_banned_ips(&self, ips: BannedIps) {
        *self.banned_ips.write().await = ips;
    }

    pub fn set_banned_ips_blocking(&self, ips: BannedIps) {
        futures::executor::block_on(async { self.set_banned_ips(ips).await })
    }

    pub async fn set_whitelist(&self, whitelist: Option<Whitelist>) {
        *self.whitelist.write().await = whitelist;
    }

    pub fn set_whitelist_blocking(&self, whitelist: Option<Whitelist>) {
        futures::executor::block_on(async { self.set_whitelist(whitelist).await })
    }
}

impl Default for Server {
    fn default() -> Self {
        let (state_watch_sender, state_watch_receiver) = watch::channel(State::Stopped);

        Self {
            state: AtomicU8::new(State::Stopped.to_u8()),
            state_watch_sender,
            state_watch_receiver,
            pid: Default::default(),
            status: Default::default(),
            last_active: Default::default(),
            keep_online_until: Default::default(),
            kill_at: Default::default(),
            banned_ips: Default::default(),
            whitelist: Default::default(),
            #[cfg(feature = "rcon")]
            rcon_lock: Semaphore::new(1),
            #[cfg(feature = "rcon")]
            rcon_last_stop: Default::default(),
            probed_join_game: Default::default(),
            forge_payload: Default::default(),
        }
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum State {

    Stopped,

    Starting,

    Started,

    Stopping,
}

impl State {

    pub fn from_u8(state: u8) -> Self {
        match state {
            0 => Self::Stopped,
            1 => Self::Starting,
            2 => Self::Started,
            3 => Self::Stopping,
            _ => panic!("invalid State u8"),
        }
    }

    pub fn to_u8(self) -> u8 {
        match self {
            Self::Stopped => 0,
            Self::Starting => 1,
            Self::Started => 2,
            Self::Stopping => 3,
        }
    }
}

pub async fn invoke_server_cmd(
    config: Arc<Config>,
    state: Arc<Server>,
) -> Result<(), Box<dyn std::error::Error>> {

    let args = shlex::split(&config.server.command).expect("invalid server command");
    let mut cmd = Command::new(&args[0]);
    cmd.args(args.iter().skip(1));
    cmd.kill_on_drop(true);

    if let Some(ref dir) = ConfigServer::server_directory(&config) {
        cmd.current_dir(dir);
    }

    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(err) => {
            error!(target: "plexpaper", "Failed to start server process through command");
            return Err(err.into());
        }
    };

    state
        .pid
        .lock()
        .await
        .replace(child.id().expect("unknown server PID"));

    let crashed = match child.wait().await {
        Ok(status) if status.success() => {
            debug!(target: "plexpaper", "Server process stopped successfully ({})", status);
            false
        }
        Ok(status)
            if status
                .code()
                .map(|ref code| ALLOWED_EXIT_CODES.contains(code))
                .unwrap_or(false) =>
        {
            debug!(target: "plexpaper", "Server process stopped successfully by SIGTERM ({})", status);
            false
        }
        Ok(status) => {
            warn!(target: "plexpaper", "Server process stopped with error code ({})", status);
            state.state() == State::Started
        }
        Err(err) => {
            error!(target: "plexpaper", "Failed to wait for server process to quit: {}", err);
            error!(target: "plexpaper", "Assuming server quit, cleaning up...");
            false
        }
    };

    state.pid.lock().await.take();

    time::sleep(SERVER_QUIT_COOLDOWN).await;

    state.update_state(State::Stopped, &config).await;

    if crashed && config.server.wake_on_crash {
        warn!(target: "plexpaper", "Server crashed, restarting...");
        Server::start(config, state, None).await;
    }

    Ok(())
}

#[cfg(feature = "rcon")]
async fn stop_server_rcon(config: &Config, server: &Server) -> bool {
    use crate::mc::rcon::Rcon;

    if !config.rcon.enabled {
        trace!(target: "plexpaper", "Not using RCON to stop server, disabled in config");
        return false;
    }

    let rcon_lock = server.rcon_lock.acquire().await.unwrap();

    let rcon_cooled_down = server
        .rcon_last_stop
        .lock()
        .await
        .map(|t| t.elapsed() >= RCON_COOLDOWN)
        .unwrap_or(true);
    if !rcon_cooled_down {
        debug!(target: "plexpaper", "Not using RCON to stop server, in cooldown, used too recently");
        return false;
    }

    let mut rcon = match Rcon::connect_config(config).await {
        Ok(rcon) => rcon,
        Err(err) => {
            error!(target: "plexpaper", "Failed to RCON server to sleep: {}", err);
            return false;
        }
    };

    if let Err(err) = rcon.cmd("stop").await {
        error!(target: "plexpaper", "Failed to invoke stop through RCON: {}", err);
        return false;
    }

    server.rcon_last_stop.lock().await.replace(Instant::now());
    server.update_state(State::Stopping, config).await;

    rcon.close().await;

    drop(rcon_lock);

    true
}

#[cfg(unix)]
async fn stop_server_signal(config: &Config, server: &Server) -> bool {

    let pid = match *server.pid.lock().await {
        Some(pid) => pid,
        None => {
            debug!(target: "plexpaper", "Could not send stop signal to server process, PID unknown");
            return false;
        }
    };

    if !crate::os::kill_gracefully(pid) {
        error!(target: "plexpaper", "Failed to send stop signal to server process");
        return false;
    }

    server
        .update_state_from(Some(State::Starting), State::Stopping, config)
        .await;
    server
        .update_state_from(Some(State::Started), State::Stopping, config)
        .await;

    true
}

#[cfg(unix)]
async fn freeze_server_signal(config: &Config, server: &Server) -> bool {

    let pid = match *server.pid.lock().await {
        Some(pid) => pid,
        None => {
            debug!(target: "plexpaper", "Could not send freeze signal to server process, PID unknown");
            return false;
        }
    };

    if !os::freeze(pid) {
        error!(target: "plexpaper", "Failed to send freeze signal to server process.");
    }

    server
        .update_state_from(Some(State::Starting), State::Stopped, config)
        .await;
    server
        .update_state_from(Some(State::Started), State::Stopped, config)
        .await;

    true
}

#[cfg(unix)]
async fn unfreeze_server_signal(config: &Config, server: &Server) -> bool {

    let pid = match *server.pid.lock().await {
        Some(pid) => pid,
        None => {
            debug!(target: "plexpaper", "Could not send unfreeze signal to server process, PID unknown");
            return false;
        }
    };

    if !os::unfreeze(pid) {
        error!(target: "plexpaper", "Failed to send unfreeze signal to server process.");
    }

    server
        .update_state_from(Some(State::Stopping), State::Starting, config)
        .await;
    server
        .update_state_from(Some(State::Stopped), State::Starting, config)
        .await;

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use minecraft_protocol::data::server_status::{OnlinePlayers, ServerVersion};

    fn test_config() -> Config {
        let mut config = Config::runtime(&[]);
        config.server.command = "true".to_owned();
        config
    }

    fn dummy_status() -> ServerStatus {
        ServerStatus {
            version: ServerVersion {
                name: "1.20.4".to_string(),
                protocol: 765,
            },
            players: OnlinePlayers {
                online: 0,
                max: 20,
                sample: vec![],
            },
            description: String::new(),
            favicon: None,
        }
    }

    #[tokio::test]
    async fn detecting_already_started_server_resets_idle_timer() {
        let server = Server::default();
        let config = test_config();

        server
            .last_active
            .write()
            .await
            .replace(Instant::now() - Duration::from_secs(3600));

        assert_eq!(server.state(), State::Stopped);

        server.update_status(&config, Some(dummy_status())).await;

        assert_eq!(server.state(), State::Started);

        assert!(
            !server.should_sleep(&config).await,
            "server should not be considered idle immediately after being detected as started"
        );
    }
}
