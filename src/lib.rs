//! bevy_ggrs is a bevy plugin for the P2P rollback networking library GGRS.
//!
//! See [`GgrsPlugin`] for getting started.
//! For an overview of the internals, see the
//! [architecture doc](https://github.com/gschup/bevy_ggrs/blob/main/docs/architecture.md).
#![warn(missing_docs)]
#![allow(clippy::type_complexity)] // Suppress warnings around Query

use bevy::ecs::intern::Interned;
use bevy::ecs::schedule::SingleThreadedExecutor;
use bevy::{
    ecs::schedule::{LogLevel, ScheduleBuildSettings, ScheduleLabel},
    input::InputSystems,
    platform::collections::HashMap,
    prelude::*,
};
use core::time::Duration;
pub use ggrs;
use ggrs::{
    Config, ExternalSession, Frame, InputStatus, P2PSession, PlayerHandle, SpectatorSession,
    SyncTestSession,
};
use serde::{Deserialize, Serialize};
use std::{fmt::Debug, hash::Hash, marker::PhantomData, net::SocketAddr};

pub use snapshot::*;
pub use time::*;

pub(crate) mod schedule_systems;
pub(crate) mod snapshot;
pub(crate) mod time;

/// Convenient re-exports of the most commonly used types. Glob-import this to get started.
pub mod prelude {
    pub use crate::{
        ExternalFrameBudget, ExternalInputs, GgrsConfig, GgrsPlugin, GgrsSchedule, GgrsTime,
        PlayerInputs, ReadInputs, Rollback, RollbackApp, RollbackFrameRate, RollbackId, Session,
        SyncTestMismatch, snapshot::prelude::*,
    };
    pub use ggrs::{GgrsEvent, PlayerType, SessionBuilder};
}

/// A sensible default [GGRS Config](`ggrs::Config`) type suitable for most applications.
///
/// If you require a more specialized configuration, you can create your own type implementing
/// [`Config`](`ggrs::Config`).
#[derive(Debug)]
pub struct GgrsConfig<Input, Address = SocketAddr, State = u8> {
    _phantom: PhantomData<(Input, Address, State)>,
}

impl<Input, Address, State> Config for GgrsConfig<Input, Address, State>
where
    Self: 'static,
    Input: Send + Sync + PartialEq + Serialize + for<'a> Deserialize<'a> + Default + Copy,
    Address: Send + Sync + Debug + Hash + Eq + Clone,
    State: Send + Sync + Clone,
{
    type Input = Input;
    type State = State;
    type Address = Address;
    type InputPredictor = ggrs::PredictRepeatLast;
}

const DEFAULT_FPS: usize = 60;

/// The schedule that runs your rollback game logic each GGRS frame.
///
/// Systems added to this schedule will be saved and rolled back by bevy_ggrs.
/// It runs inside [`AdvanceWorld`] and inherits its ambiguity detection settings
/// (set to [`LogLevel::Error`](`bevy::ecs::schedule::LogLevel`) by default).
///
/// Add your gameplay systems here:
///
/// ```rust,ignore
/// app.add_systems(GgrsSchedule, (move_players, apply_inputs).chain());
/// ```
#[derive(ScheduleLabel, Debug, Hash, PartialEq, Eq, Clone)]
pub struct GgrsSchedule;

/// Defines the Session that the GGRS Plugin should expect as a resource.
#[allow(clippy::large_enum_variant)]
#[derive(Resource)]
pub enum Session<T: Config> {
    /// A local determinism-check session that resimulates every frame to verify rollback correctness.
    SyncTest(SyncTestSession<T>),
    /// A peer-to-peer session with rollback between connected players.
    P2P(P2PSession<T>),
    /// A spectator session that follows a P2P game without participating in input.
    Spectator(SpectatorSession<T>),
    /// A transport-free session driven by externally supplied inputs.
    External(ExternalSession<T>),
}

/// One frame of ordered inputs for an [`ExternalSession`].
#[derive(Resource)]
pub struct ExternalInputs<T: Config> {
    frame: Frame,
    inputs: Vec<Option<T::Input>>,
    more_frames: Vec<Vec<Option<T::Input>>>,
}

