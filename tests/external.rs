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
