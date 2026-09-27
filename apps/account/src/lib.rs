//! The Account app: who you are signed in as, and a way to sign out.
//!
//! Signing out is one RPC. Once it lands the session is gone, so this app asks who is signed
//! in once more: that call is refused, and the shell answers a refusal by going back to the
//! sign-in page. The app never navigates the browser itself.

use ccosel_proto::account::{SignOut, WhoAmI};
use ccosel_sdk::{App, Poll, Ui};

#[derive(Default)]
pub struct Account {
    /// Set by the Sign out button. From then on the app keeps asking for [`SignOut`] until it
    /// lands, like any other query.
    signing_out: bool,
    /// Whether the post-sign-out [`WhoAmI`] has been re-issued. Once is enough: its refusal
    /// is the shell's cue, and asking again every frame would only repeat the refusal.
    rechecked: bool,
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
                (true, Some(name)) => {
                    ui.label("Signed in as");
                    ui.label(name);
                    ui.separator();
                    if ui.button("Sign out").clicked() {
                        self.signing_out = true;
                    }
                }
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