impl<T: Config> ExternalInputs<T> {
    /// Creates inputs for one explicit GGRS frame.
    pub fn new(frame: Frame, inputs: Vec<Option<T::Input>>) -> Self {
        Self {
            frame,
            inputs,
            more_frames: Vec::new(),
        }
    }

    /// Creates inputs for consecutive GGRS frames.
    pub fn with_more_frames(
        frame: Frame,
        inputs: Vec<Option<T::Input>>,
        more_frames: Vec<Vec<Option<T::Input>>>,
    ) -> Self {
        Self {
            frame,
            inputs,
            more_frames,
        }
    }

    /// Returns the frame these inputs belong to.
    pub fn frame(&self) -> Frame {
        self.frame
    }

    /// Returns the ordered inputs.
    pub fn inputs(&self) -> &[Option<T::Input>] {
        &self.inputs
    }

    /// Consumes the batch and yields each frame with its ordered inputs.
    pub fn into_iter(self) -> impl Iterator<Item = (Frame, Vec<Option<T::Input>>)> {
        std::iter::once((self.frame, self.inputs)).chain(
            self.more_frames
                .into_iter()
                .enumerate()
                .map(move |(offset, inputs)| (self.frame + offset as Frame + 1, inputs)),
        )
    }
}

/// A one-shot frame execution budget for an [`Session::External`] runner call.
///
/// While this resource is present, the next [`External`](Session::External) runner call
/// skips the realtime fixed-timestep accumulator entirely and executes up to `N` staged
/// forward input frames (see [`ExternalInputs`]). A budget of `0` is a valid explicit
/// budget: it executes zero frames, retains all staged inputs, leaves no accumulated
/// time behind, and is consumed like any other value. Only the *absence* of this
/// resource selects the normal accumulator-gated execution mode.
///
/// The budget is removed as soon as the runner consumes it, so it applies to exactly
/// one runner call. It counts submitted forward input frames only; rollback replay
/// frames that GGRS re-executes internally as part of a single [`ExternalInputs`]
/// submission are not charged against it.
///
/// Insert this resource, run one Bevy update, and the staged inputs advance without
/// waiting for accumulated delta time:
///
/// ```rust,ignore
/// world.insert_resource(ExternalInputs::<GgrsConfig>::with_more_frames(
///     0,
///     vec![Some(input)],
///     more_frames,
/// ));
/// world.insert_resource(ExternalFrameBudget(10));
/// // next `run_ggrs_schedules` call advances up to 10 frames regardless of delta time
/// ```
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExternalFrameBudget(pub u32);

/// A resource holding the inputs for all players in the current GGRS frame.
///
/// Each entry is a `(Input, `[`InputStatus`]`)` pair. The [`InputStatus`] indicates
/// whether the input was received, predicted, or is from a disconnected player.
///
/// This resource is populated by bevy_ggrs before [`GgrsSchedule`] runs and should
/// be read by your input-handling systems.
#[derive(Resource, Deref, DerefMut)]
pub struct PlayerInputs<T: Config>(Vec<(T::Input, InputStatus)>);

#[derive(Resource, Copy, Clone, Debug)]
struct FixedTimestepData {
    /// accumulated time. once enough time has been accumulated, an update is executed
    accumulator: Duration,
    /// boolean to see if we should run slow to let remote clients catch up
    run_slow: bool,
}

impl Default for FixedTimestepData {
    fn default() -> Self {
        Self {
            accumulator: Duration::ZERO,
            run_slow: false,
        }
    }
}

/// The maximum prediction window for this [`Session`], provided as a concrete [`Resource`].
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MaxPredictionWindow(usize);

/// Triggered when a [`SyncTestSession`] detects a checksum mismatch after
/// rollback resimulation. This means the resimulated state diverged from
/// the original — indicating a rollback correctness issue.
///
/// Observe this event to handle desyncs in tests:
///
/// ```rust,ignore
/// app.world_mut().add_observer(|trigger: On<SyncTestMismatch>| {
///     panic!("Desync at frame {}: mismatched frames {:?}",
///         trigger.event().current_frame, trigger.event().mismatched_frames);
/// });
/// ```
#[derive(Event, Debug, Clone)]
pub struct SyncTestMismatch {
    /// The frame at which the mismatch was detected.
    pub current_frame: ggrs::Frame,
    /// The frames whose checksums did not match.
    pub mismatched_frames: Vec<ggrs::Frame>,
}

