use std::sync::Arc;

use bytes::BytesMut;
use tokio::net::TcpStream;

use crate::config::*;
use crate::lobby;
use crate::proto::client::{Client, ClientInfo};
use crate::server::Server;

use super::MethodResult;

pub async fn occupy(
    client: &Client,
    client_info: ClientInfo,
    config: Arc<Config>,
    server: Arc<Server>,
    inbound: TcpStream,
    inbound_queue: BytesMut,
) -> Result<MethodResult, ()> {
    trace!(target: "plexpaper", "Using lobby method to occupy joining client");

    if must_still_probe(&config, &server).await {
        warn!(target: "plexpaper", "Client connected but lobby is not ready, using next join method, probing not completed");
        return Ok(MethodResult::Continue(inbound));
    }

    lobby::serve(client, client_info, inbound, config, server, inbound_queue).await?;

    Ok(MethodResult::Consumed)
}

async fn must_still_probe(config: &Config, server: &Server) -> bool {
    must_probe(config) && server.probed_join_game.read().await.is_none()
}

fn must_probe(config: &Config) -> bool {
    config.server.forge
}
