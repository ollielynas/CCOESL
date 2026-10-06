//! A workbook held as Gnumeric's own XML, edited cell by cell.
//!
//! Only each sheet's `<gnm:Cells>` is parsed and rewritten. Everything else in the file (styles,
//! column widths, names, charts, merged cells) is kept as it came, so saving the workbook loses
//! nothing the Spreadsheet app doesn't show. The XML is what `ssconvert` itself writes, so this
//! reads that, not XML in general.
//!
//! Gnumeric writes a formula repeated down a column once, and the other cells as references to
//! it (`ExprID`). Those are made explicit before anything is edited (see
//! [`Workbook::shared_cells`]), so every cell stands on its own.

use std::collections::BTreeMap;

/// One cell, as it is in the file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cell {
    /// Gnumeric's `ValueType`: 40 a number, 60 text, 20 true/false, 50 an error. `None` for
    /// a formula.
    pub value_type: Option<u32>,
    /// The content: a value, or a formula starting with `=`.
    pub text: String,
    /// Its other attributes (a number format, an array formula's size), written back as they
    /// were.
    pub extra: String,
    /// It repeats another cell's formula: `ExprID`, with no text of its own.
    pub shared: Option<String>,
}

impl Cell {
    /// What the app shows as the cell's content, and lets people edit.
    pub fn raw(&self) -> String {
        match self.value_type {
            // Text that would read as a formula or a quote is shown with a leading quote, as
            // spreadsheets do, so editing it keeps it text.
            Some(60) if self.text.starts_with(['=', '\'']) => format!("'{}", self.text),
            // Gnumeric writes numbers with every digit a double holds (`0.10000000000000001`);
            // the shortest text that reads back as the same number is what was typed.
            Some(40) => self
                .text
                .parse::<f64>()
                .map_or_else(|_| self.text.clone(), |n| n.to_string()),
            _ => self.text.clone(),
        }
    }

    /// A cell holding what someone typed: a formula, a number, true/false, or text. A leading
    /// `'` makes the rest text whatever it looks like. Keeps `previous`'s number format.
    pub fn typed(raw: &str, previous: Option<&Cell>) -> Self {
        let format = previous
            .map(|c| attr_text(&c.extra, "ValueFormat"))
            .unwrap_or_default();
        let (value_type, text) = if let Some(text) = raw.strip_prefix('\'') {
            (Some(60), text.to_owned())
        } else if raw.starts_with('=') && raw.len() > 1 {
            (None, raw.to_owned())
        } else if raw.trim().parse::<f64>().is_ok_and(f64::is_finite) {
            (Some(40), raw.trim().to_owned())
        } else if raw.eq_ignore_ascii_case("true") || raw.eq_ignore_ascii_case("false") {
            (Some(20), raw.to_ascii_uppercase())
        } else {
            (Some(60), raw.to_owned())
        };
        Self {
            value_type,
            text,
            extra: format,
            shared: None,
        }
    }

    fn write(&self, row: u32, col: u32, out: &mut String) {
        out.push_str(&format!("<gnm:Cell Row=\"{row}\" Col=\"{col}\""));
        if let Some(vt) = self.value_type {
            out.push_str(&format!(" ValueType=\"{vt}\""));
        }
        // Until it is resolved, a repeat is nothing but its link to the formula it repeats.
        if let Some(id) = &self.shared {
            out.push_str(&format!(" ExprID=\"{}\"", escape(id)));
        }
        out.push_str(&self.extra);
        out.push('>');
        out.push_str(&escape(&self.text));
        out.push_str("</gnm:Cell>\n");
    }
}

/// One sheet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sheet {
    pub name: String,
    /// How many rows and columns the sheet has room for.
    pub rows: u32,
    pub cols: u32,
    /// By `(row, column)`.
    pub cells: BTreeMap<(u32, u32), Cell>,
}

/// A whole workbook.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Workbook {
    /// The file around each sheet's cells: `parts[i]` comes before sheet `i`'s cells, and the
    /// last part after the last sheet's.
    parts: Vec<String>,
    pub sheets: Vec<Sheet>,
}

