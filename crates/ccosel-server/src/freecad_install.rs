//! Installing FreeCAD on a server that has none, the first time the Modeller needs it.
//!
//! The official FreeCAD AppImage is downloaded, **checked against a SHA256 pinned in this
//! file** before anything in it runs, and unpacked with its own `--appimage-extract` (no FUSE,
//! no root). It goes outside the file jail, so no user can browse, download or replace it.
//!
//! It happens on first use rather than at start-up because it is most of a gigabyte: a server
//! nobody models on never spends it. The Modeller shows the progress meanwhile.
//! `CCOSEL_FREECAD_AUTO_INSTALL=0` turns it off; a FreeCAD already on `PATH`, or named by
//! `CCOSEL_FREECADCMD`, is always preferred.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use sha2::{Digest, Sha256};

/// The FreeCAD release installed. Changing it means changing both checksums, taken from the
/// release's own `-SHA256.txt` files.
pub const VERSION: &str = "1.1.3";

const X86_64_SHA256: &str = "3a853eb69ee595f779f2255dbf80a765926981d8ff68903cefee4dfb03a8f5ef";
const AARCH64_SHA256: &str = "9a8f9f7f2802bb856f2bb70f53d536e2ae06569f4e6d718407803076104ff55e";

/// Where an install is up to. Shared with the rebuild jobs that wait on it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    Idle,
    Downloading { done: u64, total: u64 },
    Unpacking,
    Ready(PathBuf),
    Failed(String),
}

impl State {
    /// What a waiting user is told.
    pub fn describe(&self) -> String {
        match self {
            Self::Idle => "Installing FreeCAD on the server…".to_owned(),
            Self::Downloading { done, total } if *total > 0 => format!(
                "Installing FreeCAD on the server: downloading, {}% of {} MB",
                done * 100 / total,
                total / 1_000_000
            ),
            Self::Downloading { .. } => "Installing FreeCAD on the server: downloading…".to_owned(),
            Self::Unpacking => "Installing FreeCAD on the server: unpacking…".to_owned(),
            Self::Ready(_) => "FreeCAD is installed".to_owned(),
            Self::Failed(e) => format!("Couldn't install FreeCAD on the server: {e}"),
        }
    }
}

/// What to download, what it must hash to, and where it goes.
#[derive(Clone, Debug)]
pub struct Installer {
    pub url: String,
    pub sha256: String,
    /// Holds the unpacked FreeCAD (`squashfs-root/`) once installed.
    pub dir: PathBuf,
    pub state: Arc<Mutex<State>>,
}

impl Installer {
    /// The pinned official release for this machine, into `dir`, or `None` where there is no
    /// AppImage to be had (not Linux, or not x86_64/ARM64).
    pub fn pinned(dir: &Path) -> Option<Self> {
        if !cfg!(target_os = "linux") {
            return None;
        }
        let (arch, sha) = match std::env::consts::ARCH {
            "x86_64" => ("x86_64", X86_64_SHA256),
            "aarch64" => ("aarch64", AARCH64_SHA256),
            _ => return None,
        };
        Some(Self::new(
            format!(
                "https://github.com/FreeCAD/FreeCAD/releases/download/{VERSION}/\
                 FreeCAD_{VERSION}-Linux-{arch}-py311.AppImage"
            ),
            sha.to_owned(),
            dir.join(VERSION),
        ))
    }

    pub fn new(url: String, sha256: String, dir: PathBuf) -> Self {
        let installer = Self {
            url,
            sha256: sha256.to_ascii_lowercase(),
            dir,
            state: Arc::new(Mutex::new(State::Idle)),
        };
        if let Some(cmd) = installer.installed() {
            *installer.state.lock().unwrap() = State::Ready(cmd);
        }
        installer
    }

