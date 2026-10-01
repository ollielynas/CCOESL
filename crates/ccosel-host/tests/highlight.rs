//! The code editor's colouring: what the Octave lexer calls each part of a line.

use ccosel_abi::CodeLang;
use ccosel_host::highlight::{Token, spans};

/// Each coloured run of `text`, as the text it covers.
fn runs(text: &str) -> Vec<(&str, Token)> {
    spans(text, CodeLang::Octave)
        .into_iter()
        .map(|(r, t)| (&text[r], t))
        .collect()
}

use Token::{Comment, Keyword, Number, String};

#[test]
fn comments_numbers_and_keywords() {
    assert_eq!(
        runs("x = 3; % three\nif x > 1.5e-3, y = 2i; end # done"),
        [
            ("3", Number),
            ("% three", Comment),
            ("if", Keyword),
            ("1.5e-3", Number),
            ("2i", Number),
            ("end", Keyword),
            ("# done", Comment),
        ]
    );
}

#[test]
fn a_quote_after_a_value_is_a_transpose_not_a_string() {
    assert_eq!(
        runs("b = a' * x(1)'; s = 'it''s';"),
        [("1", Number), ("'it''s'", String)]
    );
    assert_eq!(runs("c = [a b]';"), []);
}

#[test]
fn double_quoted_strings_escape_with_a_backslash() {
    assert_eq!(
        runs(r#"printf("say \"hi\" %d\n", 2) % out"#),
        [
            (r#""say \"hi\" %d\n""#, String),
            ("2", Number),
            ("% out", Comment),
        ]
    );
}

#[test]
fn a_comment_sign_in_a_string_is_part_of_the_string() {
    assert_eq!(runs("disp('100%')"), [("'100%'", String)]);
}

#[test]
fn block_comments_and_continuations() {
    let text = "%{\nnot = code;\n%}\nx = [1 ...\n 2];";
    assert_eq!(
        runs(text),
        [
            ("%{", Comment),
            ("not = code;", Comment),
            ("%}", Comment),
            ("1", Number),
            ("...", Comment),
            ("2", Number),
        ]
    );
}

#[test]
fn a_field_named_like_a_keyword_is_a_field() {
    assert_eq!(
        runs("s.end = 1; for k = 1:n, endfor"),
        [
            ("1", Number),
            ("for", Keyword),
            ("1", Number),
            ("endfor", Keyword),
        ]
    );
    assert_eq!(runs("x2 = y3"), [], "digits in a name are the name");
}

#[test]
fn unfinished_strings_stop_at_the_line_end_and_nothing_splits_a_character() {
    // Being typed: the string isn't closed yet, and the next line is still code.
    assert_eq!(
        runs("s = 'héllo\nt = 1"),
        [("'héllo", String), ("1", Number)]
    );
    // Every run lands on character boundaries, whatever the text.
    for text in ["\"é\\é\"", "'ü'", "% ☃", "x = \"\\", "é'é'", "1é", ".5é"] {
        for (range, _) in spans(text, CodeLang::Octave) {
            assert!(
                text.is_char_boundary(range.start) && text.is_char_boundary(range.end),
                "{text:?} {range:?}"
            );
        }
    }
}
