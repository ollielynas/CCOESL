//! Which files of a folder `.gitignore` excludes, so a project upload skips `target/`,
//! `node_modules/` and the like instead of sending gigabytes of build output to be rebuilt.
//!
//! Pure and dependency-free: the shell runs it in the browser on the list of picked files, and
//! `tests/against_git.rs` checks it against real `git` on the same tree.
//!
//! Follows gitignore(5): `.gitignore` files at any depth, patterns relative to their own
//! directory, deeper files and later lines winning, `!` to re-include, a trailing `/` for
//! directories only, a leading or middle `/` to anchor, `*` `?` `[...]` and `**`. As in git, a
//! file inside an ignored directory stays ignored whatever a later `!` says, because git never
//! looks inside that directory. Not covered: `.git/info/exclude` and `core.excludesFile`, which
//! live outside the project, and case-insensitive matching.

#![forbid(unsafe_code)]

/// Every rule from every `.gitignore` added so far.
#[derive(Debug, Default)]
pub struct Filter {
    rules: Vec<Rule>,
}

#[derive(Debug)]
struct Rule {
    /// Directory holding the `.gitignore` this came from, relative to the root, no trailing
    /// slash; empty for the root itself.
    base: String,
    negate: bool,
    dir_only: bool,
    /// Matched against the path relative to `base` rather than just the last component.
    anchored: bool,
    glob: Vec<Token>,
}

impl Filter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add the patterns of the `.gitignore` in directory `base` (relative to the root, `""`
    /// for the root). Add shallower files first: that is the precedence git gives them.
    pub fn add(&mut self, base: &str, contents: &str) {
        let base = base.trim_matches('/').to_owned();
        for line in contents.lines() {
            if let Some(rule) = parse(&base, line) {
                self.rules.push(rule);
            }
        }
    }

    /// Whether the file at `path` (relative to the root, `/`-separated) is ignored: itself, or
    /// because a directory it's in is.
    pub fn is_ignored(&self, path: &str) -> bool {
        let path = path.trim_matches('/');
        let mut end = 0;
        // Each ancestor directory first, shallowest first, as git walks the tree.
        while let Some(i) = path[end..].find('/') {
            end += i;
            if self.decide(&path[..end], true) {
                return true;
            }
            end += 1;
        }
        self.decide(path, false)
    }

    /// The last matching rule wins. Rules are stored shallow file first, lines in order, so
    /// "last" is exactly git's precedence.
    fn decide(&self, path: &str, is_dir: bool) -> bool {
        let mut ignored = false;
        for rule in &self.rules {
            if rule.dir_only && !is_dir {
                continue;
            }
            let Some(rel) = relative_to(path, &rule.base) else {
                continue;
            };
            let subject = if rule.anchored {
                rel
            } else {
                rel.rsplit('/').next().unwrap_or(rel)
            };
            if matches(&rule.glob, subject.as_bytes()) {
                ignored = !rule.negate;
            }
        }
        ignored
    }
}

/// `path` relative to `base`, if it's strictly inside it.
fn relative_to<'a>(path: &'a str, base: &str) -> Option<&'a str> {
    if base.is_empty() {
        return Some(path);
    }
    path.strip_prefix(base)?.strip_prefix('/')
}

fn parse(base: &str, line: &str) -> Option<Rule> {
    let line = line.strip_suffix('\r').unwrap_or(line);
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let mut pat = trim_trailing_spaces(line);
    let negate = pat.starts_with('!');
    if negate {
        pat = &pat[1..];
    }
    // `\#` and `\!` at the start are literal; the glob's own escape handling takes care of
    // them, since `\` before any character means that character.
    let dir_only = pat.ends_with('/') && !pat.ends_with("\\/");
    if dir_only {
        pat = &pat[..pat.len() - 1];
    }
    if pat.is_empty() {
        return None;
    }
    let anchored = pat.contains('/');
    let pat = pat.strip_prefix('/').unwrap_or(pat);
    Some(Rule {
        base: base.to_owned(),
        negate,
        dir_only,
        anchored,
        glob: tokenize(pat.as_bytes()),
    })
}

