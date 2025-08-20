import os
import time
import threading
from datetime import datetime, timedelta
from pathlib import Path
from typing import Dict, List, Optional, Tuple
import shutil


class StorageManager:
    """
    Storage management for ObsCam with dual retention strategy:
    1. Maximum age: 3 days (configurable)
    2. Maximum size: 3GB total (configurable) 
    
    Inspired by AllSky's 14-day retention but optimized for 32GB Pi systems.
    """
    
    def __init__(
        self, 
        image_dir: Path,
        max_age_days: int = 3,
        max_storage_gb: float = 3.0,
        cleanup_interval_hours: int = 1
    ):
        self.image_dir = Path(image_dir)
        self.max_age_days = max_age_days
        self.max_storage_gb = max_storage_gb
        self.MAX_STORAGE_GB = max_storage_gb  # For compatibility with web.py
        self.max_storage_bytes = int(max_storage_gb * 1024 * 1024 * 1024)
        self.cleanup_interval_hours = cleanup_interval_hours
        
        # Ensure directory exists
        self.image_dir.mkdir(exist_ok=True, parents=True)
        
        # Background cleanup thread
        self._cleanup_thread = None
        self._stop_cleanup = threading.Event()
        
        # Statistics
        self.stats = {
            'last_cleanup': None,
            'files_deleted_total': 0,
            'bytes_freed_total': 0,
            'last_cleanup_duration': 0
        }
    
    def start_background_cleanup(self) -> None:
        """Start the background cleanup service."""
        if self._cleanup_thread and self._cleanup_thread.is_alive():
            return
            
        self._stop_cleanup.clear()
        self._cleanup_thread = threading.Thread(
            target=self._background_cleanup_loop,
            name="storage-cleanup",
            daemon=True
        )
        self._cleanup_thread.start()
        print(f"Storage cleanup service started (interval: {self.cleanup_interval_hours}h)")
    
    def stop_background_cleanup(self) -> None:
        """Stop the background cleanup service."""
        if self._cleanup_thread and self._cleanup_thread.is_alive():
            self._stop_cleanup.set()
            self._cleanup_thread.join(timeout=5.0)
        print("Storage cleanup service stopped")
    
    def _background_cleanup_loop(self) -> None:
        """Background thread that runs cleanup periodically."""
        while not self._stop_cleanup.is_set():
            try:
                self.cleanup_old_images()
                
                # Wait for next interval or stop signal
                self._stop_cleanup.wait(timeout=self.cleanup_interval_hours * 3600)
                
            except Exception as e:
                print(f"Background cleanup error: {e}")
                # Wait 10 minutes before retrying on error
                self._stop_cleanup.wait(timeout=600)
    
    def get_storage_stats(self) -> Dict:
        """Get comprehensive storage information."""
        try:
            # Directory statistics
            total_size = 0
            file_count = 0
            oldest_file = None
            newest_file = None
            
            image_files = list(self.image_dir.glob("*.jpg")) + list(self.image_dir.glob("*.jpeg"))
            
            for file_path in image_files:
                try:
                    stat = file_path.stat()
                    total_size += stat.st_size
                    file_count += 1
                    
                    if oldest_file is None or stat.st_mtime < oldest_file[1]:
                        oldest_file = (file_path, stat.st_mtime)
                    if newest_file is None or stat.st_mtime > newest_file[1]:
                        newest_file = (file_path, stat.st_mtime)
                        
                except OSError:
                    continue
            
            # System disk space
            disk_usage = shutil.disk_usage(self.image_dir)
            disk_total = disk_usage.total
            disk_free = disk_usage.free
            disk_used = disk_total - disk_free
            
            # Calculate percentages and estimates
            storage_used_percent = (total_size / self.max_storage_bytes) * 100
            disk_used_percent = (disk_used / disk_total) * 100
            
            # Age-based cleanup estimate
            now = time.time()
            age_cutoff = now - (self.max_age_days * 24 * 3600)
            old_files = [f for f in image_files if f.stat().st_mtime < age_cutoff]
            
            return {
                'total_files': file_count,
                'total_size_bytes': total_size,
                'total_size_gb': total_size / (1024**3),
                'storage_used_percent': storage_used_percent,
                'max_storage_gb': self.max_storage_gb,
                'max_storage_bytes': self.max_storage_bytes,
                
                'oldest_file': {
                    'path': str(oldest_file[0]) if oldest_file else None,
                    'age_hours': (now - oldest_file[1]) / 3600 if oldest_file else None,
                    'age_days': (now - oldest_file[1]) / (24 * 3600) if oldest_file else None,
                } if oldest_file else None,
                
                'newest_file': {
                    'path': str(newest_file[0]) if newest_file else None,
                    'age_minutes': (now - newest_file[1]) / 60 if newest_file else None,
                } if newest_file else None,
                
                'cleanup_needed': {
                    'by_age': len(old_files),
                    'by_size': storage_used_percent > 100,
                    'emergency': storage_used_percent > 95
                },
                
                'disk_info': {
                    'total_gb': disk_total / (1024**3),
                    'used_gb': disk_used / (1024**3),
                    'free_gb': disk_free / (1024**3),
                    'used_percent': disk_used_percent
                },
                
                'stats': self.stats.copy()
            }
            
        except Exception as e:
            return {
                'error': str(e),
                'total_files': 0,
                'total_size_gb': 0,
                'storage_used_percent': 0,
                'max_storage_gb': self.max_storage_gb
            }
    
    def cleanup_old_images(self, force_emergency: bool = False) -> Dict:
        """
        Clean up old images based on dual retention strategy.
        
        Args:
            force_emergency: If True, performs aggressive cleanup regardless of normal limits
            
        Returns:
            Dict with cleanup statistics
        """
        start_time = time.time()
        files_deleted = 0
        bytes_freed = 0
        errors = []
        
        try:
            # Get all image files with their stats
            image_files = []
            for file_path in self.image_dir.glob("*.jpg"):
                try:
                    stat_info = file_path.stat()
                    image_files.append((file_path, stat_info.st_mtime, stat_info.st_size))
                except OSError as e:
                    errors.append(f"Failed to stat {file_path}: {e}")
            
            for file_path in self.image_dir.glob("*.jpeg"):
                try:
                    stat_info = file_path.stat()
                    image_files.append((file_path, stat_info.st_mtime, stat_info.st_size))
                except OSError as e:
                    errors.append(f"Failed to stat {file_path}: {e}")
            
            if not image_files:
                return self._cleanup_result(start_time, files_deleted, bytes_freed, errors)
            
            # Sort by modification time (oldest first)
            image_files.sort(key=lambda x: x[1])
            
            total_size = sum(size for _, _, size in image_files)
            now = time.time()
            age_cutoff = now - (self.max_age_days * 24 * 3600)
            
            # Strategy 1: Delete files older than max_age_days
            for file_path, mtime, size in image_files[:]:
                if mtime < age_cutoff:
                    if self._delete_file(file_path):
                        files_deleted += 1
                        bytes_freed += size
                        total_size -= size
                        image_files.remove((file_path, mtime, size))
                    else:
                        errors.append(f"Failed to delete old file: {file_path}")
            
            # Strategy 2: Delete oldest files if total size > max_storage_bytes
            # OR if force_emergency is True and we're over 95% of max
            size_limit = self.max_storage_bytes
            if force_emergency:
                size_limit = int(self.max_storage_bytes * 0.90)  # Clean to 90% in emergency
            
            while total_size > size_limit and image_files:
                # Always keep at least the 10 most recent images
                if len(image_files) <= 10 and not force_emergency:
                    break
                    
                file_path, mtime, size = image_files[0]  # Oldest remaining file
                
                if self._delete_file(file_path):
                    files_deleted += 1
                    bytes_freed += size
                    total_size -= size
                    image_files.remove((file_path, mtime, size))
                else:
                    errors.append(f"Failed to delete oversized file: {file_path}")
                    # Break to avoid infinite loop if we can't delete files
                    break
            
            # Update statistics
            duration = time.time() - start_time
            self.stats.update({
                'last_cleanup': datetime.now(),
                'files_deleted_total': self.stats['files_deleted_total'] + files_deleted,
                'bytes_freed_total': self.stats['bytes_freed_total'] + bytes_freed,
                'last_cleanup_duration': duration
            })
            
            result = self._cleanup_result(start_time, files_deleted, bytes_freed, errors)
            
            if files_deleted > 0:
                print(f"Cleanup completed: {files_deleted} files deleted, "
                      f"{bytes_freed / (1024**2):.1f}MB freed in {duration:.2f}s")
            
            return result
            
        except Exception as e:
            errors.append(f"Cleanup failed: {e}")
            return self._cleanup_result(start_time, files_deleted, bytes_freed, errors)
    
    def _delete_file(self, file_path: Path) -> bool:
        """Safely delete a file with error handling."""
        try:
            file_path.unlink()
            return True
        except OSError as e:
            print(f"Failed to delete {file_path}: {e}")
            return False
    
    def _cleanup_result(self, start_time: float, files_deleted: int, bytes_freed: int, errors: List[str]) -> Dict:
        """Create standardized cleanup result dictionary."""
        return {
            'files_deleted': files_deleted,
            'bytes_freed': bytes_freed,
            'duration_seconds': time.time() - start_time,
            'errors': errors,
            'timestamp': time.time()
        }
    
    def emergency_cleanup(self) -> Dict:
        """Perform emergency cleanup when storage is critically low."""
        print("EMERGENCY CLEANUP: Storage critically low, performing aggressive cleanup...")
        return self.cleanup_old_images(force_emergency=True)
    
    def get_oldest_files(self, count: int = 10) -> List[Dict]:
        """Get information about the oldest files."""
        try:
            image_files = []
            for file_path in self.image_dir.glob("*.jpg"):
                try:
                    stat_info = file_path.stat()
                    age_days = (time.time() - stat_info.st_mtime) / (24 * 3600)
                    image_files.append({
                        'path': str(file_path),
                        'name': file_path.name,
                        'size_mb': stat_info.st_size / (1024**2),
                        'age_days': age_days,
                        'timestamp': stat_info.st_mtime
                    })
                except OSError:
                    continue
            
            # Sort by age (oldest first) and return top count
            image_files.sort(key=lambda x: x['timestamp'])
            return image_files[:count]
            
        except Exception as e:
            return [{'error': str(e)}]