use std::error::Error;
use std::io;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;

pub async fn close_tcp_stream(mut stream: TcpStream) -> Result<(), Box<dyn Error>> {
    close_tcp_stream_ref(&mut stream).await
}

pub async fn close_tcp_stream_ref(stream: &mut TcpStream) -> Result<(), Box<dyn Error>> {
    match stream.shutdown().await {
        Ok(_) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotConnected => Ok(()),
        Err(err) => Err(err.into()),
    }
}
