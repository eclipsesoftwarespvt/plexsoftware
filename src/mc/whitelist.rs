use std::error::Error;
use std::fs;
use std::path::Path;

use serde::Deserialize;

pub const WHITELIST_FILE: &str = "whitelist.json";

pub const OPS_FILE: &str = "ops.json";

#[derive(Debug, Default)]
pub struct Whitelist {

    whitelist: Vec<String>,

    ops: Vec<String>,
}

impl Whitelist {

    pub fn is_whitelisted(&self, username: &str) -> bool {
        self.whitelist.iter().any(|u| u == username) || self.ops.iter().any(|u| u == username)
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct WhitelistUser {

    #[serde(rename = "name", alias = "username")]
    pub username: String,

    pub uuid: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct OpUser {

    #[serde(rename = "name", alias = "username")]
    pub username: String,

    pub uuid: Option<String>,

    pub level: Option<u32>,

    #[serde(rename = "bypassesPlayerLimit")]
    pub byapsses_player_limit: Option<bool>,
}

pub fn load_dir(path: &Path) -> Result<Whitelist, Box<dyn Error>> {
    let whitelist_file = path.join(WHITELIST_FILE);
    let ops_file = path.join(OPS_FILE);

    let whitelist = if whitelist_file.is_file() {
        load_whitelist(&whitelist_file)?
    } else {
        vec![]
    };

    let ops = if ops_file.is_file() {
        load_ops(&ops_file)?
    } else {
        vec![]
    };

    debug!(target: "plexpaper", "Loaded {} whitelist and {} OP users", whitelist.len(), ops.len());

    Ok(Whitelist { whitelist, ops })
}

fn load_whitelist(path: &Path) -> Result<Vec<String>, Box<dyn Error>> {

    let contents = fs::read_to_string(path)?;

    let users: Vec<WhitelistUser> = serde_json::from_str(&contents)?;

    Ok(users.into_iter().map(|user| user.username).collect())
}

fn load_ops(path: &Path) -> Result<Vec<String>, Box<dyn Error>> {

    let contents = fs::read_to_string(path)?;

    let users: Vec<OpUser> = serde_json::from_str(&contents)?;

    Ok(users.into_iter().map(|user| user.username).collect())
}
