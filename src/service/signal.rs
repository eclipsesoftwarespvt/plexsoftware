use std::sync::Arc;

use crate::config::Config;
use crate::server::{self, Server};
use crate::util::error;

pub async fn service(config: Arc<Config>, server: Arc<Server>) {
    loop {

        tokio::signal::ctrl_c().await.unwrap();

        if server.state() == server::State::Stopped {
            quit();
        }

        let stopping = server.stop(&config).await;

        if !stopping {
            quit();
        }
    }
}

fn quit() -> ! {

    error::quit();
}
