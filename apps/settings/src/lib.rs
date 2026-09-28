//! The Settings app: theme, background, motion and data use, the dock, and switching account.
//!
//! Every control here takes effect. The app keeps a copy of the settings, changes it, and
//! sends the whole thing with `SetSettings`. The shell sees that call on its way out and
//! applies it the same frame; the server stores it in the user's home folder (or, with login
//! off, the shell keeps it in the browser). So the app never waits to show a change, and it
//! never has to reload what it just saved.

use ccosel_proto::account::{SignOut, WhoAmI};
use ccosel_proto::settings::{
    BackgroundChoice, GetSettings, ListApps, SetSettings, Settings as Prefs, ThemeChoice,
};
use ccosel_sdk::{App, CallId, Poll, Text, Ui};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
enum Section {
    #[default]
    Appearance,
    Performance,
    Dock,
    Account,
}

impl Section {
    const ALL: [Section; 4] = [
        Section::Appearance,
        Section::Performance,
        Section::Dock,
        Section::Account,
    ];

    fn label(self) -> &'static str {
        match self {
            Section::Appearance => "Appearance",
            Section::Performance => "Performance",
            Section::Dock => "Dock",
            Section::Account => "Account",
        }
    }
}

const THEMES: [(ThemeChoice, &str); 3] = [
    (ThemeChoice::System, "System"),
    (ThemeChoice::Light, "Light"),
    (ThemeChoice::Dark, "Dark"),
];

/// Each background, its button, and the one line saying what it costs.
const BACKGROUNDS: [(BackgroundChoice, &str, &str); 4] = [
    (
        BackgroundChoice::Auto,
        "Automatic",
        "The photo on a good connection, plain otherwise.",
    ),
    (
        BackgroundChoice::Photo,
        "Photo",
        "Always the photo. Uses more data.",
    ),
    (
        BackgroundChoice::Plain,
        "Plain",
        "Drawn shapes. Uses no data.",
    ),
    (
        BackgroundChoice::Custom,
        "My picture",
        "A picture from your files. Uses more data.",
    ),
];

#[derive(Default)]
pub struct Settings {
    section: Section,
    /// The settings as last sent. `None` until they have loaded.
    prefs: Option<Prefs>,
    /// The path typed for "My picture", applied with its Use button rather than per keystroke.
    picture: Text,
    /// Saves not yet answered. Each outcome is collected, so none is left behind.
    saving: Vec<CallId>,
    save_failed: Option<&'static str>,
    signing_out: bool,
    /// Whether `WhoAmI` has been re-asked after signing out. Its refusal is the shell's cue
    /// to go back to the sign-in page, and once is enough.
    rechecked: bool,
}

impl App for Settings {
    fn update(&mut self, ui: &mut Ui<'_>) {
        self.collect_saves(ui);

        if self.prefs.is_none() {
            match ui.rpc().get::<GetSettings>(&()) {
                Poll::Pending => {
                    ui.label("Loading…");
                    return;
                }
                Poll::Failed(e) => {
                    ui.label("Could not load your settings:");
                    ui.label(e.message());
                    if ui.button("Try again").clicked() {
                        ui.rpc().invalidate::<GetSettings>(&());
                    }
                    return;
                }
                Poll::Ready(prefs) => {
                    self.picture.set(&prefs.custom_image);
                    self.prefs = Some((*prefs).clone());
                }
            }
        }

        // Under their own id salt, as is the section below: two scopes that both sit at the
        // top level can otherwise hand out colliding widget ids.
        ui.push_id("tabs", |ui| {
            ui.horizontal(|ui| {
                for section in Section::ALL {
                    if ui
                        .selectable(section == self.section, section.label())
                        .clicked()
                    {
                        self.section = section;
                    }
                }
            });
        });
        ui.separator();

        let before = self.prefs.clone();
        let section = self.section;
        ui.push_id(section.label(), |ui| match section {
            Section::Appearance => self.appearance(ui),
            Section::Performance => self.performance(ui),
            Section::Dock => self.dock(ui),
            Section::Account => self.account(ui),
        });
        if self.prefs != before
            && let Some(prefs) = &self.prefs
        {
            self.saving.push(ui.rpc().send::<SetSettings>(prefs));
            self.save_failed = None;
        }

        if let Some(why) = self.save_failed {
            ui.separator();
            ui.label("Couldn't save that change:");
            ui.label(why);
            ui.label("It applies until you reload.");
        }
    }
}

