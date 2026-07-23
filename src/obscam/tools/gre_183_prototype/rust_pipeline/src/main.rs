use std::env;
use std::ffi::CStr;
use std::fmt::Write;
use std::os::raw::{c_char, c_double, c_float, c_int, c_long, c_uchar, c_uint, c_ulong};
use std::process::ExitCode;
use std::ptr;
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const ASI_SUCCESS: c_int = 0;
const ASI_IMG_RAW8: c_int = 0;
const ASI_IMG_RGB24: c_int = 1;
const ASI_IMG_Y8: c_int = 3;
const ASI_IMG_END: c_int = -1;
const ASI_GAIN: c_int = 0;
const ASI_EXPOSURE: c_int = 1;
const ASI_BANDWIDTH_OVERLOAD: c_int = 6;
const ASI_HIGH_SPEED_MODE: c_int = 14;
const FULL_WIDTH: c_int = 1920;
const FULL_HEIGHT: c_int = 1080;
const GUARD_BYTES: usize = 64;
const BUFFER_COUNT: usize = 4;
const MAX_EXPECTED_FPS: f64 = 150.0;
const TIMING_HEADROOM: usize = 1024;

#[repr(C)]
#[derive(Clone, Copy)]
struct AsiCameraInfo {
    name: [c_char; 64],
    camera_id: c_int,
    max_height: c_long,
    max_width: c_long,
    is_color_camera: c_int,
    bayer_pattern: c_int,
    supported_bins: [c_int; 16],
    supported_video_format: [c_int; 8],
    pixel_size: c_double,
    mechanical_shutter: c_int,
    st4_port: c_int,
    is_cooler_camera: c_int,
    is_usb3_host: c_int,
    is_usb3_camera: c_int,
    electrons_per_adu: c_float,
    bit_depth: c_int,
    is_trigger_camera: c_int,
    unused: [c_char; 16],
}

