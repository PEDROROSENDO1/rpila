//! The desktop the bar shows: the workspaces (ext-workspace), followed on a
//! Wayland connection of the bar's own, on a thread: each change the
//! compositor finishes (`done`) reaches the bar as a new `Desktop`.
//!
//! `Model` turns the protocol's events into that state; it knows nothing of
//! Wayland (its keys are the handles), so it's tested on its own.

use wayland_client::{
    event_created_child,
    globals::{registry_queue_init, GlobalListContents},
    protocol::wl_registry,
    Connection, Dispatch, QueueHandle, WEnum,
};
use wayland_protocols::ext::workspace::v1::client::{
    ext_workspace_group_handle_v1::{self, ExtWorkspaceGroupHandleV1},
    ext_workspace_handle_v1::{self, ExtWorkspaceHandleV1},
    ext_workspace_manager_v1::{self, ExtWorkspaceManagerV1},
};

/// What the bar shows of the desktop. `W` names a workspace (its handle).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Desktop<W> {
    /// The workspaces to show, in order.
    pub workspaces: Vec<Workspace<W>>,
}

impl<W> Default for Desktop<W> {
    fn default() -> Self {
        Desktop {
            workspaces: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace<W> {
    pub key: W,
    pub name: String,
    pub active: bool,
    pub urgent: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct WorkspaceState {
    name: String,
    coordinates: Vec<u32>,
    active: bool,
    urgent: bool,
    hidden: bool,
}

#[derive(Debug)]
struct WorkspaceEntry<W> {
    key: W,
    /// What the compositor said since its last `done`.
    pending: WorkspaceState,
    /// What it said up to then; `None` before the first `done`.
    current: Option<WorkspaceState>,
}

/// ext-workspace's events, applied at the manager's `done`. `W` names a
/// workspace.
#[derive(Debug)]
pub struct Model<W> {
    /// In the order they came.
    workspaces: Vec<WorkspaceEntry<W>>,
}

impl<W> Default for Model<W> {
    fn default() -> Self {
        Model {
            workspaces: Vec::new(),
        }
    }
}

impl<W: Clone + PartialEq> Model<W> {
    fn workspace(&mut self, key: &W) -> Option<&mut WorkspaceState> {
        let entry = self.workspaces.iter_mut().find(|e| &e.key == key)?;
        Some(&mut entry.pending)
    }

    pub fn workspace_added(&mut self, key: W) {
        self.workspaces.push(WorkspaceEntry {
            key,
            pending: WorkspaceState::default(),
            current: None,
        });
    }

    pub fn workspace_name(&mut self, key: &W, name: String) {
        if let Some(w) = self.workspace(key) {
            w.name = name;
        }
    }

    pub fn workspace_coordinates(&mut self, key: &W, coordinates: Vec<u32>) {
        if let Some(w) = self.workspace(key) {
            w.coordinates = coordinates;
        }
    }

    pub fn workspace_state(&mut self, key: &W, active: bool, urgent: bool, hidden: bool) {
        if let Some(w) = self.workspace(key) {
            w.active = active;
            w.urgent = urgent;
            w.hidden = hidden;
        }
    }

    /// Gone at once: the compositor sends nothing more for it.
    pub fn workspace_removed(&mut self, key: &W) {
        self.workspaces.retain(|e| &e.key != key);
    }

    pub fn workspaces_done(&mut self) {
        for entry in &mut self.workspaces {
            entry.current = Some(entry.pending.clone());
        }
    }

    /// The desktop as the compositor last finished saying it: the
    /// workspaces not hidden, by their coordinates (in the order they came
    /// where they have none).
    pub fn desktop(&self) -> Desktop<W> {
        let mut shown: Vec<(&W, &WorkspaceState)> = self
            .workspaces
            .iter()
            .filter_map(|e| Some((&e.key, e.current.as_ref()?)))
            .filter(|(_, w)| !w.hidden)
            .collect();
        // Stable: equal coordinates keep the order they came in.
        shown.sort_by(|a, b| a.1.coordinates.cmp(&b.1.coordinates));
        let workspaces = shown
            .into_iter()
            .map(|(key, w)| Workspace {
                key: key.clone(),
                name: w.name.clone(),
                active: w.active,
                urgent: w.urgent,
            })
            .collect();
        Desktop { workspaces }
    }
}

/// An array of u32 in a protocol event (native byte order).
fn u32_array(bytes: &[u8]) -> Vec<u32> {
    let (chunks, _) = bytes.as_chunks::<4>();
    chunks.iter().map(|c| u32::from_ne_bytes(*c)).collect()
}

/// Switches workspaces, from the bar's thread: requests go out on the
/// desktop thread's connection, which any thread may use.
#[derive(Debug, Clone)]
pub struct Switcher {
    connection: Connection,
    manager: ExtWorkspaceManagerV1,
}

impl Switcher {
    pub fn activate(&self, workspace: &ExtWorkspaceHandleV1) {
        workspace.activate();
        self.manager.commit();
        if let Err(err) = self.connection.flush() {
            eprintln!("pila-bar: can't switch workspaces: {err}");
        }
    }
}

/// What the desktop thread sends the bar.
#[derive(Debug, Clone)]
pub enum Update {
    /// Workspaces can be switched (the compositor has ext-workspace).
    Switcher(Switcher),
    Desktop(Desktop<ExtWorkspaceHandleV1>),
}

/// The desktop's updates, from a thread with a connection of its own. It
/// ends with the connection (the compositor gone: the bar's own goes too).
pub fn updates() -> impl iced::futures::Stream<Item = Update> {
    let (sender, receiver) = iced::futures::channel::mpsc::unbounded();
    let spawned = std::thread::Builder::new()
        .name("pila-bar-desktop".into())
        .spawn(move || {
            if let Err(err) = follow(|update| sender.unbounded_send(update).is_ok()) {
                eprintln!("pila-bar: {err}");
            }
        });
    if let Err(err) = spawned {
        eprintln!("pila-bar: can't follow the desktop: {err}");
    }
    receiver
}

struct State {
    model: Model<ExtWorkspaceHandleV1>,
}

/// Connect, bind what's there and send each change until the connection
/// ends or `send` says no one listens.
fn follow(mut send: impl FnMut(Update) -> bool) -> Result<(), String> {
    let connection =
        Connection::connect_to_env().map_err(|err| format!("can't connect to Wayland: {err}"))?;
    let (globals, mut queue) = registry_queue_init::<State>(&connection)
        .map_err(|err| format!("can't list the globals: {err}"))?;
    let qh = queue.handle();
    let workspaces = globals
        .bind::<ExtWorkspaceManagerV1, _, _>(&qh, 1..=1, ())
        .ok();
    if workspaces.is_none() {
        eprintln!("pila-bar: the compositor has no ext-workspace: no workspaces shown");
    }
    if let Some(manager) = workspaces {
        let switcher = Switcher {
            connection: connection.clone(),
            manager,
        };
        if !send(Update::Switcher(switcher)) {
            return Ok(());
        }
    }
    let mut state = State {
        model: Model::default(),
    };
    let mut shown = Desktop::default();
    loop {
        queue
            .blocking_dispatch(&mut state)
            .map_err(|err| format!("lost the compositor: {err}"))?;
        let desktop = state.model.desktop();
        if desktop != shown {
            shown = desktop.clone();
            if !send(Update::Desktop(desktop)) {
                return Ok(());
            }
        }
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ExtWorkspaceManagerV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ExtWorkspaceManagerV1,
        event: ext_workspace_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_workspace_manager_v1::Event::Workspace { workspace } => {
                state.model.workspace_added(workspace)
            }
            ext_workspace_manager_v1::Event::Done => state.model.workspaces_done(),
            ext_workspace_manager_v1::Event::Finished => {
                eprintln!("pila-bar: the compositor stopped sending workspaces")
            }
            _ => {}
        }
    }

    event_created_child!(State, ExtWorkspaceManagerV1, [
        ext_workspace_manager_v1::EVT_WORKSPACE_GROUP_OPCODE => (ExtWorkspaceGroupHandleV1, ()),
        ext_workspace_manager_v1::EVT_WORKSPACE_OPCODE => (ExtWorkspaceHandleV1, ()),
    ]);
}

impl Dispatch<ExtWorkspaceGroupHandleV1, ()> for State {
    fn event(
        _: &mut Self,
        group: &ExtWorkspaceGroupHandleV1,
        event: ext_workspace_group_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // Groups (which outputs show which workspaces) aren't shown.
        if let ext_workspace_group_handle_v1::Event::Removed = event {
            group.destroy();
        }
    }
}

impl Dispatch<ExtWorkspaceHandleV1, ()> for State {
    fn event(
        state: &mut Self,
        workspace: &ExtWorkspaceHandleV1,
        event: ext_workspace_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use ext_workspace_handle_v1::{Event, State as Flags};
        let model = &mut state.model;
        match event {
            Event::Name { name } => model.workspace_name(workspace, name),
            Event::Coordinates { coordinates } => {
                model.workspace_coordinates(workspace, u32_array(&coordinates))
            }
            Event::State {
                state: WEnum::Value(flags),
            } => model.workspace_state(
                workspace,
                flags.contains(Flags::Active),
                flags.contains(Flags::Urgent),
                flags.contains(Flags::Hidden),
            ),
            Event::Removed => {
                model.workspace_removed(workspace);
                workspace.destroy();
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type TestModel = Model<u32>;

    fn names(desktop: &Desktop<u32>) -> Vec<(&str, bool, bool)> {
        desktop
            .workspaces
            .iter()
            .map(|w| (w.name.as_str(), w.active, w.urgent))
            .collect()
    }

    fn workspaces(model: &mut TestModel, list: &[(u32, &str, bool)]) {
        for &(key, name, active) in list {
            model.workspace_added(key);
            model.workspace_name(&key, name.into());
            model.workspace_coordinates(&key, vec![key]);
            model.workspace_state(&key, active, false, false);
        }
    }

    #[test]
    fn workspaces_show_at_done() {
        let mut model = TestModel::default();
        workspaces(&mut model, &[(0, "1", true), (1, "2", false)]);
        assert!(model.desktop().workspaces.is_empty(), "before done");
        model.workspaces_done();
        assert_eq!(
            names(&model.desktop()),
            [("1", true, false), ("2", false, false)]
        );

        // A switch: both states change, shown together at done.
        model.workspace_state(&0, false, false, false);
        model.workspace_state(&1, true, false, false);
        assert_eq!(
            names(&model.desktop()),
            [("1", true, false), ("2", false, false)]
        );
        model.workspaces_done();
        assert_eq!(
            names(&model.desktop()),
            [("1", false, false), ("2", true, false)]
        );

        // Urgent, and hidden ones left out.
        model.workspace_state(&0, false, true, false);
        workspaces(&mut model, &[(2, "secret", false)]);
        model.workspace_state(&2, false, false, true);
        model.workspaces_done();
        assert_eq!(
            names(&model.desktop()),
            [("1", false, true), ("2", true, false)]
        );

        model.workspace_removed(&0);
        model.workspaces_done();
        assert_eq!(names(&model.desktop()), [("2", true, false)]);
    }

    #[test]
    fn workspaces_go_by_coordinates_then_as_they_came() {
        let mut model = TestModel::default();
        workspaces(&mut model, &[(5, "b", false), (3, "a", true)]);
        model.workspaces_done();
        assert_eq!(
            names(&model.desktop()),
            [("a", true, false), ("b", false, false)]
        );

        let mut model = TestModel::default();
        for (key, name) in [(5, "b"), (3, "a")] {
            model.workspace_added(key);
            model.workspace_name(&key, name.into());
        }
        model.workspaces_done();
        assert_eq!(
            names(&model.desktop()),
            [("b", false, false), ("a", false, false)]
        );
    }

    #[test]
    fn events_for_unknown_handles_are_ignored() {
        let mut model = TestModel::default();
        model.workspace_name(&9, "x".into());
        model.workspace_removed(&9);
        model.workspaces_done();
        assert_eq!(model.desktop(), Desktop::default());
    }

    #[test]
    fn arrays_are_native_u32s() {
        let bytes: Vec<u8> = [2u32, 7].iter().flat_map(|v| v.to_ne_bytes()).collect();
        assert_eq!(u32_array(&bytes), [2, 7]);
        assert_eq!(u32_array(&bytes[..5]), [2]);
    }
}
