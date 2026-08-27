//! Tests for the explicit [`bevy_ggrs::ResetWorld`] / [`bevy_ggrs::reset_ggrs_world`] API:
//! snapshot histories are cleared, runtime state resets to initial values, and neither
//! the [`Session`] nor live rollback entities are touched.

#[allow(dead_code)]
mod common;

use bevy::{prelude::*, time::TimeUpdateStrategy};
use bevy_ggrs::{prelude::*, *};
use common::GgrsConfig;
use core::time::Duration;
use ggrs::SessionBuilder;

fn external_session(history: usize) -> Session<GgrsConfig> {
    Session::External(
        SessionBuilder::<GgrsConfig>::new()
            .with_num_players(1)
            .unwrap()
            .with_rollback_history_frames(history)
            .start_external_session(),
    )
}

#[derive(Component, Clone, Copy, Debug, PartialEq)]
#[require(Rollback)]
struct Pos(u8);

#[derive(Component, Clone, Copy, Debug, PartialEq)]
#[component(immutable)]
#[require(Rollback)]
struct Frozen(u8);

#[derive(Resource, Clone, Copy, Default, Debug, PartialEq)]
struct Stat(u32);

/// Applies stored state mutators against a bare [`App`] to build the snapshot-history setup.
#[test]
fn reset_clears_snapshot_stores_and_keeps_depth() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(GgrsPlugin::<GgrsConfig>::default())
        .rollback_component_with_clone::<Pos>()
        .rollback_immutable_component_with_clone::<Frozen>()
        .rollback_resource_with_clone::<Stat>()
        .insert_resource(Stat(7));

    let world = app.world_mut();
    let entity = world.spawn((Pos(1), Frozen(2))).id();
    let rollback_id = *world.entity(entity).get::<RollbackId>().unwrap();

    // Grow the reserved snapshot window beyond the default so clearing is
    // observable while the configured depth stays verifiable.
    world
        .resource_mut::<GgrsComponentSnapshots<Entity>>()
        .set_depth(6);
    world
        .resource_mut::<GgrsComponentSnapshots<Pos>>()
        .set_depth(6);
    world
        .resource_mut::<GgrsComponentSnapshots<Frozen>>()
        .set_depth(6);
    world
        .resource_mut::<GgrsResourceSnapshots<Stat>>()
        .set_depth(6);

    // Fill every snapshot store over four frames via the public store API.
    // (Directly pushed, because `SaveWorld`'s sync_depth would otherwise clamp the
    // depth back to MaxPredictionWindow's default, which is 0 outside a session.)
    {
        let mut entity_store = world.resource_mut::<GgrsComponentSnapshots<Entity>>();
        for frame in 0..4_i32 {
            entity_store.push(frame, GgrsComponentSnapshot::new([(rollback_id, entity)]));
        }
    }
    for frame in 0..4_i32 {
        world.resource_mut::<GgrsComponentSnapshots<Pos>>().push(
            frame,
            GgrsComponentSnapshot::new([(rollback_id, Pos(frame as u8))]),
        );
        world.resource_mut::<GgrsComponentSnapshots<Frozen>>().push(
            frame,
            GgrsComponentSnapshot::new([(rollback_id, Frozen(frame as u8))]),
        );
        world
            .resource_mut::<GgrsResourceSnapshots<Stat>>()
            .push(frame, Some(Stat(frame as u32)));
    }

    let entity_store = world.resource::<GgrsComponentSnapshots<Entity>>();
    assert_eq!(entity_store.depth(), 6);
    assert!(
        (0..4).all(|f| entity_store.peek(f).is_some()),
        "entity snapshots should exist before reset"
    );
    let pos_store = world.resource::<GgrsComponentSnapshots<Pos>>();
    assert!((0..4).all(|f| pos_store.peek(f).is_some()));
    let frozen_store = world.resource::<GgrsComponentSnapshots<Frozen>>();
    assert!((0..4).all(|f| frozen_store.peek(f).is_some()));
    let stat_store = world.resource::<GgrsResourceSnapshots<Stat>>();
    assert!((0..4).all(|f| stat_store.peek(f).is_some()));

    // Give RollbackEntityMap content so we can watch it reset.
    world.resource_mut::<RollbackOrdered>();

    let ordered_before: Vec<_> = world.resource::<RollbackOrdered>().iter_sorted().collect();
    let rollback_entities: Vec<_> = world
        .query_filtered::<Entity, With<Rollback>>()
        .iter(world)
        .collect();

    bevy_ggrs::reset_ggrs_world(world);

    // All snapshot histories are gone ...
    let world = app.world();
    for frame in 0..4 {
        assert!(
            world
                .resource::<GgrsComponentSnapshots<Entity>>()
                .peek(frame)
                .is_none()
        );
        assert!(
            world
                .resource::<GgrsComponentSnapshots<Pos>>()
                .peek(frame)
                .is_none()
        );
        assert!(
            world
                .resource::<GgrsComponentSnapshots<Frozen>>()
                .peek(frame)
                .is_none()
        );
        assert!(
            world
                .resource::<GgrsResourceSnapshots<Stat>>()
                .peek(frame)
                .is_none()
        );
    }

    // ... but the configured depth survived.
    assert_eq!(
        world.resource::<GgrsComponentSnapshots<Entity>>().depth(),
        6,
        "depth must survive a reset"
    );
    assert_eq!(world.resource::<GgrsComponentSnapshots<Pos>>().depth(), 6);
    assert_eq!(
        world.resource::<GgrsComponentSnapshots<Frozen>>().depth(),
        6
    );
    assert_eq!(world.resource::<GgrsResourceSnapshots<Stat>>().depth(), 6);

    // Live data is untouched.
    let world = app.world_mut();
    assert_eq!(
        world
            .query_filtered::<Entity, With<Rollback>>()
            .iter(world)
            .count(),
        rollback_entities.len(),
        "live rollback entities must survive"
    );
    let ordered_after: Vec<_> = world.resource::<RollbackOrdered>().iter_sorted().collect();
    assert!(
        !ordered_after.is_empty(),
        "RollbackOrdered must not be emptied by a reset"
    );
    assert_eq!(
        ordered_after, ordered_before,
        "RollbackOrdered must not change"
    );
    assert_eq!(
        world.resource::<Stat>(),
        &Stat(7),
        "live resources must survive"
    );

    // Snapshots can be recorded again afterwards (stores remain functional).
    let world = app.world_mut();
    world
        .resource_mut::<GgrsComponentSnapshots<Pos>>()
        .push(5, GgrsComponentSnapshot::new([(rollback_id, Pos(9))]));
    assert!(
        world
            .resource::<GgrsComponentSnapshots<Pos>>()
            .peek(5)
            .is_some()
    );
}

