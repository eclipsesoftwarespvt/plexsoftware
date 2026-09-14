use std::path::Path;
use std::sync::mpsc::channel;
use std::sync::Arc;
use std::time::Duration;

use notify::RecursiveMode;
use notify_debouncer_mini::new_debouncer;

use crate::config::{Config, Server as ConfigServer};
use crate::mc::ban::{self, BannedIps};
use crate::mc::{server_properties, whitelist};
use crate::server::Server;

const WATCH_DEBOUNCE: Duration = Duration::from_secs(2);

pub fn service(config: Arc<Config>, server: Arc<Server>) {

    let dir = match ConfigServer::server_directory(&config) {
        Some(dir) if dir.is_dir() => dir,
        _ => {
            warn!(target: "plexpaper", "Server directory doesn't exist, can't watch file changes to reload whitelist and banned IPs");
            return;
        }
    };

    #[allow(clippy::blocks_in_conditions)]
    while {

        reload_bans(&config, &server, &dir.join(ban::FILE));
        reload_whitelist(&config, &server, &dir);

        watch_server(&config, &server, &dir)
    } {}
}

#[must_use]
fn watch_server(config: &Config, server: &Server, dir: &Path) -> bool {

    if !dir.is_dir() {
        error!(target: "plexpaper", "Server directory does not exist at {} anymore, not watching changes", dir.display());
        return false;
    }

    let (tx, rx) = channel();
    let mut debouncer = match new_debouncer(WATCH_DEBOUNCE, tx) {
        Ok(debouncer) => debouncer,
        Err(err) => {
            error!(target: "plexpaper", "An error occured while creating watcher for server files: {}", err);
            return true;
        }
    };
    if let Err(err) = debouncer.watcher().watch(dir, RecursiveMode::NonRecursive) {
        error!(target: "plexpaper", "An error occured while creating watcher for server files: {}", err);
        return true;
    }

    loop {
        match rx.recv() {

            Ok(Ok(events)) => {
                for event in events {
                    update(config, server, dir, &event.path);
                }
            }

            Ok(Err(err)) => {
                error!(target: "plexpaper", "Error occurred while watching server directory for file changes: {}", err);
                return true;
            }

            Err(_) => {
                debug!(target: "plexpaper", "Rescanning server directory files due to file watching problem");
                return true;
            }
        }
    }
}

fn update(config: &Config, server: &Server, dir: &Path, path: &Path) {

    if path.ends_with(ban::FILE) {
        reload_bans(config, server, path);
    }

    if path.ends_with(whitelist::WHITELIST_FILE)
        || path.ends_with(whitelist::OPS_FILE)
        || path.ends_with(server_properties::FILE)
    {
        reload_whitelist(config, server, dir);
    }
}

fn reload_bans(config: &Config, server: &Server, path: &Path) {

    if !config.server.block_banned_ips && !config.server.drop_banned_ips {
        return;
    }

    trace!(target: "plexpaper", "Reloading banned IPs...");

    if !path.is_file() {
        debug!(target: "plexpaper", "No banned IPs, {} does not exist", ban::FILE);

        server.set_banned_ips_blocking(BannedIps::default());
        return;
    }

    match ban::load(path) {
        Ok(ips) => server.set_banned_ips_blocking(ips),
        Err(err) => {
            debug!(target: "plexpaper", "Failed load banned IPs from {}, ignoring: {}", ban::FILE, err);
        }
    }

    if server.is_banned_ip_blocking(&("127.0.0.1".parse().unwrap())) {
        warn!(target: "plexpaper", "Local address 127.0.0.1 IP banned, probably not what you want");
        warn!(target: "plexpaper", "Use '/pardon-ip 127.0.0.1' on the server to unban");
    }
}

fn reload_whitelist(config: &Config, server: &Server, dir: &Path) {

    if !config.server.wake_whitelist {
        return;
    }

    let enabled = server_properties::read_property(dir.join(server_properties::FILE), "white-list")
        .map(|v| v.trim() == "true")
        .unwrap_or(false);
    if !enabled {
        server.set_whitelist_blocking(None);
        debug!(target: "plexpaper", "Not using whitelist, not enabled in {}", server_properties::FILE);
        return;
    }

    trace!(target: "plexpaper", "Reloading whitelisted users...");

    match whitelist::load_dir(dir) {
        Ok(whitelist) => server.set_whitelist_blocking(Some(whitelist)),
        Err(err) => {
            debug!(target: "plexpaper", "Failed load whitelist from {}, ignoring: {}", dir.display(), err);
        }
    }
}
