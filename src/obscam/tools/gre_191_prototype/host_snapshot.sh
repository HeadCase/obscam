#!/usr/bin/env bash

set -u

if [[ $# -ne 1 ]]; then
    echo "usage: $0 OUTPUT_FILE" >&2
    exit 2
fi

output_file=$1
mkdir -p "$(dirname "$output_file")"
exec > >(tee "$output_file") 2>&1

section() {
    printf '\n## %s\n' "$1"
}

run_optional() {
    local label=$1
    shift
    section "$label"
    "$@" || printf 'command failed with status %s\n' "$?"
}

section "snapshot"
printf 'captured_at=%s\n' "$(date --iso-8601=seconds)"
printf 'boot_id=%s\n' "$(</proc/sys/kernel/random/boot_id)"
printf 'booted_at=%s\n' "$(uptime -s)"
uname -a

run_optional "services" systemctl show \
    allsky.service allskyperiodic.service asiair-sync.service \
    --property=Id,UnitFileState,ActiveState,SubState,MainPID,NRestarts,ExecMainStartTimestamp
run_optional "failed services" systemctl --failed --no-pager
run_optional "camera processes" pgrep -a -f 'capture_ZWO|allsky|obscam'
run_optional "USB devices" lsusb
run_optional "USB tree" lsusb -t
run_optional "WireGuard" sudo -n wg show
run_optional "network links" ip -brief link
run_optional "temperature" vcgencmd measure_temp
run_optional "throttling" vcgencmd get_throttled
run_optional "load" uptime
run_optional "recent boot kernel camera events" journalctl -b -k -n 300 --no-pager \
    --grep='03c3|ASI|usb .*reset|USB disconnect|new .*USB device|usbfs'
run_optional "recent boot service lifecycle" journalctl -b -n 200 --no-pager \
    -u allsky.service -u allskyperiodic.service -u asiair-sync.service \
    --grep='Starting|Started|Stopping|Stopped|Failed|failed|error|ERROR'
