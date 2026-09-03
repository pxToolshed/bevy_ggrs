#[allow(dead_code)]
mod common;

use bevy::{prelude::*, time::TimeUpdateStrategy};
use bevy_ggrs::{prelude::*, *};
use common::GgrsConfig;
use core::time::Duration;
use ggrs::{InputStatus, SessionBuilder};

#[derive(Resource, Default)]
struct Observed(Vec<(u8, InputStatus)>);

#[derive(Resource, Copy, Clone, Default, PartialEq, Eq, Hash)]
struct ReadInputsRuns(u32);

fn external_session(players: usize, history: usize) -> Session<GgrsConfig> {
    Session::External(
        SessionBuilder::<GgrsConfig>::new()
            .with_num_players(players)
            .unwrap()
            .with_rollback_history_frames(history)
            .start_external_session(),
    )
}

fn observe_inputs(mut observed: ResMut<Observed>, inputs: Res<PlayerInputs<GgrsConfig>>) {
    observed.0.extend(inputs.iter().copied());
}

fn read_inputs_must_not_run(mut runs: ResMut<ReadInputsRuns>) {
    runs.0 += 1;
}

fn app(session: Session<GgrsConfig>) -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
            1.0 / 60.0,
        )))
        .insert_resource(session)
        .init_resource::<Observed>()
        .init_resource::<ReadInputsRuns>()
        .add_plugins(GgrsPlugin::<GgrsConfig>::default())
        .add_systems(ReadInputs, read_inputs_must_not_run)
        .add_systems(GgrsSchedule, observe_inputs);
    app
}

#[test]
fn external_session_advances_and_preserves_input_status() {
    let mut app = app(external_session(1, 2));
    app.world_mut()
        .insert_resource(ExternalInputs::<GgrsConfig>::new(0, vec![None]));
    app.update();
    app.update();
    assert_eq!(app.world().resource::<RollbackFrameCount>().0, 1);
    assert_eq!(app.world().resource::<Observed>().0[0].0, 0);
    assert!(matches!(
        app.world().resource::<Observed>().0[0].1,
        InputStatus::Predicted
    ));

    app.world_mut()
        .insert_resource(ExternalInputs::<GgrsConfig>::new(1, vec![Some(0)]));
    app.update();
    assert!(matches!(
        app.world().resource::<Observed>().0[1].1,
        InputStatus::Confirmed
    ));
    assert_eq!(app.world().resource::<Observed>().0[1].0, 0);

    app.world_mut()
        .insert_resource(ExternalInputs::<GgrsConfig>::new(2, vec![Some(7)]));
    app.update();
    assert!(matches!(
        app.world().resource::<Observed>().0[2].1,
        InputStatus::Confirmed
    ));
    assert_eq!(app.world().resource::<Observed>().0[2].0, 7);
    assert_eq!(app.world().resource::<ReadInputsRuns>().0, 0);
}

#[test]
fn external_input_is_one_use_and_mismatch_is_retained() {
    let mut app = app(external_session(1, 0));
    app.update();
    app.world_mut()
        .insert_resource(ExternalInputs::<GgrsConfig>::new(1, vec![Some(7)]));
    app.update();
    assert_eq!(app.world().resource::<RollbackFrameCount>().0, 0);
    assert!(
        app.world()
            .get_resource::<ExternalInputs<GgrsConfig>>()
            .is_some()
    );

    app.world_mut()
        .insert_resource(ExternalInputs::<GgrsConfig>::new(0, vec![Some(7)]));
    app.update();
    assert_eq!(app.world().resource::<RollbackFrameCount>().0, 1);
    assert!(
        app.world()
            .get_resource::<ExternalInputs<GgrsConfig>>()
            .is_none()
    );
    app.update();
    assert_eq!(app.world().resource::<RollbackFrameCount>().0, 1);
}

