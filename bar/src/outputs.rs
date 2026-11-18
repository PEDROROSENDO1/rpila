//! Which outputs get a bar: all of them, or those `--output` names. The
//! outputs come from the bar's own connection (`desktop.rs`); `Outputs` says
//! when one should get a surface and when its surface goes. It knows
//! nothing of Wayland (`O` names an output), so it's tested on its own.

/// The outputs there are, and which have a bar.
#[derive(Debug)]
pub struct Outputs<O> {
    /// The names to show the bar on; empty for every output.
    wanted: Vec<String>,
    entries: Vec<Entry<O>>,
}

#[derive(Debug)]
struct Entry<O> {
    key: O,
    /// None until the compositor says it (wl_output 4), or if it never does.
    name: Option<String>,
    shown: bool,
}

/// What to do with an output's surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change<O> {
    Open(O),
    Close(O),
}

impl<O: Clone + PartialEq> Outputs<O> {
    pub fn new(wanted: Vec<String>) -> Self {
        Outputs {
            wanted,
            entries: Vec::new(),
        }
    }

    pub fn added(&mut self, key: O) {
        self.entries.push(Entry {
            key,
            name: None,
            shown: false,
        });
    }

    pub fn named(&mut self, key: &O, name: String) {
        if let Some(entry) = self.entries.iter_mut().find(|e| &e.key == key) {
            entry.name = Some(name);
        }
    }

    /// The compositor said all it had to about the output: it gets its bar
    /// if it's one of those wanted (once).
    pub fn done(&mut self, key: &O) -> Option<Change<O>> {
        let wanted = &self.wanted;
        let entry = self.entries.iter_mut().find(|e| &e.key == key)?;
        let show = wanted.is_empty()
            || entry
                .name
                .as_ref()
                .is_some_and(|name| wanted.contains(name));
        if show && !entry.shown {
            entry.shown = true;
            return Some(Change::Open(entry.key.clone()));
        }
        None
    }

    pub fn keys(&self) -> impl Iterator<Item = &O> {
        self.entries.iter().map(|e| &e.key)
    }

    pub fn removed(&mut self, key: &O) -> Option<Change<O>> {
        let index = self.entries.iter().position(|e| &e.key == key)?;
        let entry = self.entries.remove(index);
        entry.shown.then_some(Change::Close(entry.key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output(outputs: &mut Outputs<u32>, key: u32, name: &str) -> Option<Change<u32>> {
        outputs.added(key);
        outputs.named(&key, name.into());
        outputs.done(&key)
    }

    #[test]
    fn every_output_by_default() {
        let mut outputs = Outputs::new(Vec::new());
        assert_eq!(output(&mut outputs, 1, "eDP-1"), Some(Change::Open(1)));
        assert_eq!(output(&mut outputs, 2, "DP-1"), Some(Change::Open(2)));
        // A later done (a mode change) opens nothing more.
        assert_eq!(outputs.done(&1), None);
        // One with no name too.
        outputs.added(3);
        assert_eq!(outputs.done(&3), Some(Change::Open(3)));
        assert_eq!(outputs.removed(&2), Some(Change::Close(2)));
        assert_eq!(outputs.removed(&2), None);
    }

    #[test]
    fn only_the_outputs_named() {
        let mut outputs = Outputs::new(vec!["DP-1".into()]);
        assert_eq!(output(&mut outputs, 1, "eDP-1"), None);
        assert_eq!(output(&mut outputs, 2, "DP-1"), Some(Change::Open(2)));
        outputs.added(3);
        assert_eq!(outputs.done(&3), None, "no name, not wanted");
        // Unplugged and plugged again: closed, then opened anew.
        assert_eq!(outputs.removed(&1), None);
        assert_eq!(outputs.removed(&2), Some(Change::Close(2)));
        assert_eq!(output(&mut outputs, 4, "DP-1"), Some(Change::Open(4)));
    }

    #[test]
    fn events_for_unknown_outputs_are_ignored() {
        let mut outputs = Outputs::<u32>::new(Vec::new());
        outputs.named(&9, "x".into());
        assert_eq!(outputs.done(&9), None);
        assert_eq!(outputs.removed(&9), None);
    }
}
