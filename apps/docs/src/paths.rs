//! Jail paths, as this app handles them: always absolute, `/`-separated, no trailing slash
//! except the root itself.

/// The folder a path is in. The root is its own parent.
pub fn parent(path: &str) -> String {
    match path.trim_end_matches('/').rfind('/') {
        Some(0) | None => "/".to_owned(),
        Some(i) => path[..i].to_owned(),
    }
}

/// The last component: a file or folder name.
pub fn name(path: &str) -> &str {
    let path = path.trim_end_matches('/');
    path.rsplit('/').next().unwrap_or(path)
}

/// `dir` joined with `name`.
pub fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

/// Where a link in the document at `from` points, as an absolute path: `/x` is from the
/// root, anything else is relative to the document's folder, and `.` and `..` are applied.
/// A `#fragment` is dropped. `None` for a link to nothing but a fragment.
pub fn resolve(from: &str, target: &str) -> Option<String> {
    let target = target.split('#').next().unwrap_or("");
    if target.is_empty() {
        return None;
    }
    let base = if target.starts_with('/') {
        String::new()
    } else {
        parent(from)
    };
    let mut parts: Vec<&str> = base.split('/').filter(|s| !s.is_empty()).collect();
    for seg in target.split('/') {
        match seg {
            "" | "." => {}
            // Climbing past the root stays at the root, as it does on the server.
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    Some(format!("/{}", parts.join("/")))
}

/// Whether a path names a Markdown document, which this app opens itself. Anything else is
/// handed to the browser as a download.
pub fn is_doc(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".md") || lower.ends_with(".markdown")
}

/// A document's name as a title: `getting-started.md` reads as `getting-started`.
pub fn title(path: &str) -> &str {
    let n = name(path);
    n.strip_suffix(".md")
        .or_else(|| n.strip_suffix(".markdown"))
        .unwrap_or(n)
}

/// A name the user typed for a new document or folder, if it is usable: not empty, not a
/// path, not hidden. A document gets `.md` added when it has no extension of its own.
pub fn new_name(typed: &str, doc: bool) -> Option<String> {
    let typed = typed.trim();
    if typed.is_empty()
        || typed.starts_with('.')
        || typed.contains('/')
        || typed.contains('\\')
        || typed.len() > 120
    {
        return None;
    }
    if doc && !is_doc(typed) {
        Some(format!("{typed}.md"))
    } else {
        Some(typed.to_owned())
    }
}

/// `/files/...`, each segment percent-encoded, for handing a file to the browser.
pub fn download_url(path: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut url = String::from("/files");
    for segment in path.split('/').filter(|s| !s.is_empty()) {
        url.push('/');
        for &b in segment.as_bytes() {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
                url.push(b as char);
            } else {
                url.push('%');
                url.push(HEX[usize::from(b >> 4)] as char);
                url.push(HEX[usize::from(b & 0xF)] as char);
            }
        }
    }
    url
}