#[test]
fn missing_external_input_preserves_time_for_the_next_update() {
    let mut app = app(external_session(1, 0));
    app.update();
    app.update();
    assert_eq!(app.world().resource::<RollbackFrameCount>().0, 0);
    app.world_mut()
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO));
    app.world_mut()
        .insert_resource(ExternalInputs::<GgrsConfig>::new(0, vec![None]));
    app.update();
    assert_eq!(app.world().resource::<RollbackFrameCount>().0, 1);
}

#[test]
fn invalid_external_input_count_is_recoverable_and_keeps_session() {
    let mut app = app(external_session(1, 0));
    app.update();
    app.world_mut()
        .insert_resource(ExternalInputs::<GgrsConfig>::new(0, Vec::new()));
    app.update();
    assert_eq!(app.world().resource::<RollbackFrameCount>().0, 0);
    assert!(matches!(
        app.world().resource::<Session<GgrsConfig>>(),
        Session::External(_)
    ));
    assert!(
        app.world()
            .get_resource::<ExternalInputs<GgrsConfig>>()
            .is_none()
    );

    app.world_mut()
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO));
    app.world_mut()
        .insert_resource(ExternalInputs::<GgrsConfig>::new(0, vec![Some(9)]));
    app.update();
    assert_eq!(app.world().resource::<RollbackFrameCount>().0, 1);
}

#[test]
fn external_session_advances_once_for_accumulated_time() {
    let mut app = app(external_session(1, 3));
    app.world_mut()
        .insert_resource(ExternalInputs::<GgrsConfig>::new(0, vec![Some(1)]));
    app.world_mut()
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
            3.0 / 60.0,
        )));
    app.update();
    app.update();
    assert_eq!(app.world().resource::<RollbackFrameCount>().0, 1);
    assert_eq!(app.world().resource::<ConfirmedFrameCount>().0, -1);
    assert!(app.world().resource::<LocalPlayers>().0.is_empty());
}

#[test]
fn multi_frame_catchup() {
    let mut app = app(external_session(1, 3));
    app.world_mut()
        .insert_resource(ExternalInputs::<GgrsConfig>::with_more_frames(
            0,
            vec![Some(1)],
            vec![vec![Some(2)], vec![Some(3)]],
        ));
    app.world_mut()
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
            3.0 / 60.0,
        )));
    app.update();
    app.update();
    assert_eq!(app.world().resource::<RollbackFrameCount>().0, 3);
}

#[test]
fn accumulator_limits_batch_and_retains_tail() {
    let mut app = app(external_session(1, 3));
    app.world_mut()
        .insert_resource(ExternalInputs::<GgrsConfig>::with_more_frames(
            0,
            vec![Some(1)],
            vec![vec![Some(2)], vec![Some(3)]],
        ));
    app.update();
    app.update();
    // First update only warms up Time; the second executes one frame of delta ...
    assert_eq!(app.world().resource::<RollbackFrameCount>().0, 1);
    // ... and the not-yet-executed tail is retained losslessly for later updates.
    assert_eq!(
        app.world().resource::<ExternalInputs<GgrsConfig>>().frame(),
        1
    );
    app.update();
    assert_eq!(app.world().resource::<RollbackFrameCount>().0, 2);
    assert_eq!(
        app.world().resource::<ExternalInputs<GgrsConfig>>().frame(),
        2
    );
    app.update();
    assert_eq!(app.world().resource::<RollbackFrameCount>().0, 3);
    assert!(
        app.world()
            .get_resource::<ExternalInputs<GgrsConfig>>()
            .is_none()
    );
}

