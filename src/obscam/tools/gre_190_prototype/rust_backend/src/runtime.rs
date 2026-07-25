//! Concurrent fixed-pool runtime around the pure ownership model.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use crate::camera::RAW8_FRAME_BYTES;
use crate::{BackendState, Consumer, FrameRef};

pub struct FrameHub {
    state: Mutex<BackendState>,
    available: Condvar,
    buffers: Vec<Mutex<Vec<u8>>>,
    stopped: AtomicBool,
}

impl FrameHub {
    pub fn new(buffer_count: usize) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(BackendState::new(buffer_count)),
            available: Condvar::new(),
            buffers: (0..buffer_count)
                .map(|_| Mutex::new(vec![0; RAW8_FRAME_BYTES]))
                .collect(),
            stopped: AtomicBool::new(false),
        })
    }

    pub fn capture<F>(&self, capture: F) -> Result<FrameRef, String>
    where
        F: FnOnce(&mut [u8]) -> Result<u64, String>,
    {
        let buffer = self
            .state
            .lock()
            .map_err(|_| "frame state mutex was poisoned".to_owned())?
            .begin_capture()
            .ok_or_else(|| "capture buffer pool is exhausted".to_owned())?;
        let completed_ns = {
            let mut bytes = self.buffers[buffer]
                .lock()
                .map_err(|_| "frame buffer mutex was poisoned".to_owned())?;
            capture(&mut bytes)?
        };
        let frame = self
            .state
            .lock()
            .map_err(|_| "frame state mutex was poisoned".to_owned())?
            .publish(buffer, completed_ns);
        self.available.notify_all();
        Ok(frame)
    }

    pub fn consume<F>(&self, consumer: Consumer, mut consume: F) -> Result<(), String>
    where
        F: FnMut(&[u8], FrameRef) -> Result<(), String>,
    {
        loop {
            let frame = {
                let mut state = self
                    .state
                    .lock()
                    .map_err(|_| "frame state mutex was poisoned".to_owned())?;
                loop {
                    if let Some(frame) = state.start_consumer(consumer) {
                        break Some(frame);
                    }
                    if self.stopped.load(Ordering::Acquire) {
                        break None;
                    }
                    let waited = self
                        .available
                        .wait_timeout(state, Duration::from_millis(100))
                        .map_err(|_| "frame state mutex was poisoned".to_owned())?;
                    state = waited.0;
                }
            };
            let frame = match frame {
                Some(frame) => frame,
                None => return Ok(()),
            };
            let result = {
                let bytes = self.buffers[frame.buffer]
                    .lock()
                    .map_err(|_| "frame buffer mutex was poisoned".to_owned())?;
                consume(&bytes, frame)
            };
            self.state
                .lock()
                .map_err(|_| "frame state mutex was poisoned".to_owned())?
                .complete_consumer(consumer);
            result?;
        }
    }

    pub fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
        self.available.notify_all();
    }

    pub fn render(&self) -> Result<String, String> {
        self.state
            .lock()
            .map(|state| state.render())
            .map_err(|_| "frame state mutex was poisoned".to_owned())
    }
}
