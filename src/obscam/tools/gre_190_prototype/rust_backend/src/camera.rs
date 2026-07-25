//! Exact-identity ZWO camera ownership boundary proven by GRE-191.

use std::ffi::CStr;
use std::os::raw::{c_char, c_double, c_float, c_int, c_long, c_uchar};
use std::time::{SystemTime, UNIX_EPOCH};

const ASI_SUCCESS: c_int = 0;
const ASI_IMG_RAW8: c_int = 0;
const ASI_IMG_END: c_int = -1;
const ASI_GAIN: c_int = 0;
const ASI_EXPOSURE: c_int = 1;
const ASI_BANDWIDTH_OVERLOAD: c_int = 6;
const ASI_HIGH_SPEED_MODE: c_int = 14;
pub const FULL_WIDTH: usize = 1920;
pub const FULL_HEIGHT: usize = 1080;
pub const RAW8_FRAME_BYTES: usize = FULL_WIDTH * FULL_HEIGHT;

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
        // The SDK defines this structure as a caller-owned output buffer.
        unsafe { std::mem::zeroed() }
    }
}

#[repr(C)]
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
        // The SDK defines this structure as a caller-owned output buffer.
        unsafe { std::mem::zeroed() }
    }
}

#[repr(C)]
struct AsiSerial {
    bytes: [u8; 8],
}

#[link(name = "ASICamera2")]
extern "C" {
    fn ASIGetNumOfConnectedCameras() -> c_int;
    fn ASIGetCameraProperty(info: *mut AsiCameraInfo, camera_index: c_int) -> c_int;
    fn ASIOpenCamera(camera_id: c_int) -> c_int;
    fn ASIInitCamera(camera_id: c_int) -> c_int;
    fn ASIGetSerialNumber(camera_id: c_int, serial: *mut AsiSerial) -> c_int;
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
    fn ASICloseCamera(camera_id: c_int) -> c_int;
}

pub struct CameraOwner {
    camera_id: c_int,
    info: AsiCameraInfo,
    pub model: String,
    pub serial_hex: String,
}

impl CameraOwner {
    pub fn open_exact(required_model: &str, required_serial_hex: &str) -> Result<Self, String> {
        let required_serial = parse_serial(required_serial_hex)?;
        let camera_count = unsafe { ASIGetNumOfConnectedCameras() };
        let mut matches = Vec::new();
        for camera_index in 0..camera_count {
            let mut info = AsiCameraInfo::default();
            checked(
                unsafe { ASIGetCameraProperty(&mut info, camera_index) },
                "ASIGetCameraProperty",
            )?;
            let model = unsafe { CStr::from_ptr(info.name.as_ptr()) }
                .to_str()
                .map_err(|error| format!("camera model is not UTF-8: {error}"))?;
            if model == required_model {
                matches.push((info, model.to_owned()));
            }
        }
        if matches.len() != 1 {
            return Err(format!(
                "required model {required_model} matched {} cameras; refusing to open",
                matches.len()
            ));
        }
        let (info, model) = matches.remove(0);
        let camera_id = info.camera_id;
        checked(unsafe { ASIOpenCamera(camera_id) }, "ASIOpenCamera")?;
        let result = Self::initialize(camera_id, info, model, required_serial);
        if result.is_err() {
            unsafe {
                ASICloseCamera(camera_id);
            }
        }
        result
    }

    fn initialize(
        camera_id: c_int,
        info: AsiCameraInfo,
        model: String,
        required_serial: [u8; 8],
    ) -> Result<Self, String> {
        checked(unsafe { ASIInitCamera(camera_id) }, "ASIInitCamera")?;
        let mut serial = AsiSerial { bytes: [0; 8] };
        checked(
            unsafe { ASIGetSerialNumber(camera_id, &mut serial) },
            "ASIGetSerialNumber",
        )?;
        if serial.bytes != required_serial {
            return Err(format!(
                "factory serial mismatch: expected {}, observed {}",
                format_serial(required_serial),
                format_serial(serial.bytes)
            ));
        }
        Ok(Self {
            camera_id,
            info,
            model,
            serial_hex: format_serial(serial.bytes),
        })
    }

    pub fn camera_id(&self) -> c_int {
        self.camera_id
    }

