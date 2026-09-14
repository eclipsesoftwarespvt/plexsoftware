use std::collections::HashMap;
use std::error::Error;
use std::fs;
use std::net::IpAddr;
use std::path::Path;

use chrono::{DateTime, Utc};
use serde::Deserialize;

pub const FILE: &str = "banned-ips.json";

const EXPIRY_FOREVER: &str = "forever";

#[derive(Debug, Default)]
pub struct BannedIps {

    ips: HashMap<IpAddr, BannedIp>,
}

impl BannedIps {

    pub fn get(&self, ip: &IpAddr) -> Option<BannedIp> {
        self.ips.get(ip).cloned()
    }

    pub fn is_banned(&self, ip: &IpAddr) -> bool {
        self.ips.get(ip).map(|ip| ip.is_banned()).unwrap_or(false)
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct BannedIp {

    pub ip: IpAddr,

    pub created: Option<String>,

    pub source: Option<String>,

    pub expires: Option<String>,

    pub reason: Option<String>,
}

impl BannedIp {

    pub fn is_banned(&self) -> bool {

        let expires = match &self.expires {
            Some(expires) => expires,
            None => return true,
        };

        if expires.trim().to_lowercase() == EXPIRY_FOREVER {
            return true;
        }

        let expiry = match DateTime::parse_from_str(expires, "%Y-%m-%d %H:%M:%S %z") {
            Ok(expiry) => expiry,
            Err(err) => {
                error!(target: "plexpaper", "Failed to parse ban expiry '{}', assuming still banned: {}", expires, err);
                return true;
            }
        };

        expiry > Utc::now()
    }
}

pub fn load(path: &Path) -> Result<BannedIps, Box<dyn Error>> {

    let contents = fs::read_to_string(path)?;

    let ips: Vec<BannedIp> = serde_json::from_str(&contents)?;
    debug!(target: "plexpaper", "Loaded {} banned IPs", ips.len());

    let ips = ips.into_iter().map(|ip| (ip.ip, ip)).collect();
    Ok(BannedIps { ips })
}
