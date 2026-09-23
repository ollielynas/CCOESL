//! The Settings app.
//!
//! What this app can actually do is bounded by what the shell exposes to a guest today: a
//! read-only theme flag ([`ccosel_sdk::FrameCtx::dark_mode`]) and nothing else — no RPC to
//! persist a preference, no channel back into the desktop's wallpaper, taskbar or account
//! state. So every section here is real UI over the real, currently-available surface, but
//! **every value it holds is local to this window and forgotten when it closes**: there is no
//! wire format yet for a guest to hand a preference back to the shell or the server. Each
//! section says so, briefly, rather than only in a doc comment nobody using the app will read.
//!
//! Sections and what backs them:
//! - **Appearance**: the dark/light switch reflects the *real* flag the shell sends every
//!   frame, but flipping it here cannot reach the shell (no such call exists), so the button
//!   only tracks a local override and says as much. The background field is entirely
//!   aspirational — the desktop currently paints a fixed gradient, not an image — so this is a
//!   text field the app remembers for itself, nothing more.
//! - **Performance**: framed the way the ticket asks — by what the user wants, not the knob's
//!   name — but there is no rendering-config call to send either, so these are local toggles.
//! - **Dock**: which apps should show. The dock is the shell's taskbar, drawn from
//!   `registry::catalog()`, which this app cannot see (it's a shell-crate type, and a guest
//!   cannot depend on `ccosel-shell`) or influence (no such RPC exists yet). The two entries
//!   below mirror today's catalog by name so the control means something, but toggling them
//!   changes only this app's own state.
//! - **Account**: there is no account system in this codebase yet (see `README.md`'s "user
//!   accounts (minimal auth for now)" under future work), so this is a local list this window
//!   keeps for itself, seeded with a single default account.

use ccosel_sdk::{App, Text, Ui};

#[derive(Clone, Copy, PartialEq, Eq, Default)]
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

/// One entry the Dock section can show or hide.
///
/// Mirrors `crates/ccosel-shell/src/registry.rs::catalog()` by name only — this app cannot
/// depend on `ccosel-shell` (apps and the shell are deliberately separate) and there is no RPC
/// that reports the real catalog to a guest, so the list is hand-kept and, like everything
/// else here, purely local.
struct DockItem {
    name: &'static str,
    icon: &'static str,
    shown: bool,
}

pub struct Settings {
    section: Section,
    /// Local-only override of the theme. `None` means "follow the shell's real flag".
    dark_override: Option<bool>,
    background_url: Text,
    /// "Prevent screen tearing" — the user-facing name for vsync. Trade-off: caps the frame
    /// rate to the display's refresh rate.
    prevent_tearing: bool,
    /// "Save battery" — reduces how often idle windows redraw. Trade-off: animations look less
    /// smooth.
    save_battery: bool,
    dock: [DockItem; 2],
    accounts: Vec<String>,
    active_account: usize,
    new_account: Text,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            section: Section::default(),
            dark_override: None,
            background_url: Text::new(""),
            prevent_tearing: true,
            save_battery: false,
            dock: [
                DockItem {
                    name: "Files",
                    icon: "🗀",
                    shown: true,
                },
                DockItem {
                    name: "Clock",
                    icon: "◴",
                    shown: true,
                },
            ],
            accounts: vec!["Guest".to_owned()],
            active_account: 0,
            new_account: Text::new(""),
        }
    }
}

impl App for Settings {
    fn update(&mut self, ui: &mut Ui<'_>) {
        ui.label("Settings");
        ui.separator();

        ui.horizontal(|ui| {
            for section in Section::ALL {
                ui.push_id(section.label(), |ui| {
                    let label = if section == self.section {
                        format!("▸ {}", section.label())
                    } else {
                        section.label().to_owned()
                    };
                    if ui.button(&label).clicked() {
                        self.section = section;
                    }
                });
            }
        });
        ui.separator();

        match self.section {
            Section::Appearance => self.appearance(ui),
            Section::Performance => self.performance(ui),
            Section::Dock => self.dock(ui),
            Section::Account => self.account(ui),
        }
    }
}

impl Settings {
    fn appearance(&mut self, ui: &mut Ui<'_>) {
        let dark = self.dark_override.unwrap_or_else(|| ui.ctx().dark_mode());
        ui.horizontal(|ui| {
            let label = if dark { "🌙 Dark" } else { "☀ Light" };
            if ui.button(label).clicked() {
                self.dark_override = Some(!dark);
            }
        });
        ui.label("Not applied yet — the shell doesn't accept a theme override from an app.");
        ui.separator();

        ui.label("Background image URL");
        ui.text_edit(&mut self.background_url);
        ui.label("Not applied yet — the desktop paints a fixed gradient, not an image.");
    }

    fn performance(&mut self, ui: &mut Ui<'_>) {
        ui.horizontal(|ui| {
            let label = if self.prevent_tearing {
                "✓ Prevent screen tearing"
            } else {
                "Prevent screen tearing"
            };
            if ui.button(label).clicked() {
                self.prevent_tearing = !self.prevent_tearing;
            }
        });
        ui.label("Trade-off: caps the frame rate.");
        ui.separator();

        ui.horizontal(|ui| {
            let label = if self.save_battery {
                "✓ Save battery"
            } else {
                "Save battery"
            };
            if ui.button(label).clicked() {
                self.save_battery = !self.save_battery;
            }
        });
        ui.label("Trade-off: animations look less smooth.");
        ui.separator();
        ui.label("Not applied yet — there is no rendering setting a guest can change.");
    }

    fn dock(&mut self, ui: &mut Ui<'_>) {
        for item in &mut self.dock {
            ui.push_id(item.name, |ui| {
                ui.horizontal(|ui| {
                    let label = if item.shown {
                        format!("✓ {} {}", item.icon, item.name)
                    } else {
                        format!("{} {}", item.icon, item.name)
                    };
                    if ui.button(&label).clicked() {
                        item.shown = !item.shown;
                    }
                });
            });
        }
        ui.separator();
        ui.label("Not applied yet — the taskbar always shows every installed app.");
    }

    fn account(&mut self, ui: &mut Ui<'_>) {
        for (i, name) in self.accounts.iter().enumerate() {
            ui.push_id(name.as_str(), |ui| {
                ui.horizontal(|ui| {
                    let label = if i == self.active_account {
                        format!("▸ {name}")
                    } else {
                        name.clone()
                    };
                    if ui.button(&label).clicked() {
                        self.active_account = i;
                    }
                });
            });
        }
        ui.separator();

        ui.horizontal(|ui| {
            ui.label("New account");
            ui.text_edit(&mut self.new_account);
            if ui.button("Add").clicked() {
                let name = self.new_account.as_str().trim();
                if !name.is_empty() {
                    self.accounts.push(name.to_owned());
                    self.active_account = self.accounts.len() - 1;
                    self.new_account.set("");
                }
            }
        });
        ui.label("Not applied yet — there is no account system to switch yet.");
    }
}

#[cfg(test)]
mod tests;

ccosel_sdk::ccosel_app!(Settings);
