//! Driven through the SDK harness, which plays both the shell (answering `GetSettings` and
//! `ListApps`, as the real shell does) and the server (`SetSettings`, `WhoAmI`, `SignOut`).

use ccosel_proto::account::Account as Me;
use ccosel_proto::settings::AppInfo;
use ccosel_sdk::testing::{Harness, rpc_error};

use super::*;

/// A Settings window that has loaded `prefs`.
fn loaded(prefs: Prefs) -> Harness<Settings> {
    let mut h = Harness::new(Settings::default());
    h.frame();
    assert!(h.has_label("Loading…"));
    h.reply::<GetSettings>(&prefs);
    h.frame();
    h
}

fn prefs(h: &Harness<Settings>) -> &Prefs {
    h.app.prefs.as_ref().expect("settings have loaded")
}

/// Click `text`, and let the app see it.
fn press(h: &mut Harness<Settings>, text: &str) {
    h.click(text);
    h.frame();
}

fn selected(h: &Harness<Settings>, text: &str) -> bool {
    h.selectables()
        .into_iter()
        .find(|(t, _)| t == text)
        .unwrap_or_else(|| panic!("no row {text:?} in {:?}", h.selectables()))
        .1
}

#[test]
fn opens_on_appearance_with_what_is_saved() {
    let h = loaded(Prefs {
        theme: ThemeChoice::Dark,
        ..Prefs::default()
    });
    assert!(selected(&h, "Appearance"));
    assert!(selected(&h, "Dark"));
    assert!(!selected(&h, "System"));
    assert!(selected(&h, "Automatic"));
    assert!(h.has_label("The photo on a good connection, plain otherwise."));
    assert_eq!(h.outstanding::<SetSettings>(), 0);
}

#[test]
fn a_failed_load_can_be_retried() {
    let mut h = Harness::new(Settings::default());
    h.frame();
    h.fail::<GetSettings>(rpc_error::TIMEOUT);
    h.frame();
    assert!(h.has_label("Could not load your settings:"));
    assert!(h.has_label("timed out"));

    press(&mut h, "Try again");
    h.frame();
    assert_eq!(h.outstanding::<GetSettings>(), 1);
    h.reply::<GetSettings>(&Prefs::default());
    h.frame();
    assert!(h.app.prefs.is_some());
}

#[test]
fn choosing_a_theme_saves_it_once() {
    let mut h = loaded(Prefs::default());
    press(&mut h, "Light");
    assert_eq!(prefs(&h).theme, ThemeChoice::Light);
    assert_eq!(h.outstanding::<SetSettings>(), 1);

    // Nothing changed since, so nothing more is sent.
    h.frame();
    assert_eq!(h.outstanding::<SetSettings>(), 1);
    h.reply::<SetSettings>(&());
    h.frame();
    assert!(h.app.saving.is_empty());
    assert!(selected(&h, "Light"));
}

#[test]
fn backgrounds_say_what_they_cost() {
    let mut h = loaded(Prefs::default());
    press(&mut h, "Photo");
    assert_eq!(prefs(&h).background, BackgroundChoice::Photo);
    h.frame();
    assert!(h.has_label("Always the photo. Uses more data."));

    press(&mut h, "Plain");
    h.frame();
    assert!(h.has_label("Drawn shapes. Uses no data."));
    assert_eq!(h.outstanding::<SetSettings>(), 2);
}

#[test]
fn my_picture_is_applied_with_use_not_per_keystroke() {
    let mut h = loaded(Prefs::default());
    press(&mut h, "My picture");
    h.frame();
    assert_eq!(h.outstanding::<SetSettings>(), 1);

    h.type_text(0, " /home/alice/beach.jpg ");
    h.frame();
    assert_eq!(h.outstanding::<SetSettings>(), 1);
    assert_eq!(prefs(&h).custom_image, "");

    press(&mut h, "Use");
    assert_eq!(prefs(&h).custom_image, "/home/alice/beach.jpg");
    assert_eq!(h.outstanding::<SetSettings>(), 2);
}

#[test]
fn a_saved_picture_path_is_shown_for_editing() {
    let mut h = loaded(Prefs {
        background: BackgroundChoice::Custom,
        custom_image: "/home/alice/beach.jpg".to_owned(),
        ..Prefs::default()
    });
    h.frame();
    assert_eq!(h.text_fields(), ["/home/alice/beach.jpg"]);
}

#[test]
fn save_data_explains_why_there_is_no_picture() {
    let h = loaded(Prefs {
        background: BackgroundChoice::Photo,
        save_data: true,
        ..Prefs::default()
    });
    assert!(h.has_label("Save data is on, so no picture is shown."));
}

#[test]
fn performance_toggles_state_their_trade_off() {
    let mut h = loaded(Prefs::default());
    press(&mut h, "Performance");
    assert!(h.has_label("Turns animations off. Less smooth."));
    assert!(!selected(&h, "Reduce motion"));

    press(&mut h, "Reduce motion");
    assert!(prefs(&h).reduce_motion);
    press(&mut h, "Save data");
    assert!(prefs(&h).save_data);
    press(&mut h, "Save data");
    assert!(!prefs(&h).save_data);
    assert_eq!(h.outstanding::<SetSettings>(), 3);
}

