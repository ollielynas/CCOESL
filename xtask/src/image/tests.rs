use super::*;

#[test]
fn the_login_form_action_is_found_and_unescaped() {
    let html = r#"<div><form id="kc-form-login" onsubmit="login.disabled = true; return true;" action="http://localhost:1/idp/realms/ccosel/login-actions/authenticate?session_code=a&amp;execution=b&amp;client_id=ccosel" method="post"><input name="username"></form></div>"#;
    assert_eq!(
        login_form_action(html).as_deref(),
        Some(
            "http://localhost:1/idp/realms/ccosel/login-actions/authenticate?session_code=a&execution=b&client_id=ccosel"
        )
    );
}

#[test]
fn a_page_without_the_login_form_has_no_action() {
    assert_eq!(login_form_action("<form action=\"/x\"></form>"), None);
    // An action on some later tag is not the login form's.
    assert_eq!(
        login_form_action(r#"<form id="kc-form-login"></form><form action="/x">"#),
        None
    );
}

#[test]
fn a_response_with_another_status_is_an_error_that_names_both() {
    let resp = Response {
        status: 502,
        url: "http://localhost:1/x".into(),
        body: b"bad gateway".to_vec(),
    };
    let err = resp.expect(200).err().unwrap().to_string();
    assert!(err.contains("502") && err.contains("200") && err.contains("bad gateway"));
}

#[test]
fn test_image_takes_only_no_build() {
    let err = test_image(&["--fast".into()]).err().unwrap().to_string();
    assert!(err.contains("usage"));
}

#[test]
fn a_free_port_is_a_real_port() {
    assert_ne!(free_port().unwrap(), 0);
}
