//! Thin FFmpeg hardware-H.264 sink for the GRE-190 prototype.

use std::io::Write;
use std::process::{Child, ChildStdin, Command, Stdio};

use crate::camera::{FULL_HEIGHT, FULL_WIDTH, RAW8_FRAME_BYTES};

pub struct H264Sink {
    child: Child,
    input: Option<ChildStdin>,
}

impl H264Sink {
    pub fn publish(
        rtsp_url: &str,
        fps: u32,
        bayer_pixel_format: &str,
        video_filter: &str,
    ) -> Result<Self, String> {
        let mut child = Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "warning",
                "-re",
                "-f",
                "rawvideo",
                "-pixel_format",
                bayer_pixel_format,
                "-video_size",
                &format!("{FULL_WIDTH}x{FULL_HEIGHT}"),
                "-framerate",
                &fps.to_string(),
                "-i",
                "pipe:0",
                "-vf",
                video_filter,
                "-c:v",
                "h264_v4l2m2m",
                "-profile:v",
                "578",
                "-b:v",
                "8M",
                "-g",
                &fps.to_string(),
                "-f",
                "rtsp",
                "-rtsp_transport",
                "tcp",
                rtsp_url,
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .spawn()
            .map_err(|error| format!("could not start FFmpeg H.264 sink: {error}"))?;
        let input = child
            .stdin
            .take()
            .ok_or_else(|| "FFmpeg did not expose stdin".to_owned())?;
        Ok(Self {
            child,
            input: Some(input),
        })
    }

    pub fn encode(&mut self, raw8: &[u8]) -> Result<(), String> {
        if raw8.len() != RAW8_FRAME_BYTES {
            return Err("H.264 sink received an invalid RAW8 frame".to_owned());
        }
        self.input
            .as_mut()
            .ok_or_else(|| "FFmpeg input is closed".to_owned())?
            .write_all(raw8)
            .map_err(|error| format!("FFmpeg H.264 input failed: {error}"))
    }

    pub fn finish(mut self) -> Result<(), String> {
        drop(self.input.take());
        let status = self
            .child
            .wait()
            .map_err(|error| format!("could not wait for FFmpeg: {error}"))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("FFmpeg H.264 sink exited with {status}"))
        }
    }
}
