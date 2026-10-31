//! An asynchronous sink wrapper that processes records on a background thread,
//! mirroring loguru's `enqueue=True`.

use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::Arc;
use std::thread::JoinHandle;

use crate::record::Record;

use super::Sink;

enum Msg {
    Record(Box<Record>),
    Flush,
    Complete(SyncSender<()>),
}

/// Wraps another [`Sink`] so that records are handed off to a background
/// thread instead of being written on the logging thread.
///
/// This keeps logging calls from blocking on slow I/O. Records are delivered in
/// order. Call [`Sink::complete`] (or [`crate::Logger::complete`]) to wait for
/// the queue to drain; dropping the sink also drains and joins the thread.
///
/// ```no_run
/// use loguru::{AsyncSink, FileSink};
/// let sink = AsyncSink::new(FileSink::new("app.log")?);
/// # Ok::<(), std::io::Error>(())
/// ```
pub struct AsyncSink {
    level: u16,
    tx: Option<SyncSender<Msg>>,
    handle: Option<JoinHandle<()>>,
}

impl AsyncSink {
    /// Wrap `inner` in a background-thread queue. The queue holds up to
    /// `capacity` pending records before `write` blocks (back-pressure).
    pub fn with_capacity<S: Sink + 'static>(inner: S, capacity: usize) -> Self {
        let level = inner.level();
        let (tx, rx): (SyncSender<Msg>, Receiver<Msg>) = sync_channel(capacity);
        let inner: Arc<dyn Sink> = Arc::new(inner);
        let handle = std::thread::Builder::new()
            .name("loguru-async".into())
            .spawn(move || worker(inner, rx))
            .expect("failed to spawn loguru async sink thread");
        AsyncSink {
            level,
            tx: Some(tx),
            handle: Some(handle),
        }
    }

    /// Wrap `inner` with a default queue capacity (1024 records).
    pub fn new<S: Sink + 'static>(inner: S) -> Self {
        Self::with_capacity(inner, 1024)
    }
}

fn worker(inner: Arc<dyn Sink>, rx: Receiver<Msg>) {
    while let Ok(msg) = rx.recv() {
        match msg {
            Msg::Record(record) => inner.write(&record),
            Msg::Flush => inner.flush(),
            Msg::Complete(ack) => {
                inner.flush();
                let _ = ack.send(());
            }
        }
    }
    inner.flush();
}

impl Sink for AsyncSink {
    fn level(&self) -> u16 {
        self.level
    }

    fn write(&self, record: &Record) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(Msg::Record(Box::new(record.clone())));
        }
    }

    fn flush(&self) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(Msg::Flush);
        }
    }

    fn complete(&self) {
        if let Some(tx) = &self.tx {
            let (ack_tx, ack_rx) = sync_channel(1);
            if tx.send(Msg::Complete(ack_tx)).is_ok() {
                let _ = ack_rx.recv();
            }
        }
    }
}

impl Drop for AsyncSink {
    fn drop(&mut self) {
        // Close the channel so the worker drains and exits, then join it.
        self.tx.take();
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}
