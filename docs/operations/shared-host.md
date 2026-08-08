# Shared-host resource policy and diagnostics

ObsCam shares the Raspberry Pi with AllSky, WireGuard, and optional ASIAIR
synchronization. Static relative policy keeps interactive monitoring useful
under contention without reserving idle capacity or imposing hard CPU quotas.

## Contention policy

| Workload | CPU weight | I/O weight | OOM adjustment | Memory high/max |
| --- | ---: | ---: | ---: | --- |
| ObsCam | 200 | 200 | -250 | 384/512 MiB |
| MediaMTX | 200 | 200 | -250 | 256/384 MiB |
| AllSky | 100 | 100 | -100 | unlimited |
| `asiair-sync` | 10 | 10 plus idle I/O | 500 | unlimited |

Weights matter only while resources are contended. AllSky retains the normal
systemd service weight, ObsCam and MediaMTX receive favorable interactive
weights, and synchronization yields first. The memory thresholds are generous
relative to the approximately 40–42 MiB steady-state measurements recorded
during GRE-224 qualification. `MemoryHigh` applies reclaim pressure before
`MemoryMax` prevents a runaway interactive service from exhausting the host.

The OOM adjustments prefer synchronization as the first managed victim during
severe memory exhaustion, then preserve both camera owners and the relay.
There is no viewer-aware policy; add one only if deployed evidence shows this
static policy is insufficient.

WireGuard traffic is handled by the kernel after the `wg-quick@wg0` oneshot
finishes, so process weights on that unit would not protect VPN data. WireGuard
is instead preserved by avoiding host CPU quotas, making optional sync yield,
bounding camera-service memory, and qualifying `wg0` under combined load.

The repository owns only
`/etc/systemd/system/allsky.service.d/50-obscam-resource-policy.conf`; it does
not replace or couple AllSky's service definition to the ObsCam target.

## Read-only diagnostic report

Run:

```sh
/usr/local/libexec/obscam/obscam-diagnostics
```

The command reads qualified binary/configuration identity, service state, the
latest 100 managed journal records, local health, automount/mount state, `wg0`,
resource controls and usage, free memory, temperature, and throttling state.
It neither starts services nor touches mount contents. A failed section is
reported with its command status and later sections still run.

Journal and WireGuard visibility depend on the invoking operator's host
permissions. Re-run the same command with `sudo` when a report contains only a
permission failure for those sections; elevated execution does not change the
command's read-only behavior.

`findmnt` reads the kernel mount table for `/mnt/asiair` and `/mnt/library`.
It does not probe the paths or trigger their automounts. The report deliberately
names only the approved browser VPN interface and must never be broadened to
unrelated infrastructure.

## Memory-controller prerequisite

`MemoryHigh` and `MemoryMax` require the cgroup v2 memory controller. A unit
file can retain those properties even when the kernel controller is disabled,
so successful `systemd-analyze verify` is not proof that the limits are active.
The appliance installer therefore fails closed on a live host unless `memory`
appears in:

```sh
cat /sys/fs/cgroup/cgroup.controllers
```

Raspberry Pi firmware can add `cgroup_disable=memory` ahead of the arguments in
`cmdline.txt`. On the qualified host, retain the existing single boot line and
append:

```text
cgroup_enable=memory cgroup_memory=1
```

After reboot, confirm that `/proc/cmdline` no longer disables the controller,
`memory` appears in the controller inventory, and each managed service cgroup
contains `memory.current`, `memory.high`, and `memory.max`. The diagnostic
command reports these facts and their actual values explicitly.

Use [`../verification/gre-225-shared-host.md`](../verification/gre-225-shared-host.md)
for repository and deployed qualification.
