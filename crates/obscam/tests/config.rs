use obscam::{Config, ConfigError};

#[test]
fn accepts_local_bind_and_hostless_whep_descriptor() {
    let config =
        Config::parse("127.0.0.1:8080", "8889", "/obscam/whep").expect("valid local configuration");

    assert_eq!(config.bind_address().to_string(), "127.0.0.1:8080");
    assert_eq!(config.whep_port(), 8889);
    assert_eq!(config.whep_path(), "/obscam/whep");
}

#[test]
fn rejects_whep_descriptor_with_a_host() {
    let error = Config::parse(
        "127.0.0.1:8080",
        "8889",
        "http://camera.example/obscam/whep",
    )
    .expect_err("a runtime media descriptor must be hostless");

    assert_eq!(error, ConfigError::InvalidWhepPath);
}

#[test]
fn rejects_relative_whep_path() {
    let error = Config::parse("127.0.0.1:8080", "8889", "obscam/whep")
        .expect_err("a WHEP path must start at the origin root");

    assert_eq!(error, ConfigError::InvalidWhepPath);
}

#[test]
fn rejects_zero_whep_port() {
    let error = Config::parse("127.0.0.1:8080", "0", "/obscam/whep")
        .expect_err("port zero is not a usable media endpoint");

    assert_eq!(error, ConfigError::InvalidWhepPort);
}

#[test]
fn rejects_whep_path_that_exceeds_the_runtime_bound() {
    let path = format!("/{}", "a".repeat(256));
    let error = Config::parse("127.0.0.1:8080", "8889", &path)
        .expect_err("runtime configuration must have a fixed upper bound");

    assert_eq!(error, ConfigError::WhepPathTooLong);
}
