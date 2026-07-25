//! FFmpeg JPEG sink used to test the independent browser-delivery finalist.

use std::io::{Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{sync_channel, Receiver};
use std::thread::{self, JoinHandle};

use crate::camera::{FULL_HEIGHT, FULL_WIDTH, RAW8_FRAME_BYTES};

pub struct JpegSink {
    child: Child,
    input: Option<ChildStdin>,
    reader: Option<JoinHandle<()>>,
}

impl JpegSink {
    pub fn new(
        bayer_pixel_format: &str,
        quality: u8,
    ) -> Result<(Self, Receiver<Result<Vec<u8>, String>>), String> {
        let mut child = Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "warning",
                "-f",
                "rawvideo",
                "-pixel_format",
                bayer_pixel_format,
                "-video_size",
                &format!("{FULL_WIDTH}x{FULL_HEIGHT}"),
                "-i",
                "pipe:0",
                "-vf",
                "format=yuv420p",
                "-threads",
                "1",
                "-c:v",
                "mjpeg",
                "-q:v",
                &quality.to_string(),
                "-fps_mode",
                "passthrough",
                "-flush_packets",
                "1",
                "-f",
                "image2pipe",
                "pipe:1",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|error| format!("could not start FFmpeg JPEG sink: {error}"))?;
        let input = child
            .stdin
            .take()
            .ok_or_else(|| "FFmpeg did not expose JPEG stdin".to_owned())?;
        let output = child
            .stdout
            .take()
            .ok_or_else(|| "FFmpeg did not expose JPEG stdout".to_owned())?;
        let (sender, receiver) = sync_channel(1);
        let reader = thread::spawn(move || {
            let mut output = JpegReader::new(output);
            loop {
                let frame = output.read_frame();
                let failed = frame.is_err();
                if sender.send(frame).is_err() || failed {
                    break;
                }
            }
        });
        Ok((
            Self {
                child,
                input: Some(input),
                reader: Some(reader),
            },
            receiver,
        ))
    }

    pub fn submit(&mut self, raw8: &[u8]) -> Result<(), String> {
        if raw8.len() != RAW8_FRAME_BYTES {
            return Err("JPEG sink received an invalid RAW8 frame".to_owned());
        }
        self.input
            .as_mut()
            .ok_or_else(|| "FFmpeg JPEG input is closed".to_owned())?
            .write_all(raw8)
            .map_err(|error| format!("FFmpeg JPEG input failed: {error}"))
    }

    pub fn finish(mut self) -> Result<(), String> {
        drop(self.input.take());
        let status = self
            .child
            .wait()
            .map_err(|error| format!("could not wait for JPEG FFmpeg: {error}"))?;
        if let Some(reader) = self.reader.take() {
            reader
                .join()
                .map_err(|_| "FFmpeg JPEG reader panicked".to_owned())?;
        }
        if status.success() {
            Ok(())
        } else {
            Err(format!("FFmpeg JPEG sink exited with {status}"))
        }
    }
}

struct JpegReader {
    output: ChildStdout,
    pending: Vec<u8>,
}

impl JpegReader {
    fn new(output: ChildStdout) -> Self {
        Self {
            output,
            pending: Vec::new(),
        }
    }

    fn read_frame(&mut self) -> Result<Vec<u8>, String> {
        loop {
            if let Some(start) = marker(&self.pending, [0xff, 0xd8], 0) {
                if start > 0 {
                    self.pending.drain(..start);
                }
                if let Some(end) = marker(&self.pending, [0xff, 0xd9], 2) {
                    return Ok(self.pending.drain(..end + 2).collect());
                }
            }
            let mut chunk = [0_u8; 64 * 1024];
            let read = self
                .output
                .read(&mut chunk)
                .map_err(|error| format!("FFmpeg JPEG output failed: {error}"))?;
            if read == 0 {
                return Err("FFmpeg JPEG output ended before a complete frame".to_owned());
            }
            self.pending.extend_from_slice(&chunk[..read]);
        }
    }
}

fn marker(data: &[u8], wanted: [u8; 2], offset: usize) -> Option<usize> {
    data.get(offset..)?
        .windows(2)
        .position(|window| window == wanted)
        .map(|index| index + offset)
}

#[cfg(test)]
mod tests {
    use super::marker;

    #[test]
    fn marker_honors_search_offset() {
        let data = [0xff, 0xd8, 1, 0xff, 0xd9];
        assert_eq!(marker(&data, [0xff, 0xd8], 0), Some(0));
        assert_eq!(marker(&data, [0xff, 0xd9], 2), Some(3));
        assert_eq!(marker(&data, [0xff, 0xd8], 1), None);
    }
}
