#define _POSIX_C_SOURCE 200809L

#include <ASICamera2.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define ID_BYTES 8
#define HEX_TEXT_BYTES ((ID_BYTES * 2) + 1)

typedef struct {
    const char *camera_model;
    const char *expected_serial_hex;
} Options;

static void usage(const char *program) {
    fprintf(stderr,
            "Usage: %s --camera-model NAME "
            "[--expected-serial-hex 16_HEX_DIGITS]\n",
            program);
}

static bool parse_options(int argc, char **argv, Options *options) {
    if (argc != 3 && argc != 5) {
        return false;
    }
    for (int index = 1; index < argc; index += 2) {
        if (index + 1 >= argc) {
            return false;
        }
        if (strcmp(argv[index], "--camera-model") == 0) {
            options->camera_model = argv[index + 1];
        } else if (strcmp(argv[index], "--expected-serial-hex") == 0) {
            options->expected_serial_hex = argv[index + 1];
        } else {
            return false;
        }
    }
    return options->camera_model != NULL;
}

static int hex_digit(unsigned char character) {
    if (character >= '0' && character <= '9') {
        return character - '0';
    }
    if (character >= 'a' && character <= 'f') {
        return character - 'a' + 10;
    }
    if (character >= 'A' && character <= 'F') {
        return character - 'A' + 10;
    }
    return -1;
}

static bool parse_hex_id(const char *text, ASI_ID *identity) {
    if (strlen(text) != ID_BYTES * 2) {
        return false;
    }
    for (size_t index = 0; index < ID_BYTES; ++index) {
        const int high = hex_digit((unsigned char)text[index * 2]);
        const int low = hex_digit((unsigned char)text[(index * 2) + 1]);
        if (high < 0 || low < 0) {
            return false;
        }
        identity->id[index] = (unsigned char)((high << 4) | low);
    }
    return true;
}

static void format_hex_id(const ASI_ID *identity, char output[HEX_TEXT_BYTES]) {
    static const char digits[] = "0123456789abcdef";
    for (size_t index = 0; index < ID_BYTES; ++index) {
        output[index * 2] = digits[identity->id[index] >> 4];
        output[(index * 2) + 1] = digits[identity->id[index] & 0x0f];
    }
    output[ID_BYTES * 2] = '\0';
}

static void print_json_string(const char *text) {
    putchar('"');
    for (const unsigned char *cursor = (const unsigned char *)text; *cursor;
         ++cursor) {
        if (*cursor == '"' || *cursor == '\\') {
            putchar('\\');
        }
        putchar(*cursor);
    }
    putchar('"');
}

int main(int argc, char **argv) {
    Options options = {0};
    if (!parse_options(argc, argv, &options)) {
        usage(argv[0]);
        return EXIT_FAILURE;
    }

    ASI_ID expected_serial = {{0}};
    if (options.expected_serial_hex != NULL &&
        !parse_hex_id(options.expected_serial_hex, &expected_serial)) {
        fprintf(stderr, "Expected serial must contain exactly 16 hex digits\n");
        return EXIT_FAILURE;
    }

    const int camera_count = ASIGetNumOfConnectedCameras();
    int matching_count = 0;
    ASI_CAMERA_INFO selected = {0};
    for (int index = 0; index < camera_count; ++index) {
        ASI_CAMERA_INFO candidate = {0};
        if (ASIGetCameraProperty(&candidate, index) != ASI_SUCCESS) {
            fprintf(stderr, "Could not read camera property at index %d\n", index);
            return EXIT_FAILURE;
        }
        if (strcmp(candidate.Name, options.camera_model) == 0) {
            selected = candidate;
            matching_count += 1;
        }
    }

    if (matching_count != 1) {
        fprintf(stderr, "Required model %s matched %d cameras; refusing to open\n",
                options.camera_model, matching_count);
        return EXIT_FAILURE;
    }

    if (ASIOpenCamera(selected.CameraID) != ASI_SUCCESS) {
        fprintf(stderr, "Could not open selected %s CameraID %d\n", selected.Name,
                selected.CameraID);
        return EXIT_FAILURE;
    }
    int exit_code = EXIT_FAILURE;
    if (ASIInitCamera(selected.CameraID) != ASI_SUCCESS) {
        fprintf(stderr, "Could not initialize selected camera\n");
        goto cleanup;
    }

    ASI_SN serial = {{0}};
    const ASI_ERROR_CODE serial_result =
        ASIGetSerialNumber(selected.CameraID, &serial);
    ASI_ID flash_id = {{0}};
    const ASI_ERROR_CODE id_result = ASIGetID(selected.CameraID, &flash_id);

    if (serial_result != ASI_SUCCESS) {
        fprintf(stderr, "Selected camera does not expose a factory serial: %d\n",
                serial_result);
        goto cleanup;
    }
    if (options.expected_serial_hex != NULL &&
        memcmp(serial.id, expected_serial.id, ID_BYTES) != 0) {
        char actual_serial_hex[HEX_TEXT_BYTES];
        format_hex_id(&serial, actual_serial_hex);
        fprintf(stderr, "Factory serial mismatch: expected %s, observed %s\n",
                options.expected_serial_hex, actual_serial_hex);
        goto cleanup;
    }

    char serial_hex[HEX_TEXT_BYTES];
    char id_hex[HEX_TEXT_BYTES];
    format_hex_id(&serial, serial_hex);
    format_hex_id(&flash_id, id_hex);
    fputs("{\"camera_model\":", stdout);
    print_json_string(selected.Name);
    printf(",\"camera_id\":%d,\"factory_serial_hex\":\"%s\",",
           selected.CameraID, serial_hex);
    if (id_result == ASI_SUCCESS) {
        printf("\"asi_id_hex\":\"%s\"", id_hex);
    } else {
        fputs("\"asi_id_hex\":null", stdout);
    }
    fputs("}\n", stdout);
    exit_code = EXIT_SUCCESS;

cleanup:
    if (ASICloseCamera(selected.CameraID) != ASI_SUCCESS) {
        fprintf(stderr, "Could not close selected camera\n");
        return EXIT_FAILURE;
    }
    return exit_code;
}