fn apps() -> Vec<AppInfo> {
    ["Files", "Clock"]
        .iter()
        .map(|name| AppInfo {
            id: name.to_lowercase(),
            name: (*name).to_owned(),
            icon: "*".to_owned(),
        })
        .collect()
}

#[test]
fn pinning_and_unpinning_apps() {
    let mut h = loaded(Prefs::default());
    press(&mut h, "Dock");
    assert!(h.has_label("Loading…"));
    h.reply::<ListApps>(&apps());
    h.frame();
    assert_eq!(
        h.selectables()[4..],
        [
            ("*  Files".to_owned(), false),
            ("*  Clock".to_owned(), false)
        ]
    );

    press(&mut h, "*  Clock");
    press(&mut h, "*  Files");
    assert_eq!(prefs(&h).pinned, ["clock", "files"]);

    press(&mut h, "*  Clock");
    assert_eq!(prefs(&h).pinned, ["files"]);
    assert_eq!(h.outstanding::<SetSettings>(), 3);
}

#[test]
fn the_dock_says_when_it_cannot_list_apps() {
    let mut h = loaded(Prefs::default());
    press(&mut h, "Dock");
    h.fail::<ListApps>(rpc_error::TIMEOUT);
    h.frame();
    assert!(h.has_label("timed out"));
}

#[test]
fn a_failed_save_is_reported_and_cleared_by_the_next() {
    let mut h = loaded(Prefs::default());
    press(&mut h, "Dark");
    h.fail::<SetSettings>(rpc_error::SERVER);
    h.frame();
    assert!(h.has_label("Couldn't save that change:"));
    assert!(h.has_label("server error"));
    // Still applied locally: the shell already has it.
    assert_eq!(prefs(&h).theme, ThemeChoice::Dark);

    press(&mut h, "Light");
    assert!(!h.has_label("Couldn't save that change:"));
}

fn signed_in(name: &str) -> Me {
    Me {
        login_enabled: true,
        name: Some(name.to_owned()),
        account_url: None,
    }
}

#[test]
fn switching_account_signs_out_then_lets_the_shell_take_over() {
    let mut h = loaded(Prefs::default());
    press(&mut h, "Account");
    assert!(h.has_label("Loading…"));
    h.reply::<WhoAmI>(&signed_in("alice"));
    h.frame();
    assert!(h.has_label("alice"));

    press(&mut h, "Switch account");
    h.frame();
    assert!(h.has_label("Signing out…"));
    h.reply::<SignOut>(&());
    h.frame();
    assert!(h.has_label("Signed out."));
    // Asks once more; the refusal is what sends the shell to the sign-in page.
    assert_eq!(h.outstanding::<WhoAmI>(), 1);
    h.fail::<WhoAmI>(rpc_error::DENIED);
    h.frame();
    assert_eq!(h.outstanding::<WhoAmI>(), 0);
}

#[test]
fn a_failed_sign_out_can_be_retried() {
    let mut h = loaded(Prefs::default());
    press(&mut h, "Account");
    h.reply::<WhoAmI>(&signed_in("alice"));
    h.frame();
    press(&mut h, "Switch account");
    h.frame();
    h.fail::<SignOut>(rpc_error::TIMEOUT);
    h.frame();
    assert!(h.has_label("Could not sign out:"));
    press(&mut h, "Try again");
    h.frame();
    assert_eq!(h.outstanding::<SignOut>(), 1);
}

#[test]
fn no_accounts_when_login_is_off() {
    let mut h = loaded(Prefs::default());
    press(&mut h, "Account");
    h.reply::<WhoAmI>(&Me {
        login_enabled: false,
        name: None,
        account_url: None,
    });
    h.frame();
    assert!(h.has_label("Login is turned off on this server, so there are no accounts."));
    assert!(!h.has_button("Switch account"));
}

#[test]
fn account_errors_and_nobody_signed_in() {
    let mut h = loaded(Prefs::default());
    press(&mut h, "Account");
    h.fail::<WhoAmI>(rpc_error::TIMEOUT);
    h.frame();
    assert!(h.has_label("timed out"));

    let mut h = loaded(Prefs::default());
    press(&mut h, "Account");
    h.reply::<WhoAmI>(&Me {
        login_enabled: true,
        name: None,
        account_url: None,
    });
    h.frame();
    assert!(h.has_label("Not signed in."));
}

/// Changing section must not also press whatever the new section draws where the tab was:
/// top-level scopes can share widget ids unless the app salts them apart.
#[test]
fn switching_section_changes_no_setting() {
    let mut h = loaded(Prefs::default());
    for section in ["Performance", "Dock", "Account", "Appearance"] {
        press(&mut h, section);
    }
    assert_eq!(prefs(&h), &Prefs::default());
    assert_eq!(h.outstanding::<SetSettings>(), 0);
}