#[test]
fn normal_mode_preserves_fractional_residue() {
    let mut app = app(external_session(1, 3));
    // Half a frame of delta per update: only every second update may execute.
    app.world_mut()
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
            0.5 / 60.0,
        )));
    app.world_mut()
        .insert_resource(ExternalInputs::<GgrsConfig>::with_more_frames(
            0,
            vec![Some(1)],
            vec![vec![Some(2)]],
        ));
    for frames in [0, 0, 1, 1, 2] {
        app.update();
        assert_eq!(app.world().resource::<RollbackFrameCount>().0, frames);
    }
    // The remainder was never lost: a full delta executes the last staged frame.
    app.update();
    assert_eq!(app.world().resource::<RollbackFrameCount>().0, 2);
}

#[test]
fn missing_inputs_cap_accumulator_at_one_frame() {
    let mut app = app(external_session(1, 0));
    // Pile up several frames of real-time delta while no inputs are staged.
    app.world_mut()
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
            2.5 / 60.0,
        )));
    for _ in 0..4 {
        app.update();
    }
    // Staged inputs with zero delta execute at most one frame: the accumulator
    // was capped while the inputs were missing, so there is no prepaid authority.
    app.world_mut()
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO));
    app.world_mut()
        .insert_resource(ExternalInputs::<GgrsConfig>::new(0, vec![None]));
    app.update();
    assert_eq!(app.world().resource::<RollbackFrameCount>().0, 1);
    // Only a fraction below one frame remains, so the next frame cannot execute.
    app.world_mut()
        .insert_resource(ExternalInputs::<GgrsConfig>::new(1, vec![None]));
    app.update();
    assert_eq!(app.world().resource::<RollbackFrameCount>().0, 1);
}

#[test]
fn zero_delta_budget_executes_staged_frames() {
    let mut app = app(external_session(1, 3));
    app.world_mut()
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO));
    app.world_mut()
        .insert_resource(ExternalInputs::<GgrsConfig>::with_more_frames(
            0,
            vec![Some(1)],
            vec![vec![Some(2)], vec![Some(3)]],
        ));
    app.world_mut().insert_resource(ExternalFrameBudget(3));
    app.update();
    assert_eq!(app.world().resource::<RollbackFrameCount>().0, 3);
    assert!(app.world().get_resource::<ExternalFrameBudget>().is_none());
}

#[test]
fn budget_minimum_executes_one_frame_and_retains_remainder() {
    let mut app = app(external_session(1, 3));
    app.world_mut()
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO));
    app.world_mut()
        .insert_resource(ExternalInputs::<GgrsConfig>::with_more_frames(
            0,
            vec![Some(1)],
            vec![vec![Some(2)], vec![Some(3)]],
        ));
    app.world_mut().insert_resource(ExternalFrameBudget(1));
    app.update();
    assert_eq!(app.world().resource::<RollbackFrameCount>().0, 1);
    assert!(app.world().get_resource::<ExternalFrameBudget>().is_none());
    assert_eq!(
        app.world().resource::<ExternalInputs<GgrsConfig>>().frame(),
        1
    );

    // The budget is one-shot: without a fresh one, nothing executes.
    app.update();
    assert_eq!(app.world().resource::<RollbackFrameCount>().0, 1);

    // A new budget continues from the retained tail, in order.
    app.world_mut().insert_resource(ExternalFrameBudget(1));
    app.update();
    assert_eq!(app.world().resource::<RollbackFrameCount>().0, 2);
    assert_eq!(
        app.world().resource::<ExternalInputs<GgrsConfig>>().frame(),
        2
    );
    app.world_mut().insert_resource(ExternalFrameBudget(1));
    app.update();
    assert_eq!(app.world().resource::<RollbackFrameCount>().0, 3);
    assert!(
        app.world()
            .get_resource::<ExternalInputs<GgrsConfig>>()
            .is_none()
    );
}

