use std::ops::Deref;
use std::sync::Arc;
use std::time::Duration;

use bytes::BytesMut;
use tokio::net::TcpStream;
use tokio::time;

use crate::config::*;
use crate::server::{Server, State};
use crate::service;

use super::MethodResult;

pub async fn occupy(
    config: Arc<Config>,
    server: Arc<Server>,
    inbound: TcpStream,
    inbound_history: &mut BytesMut,
) -> Result<MethodResult, ()> {
    trace!(target: "plexpaper", "Using hold method to occupy joining client");

    if server.state() != State::Starting {
        return Ok(MethodResult::Continue(inbound));
    }

    if hold(&config, &server).await? {
        service::server::route_proxy_queue(inbound, config, inbound_history.clone());
        return Ok(MethodResult::Consumed);
    }

    Ok(MethodResult::Continue(inbound))
}

async fn hold<'a>(config: &Config, server: &Server) -> Result<bool, ()> {
    trace!(target: "plexpaper", "Started holding client");

    let task_wait = async {
        let mut state = server.state_receiver();
        loop {

            state.changed().await.unwrap();

            match state.borrow().deref() {

                State::Starting => {
                    trace!(target: "plexpaper", "Server not ready, holding client for longer");
                    continue;
                }

                State::Started => {
                    break true;
                }

                State::Stopping => {
                    warn!(target: "plexpaper", "Server stopping for held client, disconnecting");
                    break false;
                }

                State::Stopped => {
                    error!(target: "plexpaper", "Server stopped for held client, disconnecting");
                    break false;
                }
            }
        }
    };

    let timeout = Duration::from_secs(config.join.hold.timeout as u64);
    match time::timeout(timeout, task_wait).await {

        Ok(true) => {
            info!(target: "plexpaper", "Server ready for held client, relaying to server");
            Ok(true)
        }

        Ok(false) => {
            warn!(target: "plexpaper", "Server stopping for held client");
            Ok(false)
        }

        Err(_) => {
            warn!(target: "plexpaper", "Held client reached timeout of {}s", config.join.hold.timeout);
            Ok(false)
        }
    }
}
