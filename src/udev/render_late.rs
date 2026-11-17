//! Rendering late (`render-late`, bare metal): a frame for a vblank is drawn
//! as close to it as it safely can be, not as soon as the last frame was
//! shown, so what clients and input bring in the meantime makes it into that
//! vblank instead of the next one. A client drawing at the refresh rate gets
//! its frame shown up to a period sooner; so does the cursor's position.
//!
//! How close: each output keeps what its recent frames took to be ready
//! (`RenderTiming`): from the start of their drawing (`prepare`) until the
//! later of their commit queued and their GPU work done (the fence the kernel
//! waits on before the flip; `frame_stats::GpuFrame`), the worst of the last
//! `WINDOW` frames. Its margin before the vblank is that, plus the worst of
//! the device's other outputs (one drawn first holds it back), plus `SLACK`
//! (the timer waking up late, the kernel's lead before the vblank). A frame
//! that still shows late after being held turns it off for that output for
//! `BACKOFF`.
//!
//! Not for a frame that can't be measured or has no vblank to aim at: before
//! any frame was measured, after a composited frame with no native fence to
//! time its GPU work by, with the vblanks' grid older than `FRESH` (the
//! mode's refresh is rounded: the grid drifts), while tearing or with
//! adaptive sync (the flip makes the vblank), or while a lock waits for its
//! frame.

use std::collections::VecDeque;
use std::time::Duration;

/// Frames whose readiness sets the margin: two seconds at 60 Hz.
const WINDOW: usize = 128;

/// On top of the worst frame: the timer waking up late, the commit's way
/// through the kernel before the vblank.
pub(super) const SLACK: Duration = Duration::from_micros(1500);

/// A hold shorter than this isn't worth a timer: drawn at once.
const SHORTEST_HOLD: Duration = Duration::from_micros(500);

/// How long rendering late stays off for an output after a frame it held
/// showed late.
const BACKOFF: Duration = Duration::from_secs(2);

/// How old the last vblank may be for the grid it sets to be trusted.
const FRESH: Duration = Duration::from_secs(1);

/// What a queued frame waits for before it can be shown.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Waits {
    /// Nothing drawn: a client's buffer as it is (direct scanout).
    Nothing,
    /// Its GPU work, done when its native fence signals (`gpu_done`).
    Fence,
    /// GPU work with no fence to time it: it can't be measured.
    Unknown,
}

/// An output's recent frames: how long they took to be ready (see the
/// module docs).
#[derive(Debug, Default)]
pub(super) struct RenderTiming {
    took: VecDeque<Duration>,
    /// The frame queued last, waiting for its fence: when its drawing
    /// started and when it was queued (monotonic).
    pending: Option<(Duration, Duration)>,
    /// A composited frame had no fence: nothing is known of its GPU work.
    unmeasurable: bool,
    /// Off until then, after a held frame showed late.
    backoff_until: Option<Duration>,
}

impl RenderTiming {
    /// A frame whose drawing started at `started` was queued at `queued`,
    /// waiting for `waits`.
    pub fn queued(&mut self, started: Duration, queued: Duration, waits: Waits) {
        self.pending = None;
        match waits {
            Waits::Nothing => self.record(queued.saturating_sub(started)),
            Waits::Fence => self.pending = Some((started, queued)),
            Waits::Unknown => self.unmeasurable = true,
        }
    }

    /// The GPU finished, at `signal`, the frame whose drawing started at
    /// `started`, if it's the one waiting.
    pub fn gpu_done(&mut self, started: Duration, signal: Duration) {
        if let Some((pending_started, queued)) = self.pending {
            if pending_started == started {
                self.pending = None;
                self.record(signal.max(queued).saturating_sub(started));
            }
        }
    }

    fn record(&mut self, took: Duration) {
        self.unmeasurable = false;
        if self.took.len() == WINDOW {
            self.took.pop_front();
        }
        self.took.push_back(took);
    }

    /// The worst of the recent frames, if any was measured.
    pub fn worst(&self) -> Option<Duration> {
        self.took.iter().max().copied()
    }

    /// A frame it held showed late at `now`: off for `BACKOFF`.
    pub fn showed_late(&mut self, now: Duration) {
        self.backoff_until = Some(now + BACKOFF);
    }

    /// How long before a vblank to draw for it at `now`, given the worst of
    /// the device's other outputs (`others`); `None`: don't hold.
    pub fn margin(&self, now: Duration, others: Duration) -> Option<Duration> {
        if self.unmeasurable || self.backoff_until.is_some_and(|until| now < until) {
            return None;
        }
        Some(self.worst()? + others + SLACK)
    }
}

