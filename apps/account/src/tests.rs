//! Driven through the SDK harness, which plays the server: it answers `WhoAmI` and `SignOut`
//! the way `/rpc` would, including refusing `WhoAmI` once the session is gone.

use ccosel_proto::account::{
    Account as Me, AppPasswordInfo, CreateAppPassword, ListAppPasswords, MAX_APP_PASSWORDS,
    NewAppPassword, RevokeAppPassword, SignOut, WhoAmI,
};
use ccosel_sdk::testing::{Harness, rpc_error};

use super::*;

const ACCOUNT_URL: &str = "http://localhost:8080/realms/ccosel/account";

fn signed_in(name: &str) -> Me {
    Me {
        login_enabled: true,
        name: Some(name.to_owned()),
        account_url: Some(ACCOUNT_URL.to_owned()),
    }
}

/// An Account window that has asked who is signed in and been told `me`.
fn showing(me: Me) -> Harness<Account> {
    let mut h = Harness::new(Account::default());
    h.frame();
    assert!(h.has_label("Loading…"));
    h.reply::<WhoAmI>(&me);
    h.frame();
    h
}

#[test]
fn shows_who_is_signed_in() {
    let h = showing(signed_in("ollielynas"));
    assert!(h.has_label("Signed in as"));
    assert!(h.has_label("ollielynas"));
    assert!(h.has_button("Sign out"));
    assert_eq!(
        h.open_urls(),
        [("Manage account".to_owned(), ACCOUNT_URL.to_owned())]
    );
}

#[test]
fn says_when_login_is_off() {
    let h = showing(Me {
        login_enabled: false,
        name: None,
        account_url: None,
    });
    assert!(h.has_label("Login is turned off on this server."));
    assert!(!h.has_button("Sign out"));
    assert!(h.open_urls().is_empty());
}

#[test]
fn says_when_nobody_is_signed_in() {
    let h = showing(Me {
        login_enabled: true,
        name: None,
        account_url: Some(ACCOUNT_URL.to_owned()),
    });
    assert!(h.has_label("Not signed in."));
    assert!(!h.has_button("Sign out"));
}

#[test]
fn shows_a_failed_lookup() {
    let mut h = Harness::new(Account::default());
    h.frame();
    h.fail::<WhoAmI>(rpc_error::TRANSPORT);
    h.frame();
    assert!(h.has_label("connection lost"));
}

#[test]
fn sign_out_ends_the_session_then_asks_again_once() {
    let mut h = showing(signed_in("ollielynas"));

    h.click("Sign out");
    h.frame(); // the app sees the click
    h.frame(); // and asks the server
    assert!(h.has_label("Signing out…"));
    assert_eq!(h.outstanding::<SignOut>(), 1);

    h.reply::<SignOut>(&());
    h.frame();
    assert!(h.has_label("Signed out."));
    // The re-asked WhoAmI is what the server refuses, sending the shell to sign-in.
    assert_eq!(h.outstanding::<WhoAmI>(), 1);

    h.fail::<WhoAmI>(rpc_error::TRANSPORT);
    h.frame();
    h.frame();
    assert!(h.has_label("Signed out."));
    assert_eq!(h.outstanding::<WhoAmI>(), 0);
}

#[test]
fn a_failed_sign_out_can_be_retried() {
    let mut h = showing(signed_in("ollielynas"));
    h.click("Sign out");
    h.frame();
    h.frame();
    h.fail::<SignOut>(rpc_error::TIMEOUT);
    h.frame();
    assert!(h.has_label("Could not sign out:"));
    assert!(h.has_label("timed out"));

    h.click("Try again");
    h.frame(); // the click drops the failure
    h.frame(); // and the next frame asks again
    assert_eq!(h.outstanding::<SignOut>(), 1);
    assert!(h.has_label("Signing out…"));
}

// ---- app passwords ----

fn info(id: &str, name: &str, last_used_s: Option<i64>) -> AppPasswordInfo {
    AppPasswordInfo {
        id: id.to_owned(),
        name: name.to_owned(),
        // 2026-10-02T12:00:00Z
        created_s: 1_790_942_400,
        last_used_s,
    }
}

/// Signed in, with the app password list answered with `list`.
fn with_passwords(list: &[AppPasswordInfo]) -> Harness<Account> {
    let mut h = showing(signed_in("ollielynas"));
    assert!(h.has_label("Loading…"));
    assert_eq!(h.outstanding::<ListAppPasswords>(), 1);
    h.reply::<ListAppPasswords>(&list.to_vec());
    h.frame();
    h
}

#[test]
fn dates_are_shown_as_year_month_day() {
    assert_eq!(date(0), "1970-01-01");
    assert_eq!(date(1_790_942_400), "2026-10-02");
    assert_eq!(date(1_791_028_799), "2026-10-03");
    assert_eq!(date(951_782_400), "2000-02-29");
    assert_eq!(date(-86_400), "1969-12-31");
}

