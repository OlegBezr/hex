//! A readable macOS dictation failure beside the user's work, even while
//! Settings is closed. Like the meeting capsules, it never activates Hex.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[cfg(target_os = "macos")]
use rune::{
    Align, DisplayTarget, OverlayHandle, OverlayInput, OverlayOptions, OverlaySpec, ScreenEdge,
};
use rune::{Anim, Cx, El, col, row, text};
use rune_kit::{Button, ButtonVariant, IconButton, IconButtonVariant};

use super::controls::{Paint, PushSize};
use crate::desktop::events::{DictationPhase, EventReader, VoiceEvent};
use crate::desktop::i18n::tr;

const SIZE: (f32, f32) = (480.0, 244.0);
const VISIBLE: Duration = Duration::from_secs(12);

#[derive(Default)]
struct Card {
    message: String,
    hide: Option<Arc<dyn Fn() + Send + Sync>>,
    settings: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl Card {
    fn dismiss(&self) {
        if let Some(hide) = &self.hide {
            hide();
        }
    }
}

/// Reads only outcomes from this launch. One panel is replaced on each
/// failure; the next attempt, dismissal, or timeout closes it.
#[cfg(target_os = "macos")]
pub(crate) struct Notice {
    feed: Feed,
    panel: OverlayHandle<Card>,
}

#[cfg(target_os = "macos")]
impl Notice {
    pub(crate) fn install<S: 'static>(
        app: &mut rune::App<S>,
        settings: impl Fn() + Send + Sync + 'static,
    ) -> color_eyre::Result<Self> {
        let feed = Feed::new(&crate::app_paths::logs_dir()?.join("live.ndjson"));
        let panel = app.overlay(
            OverlaySpec {
                name: "Hex dictation failure",
                size: SIZE,
                edge: ScreenEdge::Top,
                margin: 40.0,
                align: Align::End,
                inset: 12.0,
                interactive: true,
                view,
            }
            .options(OverlayOptions {
                display: DisplayTarget::UnderPointer,
                ..Default::default()
            })
            .release_gpu_when_hidden(),
            Card {
                settings: Some(Arc::new(settings)),
                ..Default::default()
            },
        );
        panel.set_input(OverlayInput::Regions);
        let hide = panel.clone();
        panel.update(move |card, _| card.hide = Some(Arc::new(move || hide.hide())));
        Ok(Self { feed, panel })
    }

    /// Called by the macOS pump even when the main window is hidden.
    pub(crate) fn poll(&mut self) {
        match self.feed.poll(Instant::now()) {
            Some(Change::Show(message)) => {
                let panel = self.panel.clone();
                self.panel.update(move |card, tx| {
                    card.message = message;
                    tx.instant();
                    panel.show();
                });
            }
            Some(Change::Hide) => {
                let panel = self.panel.clone();
                self.panel.update(move |_, _| panel.hide());
            }
            None => {}
        }
    }
}

#[derive(Debug, PartialEq)]
enum Change {
    Show(String),
    Hide,
}

struct Feed {
    reader: EventReader,
    since_ms: u64,
    until: Option<Instant>,
}

impl Feed {
    fn new(path: &Path) -> Self {
        Self {
            reader: EventReader::open(path),
            since_ms: crate::desktop::events::now_ms(),
            until: None,
        }
    }

    fn poll(&mut self, now: Instant) -> Option<Change> {
        if let Err(error) = self.reader.refresh() {
            tracing::warn!(%error, "could not read dictation failures");
        }
        let mut change = None;
        for event in self.reader.drain() {
            if let VoiceEvent::Dictation {
                timestamp_ms,
                phase,
                ..
            } = event
                && timestamp_ms >= self.since_ms
            {
                change = Some(match phase {
                    DictationPhase::Failed(message) => {
                        self.until = Some(now + VISIBLE);
                        Change::Show(message)
                    }
                    _ => {
                        self.until = None;
                        Change::Hide
                    }
                });
            }
        }
        if self.until.is_some_and(|until| now >= until) {
            self.until = None;
            change = Some(Change::Hide);
        }
        change
    }
}

fn view(card: &Card, cx: &mut Cx<Card>) -> El {
    let p = Paint::new(cx.system_dark(), cx.env().scale);
    let closed_lid = card.message.contains(crate::audio::LID_CLOSED_ERROR);
    let body = col()
        .key("failure-detail")
        .h(100.0)
        .gap(8.0)
        .scroll(rune::ScrollSpec {
            stick_bottom: false,
        })
        .child(
            text(card.message.clone())
                .key("reason")
                .font(p.k.ty.ui)
                .color(p.text())
                .wrap(),
        )
        .children(closed_lid.then(|| {
            text(tr(
                "Open your MacBook’s lid or connect an external microphone, then try again.",
            ))
            .key("recovery")
            .font(p.k.ty.ui)
            .color(p.muted())
            .wrap()
        }));
    let settings = Button::new(tr("Settings"))
        .variant(ButtonVariant::Neutral)
        .on_press(|card: &mut Card| {
            card.dismiss();
            if let Some(settings) = &card.settings {
                settings();
            }
        })
        .build_push(p.k)
        .key("settings");
    let close = IconButton::new(&rune_theme_opencode::icons::CLOSE)
        .variant(IconButtonVariant::Ghost)
        .label(tr("Close"))
        .on_press(|card: &mut Card| card.dismiss())
        .build(p.k)
        .key("dismiss");
    row()
        .w_full()
        .h_full()
        .padding(8.0)
        .hover_motion_within(Anim::INSTANT, Anim::INSTANT)
        .child(
            col()
                .key("dictation-failure")
                .w_full()
                .padding(16.0)
                .gap(12.0)
                .radius(12.0)
                .fill(p.k.c.bg_base)
                .border(p.hair, p.k.c.border_base)
                .hit()
                .child(
                    row()
                        .w_full()
                        .center_y()
                        .gap(8.0)
                        .child(
                            text(tr("Dictation failed"))
                                .key("title")
                                .font(p.k.ty.ui_medium)
                                .color(p.danger())
                                .grow(1.0),
                        )
                        .child(close),
                )
                .child(body)
                .child(settings),
        )
}

#[cfg(test)]
mod tests;
