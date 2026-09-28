//! Lengths typed into the Measurements box, and numbers shown in the status line.
//!
//! Hand-rolled on purpose: `str::parse::<f32>` and float `Display` each drag kilobytes of
//! formatting machinery into a module every user downloads, and a length is simple.

/// A length in millimetres from what a person typed: `250`, `-30`, `12.5`, `2m`, `40 cm`,
/// `3.5mm`. `None` for anything else.
pub fn length(s: &str) -> Option<f32> {
    let s = s.trim();
    let (num, unit) = match s.find(|c: char| c.is_ascii_alphabetic()) {
        Some(i) => (s[..i].trim(), s[i..].trim()),
        None => (s, ""),
    };
    let scale = match unit {
        "" | "mm" => 1.0,
        "cm" => 10.0,
        "m" => 1000.0,
        _ => return None,
    };
    Some(number(num)? * scale)
}

/// Two lengths for a rectangle, `width,height`, as SketchUp takes them. A semicolon works too,
/// for people whose decimal separator is the comma.
pub fn pair(s: &str) -> Option<(f32, f32)> {
    let (a, b) = s.split_once([',', ';'])?;
    Some((length(a)?, length(b)?))
}

/// A decimal number: optional sign, digits, optional fraction. No exponents, no `inf`.
fn number(s: &str) -> Option<f32> {
    let (neg, digits) = match s.as_bytes().first()? {
        b'-' => (true, &s[1..]),
        b'+' => (false, &s[1..]),
        _ => (false, s),
    };
    let (whole, frac) = match digits.split_once('.') {
        Some((w, f)) => (w, f),
        None => (digits, ""),
    };
    if whole.is_empty() && frac.is_empty() {
        return None;
    }
    let mut v: f64 = 0.0;
    for c in whole.bytes() {
        if !c.is_ascii_digit() {
            return None;
        }
        v = v * 10.0 + f64::from(c - b'0');
    }
    let mut place = 0.1;
    for c in frac.bytes() {
        if !c.is_ascii_digit() {
            return None;
        }
        v += f64::from(c - b'0') * place;
        place /= 10.0;
    }
    if v > 1e9 {
        return None;
    }
    Some(if neg { -v } else { v } as f32)
}

/// A whole number with thousands separators: `952 000`.
pub fn thousands(mut n: u64) -> String {
    let mut groups = Vec::new();
    loop {
        groups.push(n % 1000);
        n /= 1000;
        if n == 0 {
            break;
        }
    }
    let mut out = String::new();
    for (i, g) in groups.iter().rev().enumerate() {
        if i == 0 {
            out.push_str(&itoa(*g));
        } else {
            out.push(' ');
            let s = itoa(*g);
            for _ in s.len()..3 {
                out.push('0');
            }
            out.push_str(&s);
        }
    }
    out
}

/// A length in whole millimetres, signed: `-30 mm`.
pub fn mm(v: f32) -> String {
    let r = if v < 0.0 { -v } else { v };
    let n = (r + 0.5) as u64;
    let mut s = String::new();
    if v < 0.0 && n > 0 {
        s.push('-');
    }
    s.push_str(&thousands(n));
    s.push_str(" mm");
    s
}

pub fn itoa(mut n: u64) -> String {
    if n == 0 {
        return "0".to_owned();
    }
    let mut buf = [0u8; 20];
    let mut i = buf.len();
    while n > 0 {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    String::from_utf8_lossy(&buf[i..]).into_owned()
}
