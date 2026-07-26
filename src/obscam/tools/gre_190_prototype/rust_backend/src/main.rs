//! Interactive synthetic driver for the GRE-190 ownership prototype.

use std::env;
use std::io::{self, Write};
use std::sync::mpsc::sync_channel;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use gre_190_rust_backend::camera::{CameraOwner, RAW8_FRAME_BYTES};
use gre_190_rust_backend::h264::H264Sink;
use gre_190_rust_backend::jpeg::JpegSink;
use gre_190_rust_backend::runtime::FrameHub;
use gre_190_rust_backend::{BackendState, Consumer};

fn main() {
    let arguments = env::args().collect::<Vec<_>>();
    if arguments.get(1).map(String::as_str) == Some("identity") {
        let model = arguments.get(2).expect("identity requires exact model");
        let serial = arguments
            .get(3)
            .expect("identity requires factory serial hex");
        match CameraOwner::open_exact(model, serial) {
            Ok(camera) => println!(
                "{{\"camera_model\":\"{}\",\"camera_id\":{},\"factory_serial_hex\":\"{}\"}}",
                camera.model,
                camera.camera_id(),
                camera.serial_hex
            ),
            Err(error) => panic!("{error}"),
        }
        return;
    }
    if arguments.get(1).map(String::as_str) == Some("capture-smoke") {
        capture_smoke();
        return;
    }
    if arguments.get(1).map(String::as_str) == Some("h264-stream") {
        let duration_s = arguments
            .get(2)
            .map(|value| value.parse::<u64>().expect("duration must be seconds"))
            .unwrap_or(60);
        let exposure_us = arguments
            .get(3)
            .map(|value| value.parse::<i64>().expect("exposure must be microseconds"))
            .unwrap_or(10_000);
        let gain = arguments
            .get(4)
            .map(|value| value.parse::<i64>().expect("gain must be an integer"))
            .unwrap_or(0);
        let fps = arguments
            .get(5)
            .map(|value| value.parse::<u32>().expect("fps must be an integer"))
            .unwrap_or(20);
        let night_stretch = arguments.get(6).map(String::as_str) == Some("night");
        run_stream(duration_s, exposure_us, gain, fps, night_stretch, false);
        return;
    }
    if arguments.get(1).map(String::as_str) == Some("dual-stream") {
        let duration_s = arguments
            .get(2)
            .map(|value| value.parse::<u64>().expect("duration must be seconds"))
            .unwrap_or(60);
        let exposure_us = arguments
            .get(3)
            .map(|value| value.parse::<i64>().expect("exposure must be microseconds"))
            .unwrap_or(10_000);
        let gain = arguments
            .get(4)
            .map(|value| value.parse::<i64>().expect("gain must be an integer"))
            .unwrap_or(0);
        let fps = arguments
            .get(5)
            .map(|value| value.parse::<u32>().expect("fps must be an integer"))
            .unwrap_or(20);
        run_stream(duration_s, exposure_us, gain, fps, false, true);
        return;
    }
    if arguments.get(1).map(String::as_str) == Some("sample") {
        let exposure_us = arguments
            .get(2)
            .expect("sample requires exposure microseconds")
            .parse::<i64>()
            .expect("exposure must be microseconds");
        let gain = arguments
            .get(3)
            .expect("sample requires gain")
            .parse::<i64>()
            .expect("gain must be an integer");
        sample_signal(exposure_us, gain);
        return;
    }
    let mut state = BackendState::new(4);
    let mut timestamp = 0;
    loop {
        print!("\x1b[2J\x1b[H\x1b[1mGRE-190 Rust ownership prototype\x1b[0m\n\n");
        println!("{}", state.render());
        println!(
            "\n\x1b[1m[c]\x1b[0m capture  \x1b[1m[j/J]\x1b[0m JPEG start/complete  \
             \x1b[1m[h/H]\x1b[0m H.264 start/complete  \x1b[1m[q]\x1b[0m quit"
        );
        print!("> ");
        io::stdout().flush().expect("flush stdout");
        let mut input = String::new();
        if io::stdin().read_line(&mut input).is_err() {
            break;
        }
        match input.trim() {
            "c" => {
                timestamp += 10_000_000;
                if let Some(buffer) = state.begin_capture() {
                    state.publish(buffer, timestamp);
                }
            }
            "j" => {
                state.start_consumer(Consumer::Jpeg);
            }
            "J" => {
                state.complete_consumer(Consumer::Jpeg);
            }
            "h" => {
                state.start_consumer(Consumer::H264);
            }
            "H" => {
                state.complete_consumer(Consumer::H264);
            }
            "q" => break,
            _ => {}
        }
    }
}

