use std::{fs, path::PathBuf, process::Command};

fn repository_file(path: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path);
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}

#[test]
fn appliance_installer_preserves_owned_paths_and_rejects_drift() {
    let installer_test = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("deploy/tests/install-appliance-test");
    let output = Command::new(&installer_test)
        .output()
        .unwrap_or_else(|error| panic!("run {}: {error}", installer_test.display()));

    assert!(
        output.status.success(),
        "{} failed\nstdout:\n{}\nstderr:\n{}",
        installer_test.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn mediamtx_advertises_only_permitted_browser_hosts() {
    let config = repository_file("deploy/mediamtx.yml");

    assert_eq!(scalar_values(&config, "webrtcIPsFromInterfaces"), ["no"]);
    assert_eq!(
        list_values(&config, "webrtcAdditionalHosts"),
        ["10.44.0.1", "192.168.1.200"]
    );
    assert_eq!(
        scalar_values(&config, "webrtcAddress"),
        ["169.254.218.2:8889"]
    );
    assert_eq!(
        scalar_values(&config, "webrtcLocalUDPAddress"),
        ["169.254.218.2:8189"]
    );
    assert!(config.contains("source: udp+rtp://169.254.218.2:5004"));
}

#[test]
fn mediamtx_service_enters_the_dedicated_network_namespace() {
    let unit = repository_file("deploy/systemd/obscam-mediamtx.service");

    assert_eq!(directive_values(&unit, "DynamicUser"), ["yes"]);
    assert_eq!(
        directive_values(&unit, "NetworkNamespacePath"),
        ["/run/netns/obscam-media"]
    );
    assert_eq!(
        directive_values(&unit, "RestrictAddressFamilies"),
        ["AF_UNIX AF_INET AF_INET6 AF_NETLINK"]
    );
    assert!(unit.contains("Requires=obscam-media-network.service"));
    assert!(!unit.contains("RestrictNetworkInterfaces="));
    assert!(!unit.contains("network-online.target"));
}

#[test]
fn obscam_and_mediamtx_are_independently_supervised() {
    let obscam = repository_file("deploy/systemd/obscam.service");
    let mediamtx = repository_file("deploy/systemd/obscam-mediamtx.service");

    assert_eq!(directive_values(&obscam, "User"), ["obscam"]);
    assert_eq!(directive_values(&obscam, "Restart"), ["always"]);
    assert_eq!(directive_values(&obscam, "RestartSec"), ["5s"]);
    assert_eq!(directive_values(&obscam, "StartLimitIntervalSec"), ["0"]);
    assert!(!obscam.contains("WatchdogSec="));
    assert!(!obscam.contains("obscam-mediamtx.service"));
    assert!(!mediamtx.contains("obscam.service"));

    for unit in [&obscam, &mediamtx] {
        assert_eq!(directive_values(unit, "Restart"), ["always"]);
        assert!(!unit.contains("network-online.target"));
        assert!(!unit.contains("WireGuard"));
        assert!(!unit.contains("remote-fs.target"));
        assert_eq!(directive_values(unit, "StandardOutput"), ["journal"]);
        assert_eq!(directive_values(unit, "StandardError"), ["journal"]);
        assert_eq!(directive_values(unit, "LogRateLimitIntervalSec"), ["30s"]);
        assert!(!directive_values(unit, "LogRateLimitBurst").is_empty());
        assert_eq!(directive_values(unit, "CPUWeight"), ["200"]);
        assert_eq!(directive_values(unit, "IOWeight"), ["200"]);
        assert!(!directive_values(unit, "MemoryHigh").is_empty());
        assert!(!directive_values(unit, "MemoryMax").is_empty());
        assert_eq!(directive_values(unit, "OOMScoreAdjust"), ["-250"]);
    }
}

#[test]
fn appliance_target_is_the_single_operator_lifecycle_unit() {
    let target = repository_file("deploy/systemd/obscam.target");
    let members = [
        repository_file("deploy/systemd/obscam.service"),
        repository_file("deploy/systemd/obscam-mediamtx.service"),
        repository_file("deploy/systemd/obscam-media-network.service"),
    ];

    assert_eq!(
        directive_values(&target, "Wants"),
        ["obscam-media-network.service obscam-mediamtx.service obscam.service"]
    );
    assert_eq!(directive_values(&target, "WantedBy"), ["multi-user.target"]);
    assert!(!target.contains("allsky.service"));

    for member in members {
        assert_eq!(directive_values(&member, "PartOf"), ["obscam.target"]);
    }
}

#[test]
fn operations_document_all_required_read_only_diagnostics() {
    let guide = repository_file("docs/agents/local-startup.md");

    for operation in ["start", "stop", "restart"] {
        assert!(guide.contains(&format!("sudo systemctl {operation} obscam.target")));
    }

    for signal in [
        "systemctl status",
        "journalctl",
        "verify-mediamtx",
        "/api/v1/health",
        "mnt-asiair.automount",
        "mnt-library.automount",
        "wg show wg0",
        "MemoryCurrent",
        "CPUUsageNSec",
        "vcgencmd get_throttled",
    ] {
        assert!(
            guide.contains(signal),
            "missing diagnostic signal: {signal}"
        );
    }
    assert!(!guide.contains("wg show wg1"));
}

#[test]
fn obscam_identity_receives_only_the_camera_and_encoder_devices_it_owns() {
    let sysusers = repository_file("deploy/sysusers.d/obscam.conf");
    let rules = repository_file("deploy/udev/99-z-obscam.rules");
    let unit = repository_file("deploy/systemd/obscam.service");
    let installer = repository_file("deploy/install-appliance");

    assert!(sysusers.contains("u obscam - \"ObsCam camera owner\" /nonexistent /usr/sbin/nologin"));
    assert!(sysusers.contains("g obscam-camera -"));
    assert!(sysusers.contains("m obscam obscam-camera"));
    assert!(!sysusers.contains("obscam-encoder"));

    assert!(rules.contains("ATTR{idVendor}==\"03c3\""));
    assert!(rules.contains("ATTR{idProduct}==\"662b\""));
    assert!(rules.contains("GROUP:=\"obscam-camera\", MODE:=\"0660\""));
    assert!(rules.contains("ATTR{name}==\"bcm2835-codec-encode\""));
    assert!(rules.contains("OWNER:=\"obscam\", GROUP:=\"video\", MODE:=\"0660\""));
    assert!(!rules.contains("ATTR{idVendor}==\"03c3\", GROUP="));

    assert_eq!(
        directive_values(&unit, "SupplementaryGroups"),
        ["obscam-camera"]
    );
    assert_eq!(directive_values(&unit, "DevicePolicy"), ["closed"]);
    assert_eq!(
        directive_values(&unit, "DeviceAllow"),
        ["char-usb_device rw", "char-video4linux rw"]
    );
    assert_eq!(
        directive_values(&unit, "RestrictAddressFamilies"),
        ["AF_UNIX AF_INET AF_INET6 AF_NETLINK"]
    );
    assert!(!unit.contains("PrivateDevices=yes"));
    assert!(installer.contains("/sys/class/video4linux/video*"));
    assert!(installer.contains("chown obscam:video"));
    assert!(!installer.contains("udevadm settle"));
}

#[test]
fn mediamtx_metrics_are_readable_only_on_the_private_media_link() {
    let config = repository_file("deploy/mediamtx.yml");
    let rules = repository_file("deploy/obscam-media.nft");

    assert_eq!(scalar_values(&config, "metrics"), ["yes"]);
    assert_eq!(
        scalar_values(&config, "metricsAddress"),
        ["169.254.218.2:9998"]
    );
    assert!(config.contains("ips: [\"169.254.218.1\"]"));
    assert!(config.contains("- action: metrics"));
    assert!(!config.contains("- action: api"));
    assert!(
        config.find("ips: [\"169.254.218.1\"]") < config.find("ips: []"),
        "the specific private metrics identity must precede the public media identity"
    );
    assert!(!rules.contains("dport 9998"));
    assert!(rules.contains(
        "iifname \"obscam-media0\" ip saddr 169.254.218.2 tcp sport 9998 ct state established accept\n        iifname \"obscam-media0\" drop"
    ));
}

#[test]
fn mediamtx_startup_rejects_binary_or_configuration_drift() {
    let unit = repository_file("deploy/systemd/obscam-mediamtx.service");
    let verifier = repository_file("deploy/verify-mediamtx");
    let binary_checksum = repository_file("deploy/mediamtx-linux-arm64.sha256");
    let config_checksum = repository_file("deploy/mediamtx-config.sha256");

    assert_eq!(
        directive_values(&unit, "ExecStartPre"),
        [
            "/usr/local/libexec/obscam/verify-mediamtx /usr/local/libexec/obscam/mediamtx /etc/obscam/mediamtx.yml /usr/local/share/obscam/mediamtx-linux-arm64.sha256 /usr/local/share/obscam/mediamtx-config.sha256"
        ]
    );
    assert!(verifier.contains("expected_version=v1.19.3"));
    assert!(verifier.contains("MediaMTX configuration must disable MoQ exactly once"));
    assert!(
        binary_checksum
            .contains("b3b2b519420f24a1f262feccdfbee474c8bdedcbf318d5ec4d586f582dfacb00")
    );
    assert!(
        config_checksum
            .contains("91fa713f33ba529ebf07026fd99b75dd0dd9c02d562b1881aa7f9289518e70fa")
    );
}

#[test]
fn media_namespace_contains_only_a_private_point_to_point_link() {
    let setup = repository_file("deploy/setup-media-network");
    let unit = repository_file("deploy/systemd/obscam-media-network.service");

    assert!(setup.contains("ip netns add \"${namespace}\""));
    assert!(
        setup.contains("ip link add \"${host_link}\" type veth peer name \"${namespace_link}\"")
    );
    assert!(setup.contains("ip link set \"${namespace_link}\" netns \"${namespace}\""));
    assert!(setup.contains("ip -n \"${namespace}\" link set lo up"));
    assert!(setup.contains("169.254.218.1/30"));
    assert!(setup.contains("169.254.218.2/30"));
    assert!(!setup.contains("ip address show"));
    assert!(!setup.contains("ip route show"));
    assert_eq!(
        directive_values(&unit, "ExecStart"),
        ["/usr/local/libexec/obscam/setup-media-network start"]
    );
    assert_eq!(
        directive_values(&unit, "CapabilityBoundingSet"),
        ["CAP_NET_ADMIN CAP_SYS_ADMIN"]
    );
    assert_eq!(directive_values(&unit, "Restart"), ["on-failure"]);
    assert_eq!(directive_values(&unit, "RestartSec"), ["5s"]);
    assert_eq!(directive_values(&unit, "StartLimitIntervalSec"), ["0"]);
    assert_eq!(directive_values(&unit, "StandardOutput"), ["journal"]);
    assert_eq!(directive_values(&unit, "StandardError"), ["journal"]);
    assert_eq!(directive_values(&unit, "LogRateLimitIntervalSec"), ["30s"]);
    assert!(!unit.contains("ProtectSystem="));
    assert!(!unit.contains("ProtectHome="));
}

#[test]
fn media_forwarding_is_limited_to_approved_addresses_and_ports() {
    let rules = repository_file("deploy/obscam-media.nft");

    assert!(rules.contains("10.44.0.1, 192.168.1.200"));
    assert!(rules.contains("tcp dport 8889"));
    assert!(rules.contains("udp dport 8189"));
    assert!(rules.contains("dnat ip to 169.254.218.2"));
    assert!(rules.contains("iifname \"obscam-media0\""));
    assert!(rules.contains("oifname \"obscam-media0\""));
    assert!(rules.contains("10.44.0.0/24, 192.168.1.0/24"));
    assert!(rules.contains("ct state established,related accept"));
    assert!(rules.contains("ct status dnat"));
    assert!(rules.matches("drop").count() >= 3);
    assert!(!rules.contains("0.0.0.0"));
}

fn scalar_values<'a>(document: &'a str, key: &str) -> Vec<&'a str> {
    let prefix = format!("{key}:");
    document
        .lines()
        .filter_map(|line| line.strip_prefix(&prefix).map(str::trim))
        .collect()
}

fn list_values<'a>(document: &'a str, key: &str) -> Vec<&'a str> {
    let heading = format!("{key}:");
    document
        .lines()
        .skip_while(|line| *line != heading)
        .skip(1)
        .map_while(|line| line.strip_prefix("  - "))
        .collect()
}

fn directive_values<'a>(unit: &'a str, directive: &str) -> Vec<&'a str> {
    let prefix = format!("{directive}=");
    unit.lines()
        .filter_map(|line| line.strip_prefix(&prefix))
        .collect()
}
