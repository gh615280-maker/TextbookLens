#![allow(dead_code)]

use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::Notify,
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

pub struct GatedSseServer {
    uri: String,
    pub first_written: Arc<Notify>,
    pub allow_terminal: Arc<Notify>,
    pub terminal_written: Arc<Notify>,
    request_count: Arc<AtomicUsize>,
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

impl GatedSseServer {
    pub async fn start(first: Vec<u8>, terminal: Vec<u8>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let first_written = Arc::new(Notify::new());
        let allow_terminal = Arc::new(Notify::new());
        let terminal_written = Arc::new(Notify::new());
        let request_count = Arc::new(AtomicUsize::new(0));
        let first_signal = first_written.clone();
        let terminal_gate = allow_terminal.clone();
        let terminal_signal = terminal_written.clone();
        let request_counter = request_count.clone();
        let task = tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await {
                request_counter.fetch_add(1, Ordering::SeqCst);
                if read_http_request(&mut socket).await.is_ok()
                    && socket
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
                        )
                        .await
                        .is_ok()
                    && write_http_chunk(&mut socket, &first).await.is_ok()
                {
                    first_signal.notify_one();
                    terminal_gate.notified().await;
                    let _ = write_http_chunk(&mut socket, &terminal).await;
                    let _ = socket.write_all(b"0\r\n\r\n").await;
                    let _ = socket.flush().await;
                    terminal_signal.notify_one();
                }
            }
        });
        Self {
            uri: format!("http://{address}"),
            first_written,
            allow_terminal,
            terminal_written,
            request_count,
            task,
        }
    }

    pub fn uri(&self) -> &str {
        &self.uri
    }

    pub fn request_count(&self) -> usize {
        self.request_count.load(Ordering::SeqCst)
    }
}

impl Drop for GatedSseServer {
    fn drop(&mut self) {
        self.allow_terminal.notify_one();
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

async fn read_http_request(socket: &mut TcpStream) -> io::Result<()> {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 4_096];
    let mut required = None;
    loop {
        let read = socket.read(&mut buffer).await?;
        if read == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "request ended before its declared body",
            ));
        }
        request.extend_from_slice(&buffer[..read]);
        if request.len() > 16 * 1024 * 1024 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "test request exceeded bound",
            ));
        }
        if required.is_none()
            && let Some(header_end) = request.windows(4).position(|window| window == b"\r\n\r\n")
        {
            let header_end = header_end + 4;
            let headers = String::from_utf8_lossy(&request[..header_end]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
                .unwrap_or(0);
            required = Some(header_end + content_length);
        }
        if required.is_some_and(|required| request.len() >= required) {
            return Ok(());
        }
    }
}

async fn write_http_chunk(socket: &mut TcpStream, bytes: &[u8]) -> io::Result<()> {
    socket
        .write_all(format!("{:X}\r\n", bytes.len()).as_bytes())
        .await?;
    socket.write_all(bytes).await?;
    socket.write_all(b"\r\n").await?;
    socket.flush().await
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
