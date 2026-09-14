use std::collections::HashMap;
use std::sync::Arc;

use crate::config::Config;
use crate::mc::server_properties;
use crate::proto;
use crate::service;

#[cfg(feature = "rcon")]
const RCON_PASSWORD_LENGTH: usize = 32;

pub fn invoke(java_args: &[String]) -> Result<(), ()> {
    #[allow(unused_mut)]
    let mut config = Config::runtime(java_args);

    #[cfg(feature = "rcon")]
    prepare_rcon(&mut config);

    rewrite_server_properties(&config);

    service::server::service(Arc::new(config))
}

#[cfg(feature = "rcon")]
fn prepare_rcon(config: &mut Config) {
    if !config.rcon.enabled {
        return;
    }

    if config.rcon.randomize_password {
        config.rcon.password = generate_random_password();
    }
}

#[cfg(feature = "rcon")]
fn generate_random_password() -> String {
    use rand::{distributions::Alphanumeric, Rng};
    use std::iter;

    let mut rng = rand::thread_rng();
    iter::repeat(())
        .map(|()| rng.sample(Alphanumeric))
        .map(char::from)
        .take(RCON_PASSWORD_LENGTH)
        .collect()
}

fn rewrite_server_properties(config: &Config) {
    if !config.advanced.rewrite_server_properties {
        return;
    }

    let dir = match crate::config::Server::server_directory(config) {
        Some(dir) => dir,
        None => return,
    };

    #[allow(unused_mut)]
    let mut changes = HashMap::from([
        ("server-ip", config.server.address.ip().to_string()),
        ("server-port", config.server.address.port().to_string()),
        ("query.port", config.server.address.port().to_string()),
        ("enable-status", "true".into()),
        ("view-distance", "5".into()),
        ("simulation-distance", "5".into()),
        ("spawn-protection", "0".into()),
    ]);

    if config.join.methods.contains(&crate::config::Method::Lobby) {
        changes.extend([(
            "network-compression-threshold",
            proto::COMPRESSION_THRESHOLD.to_string(),
        )]);
    }

    #[cfg(feature = "rcon")]
    if config.rcon.enabled {
        changes.extend([
            ("rcon.port", config.rcon.port.to_string()),
            ("rcon.password", config.rcon.password.clone()),
            ("enable-rcon", "true".into()),
        ]);
    }

    server_properties::rewrite_dir(dir, changes)
}
