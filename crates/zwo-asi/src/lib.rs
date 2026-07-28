//! Narrow, fail-closed ownership wrapper around the vendor ASI camera SDK.

#![warn(missing_docs)]

use std::ffi::{c_int, c_long};
use std::fmt::Write;

use thiserror::Error;

const ASI_ERROR_TIMEOUT: c_int = 11;
const ASI_GAIN: c_int = 0;
const ASI_EXPOSURE: c_int = 1;
const ASI_BANDWIDTH_OVERLOAD: c_int = 6;
const ASI_HIGH_SPEED_MODE: c_int = 14;

mod ffi {
    #![allow(unsafe_code)]

    use std::ffi::{CStr, c_char, c_int, c_long, c_uchar};

    const ASI_IMG_RAW8: c_int = 0;

    #[repr(C)]
    pub(super) struct CameraInfo {
        name: [c_char; 64],
        pub camera_id: c_int,
        pub max_height: c_long,
        pub max_width: c_long,
        pub is_color_camera: c_int,
        pub bayer_pattern: c_int,
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
        pub(super) fn name(&self) -> String {
            // SAFETY: the SDK promises a NUL-terminated name inside the fixed
            // output buffer after ASIGetCameraProperty succeeds.
            unsafe { CStr::from_ptr(self.name.as_ptr()) }
                .to_string_lossy()
                .into_owned()
        }
    }

    impl Default for CameraInfo {
        fn default() -> Self {
            Self {
                name: [0; 64],
                camera_id: 0,
                max_height: 0,
                max_width: 0,
                is_color_camera: 0,
                bayer_pattern: 0,
                supported_bins: [0; 16],
                supported_video_formats: [0; 8],
                pixel_size: 0.0,
                mechanical_shutter: 0,
                st4_port: 0,
                is_cooler_camera: 0,
                is_usb3_host: 0,
                is_usb3_camera: 0,
                electrons_per_adu: 0.0,
                bit_depth: 0,
                is_trigger_camera: 0,
                unused: [0; 16],
            }
        }
    }

    #[repr(C)]
    struct SerialNumber {
        bytes: [c_uchar; 8],
    }

    unsafe extern "C" {
        fn ASIGetNumOfConnectedCameras() -> c_int;
        fn ASIGetCameraProperty(info: *mut CameraInfo, index: c_int) -> c_int;
        fn ASIOpenCamera(camera_id: c_int) -> c_int;
        fn ASIInitCamera(camera_id: c_int) -> c_int;
        fn ASICloseCamera(camera_id: c_int) -> c_int;
        fn ASIGetSerialNumber(camera_id: c_int, serial: *mut SerialNumber) -> c_int;
        fn ASISetROIFormat(
            camera_id: c_int,
            width: c_int,
            height: c_int,
            bin: c_int,
            image_type: c_int,
        ) -> c_int;
        fn ASISetControlValue(
            camera_id: c_int,
            control: c_int,
            value: c_long,
            automatic: c_int,
        ) -> c_int;
        fn ASIStartVideoCapture(camera_id: c_int) -> c_int;
        fn ASIStopVideoCapture(camera_id: c_int) -> c_int;
        fn ASIGetVideoData(
            camera_id: c_int,
            buffer: *mut c_uchar,
            buffer_size: c_long,
            wait_ms: c_int,
        ) -> c_int;
    }

    pub(super) fn camera_count() -> c_int {
        // SAFETY: this SDK query accepts no pointers and has no preconditions.
        unsafe { ASIGetNumOfConnectedCameras() }
    }

    pub(super) fn camera_property(index: c_int) -> (c_int, CameraInfo) {
        let mut info = CameraInfo::default();
        // SAFETY: info is a valid, writable, correctly sized output structure
        // and remains alive for the duration of the call.
        let code = unsafe { ASIGetCameraProperty(&raw mut info, index) };
        (code, info)
    }

    pub(super) fn open(camera_id: c_int) -> c_int {
        // SAFETY: camera_id came from a successful SDK enumeration result.
        unsafe { ASIOpenCamera(camera_id) }
    }

    pub(super) fn initialize(camera_id: c_int) -> c_int {
        // SAFETY: the caller invokes this only for an opened camera handle.
        unsafe { ASIInitCamera(camera_id) }
    }

    pub(super) fn close(camera_id: c_int) -> c_int {
        // SAFETY: Camera owns this handle and calls close at most once.
        unsafe { ASICloseCamera(camera_id) }
    }

    pub(super) fn serial(camera_id: c_int) -> (c_int, [u8; 8]) {
        let mut serial = SerialNumber { bytes: [0; 8] };
        // SAFETY: serial is a valid fixed-size writable output buffer.
        let code = unsafe { ASIGetSerialNumber(camera_id, &raw mut serial) };
        (code, serial.bytes)
    }