/// Inputs from local players. You have to fill this resource in the ReadInputs schedule.
#[derive(Resource)]
pub struct LocalInputs<C: Config>(pub HashMap<PlayerHandle, C::Input>);

/// Handles for the local players, you can use this when writing an input system.
#[derive(Resource, Default)]
pub struct LocalPlayers(pub Vec<PlayerHandle>);

/// Label for the schedule which reads the inputs for the current frame
#[derive(ScheduleLabel, Debug, Hash, PartialEq, Eq, Clone)]
pub struct ReadInputs;

/// Resets all GGRS-managed snapshot history and runtime state back to its initial values,
/// so that a fresh session can start from frame 0 in the same Bevy [`World`].
///
/// This runs the [`ResetWorld`] schedule, which is populated automatically by the
/// [`SnapshotPlugin`](snapshot::SnapshotPlugin), by [`GgrsPlugin`], and by every
/// snapshot plugin registered through [`RollbackApp`](snapshot::RollbackApp).
///
/// # Safety contract
///
/// Only call this after a **controlled session stop** (i.e. after you have removed or
/// stopped polling a [`Session`]). The reset itself:
/// - does **not** remove the [`Session`] resource;
/// - does **not** despawn any live entities, including [`Rollback`] entities;
/// - does **not** clear [`RollbackOrdered`] or alter [`RollbackDespawned`].
///
/// It clears stored snapshot histories (keeping their configured depth and reserved
/// capacity) and resets bookkeeping resources such as [`RollbackFrameCount`],
/// [`ConfirmedFrameCount`], `Time<GgrsTime>`, and input buffers.
///
/// # Examples
///
/// ```rust,ignore
/// // Stop the current session deliberately ...
/// world.remove_resource::<Session<MyConfig>>();
///
/// // ... then reset the managed state before starting a fresh one.
/// bevy_ggrs::reset_ggrs_world(world);
/// ```
pub fn reset_ggrs_world(world: &mut World) {
    world.run_schedule(ResetWorld);
}

/// A [`SystemSet`] label for the system that drives all GGRS schedules each Bevy frame.
///
/// Use this to order your systems relative to the GGRS update loop.
/// By default this set runs in [`PreUpdate`], after [`InputSystems`].
#[derive(SystemSet, Hash, Debug, PartialEq, Eq, Clone)]
pub struct RunGgrsSystems;

/// GGRS plugin for bevy.
///
/// # Rollback
///
/// This will provide rollback management for the following items in the Bevy ECS:
/// - [Entities](`Entity`)
/// - [`ChildOf`] and [`Children`] components
/// - [`Time`]
///
/// To add more data to the rollback management, see the methods provided by [`RollbackApp`].
///
/// # Examples
/// ```rust
/// # use bevy::prelude::*;
/// # use bevy_ggrs::prelude::*;
/// #
/// # const FPS: usize = 60;
/// #
/// # type MyInputType = u8;
/// #
/// # fn read_local_inputs() {}
/// #
/// # fn start(session: Session<GgrsConfig<MyInputType>>) {
/// # let mut app = App::new();
/// // Add the GgrsPlugin with your input type
/// app.add_plugins(GgrsPlugin::<GgrsConfig<MyInputType>>::default());
///
/// // (optional) Override the default frequency (60) of rollback game logic updates
/// app.insert_resource(RollbackFrameRate(FPS));
///
/// // Provide a system to get player input
/// app.add_systems(ReadInputs, read_local_inputs);
///
/// // Add custom resources/components to be rolled back
/// app.rollback_component_with_clone::<Transform>();
///
/// // Once started, add your Session
/// app.insert_resource(session);
/// # }
/// ```
pub struct GgrsPlugin<C: Config> {
    schedule: Interned<dyn ScheduleLabel>,
    /// phantom marker for ggrs config
    _marker: PhantomData<C>,
}

