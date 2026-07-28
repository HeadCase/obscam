#![allow(unsafe_code)]

//! Narrow, fail-closed ownership wrapper around the vendor ASI camera SDK.

use std::ffi::{CStr, c_char, c_int, c_long, c_uchar};
use std::fmt::Write;

use thiserror::Error;

const ASI_SUCCESS: c_int = 0;
const ASI_ERROR_TIMEOUT: c_int = 11;
const ASI_IMG_RAW8: c_int = 0;
const ASI_GAIN: c_int = 0;
const ASI_EXPOSURE: c_int = 1;
const ASI_BANDWIDTH_OVERLOAD: c_int = 6;
const ASI_HIGH_SPEED_MODE: c_int = 14;

#[repr(C)]
struct AsiCameraInfo {
    name: [c_char; 64],
    camera_id: c_int,
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

impl Default for AsiCameraInfo {
    fn default() -> Self {
        // The vendor API requires a zero-initialized output structure.
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
struct AsiSerialNumber {
    bytes: [c_uchar; 8],
}

unsafe extern "C" {
    fn ASIGetNumOfConnectedCameras() -> c_int;
    fn ASIGetCameraProperty(info: *mut AsiCameraInfo, index: c_int) -> c_int;
    fn ASIOpenCamera(camera_id: c_int) -> c_int;
    fn ASIInitCamera(camera_id: c_int) -> c_int;
    fn ASICloseCamera(camera_id: c_int) -> c_int;
    fn ASIGetSerialNumber(camera_id: c_int, serial: *mut AsiSerialNumber) -> c_int;
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

#[derive(Debug, Error)]
pub enum CameraError {
    #[error("expected one {expected} camera, found {found}")]
    IdentityCount { expected: String, found: usize },
    #[error("camera {model} serial mismatch: expected {expected}, found {found}")]
    SerialMismatch {
        model: String,
        expected: String,
        found: String,
    },
    #[error("ASI SDK operation {operation} failed with code {code}")]
    Sdk {
        operation: &'static str,
        code: c_int,
    },
    #[error("camera dimensions are invalid: {width}x{height}")]
    InvalidDimensions { width: c_long, height: c_long },
    #[error("camera capture timed out")]
    Timeout,
}

#[derive(Debug)]
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
        let count = unsafe { ASIGetNumOfConnectedCameras() };
        if count < 0 {
            return Err(CameraError::Sdk {
                operation: "enumerate cameras",
                code: count,
            });
        }
        let mut matches = Vec::new();
        for index in 0..count {
            let mut info = AsiCameraInfo::default();
            check("read camera property", unsafe {
                ASIGetCameraProperty(&raw mut info, index)
            })?;
            let name = unsafe { CStr::from_ptr(info.name.as_ptr()) }
                .to_string_lossy()
                .into_owned();
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
        check("open camera", unsafe { ASIOpenCamera(info.camera_id) })?;
        let mut camera = Self {
            id: info.camera_id,
            width: usize::try_from(info.max_width).map_err(|_| CameraError::InvalidDimensions {
                width: info.max_width,
                height: info.max_height,
            })?,
            height: usize::try_from(info.max_height).map_err(|_| {
                CameraError::InvalidDimensions {
                    width: info.max_width,
                    height: info.max_height,
                }
            })?,
            capturing: false,
        };
        if let Err(error) = camera.initialize(model, expected_serial) {
            drop(camera);
            return Err(error);
        }
        Ok(camera)
    }

    fn initialize(&mut self, model: &str, expected_serial: &str) -> Result<(), CameraError> {
        check("initialize camera", unsafe { ASIInitCamera(self.id) })?;
        let mut serial = AsiSerialNumber { bytes: [0; 8] };
        check("read camera serial", unsafe {
            ASIGetSerialNumber(self.id, &raw mut serial)
        })?;
        let mut found = String::with_capacity(16);
        for byte in serial.bytes {
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
        check("set full-frame RAW8 format", unsafe {
            ASISetROIFormat(
                self.id,
                c_int::try_from(self.width).unwrap_or(c_int::MAX),
                c_int::try_from(self.height).unwrap_or(c_int::MAX),
                1,
                ASI_IMG_RAW8,
            )
        })?;
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
        check("start video capture", unsafe {
            ASIStartVideoCapture(self.id)
        })?;
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
        check("stop video capture", unsafe {
            ASIStopVideoCapture(self.id)
        })?;
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
        let code = unsafe {
            ASIGetVideoData(
                self.id,
                buffer.as_mut_ptr(),
                c_long::try_from(buffer.len()).unwrap_or(c_long::MAX),
                wait_ms,
            )
        };
        if code == ASI_ERROR_TIMEOUT {
            return Err(CameraError::Timeout);
        }
        check("capture video frame", code)
    }

    #[must_use]
    pub const fn dimensions(&self) -> (usize, usize) {
        (self.width, self.height)
    }
}

impl Drop for Camera {
    fn drop(&mut self) {
        if self.capturing {
            unsafe { ASIStopVideoCapture(self.id) };
        }
        unsafe { ASICloseCamera(self.id) };
    }
}

fn set_control(
    camera_id: c_int,
    control: c_int,
    value: i64,
    operation: &'static str,
) -> Result<(), CameraError> {
    check(operation, unsafe {
        ASISetControlValue(camera_id, control, value, 0)
    })
}

fn check(operation: &'static str, code: c_int) -> Result<(), CameraError> {
    if code == ASI_SUCCESS {
        Ok(())
    } else {
        Err(CameraError::Sdk { operation, code })
    }
}
