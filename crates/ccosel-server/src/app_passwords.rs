//! App passwords: one password per device or program, for clients that cannot sign in through
//! the browser.
//!
//! A WebDAV client (Finder, Windows Explorer, davfs2, rclone) can only send a user name and a
//! password with each request; it cannot follow the Keycloak redirect or hold the session
//! cookie. So a signed-in user makes an app password in the Account app, and the client signs
//! in with their login and that. Their Keycloak password never reaches this server or the
//! client, and each device's password can be revoked on its own.
//!
//! Only a SHA-256 of each password is kept. That is enough for these, unlike for passwords
//! people choose: each one is [`PASSWORD_LEN`] random characters (about 190 bits), so there is
//! no dictionary to try and nothing a slow hash would add.
//!
//! The store is one JSON file in the server's data folder, outside the jail, so no file route
//! can reach it. [`AppPasswords::in_memory`] keeps it in memory only, for tests and for a
//! server with login turned off, where nobody can make one anyway.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ccosel_proto::account::{
    AppPasswordInfo, MAX_APP_PASSWORD_NAME, MAX_APP_PASSWORDS, NewAppPassword,
};
use ccosel_proto::server_error;
use rand::Rng;
use rand::distributions::Alphanumeric;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// How long a new password is.
pub const PASSWORD_LEN: usize = 32;

/// `last_used` is written to disk at most this often per password, so a client making a
/// request a second does not rewrite the file a second.
const LAST_USED_GRANULARITY_S: i64 = 60;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Record {
    id: String,
    user: String,
    name: String,
    /// Lower-case hex SHA-256 of the password.
    hash: String,
    created_s: i64,
    last_used_s: Option<i64>,
}

