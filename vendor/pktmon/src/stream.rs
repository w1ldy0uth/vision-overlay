#![cfg(feature = "tokio")]

use std::sync::mpsc::TryRecvError;

use futures::{Stream, StreamExt};

use crate::{Capture, CaptureBackend, Packet};

impl Capture {
    /// Get a stream of packets from the capture.
    ///
    /// This is only available if the `tokio` feature is enabled.
    pub fn stream(mut self) -> std::io::Result<impl Stream<Item = Packet> + Unpin> {
        // Start before setting notify as Consumer needs to be created first.
        // It's okay if some notify permits are missed, since we will try_recv first anyways.
        let notify = self.backend.notify().expect("Notify not available");
        self.start()?;

        Ok(async_stream::stream! {
            let mut checked = std::time::Instant::now();
            loop {
                if checked.elapsed() >= std::time::Duration::from_secs(1) {
                    if let Err(error) = self.backend.poll_health() { log::error!("Capture health check failed: {error}"); break; }
                    checked = std::time::Instant::now();
                }
                match self.backend.try_next_packet() {
                    Ok(packet) => yield packet,
                    Err(TryRecvError::Empty) => {
                        let _ = tokio::time::timeout(std::time::Duration::from_secs(1),notify.notified()).await;
                    }
                    Err(TryRecvError::Disconnected) => break,
                }
            }
        }
        .boxed())
    }
}