impl Workbook {
    /// Read uncompressed Gnumeric XML, as `ssconvert -T Gnumeric_XmlIO:sax:0` writes it.
    pub fn parse(xml: &str) -> Result<Self, String> {
        let bad = |what: &str| format!("the workbook couldn't be read ({what})");
        let index = between(xml, "<gnm:SheetNameIndex>", "</gnm:SheetNameIndex>")
            .ok_or_else(|| bad("no sheet list"))?;
        let mut sizes = Vec::new();
        let mut rest = index;
        while let Some(start) = rest.find("<gnm:SheetName") {
            let tag_end = rest[start..].find('>').ok_or_else(|| bad("sheet list"))? + start;
            let attrs = &rest[start + "<gnm:SheetName".len()..tag_end];
            let close = rest[tag_end..]
                .find("</gnm:SheetName>")
                .ok_or_else(|| bad("sheet list"))?
                + tag_end;
            let cols = attr_value(attrs, "gnm:Cols").and_then(|v| v.parse().ok());
            let rows = attr_value(attrs, "gnm:Rows").and_then(|v| v.parse().ok());
            sizes.push((
                unescape(&rest[tag_end + 1..close]),
                rows.unwrap_or(65_536),
                cols.unwrap_or(256),
            ));
            rest = &rest[close..];
        }

        let mut parts = Vec::new();
        let mut sheets = Vec::new();
        let mut at = 0;
        for (name, rows, cols) in sizes {
            let start = xml[at..]
                .find("<gnm:Sheet ")
                .ok_or_else(|| bad("a sheet is missing"))?
                + at;
            let end = xml[start..]
                .find("</gnm:Sheet>")
                .ok_or_else(|| bad("a sheet is unfinished"))?
                + start;
            let sheet = &xml[start..end];
            let (cells_start, cells_end, body) = if let Some(i) = sheet.find("<gnm:Cells/>") {
                (start + i, start + i + "<gnm:Cells/>".len(), "")
            } else {
                let i = sheet.find("<gnm:Cells>").ok_or_else(|| bad("no cells"))?;
                let j = sheet.find("</gnm:Cells>").ok_or_else(|| bad("no cells"))?;
                (
                    start + i,
                    start + j + "</gnm:Cells>".len(),
                    &sheet[i + "<gnm:Cells>".len()..j],
                )
            };
            parts.push(xml[at..cells_start].to_owned());
            sheets.push(Sheet {
                name,
                rows,
                cols,
                cells: parse_cells(body).map_err(|e| bad(&e))?,
            });
            at = cells_end;
        }
        parts.push(xml[at..].to_owned());
        if sheets.is_empty() {
            return Err(bad("no sheets"));
        }
        Ok(Self { parts, sheets })
    }

    /// The workbook as Gnumeric XML again, with every cell as it is now.
    pub fn to_xml(&self) -> String {
        let mut out = String::new();
        for (part, sheet) in self.parts.iter().zip(&self.sheets) {
            out.push_str(part);
            out.push_str("<gnm:Cells>\n");
            for (&(row, col), cell) in &sheet.cells {
                cell.write(row, col, &mut out);
            }
            out.push_str("</gnm:Cells>");
        }
        out.push_str(self.parts.last().map_or("", String::as_str));
        out
    }

    /// Set a cell to what someone typed; empty clears it.
    pub fn set(&mut self, sheet: u16, row: u32, col: u32, raw: &str) -> Result<(), String> {
        let s = self
            .sheets
            .get_mut(usize::from(sheet))
            .ok_or("there is no such sheet")?;
        if row >= s.rows || col >= s.cols {
            return Err("that cell is outside the sheet".into());
        }
        if raw.is_empty() {
            s.cells.remove(&(row, col));
        } else {
            let cell = Cell::typed(raw, s.cells.get(&(row, col)));
            s.cells.insert((row, col), cell);
        }
        Ok(())
    }

    /// Every cell that repeats another's formula, by sheet, row and column.
    pub fn shared_cells(&self) -> Vec<(usize, u32, u32)> {
        let mut out = Vec::new();
        for (i, sheet) in self.sheets.iter().enumerate() {
            for (&(row, col), cell) in &sheet.cells {
                if cell.shared.is_some() && cell.text.is_empty() {
                    out.push((i, row, col));
                }
            }
        }
        out
    }

    /// A copy of the workbook with a probe for each of `shared`: a cell below everything on its
    /// sheet, in column A, holding `=GET.FORMULA(..)` of it. Recalculated, each probe shows the
    /// formula of the cell it names, written out for that cell. Returns the copy and where each
    /// probe is. `None` if a sheet has no room below for them.
    pub fn with_probes(&self, shared: &[(usize, u32, u32)]) -> Option<(Self, Vec<u32>)> {
        let mut copy = self.clone();
        let mut next: Vec<u32> = self
            .sheets
            .iter()
            .map(|s| s.cells.keys().map(|&(r, _)| r + 1).max().unwrap_or(0))
            .collect();
        let mut rows = Vec::new();
        for &(sheet, row, col) in shared {
            let probe = next[sheet];
            if probe >= self.sheets[sheet].rows {
                return None;
            }
            next[sheet] += 1;
            let target = format!("${}${}", ccosel_proto::sheet::column_name(col), row + 1);
            copy.sheets[sheet].cells.insert(
                (probe, 0),
                Cell {
                    value_type: None,
                    text: format!("=GET.FORMULA({target})"),
                    extra: String::new(),
                    shared: None,
                },
            );
            rows.push(probe);
        }
        Some((copy, rows))
    }

