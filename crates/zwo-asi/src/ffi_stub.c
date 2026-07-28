/* Link-only CI adapter for the unavailable proprietary ZWO SDK edge. */
int ASIGetNumOfConnectedCameras(void) { return 0; }
int ASIGetCameraProperty(void *info, int index) { (void)info; (void)index; return 1; }
int ASIOpenCamera(int camera_id) { (void)camera_id; return 1; }
int ASIInitCamera(int camera_id) { (void)camera_id; return 1; }
int ASICloseCamera(int camera_id) { (void)camera_id; return 0; }
int ASIGetSerialNumber(int camera_id, void *serial) { (void)camera_id; (void)serial; return 1; }
int ASISetROIFormat(int camera_id, int width, int height, int bin, int image_type) {
    (void)camera_id; (void)width; (void)height; (void)bin; (void)image_type; return 1;
}
int ASISetControlValue(int camera_id, int control, long value, int automatic) {
    (void)camera_id; (void)control; (void)value; (void)automatic; return 1;
}
int ASIStartVideoCapture(int camera_id) { (void)camera_id; return 1; }
int ASIStopVideoCapture(int camera_id) { (void)camera_id; return 0; }
int ASIGetVideoData(int camera_id, unsigned char *buffer, long size, int wait_ms) {
    (void)camera_id; (void)buffer; (void)size; (void)wait_ms; return 1;
}
