use super::*;
use crate::desktop::events::EventLog;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

const LID: &str =
    "Could not open microphone: the built-in microphone is off while the lid is closed";

struct Fixture {
    path: std::path::PathBuf,
    feed: Feed,
    log: EventLog,
}

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "hex-dictation-failure-{}-{}.ndjson",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let log = EventLog::create(&path).unwrap();
        let mut feed = Feed::new(&path);
        feed.since_ms = 100;
        Self { path, feed, log }
    }

    fn emit(&self, timestamp_ms: u64, phase: DictationPhase) {
        self.log
            .emit(&VoiceEvent::Dictation {
                timestamp_ms,
                phase,
                text: String::new(),
                processing: None,
                reason: None,
            })
            .unwrap();
        self.log.flush().unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[test]
fn microphone_failure_reaches_the_notice_with_its_explanation() {
    let mut f = Fixture::new();
    f.emit(100, DictationPhase::Started);
    f.emit(101, DictationPhase::Failed(LID.into()));
    let now = Instant::now();
    assert_eq!(f.feed.poll(now), Some(Change::Show(LID.into())));
    assert_eq!(f.feed.poll(now + VISIBLE / 2), None);
    assert_eq!(f.feed.poll(now + VISIBLE), Some(Change::Hide));
    assert_eq!(f.feed.poll(now + VISIBLE * 2), None);
}

#[test]
fn old_failures_are_not_replayed_on_launch() {
    let mut f = Fixture::new();
    f.emit(99, DictationPhase::Failed(LID.into()));
    assert_eq!(f.feed.poll(Instant::now()), None);
}

#[test]
fn retry_clears_the_notice_and_a_new_failure_gets_its_own_timeout() {
    let mut f = Fixture::new();
    let now = Instant::now();
    f.emit(100, DictationPhase::Failed(LID.into()));
    assert!(matches!(f.feed.poll(now), Some(Change::Show(_))));
    f.emit(101, DictationPhase::Started);
    assert_eq!(
        f.feed.poll(now + Duration::from_secs(1)),
        Some(Change::Hide)
    );
    f.emit(102, DictationPhase::Failed("Paste failed".into()));
    assert_eq!(
        f.feed.poll(now + Duration::from_secs(2)),
        Some(Change::Show("Paste failed".into()))
    );
    assert_eq!(
        f.feed.poll(now + VISIBLE),
        None,
        "the old timeout must not hide a newer error"
    );
    assert_eq!(
        f.feed.poll(now + VISIBLE + Duration::from_secs(2)),
        Some(Change::Hide)
    );
}

#[test]
fn a_completed_retry_in_the_same_batch_does_not_show_an_old_failure() {
    let mut f = Fixture::new();
    f.emit(100, DictationPhase::Failed(LID.into()));
    f.emit(101, DictationPhase::Started);
    f.emit(102, DictationPhase::Pasted);
    assert_eq!(f.feed.poll(Instant::now()), Some(Change::Hide));
}

#[test]
fn the_notice_buttons_dismiss_and_only_settings_opens_the_window() {
    for key in ["dismiss", "settings"] {
        let dismissed = Arc::new(AtomicUsize::new(0));
        let opened = Arc::new(AtomicUsize::new(0));
        let d = dismissed.clone();
        let o = opened.clone();
        let mut rt = runtime(LID, false);
        rt.state.hide = Some(Arc::new(move || {
            d.fetch_add(1, Ordering::Relaxed);
        }));
        rt.state.settings = Some(Arc::new(move || {
            o.fetch_add(1, Ordering::Relaxed);
        }));
        let (x, y) = rt.find(key).unwrap();
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(1.0 / 60.0);
        assert_eq!(dismissed.load(Ordering::Relaxed), 1);
        assert_eq!(
            opened.load(Ordering::Relaxed),
            usize::from(key == "settings")
        );
    }
}

fn runtime(message: &str, dark: bool) -> rune::Runtime<Card> {
    let mut rt = crate::rune_ui::offscreen::runtime(
        Card {
            message: message.into(),
            ..Default::default()
        },
        view,
        SIZE,
        rune::Env {
            system_dark: dark,
            scale: 1.0,
        },
        rune::text::TextSystem::new(rune_theme_opencode::FONTS, rune_theme_opencode::FAMILIES),
    );
    rt.settle_all();
    rt.frame(0.0);
    rt
}

#[test]
fn the_notice_renders_the_cause_recovery_and_actions() {
    for dark in [false, true] {
        let rt = runtime(LID, dark);
        let elements = rt.elements();
        assert!(elements.iter().any(|e| e.text.as_deref() == Some(LID)));
        for key in ["reason", "recovery", "settings", "dismiss"] {
            let el = elements
                .iter()
                .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
                .unwrap_or_else(|| panic!("missing {key}"));
            let [x, y, w, h] = el.rect;
            assert!(w > 0.0 && h > 0.0, "{key} has a visible size");
            assert!(
                x >= 0.0 && y >= 0.0 && x + w <= SIZE.0 + 1.0 && y + h <= SIZE.1 + 1.0,
                "{key} fits inside the notice: {:?}",
                el.rect
            );
        }
        let rt = runtime("Could not paste: Accessibility access was revoked", dark);
        assert!(
            !rt.elements()
                .iter()
                .any(|e| e.key.as_deref() == Some("recovery"))
        );
    }
}

#[test]
#[cfg_attr(not(target_os = "linux"), ignore = "goldens are checked on Linux")]
fn notice_goldens() {
    let gpu = rune::render::Gpu::try_new(rune::render::Gpu::instance(), None).unwrap();
    let folder = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden");
    let mut changed = Vec::new();
    for (name, message) in [
        ("lid", LID),
        ("paste", "Could not paste: Accessibility access was revoked"),
    ] {
        for dark in [false, true] {
            let theme = if dark { "dark" } else { "light" };
            let filename = format!("dictation-failure-{name}-{theme}.png");
            let path = folder.join(&filename);
            let mut rt = runtime(message, dark);
            let (w, h) = (SIZE.0 as u32, SIZE.1 as u32);
            let pixels = crate::rune_ui::offscreen::draw(&gpu, &mut rt, w, h);
            if std::env::var_os("HEX_UPDATE_GOLDENS").is_some() {
                rune::render::write_png(&path, w, h, &pixels).unwrap();
                continue;
            }
            let matches = std::fs::File::open(&path).ok().is_some_and(|file| {
                let mut reader = png::Decoder::new(std::io::BufReader::new(file))
                    .read_info()
                    .unwrap();
                let mut golden = vec![0; reader.output_buffer_size().unwrap()];
                let info = reader.next_frame(&mut golden).unwrap();
                if (info.width, info.height) != (w, h) {
                    return false;
                }
                pixels
                    .chunks(4)
                    .zip(golden.chunks(4))
                    .filter(|(a, b)| a.iter().zip(b.iter()).any(|(x, y)| x.abs_diff(*y) > 12))
                    .count()
                    <= (w * h) as usize / 500
            });
            if !matches {
                let actual =
                    std::env::temp_dir().join(format!("rune-settings-{filename}-actual.png"));
                rune::render::write_png(&actual, w, h, &pixels).unwrap();
                changed.push(actual);
            }
        }
    }
    assert!(changed.is_empty(), "notice goldens differ: {changed:?}");
}