    pub fn ffmpeg_bayer_pixel_format(&self) -> Result<&'static str, String> {
        match self.info.bayer_pattern {
            0 => Ok("bayer_rggb8"),
            1 => Ok("bayer_bggr8"),
            2 => Ok("bayer_grbg8"),
            3 => Ok("bayer_gbrg8"),
            pattern => Err(format!("unsupported Bayer pattern {pattern}")),
        }
    }

    pub fn start_raw8_video(
        &self,
        exposure_us: c_long,
        gain: c_long,
        high_speed: c_long,
        bandwidth: c_long,
    ) -> Result<VideoCapture<'_>, String> {
        if self.info.max_width != FULL_WIDTH as c_long
            || self.info.max_height != FULL_HEIGHT as c_long
            || !self
                .info
                .supported_video_format
                .iter()
                .take_while(|&&format| format != ASI_IMG_END)
                .any(|&format| format == ASI_IMG_RAW8)
        {
            return Err("camera does not support full-resolution RAW8".to_owned());
        }
        unsafe {
            ASIStopVideoCapture(self.camera_id);
            ASIStopExposure(self.camera_id);
        }
        set_control(self.camera_id, ASI_EXPOSURE, exposure_us)?;
        set_control(self.camera_id, ASI_GAIN, gain)?;
        set_control(self.camera_id, ASI_BANDWIDTH_OVERLOAD, bandwidth)?;
        if writable_control(self.camera_id, ASI_HIGH_SPEED_MODE) {
            set_control(self.camera_id, ASI_HIGH_SPEED_MODE, high_speed)?;
        }
        checked(
            unsafe {
                ASISetROIFormat(
                    self.camera_id,
                    FULL_WIDTH as c_int,
                    FULL_HEIGHT as c_int,
                    1,
                    ASI_IMG_RAW8,
                )
            },
            "ASISetROIFormat",
        )?;
        checked(
            unsafe { ASISetStartPos(self.camera_id, 0, 0) },
            "ASISetStartPos",
        )?;
        checked(
            unsafe { ASIStartVideoCapture(self.camera_id) },
            "ASIStartVideoCapture",
        )?;
        Ok(VideoCapture {
            owner: self,
            active: true,
        })
    }
}

pub struct VideoCapture<'a> {
    owner: &'a CameraOwner,
    active: bool,
}

impl VideoCapture<'_> {
    pub fn capture_into(&mut self, frame: &mut [u8], timeout_ms: c_int) -> Result<u64, String> {
        if frame.len() != RAW8_FRAME_BYTES {
            return Err(format!(
                "RAW8 destination has {} bytes, expected {RAW8_FRAME_BYTES}",
                frame.len()
            ));
        }
        checked(
            unsafe {
                ASIGetVideoData(
                    self.owner.camera_id,
                    frame.as_mut_ptr(),
                    frame.len() as c_long,
                    timeout_ms,
                )
            },
            "ASIGetVideoData",
        )?;
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "system clock is before Unix epoch".to_owned())?;
        Ok(timestamp.as_nanos() as u64)
    }
}

impl Drop for VideoCapture<'_> {
    fn drop(&mut self) {
        if self.active {
            unsafe {
                ASIStopVideoCapture(self.owner.camera_id);
            }
            self.active = false;
        }
    }
}

impl Drop for CameraOwner {
    fn drop(&mut self) {
        unsafe {
            ASICloseCamera(self.camera_id);
        }
    }
}

fn checked(code: c_int, operation: &str) -> Result<(), String> {
    if code == ASI_SUCCESS {
        Ok(())
    } else {
        Err(format!("{operation} failed with ASI error {code}"))
    }
}

fn set_control(camera_id: c_int, control: c_int, value: c_long) -> Result<(), String> {
    checked(
        unsafe { ASISetControlValue(camera_id, control, value, 0) },
        "ASISetControlValue",
    )
}

fn writable_control(camera_id: c_int, control: c_int) -> bool {
    let mut count = 0;
    if unsafe { ASIGetNumOfControls(camera_id, &mut count) } != ASI_SUCCESS {
        return false;
    }
    (0..count).any(|index| {
        let mut caps = AsiControlCaps::default();
        (unsafe { ASIGetControlCaps(camera_id, index, &mut caps) }) == ASI_SUCCESS
            && caps.control_type == control
            && caps.is_writable != 0
    })
}

fn parse_serial(value: &str) -> Result<[u8; 8], String> {
    if value.len() != 16 {
        return Err("factory serial must contain exactly 16 hex digits".to_owned());
    }
    let mut bytes = [0; 8];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..(index * 2) + 2], 16)
            .map_err(|_| "factory serial must contain exactly 16 hex digits".to_owned())?;
    }
    Ok(bytes)
}

fn format_serial(serial: [u8; 8]) -> String {
    serial.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::{format_serial, parse_serial};

    #[test]
    fn serial_round_trip() {
        let serial = "1d274e0920010900";
        assert_eq!(format_serial(parse_serial(serial).unwrap()), serial);
    }

    #[test]
    fn serial_rejects_wrong_length_and_non_hex() {
        assert!(parse_serial("1234").is_err());
        assert!(parse_serial("1d274e092001090z").is_err());
    }
}
