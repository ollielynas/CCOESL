use std::sync::atomic::AtomicUsize;

use super::*;

/// Writes `COPY:` and the input's bytes, after `delay`, reporting half way; or fails, or says
/// the input is fine as it is. Counts its calls.
#[derive(Default)]
pub struct Fake {
    calls: AtomicUsize,
    delay: Duration,
    fail: bool,
    fine: bool,
}

impl Converter for Fake {
    fn convert(
        &self,
        input: &Path,
        output: &Path,
        _: Target,
        _: Duration,
        progress: &(dyn Fn(u16) + Sync),
    ) -> Result<Converted, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        progress(500);
        std::thread::sleep(self.delay);
        if self.fail {
            return Err("not a real video".to_owned());
        }
        if self.fine {
            return Ok(Converted::AlreadyFine);
        }
        let mut bytes = b"COPY:".to_vec();
        bytes.extend(std::fs::read(input).map_err(|e| e.to_string())?);
        std::fs::write(output, bytes).map_err(|e| e.to_string())?;
        Ok(Converted::Made)
    }
}

fn temp(name: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "ccosel-web-copy-{}-{name}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("files")).unwrap();
    dir
}

fn copies(dir: &Path, fake: Fake, max_bytes: u64) -> (WebCopies, Arc<Fake>) {
    let fake = Arc::new(fake);
    (
        WebCopies::new(dir.join("cache"), max_bytes, fake.clone()),
        fake,
    )
}

/// Polls until the job for `real` finishes.
fn settle(c: &WebCopies, real: &Path, target: Target) -> WebCopyStatus {
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        let s = c.status(real, target);
        if s.finished {
            return s;
        }
        assert!(Instant::now() < until, "never finished");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn which_formats_get_a_copy() {
    let t = |p: &str| target_for(Path::new(p));
    assert_eq!(t("IMG_1.HEIC"), Some(Target::Jpeg));
    assert_eq!(t("a.heif"), Some(Target::Jpeg));
    assert_eq!(t("scan.tiff"), Some(Target::Jpeg));
    assert_eq!(t("clip.MOV"), Some(Target::Mp4));
    assert_eq!(t("song.m4a"), Some(Target::Aac));
    assert_eq!(t("a.caf"), Some(Target::Aac));
    assert_eq!(t("a.aif"), Some(Target::Aac));
    assert_eq!(t("photo.jpg"), None);
    assert_eq!(t("film.mp4"), None);
    assert_eq!(t("noext"), None);
    assert_eq!(Target::Mp4.content_type(), "video/mp4");
    assert_eq!(Target::Aac.extension(), "m4a");
}

#[test]
fn the_key_follows_the_files_path_size_and_time() {
    let dir = temp("key");
    let a = dir.join("files/a.heic");
    let b = dir.join("files/b.heic");
    std::fs::write(&a, "one").unwrap();
    std::fs::write(&b, "one").unwrap();
    let ka = WebCopies::key(&a, Target::Jpeg).unwrap();
    assert_eq!(ka, WebCopies::key(&a, Target::Jpeg).unwrap(), "stable");
    assert_ne!(ka, WebCopies::key(&b, Target::Jpeg).unwrap(), "by path");
    assert_ne!(ka, WebCopies::key(&a, Target::Mp4).unwrap(), "by target");

    std::fs::write(&a, "longer").unwrap();
    assert_ne!(ka, WebCopies::key(&a, Target::Jpeg).unwrap(), "by size");
    let k2 = WebCopies::key(&a, Target::Jpeg).unwrap();
    let f = std::fs::File::options().append(true).open(&a).unwrap();
    f.set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000))
        .unwrap();
    assert_ne!(k2, WebCopies::key(&a, Target::Jpeg).unwrap(), "by time");
    assert_eq!(WebCopies::key(&dir.join("missing"), Target::Jpeg), None);
}

#[test]
fn a_copy_is_made_once_and_then_found() {
    let dir = temp("once");
    let src = dir.join("files/p.heic");
    std::fs::write(&src, "pixels").unwrap();
    let (c, fake) = copies(&dir, Fake::default(), 1 << 20);

    assert_eq!(settle(&c, &src, Target::Jpeg).error, None);
    let Some(Ready::Copy(copy, Target::Jpeg)) = c.cached(&src, Target::Jpeg) else {
        panic!("no copy");
    };
    assert_eq!(std::fs::read(copy).unwrap(), b"COPY:pixels");
    for _ in 0..3 {
        assert!(c.status(&src, Target::Jpeg).finished);
    }
    assert_eq!(
        fake.calls.load(Ordering::SeqCst),
        1,
        "a second view costs nothing"
    );
}

