//! The Account app: who you are signed in as, a way to sign out, and your app passwords.
//!
//! Signing out is one RPC. Once it lands the session is gone, so this app asks who is signed
//! in once more: that call is refused, and the shell answers a refusal by going back to the
//! sign-in page. The app never navigates the browser itself.
//!
//! App passwords are what WebDAV clients sign in with (see the user documentation). A new
//! one's password is shown once, here, and never again: the server keeps only a hash of it.

use ccosel_proto::account::{
    CreateAppPassword, CreateAppPasswordReq, ListAppPasswords, MAX_APP_PASSWORD_NAME,
    MAX_APP_PASSWORDS, NewAppPassword, RevokeAppPassword, RevokeAppPasswordReq, SignOut, WhoAmI,
};
use ccosel_sdk::{App, CallId, Poll, Text, TextStyle, Ui};

/// A change to the app passwords that is in flight.
#[derive(Clone, Copy, Debug)]
enum Pending {
    Create(CallId),
    Revoke(CallId),
}

#[derive(Default)]
pub struct Account {
    /// Set by the Sign out button. From then on the app keeps asking for [`SignOut`] until it
    /// lands, like any other query.
    signing_out: bool,
    /// Whether the post-sign-out [`WhoAmI`] has been re-issued. Once is enough: its refusal
    /// is the shell's cue, and asking again every frame would only repeat the refusal.
    rechecked: bool,
    /// The name typed for a new app password.
    new_name: Text,
    pending: Option<Pending>,
    /// An app password just made, with its password, until the user says they have it.
    created: Option<NewAppPassword>,
    /// What went wrong with the last change, if anything did.
    status: Option<String>,
}

/// Where the server serves WebDAV, after its address.
pub const DAV_PATH: &str = "/dav/";

/// Whether a page at `origin` is plain `http://` to somewhere other than this computer: a
/// password sent to it would cross the network unencrypted, and Windows refuses to send one.
pub fn unencrypted_over_network(origin: &str) -> bool {
    let Some(rest) = origin.strip_prefix("http://") else {
        return false;
    };
    let host = match rest.strip_prefix('[') {
        // `[::1]:8777`
        Some(v6) => v6.split(']').next().unwrap_or(v6),
        None => rest.split([':', '/']).next().unwrap_or(rest),
    };
    !matches!(host, "localhost" | "127.0.0.1" | "::1")
}

