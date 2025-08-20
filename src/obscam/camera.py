import os
import sys
import time
import io
import base64
import threading
from pathlib import Path
from typing import Optional, Dict, Any, Literal
from enum import Enum
import zwoasi as asi
import numpy as np
from PIL import Image
import cv2

from .storage import StorageManager


class ProgramMode(Enum):
    """Program modes for observatory operations."""
    AUTO = "auto"        # Adaptive exposure with low gain, handles lighting changes
    SLEWING = "slewing"  # Max 500ms exposure for real-time mount monitoring  
    MANUAL = "manual"    # User-controlled exposure and gain settings


class SceneType(Enum):
    """Scene types for adaptive white balance."""
    DAYLIGHT = "daylight"
    ARTIFICIAL = "artificial" 
    NIGHT = "night"
    AUTO = "auto"


class ASI662MCCamera:
    """Camera interface for ZWO ASI662MC using the ZWO ASI SDK."""
    
    def __init__(self, library_path: Optional[str] = None):
        """Initialize the camera.
        
        Args:
            library_path: Path to the ZWO ASI SDK library. If None, tries environment variable.
        """
        self.camera = None
        self.camera_info = None
        self.is_initialized = False
        self.image_dir = Path("images")
        self.image_dir.mkdir(exist_ok=True)
        
        # Performance optimization: reusable arrays
        self._img_buffer_8bit = None
        self._img_buffer_16bit = None
        
        # Scene detection state
        self._last_scene_type = SceneType.AUTO
        self._frame_brightness_history = []
        
        # Program mode state management
        self.current_program_mode = ProgramMode.AUTO
        self.preferred_gain = 250  # Low gain for both AUTO and SLEWING
        self.manual_exposure_us = 100000  # 100ms default for manual
        self.manual_gain = 250
        
        # Adaptive exposure state for AUTO mode
        self._current_exposure_us = 100000  # Start with 100ms
        self._last_brightness = None
        self._brightness_history = []
        self._exposure_direction = 0  # -1: decreasing, 0: stable, 1: increasing
        
        # Initialize the SDK
        self._init_sdk(library_path)
        
    def _init_sdk(self, library_path: Optional[str] = None) -> None:
        """Initialize the ZWO ASI SDK."""
        env_filename = os.getenv('ZWO_ASI_LIB')
        
        try:
            if library_path:
                asi.init(library_path)
            elif env_filename:
                asi.init(env_filename)
            else:
                # Try common library paths for Raspberry Pi
                common_paths = [
                    '/usr/local/lib/libASICamera2.so',
                    '/usr/lib/libASICamera2.so',
                    './libASICamera2.so'
                ]
                for path in common_paths:
                    if os.path.exists(path):
                        asi.init(path)
                        break
                else:
                    raise RuntimeError("ZWO ASI SDK library not found. Set ZWO_ASI_LIB environment variable or provide library_path")
                    
        except Exception as e:
            raise RuntimeError(f"Failed to initialize ZWO ASI SDK: {e}")
    
    def connect(self) -> bool:
        """Connect to the ASI662MC camera."""
        try:
            num_cameras = asi.get_num_cameras()
            if num_cameras == 0:
                raise RuntimeError("No cameras found")
            
            cameras_found = asi.list_cameras()
            print(f"Found {num_cameras} camera(s): {cameras_found}")
            
            # Look for ASI662MC specifically, otherwise use first camera
            camera_id = 0
            for i, camera_name in enumerate(cameras_found):
                if "ASI662MC" in camera_name:
                    camera_id = i
                    break
            
            self.camera = asi.Camera(camera_id)
            self.camera_info = self.camera.get_camera_property()
            
            print(f"Connected to camera: {cameras_found[camera_id]}")
            print(f"Camera info: {self.camera_info}")
            
            # Configure camera for optimal settings
            self._configure_camera()
            
            self.is_initialized = True
            return True
            
        except Exception as e:
            print(f"Failed to connect to camera: {e}")
            return False
    
    def _configure_camera(self) -> None:
        """Configure camera with sensible defaults for ASI662MC."""
        try:
            # Stop any ongoing operations
            try:
                self.camera.stop_video_capture()
                self.camera.stop_exposure()
            except:
                pass
            
            # Get available controls
            controls = self.camera.get_controls()
            
            # Use minimum USB bandwidth to reduce issues
            if 'BandWidth' in controls:
                min_bandwidth = controls['BandWidth']['MinValue']
                self.camera.set_control_value(asi.ASI_BANDWIDTHOVERLOAD, min_bandwidth)
            
            # Disable dark subtract
            self.camera.disable_dark_subtract()
            
            # Set sensible defaults for ASI662MC
            # These values work well for general purpose imaging
            self.camera.set_control_value(asi.ASI_GAIN, 150)         # Higher gain for better sensitivity
            self.camera.set_control_value(asi.ASI_EXPOSURE, 100000)  # 100ms exposure (much brighter)
            
            # Enable auto-exposure if available
            if 'AutoExpMaxExpMS' in controls:
                self.camera.set_control_value(asi.ASI_AUTO_MAX_EXP, 700)   # Max 700ms auto-exposure
                self.camera.set_control_value(asi.ASI_AUTO_MAX_GAIN, 600)  # Max gain for auto-exposure
            
            # Color camera specific settings - aggressive magenta cast reduction
            if self.camera_info.get('IsColorCam', False):
                self.camera.set_control_value(asi.ASI_WB_B, 110)   # Blue white balance (increased to counter magenta)
                self.camera.set_control_value(asi.ASI_WB_R, 70)    # Red white balance (decreased to reduce magenta)
                
            self.camera.set_control_value(asi.ASI_GAMMA, 50)        # Standard gamma
            self.camera.set_control_value(asi.ASI_BRIGHTNESS, 50)   # Standard brightness
            self.camera.set_control_value(asi.ASI_FLIP, 0)          # No flip
            
            print("Camera configured with default settings")
            
        except Exception as e:
            print(f"Warning: Could not configure all camera settings: {e}")
    
    def _calculate_brightness(self, img_array: np.ndarray) -> float:
        """Calculate overall image brightness (0-255 scale)."""
        try:
            if len(img_array.shape) == 3:  # Color image
                # Convert to grayscale for brightness calculation
                brightness = np.mean(img_array)
            else:  # Grayscale image
                brightness = np.mean(img_array)
            return float(brightness)
        except Exception as e:
            print(f"Brightness calculation failed: {e}")
            return 128.0  # Middle brightness fallback
    
    def _update_adaptive_exposure(self, brightness: float) -> int:
        """Update exposure time based on image brightness for AUTO mode."""
        if self.current_program_mode != ProgramMode.AUTO:
            return self._current_exposure_us
            
        try:
            # Target brightness range (out of 255)
            TARGET_MIN = 80   # Slightly dark images are OK
            TARGET_MAX = 160  # Avoid overexposure
            TARGET_CENTER = (TARGET_MIN + TARGET_MAX) / 2
            
            # Update brightness history
            self._brightness_history.append(brightness)
            if len(self._brightness_history) > 5:
                self._brightness_history.pop(0)
            
            # Calculate average brightness for stability
            avg_brightness = np.mean(self._brightness_history) if self._brightness_history else brightness
            
            # Exposure limits (microseconds)
            MIN_EXPOSURE = 1000    # 1ms minimum
            MAX_EXPOSURE = 10000000  # 10s maximum
            
            # Calculate adjustment factor
            if avg_brightness < TARGET_MIN:
                # Too dark - increase exposure
                if avg_brightness < 20:  # Very dark - big jump
                    adjustment = 3.0
                elif avg_brightness < 40:  # Dark - medium jump  
                    adjustment = 2.0
                else:  # Slightly dark - small jump
                    adjustment = 1.5
            elif avg_brightness > TARGET_MAX:
                # Too bright - decrease exposure
                if avg_brightness > 220:  # Very bright - big drop
                    adjustment = 0.2
                elif avg_brightness > 180:  # Bright - medium drop
                    adjustment = 0.5
                else:  # Slightly bright - small drop
                    adjustment = 0.75
            else:
                # In target range - minor adjustments toward center
                if abs(avg_brightness - TARGET_CENTER) < 10:
                    adjustment = 1.0  # No change needed
                else:
                    adjustment = 1.1 if avg_brightness < TARGET_CENTER else 0.9
            
            # Apply adjustment
            new_exposure = int(self._current_exposure_us * adjustment)
            new_exposure = max(MIN_EXPOSURE, min(MAX_EXPOSURE, new_exposure))
            
            # Detect dramatic lighting changes (roof opening/closing)
            if self._last_brightness is not None:
                brightness_change = abs(brightness - self._last_brightness)
                if brightness_change > 100:  # Dramatic change
                    print(f"Dramatic lighting change detected: {self._last_brightness:.1f} → {brightness:.1f}")
                    # Fast response to dramatic changes
                    if brightness > self._last_brightness + 100:  # Much brighter
                        new_exposure = max(MIN_EXPOSURE, self._current_exposure_us // 5)
                    elif brightness < self._last_brightness - 100:  # Much darker
                        new_exposure = min(MAX_EXPOSURE, self._current_exposure_us * 5)
            
            self._current_exposure_us = new_exposure
            self._last_brightness = brightness
            
            print(f"AUTO mode: brightness={brightness:.1f}, exposure={new_exposure/1000:.1f}ms")
            return new_exposure
            
        except Exception as e:
            print(f"Adaptive exposure update failed: {e}")
            return self._current_exposure_us

    def _detect_scene_type(self, img_array: np.ndarray) -> SceneType:
        """Detect scene type based on image brightness and characteristics."""
        try:
            # Calculate brightness metrics
            mean_brightness = np.mean(img_array)
            brightness_std = np.std(img_array)
            
            # Update history for temporal stability
            self._frame_brightness_history.append(mean_brightness)
            if len(self._frame_brightness_history) > 5:
                self._frame_brightness_history.pop(0)
            
            avg_brightness = np.mean(self._frame_brightness_history)
            
            # Classify scene based on brightness and variation
            if avg_brightness > 120:  # Bright daylight
                return SceneType.DAYLIGHT
            elif avg_brightness > 40 and brightness_std > 30:  # Artificial lighting
                return SceneType.ARTIFICIAL  
            else:  # Low light/night
                return SceneType.NIGHT
                
        except Exception as e:
            print(f"Scene detection failed: {e}")
            return SceneType.AUTO
    
    def set_program_mode(self, mode: ProgramMode, exposure_us: Optional[int] = None, gain: Optional[int] = None) -> None:
        """Set the current program mode and immediately configure camera hardware."""
        if not self.is_initialized or not self.camera:
            print("Warning: Cannot set program mode - camera not initialized")
            return
            
        self.current_program_mode = mode
        
        try:
            # Update settings based on mode and immediately apply to camera hardware
            if mode == ProgramMode.MANUAL:
                if exposure_us is not None:
                    self.manual_exposure_us = exposure_us
                if gain is not None:
                    self.manual_gain = gain
                # Immediately configure camera hardware for MANUAL mode
                self.camera.set_control_value(asi.ASI_GAIN, self.manual_gain)
                self.camera.set_control_value(asi.ASI_EXPOSURE, self.manual_exposure_us)
                print(f"MANUAL mode active: {self.manual_exposure_us/1000:.1f}ms exposure, gain {self.manual_gain}")
                
            elif mode == ProgramMode.AUTO:
                # Reset adaptive exposure to a middle ground
                self._current_exposure_us = 100000  # 100ms starting point
                self._brightness_history.clear()
                # Immediately configure camera hardware for AUTO mode
                self.camera.set_control_value(asi.ASI_GAIN, self.preferred_gain)
                self.camera.set_control_value(asi.ASI_EXPOSURE, self._current_exposure_us)
                print(f"AUTO mode active: adaptive exposure starting at {self._current_exposure_us/1000:.1f}ms, gain {self.preferred_gain}")
                
            elif mode == ProgramMode.SLEWING:
                # Immediately configure camera hardware for SLEWING mode
                MAX_SLEWING_EXPOSURE_US = 500000  # 500ms maximum
                self.camera.set_control_value(asi.ASI_GAIN, self.preferred_gain)
                self.camera.set_control_value(asi.ASI_EXPOSURE, MAX_SLEWING_EXPOSURE_US)
                print(f"SLEWING mode active: fixed 500ms exposure, gain {self.preferred_gain}")
                
            print(f"Program mode switched to: {mode.value} - camera hardware updated immediately")
            
        except Exception as e:
            print(f"Error configuring camera hardware for {mode.value} mode: {e}")
            # Still update the mode state even if hardware config fails
            print("Mode state updated, but camera settings may need manual configuration")
    
    def get_current_program_mode(self) -> ProgramMode:
        """Get the current program mode."""
        return self.current_program_mode
    
    def _flush_camera_buffer(self) -> None:
        """
        AllSky-inspired buffer flush to prevent stale frames.
        Temporarily sets very short exposure, drains buffer, then restores exposure.
        Critical for telescope CCTV tracking where fresh frames are essential.
        """
        FLUSH_EXPOSURE_US = 5000  # 5ms flush exposure (microseconds)
        FLUSH_FRAMES = 3  # Number of frames to drain
        
        try:
            if not self.camera:
                return
                
            # Save current exposure setting
            try:
                current_exposure = self.camera.get_control_value(asi.ASI_EXPOSURE)[0]
            except:
                current_exposure = 100000  # Default to 100ms if can't read
            
            # Strategy 1: Video mode flush (for active video capture)
            try:
                # Set very short exposure for fast frame drainage  
                self.camera.set_control_value(asi.ASI_EXPOSURE, FLUSH_EXPOSURE_US)
                
                # Ensure video mode is started
                try:
                    self.camera.start_video_capture()
                    video_mode_active = True
                except:
                    video_mode_active = False
                
                if video_mode_active:
                    # Drain buffer frames with short timeout
                    for i in range(FLUSH_FRAMES):
                        try:
                            _ = self.camera.get_video_data(timeout=100)  # 100ms timeout
                        except:
                            # Timeout is expected - means buffer is drained
                            break
                    
                    # CRITICAL FIX: Stop video mode after flushing
                    try:
                        self.camera.stop_video_capture()
                        # Small delay for mode transition stabilization
                        time.sleep(0.05)  # 50ms delay
                        print(f"Buffer flushed: {i+1} frames drained, video mode stopped")
                    except Exception as stop_error:
                        print(f"Warning: Failed to stop video mode after flush: {stop_error}")
                    
                    # Restore original exposure
                    self.camera.set_control_value(asi.ASI_EXPOSURE, current_exposure)
                else:
                    # Fallback: just restore exposure
                    self.camera.set_control_value(asi.ASI_EXPOSURE, current_exposure)
                    
            except Exception as flush_error:
                # Always restore exposure and ensure video mode is stopped
                try:
                    self.camera.stop_video_capture()
                    self.camera.set_control_value(asi.ASI_EXPOSURE, current_exposure)
                except:
                    pass
                print(f"Buffer flush error (cleanup attempted): {flush_error}")
                
        except Exception as e:
            print(f"Critical buffer flush failure: {e}")
    
    def _apply_adaptive_white_balance(self, scene_type: SceneType) -> None:
        """Apply white balance settings based on scene type."""
        try:
            if scene_type == SceneType.DAYLIGHT:
                # Daylight: reduce blue, balance red
                self.camera.set_control_value(asi.ASI_WB_R, 85)
                self.camera.set_control_value(asi.ASI_WB_B, 95)
            elif scene_type == SceneType.ARTIFICIAL:
                # Artificial: reduce red/magenta cast
                self.camera.set_control_value(asi.ASI_WB_R, 70)
                self.camera.set_control_value(asi.ASI_WB_B, 115) 
            elif scene_type == SceneType.NIGHT:
                # Night: preserve natural colors, slight blue boost
                self.camera.set_control_value(asi.ASI_WB_R, 80)
                self.camera.set_control_value(asi.ASI_WB_B, 105)
            
            self._last_scene_type = scene_type
            
        except Exception as e:
            print(f"Adaptive white balance failed: {e}")
    
    def capture_image(self, filename: Optional[str] = None) -> Optional[str]:
        """Capture image using current program mode.
        
        Args:
            filename: Optional filename to save image. If None, generates timestamp-based name.
            
        Returns:
            Path to saved image file, or None if capture failed.
        """
        if not self.is_initialized or not self.camera:
            print("Camera not initialized")
            return None
            
        try:
            start_time = time.time()
            
            # Stop any video capture
            try:
                self.camera.stop_video_capture() 
                self.camera.stop_exposure()
            except:
                pass
                
            # Flush buffer for fresh frame
            self._flush_camera_buffer()
            
            # CRITICAL FIX: Ensure we're in single-shot mode for all captures
            try:
                self.camera.stop_video_capture()
                self.camera.stop_exposure()
                # Additional stabilization delay after mode changes
                time.sleep(0.1)  # 100ms delay for mode stabilization
                print("Camera prepared for single-shot capture")
            except Exception as mode_error:
                print(f"Warning: Error preparing single-shot mode: {mode_error}")
            
            # Generate filename if not provided
            if filename is None:
                timestamp = int(time.time())
                filename = f"image_{timestamp}.jpg"
            
            filepath = self.image_dir / filename
            
            print(f"Capturing in {self.current_program_mode.value} mode...")
            
            # Route to appropriate capture method based on current program mode
            if self.current_program_mode == ProgramMode.AUTO:
                result = self._capture_auto_mode(filepath)
            elif self.current_program_mode == ProgramMode.SLEWING:
                result = self._capture_slewing_mode(filepath)
            else:  # MANUAL
                result = self._capture_manual_mode(filepath)
            
            elapsed = (time.time() - start_time) * 1000
            print(f"Capture completed in {elapsed:.1f}ms")
            
            return result
            
        except Exception as e:
            print(f"Failed to capture image: {e}")
            return None
    
    def _capture_auto_mode(self, filepath: Path) -> Optional[str]:
        """AUTO mode: Adaptive exposure with low gain, handles lighting changes."""
        try:
            # Camera hardware already configured by set_program_mode()
            # Just verify current exposure setting for adaptive algorithm
            current_exposure = self._current_exposure_us
            
            # Use RGB24 for good speed/quality balance
            if self.camera_info.get('IsColorCam', False):
                self.camera.set_image_type(asi.ASI_IMG_RGB24)
                width = self.camera_info['MaxWidth']
                height = self.camera_info['MaxHeight']
                
                # Capture image
                img_buffer = self.camera.capture()
                img_array = np.frombuffer(img_buffer, dtype=np.uint8).reshape((height, width, 3))
                
                # Calculate brightness and update adaptive exposure for NEXT capture
                brightness = self._calculate_brightness(img_array)
                next_exposure = self._update_adaptive_exposure(brightness)
                
                # Apply the new exposure immediately for next capture
                if next_exposure != current_exposure:
                    self.camera.set_control_value(asi.ASI_EXPOSURE, next_exposure)
                
                # Scene detection and white balance
                scene_type = self._detect_scene_type(img_array)
                if scene_type != self._last_scene_type:
                    self._apply_adaptive_white_balance(scene_type)
                
                # Apply appropriate correction based on scene
                img_corrected = self._apply_medium_correction(img_array, scene_type)
                
                # Save image
                pil_image = Image.fromarray(img_corrected)
                pil_image.save(filepath, 'JPEG', quality=90)
                
            else:
                # Monochrome fallback
                self.camera.set_image_type(asi.ASI_IMG_RAW8)
                self.camera.capture(filename=str(filepath))
            
            return str(filepath)
            
        except Exception as e:
            print(f"AUTO capture failed: {e}")
            return None
    
    def _capture_slewing_mode(self, filepath: Path) -> Optional[str]:
        """SLEWING mode: Max 500ms exposure for real-time mount monitoring."""
        try:
            # Camera hardware already configured by set_program_mode() for consistent 500ms/low gain
            
            # Use RGB24 for speed
            if self.camera_info.get('IsColorCam', False):
                self.camera.set_image_type(asi.ASI_IMG_RGB24)
                width = self.camera_info['MaxWidth']
                height = self.camera_info['MaxHeight']
                
                # Direct capture - minimal processing for speed
                img_buffer = self.camera.capture()
                img_array = np.frombuffer(img_buffer, dtype=np.uint8).reshape((height, width, 3))
                
                # Quick white balance only - no adaptive processing
                img_corrected = self._apply_fast_correction(img_array)
                
                # Save with good quality but prioritize speed
                pil_image = Image.fromarray(img_corrected)
                pil_image.save(filepath, 'JPEG', quality=85)
                
            else:
                # Monochrome fallback
                self.camera.set_image_type(asi.ASI_IMG_RAW8)
                self.camera.capture(filename=str(filepath))
            
            return str(filepath)
            
        except Exception as e:
            print(f"SLEWING capture failed: {e}")
            return None
    
    def _capture_manual_mode(self, filepath: Path) -> Optional[str]:
        """MANUAL mode: User-controlled exposure and gain settings."""
        try:
            # Use stored manual settings
            self.camera.set_control_value(asi.ASI_GAIN, self.manual_gain)
            self.camera.set_control_value(asi.ASI_EXPOSURE, self.manual_exposure_us)
            
            print(f"Manual capture: using pre-configured {self.manual_exposure_us/1000:.1f}ms exposure, gain {self.manual_gain}")
            
            # Use RGB24 for good balance
            if self.camera_info.get('IsColorCam', False):
                self.camera.set_image_type(asi.ASI_IMG_RGB24)
                width = self.camera_info['MaxWidth']
                height = self.camera_info['MaxHeight']
                
                # Direct capture with user settings
                img_buffer = self.camera.capture()
                img_array = np.frombuffer(img_buffer, dtype=np.uint8).reshape((height, width, 3))
                
                # Scene detection and appropriate correction
                scene_type = self._detect_scene_type(img_array)
                if scene_type != self._last_scene_type:
                    self._apply_adaptive_white_balance(scene_type)
                
                # Apply medium-level correction (good quality for manual use)
                img_corrected = self._apply_medium_correction(img_array, scene_type)
                
                # Save with high quality
                pil_image = Image.fromarray(img_corrected)
                pil_image.save(filepath, 'JPEG', quality=92)
                
            else:
                # Monochrome fallback
                self.camera.set_image_type(asi.ASI_IMG_RAW8)
                self.camera.capture(filename=str(filepath))
            
            return str(filepath)
            
        except Exception as e:
            print(f"MANUAL capture failed: {e}")
            return None
    
    def _apply_fast_correction(self, img_rgb: np.ndarray) -> np.ndarray:
        """Fast color correction: basic white balance only."""
        try:
            # Simple channel scaling for speed
            img_corrected = img_rgb.astype(np.float32)
            
            # Quick white balance based on scene type
            if self._last_scene_type == SceneType.DAYLIGHT:
                img_corrected[:, :, 0] *= 0.9  # Reduce red slightly
                img_corrected[:, :, 2] *= 1.05  # Increase blue slightly
            elif self._last_scene_type == SceneType.ARTIFICIAL:
                img_corrected[:, :, 0] *= 0.8  # Reduce red more
                img_corrected[:, :, 2] *= 1.15  # Increase blue more
            
            return np.clip(img_corrected, 0, 255).astype(np.uint8)
            
        except Exception as e:
            print(f"Fast correction failed: {e}")
            return img_rgb
    
    def _apply_medium_correction(self, img_rgb: np.ndarray, scene_type: SceneType) -> np.ndarray:
        """Medium color correction: balanced processing with scene adaptation."""
        try:
            img_float = img_rgb.astype(np.float32) / 255.0
            
            # Scene-adaptive color matrix
            if scene_type == SceneType.DAYLIGHT:
                color_matrix = np.array([
                    [1.1, -0.05, -0.05],
                    [-0.03, 1.05, -0.02], 
                    [-0.05, -0.1, 1.15]
                ])
            elif scene_type == SceneType.ARTIFICIAL:
                color_matrix = np.array([
                    [1.2, -0.1, -0.1],
                    [-0.05, 1.1, -0.05],
                    [-0.1, -0.2, 1.3]
                ])
            else:  # Night
                color_matrix = np.array([
                    [1.05, -0.02, -0.03],
                    [-0.02, 1.02, -0.02],
                    [-0.03, -0.05, 1.08]
                ])
            
            # Apply color matrix
            h, w, c = img_float.shape
            img_matrix = img_float.reshape(-1, 3) @ color_matrix.T
            img_matrix = img_matrix.reshape(h, w, c)
            
            # Simple gamma correction
            gamma = 0.9 if scene_type == SceneType.DAYLIGHT else 0.85
            img_gamma = np.power(np.clip(img_matrix, 0, 1), gamma)
            
            return (img_gamma * 255).astype(np.uint8)
            
        except Exception as e:
            print(f"Medium correction failed: {e}")
            return img_rgb
    
    def _apply_color_correction(self, img_rgb: np.ndarray) -> np.ndarray:
        """Apply advanced color correction to reduce magenta cast and improve color balance."""
        try:
            # Convert to float for processing
            img_float = img_rgb.astype(np.float32) / 255.0
            
            # Get current white balance settings (with improved defaults)
            status = self.get_camera_status()
            wb_r = status.get('current_settings', {}).get('WB_R', 70) / 100.0  # More aggressive red reduction
            wb_b = status.get('current_settings', {}).get('WB_B', 110) / 100.0  # Increase blue to counteract magenta
            
            # Advanced color matrix correction for daylight balance
            # This matrix helps convert from camera color space to sRGB with better color balance
            color_matrix = np.array([
                [1.2, -0.1, -0.1],  # Red: boost, reduce cross-talk
                [-0.05, 1.1, -0.05],  # Green: slight boost, reduce cross-talk  
                [-0.1, -0.2, 1.3]   # Blue: boost significantly to counter magenta
            ])
            
            # Apply color matrix
            h, w, c = img_float.shape
            img_matrix = img_float.reshape(-1, 3) @ color_matrix.T
            img_matrix = img_matrix.reshape(h, w, c)
            
            # Apply white balance correction
            img_matrix[:, :, 0] *= wb_r  # Red channel
            img_matrix[:, :, 2] *= wb_b  # Blue channel
            
            # Gamma correction (helps with contrast and color balance)
            gamma = 0.8  # Lower gamma brightens midtones
            img_gamma = np.power(np.clip(img_matrix, 0, 1), gamma)
            
            # Slight saturation boost
            # Convert to HSV to adjust saturation
            img_hsv = cv2.cvtColor((img_gamma * 255).astype(np.uint8), cv2.COLOR_RGB2HSV).astype(np.float32)
            img_hsv[:, :, 1] *= 1.1  # Increase saturation by 10%
            img_hsv[:, :, 1] = np.clip(img_hsv[:, :, 1], 0, 255)
            
            # Convert back to RGB
            img_corrected = cv2.cvtColor(img_hsv.astype(np.uint8), cv2.COLOR_HSV2RGB)
            
            return img_corrected
            
        except Exception as e:
            print(f"Advanced color correction failed, using basic correction: {e}")
            # Fallback to basic correction
            img_float = img_rgb.astype(np.float32)
            img_float[:, :, 0] *= 0.8  # Reduce red
            img_float[:, :, 2] *= 1.2  # Increase blue
            return np.clip(img_float, 0, 255).astype(np.uint8)
    
    def get_current_program_mode_status(self) -> dict:
        """Get the current program mode status and settings."""
        try:
            status = {
                "program_mode": self.current_program_mode.value,
                "preferred_gain": self.preferred_gain,
            }
            
            if self.current_program_mode == ProgramMode.AUTO:
                status["auto_exposure_us"] = self._current_exposure_us
                status["auto_exposure_ms"] = self._current_exposure_us / 1000
            elif self.current_program_mode == ProgramMode.SLEWING:
                status["slewing_exposure_ms"] = 500  # Fixed 500ms
            elif self.current_program_mode == ProgramMode.MANUAL:
                status["manual_exposure_us"] = self.manual_exposure_us
                status["manual_exposure_ms"] = self.manual_exposure_us / 1000
                status["manual_gain"] = self.manual_gain
                
            return status
        except:
            return {"program_mode": "auto", "preferred_gain": 250}
    
    def start_video_mode(self) -> bool:
        """Start video capture mode for continuous imaging."""
        if not self.is_initialized or not self.camera:
            return False
            
        try:
            # Stop any single exposure
            try:
                self.camera.stop_exposure()
            except:
                pass
                
            self.camera.start_video_capture()
            
            # Enable auto-exposure if available
            controls = self.camera.get_controls()
            if 'Exposure' in controls and controls['Exposure']['IsAutoSupported']:
                self.camera.set_control_value(
                    asi.ASI_EXPOSURE,
                    controls['Exposure']['DefaultValue'],
                    auto=True
                )
                
                # Enable auto-gain if available
                if 'Gain' in controls and controls['Gain']['IsAutoSupported']:
                    self.camera.set_control_value(
                        asi.ASI_GAIN,
                        controls['Gain']['DefaultValue'],
                        auto=True
                    )
            
            # Set appropriate timeout
            exposure_ms = self.camera.get_control_value(asi.ASI_EXPOSURE)[0]
            timeout = (exposure_ms / 1000) * 2 + 500
            self.camera.default_timeout = int(timeout)
            
            print("Video mode started")
            return True
            
        except Exception as e:
            print(f"Failed to start video mode: {e}")
            return False
    
    def capture_video_frame(self, filename: Optional[str] = None) -> Optional[str]:
        """Capture a single frame from video stream.
        
        Args:
            filename: Optional filename. If None, generates timestamp-based name.
            
        Returns:
            Path to saved image file, or None if capture failed.
        """
        if not self.is_initialized or not self.camera:
            return None
            
        try:
            if filename is None:
                timestamp = int(time.time())
                filename = f"frame_{timestamp}.jpg"
            
            filepath = self.image_dir / filename
            
            # Set image type
            if self.camera_info.get('IsColorCam', False):
                self.camera.set_image_type(asi.ASI_IMG_RGB24)
            else:
                self.camera.set_image_type(asi.ASI_IMG_RAW8)
            
            # Capture frame from video stream
            self.camera.capture_video_frame(filename=str(filepath))
            
            print(f"Video frame captured: {filepath}")
            return str(filepath)
            
        except Exception as e:
            print(f"Failed to capture video frame: {e}")
            return None
    
    def get_camera_status(self) -> Dict[str, Any]:
        """Get current camera status and settings."""
        if not self.is_initialized or not self.camera:
            return {"status": "disconnected", "error": "Camera not initialized"}
            
        try:
            control_values = self.camera.get_controls()
            current_settings = self.camera.get_control_values()
            
            # Get current exposure 
            exposure_us = current_settings.get('Exposure', 100000)
            
            # Get program mode status
            program_status = self.get_current_program_mode_status()
            
            return {
                "status": "connected",
                "camera_model": self.camera_info.get('Name', 'Unknown'),
                "is_color_camera": self.camera_info.get('IsColorCam', False),
                "current_settings": current_settings,
                "temperature": self.camera_info.get('ElecPerADU', 'N/A'),
                "dropped_frames": self.camera.get_dropped_frames() if hasattr(self.camera, 'get_dropped_frames') else 0,
                "program_mode": program_status["program_mode"],
                "scene_type": self._last_scene_type.value,
                "exposure_us": exposure_us,
                "refresh_interval": self._get_recommended_refresh_interval(),
                **program_status  # Include all program mode specific status
            }
            
        except Exception as e:
            return {"status": "error", "error": str(e)}
    
    def _get_recommended_refresh_interval(self) -> int:
        """Get recommended refresh interval in milliseconds for UI based on program mode."""
        if self.current_program_mode == ProgramMode.SLEWING:
            return 250   # Fast refresh for slewing
        elif self.current_program_mode == ProgramMode.AUTO:
            # Base on current adaptive exposure
            exposure_ms = self._current_exposure_us / 1000
            return max(500, int(exposure_ms + 200))  # Exposure time + overhead
        else:  # MANUAL
            # Base on manual exposure setting
            exposure_ms = self.manual_exposure_us / 1000
            return max(500, int(exposure_ms + 200))
    
    def disconnect(self) -> None:
        """Disconnect from the camera."""
        if self.camera:
            try:
                self.camera.stop_video_capture()
                self.camera.stop_exposure()
            except:
                pass
            
            self.camera = None
            self.is_initialized = False
            print("Camera disconnected")


# Global camera instance
_camera_instance = None

class StorageManager:
    """Manages image storage with retention policies for 32GB Pi systems."""
    
    MAX_STORAGE_GB = 3.0  # 3GB maximum storage
    MAX_AGE_DAYS = 3      # 3 day retention
    CLEANUP_INTERVAL_HOURS = 1  # Run cleanup every hour
    
    def __init__(self, image_dir: Path):
        self.image_dir = Path(image_dir)
        self.image_dir.mkdir(exist_ok=True)
        self._cleanup_thread = None
        self._cleanup_running = False
        
    def get_storage_stats(self) -> Dict[str, Any]:
        """Get current storage statistics."""
        try:
            if not self.image_dir.exists():
                return {
                    "total_images": 0,
                    "total_size_mb": 0.0,
                    "total_size_gb": 0.0,
                    "oldest_image": None,
                    "newest_image": None,
                    "storage_used_percent": 0.0,
                    "days_until_full": -1
                }
            
            # Get all image files
            image_files = list(self.image_dir.glob("*.jpg")) + list(self.image_dir.glob("*.jpeg"))
            
            if not image_files:
                return {
                    "total_images": 0,
                    "total_size_mb": 0.0,
                    "total_size_gb": 0.0,
                    "oldest_image": None,
                    "newest_image": None,
                    "storage_used_percent": 0.0,
                    "days_until_full": -1
                }
            
            # Calculate total size
            total_size_bytes = sum(f.stat().st_size for f in image_files)
            total_size_mb = total_size_bytes / (1024 * 1024)
            total_size_gb = total_size_mb / 1024
            
            # Find oldest and newest
            oldest = min(image_files, key=lambda p: p.stat().st_mtime)
            newest = max(image_files, key=lambda p: p.stat().st_mtime)
            
            # Calculate storage percentage
            storage_used_percent = (total_size_gb / self.MAX_STORAGE_GB) * 100
            
            # Estimate days until full (based on recent growth rate)
            days_until_full = self._estimate_days_until_full(image_files, total_size_gb)
            
            return {
                "total_images": len(image_files),
                "total_size_mb": round(total_size_mb, 2),
                "total_size_gb": round(total_size_gb, 3),
                "oldest_image": oldest.stat().st_mtime,
                "newest_image": newest.stat().st_mtime,
                "storage_used_percent": round(storage_used_percent, 1),
                "days_until_full": days_until_full,
                "max_storage_gb": self.MAX_STORAGE_GB,
                "max_age_days": self.MAX_AGE_DAYS
            }
            
        except Exception as e:
            print(f"Storage stats calculation failed: {e}")
            return {"error": str(e)}
    
    def _estimate_days_until_full(self, image_files: list, current_size_gb: float) -> int:
        """Estimate how many days until storage is full based on recent growth."""
        try:
            if current_size_gb >= self.MAX_STORAGE_GB:
                return 0
                
            # Look at images from last 24 hours to estimate daily growth
            now = time.time()
            day_ago = now - (24 * 3600)
            recent_files = [f for f in image_files if f.stat().st_mtime > day_ago]
            
            if len(recent_files) < 2:
                return -1  # Not enough data
                
            recent_size_bytes = sum(f.stat().st_size for f in recent_files)
            daily_growth_gb = recent_size_bytes / (1024 * 1024 * 1024)
            
            if daily_growth_gb <= 0:
                return -1
                
            remaining_gb = self.MAX_STORAGE_GB - current_size_gb
            return int(remaining_gb / daily_growth_gb)
            
        except Exception:
            return -1
    
    def cleanup_old_images(self, force: bool = False) -> Dict[str, Any]:
        """Clean up images based on age and storage limits."""
        try:
            if not self.image_dir.exists():
                return {"deleted_count": 0, "freed_mb": 0, "message": "No image directory"}
            
            image_files = list(self.image_dir.glob("*.jpg")) + list(self.image_dir.glob("*.jpeg"))
            if not image_files:
                return {"deleted_count": 0, "freed_mb": 0, "message": "No images to clean"}
            
            files_to_delete = []
            current_time = time.time()
            
            # Get current storage stats
            stats = self.get_storage_stats()
            current_size_gb = stats.get("total_size_gb", 0)
            
            # Strategy 1: Delete images older than MAX_AGE_DAYS
            age_cutoff = current_time - (self.MAX_AGE_DAYS * 24 * 3600)
            old_files = [f for f in image_files if f.stat().st_mtime < age_cutoff]
            files_to_delete.extend(old_files)
            
            # Strategy 2: If still over storage limit, delete oldest files
            if current_size_gb > self.MAX_STORAGE_GB or force:
                remaining_files = [f for f in image_files if f not in files_to_delete]
                remaining_files.sort(key=lambda p: p.stat().st_mtime)
                
                # Calculate how much to delete
                target_size_gb = self.MAX_STORAGE_GB * 0.8  # Target 80% of max
                
                while remaining_files and current_size_gb > target_size_gb:
                    oldest_file = remaining_files.pop(0)
                    file_size_gb = oldest_file.stat().st_size / (1024 * 1024 * 1024)
                    files_to_delete.append(oldest_file)
                    current_size_gb -= file_size_gb
            
            # Remove duplicates
            files_to_delete = list(set(files_to_delete))
            
            # Delete files and calculate freed space
            deleted_count = 0
            freed_bytes = 0
            
            for file_path in files_to_delete:
                try:
                    file_size = file_path.stat().st_size
                    file_path.unlink()
                    deleted_count += 1
                    freed_bytes += file_size
                    print(f"Deleted old image: {file_path.name}")
                except Exception as e:
                    print(f"Failed to delete {file_path.name}: {e}")
            
            freed_mb = freed_bytes / (1024 * 1024)
            
            return {
                "deleted_count": deleted_count,
                "freed_mb": round(freed_mb, 2),
                "message": f"Cleanup completed: {deleted_count} files deleted, {freed_mb:.1f}MB freed"
            }
            
        except Exception as e:
            print(f"Cleanup failed: {e}")
            return {"error": str(e), "deleted_count": 0, "freed_mb": 0}
    
    def start_background_cleanup(self):
        """Start background cleanup service."""
        if self._cleanup_running:
            return
            
        self._cleanup_running = True
        
        def cleanup_loop():
            while self._cleanup_running:
                try:
                    # Wait for cleanup interval
                    time.sleep(self.CLEANUP_INTERVAL_HOURS * 3600)
                    
                    if not self._cleanup_running:
                        break
                        
                    # Check if cleanup is needed
                    stats = self.get_storage_stats()
                    storage_percent = stats.get("storage_used_percent", 0)
                    
                    # Cleanup if over 85% or has images older than retention period
                    if storage_percent > 85 or stats.get("oldest_image", 0) < (time.time() - self.MAX_AGE_DAYS * 24 * 3600):
                        print(f"Background cleanup triggered (storage: {storage_percent}%)")
                        result = self.cleanup_old_images()
                        print(f"Background cleanup result: {result.get('message', 'Unknown')}")
                    
                except Exception as e:
                    print(f"Background cleanup error: {e}")
        
        self._cleanup_thread = threading.Thread(target=cleanup_loop, daemon=True)
        self._cleanup_thread.start()
        print("Background image cleanup service started")
    
    def stop_background_cleanup(self):
        """Stop background cleanup service."""
        self._cleanup_running = False
        if self._cleanup_thread:
            self._cleanup_thread.join(timeout=5)
        print("Background image cleanup service stopped")


def get_camera() -> ASI662MCCamera:
    """Get the global camera instance."""
    global _camera_instance
    if _camera_instance is None:
        _camera_instance = ASI662MCCamera()
    return _camera_instance


def get_storage_manager() -> StorageManager:
    """Get the global storage manager instance."""
    global _storage_manager_instance
    if _storage_manager_instance is None:
        _storage_manager_instance = StorageManager(Path("images"))
    return _storage_manager_instance


# Global instances
_storage_manager_instance: Optional[StorageManager] = None