#[test]
fn zero_budget_executes_nothing_retains_inputs_and_leaves_no_debt() {
    let mut app = app(external_session(1, 3));
    // Warm up Time so the following update accumulates one full frame of delta.
    app.update();
    app.world_mut()
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
            1.0 / 60.0,
        )));
    app.world_mut()
        .insert_resource(ExternalInputs::<GgrsConfig>::with_more_frames(
            0,
            vec![Some(1)],
            vec![vec![Some(2)]],
        ));
    // One accumulated frame would be available, but the explicit zero budget
    // spends nothing, is consumed, and leaves no accumulator debt behind.
    app.world_mut().insert_resource(ExternalFrameBudget(0));
    app.update();
    assert_eq!(app.world().resource::<RollbackFrameCount>().0, 0);
    assert!(app.world().get_resource::<ExternalFrameBudget>().is_none());
    assert_eq!(
        app.world().resource::<ExternalInputs<GgrsConfig>>().frame(),
        0
    );

    // Zero-delta normal mode on the next update: no advance, proving no debt.
    app.world_mut()
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO));
    app.update();
    assert_eq!(app.world().resource::<RollbackFrameCount>().0, 0);

    // The retained batch is exact and executes intact under a real budget.
    app.world_mut().insert_resource(ExternalFrameBudget(2));
    app.update();
    assert_eq!(app.world().resource::<RollbackFrameCount>().0, 2);
    assert!(
        app.world()
            .get_resource::<ExternalInputs<GgrsConfig>>()
            .is_none()
    );
    let executed: Vec<_> = app
        .world()
        .resource::<Observed>()
        .0
        .iter()
        .map(|(input, _)| *input)
        .collect();
    assert_eq!(executed, vec![1, 2]);
}

#[test]
fn budget_mode_leaves_no_accumulator_debt() {
    let mut app = app(external_session(1, 3));
    // A huge delta would normally pre-pay many frames of authority.
    app.world_mut()
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
            10.0,
        )));
    app.world_mut()
        .insert_resource(ExternalInputs::<GgrsConfig>::new(0, vec![Some(1)]));
    app.world_mut().insert_resource(ExternalFrameBudget(1));
    app.update();
    assert_eq!(app.world().resource::<RollbackFrameCount>().0, 1);

    // The delta must not have survived budget mode: with zero delta and no budget,
    // neither the staged next frame nor anything else may execute.
    app.world_mut()
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO));
    app.world_mut()
        .insert_resource(ExternalInputs::<GgrsConfig>::new(1, vec![Some(2)]));
    app.update();
    assert_eq!(app.world().resource::<RollbackFrameCount>().0, 1);

    // A normal-mode frame still requires fresh accumulated time.
    app.world_mut()
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
            1.0 / 60.0,
        )));
    app.update();
    assert_eq!(app.world().resource::<RollbackFrameCount>().0, 2);
}

#[test]
fn mismatch_retains_remainder_and_consumes_budget() {
    let mut app = app(external_session(1, 3));
    app.world_mut()
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO));
    // The first staged frame targets frame 1 while the session is at frame 0.
    // (`more_frames` entries are re-sequenced consecutively by design, so the
    // mismatch always occurs on the first staged frame.)
    app.world_mut()
        .insert_resource(ExternalInputs::<GgrsConfig>::with_more_frames(
            1,
            vec![Some(7)],
            vec![vec![Some(8)], vec![Some(9)]],
        ));
    app.world_mut().insert_resource(ExternalFrameBudget(3));
    app.update();
    assert_eq!(app.world().resource::<RollbackFrameCount>().0, 0);
    assert!(app.world().get_resource::<ExternalFrameBudget>().is_none());

    // The retained remainder is exactly the staged batch, frames and inputs intact.
    let retained: Vec<_> = app
        .world_mut()
        .remove_resource::<ExternalInputs<GgrsConfig>>()
        .unwrap()
        .into_iter()
        .collect();
    assert_eq!(
        retained,
        vec![(1, vec![Some(7)]), (2, vec![Some(8)]), (3, vec![Some(9)])]
    );

    // Catch the session up, then execute the identical retained payload unchanged.
    app.world_mut()
        .insert_resource(ExternalInputs::<GgrsConfig>::new(0, vec![Some(0)]));
    app.world_mut().insert_resource(ExternalFrameBudget(1));
    app.update();
    assert_eq!(app.world().resource::<RollbackFrameCount>().0, 1);
    app.world_mut()
        .insert_resource(ExternalInputs::<GgrsConfig>::with_more_frames(
            1,
            vec![Some(7)],
            vec![vec![Some(8)], vec![Some(9)]],
        ));
    app.world_mut().insert_resource(ExternalFrameBudget(3));
    app.update();
    assert_eq!(app.world().resource::<RollbackFrameCount>().0, 4);
    assert!(
        app.world()
            .get_resource::<ExternalInputs<GgrsConfig>>()
            .is_none()
    );
    let executed: Vec<_> = app
        .world()
        .resource::<Observed>()
        .0
        .iter()
        .map(|(input, _)| *input)
        .collect();
    assert_eq!(executed, vec![0, 7, 8, 9]);
}

