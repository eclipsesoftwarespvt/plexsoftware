use std::sync::Arc;

use bytes::BytesMut;
use tokio::net::TcpStream;

use crate::config::*;
use crate::proxy::ProxyHeader;
use crate::service;

use super::MethodResult;

pub async fn occupy(
    config: Arc<Config>,
    inbound: TcpStream,
    inbound_history: &mut BytesMut,
) -> Result<MethodResult, ()> {
    trace!(target: "plexpaper", "Using forward method to occupy joining client");

    debug!(target: "plexpaper", "Forwarding client to {:?}!", config.join.forward.address);

    service::server::route_proxy_address_queue(
        inbound,
        ProxyHeader::Proxy.not_none(config.join.forward.send_proxy_v2),
        config.join.forward.address,
        inbound_history.clone(),
    );

    Ok(MethodResult::Consumed)
}
