//! Only the parts that need no Docker: what the container is started with, and which
//! addresses Keycloak may send a browser back to.

use super::*;

#[test]
fn the_container_listens_on_this_machine_only() {
    let args = run_args();
    let published: Vec<_> = args
        .windows(2)
        .filter(|w| w[0] == "-p")
        .map(|w| w[1].as_str())
        .collect();
    assert_eq!(published, ["127.0.0.1:8080:8080"]);
}

#[test]
fn the_container_keeps_its_data_and_serves_under_idp() {
    let args = run_args();
    assert!(args.contains(&format!("{VOLUME}:/opt/keycloak/data")));
    assert!(args.contains(&"--http-relative-path=/idp".to_owned()));
    assert!(args.contains(&"--proxy-headers=xforwarded".to_owned()));
    assert!(args.contains(&format!("{CONFIG_LABEL}={CONFIG_VERSION}")));
}

#[test]
fn the_admin_password_is_never_on_the_command_line() {
    // `-e NAME` with no value makes docker read it from its own environment.
    for arg in run_args() {
        assert!(!arg.starts_with("KC_BOOTSTRAP_ADMIN_PASSWORD="), "{arg}");
        assert!(!arg.starts_with("KEYCLOAK_ADMIN_PASSWORD="), "{arg}");
    }
}

#[test]
fn redirect_uris_cover_this_machine_and_the_public_url() {
    assert_eq!(
        redirect_uris(None, 8777),
        [
            "http://localhost:8777/auth/callback",
            "http://127.0.0.1:8777/auth/callback",
        ]
    );
    assert_eq!(
        redirect_uris(Some("https://ccosel.example.com/"), 9000)
            .last()
            .unwrap(),
        "https://ccosel.example.com/auth/callback"
    );
}

#[test]
fn recognises_what_older_versions_wrote_to_env() {
    assert!(is_legacy_url("http://localhost:8080/realms/ccosel"));
    assert!(is_legacy_url("http://localhost:8080/realms/ccosel/"));
    assert!(!is_legacy_url("https://sso.example.com/realms/ccosel"));
}

#[test]
fn urls_point_at_the_idp_path() {
    assert_eq!(realm_url(), "http://127.0.0.1:8080/idp/realms/ccosel");
    assert_eq!(admin_console_url(), "http://localhost:8080/idp/admin/");
}
