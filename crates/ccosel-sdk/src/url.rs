//! Building URLs to this server without a URL crate: percent-encoding is a dozen lines, and a
//! crate for it would be paid for in every app that links one.

use alloc::string::String;

const HEX: &[u8; 16] = b"0123456789ABCDEF";

/// Appends `s` percent-encoded, keeping only the characters that never need it. With
/// `keep_slash`, `/` stays as it is, for a path.
fn push_encoded(out: &mut String, s: &str, keep_slash: bool) {
    for &b in s.as_bytes() {
        if b.is_ascii_alphanumeric()
            || matches!(b, b'-' | b'.' | b'_' | b'~')
            || (keep_slash && b == b'/')
        {
            out.push(b as char);
        } else {
            out.push('%');
            out.push(HEX[usize::from(b >> 4)] as char);
            out.push(HEX[usize::from(b & 0xF)] as char);
        }
    }
}

/// `s` percent-encoded as one URL component: `/` included, so a whole path fits in a query
/// parameter.
pub fn encode_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    push_encoded(&mut out, s, false);
    out
}

/// Where a server file's bytes are: `/files/<path>`, each segment encoded, so a name like
/// `notes #1.md` asks for that file rather than cutting the URL short at the `#`.
pub fn file_url(path: &str) -> String {
    let mut out = String::from("/files");
    if !path.starts_with('/') {
        out.push('/');
    }
    push_encoded(&mut out, path, true);
    out
}

/// [`file_url`] for showing in the page rather than downloading: the server then sends the
/// file's own type, and supports seeking, which audio and video need.
pub fn inline_file_url(path: &str) -> String {
    let mut out = file_url(path);
    out.push_str("?inline=1");
    out
}

/// A link to an app's own page, opened on `arg`: `/app/<app>?open=<arg>`. Pass it to
/// [`Ui::copy_link`](crate::Ui::copy_link) to share it.
pub fn app_link(app: &str, arg: &str) -> String {
    let mut out = String::from("/app/");
    out.push_str(app);
    out.push_str("?open=");
    push_encoded(&mut out, arg, false);
    out
}
