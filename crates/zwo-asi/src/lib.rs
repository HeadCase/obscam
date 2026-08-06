//! Exclusive, fail-closed ownership of the production ZWO ASI662MC.

#![warn(missing_docs)]

use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use thiserror::Error;

#[cfg(feature = "camera-substitute")]
mod deterministic;

#[cfg(feature = "camera-substitute")]
pub use deterministic::{CapturePlan, DeterministicCamera, DeterministicScenario};

/// Exact SDK model name accepted by `ObsCam`.
pub const MODEL: &str = "ZWO ASI662MC";
/// Exact factory serial accepted by `ObsCam`.
pub const FACTORY_SERIAL: [u8; 8] = [0x1d, 0x27, 0x4e, 0x09, 0x20, 0x01, 0x09, 0x00];
/// Native sensor width.
pub const WIDTH: usize = 1920;
/// Native sensor height.
pub const HEIGHT: usize = 1080;
/// Exact number of reusable full-frame buffers.
pub const BUFFER_COUNT: usize = 4;
const FRAME_BYTES: usize = WIDTH * HEIGHT;
const ASI_TIMEOUT: i32 = 11;
const MAX_CAPTURE_WAIT_MS: i32 = 100;
const CONTROL_EXPOSURE: i32 = 1;
const CONTROL_GAIN: i32 = 0;
const MIN_EXPOSURE_US: i64 = 50_000;
const MAX_EXPOSURE_US: i64 = 30_000_000;

/// A validated complete camera settings tuple.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Settings {
    exposure_us: i64,
    gain: i64,
}

impl Settings {
    /// Validates exposure and gain against the production operating envelope.
    ///
    /// # Errors
    ///
    /// Returns [`SettingsError`] when either value is outside its accepted range.
    pub fn new(exposure_us: i64, gain: i64) -> Result<Self, SettingsError> {
        if !(MIN_EXPOSURE_US..=MAX_EXPOSURE_US).contains(&exposure_us) {
            return Err(SettingsError::Exposure);
        }
        if !(0..=600).contains(&gain) {
            return Err(SettingsError::Gain);
        }
        Ok(Self { exposure_us, gain })
    }

    /// Configured exposure duration in microseconds.
    #[must_use]
    pub const fn exposure_us(self) -> i64 {
        self.exposure_us
    }

    /// Configured sensor gain.
    #[must_use]
    pub const fn gain(self) -> i64 {
        self.gain
    }
}

/// Rejected camera setting.
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum SettingsError {
    /// Exposure is outside 50 ms through 30 s.
    #[error("exposure must be between 50000 and 30000000 microseconds")]
    Exposure,
    /// Gain is outside 0 through 600.
    #[error("gain must be between 0 and 600")]
    Gain,
}

/// A normalized camera-owner failure.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum CameraError {
    /// Exactly one production model was not present.
    #[error("expected exactly one {MODEL}, found {found}")]
    IdentityCount {
        /// Number of exact model matches.
        found: usize,
    },
    /// The exact model did not report native 1920×1080 dimensions.
    #[error("ASI662MC dimensions were {width}x{height}, expected 1920x1080")]
    InvalidDimensions {
        /// Reported width.
        width: i64,
        /// Reported height.
        height: i64,
    },
    /// The exact model did not report a colour RGGB mosaic.
    #[error("ASI662MC colour/Bayer layout was unsupported")]
    UnsupportedBayer,
    /// The exact model did not advertise RAW8 video acquisition.
    #[error("ASI662MC did not advertise RAW8 video")]
    UnsupportedFormat,
    /// The selected model had the wrong factory serial.
    #[error("ASI662MC factory serial mismatch")]
    SerialMismatch,
    /// An SDK operation failed with its original vendor code.
    #[error("ASI SDK {operation} failed with code {code}")]
    Sdk {
        /// Normalized operation name.
        operation: &'static str,
        /// Original SDK result code.
        code: i32,
    },
    /// The requested operation is invalid in the current lifecycle state.
    #[error("camera lifecycle does not allow {operation}")]
    InvalidState {
        /// Rejected operation.
        operation: &'static str,
    },
    /// The SDK returned an invalid negative cumulative drop count.
    #[error("ASI SDK returned invalid dropped-frame count {count}")]
    InvalidDropCount {
        /// Invalid vendor value.
        count: i32,
    },
    /// Starting capture failed and the SDK also rejected the compensating stop.
    #[error(
        "ASI SDK {operation} failed with code {original_code}; rollback stop failed with code {stop_code}"
    )]
    StartRollback {
        /// Operation that originally failed.
        operation: &'static str,
        /// Original failure or invalid vendor value.
        original_code: i32,
        /// SDK result from the compensating stop attempt.
        stop_code: i32,
    },
    /// Opening failed and the SDK also rejected the compensating handle close.
    #[error("ASI SDK initialization failed and ownership cleanup failed with code {close_code}")]
    OwnershipUncertain {
        /// SDK result from the compensating close attempt.
        close_code: i32,
    },
}