#[test]
fn changing_the_file_makes_a_new_copy() {
    let dir = temp("change");
    let src = dir.join("files/p.tif");
    std::fs::write(&src, "v1").unwrap();
    let (c, fake) = copies(&dir, Fake::default(), 1 << 20);
    settle(&c, &src, Target::Jpeg);

    std::fs::write(&src, "version 2").unwrap();
    assert_eq!(
        c.cached(&src, Target::Jpeg),
        None,
        "the old copy isn't this file's"
    );
    settle(&c, &src, Target::Jpeg);
    let Some(Ready::Copy(copy, _)) = c.cached(&src, Target::Jpeg) else {
        panic!()
    };
    assert_eq!(std::fs::read(copy).unwrap(), b"COPY:version 2");
    assert_eq!(fake.calls.load(Ordering::SeqCst), 2);
}

#[test]
fn a_running_job_reports_progress_and_a_short_wait_gives_up() {
    let dir = temp("progress");
    let src = dir.join("files/v.mov");
    std::fs::write(&src, "frames").unwrap();
    let fake = Fake {
        delay: Duration::from_millis(300),
        ..Default::default()
    };
    let (c, fake) = copies(&dir, fake, 1 << 20);

    assert_eq!(c.wait(&src, Target::Mp4, Duration::from_millis(20)), None);
    let until = Instant::now() + Duration::from_secs(5);
    let running = loop {
        let s = c.status(&src, Target::Mp4);
        if s.permille.is_some() || Instant::now() > until {
            break s;
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(
        running,
        WebCopyStatus {
            finished: false,
            permille: Some(500),
            error: None
        }
    );
    assert!(matches!(
        c.wait(&src, Target::Mp4, Duration::from_secs(5)),
        Some(Ok(Ready::Copy(..)))
    ));
    assert_eq!(
        fake.calls.load(Ordering::SeqCst),
        1,
        "polling started one job"
    );
}

#[test]
fn a_failure_is_reported_and_not_retried_straight_away() {
    let dir = temp("fail");
    let src = dir.join("files/v.mov");
    std::fs::write(&src, "garbage").unwrap();
    let (c, fake) = copies(
        &dir,
        Fake {
            fail: true,
            ..Default::default()
        },
        1 << 20,
    );
    let s = settle(&c, &src, Target::Mp4);
    assert_eq!(s.error.as_deref(), Some("not a real video"));
    assert_eq!(
        c.status(&src, Target::Mp4).error.as_deref(),
        Some("not a real video")
    );
    assert_eq!(fake.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        std::fs::read_dir(dir.join("cache")).unwrap().count(),
        0,
        "nothing half-written is left"
    );
}

#[test]
fn a_file_that_plays_already_is_sent_as_it_is_and_remembered() {
    let dir = temp("fine");
    let src = dir.join("files/song.m4a");
    std::fs::write(&src, "aac").unwrap();
    let (c, fake) = copies(
        &dir,
        Fake {
            fine: true,
            ..Default::default()
        },
        1 << 20,
    );
    assert_eq!(
        c.wait(&src, Target::Aac, Duration::from_secs(5)),
        Some(Ok(Ready::Original))
    );
    assert_eq!(c.cached(&src, Target::Aac), Some(Ready::Original));
    assert_eq!(fake.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn the_cache_drops_the_least_recently_used_copies_past_its_limit() {
    let dir = temp("evict");
    // Each copy is `COPY:` plus 100 bytes: 105. Room for two.
    let (c, _) = copies(&dir, Fake::default(), 250);
    let make = |name: &str| {
        let src = dir.join("files").join(name);
        std::fs::write(&src, [b'x'; 100]).unwrap();
        settle(&c, &src, Target::Jpeg);
        src
    };
    let old = make("old.heic");
    std::thread::sleep(Duration::from_millis(20));
    let used = make("used.heic");
    std::thread::sleep(Duration::from_millis(20));
    // Viewing `old` again makes it the most recently used, so `used` goes next.
    assert!(c.cached(&old, Target::Jpeg).is_some());
    std::thread::sleep(Duration::from_millis(20));
    let new = make("new.heic");

    assert!(c.cached(&new, Target::Jpeg).is_some());
    assert!(c.cached(&old, Target::Jpeg).is_some());
    assert_eq!(c.cached(&used, Target::Jpeg), None);
}
