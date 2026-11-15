//! A decoder stall before a baseline exists is not stale inventory.
#[derive(Default)]
pub struct Health {
    ready: bool,
    paused: bool,
    capture_loss: bool,
}

impl Health {
    pub fn paused(&self) -> bool {
        self.paused
    }

    /// Returns true only when a new pause needs to be announced.
    pub fn observe(&mut self, capture_loss: bool, decoder_stalled: bool) -> bool {
        self.capture_loss |= capture_loss;
        let must_pause = self.capture_loss || (self.ready && decoder_stalled);
        let transition = must_pause && !self.paused;
        if must_pause {
            self.paused = true;
            self.ready = false;
        }
        transition
    }

    pub fn session_reset(&mut self) {
        self.ready = false;
        self.paused = self.capture_loss;
    }

    /// A fresh baseline can recover from a decoder stall, never from known capture loss.
    pub fn snapshot(&mut self) -> bool {
        self.paused = self.capture_loss;
        self.ready = !self.capture_loss;
        self.ready
    }

    pub fn invalid_update(&mut self) {
        self.ready = false;
        self.paused = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn startup_traffic_then_handshake_and_snapshot_enables_tracking() {
        let mut health = Health::default();
        assert!(!health.observe(false, true)); // traffic from an already-running old connection
        assert!(!health.paused());
        health.session_reset();
        health.session_reset();
        assert!(!health.observe(false, true)); // reconnect, baseline still unavailable
        assert!(health.snapshot());
        assert!(!health.paused());
        assert!(!health.observe(false, false));
        assert!(health.observe(false, true)); // a stall after the snapshot really is stale
        assert!(health.paused());
        assert!(!health.observe(false, true));
        health.session_reset();
        assert!(!health.paused());
        assert!(health.snapshot());
        health.invalid_update();
        assert!(health.paused());
        assert!(health.snapshot());
        assert!(!health.paused());
    }
    #[test]
    fn capture_loss_latches_even_during_a_soft_pause() {
        let mut health = Health::default();
        health.snapshot();
        health.observe(false, true);
        health.observe(true, false);
        health.session_reset();
        assert!(health.paused());
        assert!(!health.snapshot());
        let mut startup = Health::default();
        assert!(startup.observe(true, false));
        assert!(!startup.snapshot());
    }
}
