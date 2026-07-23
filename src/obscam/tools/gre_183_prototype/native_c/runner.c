#define _POSIX_C_SOURCE 200809L

#include <ASICamera2.h>
#include <errno.h>
#include <math.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <zlib.h>

#define FULL_WIDTH 1920
#define FULL_HEIGHT 1080
#define GUARD_BYTES 64
#define MAX_EXPECTED_FPS 150.0
#define TIMING_HEADROOM 1024

typedef struct {
    ASI_IMG_TYPE image_type;
    const char *image_format;
    long exposure_us;
    long gain;
    long high_speed;
    long bandwidth;
    double duration_s;
    int warmup_frames;
    int timeout_ms;
    const char *camera_model;
} Scenario;

typedef struct {
    double *values;
    size_t length;
    size_t capacity;
} Timings;

static void usage(const char *program) {
    fprintf(stderr,
            "Usage: %s --format Y8|RAW8|RGB24 --exposure-us N --gain N "
            "--high-speed 0|1 --bandwidth N --duration-s N "
            "--warmup-frames N --timeout-ms N [--camera-model NAME]\n",
            program);
}

static bool parse_long(const char *text, long *value) {
    char *end = NULL;
    errno = 0;
    long parsed = strtol(text, &end, 10);
    if (errno != 0 || end == text || *end != '\0') {
        return false;
    }
    *value = parsed;
    return true;
}

static bool parse_double(const char *text, double *value) {
    char *end = NULL;
    errno = 0;
    double parsed = strtod(text, &end);
    if (errno != 0 || end == text || *end != '\0' || !isfinite(parsed)) {
        return false;
    }
    *value = parsed;
    return true;
}

static bool parse_format(const char *text, Scenario *scenario) {
    if (strcmp(text, "Y8") == 0) {
        scenario->image_type = ASI_IMG_Y8;
        scenario->image_format = "Y8";
        return true;
    }
    if (strcmp(text, "RAW8") == 0) {
        scenario->image_type = ASI_IMG_RAW8;
        scenario->image_format = "RAW8";
        return true;
    }
    if (strcmp(text, "RGB24") == 0) {
        scenario->image_type = ASI_IMG_RGB24;
        scenario->image_format = "RGB24";
        return true;
    }
    return false;
}

static bool parse_args(int argc, char **argv, Scenario *scenario) {
    bool have_format = false;
    bool have_exposure = false;
    bool have_gain = false;
    bool have_high_speed = false;
    bool have_bandwidth = false;
    bool have_duration = false;
    bool have_warmup = false;
    bool have_timeout = false;
    scenario->camera_model = "ASI662MC";

    for (int index = 1; index < argc; index += 2) {
        if (index + 1 >= argc) {
            return false;
        }
        const char *option = argv[index];
        const char *value = argv[index + 1];
        long parsed = 0;
        if (strcmp(option, "--format") == 0) {
            have_format = parse_format(value, scenario);
        } else if (strcmp(option, "--exposure-us") == 0) {
            have_exposure = parse_long(value, &scenario->exposure_us);
        } else if (strcmp(option, "--gain") == 0) {
            have_gain = parse_long(value, &scenario->gain);
        } else if (strcmp(option, "--high-speed") == 0) {
            have_high_speed = parse_long(value, &scenario->high_speed);
        } else if (strcmp(option, "--bandwidth") == 0) {
            have_bandwidth = parse_long(value, &scenario->bandwidth);
        } else if (strcmp(option, "--duration-s") == 0) {
            have_duration = parse_double(value, &scenario->duration_s);
        } else if (strcmp(option, "--warmup-frames") == 0) {
            if (!parse_long(value, &parsed) || parsed < 0 || parsed > INT32_MAX) {
                return false;
            }
            scenario->warmup_frames = (int)parsed;
            have_warmup = true;
        } else if (strcmp(option, "--timeout-ms") == 0) {
            if (!parse_long(value, &parsed) || parsed < 1 || parsed > INT32_MAX) {
                return false;
            }
            scenario->timeout_ms = (int)parsed;
            have_timeout = true;
        } else if (strcmp(option, "--camera-model") == 0) {
            scenario->camera_model = value;
        } else {
            return false;
        }
    }

    return have_format && have_exposure && scenario->exposure_us > 0 &&
           have_gain && scenario->gain >= 0 && have_high_speed &&
           (scenario->high_speed == 0 || scenario->high_speed == 1) &&
           have_bandwidth && scenario->bandwidth >= 0 && have_duration &&
           scenario->duration_s > 0 && have_warmup && have_timeout;
}