    /// Where the server keeps FreeCAD: `CCOSEL_FREECAD_DIR`, else under `XDG_DATA_HOME` or
    /// `~/.local/share`. `None` if auto-install is turned off.
    pub fn default_dir() -> Option<PathBuf> {
        if std::env::var("CCOSEL_FREECAD_AUTO_INSTALL").is_ok_and(|v| v.trim() == "0") {
            return None;
        }
        if let Some(d) = std::env::var_os("CCOSEL_FREECAD_DIR") {
            return Some(PathBuf::from(d));
        }
        let data = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))?;
        Some(data.join("ccosel").join("freecad"))
    }

    /// `freecadcmd` from a finished install, if there is one.
    pub fn installed(&self) -> Option<PathBuf> {
        let cmd = self.dir.join("squashfs-root/usr/bin/freecadcmd");
        cmd.is_file().then_some(cmd)
    }

    pub fn state(&self) -> State {
        self.state.lock().unwrap().clone()
    }

    /// Start installing, unless an install is already running or done. A failed one is tried
    /// again. Needs a tokio runtime, which every request handler has.
    pub fn start(&self) {
        {
            let mut state = self.state.lock().unwrap();
            match &*state {
                State::Idle | State::Failed(_) => *state = State::Downloading { done: 0, total: 0 },
                _ => return,
            }
        }
        let Ok(rt) = tokio::runtime::Handle::try_current() else {
            *self.state.lock().unwrap() = State::Failed("no runtime to install on".to_owned());
            return;
        };
        let me = self.clone();
        rt.spawn(async move {
            let outcome = me.install().await;
            let mut state = me.state.lock().unwrap();
            *state = match outcome {
                Ok(cmd) => State::Ready(cmd),
                Err(e) => {
                    eprintln!("freecad: install failed: {e}");
                    State::Failed(e)
                }
            };
        });
    }

    async fn install(&self) -> Result<PathBuf, String> {
        std::fs::create_dir_all(&self.dir).map_err(|e| format!("{}: {e}", self.dir.display()))?;
        let image = self.dir.join("FreeCAD.AppImage");
        let part = self.dir.join("FreeCAD.AppImage.part");
        let result = self.download(&part).await;
        if let Err(e) = result {
            let _ = std::fs::remove_file(&part);
            return Err(e);
        }
        std::fs::rename(&part, &image).map_err(|e| e.to_string())?;

        *self.state.lock().unwrap() = State::Unpacking;
        let me = self.clone();
        let unpacked = tokio::task::spawn_blocking(move || me.unpack(&image))
            .await
            .map_err(|e| e.to_string())?;
        let _ = std::fs::remove_file(self.dir.join("FreeCAD.AppImage"));
        unpacked
    }

    /// Download to `part`, hashing as it goes, and refuse it unless the hash is the pinned one.
    async fn download(&self, part: &Path) -> Result<(), String> {
        let client = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| e.to_string())?;
        let mut resp = client
            .get(&self.url)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|e| format!("downloading {}: {e}", self.url))?;
        let total = resp.content_length().unwrap_or(0);
        let mut file = std::fs::File::create(part).map_err(|e| e.to_string())?;
        let mut hasher = Sha256::new();
        let mut done = 0u64;
        while let Some(chunk) = resp
            .chunk()
            .await
            .map_err(|e| format!("download cut off: {e}"))?
        {
            hasher.update(&chunk);
            file.write_all(&chunk)
                .map_err(|e| format!("writing FreeCAD: {e}"))?;
            done += chunk.len() as u64;
            *self.state.lock().unwrap() = State::Downloading { done, total };
        }
        file.sync_all().map_err(|e| e.to_string())?;
        let got = hex(&hasher.finalize());
        if got != self.sha256 {
            return Err(format!(
                "the download's checksum is {got}, not the expected {}; not using it",
                self.sha256
            ));
        }
        Ok(())
    }

    /// Unpack with the AppImage's own `--appimage-extract` into a fresh directory, and move the
    /// result into place only once it is complete, so a half-unpacked FreeCAD is never used.
    fn unpack(&self, image: &Path) -> Result<PathBuf, String> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(image, std::fs::Permissions::from_mode(0o755))
                .map_err(|e| e.to_string())?;
        }
        let work = self.dir.join("unpacking");
        let _ = std::fs::remove_dir_all(&work);
        std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
        let status = std::process::Command::new(image)
            .arg("--appimage-extract")
            .current_dir(&work)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map_err(|e| format!("unpacking FreeCAD: {e}"))?;
        let unpacked = work.join("squashfs-root");
        if !status.success() || !unpacked.join("usr/bin/freecadcmd").is_file() {
            let _ = std::fs::remove_dir_all(&work);
            return Err("the FreeCAD image did not unpack".to_owned());
        }
        let target = self.dir.join("squashfs-root");
        let _ = std::fs::remove_dir_all(&target);
        std::fs::rename(&unpacked, &target).map_err(|e| e.to_string())?;
        let _ = std::fs::remove_dir_all(&work);
        self.installed()
            .ok_or_else(|| "FreeCAD unpacked without freecadcmd".to_owned())
    }
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(HEX[usize::from(b >> 4)] as char);
        s.push(HEX[usize::from(b & 0xF)] as char);
    }
    s
}
