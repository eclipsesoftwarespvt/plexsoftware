use std::error::Error;
use std::net::SocketAddr;

use bytes::BytesMut;
use proxy_protocol::version2::{ProxyAddresses, ProxyCommand, ProxyTransportProtocol};
use proxy_protocol::EncodeError;
use tokio::io;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;

use crate::net;

pub async fn proxy(
    inbound: TcpStream,
    proxy_header: ProxyHeader,
    addr_target: SocketAddr,
) -> Result<(), Box<dyn Error>> {
    proxy_with_queue(inbound, proxy_header, addr_target, &[]).await
}

pub async fn proxy_with_queue(
    inbound: TcpStream,
    proxy_header: ProxyHeader,
    addr_target: SocketAddr,
    queue: &[u8],
) -> Result<(), Box<dyn Error>> {

    let mut outbound = TcpStream::connect(addr_target).await?;

    match proxy_header {
        ProxyHeader::None => {}
        ProxyHeader::Local => {
            let header = local_proxy_header()?;
            outbound.write_all(&header).await?;
        }
        ProxyHeader::Proxy => {
            let header = stream_proxy_header(&inbound)?;
            outbound.write_all(&header).await?;
        }
    }

    proxy_inbound_outbound_with_queue(inbound, outbound, &[], queue).await
}

pub async fn proxy_inbound_outbound_with_queue(
    mut inbound: TcpStream,
    mut outbound: TcpStream,
    inbound_queue: &[u8],
    outbound_queue: &[u8],
) -> Result<(), Box<dyn Error>> {
    let (mut ri, mut wi) = inbound.split();
    let (mut ro, mut wo) = outbound.split();

    if !inbound_queue.is_empty() {
        wi.writable().await?;
        trace!(target: "plexpaper", "Relaying {} queued bytes to client", inbound_queue.len());
        wi.write_all(inbound_queue).await?;
    }

    if !outbound_queue.is_empty() {
        wo.writable().await?;
        trace!(target: "plexpaper", "Relaying {} queued bytes to server", outbound_queue.len());
        wo.write_all(outbound_queue).await?;
    }

    let client_to_server = async {
        io::copy(&mut ri, &mut wo).await?;
        wo.shutdown().await
    };
    let server_to_client = async {
        io::copy(&mut ro, &mut wi).await?;
        wi.shutdown().await
    };

    tokio::try_join!(client_to_server, server_to_client)?;

    net::close_tcp_stream(inbound).await?;

    Ok(())
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum ProxyHeader {

    None,

    #[allow(unused)]
    Local,

    Proxy,
}

impl ProxyHeader {

    pub fn not_none(self, not_none: bool) -> Self {
        if not_none {
            self
        } else {
            Self::None
        }
    }
}

pub fn local_proxy_header() -> Result<BytesMut, EncodeError> {

    let header = proxy_protocol::ProxyHeader::Version2 {
        command: ProxyCommand::Local,
        transport_protocol: ProxyTransportProtocol::Stream,
        addresses: ProxyAddresses::Unspec,
    };

    proxy_protocol::encode(header)
}

pub fn stream_proxy_header(inbound: &TcpStream) -> Result<BytesMut, EncodeError> {

    let peer = inbound
        .peer_addr()
        .expect("Peer address not known for TCP stream");
    let local = inbound
        .local_addr()
        .expect("Local address not known for TCP stream");

    let header = proxy_protocol::ProxyHeader::Version2 {
        command: ProxyCommand::Proxy,
        transport_protocol: ProxyTransportProtocol::Stream,
        addresses: match (peer, local) {
            (SocketAddr::V4(source), SocketAddr::V4(destination)) => ProxyAddresses::Ipv4 {
                source,
                destination,
            },
            (SocketAddr::V6(source), SocketAddr::V6(destination)) => ProxyAddresses::Ipv6 {
                source,
                destination,
            },
            (_, _) => unreachable!(),
        },
    };

    proxy_protocol::encode(header)
}
