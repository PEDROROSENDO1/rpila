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
//! the other outputs (one drawn first holds it back), plus `SLACK` (the timer
//! waking up late, the kernel's lead before the vblank). Leaner margins were
//! tried on a TTY: counted from when a held frame was meant to be drawn and
//! without the other outputs' worst, they left 1-3 ms instead of 3-8 and
//! saved 0.7-1.8 ms of latency, but an app starting up on the same GPU
//! (shaders, uploads) made a held frame late twice in three runs, and that's
//! what no frame of Pila's foretells. Frames that were a
//! client's buffer as it is (direct scanout) are kept apart from those drawn,
//! and the margin is the worst of either: leaving direct scanout, the first
//! frame drawn again would otherwise get the margin of a buffer handed over.
//! A frame that still shows late after being held turns it off for that
//! output for `BACKOFF`.
//!
//! What it leaves is measured: for each held frame, its margin and how long
//! before the vblank it aimed at it was ready (`spare`), both in the frame
//! statistics (`frame_stats.rs`): the smallest spare says how much the margin
//! could shrink.
//!
//! Frames are held only in a steady scene (`draw::hold_late`): not while
//! Pila animates, nor for `SETTLING` after a window or layer surface appears
//! or a window draws itself at a new size (`settling`, `Pila::start_settling`,
//! animated or not). Then apps are busy on the GPU as no frame of Pila's
//! foretells: a terminal starting up (shaders, glyphs) made a held frame take
//! 9.4 ms on the GPU on battery, one redrawing itself maximized and a game
//! closing after flooding the GPU made held frames late on a single monitor,
//! all with the margin above. An animation of Pila's shows at the same
//! vblank either way: holding gains nothing there.
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

/// How long after a window or layer surface appears, or a window changes
/// size, no frame is held (`settling`): its app starting up or drawing
/// everything anew shares the GPU for a few frames, as no frame of Pila's
/// foretells.
const SETTLING: Duration = Duration::from_millis(300);

/// The margin is never less than the period divided by this: half a period.
/// With one output, the worst of its own recent frames plus `SLACK` (4-5 ms
/// on the author's laptop) let apps busy on the GPU in a steady scene (a
/// shell's panel animating itself, its new blurred popup) make a held frame
/// late on battery; two outputs had 7.7-8.3 ms and none. Half a period keeps
/// that, and the latency it gained.
const MIN_MARGIN_DIVISOR: u32 = 2;

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

/// A frame held to be drawn `at`, for the vblank at `vblank` (monotonic).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Hold {
    pub at: Duration,
    pub vblank: Duration,
}

impl Hold {
    /// How long before its vblank it was drawn.
    pub fn margin(&self) -> Duration {
        self.vblank.saturating_sub(self.at)
    }
}

/// The frame queued last, waiting for its fence.
#[derive(Clone, Copy, Debug)]
struct Pending {
    /// When its drawing started and when it was queued (monotonic).
    started: Duration,
    queued: Duration,
    /// The vblank it was held for, if it was.
    aim: Option<Duration>,
}

/// An output's recent frames: how long they took to be ready (see the
/// module docs).
#[derive(Debug, Default)]
pub(super) struct RenderTiming {
    /// What recent frames took: drawn ones, and a client's buffers as they
    /// are (direct scanout).
    took: VecDeque<Duration>,
    took_direct: VecDeque<Duration>,
    pending: Option<Pending>,
    /// A composited frame had no fence: nothing is known of its GPU work.
    unmeasurable: bool,
    /// Off until then, after a held frame showed late.
    backoff_until: Option<Duration>,
}