impl Settings {
    fn collect_saves(&mut self, ui: &mut Ui<'_>) {
        let rpc = ui.rpc();
        let mut failed = None;
        self.saving
            .retain(|&id| match rpc.outcome::<SetSettings>(id) {
                Poll::Pending => true,
                Poll::Ready(_) => false,
                Poll::Failed(e) => {
                    failed = Some(e.message());
                    false
                }
            });
        if failed.is_some() {
            self.save_failed = failed;
        }
    }

    fn appearance(&mut self, ui: &mut Ui<'_>) {
        let Some(prefs) = self.prefs.as_mut() else {
            return;
        };

        ui.label("Theme");
        ui.horizontal(|ui| {
            for (choice, label) in THEMES {
                if ui.selectable(prefs.theme == choice, label).clicked() {
                    prefs.theme = choice;
                }
            }
        });
        ui.separator();

        ui.label("Background");
        ui.horizontal(|ui| {
            for (choice, label, _) in BACKGROUNDS {
                if ui.selectable(prefs.background == choice, label).clicked() {
                    prefs.background = choice;
                }
            }
        });
        if let Some((_, _, hint)) = BACKGROUNDS.iter().find(|b| b.0 == prefs.background) {
            ui.label(hint);
        }
        if prefs.background == BackgroundChoice::Custom {
            ui.horizontal(|ui| {
                ui.text_edit(&mut self.picture);
                if ui.button("Use").clicked() {
                    prefs.custom_image = self.picture.as_str().trim().to_owned();
                }
            });
            ui.label("A path such as /home/you/beach.jpg");
        }
        if prefs.save_data && prefs.background != BackgroundChoice::Plain {
            ui.label("Save data is on, so no picture is shown.");
        }
    }

    fn performance(&mut self, ui: &mut Ui<'_>) {
        let Some(prefs) = self.prefs.as_mut() else {
            return;
        };
        if ui
            .selectable(prefs.reduce_motion, "Reduce motion")
            .clicked()
        {
            prefs.reduce_motion = !prefs.reduce_motion;
        }
        ui.label("Turns animations off. Less smooth.");
        ui.separator();
        if ui.selectable(prefs.save_data, "Save data").clicked() {
            prefs.save_data = !prefs.save_data;
        }
        ui.label("No background picture, and apps load less. Plainer.");
    }

    fn dock(&mut self, ui: &mut Ui<'_>) {
        ui.label("Pinned apps stay on the dock.");
        let apps = match ui.rpc().get::<ListApps>(&()) {
            Poll::Pending => {
                ui.label("Loading…");
                return;
            }
            Poll::Failed(e) => {
                ui.label(e.message());
                return;
            }
            Poll::Ready(apps) => apps,
        };
        let Some(prefs) = self.prefs.as_mut() else {
            return;
        };
        for app in apps.iter() {
            let pinned = prefs.pinned.contains(&app.id);
            let row = format!("{}  {}", app.icon, app.name);
            if ui.selectable(pinned, &row).clicked() {
                if pinned {
                    prefs.pinned.retain(|id| id != &app.id);
                } else {
                    prefs.pinned.push(app.id.clone());
                }
            }
        }
    }

    fn account(&mut self, ui: &mut Ui<'_>) {
        if self.signing_out {
            match ui.rpc().get::<SignOut>(&()) {
                Poll::Pending => ui.label("Signing out…"),
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
            Poll::Pending => ui.label("Loading…"),
            Poll::Failed(e) => ui.label(e.message()),
            Poll::Ready(me) => match (me.login_enabled, me.name.as_deref()) {
                (true, Some(name)) => {
                    ui.label("Signed in as");
                    ui.label(name);
                    if ui.button("Switch account").clicked() {
                        self.signing_out = true;
                    }
                    ui.label("Signs you out, so you can sign in as someone else.");
                }
                (true, None) => ui.label("Not signed in."),
                (false, _) => {
                    ui.label("Login is turned off on this server, so there are no accounts.");
                    ui.label("Your settings are kept in this browser.");
                }
            },
        }
    }
}

#[cfg(test)]
mod tests;

ccosel_sdk::ccosel_app!(Settings);