impl Default for AsiCameraInfo {
    fn default() -> Self {
        // The SDK contract treats a zeroed structure as an empty output buffer.
        unsafe { std::mem::zeroed() }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct AsiControlCaps {
    name: [c_char; 64],
    description: [c_char; 128],
    maximum: c_long,
    minimum: c_long,
    default: c_long,
    is_auto_supported: c_int,
    is_writable: c_int,
    control_type: c_int,
    unused: [c_char; 32],
}

impl Default for AsiControlCaps {
    fn default() -> Self {
        // The SDK contract treats a zeroed structure as an empty output buffer.
        unsafe { std::mem::zeroed() }
    }
}

#[link(name = "ASICamera2")]
extern "C" {
    fn ASIGetNumOfConnectedCameras() -> c_int;
    fn ASIGetCameraProperty(info: *mut AsiCameraInfo, camera_index: c_int) -> c_int;
    fn ASIOpenCamera(camera_id: c_int) -> c_int;
    fn ASIInitCamera(camera_id: c_int) -> c_int;
    fn ASICloseCamera(camera_id: c_int) -> c_int;
    fn ASIGetNumOfControls(camera_id: c_int, count: *mut c_int) -> c_int;
    fn ASIGetControlCaps(
        camera_id: c_int,
        control_index: c_int,
        caps: *mut AsiControlCaps,
    ) -> c_int;
    fn ASISetControlValue(
        camera_id: c_int,
        control_type: c_int,
        value: c_long,
        automatic: c_int,
    ) -> c_int;
    fn ASISetROIFormat(
        camera_id: c_int,
        width: c_int,
        height: c_int,
        bins: c_int,
        image_type: c_int,
    ) -> c_int;
    fn ASISetStartPos(camera_id: c_int, start_x: c_int, start_y: c_int) -> c_int;
    fn ASIStartVideoCapture(camera_id: c_int) -> c_int;
    fn ASIStopVideoCapture(camera_id: c_int) -> c_int;
    fn ASIStopExposure(camera_id: c_int) -> c_int;
    fn ASIGetVideoData(
        camera_id: c_int,
        buffer: *mut c_uchar,
        buffer_size: c_long,
        wait_ms: c_int,
    ) -> c_int;
    fn ASIGetDroppedFrames(camera_id: c_int, dropped_frames: *mut c_int) -> c_int;
    fn ASIGetSDKVersion() -> *const c_char;
}

#[link(name = "z")]
extern "C" {
    fn crc32(crc: c_ulong, buffer: *const c_uchar, length: c_uint) -> c_ulong;
}

#[derive(Clone)]
struct Scenario {
    image_type: c_int,
    image_format: String,
    exposure_us: c_long,
    gain: c_long,
    high_speed: c_long,
    bandwidth: c_long,
    duration: Duration,
    duration_s: f64,
    warmup_frames: usize,
    timeout_ms: c_int,
    camera_model: String,
}

impl Scenario {
    fn frame_bytes(&self) -> usize {
        let bytes_per_pixel = if self.image_type == ASI_IMG_RGB24 {
            3
        } else {
            1
        };
        FULL_WIDTH as usize * FULL_HEIGHT as usize * bytes_per_pixel
    }
}

struct Camera {
    id: c_int,
    info: AsiCameraInfo,
}

struct VideoCapture {
    camera_id: c_int,
    active: bool,
}

impl VideoCapture {
    fn start(camera_id: c_int) -> Result<Self, String> {
        checked(
            unsafe { ASIStartVideoCapture(camera_id) },
            "ASIStartVideoCapture",
        )?;
        Ok(Self {
            camera_id,
            active: true,
        })
    }

    fn stop(&mut self) -> Result<(), String> {
        if !self.active {
            return Ok(());
        }
        checked(
            unsafe { ASIStopVideoCapture(self.camera_id) },
            "ASIStopVideoCapture",
        )?;
        self.active = false;
        Ok(())
    }
}

impl Drop for VideoCapture {
    fn drop(&mut self) {
        if self.active {
            unsafe {
                ASIStopVideoCapture(self.camera_id);
            }
        }
    }
}

impl Drop for Camera {
    fn drop(&mut self) {
        unsafe {
            ASICloseCamera(self.id);
        }
    }
}

struct CompletedFrame {
    allocation: Vec<u8>,
    completion_ms: f64,
}

#[derive(Default)]
struct LatestSlot {
    frame: Option<CompletedFrame>,
    done: bool,
}

struct CaptureStats {
    elapsed_s: f64,
    frames: usize,
    capture_errors: usize,
    corrupt_frames: usize,
    pipeline_drops: usize,
    sdk_dropped_start: c_int,
    sdk_dropped_end: c_int,
    capture_call_ms: Vec<f64>,
    inter_frame_ms: Vec<f64>,
}

struct ConsumerStats {
    unique_frames: usize,
    adjacent_duplicates: usize,
    crc32_last: Option<u32>,
    unique_inter_frame_ms: Vec<f64>,
}

fn checked(code: c_int, operation: &str) -> Result<(), String> {
    if code == ASI_SUCCESS {
        Ok(())
    } else {
        Err(format!("{operation} failed with ASI error {code}"))
    }
}

fn parse_number<T>(value: Option<String>, option: &str) -> Result<T, String>
where
    T: std::str::FromStr,
{
    value
        .ok_or_else(|| format!("{option} requires a value"))?
        .parse::<T>()
        .map_err(|_| format!("invalid value for {option}"))
}

fn parse_args<I>(arguments: I) -> Result<Scenario, String>
where
    I: IntoIterator<Item = String>,
{
    let mut arguments = arguments.into_iter();
    let _program = arguments.next();
    let mut image_format: Option<String> = None;
    let mut exposure_us: Option<c_long> = None;
    let mut gain: Option<c_long> = None;
    let mut high_speed: Option<c_long> = None;
    let mut bandwidth: Option<c_long> = None;
    let mut duration_s: Option<f64> = None;
    let mut warmup_frames: Option<usize> = None;
    let mut timeout_ms: Option<c_int> = None;
    let mut camera_model = "ASI662MC".to_owned();

    while let Some(option) = arguments.next() {
        match option.as_str() {
            "--format" => image_format = Some(parse_number(arguments.next(), &option)?),
            "--exposure-us" => exposure_us = Some(parse_number(arguments.next(), &option)?),
            "--gain" => gain = Some(parse_number(arguments.next(), &option)?),
            "--high-speed" => high_speed = Some(parse_number(arguments.next(), &option)?),
            "--bandwidth" => bandwidth = Some(parse_number(arguments.next(), &option)?),
            "--duration-s" => duration_s = Some(parse_number(arguments.next(), &option)?),
            "--warmup-frames" => warmup_frames = Some(parse_number(arguments.next(), &option)?),
            "--timeout-ms" => timeout_ms = Some(parse_number(arguments.next(), &option)?),
            "--camera-model" => {
                camera_model = arguments
                    .next()
                    .ok_or_else(|| "--camera-model requires a value".to_owned())?
            }
            _ => return Err(format!("unknown option {option}")),
        }
    }

    let image_format = image_format.ok_or_else(|| "--format is required".to_owned())?;
    let image_type = match image_format.as_str() {
        "Y8" => ASI_IMG_Y8,
        "RAW8" => ASI_IMG_RAW8,
        "RGB24" => ASI_IMG_RGB24,
        _ => return Err("--format must be Y8, RAW8, or RGB24".to_owned()),
    };
    let exposure_us = exposure_us.ok_or_else(|| "--exposure-us is required".to_owned())?;
    let gain = gain.ok_or_else(|| "--gain is required".to_owned())?;
    let high_speed = high_speed.ok_or_else(|| "--high-speed is required".to_owned())?;
    let bandwidth = bandwidth.ok_or_else(|| "--bandwidth is required".to_owned())?;
    let duration_s = duration_s.ok_or_else(|| "--duration-s is required".to_owned())?;
    let warmup_frames = warmup_frames.ok_or_else(|| "--warmup-frames is required".to_owned())?;
    let timeout_ms = timeout_ms.ok_or_else(|| "--timeout-ms is required".to_owned())?;
    if exposure_us < 1
        || gain < 0
        || !matches!(high_speed, 0 | 1)
        || bandwidth < 0
        || !duration_s.is_finite()
        || duration_s <= 0.0
        || timeout_ms < 1
    {
        return Err("scenario values are outside the supported range".to_owned());
    }
    Ok(Scenario {
        image_type,
        image_format,
        exposure_us,
        gain,
        high_speed,
        bandwidth,
        duration: Duration::from_secs_f64(duration_s),
        duration_s,
        warmup_frames,
        timeout_ms,
        camera_model,
    })
}

fn camera_name(info: &AsiCameraInfo) -> Result<String, String> {
    unsafe { CStr::from_ptr(info.name.as_ptr()) }
        .to_str()
        .map(str::to_owned)
        .map_err(|error| format!("camera name is not UTF-8: {error}"))
}

fn sdk_version() -> Result<String, String> {
    let version_pointer = unsafe { ASIGetSDKVersion() };
    if version_pointer.is_null() {
        return Err("ASIGetSDKVersion returned null".to_owned());
    }
    unsafe { CStr::from_ptr(version_pointer) }
        .to_str()
        .map(str::to_owned)
        .map_err(|error| format!("SDK version is not UTF-8: {error}"))
}

fn open_required_camera(required_model: &str) -> Result<Camera, String> {
    let camera_count = unsafe { ASIGetNumOfConnectedCameras() };
    for camera_index in 0..camera_count {
        let mut info = AsiCameraInfo::default();
        checked(
            unsafe { ASIGetCameraProperty(&mut info, camera_index) },
            "ASIGetCameraProperty",
        )?;
        if !camera_name(&info)?.contains(required_model) {
            continue;
        }
        checked(unsafe { ASIOpenCamera(info.camera_id) }, "ASIOpenCamera")?;
        if let Err(error) = checked(unsafe { ASIInitCamera(info.camera_id) }, "ASIInitCamera") {
            unsafe {
                ASICloseCamera(info.camera_id);
            }
            return Err(error);
        }
        return Ok(Camera {
            id: info.camera_id,
            info,
        });
    }
    Err(format!("required camera {required_model} not found"))
}

fn supports_format(info: &AsiCameraInfo, image_type: c_int) -> bool {
    info.supported_video_format
        .iter()
        .take_while(|&&value| value != ASI_IMG_END)
        .any(|&value| value == image_type)
}

fn supports_writable_control(camera_id: c_int, control_type: c_int) -> bool {
    let mut control_count = 0;
    if unsafe { ASIGetNumOfControls(camera_id, &mut control_count) } != ASI_SUCCESS {
        return false;
    }
    (0..control_count).any(|control_index| {
        let mut caps = AsiControlCaps::default();
        (unsafe { ASIGetControlCaps(camera_id, control_index, &mut caps) }) == ASI_SUCCESS
            && caps.control_type == control_type
            && caps.is_writable != 0
    })
}

fn configure_camera(camera: &Camera, scenario: &Scenario) -> Result<(), String> {
    if camera.info.max_width != FULL_WIDTH as c_long
        || camera.info.max_height != FULL_HEIGHT as c_long
        || !supports_format(&camera.info, scenario.image_type)
    {
        return Err("camera does not support the requested full-frame format".to_owned());
    }
    unsafe {
        ASIStopVideoCapture(camera.id);
        ASIStopExposure(camera.id);
    }
    checked(
        unsafe { ASISetControlValue(camera.id, ASI_EXPOSURE, scenario.exposure_us, 0) },
        "set exposure",
    )?;
    checked(
        unsafe { ASISetControlValue(camera.id, ASI_GAIN, scenario.gain, 0) },
        "set gain",
    )?;
    checked(
        unsafe { ASISetControlValue(camera.id, ASI_BANDWIDTH_OVERLOAD, scenario.bandwidth, 0) },
        "set USB bandwidth",
    )?;
    if supports_writable_control(camera.id, ASI_HIGH_SPEED_MODE) {
        checked(
            unsafe { ASISetControlValue(camera.id, ASI_HIGH_SPEED_MODE, scenario.high_speed, 0) },
            "set high-speed mode",
        )?;
    }
    checked(
        unsafe { ASISetROIFormat(camera.id, FULL_WIDTH, FULL_HEIGHT, 1, scenario.image_type) },
        "set full-frame output",
    )?;
    checked(
        unsafe { ASISetStartPos(camera.id, 0, 0) },
        "set full-frame origin",
    )
}

fn allocate_guarded(frame_bytes: usize) -> Vec<u8> {
    let mut allocation = vec![0_u8; frame_bytes + (2 * GUARD_BYTES)];
    allocation[..GUARD_BYTES].fill(0xA5);
    allocation[GUARD_BYTES + frame_bytes..].fill(0xA5);
    allocation
}

fn guards_intact(allocation: &[u8], frame_bytes: usize) -> bool {
    allocation[..GUARD_BYTES].iter().all(|&value| value == 0xA5)
        && allocation[GUARD_BYTES + frame_bytes..]
            .iter()
            .all(|&value| value == 0xA5)
}

fn sdk_dropped_frames(camera_id: c_int) -> Result<c_int, String> {
    let mut dropped_frames = 0;
    checked(
        unsafe { ASIGetDroppedFrames(camera_id, &mut dropped_frames) },
        "ASIGetDroppedFrames",
    )?;
    Ok(dropped_frames)
}

fn capture_thread(
    camera_id: c_int,
    scenario: Scenario,
    free_buffers: Receiver<Vec<u8>>,
    latest: Arc<(Mutex<LatestSlot>, Condvar)>,
) -> Result<CaptureStats, String> {
    let frame_bytes = scenario.frame_bytes();
    let mut video_capture = VideoCapture::start(camera_id)?;
    let mut active_buffer = free_buffers
        .recv()
        .map_err(|_| "buffer pool closed during warm-up".to_owned())?;
    for _ in 0..scenario.warmup_frames {
        checked(
            unsafe {
                ASIGetVideoData(
                    camera_id,
                    active_buffer.as_mut_ptr().add(GUARD_BYTES),
                    frame_bytes as c_long,
                    scenario.timeout_ms,
                )
            },
            "warm-up ASIGetVideoData",
        )?;
    }

    let sdk_dropped_start = sdk_dropped_frames(camera_id)?;
    let measurement_started = Instant::now();
    let timing_capacity =
        (scenario.duration_s * MAX_EXPECTED_FPS).ceil() as usize + TIMING_HEADROOM;
    let mut capture_call_ms = Vec::with_capacity(timing_capacity);
    let mut inter_frame_ms = Vec::with_capacity(timing_capacity);
    let mut previous_completion: Option<Instant> = None;
    let mut frames = 0;
    let mut capture_errors = 0;
    let mut corrupt_frames = 0;
    let mut pipeline_drops = 0;

    while measurement_started.elapsed() < scenario.duration {
        let capture_started = Instant::now();
        let capture_code = unsafe {
            ASIGetVideoData(
                camera_id,
                active_buffer.as_mut_ptr().add(GUARD_BYTES),
                frame_bytes as c_long,
                scenario.timeout_ms,
            )
        };
        if capture_code != ASI_SUCCESS {
            capture_errors += 1;
            continue;
        }
        let completion = Instant::now();
        frames += 1;
        capture_call_ms.push((completion - capture_started).as_secs_f64() * 1000.0);
        if let Some(previous) = previous_completion {
            inter_frame_ms.push((completion - previous).as_secs_f64() * 1000.0);
        }
        previous_completion = Some(completion);
        if !guards_intact(&active_buffer, frame_bytes) {
            corrupt_frames += 1;
        }

        let completed_frame = CompletedFrame {
            allocation: active_buffer,
            completion_ms: measurement_started.elapsed().as_secs_f64() * 1000.0,
        };
        let (slot_mutex, available) = &*latest;
        let mut slot = slot_mutex
            .lock()
            .map_err(|_| "latest-frame mutex was poisoned".to_owned())?;
        let replaced = slot.frame.replace(completed_frame);
        available.notify_one();
        drop(slot);
        if let Some(replaced_frame) = replaced {
            pipeline_drops += 1;
            active_buffer = replaced_frame.allocation;
        } else {
            active_buffer = free_buffers
                .recv()
                .map_err(|_| "buffer pool closed during capture".to_owned())?;
        }
    }

    let elapsed_s = measurement_started.elapsed().as_secs_f64();
    let sdk_dropped_end = sdk_dropped_frames(camera_id)?;
    video_capture.stop()?;
    let (slot_mutex, available) = &*latest;
    let mut slot = slot_mutex
        .lock()
        .map_err(|_| "latest-frame mutex was poisoned".to_owned())?;
    slot.done = true;
    available.notify_one();
    drop(slot);

    Ok(CaptureStats {
        elapsed_s,
        frames,
        capture_errors,
        corrupt_frames,
        pipeline_drops,
        sdk_dropped_start,
        sdk_dropped_end,
        capture_call_ms,
        inter_frame_ms,
    })
}

fn signal_capture_done(latest: &Arc<(Mutex<LatestSlot>, Condvar)>) {
    let (slot_mutex, available) = &**latest;
    if let Ok(mut slot) = slot_mutex.lock() {
        slot.done = true;
        available.notify_one();
    }
}

fn consume_frames(
    frame_bytes: usize,
    free_buffers: SyncSender<Vec<u8>>,
    latest: Arc<(Mutex<LatestSlot>, Condvar)>,
) -> Result<ConsumerStats, String> {
    let mut previous_crc32: Option<u32> = None;
    let mut previous_unique_completion_ms: Option<f64> = None;
    let mut unique_inter_frame_ms = Vec::new();
    let mut unique_frames = 0;
    let mut adjacent_duplicates = 0;

    loop {
        let (slot_mutex, available) = &*latest;
        let mut slot = slot_mutex
            .lock()
            .map_err(|_| "latest-frame mutex was poisoned".to_owned())?;
        while slot.frame.is_none() && !slot.done {
            slot = available
                .wait(slot)
                .map_err(|_| "latest-frame mutex was poisoned".to_owned())?;
        }
        let completed_frame = slot.frame.take();
        let done = slot.done;
        drop(slot);

        if let Some(frame) = completed_frame {
            let frame_pointer = unsafe { frame.allocation.as_ptr().add(GUARD_BYTES) };
            let checksum = unsafe { crc32(0, ptr::null(), 0) };
            let checksum = unsafe { crc32(checksum, frame_pointer, frame_bytes as c_uint) } as u32;
            if previous_crc32 == Some(checksum) {
                adjacent_duplicates += 1;
            } else {
                unique_frames += 1;
                if let Some(previous_completion_ms) = previous_unique_completion_ms {
                    unique_inter_frame_ms.push(frame.completion_ms - previous_completion_ms);
                }
                previous_unique_completion_ms = Some(frame.completion_ms);
                previous_crc32 = Some(checksum);
            }
            free_buffers
                .send(frame.allocation)
                .map_err(|_| "buffer pool closed during consume".to_owned())?;
        } else if done {
            break;
        }
    }

    Ok(ConsumerStats {
        unique_frames,
        adjacent_duplicates,
        crc32_last: previous_crc32,
        unique_inter_frame_ms,
    })
}

fn percentile(sorted_values: &[f64], quantile: f64) -> f64 {
    let position = (sorted_values.len() - 1) as f64 * quantile;
    let lower = position.floor() as usize;
    let upper = position.ceil() as usize;
    if lower == upper {
        sorted_values[lower]
    } else {
        sorted_values[lower]
            + ((sorted_values[upper] - sorted_values[lower]) * (position - lower as f64))
    }
}

fn timing_summary_json(mut values: Vec<f64>) -> String {
    if values.is_empty() {
        return "{\"count\":0,\"minimum_ms\":null,\"mean_ms\":null,\"p50_ms\":null,\"p95_ms\":null,\"p99_ms\":null,\"maximum_ms\":null}".to_owned();
    }
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    values.sort_by(f64::total_cmp);
    format!(
        "{{\"count\":{},\"minimum_ms\":{:.9},\"mean_ms\":{:.9},\"p50_ms\":{:.9},\"p95_ms\":{:.9},\"p99_ms\":{:.9},\"maximum_ms\":{:.9}}}",
        values.len(),
        values[0],
        mean,
        percentile(&values, 0.50),
        percentile(&values, 0.95),
        percentile(&values, 0.99),
        values[values.len() - 1],
    )
}

fn json_string(value: &str) -> String {
    let mut output = String::with_capacity(value.len() + 2);
    output.push('"');
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            character if character < ' ' => {
                write!(&mut output, "\\u{:04x}", character as u32)
                    .expect("writing to a String cannot fail");
            }
            _ => output.push(character),
        }
    }
    output.push('"');
    output
}

