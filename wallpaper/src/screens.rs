//! The monitors, followed on an event queue of the wallpaper's own, on a
//! thread: each one to cover or let go reaches the wallpaper as an
//! `outputs::Change` (pila-bar's `outputs.rs`, every output wanted). The
//! queue is on iced_layershell's connection (`ui::run` makes it), so the
//! `WlOutput`s sent are ones the layer shell can open surfaces on: an output
//! is opened by its object, never by a name looked up in a cache that may
//! lag behind.

use wayland_client::{
    globals::{registry_queue_init, GlobalListContents},
    protocol::{wl_output, wl_registry},
    Connection, Dispatch, Proxy, QueueHandle,
};

use crate::outputs::{Change, Outputs};

/// The outputs to cover and those gone, from a thread with an event queue
/// of its own on `connection`. It ends with the connection.
pub fn changes(
    connection: Connection,
) -> impl iced::futures::Stream<Item = Change<wl_output::WlOutput>> {
    let (sender, receiver) = iced::futures::channel::mpsc::unbounded();
    let spawned = std::thread::Builder::new()
        .name("pila-wallpaper-outputs".into())
        .spawn(move || {
            let send = |change| sender.unbounded_send(change).is_ok();
            if let Err(err) = follow(&connection, send) {
                eprintln!("pila-wallpaper: {err}");
            }
        });
    if let Err(err) = spawned {
        eprintln!("pila-wallpaper: can't follow the outputs: {err}");
    }
    receiver
}

struct State {
    outputs: Outputs<wl_output::WlOutput>,
    changes: Vec<Change<wl_output::WlOutput>>,
}

/// The wl_output versions bound: `done` from 2, `name` from 4.
const WL_OUTPUT: std::ops::RangeInclusive<u32> = 2..=4;

/// Bind an output, its global's name as its data (for `global_remove`).
fn bind_output(
    state: &mut State,
    registry: &wl_registry::WlRegistry,
    qh: &QueueHandle<State>,
    name: u32,
    version: u32,
) {
    let version = version.min(*WL_OUTPUT.end());
    let output = registry.bind::<wl_output::WlOutput, _, _>(name, version, qh, name);
    state.outputs.added(output);
}

/// Bind the outputs there are and send each change until the connection ends
/// or `send` says no one listens.
fn follow(
    connection: &Connection,
    mut send: impl FnMut(Change<wl_output::WlOutput>) -> bool,
) -> Result<(), String> {
    let (globals, mut queue) = registry_queue_init::<State>(connection)
        .map_err(|err| format!("can't list the globals: {err}"))?;
    let qh = queue.handle();
    let mut state = State {
        outputs: Outputs::new(Vec::new()),
        changes: Vec::new(),
    };
    for global in globals.contents().clone_list() {
        if global.interface == wl_output::WlOutput::interface().name
            && global.version >= *WL_OUTPUT.start()
        {
            bind_output(
                &mut state,
                globals.registry(),
                &qh,
                global.name,
                global.version,
            );
        }
    }
    loop {
        queue
            .blocking_dispatch(&mut state)
            .map_err(|err| format!("lost the compositor: {err}"))?;
        for change in state.changes.drain(..) {
            if !send(change) {
                return Ok(());
            }
        }
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } if interface == wl_output::WlOutput::interface().name
                && version >= *WL_OUTPUT.start() =>
            {
                bind_output(state, registry, qh, name, version)
            }
            wl_registry::Event::GlobalRemove { name } => {
                let output = state
                    .outputs
                    .keys()
                    .find(|o| o.data::<u32>() == Some(&name))
                    .cloned();
                if let Some(output) = output {
                    if let Some(change) = state.outputs.removed(&output) {
                        state.changes.push(change);
                    }
                    if output.version() >= 3 {
                        output.release();
                    }
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_output::WlOutput, u32> for State {
    fn event(
        state: &mut Self,
        output: &wl_output::WlOutput,
        event: wl_output::Event,
        _: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_output::Event::Name { name } => state.outputs.named(output, name),
            wl_output::Event::Done => {
                if let Some(change) = state.outputs.done(output) {
                    state.changes.push(change);
                }
            }
            _ => {}
        }
    }
}
