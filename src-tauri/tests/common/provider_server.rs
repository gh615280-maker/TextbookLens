#![allow(dead_code)]

use std::time::Duration;

use tokio::{
    io::AsyncWriteExt,
    net::{TcpListener, TcpStream},
    task::JoinHandle,
};
use wiremock::MockServer;

pub const TEST_CREDENTIAL: &str = "test-provider-credential-not-secret";

pub struct ProviderServer {
    inner: MockServer,
}

pub struct ChunkedSseServer {
    uri: String,
    task: JoinHandle<()>,
}

impl ChunkedSseServer {
    pub async fn start(chunks: Vec<Vec<u8>>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await {
                write_chunked_response(&mut socket, chunks).await;
            }
        });
        Self {
            uri: format!("http://{address}"),
            task,
        }
    }

    pub fn uri(&self) -> &str {
        &self.uri
    }
}

impl Drop for ChunkedSseServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn write_chunked_response(socket: &mut TcpStream, chunks: Vec<Vec<u8>>) {
    if socket
        .write_all(
            b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
        )
        .await
        .is_err()
    {
        return;
    }
    for chunk in chunks {
        let prefix = format!("{:X}\r\n", chunk.len());
        if socket.write_all(prefix.as_bytes()).await.is_err()
            || socket.write_all(&chunk).await.is_err()
            || socket.write_all(b"\r\n").await.is_err()
            || socket.flush().await.is_err()
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let _ = socket.write_all(b"0\r\n\r\n").await;
    let _ = socket.flush().await;
}

impl ProviderServer {
    pub async fn start() -> Self {
        Self {
            inner: MockServer::start().await,
        }
    }

    pub fn uri(&self) -> String {
        self.inner.uri()
    }

    pub fn mock_server(&self) -> &MockServer {
        &self.inner
    }
}