#[test]
fn lists_app_passwords_with_when_they_were_made_and_used() {
    let h = with_passwords(&[
        info("a", "Laptop", Some(1_791_028_800)),
        info("b", "Phone", None),
    ]);
    assert!(h.has_text("App passwords"));
    assert!(h.has_text("Laptop"));
    assert!(h.has_text("made 2026-10-02"));
    assert!(h.has_text("last used 2026-10-03"));
    assert!(h.has_text("never used"));
    assert!(h.has_button("Revoke Laptop"));
    assert!(h.has_button("Revoke Phone"));
    assert!(h.has_button("Create"));
}

#[test]
fn says_when_there_are_none() {
    let h = with_passwords(&[]);
    assert!(h.has_text("You have none."));
    assert!(h.has_button("Create"));
}

#[test]
fn creating_one_shows_its_password_once() {
    let mut h = with_passwords(&[]);
    h.type_text(0, "  Laptop ");
    h.frame();
    h.click("Create");
    h.frame(); // the app sees the click and sends it
    h.frame();
    assert_eq!(h.outstanding::<CreateAppPassword>(), 1);
    assert!(h.has_label("Saving…"));
    assert!(!h.has_button("Create"), "one change at a time");

    h.reply::<CreateAppPassword>(&NewAppPassword {
        info: info("a", "Laptop", None),
        password: "s3cret-app-password".to_owned(),
    });
    h.frame();
    assert!(h.has_text("New password for Laptop:"));
    assert!(h.has_text("s3cret-app-password"));
    assert!(h.has_label("Copy it now. It will not be shown again."));
    // The list is asked for again, to include it.
    assert_eq!(h.outstanding::<ListAppPasswords>(), 1);
    h.reply::<ListAppPasswords>(&vec![info("a", "Laptop", None)]);
    h.frame();
    assert!(h.has_button("Revoke Laptop"));
    assert_eq!(
        h.text_fields(),
        [""],
        "the name is cleared for the next one"
    );

    h.click("Done");
    h.frame(); // the app sees the click
    h.frame();
    assert!(!h.has_text("s3cret-app-password"));
}

#[test]
fn a_new_one_needs_a_name_that_fits() {
    let mut h = with_passwords(&[]);
    h.click("Create");
    h.frame();
    h.frame();
    assert_eq!(h.outstanding::<CreateAppPassword>(), 0);
    assert!(h.has_label("Give it a name first, such as the device it is for."));

    h.type_text(0, &"x".repeat(65));
    h.frame();
    h.click("Create");
    h.frame();
    h.frame();
    assert_eq!(h.outstanding::<CreateAppPassword>(), 0);
    assert!(h.has_label("That name is too long."));
}

#[test]
fn a_failed_create_says_so() {
    let mut h = with_passwords(&[]);
    h.type_text(0, "Laptop");
    h.frame();
    h.click("Create");
    h.frame();
    h.fail::<CreateAppPassword>(rpc_error::SERVER);
    h.frame();
    assert!(h.has_label("Could not create it: server error"));
    h.reply::<ListAppPasswords>(&vec![]);
    h.frame();
    assert!(h.has_label("Could not create it: server error"));
    assert!(h.has_button("Create"));
}

#[test]
fn revoking_one_asks_the_server_and_refreshes_the_list() {
    let mut h = with_passwords(&[info("a", "Laptop", None), info("b", "Phone", None)]);
    h.click("Revoke Phone");
    h.frame(); // the app sees the click and sends it
    h.frame();
    assert_eq!(h.outstanding::<RevokeAppPassword>(), 1);
    assert!(!h.has_button("Revoke Laptop"), "one change at a time");

    h.reply::<RevokeAppPassword>(&());
    h.frame();
    assert_eq!(h.outstanding::<ListAppPasswords>(), 1);
    h.reply::<ListAppPasswords>(&vec![info("a", "Laptop", None)]);
    h.frame();
    assert!(h.has_button("Revoke Laptop"));
    assert!(!h.has_button("Revoke Phone"));
}

#[test]
fn a_failed_revoke_says_so() {
    let mut h = with_passwords(&[info("a", "Laptop", None)]);
    h.click("Revoke Laptop");
    h.frame();
    h.fail::<RevokeAppPassword>(rpc_error::TIMEOUT);
    h.frame();
    assert!(h.has_label("Could not revoke it: timed out"));
}

#[test]
fn at_the_limit_there_is_no_create() {
    let list: Vec<_> = (0..MAX_APP_PASSWORDS)
        .map(|i| info(&format!("{i}"), &format!("device {i}"), None))
        .collect();
    let h = with_passwords(&list);
    assert!(!h.has_button("Create"));
    assert!(h.has_text("That is as many as you can have. Revoke one to make another."));
}

#[test]
fn a_failed_list_can_be_retried() {
    let mut h = showing(signed_in("ollielynas"));
    h.fail::<ListAppPasswords>(rpc_error::TRANSPORT);
    h.frame();
    assert!(h.has_label("Could not load your app passwords:"));
    h.click("Try again");
    h.frame();
    h.frame();
    assert_eq!(h.outstanding::<ListAppPasswords>(), 1);
}

#[test]
fn there_are_no_app_passwords_with_login_off() {
    let h = showing(Me {
        login_enabled: false,
        name: None,
        account_url: None,
    });
    assert!(!h.has_text("App passwords"));
    assert_eq!(h.outstanding::<ListAppPasswords>(), 0);
}
