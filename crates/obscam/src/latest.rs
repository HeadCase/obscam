use std::sync::{
    Condvar, Mutex,
    atomic::{AtomicU64, Ordering},
};

use crate::MonochromeFrame;
use std::time::Duration;
use zwo_asi::{HEIGHT, WIDTH};

const I420_BYTES: usize = WIDTH * HEIGHT * 3 / 2;

/// A two-buffer, single-consumer handoff with at most one pending generation.
pub struct LatestFrameMailbox {
    state: Mutex<State>,
    available: Condvar,
    newest_generation: AtomicU64,
}

impl LatestFrameMailbox {
    /// Allocates one in-flight and one replaceable pending I420 buffer.
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: Mutex::new(State {
                pending: None,
                spare: vec![new_buffer(), new_buffer()],
            }),
            available: Condvar::new(),
            newest_generation: AtomicU64::new(0),
        }
    }

    /// Copies a complete frame into the sole pending slot, replacing older work.
    ///
    /// # Panics
    ///
    /// Panics after mailbox mutex poisoning or violation of the single-consumer
    /// buffer-ownership invariant; either condition is an internal invariant failure.
    pub fn publish(&self, frame: &MonochromeFrame<'_>) {
        let mut state = self.state.lock().expect("frame mailbox mutex poisoned");
        let newest = self.newest_generation.load(Ordering::Relaxed);
        if frame.generation() <= newest {
            return;
        }

        let mut pending = state.pending.take().unwrap_or_else(|| {
            state
                .spare
                .pop()
                .expect("one buffer remains while the consumer owns at most one")
        });
        pending.generation = frame.generation();
        pending.data.copy_from_slice(frame.data());
        state.pending = Some(pending);
        self.newest_generation
            .store(frame.generation(), Ordering::Release);
        self.available.notify_one();
    }

    /// Takes the newest pending frame, leaving no queued work.
    ///
    /// # Panics
    ///
    /// Panics after mailbox mutex poisoning, which indicates an internal invariant failure.
    #[must_use]
    pub fn take(&self) -> Option<PublishedFrame> {
        self.state
            .lock()
            .expect("frame mailbox mutex poisoned")
            .pending
            .take()
    }

    /// Waits for and takes the newest pending frame, bounded by `timeout`.
    ///
    /// # Panics
    ///
    /// Panics after mailbox mutex poisoning, which indicates an internal invariant failure.
    #[must_use]
    pub fn wait_take(&self, timeout: Duration) -> Option<PublishedFrame> {
        let state = self.state.lock().expect("frame mailbox mutex poisoned");
        let (mut state, _) = self
            .available
            .wait_timeout_while(state, timeout, |state| state.pending.is_none())
            .expect("frame mailbox mutex poisoned while waiting");
        state.pending.take()
    }

    /// Returns a completed or discarded in-flight buffer for reuse.
    ///
    /// # Panics
    ///
    /// Panics after mailbox mutex poisoning, which indicates an internal invariant failure.
    pub fn recycle(&self, frame: PublishedFrame) {
        self.state
            .lock()
            .expect("frame mailbox mutex poisoned")
            .spare
            .push(frame);
    }

    /// Reports whether newer work arrived while this generation was executing.
    #[must_use]
    pub fn is_obsolete(&self, generation: u64) -> bool {
        self.newest_generation.load(Ordering::Acquire) > generation
    }
}

impl Default for LatestFrameMailbox {
    fn default() -> Self {
        Self::new()
    }
}

struct State {
    pending: Option<PublishedFrame>,
    spare: Vec<PublishedFrame>,
}

/// One owned native-dimension I420 generation taken from the latest-only handoff.
pub struct PublishedFrame {
    generation: u64,
    data: Box<[u8]>,
}

impl PublishedFrame {
    /// Source generation represented by these bytes.
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// Complete native-dimension I420 bytes.
    #[must_use]
    pub const fn data(&self) -> &[u8] {
        &self.data
    }
}

fn new_buffer() -> PublishedFrame {
    PublishedFrame {
        generation: 0,
        data: vec![0; I420_BYTES].into_boxed_slice(),
    }
}