impl CameraError {
    /// Whether another in-process open would risk concurrent SDK ownership.
    #[must_use]
    pub const fn ownership_uncertain(&self) -> bool {
        matches!(self, Self::OwnershipUncertain { .. })
    }
}

/// Failure to obtain a complete frame generation.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum CaptureError {
    /// No complete frame arrived within the bounded SDK wait.
    #[error("camera capture timed out")]
    Timeout,
    /// The caller requested interruption.
    #[error("camera capture interrupted")]
    Interrupted,
    /// Capture failed with a non-timeout SDK result.
    #[error("ASI SDK capture failed with code {code}")]
    Sdk {
        /// Original SDK result code.
        code: i32,
    },
    /// Reading the cumulative SDK drop counter failed.
    #[error("ASI SDK dropped-frame query failed with code {code}")]
    DroppedFramesSdk {
        /// Original SDK result code.
        code: i32,
    },
    /// The SDK returned a negative or regressed cumulative drop count.
    #[error("ASI SDK returned invalid dropped-frame count {count}")]
    InvalidDropCount {
        /// Invalid vendor value.
        count: i32,
    },
    /// The source disconnected while capture was active.
    #[error("camera disconnected")]
    Disconnected,
    /// A completed source frame did not have the required native dimensions.
    #[error("camera frame dimensions were {width}x{height}, expected 1920x1080")]
    MalformedDimensions {
        /// Reported frame width.
        width: usize,
        /// Reported frame height.
        height: usize,
    },
    /// A completed source frame did not contain one full RAW8 byte per pixel.
    #[error("camera frame length was {length}, expected 2073600")]
    MalformedLength {
        /// Reported byte length.
        length: usize,
    },
    /// A completed source frame did not advance its generation monotonically.
    #[error("camera frame generation was {received}, expected greater than {previous}")]
    MalformedGeneration {
        /// Last trustworthy source generation.
        previous: u64,
        /// Invalid source generation.
        received: u64,
    },
    /// Capture was requested outside the running state.
    #[error("camera is not capturing")]
    NotCapturing,
}

/// Thread-safe request to interrupt the owner's bounded capture wait.
#[derive(Clone, Debug)]
pub struct CaptureInterrupter(Arc<AtomicBool>);

impl CaptureInterrupter {
    /// Requests interruption without calling the SDK from another thread.
    pub fn interrupt(&self) {
        self.0.store(true, Ordering::Release);
    }
}

/// Capture lifecycle shared by the production owner and explicit substitutes.
pub trait CameraSource {
    /// Returns a thread-safe interruption request handle.
    fn interrupter(&self) -> CaptureInterrupter;

    /// Applies one validated complete settings tuple while capture is stopped.
    ///
    /// # Errors
    ///
    /// Returns [`CameraError`] when the lifecycle state rejects configuration.
    fn configure(&mut self, settings: Settings) -> Result<(), CameraError>;

    /// Applies one validated complete settings tuple while acquisition remains active.
    ///
    /// The SDK controls change while acquisition remains warm. The camera may
    /// complete an already-integrating exposure and may emit a bounded number
    /// of visually transitional frames before the new tuple is trustworthy;
    /// callers must not infer exact settings identity for them.
    ///
    /// # Errors
    ///
    /// Returns [`CameraError`] when the source is not capturing or the live
    /// control mutation fails.
    fn apply_live_settings(&mut self, settings: Settings) -> Result<(), CameraError>;

    /// Starts continuously warm acquisition.
    ///
    /// # Errors
    ///
    /// Returns [`CameraError`] when the source is unconfigured or already running.
    fn start(&mut self) -> Result<(), CameraError>;

    /// Performs one bounded wait for the newest complete generation.
    ///
    /// # Errors
    ///
    /// Returns [`CaptureError`] for lifecycle, interruption, timeout, or source failures.
    fn capture_next(&mut self, wait_ms: i32) -> Result<FrameGeneration<'_>, CaptureError>;

    /// Stops acquisition. Calling stop when already stopped is harmless.
    ///
    /// # Errors
    ///
    /// Returns [`CameraError`] when the source cannot stop.
    fn stop(&mut self) -> Result<(), CameraError>;