fn run_stream(
    duration_s: u64,
    exposure_us: i64,
    gain: i64,
    fps: u32,
    night_stretch: bool,
    emit_jpeg: bool,
) {
    let camera =
        CameraOwner::open_exact("ZWO ASI662MC", "1d274e0920010900").expect("open exact ASI662MC");
    let bayer = camera
        .ffmpeg_bayer_pixel_format()
        .expect("supported Bayer pattern");
    let mut video = camera
        .start_raw8_video(exposure_us, gain, 1, 100)
        .expect("start RAW8 video");
    let timeout_ms = ((exposure_us / 1_000) * 2 + 500) as i32;
    let mut discarded = vec![0_u8; RAW8_FRAME_BYTES];
    video
        .capture_into(&mut discarded, timeout_ms)
        .expect("discard pre-generation frame");
    let hub = FrameHub::new(4);
    let h264_hub = Arc::clone(&hub);
    let h264 = thread::spawn(move || -> Result<(), String> {
        if emit_jpeg {
            return h264_hub.consume(Consumer::H264, |_bytes, _frame| Ok(()));
        }
        let filter = if night_stretch {
            "eq=gamma=3.0:contrast=3.0:brightness=0.1,format=yuv420p"
        } else {
            "format=yuv420p"
        };
        let mut sink = H264Sink::publish("rtsp://127.0.0.1:18554/gre190", fps, bayer, filter)?;
        h264_hub.consume(Consumer::H264, |bytes, _frame| sink.encode(bytes))?;
        sink.finish()
    });
    let jpeg_hub = Arc::clone(&hub);
    let jpeg = thread::spawn(move || -> Result<(), String> {
        if !emit_jpeg {
            return jpeg_hub.consume(Consumer::Jpeg, |_bytes, _frame| Ok(()));
        }
        let treatments = [("colour", "format=yuv420p"), ("mono", "format=gray")];
        let configured = treatments
            .iter()
            .map(|(name, filter)| {
                JpegSink::new(bayer, 5, filter)
                    .map(|(sink, output)| ((*name).to_owned(), sink, output))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut sinks = Vec::with_capacity(configured.len());
        let mut outputs = Vec::with_capacity(configured.len());
        for (treatment, sink, output) in configured {
            sinks.push(sink);
            outputs.push((treatment, output));
        }
        let (metadata_sender, metadata_receiver) = sync_channel(1);
        let packets = thread::spawn(move || -> Result<(), String> {
            while let Ok((frame, encode_start_ns, encode_end_ns)) = metadata_receiver.recv() {
                for (treatment, output) in &outputs {
                    let payload = output
                        .recv()
                        .map_err(|_| "FFmpeg JPEG reader stopped unexpectedly".to_owned())??;
                    write_jpeg_packet(
                        frame,
                        exposure_us,
                        encode_start_ns,
                        encode_end_ns,
                        treatment,
                        &payload,
                    )?;
                }
            }
            Ok(())
        });
        let result = jpeg_hub.consume(Consumer::Jpeg, |bytes, frame| {
            let encode_start_ns = unix_ns()?;
            for sink in &mut sinks {
                sink.submit(bytes)?;
            }
            let encode_end_ns = unix_ns()?;
            metadata_sender
                .send((frame, encode_start_ns, encode_end_ns))
                .map_err(|_| "JPEG packet writer stopped unexpectedly".to_owned())
        });
        for sink in sinks {
            sink.finish()?;
        }
        drop(metadata_sender);
        packets
            .join()
            .map_err(|_| "JPEG packet writer panicked".to_owned())??;
        result
    });
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(duration_s) {
        hub.capture(|frame| video.capture_into(frame, timeout_ms))
            .expect("capture into frame hub");
    }
    hub.stop();
    h264.join()
        .expect("H.264 worker panicked")
        .expect("H.264 sink");
    jpeg.join()
        .expect("JPEG worker panicked")
        .expect("JPEG sink");
    if emit_jpeg {
        eprintln!("{}", hub.render().expect("render frame hub"));
    } else {
        println!("{}", hub.render().expect("render frame hub"));
    }
}

fn unix_ns() -> Result<u64, String> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u64)
        .map_err(|_| "system clock is before Unix epoch".to_owned())
}

