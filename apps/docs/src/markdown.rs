//! A small Markdown parser, written for this app rather than pulled in.
//!
//! A general-purpose Markdown crate is tens of kilobytes of wasm, every byte of which every
//! user downloads again for this app over a slow LAN. Documentation needs a known subset, so
//! this handles that subset in one pass per line and turns everything else into plain text,
//! which is always a readable fallback for Markdown.
//!
//! Blocks: ATX (`#`) and setext (`===`/`---`) headings, paragraphs, `-`/`*`/`+` and numbered
//! lists with nesting and `[ ]`/`[x]` tasks, `>` quotes, fenced code, rules and pipe tables.
//! Inline: `**strong**`, `*em*`/`_em_`, `` `code` ``, `~~strike~~`, `[links](target)`,
//! `![images](src)`, `<autolinks>`, backslash escapes, and `:icon_name:` shortcodes for any
//! [Phosphor](https://phosphoricons.com) icon (see `phosphor.rs`), the way Discord does emoji.

/// Inline style bits. Kept apart from the ABI's `TextStyle` so the parser is plain data that
/// tests can compare, and the renderer decides how each maps onto the screen.
pub const STRONG: u8 = 1;
pub const ITALIC: u8 = 1 << 1;
pub const CODE: u8 = 1 << 2;
pub const STRIKE: u8 = 1 << 3;

/// A run of text with one style, and where it links to if anywhere.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inline {
    pub text: String,
    pub style: u8,
    pub link: Option<String>,
}

