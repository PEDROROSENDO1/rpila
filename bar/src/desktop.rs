//! The desktop the bar shows: the workspaces (ext-workspace) and the focused
//! window's title (wlr-foreign-toplevel-management, whose Activated state
//! is the window with the keyboard), followed on a Wayland connection of the
//! bar's own, on a thread: each change the compositor finishes (`done`)
//! reaches the bar as a new `Desktop`.
//!
//! `Model` turns the protocols' events into that state; it knows nothing of
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
use wayland_protocols_wlr::foreign_toplevel::v1::client::{
    zwlr_foreign_toplevel_handle_v1::{self, ZwlrForeignToplevelHandleV1},
    zwlr_foreign_toplevel_manager_v1::{self, ZwlrForeignToplevelManagerV1},
};

/// What the bar shows of the desktop. `W` names a workspace (its handle).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Desktop<W> {
    /// The workspaces to show, in order.
    pub workspaces: Vec<Workspace<W>>,
    /// The focused window's title (its app id if it has none).
    pub title: Option<String>,
}

impl<W> Default for Desktop<W> {
    fn default() -> Self {
        Desktop {
            workspaces: Vec::new(),
            title: None,
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

#[derive(Debug, Clone, Default)]
struct ToplevelState {
    title: String,
    app_id: String,
    activated: bool,
}

#[derive(Debug)]
struct ToplevelEntry<T> {
    key: T,
    pending: ToplevelState,
    current: Option<ToplevelState>,
}

/// The protocols' events, applied as each says: ext-workspace's on the
/// manager's `done`, a toplevel's on its own `done`. `W` and `T` name a
/// workspace and a toplevel.
#[derive(Debug)]
pub struct Model<W, T> {
    /// In the order they came.
    workspaces: Vec<WorkspaceEntry<W>>,
    toplevels: Vec<ToplevelEntry<T>>,
}

impl<W, T> Default for Model<W, T> {
    fn default() -> Self {
        Model {
            workspaces: Vec::new(),
            toplevels: Vec::new(),
        }
    }
}

impl<W: Clone + PartialEq, T: PartialEq> Model<W, T> {
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

    fn toplevel(&mut self, key: &T) -> Option<&mut ToplevelState> {
        let entry = self.toplevels.iter_mut().find(|e| &e.key == key)?;
        Some(&mut entry.pending)
    }

    pub fn toplevel_added(&mut self, key: T) {
        self.toplevels.push(ToplevelEntry {
            key,
            pending: ToplevelState::default(),
            current: None,
        });
    }

    pub fn toplevel_title(&mut self, key: &T, title: String) {
        if let Some(t) = self.toplevel(key) {
            t.title = title;
        }
    }

    pub fn toplevel_app_id(&mut self, key: &T, app_id: String) {
        if let Some(t) = self.toplevel(key) {
            t.app_id = app_id;
        }
    }

    pub fn toplevel_activated(&mut self, key: &T, activated: bool) {
        if let Some(t) = self.toplevel(key) {
            t.activated = activated;
        }
    }

    pub fn toplevel_done(&mut self, key: &T) {
        if let Some(entry) = self.toplevels.iter_mut().find(|e| &e.key == key) {
            entry.current = Some(entry.pending.clone());
        }
    }

    pub fn toplevel_closed(&mut self, key: &T) {
        self.toplevels.retain(|e| &e.key != key);
    }

    /// The desktop as the compositor last finished saying it: the
    /// workspaces not hidden, by their coordinates (in the order they came
    /// where they have none), and the focused window's title.
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
        let title = self
            .toplevels
            .iter()
            .filter_map(|e| e.current.as_ref())
            .find(|t| t.activated)
            .map(|t| {
                one_line(if t.title.is_empty() {
                    &t.app_id
                } else {
                    &t.title
                })
            })
            .filter(|t| !t.is_empty());
        Desktop { workspaces, title }
    }
}

/// A title on one line: line breaks and tabs become spaces.
fn one_line(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .trim()
        .to_string()
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

type WaylandModel = Model<ExtWorkspaceHandleV1, ZwlrForeignToplevelHandleV1>;

struct State {
    model: WaylandModel,
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
    let toplevels = globals
        .bind::<ZwlrForeignToplevelManagerV1, _, _>(&qh, 1..=3, ())
        .ok();
    if workspaces.is_none() {
        eprintln!("pila-bar: the compositor has no ext-workspace: no workspaces shown");
    }
    if toplevels.is_none() {
        eprintln!(
            "pila-bar: the compositor has no wlr-foreign-toplevel-management: no window title shown"
        );
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

impl Dispatch<ZwlrForeignToplevelManagerV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ZwlrForeignToplevelManagerV1,
        event: zwlr_foreign_toplevel_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_foreign_toplevel_manager_v1::Event::Toplevel { toplevel } => {
                state.model.toplevel_added(toplevel)
            }
            zwlr_foreign_toplevel_manager_v1::Event::Finished => {
                eprintln!("pila-bar: the compositor stopped sending windows")
            }
            _ => {}
        }
    }

    event_created_child!(State, ZwlrForeignToplevelManagerV1, [
        zwlr_foreign_toplevel_manager_v1::EVT_TOPLEVEL_OPCODE => (ZwlrForeignToplevelHandleV1, ()),
    ]);
}

impl Dispatch<ZwlrForeignToplevelHandleV1, ()> for State {
    fn event(
        state: &mut Self,
        toplevel: &ZwlrForeignToplevelHandleV1,
        event: zwlr_foreign_toplevel_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use zwlr_foreign_toplevel_handle_v1::{Event, State as Flag};
        let model = &mut state.model;
        match event {
            Event::Title { title } => model.toplevel_title(toplevel, title),
            Event::AppId { app_id } => model.toplevel_app_id(toplevel, app_id),
            Event::State { state } => {
                let activated = u32_array(&state).contains(&(Flag::Activated as u32));
                model.toplevel_activated(toplevel, activated);
            }
            Event::Done => model.toplevel_done(toplevel),
            Event::Closed => {
                model.toplevel_closed(toplevel);
                toplevel.destroy();
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type TestModel = Model<u32, u32>;

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
    fn the_title_is_the_activated_windows() {
        let mut model = TestModel::default();
        assert_eq!(model.desktop().title, None);
        model.toplevel_added(1);
        model.toplevel_title(&1, "foot".into());
        model.toplevel_activated(&1, true);
        assert_eq!(model.desktop().title, None, "before done");
        model.toplevel_done(&1);
        assert_eq!(model.desktop().title.as_deref(), Some("foot"));

        // Focus moves to a window with no title: its app id.
        model.toplevel_added(2);
        model.toplevel_app_id(&2, "org.example.App".into());
        model.toplevel_activated(&2, true);
        model.toplevel_done(&2);
        model.toplevel_activated(&1, false);
        model.toplevel_done(&1);
        assert_eq!(model.desktop().title.as_deref(), Some("org.example.App"));

        // A new title, on one line.
        model.toplevel_title(&2, "a\nb\tc ".into());
        model.toplevel_done(&2);
        assert_eq!(model.desktop().title.as_deref(), Some("a b c"));

        // Closed, or no window focused: no title.
        model.toplevel_closed(&2);
        assert_eq!(model.desktop().title, None);
        model.toplevel_activated(&1, true);
        model.toplevel_title(&1, "   ".into());
        model.toplevel_done(&1);
        assert_eq!(model.desktop().title, None, "blank");
    }

    #[test]
    fn events_for_unknown_handles_are_ignored() {
        let mut model = TestModel::default();
        model.workspace_name(&9, "x".into());
        model.workspace_removed(&9);
        model.toplevel_done(&9);
        model.toplevel_closed(&9);
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
