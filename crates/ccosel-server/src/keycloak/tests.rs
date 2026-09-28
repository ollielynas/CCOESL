//! Only the parts that need no Docker: what the container (or child process) is started
//! with, and which addresses Keycloak may send a browser back to.

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

#[test]
fn the_admin_password_is_kept_per_user_not_per_checkout() {
    let env = |vars: &'static [(&'static str, &'static str)]| {
        move |name: &str| {
            vars.iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| PathBuf::from(v))
        }
    };
    if cfg!(windows) {
        assert_eq!(
            password_file_in(env(&[("APPDATA", r"C:\Users\u\AppData\Roaming")])),
            Path::new(r"C:\Users\u\AppData\Roaming\ccosel\keycloak-admin-password")
        );
    } else {
        assert_eq!(
            password_file_in(env(&[("HOME", "/home/u")])),
            Path::new("/home/u/.local/share/ccosel/keycloak-admin-password")
        );
        assert_eq!(
            password_file_in(env(&[("HOME", "/home/u"), ("XDG_DATA_HOME", "/data")])),
            Path::new("/data/ccosel/keycloak-admin-password")
        );
    }
    // With nowhere per-user to put it, the working directory is still better than failing.
    assert_eq!(password_file_in(env(&[])), Path::new(LEGACY_PASSWORD_FILE));
}

#[test]
fn a_container_without_its_port_published_is_not_reused() {
    assert!(is_published("127.0.0.1:8080\n"));
    // `docker port` prints nothing for a container that came up with no network.
    assert!(!is_published(""));
    // Published somewhere else is no use either: the server only looks on 127.0.0.1:8080.
    assert!(!is_published("0.0.0.0:8080\n[::]:8080\n"));
    assert!(!is_published("127.0.0.1:18080\n"));
}

#[test]
fn an_existing_password_file_is_read_and_left_alone() {
    let dir = std::env::temp_dir().join(format!("ccosel-kc-{}", std::process::id()));
    let file = dir.join("ccosel").join("keycloak-admin-password");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, "s3cret\n").unwrap();
    assert_eq!(admin_password(&file).unwrap(), "s3cret");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_new_password_file_is_created_private() {
    let dir = std::env::temp_dir().join(format!("ccosel-kc-new-{}", std::process::id()));
    let file = dir.join("ccosel").join("keycloak-admin-password");
    let password = admin_password(&file).unwrap();
    assert_eq!(password.len(), 24);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), password);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&file).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    // Asking again gives the same password rather than a new one.
    assert_eq!(admin_password(&file).unwrap(), password);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_child_process_keycloak_has_the_containers_settings_and_listens_where_told() {
    let args = local_args("127.0.0.1");
    // Everything `run_args` gives Keycloak itself, after the image name.
    let container = run_args();
    let image = container.iter().position(|a| a == IMAGE).unwrap();
    for arg in &container[image + 1..] {
        assert!(args.contains(arg), "missing {arg}");
    }
    assert!(args.contains(&"--http-host=127.0.0.1".to_owned()));
    assert!(args.contains(&format!("--http-port={PORT}")));
    assert!(local_args("0.0.0.0").contains(&"--http-host=0.0.0.0".to_owned()));
    // And never the password: it goes in the environment.
    assert!(!args.iter().any(|a| a.contains("PASSWORD")));
}
