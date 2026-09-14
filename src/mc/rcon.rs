use std::time::Duration;

use rust_rcon::{Connection, Error as RconError};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::time;

use crate::config::Config;
use crate::proxy;

const QUIRK_RCON_GRACE_TIME: Duration = Duration::from_millis(200);

pub struct Rcon {
    con: Connection<TcpStream>,
}

impl Rcon {

    pub async fn connect(
        config: &Config,
        addr: &str,
        pass: &str,
    ) -> Result<Self, Box<dyn std::error::Error>> {

        let mut stream = TcpStream::connect(addr).await?;

        if config.rcon.send_proxy_v2 {
            trace!(target: "plexpaper::rcon", "Sending local proxy header for RCON connection");
            stream.write_all(&proxy::local_proxy_header()?).await?;
        }

        let con = Connection::builder()
            .enable_minecraft_quirks(true)
            .handshake(stream, pass)
            .await?;

        Ok(Self { con })
    }

    pub async fn connect_config(config: &Config) -> Result<Self, Box<dyn std::error::Error>> {

        let mut addr = config.server.address;
        addr.set_port(config.rcon.port);
        let addr = addr.to_string();

        Self::connect(config, &addr, &config.rcon.password).await
    }

    pub async fn cmd(&mut self, cmd: &str) -> Result<String, RconError> {

        time::sleep(QUIRK_RCON_GRACE_TIME).await;

        debug!(target: "plexpaper::rcon", "Sending RCON: {}", cmd);
        self.con.cmd(cmd).await
    }

    pub async fn close(self) {

        time::sleep(QUIRK_RCON_GRACE_TIME).await;
    }
}