    pub(super) fn set_raw8_roi(camera_id: c_int, width: c_int, height: c_int) -> c_int {
        // SAFETY: dimensions are validated positive sensor dimensions; bin=1
        // and RAW8 are vendor-defined supported values for the selected model.
        unsafe { ASISetROIFormat(camera_id, width, height, 1, ASI_IMG_RAW8) }
    }

    pub(super) fn set_control(camera_id: c_int, control: c_int, value: c_long) -> c_int {
        // SAFETY: the wrapper range-checks values and uses vendor control IDs.
        unsafe { ASISetControlValue(camera_id, control, value, 0) }
    }

    pub(super) fn start(camera_id: c_int) -> c_int {
        // SAFETY: the initialized camera is exclusively owned by Camera.
        unsafe { ASIStartVideoCapture(camera_id) }
    }

    pub(super) fn stop(camera_id: c_int) -> c_int {
        // SAFETY: the camera handle is valid and exclusively owned.
        unsafe { ASIStopVideoCapture(camera_id) }
    }

    pub(super) fn capture(camera_id: c_int, buffer: &mut [u8], wait_ms: c_int) -> c_int {
        // SAFETY: the slice provides a live writable pointer and its exact byte
        // length; Camera verifies that length equals the configured RAW8 frame.
        unsafe {
            ASIGetVideoData(
                camera_id,
                buffer.as_mut_ptr(),
                c_long::try_from(buffer.len()).unwrap_or(c_long::MAX),
                wait_ms,
            )
        }
    }
}

#[derive(Debug, Error)]
/// Fail-closed errors returned by the normalized camera adapter.
pub enum CameraError {
    /// The exact requested model was absent or ambiguous.
    #[error("expected one {expected} camera, found {found}")]
    IdentityCount {
        /// Requested SDK model name.
        expected: String,
        /// Number of matching connected cameras.
        found: usize,
    },
    /// The selected camera's factory serial did not match configuration.
    #[error("camera {model} serial mismatch: expected {expected}, found {found}")]
    SerialMismatch {
        /// Validated SDK model name.
        model: String,
        /// Configured factory serial.
        expected: String,
        /// Factory serial returned by the SDK.
        found: String,
    },
    /// A vendor SDK operation returned a non-success code.
    #[error("ASI SDK operation {operation} failed with code {code}")]
    Sdk {
        /// Normalized operation name.
        operation: &'static str,
        /// Unmodified vendor result code.
        code: c_int,
    },
    /// The SDK dimensions cannot represent a full-frame buffer safely.
    #[error("camera dimensions are invalid: {width}x{height}")]
    InvalidDimensions {
        /// Reported or supplied width.
        width: c_long,
        /// Reported or supplied height.
        height: c_long,
    },
    /// The camera is not the expected colour sensor with RGGB layout.
    #[error("camera has unsupported colour layout: colour={is_colour}, Bayer code={bayer_pattern}")]
    UnsupportedColourLayout {
        /// Vendor colour-camera flag.
        is_colour: c_int,
        /// Vendor Bayer-pattern enumeration value.
        bayer_pattern: c_int,
    },
    /// No complete video frame arrived within the requested wait.
    #[error("camera capture timed out")]
    Timeout,
}

#[derive(Debug)]
/// Exclusive owner of one initialized, full-frame RAW8 ASI camera.
pub struct Camera {
    id: c_int,
    width: usize,
    height: usize,
    capturing: bool,
}

impl Camera {
    /// Opens only one exact model and validates its factory serial number.
    ///
    /// # Errors
    ///
    /// Returns an error when identity is missing or ambiguous, serial validation
    /// fails, dimensions are invalid, or the vendor SDK rejects an operation.
    pub fn open_exact(model: &str, expected_serial: &str) -> Result<Self, CameraError> {
        let count = ffi::camera_count();
        if count < 0 {
            return Err(CameraError::Sdk {
                operation: "enumerate cameras",
                code: count,
            });
        }
        let mut matches = Vec::new();
        for index in 0..count {
            let (code, info) = ffi::camera_property(index);
            check("read camera property", code)?;
            let name = info.name();
            if name == model {
                matches.push(info);
            }
        }
        if matches.len() != 1 {
            return Err(CameraError::IdentityCount {
                expected: model.to_owned(),
                found: matches.len(),
            });
        }
        let info = matches
            .into_iter()
            .next()
            .ok_or_else(|| CameraError::IdentityCount {
                expected: model.to_owned(),
                found: 0,
            })?;
        // ZWO's SDK defines Bayer code 0 as top-left-origin RGGB. The
        // production treatment module relies on this exact, uninverted layout.
        if info.is_color_camera != 1 || info.bayer_pattern != 0 {
            return Err(CameraError::UnsupportedColourLayout {
                is_colour: info.is_color_camera,
                bayer_pattern: info.bayer_pattern,
            });
        }
        let width =
            usize::try_from(info.max_width).map_err(|_| CameraError::InvalidDimensions {
                width: info.max_width,
                height: info.max_height,
            })?;
        let height =
            usize::try_from(info.max_height).map_err(|_| CameraError::InvalidDimensions {
                width: info.max_width,
                height: info.max_height,
            })?;
        if width == 0 || height == 0 {
            return Err(CameraError::InvalidDimensions {
                width: info.max_width,
                height: info.max_height,
            });
        }
        check("open camera", ffi::open(info.camera_id))?;
        let mut camera = Self {
            id: info.camera_id,
            width,
            height,
            capturing: false,
        };
        if let Err(error) = camera.initialize(model, expected_serial) {
            drop(camera);
            return Err(error);
        }
        Ok(camera)
    }