#[test]
fn advance_error_with_budget_drops_failing_frame_and_remainder() {
    let mut app = app(external_session(1, 0));
    app.world_mut()
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO));
    // An empty player list makes `advance_frame` fail for the first frame.
    app.world_mut()
        .insert_resource(ExternalInputs::<GgrsConfig>::with_more_frames(
            0,
            Vec::new(),
            vec![vec![Some(1)]],
        ));
    app.world_mut().insert_resource(ExternalFrameBudget(2));
    app.update();
    assert_eq!(app.world().resource::<RollbackFrameCount>().0, 0);
    assert!(app.world().get_resource::<ExternalFrameBudget>().is_none());
    // Drop policy: the failing frame and the whole remainder are discarded.
    assert!(
        app.world()
            .get_resource::<ExternalInputs<GgrsConfig>>()
            .is_none()
    );

    // The session remains usable for the next staged frame.
    app.world_mut()
        .insert_resource(ExternalInputs::<GgrsConfig>::new(0, vec![Some(9)]));
    app.world_mut().insert_resource(ExternalFrameBudget(1));
    app.update();
    assert_eq!(app.world().resource::<RollbackFrameCount>().0, 1);
}

#[test]
fn mid_batch_mismatch_aborts() {
    let mut app = app(external_session(1, 3));
    app.world_mut()
        .insert_resource(ExternalInputs::<GgrsConfig>::with_more_frames(
            1,
            vec![Some(1)],
            vec![vec![Some(3)], vec![Some(4)]],
        ));
    app.world_mut()
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
            3.0 / 60.0,
        )));
    app.update();
    app.update();
    assert_eq!(app.world().resource::<RollbackFrameCount>().0, 0);
    assert_eq!(
        app.world().resource::<ExternalInputs<GgrsConfig>>().frame(),
        1
    );
}

#[derive(
    Debug, Copy, Clone, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize,
)]
struct AggregateInput {
    values: [u8; 2],
    len: u8,
}

struct AggregateConfig;

impl ggrs::Config for AggregateConfig {
    type Input = AggregateInput;
    type State = u8;
    type Address = usize;
    type InputPredictor = ggrs::PredictRepeatLast;
}

#[derive(Resource, Copy, Clone, Default, PartialEq, Eq, Hash)]
struct AggregateState(u32);

#[derive(Resource, Copy, Clone, Default, PartialEq, Eq)]
struct LoadWorldRuns(u32);

#[derive(Resource, Copy, Clone, Default, PartialEq, Eq)]
struct GgrsScheduleRuns(u32);

fn count_ggrs_schedule_runs(mut runs: ResMut<GgrsScheduleRuns>) {
    runs.0 += 1;
}

#[derive(Resource, Default)]
struct GgrsTimes(Vec<Duration>);

fn record_ggrs_time(time: Res<Time<GgrsTime>>, mut times: ResMut<GgrsTimes>) {
    times.0.push(time.elapsed());
}

