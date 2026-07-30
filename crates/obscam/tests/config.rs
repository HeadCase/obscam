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

#[test]
fn deterministic_camera_requires_an_explicit_configuration_value() {
    let production = Config::parse("127.0.0.1:8080", "8889", "/obscam/whep")
        .expect("default production configuration");
    let deterministic =
        Config::parse_with_camera_source("127.0.0.1:8080", "8889", "/obscam/whep", "deterministic")
            .expect("explicit acceptance configuration");

    assert_eq!(
        production.camera_source(),
        obscam::CameraSourceKind::Production
    );
    assert_eq!(
        deterministic.camera_source(),
        obscam::CameraSourceKind::Deterministic
    );
}

#[test]
fn unknown_camera_source_fails_closed() {
    let error =
        Config::parse_with_camera_source("127.0.0.1:8080", "8889", "/obscam/whep", "automatic")
            .expect_err("camera boundary cannot be selected silently");

    assert_eq!(error, ConfigError::InvalidCameraSource);
}

#[test]
fn restart_defaults_are_500_ms_gain_100_monochrome() {
    let config = Config::parse("127.0.0.1:8080", "8889", "/obscam/whep").expect("config");

    assert_eq!(config.default_settings().exposure_ms(), 500);
    assert_eq!(config.default_settings().gain(), 100);
    assert_eq!(config.default_settings().treatment().as_str(), "monochrome");
}

#[test]
fn explicit_restart_defaults_are_validated_as_a_complete_tuple() {
    let config = Config::parse_with_defaults(
        "127.0.0.1:8080",
        "8889",
        "/obscam/whep",
        "production",
        "10000",
        "350",
        "colour",
    )
    .expect("valid defaults");
    assert_eq!(config.default_settings().exposure_ms(), 10_000);
    assert_eq!(config.default_settings().gain(), 350);

    assert!(matches!(
        Config::parse_with_defaults(
            "127.0.0.1:8080",
            "8889",
            "/obscam/whep",
            "production",
            "30",
            "100",
            "monochrome",
        ),
        Err(ConfigError::InvalidDefaultSettings)
    ));
}
