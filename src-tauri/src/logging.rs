use std::{
    io::{self, Write},
    path::Path,
};

use tracing_appender::non_blocking::{NonBlocking, WorkerGuard};
use tracing_subscriber::fmt::MakeWriter;

use crate::errors::{AppError, AppResult, redact};

#[derive(Clone)]
struct RedactingMakeWriter {
    inner: NonBlocking,
}

struct RedactingWriter {
    inner: NonBlocking,
    buffer: Vec<u8>,
}

impl<'writer> MakeWriter<'writer> for RedactingMakeWriter {
    type Writer = RedactingWriter;

    fn make_writer(&'writer self) -> Self::Writer {
        RedactingWriter {
            inner: self.inner.clone(),
            buffer: Vec::new(),
        }
    }
}

impl Write for RedactingWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.buffer.extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for RedactingWriter {
    fn drop(&mut self) {
        let formatted = String::from_utf8_lossy(&self.buffer);
        let _ = self.inner.write_all(redact(&formatted).as_bytes());
        let _ = self.inner.flush();
    }
}

pub fn init(log_directory: &Path) -> AppResult<WorkerGuard> {
    let appender = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("textbooklens.log")
        .max_log_files(5)
        .build(log_directory)
        .map_err(AppError::local_io)?;
    let (writer, guard) = tracing_appender::non_blocking(appender);

    tracing_subscriber::fmt()
        .with_ansi(false)
        .with_target(false)
        .with_writer(RedactingMakeWriter { inner: writer })
        .try_init()
        .map_err(AppError::local_io)?;

    Ok(guard)
}
