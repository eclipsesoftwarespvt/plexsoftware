use std::sync::Arc;

use bytes::BytesMut;
use tokio::net::TcpStream;

use crate::config::*;
use crate::net;
use crate::proto::client::{Client, ClientInfo, ClientState};
use crate::server::Server;

pub mod forward;
pub mod hold;
pub mod kick;
#[cfg(feature = "lobby")]
pub mod lobby;

pub enum MethodResult {

    Consumed,

    Continue(TcpStream),
}

pub async fn occupy(
    client: Client,
    #[allow(unused_variables)] client_info: ClientInfo,
    config: Arc<Config>,
    server: Arc<Server>,
    mut inbound: TcpStream,
    mut inbound_history: BytesMut,
    #[allow(unused_variables)] login_queue: BytesMut,
) -> Result<(), ()> {

    assert_eq!(
        client.state(),
        ClientState::Login,
        "when occupying client, it should be in login state"
    );

    for method in &config.join.methods {

        let result = match method {

            Method::Kick => kick::occupy(&client, &config, &server, inbound).await?,

            Method::Hold => {
                hold::occupy(
                    config.clone(),
                    server.clone(),
                    inbound,
                    &mut inbound_history,
                )
                .await?
            }

            Method::Forward => {
                forward::occupy(config.clone(), inbound, &mut inbound_history).await?
            }

            #[cfg(feature = "lobby")]
            Method::Lobby => {
                lobby::occupy(
                    &client,
                    client_info.clone(),
                    config.clone(),
                    server.clone(),
                    inbound,
                    login_queue.clone(),
                )
                .await?
            }

            #[cfg(not(feature = "lobby"))]
            Method::Lobby => {
                error!(target: "plexpaper", "Lobby join method not supported in this PlexPaper build");
                MethodResult::Continue(inbound)
            }
        };

        match result {
            MethodResult::Consumed => return Ok(()),
            MethodResult::Continue(stream) => {
                inbound = stream;
                continue;
            }
        }
    }

    debug!(target: "plexpaper", "No method left to occupy joining client, disconnecting");

    net::close_tcp_stream(inbound).await.map_err(|_| ())?;

    Ok(())
}
