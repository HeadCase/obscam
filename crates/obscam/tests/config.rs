use obscam::{CameraSourceSelection, Config, ConfigError};

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
fn production_camera_is_the_only_implicit_source() {
    let config = Config::parse("127.0.0.1:8080", "8889", "/obscam/whep")
        .expect("default production configuration");

    assert_eq!(config.camera_source(), CameraSourceSelection::Production);
}

#[cfg(not(feature = "camera-substitute"))]
#[test]
fn production_build_rejects_the_deterministic_source() {
    let error =
        Config::parse_with_camera_source("127.0.0.1:8080", "8889", "/obscam/whep", "deterministic")
            .expect_err("production builds must not contain the substitute");

    assert_eq!(error, ConfigError::CameraSubstituteUnavailable);
}

#[cfg(feature = "camera-substitute")]
#[test]
fn acceptance_build_requires_explicit_deterministic_selection() {
    let config =
        Config::parse_with_camera_source("127.0.0.1:8080", "8889", "/obscam/whep", "deterministic")
            .expect("acceptance build contains the explicitly selected substitute");

    assert_eq!(config.camera_source(), CameraSourceSelection::Deterministic);
}

#[test]
fn rejects_unknown_camera_source_names() {
    let error =
        Config::parse_with_camera_source("127.0.0.1:8080", "8889", "/obscam/whep", "automatic")
            .expect_err("source selection must never fall back silently");

    assert_eq!(error, ConfigError::InvalidCameraSource);
}
