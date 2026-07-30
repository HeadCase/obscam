#include <string.h>

typedef struct {
    char Name[64]; int CameraID; long MaxHeight; long MaxWidth;
    int IsColorCam; int BayerPattern; int SupportedBins[16];
    int SupportedVideoFormat[8]; double PixelSize; int MechanicalShutter;
    int ST4Port; int IsCoolerCam; int IsUSB3Host; int IsUSB3Camera;
    float ElecPerADU; int BitDepth; int IsTriggerCam; char Unused[16];
} ASI_CAMERA_INFO;
typedef struct { unsigned char id[8]; } ASI_SN;

enum { MAX_CAMERAS = 8, SUCCESS = 0 };
static ASI_CAMERA_INFO cameras[MAX_CAMERAS];
static unsigned char serials[MAX_CAMERAS][8];
static int camera_count, open_calls[MAX_CAMERAS], init_calls[MAX_CAMERAS];
static int close_calls[MAX_CAMERAS], control_calls[MAX_CAMERAS];
static int start_calls[MAX_CAMERAS], stop_calls[MAX_CAMERAS];
static int capture_calls[MAX_CAMERAS], capture_result, dropped_frames;
static int operation_result[11];
static int last_wait_ms;

void ObsCamStubReset(void) {
    memset(cameras, 0, sizeof(cameras)); memset(serials, 0, sizeof(serials));
    memset(open_calls, 0, sizeof(open_calls)); memset(init_calls, 0, sizeof(init_calls));
    memset(close_calls, 0, sizeof(close_calls)); memset(control_calls, 0, sizeof(control_calls));
    memset(start_calls, 0, sizeof(start_calls)); memset(stop_calls, 0, sizeof(stop_calls));
    memset(capture_calls, 0, sizeof(capture_calls));
    memset(operation_result, 0, sizeof(operation_result));
    camera_count = 0; capture_result = SUCCESS; dropped_frames = 0;
    last_wait_ms = 0;
}

void ObsCamStubAddCamera(const char *name, int camera_id) {
    ASI_CAMERA_INFO *camera = &cameras[camera_count++];
    strncpy(camera->Name, name, sizeof(camera->Name) - 1); camera->CameraID = camera_id;
    camera->MaxWidth = 1920; camera->MaxHeight = 1080; camera->IsColorCam = 1;
    camera->BayerPattern = 0; camera->SupportedBins[0] = 1;
    camera->SupportedVideoFormat[0] = 0; camera->SupportedVideoFormat[1] = -1;
    const unsigned char expected[8] = {0x1d,0x27,0x4e,0x09,0x20,0x01,0x09,0x00};
    memcpy(serials[camera_id], expected, 8);
}
void ObsCamStubSetSerial(int id, const unsigned char *serial) { memcpy(serials[id], serial, 8); }
void ObsCamStubSetShape(int id, long width, long height, int color, int bayer, int raw8) {
    for (int i = 0; i < camera_count; ++i) if (cameras[i].CameraID == id) {
        cameras[i].MaxWidth = width; cameras[i].MaxHeight = height;
        cameras[i].IsColorCam = color; cameras[i].BayerPattern = bayer;
        cameras[i].SupportedVideoFormat[0] = raw8 ? 0 : 1;
    }
}
void ObsCamStubSetCaptureResult(int result) { capture_result = result; }
void ObsCamStubSetDroppedFrames(int count) { dropped_frames = count; }
void ObsCamStubSetResult(int operation, int result) { operation_result[operation] = result; }
int ObsCamStubCalls(int operation, int id) {
    int *groups[] = {open_calls, init_calls, close_calls, control_calls, start_calls, stop_calls, capture_calls};
    return groups[operation][id];
}
int ObsCamStubLastWaitMs(void) { return last_wait_ms; }

int ASIGetNumOfConnectedCameras(void) { return camera_count; }
int ASIGetCameraProperty(ASI_CAMERA_INFO *info, int index) {
    if (index < 0 || index >= camera_count) return 1;
    if (operation_result[7] != SUCCESS) return operation_result[7];
    *info = cameras[index];
    return SUCCESS;
}
int ASIOpenCamera(int id) { open_calls[id]++; return operation_result[0]; }
int ASIInitCamera(int id) { init_calls[id]++; return operation_result[1]; }
int ASICloseCamera(int id) { close_calls[id]++; return operation_result[2]; }
int ASIGetSerialNumber(int id, ASI_SN *serial) { if (operation_result[8]) return operation_result[8]; memcpy(serial->id, serials[id], 8); return SUCCESS; }
int ASISetROIFormat(int id, int width, int height, int bin, int type) {
    (void)id; if (operation_result[9]) return operation_result[9]; return width == 1920 && height == 1080 && bin == 1 && type == 0 ? SUCCESS : 8;
}
int ASISetControlValue(int id, int control, long value, int automatic) {
    (void)control; (void)value; (void)automatic; control_calls[id]++; return operation_result[3];
}
int ASIStartVideoCapture(int id) { start_calls[id]++; return operation_result[4]; }
int ASIStopVideoCapture(int id) { stop_calls[id]++; return operation_result[5]; }
int ASIGetVideoData(int id, unsigned char *buffer, long size, int wait_ms) {
    last_wait_ms = wait_ms; capture_calls[id]++; if (capture_result != SUCCESS) return capture_result;
    memset(buffer, capture_calls[id], (size_t)size); return SUCCESS;
}
int ASIGetDroppedFrames(int id, int *count) { (void)id; if (operation_result[10]) return operation_result[10]; *count = dropped_frames; return SUCCESS; }