fn result_json(
    scenario: &Scenario,
    camera: &Camera,
    capture: CaptureStats,
    consumer: ConsumerStats,
) -> Result<String, String> {
    if capture.sdk_dropped_end < capture.sdk_dropped_start {
        return Err("SDK drop counter moved backwards".to_owned());
    }
    let frame_bytes = scenario.frame_bytes();
    let crc32_last = consumer
        .crc32_last
        .map_or_else(|| "null".to_owned(), |value| value.to_string());
    Ok(format!(
        "{{\"schema_version\":1,\"runner\":\"rust-pipeline\",\"scenario\":{{\"image_format\":{},\"exposure_us\":{},\"gain\":{},\"high_speed\":{},\"bandwidth\":{},\"duration_s\":{:.9},\"warmup_frames\":{},\"timeout_ms\":{},\"width\":1920,\"height\":1080}},\"sdk_version\":{},\"camera_name\":{},\"elapsed_s\":{:.9},\"frames\":{},\"unique_frames\":{},\"adjacent_duplicates\":{},\"cadence_fps\":{:.9},\"unique_cadence_fps\":{:.9},\"sdk_dropped_start\":{},\"sdk_dropped_end\":{},\"sdk_dropped_delta\":{},\"capture_errors\":{},\"corrupt_frames\":{},\"pipeline_drops\":{},\"buffer_allocations\":{},\"buffer_allocation_bytes\":{},\"downstream_copy_bytes\":0,\"crc32_last\":{},\"capture_call_ms\":{},\"inter_frame_ms\":{},\"unique_inter_frame_ms\":{},\"notes\":[\"four guarded frame buffers allocated before capture\",\"capture thread replaces an unconsumed frame with the latest frame\",\"CRC32 consumer reads buffers in place without copying\"]}}",
        json_string(&scenario.image_format),
        scenario.exposure_us,
        scenario.gain,
        scenario.high_speed,
        scenario.bandwidth,
        scenario.duration_s,
        scenario.warmup_frames,
        scenario.timeout_ms,
        json_string(&sdk_version()?),
        json_string(&camera_name(&camera.info)?),
        capture.elapsed_s,
        capture.frames,
        consumer.unique_frames,
        consumer.adjacent_duplicates,
        capture.frames as f64 / capture.elapsed_s,
        consumer.unique_frames as f64 / capture.elapsed_s,
        capture.sdk_dropped_start,
        capture.sdk_dropped_end,
        capture.sdk_dropped_end - capture.sdk_dropped_start,
        capture.capture_errors,
        capture.corrupt_frames,
        capture.pipeline_drops,
        BUFFER_COUNT,
        BUFFER_COUNT * (frame_bytes + (2 * GUARD_BYTES)),
        crc32_last,
        timing_summary_json(capture.capture_call_ms),
        timing_summary_json(capture.inter_frame_ms),
        timing_summary_json(consumer.unique_inter_frame_ms),
    ))
}