    fn initialize(&mut self, model: &str, expected_serial: &str) -> Result<(), CameraError> {
        check("initialize camera", ffi::initialize(self.id))?;
        let (code, serial) = ffi::serial(self.id);
        check("read camera serial", code)?;
        let mut found = String::with_capacity(16);
        for byte in serial {
            let _ = write!(found, "{byte:02x}");
        }
        if found != expected_serial {
            return Err(CameraError::SerialMismatch {
                model: model.to_owned(),
                expected: expected_serial.to_owned(),
                found,
            });
        }
        if self.width == 0 || self.height == 0 {
            return Err(CameraError::InvalidDimensions {
                width: c_long::try_from(self.width).unwrap_or_default(),
                height: c_long::try_from(self.height).unwrap_or_default(),
            });
        }
        check(
            "set full-frame RAW8 format",
            ffi::set_raw8_roi(
                self.id,
                c_int::try_from(self.width).unwrap_or(c_int::MAX),
                c_int::try_from(self.height).unwrap_or(c_int::MAX),
            ),
        )?;
        Ok(())
    }

    /// Applies the complete numeric camera tuple used by the probe.
    ///
    /// # Errors
    ///
    /// Returns an error when the vendor SDK rejects any control value.
    pub fn configure(&self, exposure_us: i64, gain: i64) -> Result<(), CameraError> {
        set_control(self.id, ASI_EXPOSURE, exposure_us, "set exposure")?;
        set_control(self.id, ASI_GAIN, gain, "set gain")?;
        set_control(self.id, ASI_BANDWIDTH_OVERLOAD, 100, "set USB bandwidth")?;
        set_control(self.id, ASI_HIGH_SPEED_MODE, 1, "enable high-speed mode")?;
        Ok(())
    }

    /// Starts continuous SDK video acquisition.
    ///
    /// # Errors
    ///
    /// Returns an error when the vendor SDK cannot start capture.
    pub fn start(&mut self) -> Result<(), CameraError> {
        check("start video capture", ffi::start(self.id))?;
        self.capturing = true;
        Ok(())
    }

    /// Stops continuous SDK video acquisition.
    ///
    /// # Errors
    ///
    /// Returns an error when the vendor SDK cannot stop capture.
    pub fn stop(&mut self) -> Result<(), CameraError> {
        if !self.capturing {
            return Ok(());
        }
        check("stop video capture", ffi::stop(self.id))?;
        self.capturing = false;
        Ok(())
    }

    /// Waits for one full-frame RAW8 video buffer.
    ///
    /// # Errors
    ///
    /// Returns an error for a wrong-sized buffer, timeout, camera removal, or
    /// another vendor SDK capture failure.
    pub fn capture(&self, buffer: &mut [u8], wait_ms: i32) -> Result<(), CameraError> {
        let required = self.width * self.height;
        if buffer.len() != required {
            return Err(CameraError::InvalidDimensions {
                width: c_long::try_from(self.width).unwrap_or_default(),
                height: c_long::try_from(self.height).unwrap_or_default(),
            });
        }
        let code = ffi::capture(self.id, buffer, wait_ms);
        if code == ASI_ERROR_TIMEOUT {
            return Err(CameraError::Timeout);
        }
        check("capture video frame", code)
    }

    #[must_use]
    /// Returns the validated full-frame sensor dimensions.
    pub const fn dimensions(&self) -> (usize, usize) {
        (self.width, self.height)
    }
}

impl Drop for Camera {
    fn drop(&mut self) {
        if self.capturing {
            ffi::stop(self.id);
        }
        ffi::close(self.id);
    }
}

fn set_control(
    camera_id: c_int,
    control: c_int,
    value: i64,
    operation: &'static str,
) -> Result<(), CameraError> {
    check(operation, ffi::set_control(camera_id, control, value))
}

fn check(operation: &'static str, code: c_int) -> Result<(), CameraError> {
    if code == 0 {
        Ok(())
    } else {
        Err(CameraError::Sdk { operation, code })
    }
}