    /// Consumes the exclusive source owner after stopping it.
    ///
    /// Production implementations must report any failure to close the owned
    /// backend handle. Substitutes without an external handle may use this
    /// default stop-and-drop implementation.
    ///
    /// # Errors
    ///
    /// Returns [`CameraError`] when acquisition or backend ownership cannot be
    /// relinquished conclusively.
    fn close(mut self) -> Result<(), CameraError>
    where
        Self: Sized,
    {
        self.stop()
    }
}

/// One validated full-frame RAW8 generation borrowed from the fixed pool.
pub struct FrameGeneration<'a> {
    generation: u64,
    data: &'a [u8],
    sdk_drops: u32,
}

impl FrameGeneration<'_> {
    /// Monotonically increasing source generation.
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }
    /// Exact 1920×1080 RAW8 RGGB bytes.
    #[must_use]
    pub const fn data(&self) -> &[u8] {
        self.data
    }
    /// New SDK-reported drops since the previous successful generation.
    #[must_use]
    pub const fn sdk_drops(&self) -> u32 {
        self.sdk_drops
    }
    /// Validated native frame width.
    #[must_use]
    pub const fn width(&self) -> usize {
        WIDTH
    }
    /// Validated native frame height.
    #[must_use]
    pub const fn height(&self) -> usize {
        HEIGHT
    }
    /// Validated top-left-origin Bayer mosaic.
    #[must_use]
    pub const fn bayer_layout(&self) -> BayerLayout {
        BayerLayout::Rggb
    }
}

/// Bayer mosaic layout of every production RAW8 generation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BayerLayout {
    /// Red, green / green, blue top-left-origin mosaic.
    Rggb,
}

/// Single-threaded, exclusive owner of the configured ASI662MC SDK handle.
pub struct CameraOwner {
    id: i32,
    buffers: [Box<[u8]>; BUFFER_COUNT],
    next_buffer: usize,
    generation: u64,
    last_drops: u32,
    configured: bool,
    capturing: bool,
    closed: bool,
    interrupted: Arc<AtomicBool>,
    _single_threaded: PhantomData<Rc<()>>,
}

impl CameraOwner {
    /// Enumerates properties, opens only one exact model, initializes it, and validates its serial.
    ///
    /// # Errors
    ///
    /// Returns [`CameraError`] for identity, sensor, serial, or SDK failures.
    pub fn connect() -> Result<Self, CameraError> {
        let count = ffi::camera_count();
        if count < 0 {
            return Err(CameraError::Sdk {
                operation: "enumerate cameras",
                code: count,
            });
        }
        let mut target = None;
        let mut matches = 0;
        for index in 0..count {
            let (code, info) = ffi::camera_property(index);
            check("read camera property", code)?;
            if info.name().as_deref() == Some(MODEL) {
                matches += 1;
                target = Some(info);
            }
        }
        if matches != 1 {
            return Err(CameraError::IdentityCount { found: matches });
        }
        let info = target.ok_or(CameraError::IdentityCount { found: 0 })?;
        info.validate()?;
        let buffers = std::array::from_fn(|_| vec![0; FRAME_BYTES].into_boxed_slice());
        check("open camera", ffi::open(info.camera_id))?;
        let id = info.camera_id;
        if let Err(error) = Self::initialize_and_validate(id) {
            let close_code = ffi::close(id);
            if close_code != 0 {
                return Err(CameraError::OwnershipUncertain { close_code });
            }
            return Err(error);
        }
        Ok(Self {
            id,
            buffers,
            next_buffer: 0,
            generation: 0,
            last_drops: 0,
            configured: false,
            capturing: false,
            closed: false,
            interrupted: Arc::new(AtomicBool::new(false)),
            _single_threaded: PhantomData,
        })
    }

    fn initialize_and_validate(id: i32) -> Result<(), CameraError> {
        check("initialize camera", ffi::initialize(id))?;
        let (code, serial) = ffi::serial(id);
        check("read camera serial", code)?;
        if serial != FACTORY_SERIAL {
            return Err(CameraError::SerialMismatch);
        }
        Ok(())
    }

    /// Returns a thread-safe interruption request handle.
    #[must_use]
    pub fn interrupter(&self) -> CaptureInterrupter {
        CaptureInterrupter(Arc::clone(&self.interrupted))
    }

    /// Applies a validated full-frame RAW8 settings tuple.
    ///
    /// # Errors
    ///
    /// Returns [`CameraError`] when capture is active or an SDK call fails.
    pub fn configure(&mut self, settings: Settings) -> Result<(), CameraError> {
        if self.capturing {
            return Err(CameraError::InvalidState {
                operation: "configure while capturing",
            });
        }
        check("set full-frame RAW8", ffi::set_raw8(self.id))?;
        check(
            "set exposure",
            ffi::set_control(self.id, CONTROL_EXPOSURE, settings.exposure_us),
        )?;
        check(
            "set gain",
            ffi::set_control(self.id, CONTROL_GAIN, settings.gain),
        )?;
        self.configured = true;
        Ok(())
    }