static uint64_t monotonic_ns(void) {
    struct timespec timestamp;
#ifdef CLOCK_MONOTONIC_RAW
    const clockid_t clock_id = CLOCK_MONOTONIC_RAW;
#else
    const clockid_t clock_id = CLOCK_MONOTONIC;
#endif
    if (clock_gettime(clock_id, &timestamp) != 0) {
        perror("clock_gettime");
        exit(EXIT_FAILURE);
    }
    return ((uint64_t)timestamp.tv_sec * 1000000000ULL) +
           (uint64_t)timestamp.tv_nsec;
}

static bool timings_init(Timings *timings, size_t capacity) {
    timings->values = calloc(capacity, sizeof(double));
    timings->length = 0;
    timings->capacity = capacity;
    return timings->values != NULL;
}

static bool timings_push(Timings *timings, double value) {
    if (timings->length >= timings->capacity) {
        return false;
    }
    timings->values[timings->length++] = value;
    return true;
}

static int compare_double(const void *left, const void *right) {
    const double left_value = *(const double *)left;
    const double right_value = *(const double *)right;
    return (left_value > right_value) - (left_value < right_value);
}

static double percentile(const double *values, size_t length, double quantile) {
    const double position = (double)(length - 1) * quantile;
    const size_t lower = (size_t)floor(position);
    const size_t upper = (size_t)ceil(position);
    if (lower == upper) {
        return values[lower];
    }
    return values[lower] +
           ((values[upper] - values[lower]) * (position - (double)lower));
}

static void print_timing_summary(Timings *timings) {
    if (timings->length == 0) {
        printf("{\"count\":0,\"minimum_ms\":null,\"mean_ms\":null,"
               "\"p50_ms\":null,\"p95_ms\":null,\"p99_ms\":null,"
               "\"maximum_ms\":null}");
        return;
    }
    double sum = 0;
    for (size_t index = 0; index < timings->length; ++index) {
        sum += timings->values[index];
    }
    qsort(timings->values, timings->length, sizeof(double), compare_double);
    printf("{\"count\":%zu,\"minimum_ms\":%.9f,\"mean_ms\":%.9f,"
           "\"p50_ms\":%.9f,\"p95_ms\":%.9f,\"p99_ms\":%.9f,"
           "\"maximum_ms\":%.9f}",
           timings->length, timings->values[0], sum / (double)timings->length,
           percentile(timings->values, timings->length, 0.50),
           percentile(timings->values, timings->length, 0.95),
           percentile(timings->values, timings->length, 0.99),
           timings->values[timings->length - 1]);
}

static void print_json_string(const char *text) {
    putchar('"');
    for (const unsigned char *cursor = (const unsigned char *)text; *cursor;
         ++cursor) {
        switch (*cursor) {
        case '"':
            fputs("\\\"", stdout);
            break;
        case '\\':
            fputs("\\\\", stdout);
            break;
        case '\n':
            fputs("\\n", stdout);
            break;
        case '\r':
            fputs("\\r", stdout);
            break;
        case '\t':
            fputs("\\t", stdout);
            break;
        default:
            if (*cursor < 0x20) {
                printf("\\u%04x", *cursor);
            } else {
                putchar(*cursor);
            }
        }
    }
    putchar('"');
}

