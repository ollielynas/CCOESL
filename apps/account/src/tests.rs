//! Driven through the SDK harness, which plays the server: it answers `WhoAmI` and `SignOut`
//! the way `/rpc` would, including refusing `WhoAmI` once the session is gone.

use ccosel_proto::account::{Account as Me, SignOut, WhoAmI};
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
        h.links(),
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
    assert!(h.links().is_empty());
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