impl Record {
    fn info(&self) -> AppPasswordInfo {
        AppPasswordInfo {
            id: self.id.clone(),
            name: self.name.clone(),
            created_s: self.created_s,
            last_used_s: self.last_used_s,
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
struct StoreFile {
    passwords: Vec<Record>,
}

pub struct AppPasswords {
    /// Where the store is saved. `None` keeps it in memory only.
    file: Option<PathBuf>,
    records: Mutex<Vec<Record>>,
}

impl Default for AppPasswords {
    fn default() -> Self {
        Self::in_memory()
    }
}

fn now_s() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn random(len: usize) -> String {
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(len)
        .map(char::from)
        .collect()
}

fn hash(password: &str) -> String {
    Sha256::digest(password.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Equal without saying, by how long it took, how much of the two agreed.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

impl AppPasswords {
    pub fn in_memory() -> Self {
        Self {
            file: None,
            records: Mutex::new(Vec::new()),
        }
    }

    /// The store saved at `file`, which need not exist yet. A file that exists but cannot be
    /// read is an error rather than an empty store: starting with none would silently revoke
    /// everyone's passwords, and the next save would make that permanent.
    pub fn open(file: PathBuf) -> anyhow::Result<Self> {
        let records = match std::fs::read_to_string(&file) {
            Ok(text) => {
                serde_json::from_str::<StoreFile>(&text)
                    .map_err(|e| anyhow::anyhow!("{}: {e}", file.display()))?
                    .passwords
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => anyhow::bail!("{}: {e}", file.display()),
        };
        Ok(Self {
            file: Some(file),
            records: Mutex::new(records),
        })
    }

    /// Where the store is saved, if anywhere.
    pub fn file(&self) -> Option<&Path> {
        self.file.as_deref()
    }

    /// `user`'s app passwords, oldest first.
    pub fn list(&self, user: &str) -> Vec<AppPasswordInfo> {
        self.records
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.user == user)
            .map(Record::info)
            .collect()
    }

    /// A new app password for `user`, called `name`.
    pub fn create(&self, user: &str, name: &str) -> Result<NewAppPassword, u32> {
        let name = name.trim();
        if name.is_empty()
            || name.len() > MAX_APP_PASSWORD_NAME
            || name.chars().any(char::is_control)
        {
            return Err(server_error::MALFORMED);
        }
        let mut records = self.records.lock().unwrap();
        if records.iter().filter(|r| r.user == user).count() >= MAX_APP_PASSWORDS {
            return Err(server_error::LIMIT);
        }
        let password = random(PASSWORD_LEN);
        let record = Record {
            id: random(12),
            user: user.to_owned(),
            name: name.to_owned(),
            hash: hash(&password),
            created_s: now_s(),
            last_used_s: None,
        };
        let info = record.info();
        records.push(record);
        if let Err(e) = self.save(&records) {
            records.pop();
            eprintln!("app passwords: {e}");
            return Err(server_error::IO);
        }
        Ok(NewAppPassword { info, password })
    }

    /// Revoke `user`'s app password `id`. Someone else's id is `NOT_FOUND`, as if it did not
    /// exist.
    pub fn revoke(&self, user: &str, id: &str) -> Result<(), u32> {
        let mut records = self.records.lock().unwrap();
        let Some(at) = records.iter().position(|r| r.user == user && r.id == id) else {
            return Err(server_error::NOT_FOUND);
        };
        let removed = records.remove(at);
        if let Err(e) = self.save(&records) {
            records.insert(at, removed);
            eprintln!("app passwords: {e}");
            return Err(server_error::IO);
        }
        Ok(())
    }

    /// Whether `password` is one of `user`'s app passwords. A match is noted as used.
    pub fn verify(&self, user: &str, password: &str) -> bool {
        let wanted = hash(password);
        let mut records = self.records.lock().unwrap();
        let Some(record) = records
            .iter_mut()
            .find(|r| r.user == user && same(&r.hash, &wanted))
        else {
            return false;
        };
        let now = now_s();
        let stale = record
            .last_used_s
            .is_none_or(|t| now - t >= LAST_USED_GRANULARITY_S);
        if stale {
            record.last_used_s = Some(now);
            // Only the time of last use is lost if this fails, so the sign-in still counts.
            if let Err(e) = self.save(&records) {
                eprintln!("app passwords: {e}");
            }
        }
        true
    }

    /// Write the store, replacing the old file only once the new one is complete, readable
    /// by this account alone.
    fn save(&self, records: &[Record]) -> anyhow::Result<()> {
        let Some(file) = &self.file else {
            return Ok(());
        };
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = serde_json::to_string_pretty(&StoreFile {
            passwords: records.to_vec(),
        })?;
        let tmp = file.with_extension("json.tmp");
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        std::io::Write::write_all(&mut options.open(&tmp)?, text.as_bytes())?;
        std::fs::rename(&tmp, file)?;
        Ok(())
    }
}

/// Failed sign-ins allowed per key in each [`THROTTLE_WINDOW`].
pub const THROTTLE_FAILURES: u32 = 10;
pub const THROTTLE_WINDOW: Duration = Duration::from_secs(5 * 60);

/// Counts failed sign-ins per key (a login, a client address) and says when one has had too
/// many. A window starts at a key's first failure and the count resets when it ends, so a
/// guesser gets [`THROTTLE_FAILURES`] tries per [`THROTTLE_WINDOW`] per login and per address.
#[derive(Default)]
pub struct Throttle {
    failures: Mutex<HashMap<String, (u32, Instant)>>,
}

impl Throttle {
    /// How long until one of `keys` may try again, if any of them is over the limit.
    pub fn blocked(&self, keys: &[String], now: Instant) -> Option<Duration> {
        let mut failures = self.failures.lock().unwrap();
        failures.retain(|_, (_, start)| now.duration_since(*start) < THROTTLE_WINDOW);
        keys.iter()
            .filter_map(|k| failures.get(k))
            .filter(|(count, _)| *count >= THROTTLE_FAILURES)
            .map(|(_, start)| THROTTLE_WINDOW - now.duration_since(*start))
            .max()
    }

    /// Count a failed sign-in against each of `keys`.
    pub fn fail(&self, keys: &[String], now: Instant) {
        let mut failures = self.failures.lock().unwrap();
        for key in keys {
            let entry = failures.entry(key.clone()).or_insert((0, now));
            if now.duration_since(entry.1) >= THROTTLE_WINDOW {
                *entry = (0, now);
            }
            entry.0 += 1;
        }
    }
}

#[cfg(test)]
mod tests;