static bool format_supported(const ASI_CAMERA_INFO *info,
                             ASI_IMG_TYPE image_type) {
    for (size_t index = 0; index < 8; ++index) {
        if (info->SupportedVideoFormat[index] == ASI_IMG_END) {
            return false;
        }
        if (info->SupportedVideoFormat[index] == image_type) {
            return true;
        }
    }
    return false;
}

static bool camera_supports_control(int camera_id,
                                    ASI_CONTROL_TYPE control_type) {
    int control_count = 0;
    if (ASIGetNumOfControls(camera_id, &control_count) != ASI_SUCCESS) {
        return false;
    }
    for (int index = 0; index < control_count; ++index) {
        ASI_CONTROL_CAPS capability = {0};
        if (ASIGetControlCaps(camera_id, index, &capability) == ASI_SUCCESS &&
            (int)capability.ControlType == control_type) {
            return capability.IsWritable == ASI_TRUE;
        }
    }
    return false;
}

static int open_required_camera(const char *required_model,
                                ASI_CAMERA_INFO *selected_info) {
    const int camera_count = ASIGetNumOfConnectedCameras();
    for (int index = 0; index < camera_count; ++index) {
        ASI_CAMERA_INFO info = {0};
        if (ASIGetCameraProperty(&info, index) != ASI_SUCCESS) {
            continue;
        }
        if (strstr(info.Name, required_model) == NULL) {
            continue;
        }
        if (ASIOpenCamera(info.CameraID) != ASI_SUCCESS) {
            return -1;
        }
        if (ASIInitCamera(info.CameraID) != ASI_SUCCESS) {
            ASICloseCamera(info.CameraID);
            return -1;
        }
        *selected_info = info;
        return info.CameraID;
    }
    return -1;
}

static bool guards_intact(const unsigned char *allocation,
                          size_t frame_bytes) {
    for (size_t index = 0; index < GUARD_BYTES; ++index) {
        if (allocation[index] != 0xA5 ||
            allocation[GUARD_BYTES + frame_bytes + index] != 0xA5) {
            return false;
        }
    }
    return true;
}

