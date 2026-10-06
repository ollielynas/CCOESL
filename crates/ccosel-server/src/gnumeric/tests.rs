//! Against `fixture.xml`: a three-sheet workbook exactly as `ssconvert` 1.12.57 wrote it from an
//! `.xlsx`, with a formula repeated down a column (`ExprID`), a formula on another sheet, a
//! sheet name with `&` in it, and an empty sheet.

use super::*;

const FIXTURE: &str = include_str!("fixture.xml");

fn book() -> Workbook {
    Workbook::parse(FIXTURE).unwrap()
}

#[test]
fn reads_every_sheet_with_its_name_size_and_cells() {
    let b = book();
    let names: Vec<_> = b.sheets.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["t.csv", "Second & more", "Empty"]);
    assert_eq!((b.sheets[0].rows, b.sheets[0].cols), (1_048_576, 16_384));
    assert_eq!(b.sheets[0].cells.len(), 11);
    assert!(b.sheets[2].cells.is_empty());
    let c = &b.sheets[0].cells;
    assert_eq!(c[&(0, 0)].raw(), "a");
    assert_eq!(c[&(1, 0)].raw(), "1");
    assert_eq!(c[&(1, 2)].raw(), "=A2+B2");
    assert_eq!(c[&(4, 2)].raw(), "=1/0");
    assert_eq!(b.sheets[1].cells[&(0, 0)].raw(), "=sum(t.csv!C2:C3)*2");
    assert_eq!(b.cell_count(), 12);
}

#[test]
fn writing_back_unchanged_keeps_everything_and_reads_the_same() {
    let b = book();
    let again = Workbook::parse(&b.to_xml()).unwrap();
    assert_eq!(again, b);
    // Everything around the cells is kept as it was: styles, names, sizes.
    let outside = |xml: &str| {
        let mut s = String::new();
        let mut rest = xml;
        while let Some(i) = rest.find("<gnm:Cells") {
            s.push_str(&rest[..i]);
            let j = rest[i..]
                .find("</gnm:Cells>")
                .map(|j| i + j + "</gnm:Cells>".len())
                .or_else(|| rest[i..].find("<gnm:Cells/>").map(|j| i + j + 12))
                .unwrap();
            rest = &rest[j..];
        }
        s.push_str(rest);
        s
    };
    assert_eq!(outside(&b.to_xml()), outside(FIXTURE));
}

#[test]
fn a_repeated_formula_is_found_and_resolved_through_probes() {
    let mut b = book();
    let shared = b.shared_cells();
    assert_eq!(shared, [(0, 2, 2)], "C3 repeats C2's formula");
    let (probed, rows) = b.with_probes(&shared).unwrap();
    // Below the last used row (4), in column A.
    assert_eq!(rows, [5]);
    assert_eq!(probed.sheets[0].cells[&(5, 0)].text, "=GET.FORMULA($C$3)");

    b.resolve(0, 2, 2, "=A3+B3");
    b.unshare();
    assert!(b.shared_cells().is_empty());
    let xml = b.to_xml();
    assert!(!xml.contains("ExprID"), "every cell stands on its own");
    assert!(xml.contains("<gnm:Cell Row=\"2\" Col=\"2\">=A3+B3</gnm:Cell>"));
}

#[test]
fn no_room_for_probes_is_reported() {
    let mut b = book();
    b.sheets[0].rows = 5;
    assert_eq!(b.with_probes(&b.shared_cells()), None);
}

#[test]
fn typing_makes_numbers_text_booleans_and_formulas() {
    let t = |raw| Cell::typed(raw, None);
    assert_eq!(
        (t("12.5").value_type, t("12.5").text.as_str()),
        (Some(40), "12.5")
    );
    assert_eq!(t(" 7 ").text, "7");
    assert_eq!(t("-1e3").value_type, Some(40));
    assert_eq!(t("hello").value_type, Some(60));
    assert_eq!(t("inf").value_type, Some(60), "not a finite number");
    assert_eq!(
        (t("true").value_type, t("true").text.as_str()),
        (Some(20), "TRUE")
    );
    assert_eq!(t("=A1*2").value_type, None);
    assert_eq!(t("=").value_type, Some(60), "a lone = is text");
    // A leading quote keeps the rest as text, and reads back with it.
    let quoted = t("'=not a formula");
    assert_eq!(
        (quoted.value_type, quoted.text.as_str()),
        (Some(60), "=not a formula")
    );
    assert_eq!(quoted.raw(), "'=not a formula");
    assert_eq!(t("'0042").raw(), "0042");
}

#[test]
fn an_edit_keeps_the_cells_number_format() {
    let previous = Cell {
        value_type: Some(40),
        text: "45000".into(),
        extra: " ValueFormat=\"yyyy-mm-dd\" Rows=\"2\"".into(),
        shared: None,
    };
    let cell = Cell::typed("45001", Some(&previous));
    assert_eq!(
        cell.extra, " ValueFormat=\"yyyy-mm-dd\"",
        "the format, not the array size"
    );
}

#[test]
fn setting_and_clearing_cells() {
    let mut b = book();
    b.set(2, 3, 1, "a < b & \"c\"").unwrap();
    b.set(0, 1, 2, "").unwrap();
    let again = Workbook::parse(&b.to_xml()).unwrap();
    assert_eq!(again.sheets[2].cells[&(3, 1)].raw(), "a < b & \"c\"");
    assert!(!again.sheets[0].cells.contains_key(&(1, 2)));
    assert_eq!(
        b.set(9, 0, 0, "x"),
        Err("there is no such sheet".to_owned())
    );
    assert_eq!(
        b.set(0, 1_048_576, 0, "x"),
        Err("that cell is outside the sheet".to_owned())
    );
}

#[test]
fn unreadable_workbooks_say_so() {
    assert!(Workbook::parse("<xml/>").is_err());
    let no_sheet = "<gnm:SheetNameIndex><gnm:SheetName>A</gnm:SheetName></gnm:SheetNameIndex>";
    assert!(Workbook::parse(no_sheet).is_err());
    let no_cells = format!("{no_sheet}<gnm:Sheet x=\"1\"></gnm:Sheet>");
    assert!(Workbook::parse(&no_cells).is_err());
    let bad_cell = format!(
        "{no_sheet}<gnm:Sheet x=\"1\"><gnm:Cells><gnm:Cell Col=\"1\">x</gnm:Cell></gnm:Cells></gnm:Sheet>"
    );
    assert!(Workbook::parse(&bad_cell).is_err());
    let empty_index = "<gnm:SheetNameIndex></gnm:SheetNameIndex>";
    assert!(Workbook::parse(empty_index).is_err());
}

#[test]
fn entities_and_csv() {
    assert_eq!(
        unescape("a &amp; b &lt;c&gt; &quot;d&quot; &apos;e&apos; &#65;&#x42;"),
        "a & b <c> \"d\" 'e' AB"
    );
    assert_eq!(unescape("x &bogus; & y"), "x &bogus; & y");
    assert_eq!(escape("<a & \"b\">"), "&lt;a &amp; &quot;b&quot;&gt;");
    assert_eq!(
        parse_csv("a,\"b,c\",\"d \"\"e\"\"\"\r\n,\"multi\nline\"\nlast"),
        [
            vec!["a", "b,c", "d \"e\""],
            vec!["", "multi\nline"],
            vec!["last"]
        ]
    );
    assert!(parse_csv("").is_empty());
}
