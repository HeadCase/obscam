use std::{fs, path::PathBuf};

fn repository_file(path: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path);
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}

#[test]
fn mediamtx_advertises_only_permitted_browser_hosts() {
    let config = repository_file("deploy/mediamtx.yml");

    assert_eq!(scalar_values(&config, "webrtcIPsFromInterfaces"), ["no"]);
    assert_eq!(
        list_values(&config, "webrtcAdditionalHosts"),
        ["10.164.190.1", "192.168.1.200"]
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
    assert!(!unit.contains("ProtectSystem="));
    assert!(!unit.contains("ProtectHome="));
}

#[test]
fn media_forwarding_is_limited_to_approved_addresses_and_ports() {
    let rules = repository_file("deploy/obscam-media.nft");

    assert!(rules.contains("10.164.190.1, 192.168.1.200"));
    assert!(rules.contains("tcp dport 8889"));
    assert!(rules.contains("udp dport 8189"));
    assert!(rules.contains("dnat ip to 169.254.218.2"));
    assert!(rules.contains("iifname \"obscam-media0\""));
    assert!(rules.contains("oifname \"obscam-media0\""));
    assert!(rules.contains("10.164.190.0/24, 192.168.1.0/24"));
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