int main(int argc, char **argv) {
    Scenario scenario = {0};
    if (!parse_args(argc, argv, &scenario)) {
        usage(argv[0]);
        return EXIT_FAILURE;
    }

    ASI_CAMERA_INFO camera_info = {0};
    const int camera_id = open_required_camera(scenario.camera_model, &camera_info);
    if (camera_id < 0) {
        fprintf(stderr, "Required camera %s could not be opened\n",
                scenario.camera_model);
        return EXIT_FAILURE;
    }
    int exit_code = EXIT_FAILURE;
    unsigned char *allocation = NULL;
    Timings capture_timings = {0};
    Timings inter_frame_timings = {0};
    Timings unique_inter_frame_timings = {0};
    bool video_started = false;

    if (camera_info.MaxWidth != FULL_WIDTH ||
        camera_info.MaxHeight != FULL_HEIGHT ||
        !format_supported(&camera_info, scenario.image_type)) {
        fprintf(stderr, "Camera does not support the requested full-frame format\n");
        goto cleanup;
    }
    const size_t bytes_per_pixel =
        scenario.image_type == ASI_IMG_RGB24 ? 3U : 1U;
    const size_t frame_bytes =
        (size_t)FULL_WIDTH * (size_t)FULL_HEIGHT * bytes_per_pixel;
    const size_t allocation_bytes = frame_bytes + (2U * GUARD_BYTES);
    allocation = malloc(allocation_bytes);
    if (allocation == NULL) {
        perror("malloc frame buffer");
        goto cleanup;
    }
    memset(allocation, 0xA5, allocation_bytes);
    unsigned char *frame_buffer = allocation + GUARD_BYTES;

    const size_t timing_capacity =
        (size_t)ceil(scenario.duration_s * MAX_EXPECTED_FPS) + TIMING_HEADROOM;
    if (!timings_init(&capture_timings, timing_capacity) ||
        !timings_init(&inter_frame_timings, timing_capacity) ||
        !timings_init(&unique_inter_frame_timings, timing_capacity)) {
        fprintf(stderr, "Could not allocate timing arrays\n");
        goto cleanup;
    }

    (void)ASIStopVideoCapture(camera_id);
    (void)ASIStopExposure(camera_id);
    if (ASISetControlValue(camera_id, ASI_EXPOSURE, scenario.exposure_us,
                           ASI_FALSE) != ASI_SUCCESS ||
        ASISetControlValue(camera_id, ASI_GAIN, scenario.gain, ASI_FALSE) !=
            ASI_SUCCESS ||
        ASISetControlValue(camera_id, ASI_BANDWIDTHOVERLOAD,
                           scenario.bandwidth, ASI_FALSE) != ASI_SUCCESS) {
        fprintf(stderr, "Could not configure required SDK controls\n");
        goto cleanup;
    }
    if (camera_supports_control(camera_id, ASI_HIGH_SPEED_MODE) &&
        ASISetControlValue(camera_id, ASI_HIGH_SPEED_MODE, scenario.high_speed,
                           ASI_FALSE) != ASI_SUCCESS) {
        fprintf(stderr, "Could not configure high-speed mode\n");
        goto cleanup;
    }
    if (ASISetROIFormat(camera_id, FULL_WIDTH, FULL_HEIGHT, 1,
                        scenario.image_type) != ASI_SUCCESS ||
        ASISetStartPos(camera_id, 0, 0) != ASI_SUCCESS) {
        fprintf(stderr, "Could not configure full-frame SDK output\n");
        goto cleanup;
    }
    if (ASIStartVideoCapture(camera_id) != ASI_SUCCESS) {
        fprintf(stderr, "Could not start video capture\n");
        goto cleanup;
    }
    video_started = true;

    for (int index = 0; index < scenario.warmup_frames; ++index) {
        if (ASIGetVideoData(camera_id, frame_buffer, (long)frame_bytes,
                            scenario.timeout_ms) != ASI_SUCCESS) {
            fprintf(stderr, "Warm-up frame failed\n");
            goto cleanup;
        }
    }

    int sdk_dropped_start = 0;
    int sdk_dropped_end = 0;
    if (ASIGetDroppedFrames(camera_id, &sdk_dropped_start) != ASI_SUCCESS) {
        fprintf(stderr, "Could not read initial SDK drop counter\n");
        goto cleanup;
    }

    const uint64_t measurement_started_ns = monotonic_ns();
    const uint64_t duration_ns = (uint64_t)(scenario.duration_s * 1e9);
    uint64_t measurement_ended_ns = measurement_started_ns;
    uint64_t previous_completion_ns = 0;
    uint64_t previous_unique_completion_ns = 0;
    uLong previous_crc32 = 0;
    bool have_previous_crc32 = false;
    size_t frames = 0;
    size_t unique_frames = 0;
    size_t adjacent_duplicates = 0;
    size_t capture_errors = 0;
    size_t corrupt_frames = 0;

    while (monotonic_ns() - measurement_started_ns < duration_ns) {
        const uint64_t capture_started_ns = monotonic_ns();
        const ASI_ERROR_CODE capture_result = ASIGetVideoData(
            camera_id, frame_buffer, (long)frame_bytes, scenario.timeout_ms);
        if (capture_result != ASI_SUCCESS) {
            ++capture_errors;
            continue;
        }
        const uint64_t completion_ns = monotonic_ns();
        ++frames;
        if (!timings_push(&capture_timings,
                          (double)(completion_ns - capture_started_ns) / 1e6) ||
            (previous_completion_ns != 0 &&
             !timings_push(&inter_frame_timings,
                           (double)(completion_ns - previous_completion_ns) /
                               1e6))) {
            fprintf(stderr, "Timing capacity exceeded\n");
            goto cleanup;
        }
        previous_completion_ns = completion_ns;
        if (!guards_intact(allocation, frame_bytes)) {
            ++corrupt_frames;
        }

        uLong frame_crc32 = crc32(0L, Z_NULL, 0);
        frame_crc32 = crc32(frame_crc32, frame_buffer, (uInt)frame_bytes);
        if (have_previous_crc32 && frame_crc32 == previous_crc32) {
            ++adjacent_duplicates;
            continue;
        }
        previous_crc32 = frame_crc32;
        have_previous_crc32 = true;
        ++unique_frames;
        if (previous_unique_completion_ns != 0 &&
            !timings_push(
                &unique_inter_frame_timings,
                (double)(completion_ns - previous_unique_completion_ns) / 1e6)) {
            fprintf(stderr, "Unique timing capacity exceeded\n");
            goto cleanup;
        }
        previous_unique_completion_ns = completion_ns;
    }
    measurement_ended_ns = monotonic_ns();
    if (ASIGetDroppedFrames(camera_id, &sdk_dropped_end) != ASI_SUCCESS) {
        fprintf(stderr, "Could not read final SDK drop counter\n");
        goto cleanup;
    }
    if (sdk_dropped_end < sdk_dropped_start) {
        fprintf(stderr, "SDK drop counter moved backwards\n");
        goto cleanup;
    }

    const double elapsed_s =
        (double)(measurement_ended_ns - measurement_started_ns) / 1e9;
    printf("{\"schema_version\":1,\"runner\":\"native-c\",\"scenario\":{");
    printf("\"image_format\":\"%s\",\"exposure_us\":%ld,\"gain\":%ld,",
           scenario.image_format, scenario.exposure_us, scenario.gain);
    printf("\"high_speed\":%ld,\"bandwidth\":%ld,\"duration_s\":%.9f,",
           scenario.high_speed, scenario.bandwidth, scenario.duration_s);
    printf("\"warmup_frames\":%d,\"timeout_ms\":%d,\"width\":%d,"
           "\"height\":%d},\"sdk_version\":",
           scenario.warmup_frames, scenario.timeout_ms, FULL_WIDTH, FULL_HEIGHT);
    print_json_string(ASIGetSDKVersion());
    printf(",\"camera_name\":");
    print_json_string(camera_info.Name);
    printf(",\"elapsed_s\":%.9f,\"frames\":%zu,\"unique_frames\":%zu,"
           "\"adjacent_duplicates\":%zu,\"cadence_fps\":%.9f,"
           "\"unique_cadence_fps\":%.9f,",
           elapsed_s, frames, unique_frames, adjacent_duplicates,
           (double)frames / elapsed_s, (double)unique_frames / elapsed_s);
    printf("\"sdk_dropped_start\":%d,\"sdk_dropped_end\":%d,"
           "\"sdk_dropped_delta\":%d,\"capture_errors\":%zu,"
           "\"corrupt_frames\":%zu,\"pipeline_drops\":0,",
           sdk_dropped_start, sdk_dropped_end,
           sdk_dropped_end - sdk_dropped_start, capture_errors, corrupt_frames);
    printf("\"buffer_allocations\":1,\"buffer_allocation_bytes\":%zu,"
           "\"downstream_copy_bytes\":0,\"crc32_last\":",
           allocation_bytes);
    if (have_previous_crc32) {
        printf("%lu", (unsigned long)previous_crc32);
    } else {
        printf("null");
    }
    printf(",\"capture_call_ms\":");
    print_timing_summary(&capture_timings);
    printf(",\"inter_frame_ms\":");
    print_timing_summary(&inter_frame_timings);
    printf(",\"unique_inter_frame_ms\":");
    print_timing_summary(&unique_inter_frame_timings);
    printf(",\"notes\":[\"caller-owned guarded SDK frame buffer reused for "
           "the complete run\",\"timing arrays preallocated before "
           "capture\"]}\n");
    exit_code = EXIT_SUCCESS;

cleanup:
    if (video_started) {
        (void)ASIStopVideoCapture(camera_id);
    }
    (void)ASICloseCamera(camera_id);
    free(unique_inter_frame_timings.values);
    free(inter_frame_timings.values);
    free(capture_timings.values);
    free(allocation);
    return exit_code;
}
