//! Throwaway GRE-190 frame-ownership model.

pub mod camera;
pub mod h264;
pub mod runtime;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Consumer {
    Jpeg,
    H264,
}

impl Consumer {
    const ALL: [Self; 2] = [Self::Jpeg, Self::H264];

    const fn index(self) -> usize {
        match self {
            Self::Jpeg => 0,
            Self::H264 => 1,
        }
    }

    const fn bit(self) -> u8 {
        1 << self.index()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameRef {
    pub generation: u64,
    pub buffer: usize,
    pub capture_complete_ns: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BufferState {
    Free,
    Capturing,
    Published { generation: u64, pending: u8 },
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Counters {
    pub captured: u64,
    pub capture_starved: u64,
    pub jpeg_replaced: u64,
    pub h264_replaced: u64,
    pub jpeg_completed: u64,
    pub h264_completed: u64,
}

#[derive(Debug)]
pub struct BackendState {
    buffers: Vec<BufferState>,
    waiting: [Option<FrameRef>; 2],
    active: [Option<FrameRef>; 2],
    next_generation: u64,
    pub counters: Counters,
}

impl BackendState {
    pub fn new(buffer_count: usize) -> Self {
        assert!(buffer_count >= 3, "at least three buffers are required");
        Self {
            buffers: vec![BufferState::Free; buffer_count],
            waiting: [None; 2],
            active: [None; 2],
            next_generation: 1,
            counters: Counters::default(),
        }
    }

    pub fn begin_capture(&mut self) -> Option<usize> {
        let index = match self
            .buffers
            .iter()
            .position(|state| *state == BufferState::Free)
        {
            Some(index) => index,
            None => {
                self.counters.capture_starved += 1;
                return None;
            }
        };
        self.buffers[index] = BufferState::Capturing;
        Some(index)
    }

    pub fn publish(&mut self, buffer: usize, capture_complete_ns: u64) -> FrameRef {
        assert_eq!(self.buffers[buffer], BufferState::Capturing);
        let frame = FrameRef {
            generation: self.next_generation,
            buffer,
            capture_complete_ns,
        };
        self.next_generation += 1;
        self.counters.captured += 1;
        self.buffers[buffer] = BufferState::Published {
            generation: frame.generation,
            pending: Consumer::Jpeg.bit() | Consumer::H264.bit(),
        };
        for consumer in Consumer::ALL {
            let index = consumer.index();
            if let Some(replaced) = self.waiting[index].replace(frame) {
                self.release(replaced, consumer);
                match consumer {
                    Consumer::Jpeg => self.counters.jpeg_replaced += 1,
                    Consumer::H264 => self.counters.h264_replaced += 1,
                }
            }
        }
        frame
    }

    pub fn start_consumer(&mut self, consumer: Consumer) -> Option<FrameRef> {
        let index = consumer.index();
        if self.active[index].is_some() {
            return None;
        }
        let frame = self.waiting[index].take()?;
        self.active[index] = Some(frame);
        Some(frame)
    }

    pub fn complete_consumer(&mut self, consumer: Consumer) -> Option<FrameRef> {
        let index = consumer.index();
        let frame = self.active[index].take()?;
        self.release(frame, consumer);
        match consumer {
            Consumer::Jpeg => self.counters.jpeg_completed += 1,
            Consumer::H264 => self.counters.h264_completed += 1,
        }
        Some(frame)
    }

    fn release(&mut self, frame: FrameRef, consumer: Consumer) {
        let (generation, mut pending) = match self.buffers[frame.buffer] {
            BufferState::Published {
                generation,
                pending,
            } => (generation, pending),
            _ => panic!("consumer released a buffer that is not published"),
        };
        assert_eq!(generation, frame.generation);
        assert_ne!(pending & consumer.bit(), 0, "consumer released twice");
        pending &= !consumer.bit();
        self.buffers[frame.buffer] = if pending == 0 {
            BufferState::Free
        } else {
            BufferState::Published {
                generation,
                pending,
            }
        };
    }

    pub fn render(&self) -> String {
        let buffers = self
            .buffers
            .iter()
            .enumerate()
            .map(|(index, state)| format!("  {index}: {state:?}"))
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            "buffers:\n{buffers}\nwaiting: {:?}\nactive: {:?}\ncounters: {:#?}",
            self.waiting, self.active, self.counters
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{BackendState, Consumer};

    fn capture(state: &mut BackendState, timestamp: u64) -> u64 {
        let buffer = state.begin_capture().expect("free capture buffer");
        state.publish(buffer, timestamp).generation
    }

    #[test]
    fn slow_consumer_only_replaces_its_own_waiting_frame() {
        let mut state = BackendState::new(4);
        assert_eq!(capture(&mut state, 10), 1);
        assert_eq!(state.start_consumer(Consumer::H264).unwrap().generation, 1);
        assert_eq!(state.start_consumer(Consumer::Jpeg).unwrap().generation, 1);
        state.complete_consumer(Consumer::Jpeg);

        assert_eq!(capture(&mut state, 20), 2);
        assert_eq!(state.start_consumer(Consumer::Jpeg).unwrap().generation, 2);
        state.complete_consumer(Consumer::Jpeg);
        assert_eq!(capture(&mut state, 30), 3);
        assert_eq!(capture(&mut state, 40), 4);

        assert_eq!(state.counters.jpeg_replaced, 1);
        assert_eq!(state.counters.h264_replaced, 2);
        assert_eq!(state.start_consumer(Consumer::Jpeg).unwrap().generation, 4);
        assert!(state.start_consumer(Consumer::H264).is_none());
        state.complete_consumer(Consumer::H264);
        assert_eq!(state.start_consumer(Consumer::H264).unwrap().generation, 4);
    }

    #[test]
    fn buffers_return_only_after_both_consumers_release() {
        let mut state = BackendState::new(3);
        capture(&mut state, 10);
        state.start_consumer(Consumer::Jpeg);
        state.start_consumer(Consumer::H264);
        state.complete_consumer(Consumer::Jpeg);
        assert_eq!(state.begin_capture(), Some(1));
        state.complete_consumer(Consumer::H264);
        assert_eq!(state.begin_capture(), Some(0));
    }

    #[test]
    fn pool_exhaustion_is_explicit_and_bounded() {
        let mut state = BackendState::new(3);
        assert_eq!(state.begin_capture(), Some(0));
        assert_eq!(state.begin_capture(), Some(1));
        assert_eq!(state.begin_capture(), Some(2));
        assert_eq!(state.begin_capture(), None);
        assert_eq!(state.counters.capture_starved, 1);
    }
}
