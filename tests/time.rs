#[allow(dead_code)]
mod common;

use bevy::{prelude::*, time::TimeUpdateStrategy};
use bevy_ggrs::{RollbackFrameCount, prelude::*};
use common::{GgrsConfig, base_synctest_app, synctest_session};
use core::time::Duration;

/// Restarting a session after frames have elapsed must not panic.
///
/// `Time<GgrsTime>` is derived from `RollbackFrameCount`. When a session is
/// stopped and a new one started, the frame count resets to 0 while the clock
/// still holds the previous session's elapsed time. The clock must follow the
/// frame count backward instead of panicking inside `Time::advance_to`.
///
/// Regression test for the session-restart panic.
#[test]
fn ggrs_time_survives_session_restart() {
    let mut app = base_synctest_app(2);

    // Advance the first session so GgrsTime accumulates elapsed time.
    for _ in 0..30 {
        app.update();
    }
    assert!(
        app.world().resource::<Time<GgrsTime>>().elapsed() > Duration::ZERO,
        "GgrsTime should have advanced during the first session"
    );

    // Stop the session; the next tick runs bevy_ggrs's no-session branch,
    // which resets RollbackFrameCount to 0 while leaving GgrsTime stale.
    app.world_mut().remove_resource::<Session<GgrsConfig>>();
    app.update();

    // Start a fresh session at frame 0 and advance. This previously panicked
    // because GgrsTime's elapsed was ahead of the new frame's runtime.
    app.world_mut().insert_resource(synctest_session(2));
    app.update();

    // The clock reflects the restarted session's (low) frame count, not the
    // stale value from before the restart.
    let frame = app.world().resource::<RollbackFrameCount>().0;
    let expected = Duration::from_nanos(frame as u64 * 1_000_000_000 / 60);
    assert_eq!(
        app.world().resource::<Time<GgrsTime>>().elapsed(),
        expected,
        "GgrsTime should track the restarted session's frame count"
    );
}

#[test]
fn ggrs_frame_timing_tracks_driver_accumulator() {
    let mut app = base_synctest_app(2);
    app.insert_resource(RollbackFrameRate(50));
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
        10,
    )));

    // The resource exists before the driver has produced a timing sample.
    let timing = app.world().resource::<GgrsFrameTiming>();
    assert_eq!(timing.timestep(), Duration::ZERO);
    assert_eq!(timing.overstep(), Duration::ZERO);
    assert_eq!(timing.overstep_fraction(), 0.0);

    // Bevy's first update initializes Time without advancing it.
    app.update();
    let timing = app.world().resource::<GgrsFrameTiming>();
    assert_eq!(timing.timestep(), Duration::from_millis(20));
    assert_eq!(
        timing.overstep(),
        Duration::ZERO,
        "the initial zero delta should leave the accumulator empty"
    );
    assert_eq!(timing.overstep_fraction(), 0.0);

    // Ten milliseconds is half of the 50 Hz rollback timestep.
    app.update();
    let timing = app.world().resource::<GgrsFrameTiming>();
    assert_eq!(timing.timestep(), Duration::from_millis(20));
    assert_eq!(timing.overstep(), Duration::from_millis(10));
    assert_eq!(timing.overstep_fraction(), 0.5);

    // The next ten milliseconds advances one rollback frame.
    app.update();
    let timing = app.world().resource::<GgrsFrameTiming>();
    assert_eq!(timing.timestep(), Duration::from_millis(20));
    assert_eq!(
        timing.overstep(),
        Duration::ZERO,
        "a complete rollback timestep should drain the accumulator"
    );
    assert_eq!(timing.overstep_fraction(), 0.0);
}
