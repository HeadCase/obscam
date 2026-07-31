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
}

#[test]
fn mediamtx_service_can_use_only_obscam_network_interfaces() {
    let unit = repository_file("deploy/systemd/obscam-mediamtx.service");

    assert_eq!(directive_values(&unit, "DynamicUser"), ["yes"]);
    assert_eq!(
        directive_values(&unit, "RestrictNetworkInterfaces"),
        ["lo eth0 wg0"]
    );
    assert!(!unit.contains("network-online.target"));
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
