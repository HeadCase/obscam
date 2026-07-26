# GRE-191 native camera identity prototype

**Throwaway evidence tooling.** This probe answers whether a native process can
resolve and validate the ASI662MC's persistent SDK identity without opening the
production ASI178MC. It is not a production camera owner.

## Result

**Theory validated under the operational ownership invariant.** The SDK reports
the exact model name `ZWO ASI662MC`; an initial short-name input correctly found
no exact match. With the exact name, the standalone native probe consistently
selected CameraID `0` and reported factory serial `1d274e0920010900`.

Twenty repeated expected-serial starts succeeded, and a deliberately incorrect
serial failed closed. AllSky retained its original ASI178MC capture process with
zero service restarts; neither camera disconnected or re-enumerated.

SDK enumeration does perform a USB configuration operation against the
AllSky-owned ASI178MC:

```text
usb 1-1.1.4: usbfs: interface 0 claimed by usbfs while
'gre-191-identit' sets config #1
```

This is accepted as enumeration rather than ownership. The ObsCam invariant is
that it must never open, initialize, control, stop, close, disconnect, or
displace AllSky's ASI178MC handle. The experiment observed none of those events.
Production design still needs to account for SDK-wide enumeration, but native
model-plus-factory-serial validation is credible on the deployed two-model
topology.

## Host reboot and application restart result

One operator-initiated host reboot preserved both camera identities and their
USB topology:

- ASI178MC: `03c3:178a`, path `1-1.1.4`, 480 Mbit/s, factory serial
  `0f22430125090900`.
- ASI662MC: `03c3:662b`, path `2-2`, 5000 Mbit/s, factory serial
  `1d274e0920010900`.

AllSky started after the boot reached `multi-user.target`, opened the ASI178MC,
reported its expected serial, and captured frames. The ASI662MC probe then
validated its expected serial without displacing AllSky. An independent
`allsky.service` restart cleanly stopped the original capture process and
started a replacement that reacquired the same ASI178MC serial and resumed
capture. A second ASI662MC validation also passed. There were no USB disconnects
or topology changes.

The boot was delayed by an unrelated shared-host dependency: the remote
`/mnt/library` NFS mount was attempted before its WireGuard path was usable and
timed out after approximately 104 seconds. Because AllSky is ordered after
`multi-user.target`, its start waited behind that mount. AllSky then started
normally. A retry after WireGuard was established mounted the share and allowed
`asiair-sync.service` and rclone to resume. This is deployment-policy evidence,
not a camera-ownership failure.

Physical camera and hub unplug/replug permutations were removed from the test
scope because the deployed Pi and its I/O are fixed and rack-mounted.

Build and run it without Python:

```bash
cc -O2 -std=c11 -Wall -Wextra -Werror \
  -I/home/gheadley/allsky/src/include \
  src/obscam/tools/gre_191_prototype/identity_probe.c \
  -L/usr/local/lib -lASICamera2 \
  -o /tmp/gre-191-identity-probe

/tmp/gre-191-identity-probe --camera-model 'ZWO ASI662MC'
```

After enrollment, require the reported factory serial on every start:

```bash
/tmp/gre-191-identity-probe \
  --camera-model 'ZWO ASI662MC' \
  --expected-serial-hex 1d274e0920010900
```

The probe enumerates non-opening camera properties, requires exactly one model
match, opens only that candidate, reads its factory serial and writable ASI ID,
and fails closed if the expected serial differs. Missing or ambiguous model
selectors fail before any camera is opened.

## Controlled host-reboot validation

The project owner subsequently approved one host reboot and a brief AllSky
interruption during a maintenance window. Physical camera and hub changes remain
out of scope for the fixed, rack-mounted installation.

Capture comparable evidence before and after reboot with:

```bash
src/obscam/tools/gre_191_prototype/host_snapshot.sh SNAPSHOT_FILE
```

The reboot must be initiated by the operator after the pre-reboot snapshot is
complete. After reconnecting, capture the post-reboot snapshot before changing
service state, then validate the ASI662MC with the exact model and enrolled
factory serial. AllSky must reacquire and retain exclusive ownership of the
ASI178MC throughout application-level testing.