impl Inline {
    pub fn plain(text: &str) -> Self {
        Self {
            text: text.to_owned(),
            style: 0,
            link: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    Heading {
        level: u8,
        inlines: Vec<Inline>,
    },
    Paragraph(Vec<Inline>),
    Item {
        /// Nesting, 0 for a top-level item.
        depth: u8,
        /// The number of an ordered item, `None` for a bullet.
        number: Option<u32>,
        /// `Some` for a task-list item: whether it is ticked.
        checked: Option<bool>,
        inlines: Vec<Inline>,
    },
    Quote(Vec<Inline>),
    Code {
        lang: String,
        text: String,
    },
    Rule,
    /// A pipe table, header row first. Every row is a list of cells.
    Table(Vec<Vec<Vec<Inline>>>),
}

pub fn parse(src: &str) -> Vec<Block> {
    let mut out = Vec::new();
    // Paragraph and quote lines are gathered, then parsed as one run of inline text, so
    // emphasis can span a line break the way it does in Markdown.
    let mut para: Vec<&str> = Vec::new();
    let mut quote: Vec<&str> = Vec::new();
    let mut table: Vec<Vec<Vec<Inline>>> = Vec::new();
    let mut lines = src.lines();

    fn flush(
        out: &mut Vec<Block>,
        para: &mut Vec<&str>,
        quote: &mut Vec<&str>,
        table: &mut Vec<Vec<Vec<Inline>>>,
    ) {
        if !para.is_empty() {
            out.push(Block::Paragraph(inlines(&para.join(" "))));
            para.clear();
        }
        if !quote.is_empty() {
            out.push(Block::Quote(inlines(&quote.join(" "))));
            quote.clear();
        }
        if !table.is_empty() {
            out.push(Block::Table(core::mem::take(table)));
        }
    }

    while let Some(raw) = lines.next() {
        let line = raw.trim_end();
        let trimmed = line.trim_start();

        if let Some(fence) = fence(trimmed) {
            flush(&mut out, &mut para, &mut quote, &mut table);
            let lang = trimmed[fence.len()..].trim().to_owned();
            let mut body: Vec<&str> = Vec::new();
            for code_line in lines.by_ref() {
                if code_line.trim_start().starts_with(fence) {
                    break;
                }
                body.push(code_line);
            }
            out.push(Block::Code {
                lang,
                text: body.join("\n"),
            });
            continue;
        }

        if trimmed.is_empty() {
            flush(&mut out, &mut para, &mut quote, &mut table);
            continue;
        }

        // Setext: a line of `=` or `-` under a paragraph makes it a heading.
        if !para.is_empty() && is_run_of(trimmed, '=') {
            let text = para.join(" ");
            para.clear();
            out.push(Block::Heading {
                level: 1,
                inlines: inlines(&text),
            });
            continue;
        }
        if !para.is_empty() && is_run_of(trimmed, '-') {
            let text = para.join(" ");
            para.clear();
            out.push(Block::Heading {
                level: 2,
                inlines: inlines(&text),
            });
            continue;
        }

        if let Some((level, text)) = heading(trimmed) {
            flush(&mut out, &mut para, &mut quote, &mut table);
            out.push(Block::Heading {
                level,
                inlines: inlines(text),
            });
            continue;
        }

        if is_rule(trimmed) {
            flush(&mut out, &mut para, &mut quote, &mut table);
            out.push(Block::Rule);
            continue;
        }

        if let Some(rest) = trimmed.strip_prefix('>') {
            if !para.is_empty() || !table.is_empty() {
                flush(&mut out, &mut para, &mut quote, &mut table);
            }
            quote.push(rest.trim());
            continue;
        }

        if let Some(item) = list_item(line) {
            flush(&mut out, &mut para, &mut quote, &mut table);
            out.push(item);
            continue;
        }

        if trimmed.starts_with('|') {
            if !para.is_empty() || !quote.is_empty() {
                flush(&mut out, &mut para, &mut quote, &mut table);
            }
            // The `|---|---|` line under the header carries no content.
            if !trimmed.chars().all(|c| matches!(c, '|' | '-' | ':' | ' ')) {
                table.push(cells(trimmed));
            }
            continue;
        }

        if !quote.is_empty() || !table.is_empty() {
            flush(&mut out, &mut para, &mut quote, &mut table);
        }
        para.push(trimmed);
    }
    flush(&mut out, &mut para, &mut quote, &mut table);
    out
}

fn fence(line: &str) -> Option<&'static str> {
    ["```", "~~~"].into_iter().find(|f| line.starts_with(f))
}

fn is_run_of(line: &str, c: char) -> bool {
    line.len() >= 2 && line.chars().all(|x| x == c)
}

fn heading(line: &str) -> Option<(u8, &str)> {
    let hashes = line.chars().take_while(|&c| c == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = &line[hashes..];
    if !rest.is_empty() && !rest.starts_with(' ') {
        // `#hashtag` is text, not a heading.
        return None;
    }
    // A closing run of `#`s is decoration: `## Title ##`.
    Some((hashes as u8, rest.trim().trim_end_matches('#').trim_end()))
}

fn is_rule(line: &str) -> bool {
    let compact: String = line.chars().filter(|c| *c != ' ').collect();
    compact.len() >= 3
        && ['-', '*', '_']
            .into_iter()
            .any(|c| compact.chars().all(|x| x == c))
}

fn list_item(line: &str) -> Option<Block> {
    let indent = line.len() - line.trim_start().len();
    let rest = line.trim_start();
    let (number, body) = if let Some(body) = ["- ", "* ", "+ "]
        .into_iter()
        .find_map(|m| rest.strip_prefix(m))
    {
        (None, body)
    } else {
        let digits = rest.chars().take_while(char::is_ascii_digit).count();
        if digits == 0 || digits > 9 {
            return None;
        }
        let after = &rest[digits..];
        let body = after
            .strip_prefix(". ")
            .or_else(|| after.strip_prefix(") "))?;
        (rest[..digits].parse().ok(), body)
    };
    let (checked, body) = if let Some(b) = body.strip_prefix("[ ] ") {
        (Some(false), b)
    } else if let Some(b) = body
        .strip_prefix("[x] ")
        .or_else(|| body.strip_prefix("[X] "))
    {
        (Some(true), b)
    } else {
        (None, body)
    };
    Some(Block::Item {
        depth: (indent / 2).min(6) as u8,
        number,
        checked,
        inlines: inlines(body.trim()),
    })
}

fn cells(line: &str) -> Vec<Vec<Inline>> {
    let inner = line.trim().trim_start_matches('|').trim_end_matches('|');
    inner.split('|').map(|c| inlines(c.trim())).collect()
}

/// Parse inline markup into runs.
pub fn inlines(src: &str) -> Vec<Inline> {
    let mut p = InlineParser {
        out: Vec::new(),
        text: String::new(),
        style: 0,
    };
    p.run(src);
    p.flush();
    p.out
}

struct InlineParser {
    out: Vec<Inline>,
    text: String,
    style: u8,
}

impl InlineParser {
    fn flush(&mut self) {
        if !self.text.is_empty() {
            self.out.push(Inline {
                text: core::mem::take(&mut self.text),
                style: self.style,
                link: None,
            });
        }
    }

    fn toggle(&mut self, bit: u8) {
        self.flush();
        self.style ^= bit;
    }

    fn run(&mut self, src: &str) {
        let mut i = 0;
        while i < src.len() {
            let rest = &src[i..];
            let c = rest.chars().next().unwrap_or(' ');

            if c == '\\' {
                if let Some(next) = rest[1..].chars().next() {
                    self.text.push(next);
                    i += 1 + next.len_utf8();
                } else {
                    self.text.push('\\');
                    i += 1;
                }
                continue;
            }

            if c == '`'
                && let Some(end) = rest[1..].find('`')
            {
                self.flush();
                self.out.push(Inline {
                    text: rest[1..1 + end].to_owned(),
                    style: self.style | CODE,
                    link: None,
                });
                i += end + 2;
                continue;
            }

            if let Some((len, text, target)) = link(rest) {
                self.flush();
                let image = rest.starts_with('!');
                // The link text keeps the surrounding style, so a link inside bold is bold.
                let mut label = inlines(text);
                if label.is_empty() || image {
                    let alt = if text.is_empty() { target } else { text };
                    label = vec![Inline::plain(&format!(
                        "{} {alt}",
                        ccosel_sdk::icons::IMAGE
                    ))];
                }
                for mut run in label {
                    run.style |= self.style;
                    run.link = Some(target.to_owned());
                    self.out.push(run);
                }
                i += len;
                continue;
            }

            if let Some((len, glyph)) = shortcode(rest) {
                // Joins the text around it, in the same style: an icon is just a character.
                self.text.push(glyph);
                i += len;
                continue;
            }

            if let Some(url) = autolink(rest) {
                self.flush();
                self.out.push(Inline {
                    text: url.to_owned(),
                    style: self.style,
                    link: Some(url.to_owned()),
                });
                i += url.len() + 2;
                continue;
            }

            // Double markers first, so `**` is never read as two `*`s.
            let mut matched = false;
            for (marker, bit) in [("**", STRONG), ("__", STRONG), ("~~", STRIKE)] {
                if rest.starts_with(marker) && self.can_toggle(bit, &rest[2..], marker) {
                    self.toggle(bit);
                    i += 2;
                    matched = true;
                    break;
                }
            }
            if matched {
                continue;
            }
            if (c == '*' || c == '_') && self.can_toggle(ITALIC, &rest[1..], &rest[..1]) {
                // `snake_case_name` is not emphasis: an underscore between two word characters
                // is just an underscore.
                let before = src[..i].chars().next_back();
                let after = rest[1..].chars().next();
                let intraword = c == '_'
                    && before.is_some_and(char::is_alphanumeric)
                    && after.is_some_and(char::is_alphanumeric);
                if !intraword {
                    self.toggle(ITALIC);
                    i += 1;
                    continue;
                }
            }

            self.text.push(c);
            i += c.len_utf8();
        }
    }

    /// Opening a style needs a closing marker later on; closing one needs it to be open.
    /// Without this an unmatched `*` would restyle the rest of the paragraph.
    fn can_toggle(&self, bit: u8, after: &str, marker: &str) -> bool {
        if self.style & bit != 0 {
            return true;
        }
        !after.starts_with(' ') && !after.is_empty() && after.contains(marker)
    }
}

/// `[text](target)` or `![alt](src)`: the byte length consumed, the text and the target.
fn link(s: &str) -> Option<(usize, &str, &str)> {
    let start = usize::from(s.starts_with('!'));
    let body = s[start..].strip_prefix('[')?;
    let close = body.find("](")?;
    let text = &body[..close];
    let after = &body[close + 2..];
    let end = after.find(')')?;
    // An optional title, `[a](url "title")`, is dropped.
    let target = after[..end].split_whitespace().next().unwrap_or("");
    Some((start + 1 + close + 2 + end + 1, text, target))
}

/// `:name:` naming a Phosphor icon: the bytes consumed and the icon. Anything else, such as
/// the time `10:30:00` or an unknown name, is left as text.
fn shortcode(s: &str) -> Option<(usize, char)> {
    let body = s.strip_prefix(':')?;
    let end = body.find(':')?;
    let name = &body[..end];
    if !name
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return None;
    }
    crate::phosphor::lookup(name).map(|glyph| (end + 2, glyph))
}

/// `<https://...>`: the URL, if this is one.
fn autolink(s: &str) -> Option<&str> {
    let body = s.strip_prefix('<')?;
    let end = body.find('>')?;
    let url = &body[..end];
    (is_external(url) && !url.contains(' ')).then_some(url)
}

/// A link that leaves this app: to the web, or an email address.
pub fn is_external(target: &str) -> bool {
    ["http://", "https://", "mailto:"]
        .into_iter()
        .any(|p| target.starts_with(p))
}
