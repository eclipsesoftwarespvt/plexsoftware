use std::sync::Arc;

use crate::config::Config;
use crate::monitor;
use crate::server::Server;

pub async fn service(config: Arc<Config>, state: Arc<Server>) {
    monitor::monitor_server(config, state).await
}
