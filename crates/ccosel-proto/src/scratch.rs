//! Temporary project folders: somewhere to upload a project to build without keeping it.
//!
//! The server makes one on `POST /scratch`, answering with its id, and deletes it once it has
//! gone unused for [`TTL_SECS`], or when the server restarts. Everything else treats it as an
//! ordinary jail path, [`path`], so building and downloading need nothing new.

use alloc::string::String;

/// Top-level jail directory holding every scratch folder. Hidden from listings of `/`.
pub const DIR: &str = ".scratch";

/// How long a scratch folder survives without being uploaded to, built, listed or downloaded
/// from.
pub const TTL_SECS: u64 = 60 * 60;

/// The jail path of scratch folder `id`, e.g. `/.scratch/1234`.
pub fn path(id: u32) -> String {
    let mut s = String::from("/");
    s.push_str(DIR);
    s.push('/');
    // Decimal by hand: `format!` pulls float formatting into guests that call this.
    let mut digits = [0u8; 10];
    let mut n = id;
    let mut i = digits.len();
    loop {
        i -= 1;
        digits[i] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    for &d in &digits[i..] {
        s.push(d as char);
    }
    s
}

/// The scratch id a jail path is inside, if any: `/.scratch/12/x/y` is `Some(12)`.
pub fn id_of(path: &str) -> Option<u32> {
    let rest = path
        .trim_start_matches('/')
        .strip_prefix(DIR)?
        .strip_prefix('/')?;
    rest.split('/').next()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_and_id_of_round_trip() {
        for id in [0, 7, 1234, u32::MAX] {
            assert_eq!(id_of(&path(id)), Some(id));
        }
        assert_eq!(path(42), "/.scratch/42");
        assert_eq!(id_of("/.scratch/42/proj/src/main.rs"), Some(42));
        assert_eq!(id_of(".scratch/42"), Some(42));
    }

    #[test]
    fn other_paths_have_no_id() {
        for p in [
            "/",
            "/.scratch",
            "/.scratch/",
            "/.scratch/x",
            "/a/.scratch/1",
            "/.scratchy/1",
        ] {
            assert_eq!(id_of(p), None, "{p}");
        }
    }
}
