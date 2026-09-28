use crate::markdown::*;

fn plain(text: &str) -> Inline {
    Inline::plain(text)
}

fn styled(text: &str, style: u8) -> Inline {
    Inline {
        text: text.to_owned(),
        style,
        link: None,
    }
}

fn linked(text: &str, style: u8, to: &str) -> Inline {
    Inline {
        text: text.to_owned(),
        style,
        link: Some(to.to_owned()),
    }
}

#[test]
fn headings_paragraphs_and_rules() {
    let doc = parse("# Title #\n\nFirst line\nsecond line.\n\n---\n### Small\n#hashtag");
    assert_eq!(
        doc,
        vec![
            Block::Heading {
                level: 1,
                inlines: vec![plain("Title")]
            },
            Block::Paragraph(vec![plain("First line second line.")]),
            Block::Rule,
            Block::Heading {
                level: 3,
                inlines: vec![plain("Small")]
            },
            Block::Paragraph(vec![plain("#hashtag")]),
        ]
    );
    assert_eq!(parse("####### seven").len(), 1);
    assert!(matches!(parse("####### seven")[0], Block::Paragraph(_)));
}

#[test]
fn setext_headings() {
    assert_eq!(
        parse("Big\n===\nSmaller\n---"),
        vec![
            Block::Heading {
                level: 1,
                inlines: vec![plain("Big")]
            },
            Block::Heading {
                level: 2,
                inlines: vec![plain("Smaller")]
            },
        ]
    );
    // With nothing above it, `---` is a rule, and `* * *` is one too.
    assert_eq!(parse("---\n* * *"), vec![Block::Rule, Block::Rule]);
}

#[test]
fn inline_styles() {
    assert_eq!(
        inlines("a **b** *c* _d_ `e` ~~f~~ __g__"),
        vec![
            plain("a "),
            styled("b", STRONG),
            plain(" "),
            styled("c", ITALIC),
            plain(" "),
            styled("d", ITALIC),
            plain(" "),
            styled("e", CODE),
            plain(" "),
            styled("f", STRIKE),
            plain(" "),
            styled("g", STRONG),
        ]
    );
    // Styles nest.
    assert_eq!(
        inlines("**bold *both***"),
        vec![styled("bold ", STRONG), styled("both", STRONG | ITALIC)]
    );
}