    /// Applies exposure and gain without stopping continuously warm acquisition.
    ///
    /// # Errors
    ///
    /// Returns [`CameraError`] when capture is stopped or either SDK control
    /// mutation fails. A partial SDK mutation is recovered by the pipeline's
    /// existing fail-closed camera recovery path.
    pub fn apply_live_settings(&mut self, settings: Settings) -> Result<(), CameraError> {
        if !self.capturing {
            return Err(CameraError::InvalidState {
                operation: "apply live settings while stopped",
            });
        }
        check(
            "set live exposure",
            ffi::set_control(self.id, CONTROL_EXPOSURE, settings.exposure_us),
        )?;
        check(
            "set live gain",
            ffi::set_control(self.id, CONTROL_GAIN, settings.gain),
        )
    }

    /// Starts continuously warm SDK video acquisition.
    ///
    /// # Errors
    ///
    /// Returns [`CameraError`] when unconfigured, already started, or rejected by the SDK.
    pub fn start(&mut self) -> Result<(), CameraError> {
        if !self.configured || self.capturing {
            return Err(CameraError::InvalidState { operation: "start" });
        }
        check("start video capture", ffi::start(self.id))?;
        self.capturing = true;
        let (code, drops) = ffi::dropped_frames(self.id);
        if let Err(error) = check("read dropped frames", code) {
            let stop_code = ffi::stop(self.id);
            if stop_code == 0 {
                self.capturing = false;
                return Err(error);
            }
            return Err(CameraError::StartRollback {
                operation: "read dropped frames",
                original_code: code,
                stop_code,
            });
        }
        self.last_drops = if let Ok(drops) = u32::try_from(drops) {
            drops
        } else {
            let stop_code = ffi::stop(self.id);
            if stop_code == 0 {
                self.capturing = false;
                return Err(CameraError::InvalidDropCount { count: drops });
            }
            return Err(CameraError::StartRollback {
                operation: "validate dropped frames",
                original_code: drops,
                stop_code,
            });
        };
        self.interrupted.store(false, Ordering::Release);
        Ok(())
    }

    /// Performs one bounded wait for the newest complete RAW8 frame.
    ///
    /// # Errors
    ///
    /// Returns [`CaptureError`] for lifecycle, interruption, timeout, or SDK failures.
    pub fn capture_next(&mut self, wait_ms: i32) -> Result<FrameGeneration<'_>, CaptureError> {
        if !self.capturing {
            return Err(CaptureError::NotCapturing);
        }
        if self.interrupted.swap(false, Ordering::AcqRel) {
            return Err(CaptureError::Interrupted);
        }
        let buffer = &mut self.buffers[self.next_buffer];
        let code = ffi::capture(self.id, buffer, wait_ms.clamp(0, MAX_CAPTURE_WAIT_MS));
        if self.interrupted.swap(false, Ordering::AcqRel) {
            return Err(CaptureError::Interrupted);
        }
        if code == ASI_TIMEOUT {
            return Err(CaptureError::Timeout);
        }
        if code != 0 {
            return Err(CaptureError::Sdk { code });
        }
        let (drop_code, drops) = ffi::dropped_frames(self.id);
        if drop_code != 0 {
            return Err(CaptureError::DroppedFramesSdk { code: drop_code });
        }
        let drops =
            u32::try_from(drops).map_err(|_| CaptureError::InvalidDropCount { count: drops })?;
        if drops < self.last_drops {
            return Err(CaptureError::InvalidDropCount {
                count: i32::try_from(drops).unwrap_or(i32::MAX),
            });
        }
        let sdk_drops = drops - self.last_drops;
        self.last_drops = drops;
        self.generation = self.generation.saturating_add(1);
        self.next_buffer = (self.next_buffer + 1) % BUFFER_COUNT;
        Ok(FrameGeneration {
            generation: self.generation,
            data: buffer,
            sdk_drops,
        })
    }

    /// Stops video acquisition. Calling stop when already stopped is harmless.
    ///
    /// # Errors
    ///
    /// Returns [`CameraError`] when the SDK cannot stop acquisition.
    pub fn stop(&mut self) -> Result<(), CameraError> {
        if !self.capturing {
            return Ok(());
        }
        check("stop video capture", ffi::stop(self.id))?;
        self.capturing = false;
        Ok(())
    }

    /// Stops and closes the SDK handle, reporting normalized shutdown failures.
    ///
    /// # Errors
    ///
    /// Returns [`CameraError`] when stop or close fails.
    pub fn close(mut self) -> Result<(), CameraError> {
        let stop = self.stop();
        self.closed = true;
        let close = check("close camera", ffi::close(self.id));
        stop.and(close)
    }
}

