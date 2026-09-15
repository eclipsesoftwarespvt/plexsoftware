use std::net::SocketAddr;
use std::sync::Arc;

use bytes::BytesMut;
use futures::FutureExt;
use tokio::net::{TcpListener, TcpStream};

use crate::config::Config;
use crate::proto::client::Client;
use crate::proxy::{self, ProxyHeader};
use crate::server::{self, Server};
use crate::service;
use crate::status;
use crate::util::error::{quit_error, ErrorHints};

#[tokio::main(flavor = "multi_thread")]
pub async fn service(config: Arc<Config>) -> Result<(), ()> {

    let server = Arc::new(Server::default());

    let listener = TcpListener::bind(config.public.address)
        .await
        .map_err(|err| {
            quit_error(
                anyhow!(err).context("Failed to start proxy server"),
                ErrorHints::default(),
            );
        })?;

    info!(target: "plexpaper", "PlexScale v{} — @lucawyck", env!("CARGO_PKG_VERSION"));
    info!(
        target: "plexpaper",
        "Proxying public {} to PlexScale secure server gateway...",
        config.public.address,
    );
    info!(target: "plexpaper", "Boot complete, currently sleeping...");

    if config.lockout.enabled {
        warn!(
            target: "plexpaper",
            "Lockout mode is enabled, nobody will be able to connect through the proxy",
        );
    }

    tokio::spawn(service::monitor::service(config.clone(), server.clone()));
    tokio::spawn(service::signal::service(config.clone(), server.clone()));

    if config.server.wake_on_start {
        Server::start(config.clone(), server.clone(), None).await;
    }

    tokio::spawn(service::probe::service(config.clone(), server.clone()));
    tokio::task::spawn_blocking({
        let (config, server) = (config.clone(), server.clone());
        || service::file_watcher::service(config, server)
    });

    while let Ok((inbound, _)) = listener.accept().await {
        route(inbound, config.clone(), server.clone());
    }

    Ok(())
}

#[inline]
fn route(inbound: TcpStream, config: Arc<Config>, server: Arc<Server>) {

    let peer = match inbound.peer_addr() {
        Ok(peer) => peer,
        Err(err) => {
            warn!(target: "plexpaper", "Connection from unknown peer address, disconnecting: {}", err);
            return;
        }
    };

    let banned = server.is_banned_ip_blocking(&peer.ip());
    if banned && config.server.drop_banned_ips {
        info!(target: "plexpaper", "Connection from banned IP {}, dropping", peer.ip());
        return;
    }

    let should_proxy =
        !banned && server.state() == server::State::Started && !config.lockout.enabled;
    if should_proxy {
        route_proxy(inbound, config)
    } else {
        route_status(inbound, config, server, peer)
    }
}

#[inline]
fn route_status(inbound: TcpStream, config: Arc<Config>, server: Arc<Server>, peer: SocketAddr) {

    let client = Client::new(peer);
    let service = status::serve(client, inbound, config, server).map(|r| {
        if let Err(err) = r {
            warn!(target: "plexpaper", "Failed to serve status: {:?}", err);
        }
    });

    tokio::spawn(service);
}

#[inline]
fn route_proxy(inbound: TcpStream, config: Arc<Config>) {

    let service = proxy::proxy(
        inbound,
        ProxyHeader::Proxy.not_none(config.server.send_proxy_v2),
        config.server.address,
    )
    .map(|r| {
        if let Err(err) = r {
            warn!(target: "plexpaper", "Failed to proxy: {}", err);
        }
    });

    tokio::spawn(service);
}

#[inline]
pub fn route_proxy_queue(inbound: TcpStream, config: Arc<Config>, queue: BytesMut) {
    route_proxy_address_queue(
        inbound,
        ProxyHeader::Proxy.not_none(config.server.send_proxy_v2),
        config.server.address,
        queue,
    );
}

#[inline]
pub fn route_proxy_address_queue(
    inbound: TcpStream,
    proxy_header: ProxyHeader,
    addr: SocketAddr,
    queue: BytesMut,
) {

    let service = async move {
        proxy::proxy_with_queue(inbound, proxy_header, addr, &queue)
            .map(|r| {
                if let Err(err) = r {
                    warn!(target: "plexpaper", "Failed to proxy: {}", err);
                }
            })
            .await
    };

    tokio::spawn(service);
}