/// `YYYY-MM-DD` for a time in seconds since the Unix epoch, in UTC. Integer arithmetic only:
/// formatting a float would pull `core::fmt`'s float code into the app's download.
pub fn date(unix_s: i64) -> String {
    // Howard Hinnant's `civil_from_days`.
    let z = unix_s.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

impl Account {
    fn signed_in(&mut self, ui: &mut Ui<'_>, name: &str, account_url: Option<&str>) {
        ui.label("Signed in as");
        ui.label(name);
        ui.separator();
        ui.horizontal(|ui| {
            if let Some(url) = account_url {
                ui.open_url("Manage account", url);
            }
            if ui.button("Sign out").clicked() {
                self.signing_out = true;
            }
        });
        ui.separator();
        self.app_passwords(ui, name);
    }

    /// How to connect a WebDAV client: this page's address with [`DAV_PATH`] on the end.
    fn dav_address(ui: &mut Ui<'_>) {
        let Some(origin) = ui.page().origin() else {
            // A shell too old to say where the page is.
            ui.label("Connect to this server's address with /dav/ on the end.");
            return;
        };
        ui.horizontal(|ui| {
            ui.label("Connect to");
            ui.styled(&format!("{origin}{DAV_PATH}"), TextStyle::CODE);
            ui.copy_link("Copy address", DAV_PATH);
        });
        if unencrypted_over_network(origin) {
            ui.styled(
                "This page is not using HTTPS, so a password sent to this address would cross \
                 the network unencrypted, and Windows will not send it at all. Ask whoever runs \
                 the server to set up HTTPS.",
                TextStyle::WEAK,
            );
        }
    }

    fn app_passwords(&mut self, ui: &mut Ui<'_>, login: &str) {
        self.poll_pending(ui);
        ui.styled("App passwords", TextStyle::heading(2));
        ui.label("For programs that cannot sign in through this page, such as WebDAV.");
        Self::dav_address(ui);

        if let Some(made) = &self.created {
            ui.styled(
                &format!("New password for {}:", made.info.name),
                TextStyle::STRONG,
            );
            ui.styled(&made.password, TextStyle::CODE);
            ui.label(&format!(
                "Sign in with the user name {login} and this password."
            ));
            ui.label("Copy it now. It will not be shown again.");
            if ui.button("Done").clicked() {
                self.created = None;
            }
            ui.separator();
        }
        // Above the list, which is asked for again after every change and shows "Loading…"
        // meanwhile: down there, how the change went would not be seen until it arrived.
        if let Some(status) = &self.status {
            ui.label(status);
        }
        if self.pending.is_some() {
            ui.label("Saving…");
        }

        let count = match ui.rpc().get::<ListAppPasswords>(&()) {
            Poll::Pending => {
                ui.label("Loading…");
                return;
            }
            Poll::Failed(e) => {
                ui.label("Could not load your app passwords:");
                ui.label(e.message());
                if ui.button("Try again").clicked() {
                    ui.rpc().invalidate::<ListAppPasswords>(&());
                }
                return;
            }
            Poll::Ready(list) => {
                if list.is_empty() {
                    ui.styled("You have none.", TextStyle::WEAK);
                }
                for p in list.iter() {
                    ui.push_id(&p.id, |ui| {
                        ui.horizontal(|ui| {
                            ui.styled(&p.name, TextStyle::STRONG);
                            ui.styled(&format!("made {}", date(p.created_s)), TextStyle::WEAK);
                            let used = match p.last_used_s {
                                Some(t) => format!("last used {}", date(t)),
                                None => "never used".to_owned(),
                            };
                            ui.styled(&used, TextStyle::WEAK);
                            if self.pending.is_none()
                                && ui.button(&format!("Revoke {}", p.name)).clicked()
                            {
                                self.status = None;
                                let req = RevokeAppPasswordReq { id: &p.id };
                                let id = ui.rpc().send::<RevokeAppPassword>(&req);
                                self.pending = Some(Pending::Revoke(id));
                            }
                        });
                    });
                }
                list.len()
            }
        };

        if self.pending.is_some() {
            return;
        }
        if count >= MAX_APP_PASSWORDS {
            ui.styled(
                "That is as many as you can have. Revoke one to make another.",
                TextStyle::WEAK,
            );
            return;
        }
        ui.push_id("new", |ui| {
            ui.horizontal(|ui| {
                ui.label("Name");
                ui.text_edit(&mut self.new_name);
                if ui.button("Create").clicked() {
                    self.create(ui);
                }
            });
        });
    }

    fn create(&mut self, ui: &mut Ui<'_>) {
        let name = self.new_name.as_str().trim();
        if name.is_empty() {
            self.status = Some("Give it a name first, such as the device it is for.".to_owned());
            return;
        }
        if name.len() > MAX_APP_PASSWORD_NAME {
            self.status = Some("That name is too long.".to_owned());
            return;
        }
        self.status = None;
        let id = ui
            .rpc()
            .send::<CreateAppPassword>(&CreateAppPasswordReq { name });
        self.pending = Some(Pending::Create(id));
    }

    /// Check on the change in flight, if any, and act on how it went.
    fn poll_pending(&mut self, ui: &mut Ui<'_>) {
        let rpc = ui.rpc();
        let failed = match self.pending {
            None => return,
            Some(Pending::Create(id)) => match rpc.outcome::<CreateAppPassword>(id) {
                Poll::Pending => return,
                Poll::Ready(made) => {
                    self.created = Some((*made).clone());
                    self.new_name.set("");
                    None
                }
                Poll::Failed(e) => Some(format!("Could not create it: {}", e.message())),
            },
            Some(Pending::Revoke(id)) => match rpc.outcome::<RevokeAppPassword>(id) {
                Poll::Pending => return,
                Poll::Ready(_) => None,
                Poll::Failed(e) => Some(format!("Could not revoke it: {}", e.message())),
            },
        };
        self.pending = None;
        self.status = failed;
        rpc.invalidate::<ListAppPasswords>(&());
    }
}

impl App for Account {
    fn update(&mut self, ui: &mut Ui<'_>) {
        if self.signing_out {
            match ui.rpc().get::<SignOut>(&()) {
                Poll::Pending => {
                    ui.label("Signing out…");
                }
                Poll::Failed(e) => {
                    ui.label("Could not sign out:");
                    ui.label(e.message());
                    if ui.button("Try again").clicked() {
                        ui.rpc().invalidate::<SignOut>(&());
                    }
                }
                Poll::Ready(_) => {
                    ui.label("Signed out.");
                    // The refusal of this call is what sends the shell to the sign-in page.
                    if !self.rechecked {
                        ui.rpc().invalidate::<WhoAmI>(&());
                        self.rechecked = true;
                    }
                    let _ = ui.rpc().get::<WhoAmI>(&());
                }
            }
            return;
        }

        match ui.rpc().get::<WhoAmI>(&()) {
            Poll::Pending => {
                ui.label("Loading…");
            }
            Poll::Failed(e) => {
                ui.label(e.message());
            }
            Poll::Ready(account) => match (account.login_enabled, account.name.as_deref()) {
                (true, Some(name)) => self.signed_in(ui, name, account.account_url.as_deref()),
                (true, None) => {
                    ui.label("Not signed in.");
                }
                (false, _) => {
                    ui.label("Login is turned off on this server.");
                    ui.label("Everyone on the network gets in.");
                }
            },
        }
    }
}

#[cfg(test)]
mod tests;

ccosel_sdk::ccosel_app!(Account);
