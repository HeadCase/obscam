use std::{
    sync::{
        Condvar, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use crate::MonochromeFrame;
use zwo_asi::{HEIGHT, WIDTH};

const I420_BYTES: usize = WIDTH * HEIGHT * 3 / 2;

/// A two-buffer, single-consumer I420 handoff with at most one pending generation.
pub struct LatestFrameMailbox {
    inner: LatestBufferMailbox,
}

impl LatestFrameMailbox {
    /// Allocates one in-flight and one replaceable pending I420 buffer.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: LatestBufferMailbox::new(I420_BYTES),
        }
    }

    /// Copies a complete frame into the sole pending slot, replacing older work.
    ///
    /// # Panics
    ///
    /// Panics after mailbox mutex poisoning or violation of the single-consumer
    /// buffer-ownership invariant; either condition is an internal invariant failure.
    pub fn publish(&self, frame: &MonochromeFrame<'_>) {
        self.inner.publish(frame.generation(), frame.data());
    }

    /// Takes the newest pending frame, leaving no queued work.
    ///
    /// # Panics
    ///
    /// Panics after mailbox mutex poisoning, which indicates an internal invariant failure.
    #[must_use]
    pub fn take(&self) -> Option<PublishedFrame> {
        self.inner.take().map(PublishedFrame)
    }

    /// Waits for and takes the newest pending frame, bounded by `timeout`.
    ///
    /// # Panics
    ///
    /// Panics after mailbox mutex poisoning, which indicates an internal invariant failure.
    #[must_use]
    pub fn wait_take(&self, timeout: Duration) -> Option<PublishedFrame> {
        self.inner.wait_take(timeout).map(PublishedFrame)
    }

    /// Returns a completed or discarded in-flight buffer for reuse.
    ///
    /// # Panics
    ///
    /// Panics after mailbox mutex poisoning, which indicates an internal invariant failure.
    pub fn recycle(&self, frame: PublishedFrame) {
        self.inner.recycle(frame.0);
    }

    /// Reports whether newer work arrived while this generation was executing.
    #[must_use]
    pub fn is_obsolete(&self, generation: u64) -> bool {
        self.inner.is_obsolete(generation)
    }
}

impl Default for LatestFrameMailbox {
    fn default() -> Self {
        Self::new()
    }
}

pub(crate) struct LatestBufferMailbox {
    state: Mutex<State>,
    available: Condvar,
    newest_generation: AtomicU64,
}

impl LatestBufferMailbox {
    pub(crate) fn new(buffer_bytes: usize) -> Self {
        Self {
            state: Mutex::new(State {
                pending: None,
                spare: vec![new_buffer(buffer_bytes), new_buffer(buffer_bytes)],
            }),
            available: Condvar::new(),
            newest_generation: AtomicU64::new(0),
        }
    }

    pub(crate) fn publish(&self, generation: u64, data: &[u8]) {
        let mut state = self.state.lock().expect("frame mailbox mutex poisoned");
        let newest = self.newest_generation.load(Ordering::Relaxed);
        if generation <= newest {
            return;
        }

        let mut pending = state.pending.take().unwrap_or_else(|| {
            state
                .spare
                .pop()
                .expect("one buffer remains while the consumer owns at most one")
        });
        pending.generation = generation;
        pending.data.copy_from_slice(data);
        state.pending = Some(pending);
        self.newest_generation.store(generation, Ordering::Release);
        self.available.notify_one();
    }

    pub(crate) fn take(&self) -> Option<BufferGeneration> {
        self.state
            .lock()
            .expect("frame mailbox mutex poisoned")
            .pending
            .take()
    }

    pub(crate) fn wait_take(&self, timeout: Duration) -> Option<BufferGeneration> {
        let state = self.state.lock().expect("frame mailbox mutex poisoned");
        let (mut state, _) = self
            .available
            .wait_timeout_while(state, timeout, |state| state.pending.is_none())
            .expect("frame mailbox mutex poisoned while waiting");
        state.pending.take()
    }

    pub(crate) fn recycle(&self, frame: BufferGeneration) {
        self.state
            .lock()
            .expect("frame mailbox mutex poisoned")
            .spare
            .push(frame);
    }

    pub(crate) fn is_obsolete(&self, generation: u64) -> bool {
        self.newest_generation.load(Ordering::Acquire) > generation
    }
}

struct State {
    pending: Option<BufferGeneration>,
    spare: Vec<BufferGeneration>,
}

pub(crate) struct BufferGeneration {
    generation: u64,
    data: Box<[u8]>,
}

impl BufferGeneration {
    pub(crate) const fn generation(&self) -> u64 {
        self.generation
    }

    pub(crate) const fn data(&self) -> &[u8] {
        &self.data
    }
}

/// One owned native-dimension I420 generation taken from the latest-only handoff.
pub struct PublishedFrame(BufferGeneration);

impl PublishedFrame {
    /// Source generation represented by these bytes.
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.0.generation()
    }

    /// Complete native-dimension I420 bytes.
    #[must_use]
    pub const fn data(&self) -> &[u8] {
        self.0.data()
    }
}

fn new_buffer(buffer_bytes: usize) -> BufferGeneration {
    BufferGeneration {
        generation: 0,
        data: vec![0; buffer_bytes].into_boxed_slice(),
    }
}
