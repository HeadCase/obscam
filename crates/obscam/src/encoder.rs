use std::{
    io::{self, Write},
    path::Path,
    process::{Child, ChildStdin, Command, Stdio},
};

use crate::PublishedFrame;

const PUBLISH_URL: &str = "rtsp://127.0.0.1:8554/obscam";

/// One long-lived `FFmpeg` hardware-H.264 publication child.
#[derive(Debug)]
pub struct FfmpegEncoder {
    child: Option<Child>,
    stdin: Option<ChildStdin>,
}

impl FfmpegEncoder {
    /// Starts the qualified native-resolution hardware encode and RTSP publication profile.
    ///
    /// # Errors
    ///
    /// Returns the process spawn or pipe error. No alternate encoder is attempted.
    pub fn start(program: &Path) -> io::Result<Self> {
        let mut child = Command::new(program)
            .args([
                "-hide_banner",
                "-loglevel",
                "warning",
                "-nostdin",
                "-f",
                "rawvideo",
                "-pixel_format",
                "yuv420p",
                "-video_size",
                "1920x1080",
                "-framerate",
                "20",
                "-i",
                "pipe:0",
                "-an",
                "-c:v",
                "h264_v4l2m2m",
                "-b:v",
                "1500k",
                "-maxrate",
                "1500k",
                "-bufsize",
                "1500k",
                "-g",
                "20",
                "-bf",
                "0",
                "-f",
                "rtsp",
                "-rtsp_transport",
                "tcp",
                PUBLISH_URL,
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("FFmpeg stdin pipe was not created"))?;
        Ok(Self {
            child: Some(child),
            stdin: Some(stdin),
        })
    }

    /// Writes one complete native I420 generation to the shared encoder.
    ///
    /// # Errors
    ///
    /// Returns the pipe error when the `FFmpeg` child no longer accepts input.
    pub fn publish(&mut self, frame: &PublishedFrame) -> io::Result<()> {
        self.stdin
            .as_mut()
            .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "FFmpeg input is closed"))?
            .write_all(frame.data())
    }

    /// Closes input and waits for the child, primarily at bounded verification seams.
    ///
    /// # Errors
    ///
    /// Returns a wait error or an error when `FFmpeg` exits unsuccessfully.
    pub fn finish(mut self) -> io::Result<()> {
        self.stdin.take();
        let status = self
            .child
            .take()
            .ok_or_else(|| io::Error::other("FFmpeg child is unavailable"))?
            .wait()?;
        if status.success() {
            Ok(())
        } else {
            Err(io::Error::other(format!(
                "FFmpeg exited with status {status}"
            )))
        }
    }
}

impl Drop for FfmpegEncoder {
    fn drop(&mut self) {
        self.stdin.take();
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