fn apply_aggregate_input(
    mut state: ResMut<AggregateState>,
    inputs: Res<PlayerInputs<AggregateConfig>>,
) {
    let input = inputs[0].0;
    state.0 += input.values[..input.len as usize]
        .iter()
        .map(|&value| value as u32)
        .sum::<u32>();
}

fn count_load_world(mut loads: ResMut<LoadWorldRuns>) {
    loads.0 += 1;
}

#[test]
fn external_past_replacement_replays_bevy_world_and_continues() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
            1.0 / 60.0,
        )))
        .insert_resource(Session::External(
            SessionBuilder::<AggregateConfig>::new()
                .with_num_players(1)
                .unwrap()
                .with_rollback_history_frames(4)
                .start_external_session(),
        ))
        .init_resource::<AggregateState>()
        .init_resource::<LoadWorldRuns>()
        .add_plugins(GgrsPlugin::<AggregateConfig>::default())
        .rollback_resource_with_copy::<AggregateState>()
        .checksum_resource_with_hash::<AggregateState>()
        .add_systems(GgrsSchedule, apply_aggregate_input)
        .add_systems(LoadWorld, count_load_world);

    app.update();
    app.world_mut()
        .insert_resource(ExternalInputs::<AggregateConfig>::new(
            0,
            vec![Some(AggregateInput {
                values: [1, 0],
                len: 1,
            })],
        ));
    app.update();
    app.world_mut()
        .insert_resource(ExternalInputs::<AggregateConfig>::new(
            1,
            vec![Some(AggregateInput {
                values: [1, 0],
                len: 1,
            })],
        ));
    app.update();
    for frame in 2..=4 {
        app.world_mut()
            .insert_resource(ExternalInputs::<AggregateConfig>::new(
                frame,
                vec![Some(AggregateInput {
                    values: [1, 0],
                    len: 1,
                })],
            ));
        app.update();
    }
    assert_eq!(app.world().resource::<AggregateState>().0, 5);
    assert!(
        app.world()
            .resource::<GgrsResourceSnapshots<AggregateState>>()
            .peek(0)
            .is_some(),
        "frame 0 snapshot was not retained"
    );

    let expected = match app.world().resource::<Session<AggregateConfig>>() {
        Session::External(session) => session.input_state(0, 1).unwrap(),
        Session::SyncTest(_) | Session::P2P(_) | Session::Spectator(_) => {
            panic!("test session changed type")
        }
    };
    let replacement = AggregateInput {
        values: [1, 2],
        len: 2,
    };
    match &mut *app.world_mut().resource_mut::<Session<AggregateConfig>>() {
        Session::External(session) => {
            let result = session.replace_past_input(0, 1, expected, replacement);
            assert!(result.is_ok(), "past replacement failed: {result:?}");
        }
        Session::SyncTest(_) | Session::P2P(_) | Session::Spectator(_) => {
            panic!("test session changed type")
        }
    }

    app.world_mut()
        .insert_resource(ExternalInputs::<AggregateConfig>::new(
            5,
            vec![Some(AggregateInput {
                values: [3, 0],
                len: 1,
            })],
        ));
    app.update();
    assert!(app.world().resource::<LoadWorldRuns>().0 > 0);
    assert_eq!(app.world().resource::<AggregateState>().0, 10);
    assert_eq!(app.world().resource::<RollbackFrameCount>().0, 6);

    app.world_mut()
        .insert_resource(ExternalInputs::<AggregateConfig>::new(
            6,
            vec![Some(AggregateInput {
                values: [4, 0],
                len: 1,
            })],
        ));
    app.update();
    assert_eq!(app.world().resource::<AggregateState>().0, 14);
    assert_eq!(app.world().resource::<RollbackFrameCount>().0, 7);
}