fn run() -> Result<(), String> {
    let scenario = parse_args(env::args())?;
    let camera = open_required_camera(&scenario.camera_model)?;
    configure_camera(&camera, &scenario)?;
    let frame_bytes = scenario.frame_bytes();
    let (free_sender, free_receiver) = sync_channel(BUFFER_COUNT);
    for _ in 0..BUFFER_COUNT {
        free_sender
            .send(allocate_guarded(frame_bytes))
            .map_err(|_| "could not initialize buffer pool".to_owned())?;
    }
    let latest = Arc::new((Mutex::new(LatestSlot::default()), Condvar::new()));
    let capture_latest = Arc::clone(&latest);
    let capture_scenario = scenario.clone();
    let camera_id = camera.id;
    let capture_handle = thread::spawn(move || {
        let result = capture_thread(
            camera_id,
            capture_scenario,
            free_receiver,
            Arc::clone(&capture_latest),
        );
        if result.is_err() {
            signal_capture_done(&capture_latest);
        }
        result
    });
    let consumer_result = consume_frames(frame_bytes, free_sender, latest);
    let capture_result = capture_handle
        .join()
        .map_err(|_| "capture thread panicked".to_owned())?;
    let consumer = consumer_result?;
    let capture = capture_result?;
    println!("{}", result_json(&scenario, &camera, capture, consumer)?);
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{json_string, parse_args, percentile};

    #[test]
    fn parses_common_runner_arguments() {
        let scenario = parse_args(
            [
                "runner",
                "--format",
                "Y8",
                "--exposure-us",
                "10000",
                "--gain",
                "250",
                "--high-speed",
                "1",
                "--bandwidth",
                "80",
                "--duration-s",
                "2",
                "--warmup-frames",
                "5",
                "--timeout-ms",
                "520",
            ]
            .into_iter()
            .map(str::to_owned),
        )
        .expect("scenario should parse");

        assert_eq!(scenario.image_format, "Y8");
        assert_eq!(scenario.exposure_us, 10_000);
        assert_eq!(scenario.duration_s, 2.0);
    }

    #[test]
    fn percentile_matches_the_shared_linear_interpolation() {
        let values = [1.0, 2.0, 3.0, 4.0];

        assert_eq!(percentile(&values, 0.50), 2.5);
        assert!((percentile(&values, 0.95) - 3.85).abs() < f64::EPSILON * 10.0);
    }

    #[test]
    fn escapes_json_strings() {
        assert_eq!(json_string("a\"b\\c\n"), "\"a\\\"b\\\\c\\n\"");
    }
}