    /// Give a repeating cell its formula, written out, so it no longer depends on another.
    pub fn resolve(&mut self, sheet: usize, row: u32, col: u32, formula: &str) {
        if let Some(cell) = self.sheets[sheet].cells.get_mut(&(row, col)) {
            cell.text = formula.to_owned();
            cell.value_type = None;
            cell.shared = None;
        }
    }

    /// Make the cells that define a shared formula ordinary ones, once every repeat has its own.
    pub fn unshare(&mut self) {
        for sheet in &mut self.sheets {
            for cell in sheet.cells.values_mut() {
                cell.shared = None;
            }
        }
    }

    pub fn cell_count(&self) -> usize {
        self.sheets.iter().map(|s| s.cells.len()).sum()
    }
}

fn between<'a>(text: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let start = text.find(open)? + open.len();
    let end = text[start..].find(close)? + start;
    Some(&text[start..end])
}

fn parse_cells(body: &str) -> Result<BTreeMap<(u32, u32), Cell>, String> {
    let mut cells = BTreeMap::new();
    let mut rest = body;
    while let Some(start) = rest.find("<gnm:Cell ") {
        rest = &rest[start + "<gnm:Cell ".len()..];
        let tag_end = tag_end(rest).ok_or("a cell is unfinished")?;
        let self_closing = rest[..tag_end].ends_with('/');
        let attrs = rest[..tag_end].trim_end_matches('/');
        let text = if self_closing {
            rest = &rest[tag_end + 1..];
            String::new()
        } else {
            let close = rest.find("</gnm:Cell>").ok_or("a cell is unfinished")?;
            let text = unescape(&rest[tag_end + 1..close]);
            rest = &rest[close + "</gnm:Cell>".len()..];
            text
        };
        let num = |name: &str| attr_value(attrs, name).and_then(|v| v.parse::<u32>().ok());
        let (Some(row), Some(col)) = (num("Row"), num("Col")) else {
            return Err("a cell has no place".into());
        };
        let mut extra = String::new();
        for (name, value) in attrs_of(attrs) {
            if !matches!(name, "Row" | "Col" | "ValueType" | "ExprID") {
                extra.push_str(&format!(" {name}=\"{value}\""));
            }
        }
        cells.insert(
            (row, col),
            Cell {
                value_type: num("ValueType"),
                text,
                extra,
                shared: attr_value(attrs, "ExprID"),
            },
        );
    }
    Ok(cells)
}

/// Where the tag that `text` is inside of ends (its `>`), skipping `>` in quoted values.
fn tag_end(text: &str) -> Option<usize> {
    let mut quoted = false;
    for (i, c) in text.char_indices() {
        match c {
            '"' => quoted = !quoted,
            '>' if !quoted => return Some(i),
            _ => {}
        }
    }
    None
}

/// Each `name="value"` in a tag's attribute text, the value still escaped.
fn attrs_of(attrs: &str) -> Vec<(&str, &str)> {
    let mut out = Vec::new();
    let mut rest = attrs;
    while let Some(eq) = rest.find("=\"") {
        let name = rest[..eq].trim();
        let after = &rest[eq + 2..];
        let Some(end) = after.find('"') else { break };
        out.push((name, &after[..end]));
        rest = &after[end + 1..];
    }
    out
}

fn attr_value(attrs: &str, name: &str) -> Option<String> {
    attrs_of(attrs)
        .into_iter()
        .find(|(n, _)| *n == name)
        .map(|(_, v)| unescape(v))
}

/// ` ValueFormat="…"` from attribute text, if it is there.
fn attr_text(extra: &str, name: &str) -> String {
    attrs_of(extra)
        .into_iter()
        .find(|(n, _)| *n == name)
        .map(|(n, v)| format!(" {n}=\"{v}\""))
        .unwrap_or_default()
}

pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
    out
}

pub fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        let Some(semi) = rest.find(';') else { break };
        let entity = &rest[1..semi];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            e if e.starts_with("#x") => u32::from_str_radix(&e[2..], 16)
                .ok()
                .and_then(char::from_u32),
            e if e.starts_with('#') => e[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &rest[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// The rows of CSV text, as `ssconvert` writes them: quoted fields may hold commas, quotes
/// (doubled) and line breaks.
pub fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted => {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    quoted = false;
                }
            }
            '"' if field.is_empty() => quoted = true,
            c if quoted => field.push(c),
            ',' => row.push(std::mem::take(&mut field)),
            '\r' if chars.peek() == Some(&'\n') => {}
            '\n' => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            c => field.push(c),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

#[cfg(test)]
mod tests;
