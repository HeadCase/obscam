//! Interactive synthetic driver for the GRE-190 ownership prototype.

use std::env;
use std::io::{self, Write};

use gre_190_rust_backend::camera::{CameraOwner, RAW8_FRAME_BYTES};
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