impl CameraSource for CameraOwner {
    fn interrupter(&self) -> CaptureInterrupter {
        Self::interrupter(self)
    }

    fn configure(&mut self, settings: Settings) -> Result<(), CameraError> {
        Self::configure(self, settings)
    }

    fn apply_live_settings(&mut self, settings: Settings) -> Result<(), CameraError> {
        Self::apply_live_settings(self, settings)
    }

    fn start(&mut self) -> Result<(), CameraError> {
        Self::start(self)
    }

    fn capture_next(&mut self, wait_ms: i32) -> Result<FrameGeneration<'_>, CaptureError> {
        Self::capture_next(self, wait_ms)
    }

    fn stop(&mut self) -> Result<(), CameraError> {
        Self::stop(self)
    }

    fn close(self) -> Result<(), CameraError> {
        Self::close(self)
    }
}

impl Drop for CameraOwner {
    fn drop(&mut self) {
        if self.closed {
            return;
        }
        if self.capturing {
            let _ = ffi::stop(self.id);
        }
        let _ = ffi::close(self.id);
    }
}

fn check(operation: &'static str, code: i32) -> Result<(), CameraError> {
    if code == 0 {
        Ok(())
    } else {
        Err(CameraError::Sdk { operation, code })
    }
}

mod ffi {
    //! The only unsafe boundary in the crate.
    //!
    //! Every pointer passed to the SDK refers to a live, aligned Rust value for
    //! the full call. Capture receives the exact fixed full-frame slice length.
    //! `CameraOwner` is deliberately `!Send`, so open handles and every SDK call
    //! remain on one thread. The owner opens only its enumerated ASI662MC ID,
    //! closes it at most once, and never lends the handle or buffer ownership to C.
    #![allow(unsafe_code)]
    use std::ffi::{c_char, c_int, c_long, c_uchar};