/// Trailing spaces are dropped unless the last one is escaped with a backslash.
fn trim_trailing_spaces(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut end = bytes.len();
    while end > 0 && bytes[end - 1] == b' ' {
        if end >= 2 && bytes[end - 2] == b'\\' {
            break;
        }
        end -= 1;
    }
    &line[..end]
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Byte(u8),
    /// `?`: one byte, not `/`.
    Any,
    /// `*`: any run of bytes without `/`.
    Star,
    /// `**/` at the start or `/**/` in the middle (the leading `/` stays a `Byte`): zero or
    /// more whole directories.
    Dirs,
    /// `/**` at the end (again, the `/` stays a `Byte`): anything at all.
    Rest,
    /// `[...]`: one byte, not `/`, in (or with `negated`, not in) the ranges.
    Class {
        negated: bool,
        ranges: Vec<(u8, u8)>,
    },
}

fn tokenize(p: &[u8]) -> Vec<Token> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < p.len() {
        match p[i] {
            b'\\' if i + 1 < p.len() => {
                out.push(Token::Byte(p[i + 1]));
                i += 2;
            }
            b'*' if p.get(i + 1) == Some(&b'*') => {
                let at_segment_start = i == 0 || p[i - 1] == b'/';
                let then = p.get(i + 2);
                if at_segment_start && then == Some(&b'/') {
                    out.push(Token::Dirs);
                    i += 3;
                } else if at_segment_start && then.is_none() {
                    out.push(Token::Rest);
                    i += 2;
                } else {
                    // Anywhere else `**` is just two `*`s.
                    out.push(Token::Star);
                    i += 2;
                }
            }
            b'*' => {
                out.push(Token::Star);
                i += 1;
            }
            b'?' => {
                out.push(Token::Any);
                i += 1;
            }
            b'[' => match class(&p[i..]) {
                Some((tok, len)) => {
                    out.push(tok);
                    i += len;
                }
                None => {
                    out.push(Token::Byte(b'['));
                    i += 1;
                }
            },
            c => {
                out.push(Token::Byte(c));
                i += 1;
            }
        }
    }
    out
}

/// Parse `[...]` at the start of `p`, returning the token and its length. `None` if it never
/// closes, in which case the `[` is literal.
fn class(p: &[u8]) -> Option<(Token, usize)> {
    let mut i = 1;
    let negated = matches!(p.get(i), Some(b'!') | Some(b'^'));
    if negated {
        i += 1;
    }
    let mut ranges = Vec::new();
    let mut first = true;
    loop {
        let mut c = *p.get(i)?;
        if c == b']' && !first {
            return Some((Token::Class { negated, ranges }, i + 1));
        }
        first = false;
        if c == b'\\' {
            i += 1;
            c = *p.get(i)?;
        }
        if p.get(i + 1) == Some(&b'-') && p.get(i + 2).is_some_and(|&e| e != b']') {
            let mut hi = p[i + 2];
            let mut step = 3;
            if hi == b'\\' {
                hi = *p.get(i + 3)?;
                step = 4;
            }
            ranges.push((c, hi));
            i += step;
        } else {
            ranges.push((c, c));
            i += 1;
        }
    }
}

fn matches(glob: &[Token], s: &[u8]) -> bool {
    let Some((tok, rest)) = glob.split_first() else {
        return s.is_empty();
    };
    match tok {
        Token::Byte(b) => s.first() == Some(b) && matches(rest, &s[1..]),
        Token::Any => s.first().is_some_and(|&c| c != b'/') && matches(rest, &s[1..]),
        Token::Class { negated, ranges } => {
            s.first().is_some_and(|&c| {
                c != b'/' && ranges.iter().any(|&(lo, hi)| lo <= c && c <= hi) != *negated
            }) && matches(rest, &s[1..])
        }
        Token::Star => {
            // Try every split that doesn't cross a `/`.
            let limit = s.iter().position(|&c| c == b'/').unwrap_or(s.len());
            (0..=limit).any(|n| matches(rest, &s[n..]))
        }
        Token::Dirs => {
            // Zero directories, or skip one whole `dir/` and try again.
            matches(rest, s)
                || s.iter()
                    .position(|&c| c == b'/')
                    .is_some_and(|i| matches(glob, &s[i + 1..]))
        }
        Token::Rest => true,
    }
}

#[cfg(test)]
mod tests;