#[test]
fn reset_restores_runtime_state_at_zero_delta_and_keeps_session() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO))
        .insert_resource(external_session(0))
        .add_plugins(GgrsPlugin::<GgrsConfig>::default());

    // The stale session left arbitrary runtime state behind.
    *app.world_mut().resource_mut::<RollbackFrameCount>() = RollbackFrameCount(5);
    *app.world_mut().resource_mut::<ConfirmedFrameCount>() = ConfirmedFrameCount(3);
    *app.world_mut().resource_mut::<Checksum>() = Checksum(12345);
    app.world_mut()
        .resource_mut::<Time<GgrsTime>>()
        .advance_by(Duration::from_secs(42));
    app.world_mut().resource_mut::<LocalPlayers>().0.push(0);
    app.world_mut()
        .insert_resource(LocalInputs::<GgrsConfig>(default()));

    // Reset works fine while the real-time delta is zero.
    bevy_ggrs::reset_ggrs_world(app.world_mut());

    assert_eq!(
        app.world().resource::<RollbackFrameCount>().0,
        0,
        "frame counter must reset to 0"
    );
    assert_eq!(
        app.world().resource::<ConfirmedFrameCount>().0,
        -1,
        "confirmed frame must reset to -1"
    );
    assert_eq!(app.world().resource::<Checksum>().0, 0);
    assert_eq!(
        app.world().resource::<Time<GgrsTime>>().elapsed(),
        Duration::ZERO
    );
    assert_eq!(
        app.world().resource::<Time<GgrsTime>>().delta(),
        Duration::ZERO
    );
    assert!(app.world().resource::<LocalPlayers>().0.is_empty());

    assert!(
        app.world()
            .get_resource::<LocalInputs<GgrsConfig>>()
            .is_none(),
        "LocalInputs must be removed"
    );
    assert!(
        app.world()
            .get_resource::<PlayerInputs<GgrsConfig>>()
            .is_none(),
        "PlayerInputs must be removed"
    );
    assert!(
        app.world()
            .get_resource::<ExternalInputs<GgrsConfig>>()
            .is_none(),
        "ExternalInputs must be removed"
    );

    // The session itself survives the reset.
    assert!(
        matches!(
            app.world().resource::<Session<GgrsConfig>>(),
            Session::External(_)
        ),
        "reset must not remove the Session"
    );

    // The Entity->Entity mapping registry is reset as well.
    assert!(app.world().resource::<RollbackEntityMap>().is_empty());
}

#[test]
fn controlled_stop_reset_restarts_external_session_at_frame_zero() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
            1.0 / 60.0,
        )))
        .insert_resource(external_session(0))
        .add_plugins(GgrsPlugin::<GgrsConfig>::default());

    // Run two frames of a first session. (The first update only warms up Bevy's
    // Time<Real> and accumulates; advancing starts on later updates.)
    app.world_mut()
        .insert_resource(ExternalInputs::<GgrsConfig>::new(0, vec![Some(1)]));
    app.update();
    app.update();
    app.world_mut()
        .insert_resource(ExternalInputs::<GgrsConfig>::new(1, vec![Some(2)]));
    app.update();
    assert_eq!(app.world().resource::<RollbackFrameCount>().0, 2);

    // Stop delivering inputs: each update piles leftover time into the fixed-timestep
    // accumulator (this is what a stale accumulator looks like across a restart).
    for _ in 0..5 {
        app.update();
    }

    // Controlled session stop followed by the explicit reset.
    app.world_mut().remove_resource::<Session<GgrsConfig>>();
    bevy_ggrs::reset_ggrs_world(app.world_mut());

    // A fresh session starts at frame 0 ...
    app.insert_resource(external_session(0));
    assert!(matches!(
        app.world().resource::<Session<GgrsConfig>>(),
        Session::External(_)
    ));
    let current_frame = match app.world().resource::<Session<GgrsConfig>>() {
        Session::External(session) => session.current_frame(),
        _ => unreachable!(),
    };
    assert_eq!(current_frame, 0);

    // ... and with a zero real-time delta (plus cleared accumulator) it cannot advance,
    // proving FixedTimestepData was actually reset rather than carrying leftovers.
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO));
    app.update();
    assert_eq!(app.world().resource::<RollbackFrameCount>().0, 0);

    // First delivered input drives the fresh session from frame 0 to frame 1.
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
        1.0 / 60.0,
    )));
    app.world_mut()
        .insert_resource(ExternalInputs::<GgrsConfig>::new(0, vec![Some(7)]));
    app.update();
    app.update();

    assert_eq!(app.world().resource::<RollbackFrameCount>().0, 1);
}