/// A rollback triggered inside a multi-frame budget must replay the whole corrected
/// past: the number of [`GgrsSchedule`] executions in one runner call can exceed the
/// budget, because the budget only counts submitted forward input frames.
#[test]
fn past_replacement_inside_budget_replays_beyond_budget() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO))
        .insert_resource(Session::External(
            SessionBuilder::<AggregateConfig>::new()
                .with_num_players(1)
                .unwrap()
                .with_rollback_history_frames(4)
                .start_external_session(),
        ))
        .init_resource::<AggregateState>()
        .init_resource::<LoadWorldRuns>()
        .init_resource::<GgrsScheduleRuns>()
        .init_resource::<GgrsTimes>()
        .add_plugins(GgrsPlugin::<AggregateConfig>::default())
        .rollback_resource_with_copy::<AggregateState>()
        .checksum_resource_with_hash::<AggregateState>()
        .add_systems(
            GgrsSchedule,
            (
                apply_aggregate_input,
                count_ggrs_schedule_runs,
                record_ggrs_time,
            ),
        )
        .add_systems(LoadWorld, count_load_world);

    let input = |value: u8| {
        Some(AggregateInput {
            values: [value, 0],
            len: 1,
        })
    };
    app.world_mut().insert_resource(ExternalFrameBudget(5));
    app.world_mut()
        .insert_resource(ExternalInputs::<AggregateConfig>::with_more_frames(
            0,
            vec![input(1)],
            vec![vec![input(1)]; 4],
        ));
    app.update();
    assert_eq!(app.world().resource::<RollbackFrameCount>().0, 5);
    assert_eq!(app.world().resource::<AggregateState>().0, 5);
    assert_eq!(app.world().resource::<GgrsScheduleRuns>().0, 5);

    let expected = match app.world().resource::<Session<AggregateConfig>>() {
        Session::External(session) => session.input_state(0, 1).unwrap(),
        _ => panic!("test session changed type"),
    };
    let replacement = AggregateInput {
        values: [1, 2],
        len: 2,
    };
    match &mut *app.world_mut().resource_mut::<Session<AggregateConfig>>() {
        Session::External(session) => {
            assert!(
                session
                    .replace_past_input(0, 1, expected, replacement)
                    .is_ok()
            );
        }
        _ => panic!("test session changed type"),
    }

    // Two new frames are submitted, but the corrected frame 0 forces a full replay
    // of frames 1..=4 first: 4 replay + 2 forward = 6 executions for a budget of 2.
    app.world_mut().insert_resource(ExternalFrameBudget(2));
    app.world_mut()
        .insert_resource(ExternalInputs::<AggregateConfig>::with_more_frames(
            5,
            vec![input(3)],
            vec![vec![input(4)]],
        ));
    app.update();
    assert!(app.world().resource::<LoadWorldRuns>().0 > 0);
    assert_eq!(app.world().resource::<GgrsScheduleRuns>().0, 11);
    assert_eq!(app.world().resource::<AggregateState>().0, 14);
    assert_eq!(app.world().resource::<RollbackFrameCount>().0, 7);

    // GgrsTime is deterministic: the clock reads (frame + 1) * 1/60 while frame
    // `frame` executes (it advances after RollbackFrameCount). The replay pass of
    // frames 1..=4 yields exactly the same timestamps as the original pass, proving
    // a lossless rewind through the corrected history.
    let frames = [1, 2, 3, 4, 5, 2, 3, 4, 5, 6, 7];
    let expected_times: Vec<Duration> = frames
        .into_iter()
        .map(|frame| Duration::from_nanos(frame as u64 * 1_000_000_000 / 60))
        .collect();
    assert_eq!(app.world().resource::<GgrsTimes>().0, expected_times);

    // After the runner call, the default time is restored from Time<Virtual>.
    let default_elapsed = app.world().resource::<Time<()>>().elapsed();
    assert_eq!(
        default_elapsed,
        app.world().resource::<Time<Virtual>>().elapsed()
    );
}