#[test]
fn stray_markers_stay_text() {
    assert_eq!(inlines("2 * 3 = 6"), vec![plain("2 * 3 = 6")]);
    assert_eq!(inlines("a *b"), vec![plain("a *b")]);
    assert_eq!(inlines("snake_case_name"), vec![plain("snake_case_name")]);
    assert_eq!(
        inlines(r"\*not em\* and \\"),
        vec![plain(r"*not em* and \")]
    );
    assert_eq!(inlines("trailing \\"), vec![plain("trailing \\")]);
    assert_eq!(inlines("`unclosed"), vec![plain("`unclosed")]);
}

#[test]
fn links_images_and_autolinks() {
    assert_eq!(
        inlines("see [the **guide**](guide.md \"Guide\") now"),
        vec![
            plain("see "),
            linked("the ", 0, "guide.md"),
            linked("guide", STRONG, "guide.md"),
            plain(" now"),
        ]
    );
    assert_eq!(
        inlines("**[bold link](a.md)**"),
        vec![linked("bold link", STRONG, "a.md")]
    );
    assert_eq!(
        inlines("![a cat](cat.png)"),
        vec![linked(
            &format!("{} a cat", ccosel_sdk::icons::IMAGE),
            0,
            "cat.png"
        )]
    );
    assert_eq!(
        inlines("[](x.md)"),
        vec![linked(
            &format!("{} x.md", ccosel_sdk::icons::IMAGE),
            0,
            "x.md"
        )]
    );
    assert_eq!(
        inlines("<https://example.com> <not a link>"),
        vec![
            linked("https://example.com", 0, "https://example.com"),
            plain(" <not a link>"),
        ]
    );
    assert_eq!(inlines("[no target]"), vec![plain("[no target]")]);
    assert!(is_external("mailto:a@b.c"));
    assert!(!is_external("/Docs/a.md"));
}

#[test]
fn lists_and_tasks() {
    let doc = parse("- one\n  * nested\n3. three\n4) four\n- [ ] todo\n- [x] done\n- [X] Done");
    let items: Vec<(u8, Option<u32>, Option<bool>, String)> = doc
        .iter()
        .map(|b| match b {
            Block::Item {
                depth,
                number,
                checked,
                inlines,
            } => (*depth, *number, *checked, inlines[0].text.clone()),
            other => panic!("not an item: {other:?}"),
        })
        .collect();
    assert_eq!(
        items,
        vec![
            (0, None, None, "one".into()),
            (1, None, None, "nested".into()),
            (0, Some(3), None, "three".into()),
            (0, Some(4), None, "four".into()),
            (0, None, Some(false), "todo".into()),
            (0, None, Some(true), "done".into()),
            (0, None, Some(true), "Done".into()),
        ]
    );
    // Not list items.
    assert!(matches!(parse("-no space")[0], Block::Paragraph(_)));
    assert!(matches!(
        parse("1234567890. too long")[0],
        Block::Paragraph(_)
    ));
    assert!(matches!(parse("12 apples")[0], Block::Paragraph(_)));
}

#[test]
fn quotes_code_and_tables() {
    let doc = parse(
        "> quoted\n> *more*\npara\n\n```rust\nfn main() {}\n\n```\n~~~\nx\n~~~\n| a | b |\n|---|:-:|\n| [c](c.md) | d |\ntail",
    );
    assert_eq!(
        doc,
        vec![
            Block::Quote(vec![plain("quoted "), styled("more", ITALIC)]),
            Block::Paragraph(vec![plain("para")]),
            Block::Code {
                lang: "rust".into(),
                text: "fn main() {}\n".into()
            },
            Block::Code {
                lang: String::new(),
                text: "x".into()
            },
            Block::Table(vec![
                vec![vec![plain("a")], vec![plain("b")]],
                vec![vec![linked("c", 0, "c.md")], vec![plain("d")]],
            ]),
            Block::Paragraph(vec![plain("tail")]),
        ]
    );
    // A paragraph running into a quote or a table ends first.
    assert_eq!(parse("p\n> q").len(), 2);
    assert_eq!(parse("p\n| t |").len(), 2);
    assert_eq!(parse("> q\n| t |").len(), 2);
    assert_eq!(parse("| t |\n> q").len(), 2);
    // An unclosed fence runs to the end.
    assert_eq!(
        parse("```\nopen"),
        vec![Block::Code {
            lang: String::new(),
            text: "open".into()
        }]
    );
}

#[test]
fn empty_input_is_no_blocks() {
    assert!(parse("").is_empty());
    assert!(parse("\n\n  \n").is_empty());
}

#[test]
fn shortcodes_become_phosphor_icons() {
    let house = ccosel_sdk::icons::HOUSE;
    assert_eq!(
        inlines(":house: Home"),
        vec![plain(&format!("{house} Home"))]
    );
    // Phosphor's own kebab-case names work too, and case does not matter.
    assert_eq!(
        inlines(":folder-open: and :FOLDER_OPEN:"),
        vec![plain(&format!(
            "{f} and {f}",
            f = ccosel_sdk::icons::FOLDER_OPEN
        ))]
    );
    // An icon takes the style it sits in.
    assert_eq!(inlines("**:house:**"), vec![styled(house, STRONG)]);
    // Not icons: unknown names, times, spaces, a lone colon, and code.
    for text in [
        ":nope:",
        "10:30:00",
        ": house :",
        "a: b",
        "ends with :",
        ":house",
    ] {
        assert_eq!(inlines(text), vec![plain(text)], "{text:?}");
    }
    assert_eq!(inlines("`:house:`"), vec![styled(":house:", CODE)]);
}

#[test]
fn the_icon_table_is_sorted_and_matches_the_font() {
    use crate::phosphor::{CODES, NAMES, lookup};
    let names: Vec<&str> = NAMES.split('\n').collect();
    assert_eq!(names.len(), CODES.len(), "one codepoint per name");
    assert!(
        names.windows(2).all(|w| w[0] < w[1]),
        "sorted, no duplicates"
    );
    assert!(names.len() > 1000, "the whole set, not a sample");
    // Spot checks against the icons the SDK already verifies against the font.
    for (name, icon) in [
        ("house", ccosel_sdk::icons::HOUSE),
        ("book_open", ccosel_sdk::icons::BOOK_OPEN),
        ("magnifying_glass", ccosel_sdk::icons::MAGNIFYING_GLASS),
        ("check_square", ccosel_sdk::icons::CHECK_SQUARE),
    ] {
        assert_eq!(
            lookup(name).map(String::from).as_deref(),
            Some(icon),
            "{name}"
        );
    }
    assert_eq!(lookup(""), None);
    assert_eq!(lookup(&"a".repeat(41)), None);
}