fn write_jpeg_packet(
    frame: gre_190_rust_backend::FrameRef,
    exposure_us: i64,
    encode_start_ns: u64,
    encode_end_ns: u64,
    treatment: &str,
    payload: &[u8],
) -> Result<(), String> {
    let exposure_end_ns = frame
        .capture_complete_ns
        .saturating_sub(exposure_us as u64 * 1_000);
    let metadata = format!(
        "{{\"schema_version\":1,\"generation\":{},\"exposure_end_ns\":{},\"exposure_end_unix_ns\":{},\"capture_complete_ns\":{},\"capture_complete_unix_ns\":{},\"encode_start_ns\":{},\"encode_start_unix_ns\":{},\"encode_end_ns\":{},\"encode_end_unix_ns\":{},\"width\":1920,\"height\":1080,\"encoded_bytes\":{},\"treatment\":\"{}\"}}",
        frame.generation,
        exposure_end_ns,
        exposure_end_ns,
        frame.capture_complete_ns,
        frame.capture_complete_ns,
        encode_start_ns,
        encode_start_ns,
        encode_end_ns,
        encode_end_ns,
        payload.len(),
        treatment
    );
    let mut output = io::stdout().lock();
    output
        .write_all(b"GREJ")
        .and_then(|_| output.write_all(&(metadata.len() as u32).to_be_bytes()))
        .and_then(|_| output.write_all(metadata.as_bytes()))
        .and_then(|_| output.write_all(&(payload.len() as u32).to_be_bytes()))
        .and_then(|_| output.write_all(payload))
        .and_then(|_| output.flush())
        .map_err(|error| format!("could not emit JPEG packet: {error}"))
}

fn sample_signal(exposure_us: i64, gain: i64) {
    let camera =
        CameraOwner::open_exact("ZWO ASI662MC", "1d274e0920010900").expect("open exact ASI662MC");
    let mut video = camera
        .start_raw8_video(exposure_us, gain, 0, 50)
        .expect("start RAW8 video");
    let timeout_ms = ((exposure_us / 1_000) * 2 + 500) as i32;
    let mut frame = vec![0_u8; RAW8_FRAME_BYTES];
    for generation in 1..=3 {
        video
            .capture_into(&mut frame, timeout_ms)
            .expect("capture signal sample");
        let sum = frame.iter().map(|&value| u64::from(value)).sum::<u64>();
        let saturated = frame.iter().filter(|&&value| value == 255).count();
        println!(
            "generation={generation} exposure_us={exposure_us} gain={gain} min={} max={} mean={:.3} saturated_percent={:.5}",
            frame.iter().min().unwrap(),
            frame.iter().max().unwrap(),
            sum as f64 / frame.len() as f64,
            saturated as f64 * 100.0 / frame.len() as f64
        );
    }
}

fn capture_smoke() {
    let camera =
        CameraOwner::open_exact("ZWO ASI662MC", "1d274e0920010900").expect("open exact ASI662MC");
    let mut video = camera
        .start_raw8_video(10_000, 0, 1, 100)
        .expect("start RAW8 video");
    let mut buffers = vec![vec![0_u8; RAW8_FRAME_BYTES]; 4];
    let mut state = BackendState::new(buffers.len());
    for _ in 0..20 {
        let buffer = state.begin_capture().expect("free capture buffer");
        let completed_ns = video
            .capture_into(&mut buffers[buffer], 500)
            .expect("capture RAW8 frame");
        state.publish(buffer, completed_ns);
        state.start_consumer(Consumer::Jpeg);
        state.start_consumer(Consumer::H264);
        state.complete_consumer(Consumer::Jpeg);
        state.complete_consumer(Consumer::H264);
    }
    println!("{}", state.render());
}