impl<C: Config> GgrsPlugin<C> {
    /// Creates a new [`GgrsPlugin`] that runs the GGRS update loop in the given `schedule`.
    ///
    /// Use this when you need GGRS to run in a schedule other than the default [`PreUpdate`].
    pub fn new(schedule: impl ScheduleLabel) -> Self {
        Self {
            schedule: schedule.intern(),
            _marker: default(),
        }
    }
}

impl<C: Config> Default for GgrsPlugin<C> {
    /// Creates a [`GgrsPlugin`] that runs the GGRS update loop in [`PreUpdate`] (the recommended default).
    fn default() -> Self {
        Self {
            schedule: PreUpdate.intern(),
            _marker: default(),
        }
    }
}

/// Resets the GGRS runtime resources specific to config `C` back to their initial values.
///
/// Registered in [`ResetWorld`] by [`GgrsPlugin`]. Complements the snapshot-history
/// clears registered by the snapshot plugins. Does not touch [`Session<C>`], live
/// entities, [`RollbackOrdered`] or [`RollbackDespawned`].
fn reset_runtime_state<C: Config>(
    mut commands: Commands,
    fixed_timestep: Option<ResMut<FixedTimestepData>>,
    frame: Option<ResMut<RollbackFrameCount>>,
    confirmed: Option<ResMut<ConfirmedFrameCount>>,
    max_prediction: Option<ResMut<MaxPredictionWindow>>,
    local_players: Option<ResMut<LocalPlayers>>,
    checksum: Option<ResMut<Checksum>>,
    entity_map: Option<ResMut<RollbackEntityMap>>,
) {
    if let Some(mut data) = fixed_timestep {
        *data = FixedTimestepData::default();
    }
    if let Some(mut frame) = frame {
        *frame = RollbackFrameCount(0);
    }
    if let Some(mut confirmed) = confirmed {
        *confirmed = ConfirmedFrameCount(-1);
    }
    if let Some(mut max_prediction) = max_prediction {
        // Same initial value as the no-session branch of `run_ggrs_schedules`.
        *max_prediction = MaxPredictionWindow(8);
    }
    if let Some(mut local_players) = local_players {
        *local_players = LocalPlayers::default();
    }
    if let Some(mut checksum) = checksum {
        *checksum = Checksum::default();
    }
    if let Some(mut entity_map) = entity_map {
        *entity_map = RollbackEntityMap::default();
    }

    // Input buffers are transient per-frame data; a fresh session must not inherit them.
    // A pending one-shot budget belongs to the old session in the same way.
    commands.remove_resource::<PlayerInputs<C>>();
    commands.remove_resource::<LocalInputs<C>>();
    commands.remove_resource::<ExternalInputs<C>>();
    commands.remove_resource::<ExternalFrameBudget>();
}

impl<C: Config> Plugin for GgrsPlugin<C> {
    /// Registers all GGRS resources, schedules, and the session update system.
    fn build(&self, app: &mut App) {
        app.add_plugins(SnapshotPlugin)
            .init_resource::<MaxPredictionWindow>()
            .init_resource::<LocalPlayers>()
            .init_resource::<FixedTimestepData>()
            .init_schedule(ReadInputs)
            .edit_schedule(AdvanceWorld, |schedule| {
                // AdvanceWorld is mostly a facilitator for GgrsSchedule, so single threading avoids overhead
                // This can be overridden if desired.
                schedule.set_executor(SingleThreadedExecutor::new());
            })
            .edit_schedule(GgrsSchedule, |schedule| {
                schedule.set_build_settings(ScheduleBuildSettings {
                    ambiguity_detection: LogLevel::Error,
                    ..default()
                });
            })
            .add_systems(
                AdvanceWorld,
                (|world: &mut World| world.run_schedule(GgrsSchedule))
                    .in_set(AdvanceWorldSystems::Main),
            )
            .add_systems(
                self.schedule,
                schedule_systems::run_ggrs_schedules::<C>
                    .in_set(RunGgrsSystems)
                    .after(InputSystems), // If we are in PreUpdate, run after input is read
            )
            .add_systems(ResetWorld, reset_runtime_state::<C>)
            .add_plugins((ChecksumPlugin, EntityChecksumPlugin, GgrsTimePlugin));
    }
}