impl RenderTiming {
    /// A frame whose drawing started at `started` was queued at `queued`,
    /// waiting for `waits`, `held` for a vblank if it was. Its spare
    /// (`spare`) if it was held and is ready now.
    pub fn queued(
        &mut self,
        started: Duration,
        queued: Duration,
        waits: Waits,
        held: Option<Hold>,
    ) -> Option<Duration> {
        self.pending = None;
        let aim = held.map(|hold| hold.vblank);
        match waits {
            Waits::Nothing => {
                self.unmeasurable = false;
                push(&mut self.took_direct, queued.saturating_sub(started));
                return aim.map(|aim| spare(aim, queued));
            }
            Waits::Fence => {
                self.pending = Some(Pending {
                    started,
                    queued,
                    aim,
                })
            }
            Waits::Unknown => self.unmeasurable = true,
        }
        None
    }

    /// The GPU finished, at `signal`, the frame whose drawing started at
    /// `started`, if it's the one waiting. Its spare if it was held.
    pub fn gpu_done(&mut self, started: Duration, signal: Duration) -> Option<Duration> {
        let pending = self.pending.filter(|p| p.started == started)?;
        self.pending = None;
        let ready = signal.max(pending.queued);
        self.record(ready.saturating_sub(pending.started));
        pending.aim.map(|aim| spare(aim, ready))
    }

    fn record(&mut self, took: Duration) {
        self.unmeasurable = false;
        push(&mut self.took, took);
    }

    /// The worst of the recent frames, drawn or direct, if any was measured.
    pub fn worst(&self) -> Option<Duration> {
        self.took.iter().chain(&self.took_direct).max().copied()
    }

    /// A frame it held showed late at `now`: off for `BACKOFF`.
    pub fn showed_late(&mut self, now: Duration) {
        self.backoff_until = Some(now + BACKOFF);
    }

    /// How long before a vblank to draw for it at `now`, given the worst of
    /// the other outputs (`others`), on an output whose refresh lasts
    /// `period`: never less than `period / MIN_MARGIN_DIVISOR`. `None`: don't
    /// hold.
    pub fn margin(&self, now: Duration, others: Duration, period: Duration) -> Option<Duration> {
        if self.unmeasurable || self.backoff_until.is_some_and(|until| now < until) {
            return None;
        }
        let floor = period / MIN_MARGIN_DIVISOR;
        Some((self.worst()? + others + SLACK).max(floor))
    }
}

/// Add `took` to a window of the last `WINDOW` frames.
fn push(window: &mut VecDeque<Duration>, took: Duration) {
    if window.len() == WINDOW {
        window.pop_front();
    }
    window.push_back(took);
}

/// Whether a window or layer surface appeared, or a window changed size
/// (`Pila::start_settling`, at `since`), too recently at `now` for a frame to
/// be held.
pub(super) fn settling(since: Option<Duration>, now: Duration) -> bool {
    since.is_some_and(|at| now.saturating_sub(at) < SETTLING)
}

/// How long before the vblank at `aim` a frame held for it was ready, at
/// `ready`: zero if it wasn't.
pub(super) fn spare(aim: Duration, ready: Duration) -> Duration {
    aim.saturating_sub(ready)
}