    #[repr(C)]
    pub(super) struct CameraInfo {
        name: [c_char; 64],
        pub camera_id: c_int,
        max_height: c_long,
        max_width: c_long,
        is_color_camera: c_int,
        bayer_pattern: c_int,
        supported_bins: [c_int; 16],
        supported_video_formats: [c_int; 8],
        pixel_size: f64,
        mechanical_shutter: c_int,
        st4_port: c_int,
        is_cooler_camera: c_int,
        is_usb3_host: c_int,
        is_usb3_camera: c_int,
        electrons_per_adu: f32,
        bit_depth: c_int,
        is_trigger_camera: c_int,
        unused: [c_char; 16],
    }
    impl CameraInfo {
        pub(super) fn name(&self) -> Option<String> {
            let bytes = self.name.map(|byte| byte.to_ne_bytes()[0]);
            let length = bytes.iter().position(|&byte| byte == 0)?;
            std::str::from_utf8(&bytes[..length])
                .ok()
                .map(str::to_owned)
        }
        pub(super) fn validate(&self) -> Result<(), super::CameraError> {
            if self.max_width != 1920 || self.max_height != 1080 {
                return Err(super::CameraError::InvalidDimensions {
                    width: self.max_width,
                    height: self.max_height,
                });
            }
            if self.is_color_camera != 1 || self.bayer_pattern != 0 {
                return Err(super::CameraError::UnsupportedBayer);
            }
            let raw8 = self
                .supported_video_formats
                .iter()
                .take_while(|&&format| format != -1)
                .any(|&format| format == 0);
            if !raw8 {
                return Err(super::CameraError::UnsupportedFormat);
            }
            Ok(())
        }
    }
    impl Default for CameraInfo {
        fn default() -> Self {
            // SAFETY: every C scalar field admits zero before the SDK initializes the output.
            unsafe { std::mem::zeroed() }
        }
    }
    #[repr(C)]
    struct SerialNumber {
        bytes: [c_uchar; 8],
    }
    unsafe extern "C" {
        fn ASIGetNumOfConnectedCameras() -> c_int;
        fn ASIGetCameraProperty(info: *mut CameraInfo, index: c_int) -> c_int;
        fn ASIOpenCamera(id: c_int) -> c_int;
        fn ASIInitCamera(id: c_int) -> c_int;
        fn ASICloseCamera(id: c_int) -> c_int;
        fn ASIGetSerialNumber(id: c_int, serial: *mut SerialNumber) -> c_int;
        fn ASISetROIFormat(
            id: c_int,
            width: c_int,
            height: c_int,
            bin: c_int,
            image_type: c_int,
        ) -> c_int;
        fn ASISetControlValue(id: c_int, control: c_int, value: c_long, automatic: c_int) -> c_int;
        fn ASIStartVideoCapture(id: c_int) -> c_int;
        fn ASIStopVideoCapture(id: c_int) -> c_int;
        fn ASIGetVideoData(id: c_int, buffer: *mut c_uchar, size: c_long, wait_ms: c_int) -> c_int;
        fn ASIGetDroppedFrames(id: c_int, count: *mut c_int) -> c_int;
    }
    pub(super) fn camera_count() -> c_int {
        unsafe { ASIGetNumOfConnectedCameras() }
    }
    pub(super) fn camera_property(index: c_int) -> (c_int, CameraInfo) {
        let mut value = CameraInfo::default();
        let code = unsafe { ASIGetCameraProperty(&raw mut value, index) };
        (code, value)
    }
    pub(super) fn open(id: c_int) -> c_int {
        unsafe { ASIOpenCamera(id) }
    }
    pub(super) fn initialize(id: c_int) -> c_int {
        unsafe { ASIInitCamera(id) }
    }
    pub(super) fn close(id: c_int) -> c_int {
        unsafe { ASICloseCamera(id) }
    }
    pub(super) fn serial(id: c_int) -> (c_int, [u8; 8]) {
        let mut value = SerialNumber { bytes: [0; 8] };
        let code = unsafe { ASIGetSerialNumber(id, &raw mut value) };
        (code, value.bytes)
    }
    pub(super) fn set_raw8(id: c_int) -> c_int {
        unsafe { ASISetROIFormat(id, 1920, 1080, 1, 0) }
    }
    pub(super) fn set_control(id: c_int, control: c_int, value: i64) -> c_int {
        unsafe { ASISetControlValue(id, control, value, 0) }
    }
    pub(super) fn start(id: c_int) -> c_int {
        unsafe { ASIStartVideoCapture(id) }
    }
    pub(super) fn stop(id: c_int) -> c_int {
        unsafe { ASIStopVideoCapture(id) }
    }
    pub(super) fn capture(id: c_int, buffer: &mut [u8], wait_ms: c_int) -> c_int {
        unsafe {
            ASIGetVideoData(
                id,
                buffer.as_mut_ptr(),
                c_long::try_from(buffer.len()).expect("frame fits C long"),
                wait_ms,
            )
        }
    }
    pub(super) fn dropped_frames(id: c_int) -> (c_int, c_int) {
        let mut count = 0;
        let code = unsafe { ASIGetDroppedFrames(id, &raw mut count) };
        (code, count)
    }

    #[cfg(all(zwo_sdk_stub, test))]
    pub(super) mod stub {
        use std::ffi::{CString, c_char, c_int, c_long, c_uchar};
        unsafe extern "C" {
            fn ObsCamStubReset();
            fn ObsCamStubAddCamera(name: *const c_char, id: c_int);
            fn ObsCamStubSetSerial(id: c_int, serial: *const c_uchar);
            fn ObsCamStubSetShape(
                id: c_int,
                width: c_long,
                height: c_long,
                color: c_int,
                bayer: c_int,
                raw8: c_int,
            );
            fn ObsCamStubSetCaptureResult(result: c_int);
            fn ObsCamStubSetDroppedFrames(count: c_int);
            fn ObsCamStubSetResult(operation: c_int, result: c_int);
            fn ObsCamStubCalls(operation: c_int, id: c_int) -> c_int;
            fn ObsCamStubLastWaitMs() -> c_int;
        }
        pub(crate) fn reset() {
            unsafe { ObsCamStubReset() }
        }
        pub(crate) fn add(name: &str, id: i32) {
            let name = CString::new(name).unwrap();
            unsafe { ObsCamStubAddCamera(name.as_ptr(), id) }
        }
        pub(crate) fn serial(id: i32, serial: [u8; 8]) {
            unsafe { ObsCamStubSetSerial(id, serial.as_ptr()) }
        }
        pub(crate) fn shape(id: i32, width: i64, height: i64, color: i32, bayer: i32, raw8: bool) {
            unsafe { ObsCamStubSetShape(id, width, height, color, bayer, i32::from(raw8)) }
        }
        pub(crate) fn capture_result(code: i32) {
            unsafe { ObsCamStubSetCaptureResult(code) }
        }
        pub(crate) fn drops(count: i32) {
            unsafe { ObsCamStubSetDroppedFrames(count) }
        }
        pub(crate) fn calls(operation: i32, id: i32) -> i32 {
            unsafe { ObsCamStubCalls(operation, id) }
        }
        pub(crate) fn result(operation: i32, code: i32) {
            unsafe { ObsCamStubSetResult(operation, code) }
        }
        pub(crate) fn last_wait_ms() -> i32 {
            unsafe { ObsCamStubLastWaitMs() }
        }
    }
}

