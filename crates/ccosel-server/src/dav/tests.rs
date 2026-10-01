//! The pieces of `dav` that need no socket. The routes themselves, permissions included, are
//! tested end to end in `tests/dav_http.rs`.

use super::*;

fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
    let mut h = HeaderMap::new();
    for (k, v) in pairs {
        h.insert(*k, HeaderValue::from_str(v).unwrap());
    }
    h
}

fn basic(login_password: &str) -> String {
    format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(login_password)
    )
}

#[test]
fn request_paths_become_jail_paths() {
    assert_eq!(jail_path("/dav").as_deref(), Some(""));
    assert_eq!(jail_path("/dav/").as_deref(), Some(""));
    assert_eq!(
        jail_path("/dav/Docs/a%20b.md").as_deref(),
        Some("Docs/a b.md")
    );
    assert_eq!(jail_path("/dav/Docs/").as_deref(), Some("Docs"));
    // `..` is resolved before the prefix is checked, so it cannot climb out of `/dav`.
    assert_eq!(jail_path("/dav/a/../b").as_deref(), Some("b"));
    assert_eq!(jail_path("/dav/../etc/passwd"), None);
    assert_eq!(jail_path("/files/a"), None);
    assert_eq!(jail_path("/davish/a"), None);
}

#[test]
fn basic_credentials_are_read_from_the_header() {
    let h = headers(&[("authorization", &basic("alice:pa:ss"))]);
    assert_eq!(
        basic_credentials(&h),
        Some(("alice".to_owned(), "pa:ss".to_owned()))
    );
    let lower = format!("basic {}", &basic("bob:x")[6..]);
    assert_eq!(
        basic_credentials(&headers(&[("authorization", &lower)])),
        Some(("bob".to_owned(), "x".to_owned()))
    );
    for bad in ["Bearer abc", "Basic !!!", "Basic", &basic("no-colon")] {
        assert_eq!(
            basic_credentials(&headers(&[("authorization", bad)])),
            None,
            "{bad}"
        );
    }
    assert_eq!(basic_credentials(&HeaderMap::new()), None);
}

#[test]
fn only_an_explicit_infinite_depth_is_infinite() {
    assert!(is_infinite_depth(&headers(&[("depth", "infinity")])));
    assert!(is_infinite_depth(&headers(&[("depth", " Infinity ")])));
    assert!(!is_infinite_depth(&headers(&[("depth", "1")])));
    assert!(!is_infinite_depth(&HeaderMap::new()));
}

#[test]
fn declared_length_is_read_when_it_is_a_number() {
    assert_eq!(
        declared_length(&headers(&[("content-length", "12")])),
        Some(12)
    );
    assert_eq!(
        declared_length(&headers(&[("content-length", "lots")])),
        None
    );
    assert_eq!(declared_length(&HeaderMap::new()), None);
}

#[test]
fn refusals_map_to_statuses_and_fs_errors() {
    assert_eq!(status_of(server_error::NOT_FOUND), StatusCode::NOT_FOUND);
    assert_eq!(status_of(server_error::DENIED), StatusCode::FORBIDDEN);
    assert_eq!(
        status_of(server_error::EXISTS),
        StatusCode::PRECONDITION_FAILED
    );
    assert_eq!(
        status_of(server_error::IO),
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(fs_error(server_error::NOT_FOUND), FsError::NotFound);
    assert_eq!(fs_error(server_error::DENIED), FsError::Forbidden);
    assert_eq!(fs_error(server_error::NOT_A_DIRECTORY), FsError::Forbidden);
    assert_eq!(fs_error(server_error::EXISTS), FsError::Exists);
    assert_eq!(fs_error(server_error::TOO_LARGE), FsError::TooLarge);
    assert_eq!(fs_error(server_error::IO), FsError::GeneralFailure);
    use std::io::{Error, ErrorKind};
    assert_eq!(
        io_error(Error::from(ErrorKind::NotFound)),
        FsError::NotFound
    );
    assert_eq!(
        io_error(Error::from(ErrorKind::PermissionDenied)),
        FsError::Forbidden
    );
    assert_eq!(
        io_error(Error::from(ErrorKind::AlreadyExists)),
        FsError::Exists
    );
    assert_eq!(
        io_error(Error::from(ErrorKind::Other)),
        FsError::GeneralFailure
    );
}

fn temp_file() -> std::fs::File {
    use std::sync::atomic::{AtomicU32, Ordering};
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let path = std::env::temp_dir().join(format!(
        "ccosel-dav-file-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
        .unwrap()
}

#[tokio::test]
async fn writes_stop_at_the_upload_cap_however_they_arrive() {
    let cap = MAX_UPLOAD_BYTES as u64;
    let mut file = File::new(temp_file(), cap - 3);
    file.write_bytes(Bytes::from_static(b"abc")).await.unwrap();
    assert_eq!(
        file.write_bytes(Bytes::from_static(b"d")).await,
        Err(FsError::TooLarge)
    );

    let mut file = File::new(temp_file(), cap - 1);
    let buf: Box<dyn Buf + Send> = Box::new(Bytes::from_static(b"de"));
    assert_eq!(file.write_buf(buf).await, Err(FsError::TooLarge));
}

#[tokio::test]
async fn a_file_reads_back_what_was_written() {
    let mut file = File::new(temp_file(), 0);
    let buf: Box<dyn Buf + Send> = Box::new(Bytes::from_static(b"hello world"));
    file.write_buf(buf).await.unwrap();
    file.flush().await.unwrap();
    assert_eq!(file.metadata().await.unwrap().len(), 11);
    assert_eq!(file.seek(SeekFrom::Start(6)).await.unwrap(), 6);
    assert_eq!(&file.read_bytes(100).await.unwrap()[..], b"world");
    assert!(file.read_bytes(100).await.unwrap().is_empty());
}

#[test]
fn failed_sign_ins_count_against_the_tunnels_client_only_for_local_connections() {
    let lan: IpAddr = "192.168.1.20".parse().unwrap();
    let local: IpAddr = "127.0.0.1".parse().unwrap();
    let forwarded = headers(&[("x-forwarded-for", "10.0.0.1, 203.0.113.7")]);
    // From the tunnel on this machine: the client it forwarded for, as it added last.
    assert_eq!(
        client_ip(local, &forwarded),
        "203.0.113.7".parse::<IpAddr>().unwrap()
    );
    assert_eq!(
        client_ip(
            "::1".parse().unwrap(),
            &headers(&[("x-forwarded-for", "2001:db8::1")])
        ),
        "2001:db8::1".parse::<IpAddr>().unwrap()
    );
    // From anywhere else the header is the client's own word, so it counts for nothing.
    assert_eq!(client_ip(lan, &forwarded), lan);
    assert_eq!(client_ip(local, &HeaderMap::new()), local);
    assert_eq!(
        client_ip(local, &headers(&[("x-forwarded-for", "nonsense")])),
        local
    );
}