/// When to draw the frame for the next vblank after `now`, `margin` before
/// it, on a grid of vblanks `period` apart from `last`; `None`: draw now (no
/// grid, or too old to trust, or that moment is past or too close).
pub(super) fn render_at(
    last: Option<Duration>,
    now: Duration,
    period: Duration,
    margin: Duration,
) -> Option<Duration> {
    let last = last?;
    if period.is_zero() || now.saturating_sub(last) > FRESH {
        return None;
    }
    let next = crate::commit_pacing::next_vblank(last, now, period);
    let at = next.checked_sub(margin)?;
    (at > now + SHORTEST_HOLD).then_some(at)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);
    const PERIOD: Duration = Duration::from_micros(16_667);

    #[test]
    fn a_frame_is_drawn_its_margin_before_the_next_vblank() {
        // Vblank at 100, the next at 116.667: drawn 5 ms before it.
        let at = render_at(Some(100 * MS), 101 * MS, PERIOD, 5 * MS);
        assert_eq!(at, Some(100 * MS + PERIOD - 5 * MS));
        // Asked for later in the period, the same vblank.
        assert_eq!(
            render_at(Some(100 * MS), 108 * MS, PERIOD, 5 * MS),
            Some(100 * MS + PERIOD - 5 * MS)
        );
    }

    #[test]
    fn past_or_too_close_to_its_moment_a_frame_is_drawn_at_once() {
        // 5 ms before 116.667 is 111.667: past at 112, too close at 111.5.
        assert_eq!(render_at(Some(100 * MS), 112 * MS, PERIOD, 5 * MS), None);
        assert_eq!(
            render_at(Some(100 * MS), 111_500 * MS / 1000, PERIOD, 5 * MS),
            None
        );
        // A margin over a period: always at once.
        assert_eq!(render_at(Some(100 * MS), 101 * MS, PERIOD, 20 * MS), None);
    }

    #[test]
    fn without_a_recent_vblank_a_frame_is_drawn_at_once() {
        assert_eq!(render_at(None, 101 * MS, PERIOD, 5 * MS), None);
        // The grid of an hour ago has drifted.
        let hour = Duration::from_secs(3600);
        assert_eq!(render_at(Some(100 * MS), hour, PERIOD, 5 * MS), None);
        // Half a second is fine.
        assert!(render_at(Some(100 * MS), 590 * MS, PERIOD, 5 * MS).is_some());
    }

    #[test]
    fn the_margin_is_the_worst_recent_frame_plus_the_others_and_slack() {
        let mut timing = RenderTiming::default();
        // Nothing measured yet: not held.
        assert_eq!(timing.margin(Duration::ZERO, Duration::ZERO), None);
        // A composited frame: ready when its fence signals, if that's after
        // its commit.
        timing.queued(100 * MS, 101 * MS, Waits::Fence);
        timing.gpu_done(100 * MS, 103 * MS);
        assert_eq!(timing.worst(), Some(3 * MS));
        timing.queued(200 * MS, 201 * MS, Waits::Fence);
        timing.gpu_done(200 * MS, 200 * MS + MS / 2);
        assert_eq!(timing.worst(), Some(3 * MS));
        // A fence for another frame than the one waiting counts for nothing.
        timing.queued(300 * MS, 301 * MS, Waits::Fence);
        timing.gpu_done(250 * MS, 310 * MS);
        assert_eq!(timing.worst(), Some(3 * MS));
        // Direct scanout: ready once queued.
        timing.queued(400 * MS, 402 * MS, Waits::Nothing);
        assert_eq!(
            timing.margin(500 * MS, 2 * MS),
            Some(3 * MS + 2 * MS + SLACK)
        );
    }

    #[test]
    fn the_worst_frame_leaves_the_margin_once_out_of_the_window() {
        let mut timing = RenderTiming::default();
        timing.queued(Duration::ZERO, 9 * MS, Waits::Nothing);
        for i in 1..WINDOW as u32 {
            timing.queued(i * 20 * MS, i * 20 * MS + MS, Waits::Nothing);
        }
        assert_eq!(timing.worst(), Some(9 * MS));
        timing.queued(10_000 * MS, 10_001 * MS, Waits::Nothing);
        assert_eq!(timing.worst(), Some(MS));
    }

    #[test]
    fn a_frame_with_no_fence_to_time_stops_holding_until_one_is_measured() {
        let mut timing = RenderTiming::default();
        timing.queued(Duration::ZERO, MS, Waits::Nothing);
        timing.queued(20 * MS, 21 * MS, Waits::Unknown);
        assert_eq!(timing.margin(40 * MS, Duration::ZERO), None);
        timing.queued(40 * MS, 41 * MS, Waits::Fence);
        timing.gpu_done(40 * MS, 42 * MS);
        assert_eq!(timing.margin(60 * MS, Duration::ZERO), Some(2 * MS + SLACK));
    }

    #[test]
    fn a_held_frame_shown_late_stops_holding_for_a_while() {
        let mut timing = RenderTiming::default();
        timing.queued(Duration::ZERO, MS, Waits::Nothing);
        timing.showed_late(100 * MS);
        assert_eq!(timing.margin(100 * MS, Duration::ZERO), None);
        assert_eq!(timing.margin(100 * MS + BACKOFF - MS, Duration::ZERO), None);
        assert!(timing.margin(100 * MS + BACKOFF, Duration::ZERO).is_some());
    }
}