#[cfg(all(test, zwo_sdk_stub))]
mod tests {
    use super::{
        BUFFER_COUNT, CameraError, CameraOwner, CaptureError, FRAME_BYTES, Settings, SettingsError,
        ffi::stub,
    };
    use std::sync::Mutex;
    static SDK: Mutex<()> = Mutex::new(());
    fn target() {
        stub::add("ZWO ASI178MC", 1);
        stub::add("ZWO ASI662MC", 2);
    }

    #[test]
    fn identity_failures_never_open_an_unmatched_camera() {
        let _guard = SDK.lock().unwrap();
        stub::reset();
        stub::add("ZWO ASI178MC", 1);
        assert_eq!(
            CameraOwner::connect().err(),
            Some(CameraError::IdentityCount { found: 0 })
        );
        assert_eq!(stub::calls(0, 1), 0);
        stub::add("ZWO ASI662MC", 2);
        stub::add("ZWO ASI662MC", 3);
        assert_eq!(
            CameraOwner::connect().err(),
            Some(CameraError::IdentityCount { found: 2 })
        );
        assert_eq!(stub::calls(0, 1), 0);
        assert_eq!(stub::calls(0, 2), 0);
    }

    #[test]
    fn serial_and_sensor_validation_fail_closed() {
        let _guard = SDK.lock().unwrap();
        stub::reset();
        target();
        stub::shape(2, 1280, 720, 1, 0, true);
        assert_eq!(
            CameraOwner::connect().err(),
            Some(CameraError::InvalidDimensions {
                width: 1280,
                height: 720,
            })
        );
        assert_eq!(stub::calls(0, 2), 0);
        stub::reset();
        target();
        stub::serial(2, [0; 8]);
        assert_eq!(
            CameraOwner::connect().err(),
            Some(CameraError::SerialMismatch)
        );
        assert_eq!(stub::calls(0, 1), 0);
        assert_eq!(stub::calls(0, 2), 1);
        assert_eq!(stub::calls(1, 2), 1);
        assert_eq!(stub::calls(2, 2), 1);

        stub::reset();
        target();
        stub::result(1, 16);
        stub::result(2, 5);
        assert_eq!(
            CameraOwner::connect().err(),
            Some(CameraError::OwnershipUncertain { close_code: 5 })
        );

        stub::reset();
        target();
        stub::shape(2, 1920, 1080, 0, 0, true);
        assert_eq!(
            CameraOwner::connect().err(),
            Some(CameraError::UnsupportedBayer)
        );
        assert_eq!(stub::calls(0, 2), 0);
        stub::reset();
        target();
        stub::shape(2, 1920, 1080, 1, 0, false);
        assert_eq!(
            CameraOwner::connect().err(),
            Some(CameraError::UnsupportedFormat)
        );
        assert_eq!(stub::calls(0, 2), 0);
    }

    #[test]
    fn lifecycle_failures_are_normalized_and_handles_close_once() {
        let _guard = SDK.lock().unwrap();
        stub::reset();
        target();
        stub::result(1, 16);
        assert_eq!(
            CameraOwner::connect().err(),
            Some(CameraError::Sdk {
                operation: "initialize camera",
                code: 16
            })
        );
        assert_eq!(stub::calls(2, 2), 1);

        stub::reset();
        target();
        stub::result(0, 5);
        assert_eq!(
            CameraOwner::connect().err(),
            Some(CameraError::Sdk {
                operation: "open camera",
                code: 5
            })
        );
        assert_eq!(stub::calls(2, 2), 0);

        stub::reset();
        target();
        let mut owner = CameraOwner::connect().unwrap();
        stub::result(9, 8);
        assert_eq!(
            owner.configure(Settings::new(50_000, 0).unwrap()),
            Err(CameraError::Sdk {
                operation: "set full-frame RAW8",
                code: 8
            })
        );

        stub::reset();
        target();
        let mut owner = CameraOwner::connect().unwrap();
        owner.configure(Settings::new(50_000, 0).unwrap()).unwrap();
        stub::result(4, 16);
        assert_eq!(
            owner.start(),
            Err(CameraError::Sdk {
                operation: "start video capture",
                code: 16
            })
        );

        stub::reset();
        target();
        let mut owner = CameraOwner::connect().unwrap();
        owner.configure(Settings::new(50_000, 0).unwrap()).unwrap();
        owner.start().unwrap();
        stub::result(5, 5);
        assert_eq!(
            owner.stop(),
            Err(CameraError::Sdk {
                operation: "stop video capture",
                code: 5
            })
        );

        stub::reset();
        target();
        let owner = CameraOwner::connect().unwrap();
        stub::result(2, 5);
        assert_eq!(
            owner.close(),
            Err(CameraError::Sdk {
                operation: "close camera",
                code: 5
            })
        );
        assert_eq!(stub::calls(2, 2), 1);
    }