/// When to draw the frame for the next vblank after `now`, `margin` before
/// it, on a grid of vblanks `period` apart from `last`; `None`: draw now (no
/// grid, or too old to trust, or that moment is past or too close).
pub(super) fn render_at(
    last: Option<Duration>,
    now: Duration,
    period: Duration,
    margin: Duration,
) -> Option<Hold> {
    let last = last?;
    if period.is_zero() || now.saturating_sub(last) > FRESH {
        return None;
    }
    let vblank = crate::commit_pacing::next_vblank(last, now, period);
    let at = vblank.checked_sub(margin)?;
    (at > now + SHORTEST_HOLD).then_some(Hold { at, vblank })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);
    const PERIOD: Duration = Duration::from_micros(16_667);

    /// When `render_at` holds a frame until, if it does.
    fn at_of(
        last: Option<Duration>,
        now: Duration,
        period: Duration,
        margin: Duration,
    ) -> Option<Duration> {
        render_at(last, now, period, margin).map(|hold| hold.at)
    }

    #[test]
    fn a_frame_is_drawn_its_margin_before_the_next_vblank() {
        // Vblank at 100, the next at 116.667: drawn 5 ms before it.
        let at = at_of(Some(100 * MS), 101 * MS, PERIOD, 5 * MS);
        assert_eq!(at, Some(100 * MS + PERIOD - 5 * MS));
        // Asked for later in the period, the same vblank.
        assert_eq!(
            at_of(Some(100 * MS), 108 * MS, PERIOD, 5 * MS),
            Some(100 * MS + PERIOD - 5 * MS)
        );
    }

    #[test]
    fn past_or_too_close_to_its_moment_a_frame_is_drawn_at_once() {
        // 5 ms before 116.667 is 111.667: past at 112, too close at 111.5.
        assert_eq!(at_of(Some(100 * MS), 112 * MS, PERIOD, 5 * MS), None);
        assert_eq!(
            at_of(Some(100 * MS), 111_500 * MS / 1000, PERIOD, 5 * MS),
            None
        );
        // A margin over a period: always at once.
        assert_eq!(at_of(Some(100 * MS), 101 * MS, PERIOD, 20 * MS), None);
    }

    #[test]
    fn without_a_recent_vblank_a_frame_is_drawn_at_once() {
        assert_eq!(at_of(None, 101 * MS, PERIOD, 5 * MS), None);
        // The grid of an hour ago has drifted.
        let hour = Duration::from_secs(3600);
        assert_eq!(at_of(Some(100 * MS), hour, PERIOD, 5 * MS), None);
        // Half a second is fine.
        assert!(at_of(Some(100 * MS), 590 * MS, PERIOD, 5 * MS).is_some());
    }

    #[test]
    fn the_margin_is_the_worst_recent_frame_plus_the_others_and_slack() {
        let mut timing = RenderTiming::default();
        // Nothing measured yet: not held.
        assert_eq!(
            timing.margin(Duration::ZERO, Duration::ZERO, Duration::ZERO),
            None
        );
        // A composited frame: ready when its fence signals, if that's after
        // its commit.
        timing.queued(100 * MS, 101 * MS, Waits::Fence, None);
        timing.gpu_done(100 * MS, 103 * MS);
        assert_eq!(timing.worst(), Some(3 * MS));
        timing.queued(200 * MS, 201 * MS, Waits::Fence, None);
        timing.gpu_done(200 * MS, 200 * MS + MS / 2);
        assert_eq!(timing.worst(), Some(3 * MS));
        // A fence for another frame than the one waiting counts for nothing.
        timing.queued(300 * MS, 301 * MS, Waits::Fence, None);
        timing.gpu_done(250 * MS, 310 * MS);
        assert_eq!(timing.worst(), Some(3 * MS));
        // Direct scanout: ready once queued.
        timing.queued(400 * MS, 402 * MS, Waits::Nothing, None);
        assert_eq!(
            timing.margin(500 * MS, 2 * MS, Duration::ZERO),
            Some(3 * MS + 2 * MS + SLACK)
        );
    }

    #[test]
    fn the_margin_is_never_under_half_a_period() {
        let mut timing = RenderTiming::default();
        timing.queued(Duration::ZERO, MS, Waits::Nothing, None);
        // 1 ms + slack is under half of 16.7 ms: half a period.
        assert_eq!(timing.margin(MS, Duration::ZERO, PERIOD), Some(PERIOD / 2));
        // A worse frame sets it past that.
        timing.queued(100 * MS, 109 * MS, Waits::Nothing, None);
        assert_eq!(
            timing.margin(200 * MS, Duration::ZERO, PERIOD),
            Some(9 * MS + SLACK)
        );
    }

    #[test]
    fn the_worst_frame_leaves_the_margin_once_out_of_the_window() {
        let mut timing = RenderTiming::default();
        timing.queued(Duration::ZERO, 9 * MS, Waits::Nothing, None);
        for i in 1..WINDOW as u32 {
            timing.queued(i * 20 * MS, i * 20 * MS + MS, Waits::Nothing, None);
        }
        assert_eq!(timing.worst(), Some(9 * MS));
        timing.queued(10_000 * MS, 10_001 * MS, Waits::Nothing, None);
        assert_eq!(timing.worst(), Some(MS));
    }

    #[test]
    fn a_frame_with_no_fence_to_time_stops_holding_until_one_is_measured() {
        let mut timing = RenderTiming::default();
        timing.queued(Duration::ZERO, MS, Waits::Nothing, None);
        timing.queued(20 * MS, 21 * MS, Waits::Unknown, None);
        assert_eq!(timing.margin(40 * MS, Duration::ZERO, Duration::ZERO), None);
        timing.queued(40 * MS, 41 * MS, Waits::Fence, None);
        timing.gpu_done(40 * MS, 42 * MS);
        assert_eq!(
            timing.margin(60 * MS, Duration::ZERO, Duration::ZERO),
            Some(2 * MS + SLACK)
        );
    }

    #[test]
    fn a_held_frame_says_how_long_before_its_vblank_it_was_ready() {
        let hold = render_at(Some(100 * MS), 101 * MS, PERIOD, 5 * MS).expect("held");
        assert_eq!(hold.vblank, 100 * MS + PERIOD);
        assert_eq!(hold.margin(), 5 * MS);
        let mut timing = RenderTiming::default();
        // Composited: ready when its fence signals, 2 ms before its vblank.
        let started = hold.at;
        let queued = started + MS;
        let aim = Some(hold);
        assert_eq!(timing.queued(started, queued, Waits::Fence, aim), None);
        let spare = timing.gpu_done(started, hold.vblank - 2 * MS);
        assert_eq!(spare, Some(2 * MS));
        // A client's buffer as it is: ready once queued.
        let spare = timing.queued(started, hold.vblank - 4 * MS, Waits::Nothing, aim);
        assert_eq!(spare, Some(4 * MS));
        // Ready after its vblank: nothing to spare.
        timing.queued(started, queued, Waits::Fence, aim);
        assert_eq!(
            timing.gpu_done(started, hold.vblank + MS),
            Some(Duration::ZERO)
        );
        // Not held: nothing to measure against.
        timing.queued(started, queued, Waits::Fence, None);
        assert_eq!(timing.gpu_done(started, hold.vblank), None);
    }

    #[test]
    fn leaving_direct_scanout_keeps_the_margin_of_drawn_frames() {
        let mut timing = RenderTiming::default();
        timing.queued(Duration::ZERO, MS, Waits::Fence, None);
        timing.gpu_done(Duration::ZERO, 3 * MS);
        for i in 1..=WINDOW as u32 * 2 {
            let at = i * 20 * MS;
            timing.queued(at, at + MS / 10, Waits::Nothing, None);
        }
        assert_eq!(timing.worst(), Some(3 * MS));
    }

    /// Seen on a TTY, on battery: a blurred terminal opening, its second
    /// frame held by the usual margin took 9.4 ms on the GPU (the terminal's
    /// own start-up work queued first) and came late.
    #[test]
    fn nothing_is_held_while_a_new_surface_settles() {
        assert!(!settling(None, 1000 * MS));
        assert!(settling(Some(1000 * MS), 1000 * MS));
        assert!(settling(Some(1000 * MS), 1000 * MS + SETTLING - MS));
        assert!(!settling(Some(1000 * MS), 1000 * MS + SETTLING));
    }

    #[test]
    fn a_held_frame_shown_late_stops_holding_for_a_while() {
        let mut timing = RenderTiming::default();
        timing.queued(Duration::ZERO, MS, Waits::Nothing, None);
        timing.showed_late(100 * MS);
        assert_eq!(
            timing.margin(100 * MS, Duration::ZERO, Duration::ZERO),
            None
        );
        assert_eq!(
            timing.margin(100 * MS + BACKOFF - MS, Duration::ZERO, Duration::ZERO),
            None
        );
        assert!(timing
            .margin(100 * MS + BACKOFF, Duration::ZERO, Duration::ZERO)
            .is_some());
    }
}