    #[test]
    fn failed_drop_baseline_rolls_back_started_capture() {
        let _guard = SDK.lock().unwrap();
        stub::reset();
        target();
        let mut owner = CameraOwner::connect().unwrap();
        owner.configure(Settings::new(50_000, 0).unwrap()).unwrap();
        stub::result(10, 16);
        assert_eq!(
            owner.start(),
            Err(CameraError::Sdk {
                operation: "read dropped frames",
                code: 16
            })
        );
        assert_eq!(stub::calls(4, 2), 1);
        assert_eq!(stub::calls(5, 2), 1);
        stub::result(10, 0);
        owner.start().unwrap();

        stub::reset();
        target();
        let mut owner = CameraOwner::connect().unwrap();
        owner.configure(Settings::new(50_000, 0).unwrap()).unwrap();
        stub::result(10, 16);
        stub::result(5, 5);
        assert_eq!(
            owner.start(),
            Err(CameraError::StartRollback {
                operation: "read dropped frames",
                original_code: 16,
                stop_code: 5
            })
        );
        assert_eq!(stub::calls(5, 2), 1);
    }

    #[test]
    fn settings_enforce_the_complete_operating_envelope() {
        assert_eq!(Settings::new(20_000, 0), Err(SettingsError::Exposure));
        assert!(Settings::new(50_000, 0).is_ok());
        assert!(Settings::new(30_000_000, 600).is_ok());
        assert_eq!(Settings::new(49_999, 0), Err(SettingsError::Exposure));
        assert_eq!(Settings::new(50_000, 601), Err(SettingsError::Gain));
    }

    #[test]
    fn capture_rotates_four_buffers_and_reports_generations_and_drops() {
        let _guard = SDK.lock().unwrap();
        stub::reset();
        target();
        let mut owner = CameraOwner::connect().unwrap();
        owner
            .configure(Settings::new(50_000, 600).unwrap())
            .unwrap();
        owner.start().unwrap();
        let mut pointers = Vec::new();
        for generation in 1..=BUFFER_COUNT + 1 {
            if generation == 2 {
                stub::drops(3);
            }
            let frame = owner.capture_next(20).unwrap();
            assert_eq!(frame.generation(), generation as u64);
            assert_eq!(frame.data().len(), FRAME_BYTES);
            assert_eq!((frame.width(), frame.height()), (1920, 1080));
            assert_eq!(frame.bayer_layout(), super::BayerLayout::Rggb);
            assert_eq!(frame.data()[0], u8::try_from(generation).unwrap());
            pointers.push(frame.data().as_ptr());
            assert_eq!(frame.sdk_drops(), if generation == 2 { 3 } else { 0 });
        }
        assert_eq!(pointers[0], pointers[BUFFER_COUNT]);
        owner.stop().unwrap();
        owner.close().unwrap();
        assert_eq!(stub::calls(4, 2), 1);
        assert_eq!(stub::calls(5, 2), 1);
        assert_eq!(stub::calls(2, 2), 1);
    }

    #[test]
    fn timeout_capture_error_and_interruption_are_distinct() {
        let _guard = SDK.lock().unwrap();
        stub::reset();
        target();
        let mut owner = CameraOwner::connect().unwrap();
        owner
            .configure(Settings::new(500_000, 100).unwrap())
            .unwrap();
        owner.start().unwrap();
        stub::capture_result(11);
        assert_eq!(
            owner.capture_next(30_000).err(),
            Some(CaptureError::Timeout)
        );
        assert_eq!(stub::last_wait_ms(), 100);
        stub::capture_result(5);
        assert_eq!(
            owner.capture_next(20).err(),
            Some(CaptureError::Sdk { code: 5 })
        );
        owner.interrupter().interrupt();
        assert_eq!(
            owner.capture_next(20).err(),
            Some(CaptureError::Interrupted)
        );
        stub::capture_result(0);
        stub::result(10, 16);
        assert_eq!(
            owner.capture_next(20).err(),
            Some(CaptureError::DroppedFramesSdk { code: 16 })
        );
        stub::result(10, 0);
        stub::drops(-1);
        assert_eq!(
            owner.capture_next(20).err(),
            Some(CaptureError::InvalidDropCount { count: -1 })
        );
    }
}
