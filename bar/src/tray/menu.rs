//! An item's menu (com.canonical.dbusmenu): its entries as `GetLayout`
//! gives them, read ahead of a click so that the menu opens while the
//! button is still down (a popup's grab needs that press).

use zbus::zvariant::Value;

/// A menu entry the bar shows.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub id: i32,
    /// Its label, mnemonics (`_File`) taken out.
    pub label: String,
    pub enabled: bool,
    pub separator: bool,
    pub toggle: Toggle,
    /// A submenu's entries.
    pub children: Vec<Entry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Toggle {
    None,
    Check(bool),
    Radio(bool),
}

/// The entries in a layout's `av` of children, those visible.
pub fn entries<'a>(children: impl IntoIterator<Item = &'a Value<'a>>) -> Vec<Entry> {
    children.into_iter().filter_map(entry).collect()
}

/// One `(ia{sv}av)` layout, or None if it isn't one or is hidden.
fn entry(value: &Value) -> Option<Entry> {
    let Value::Structure(structure) = unwrap(value) else {
        return None;
    };
    let [Value::I32(id), Value::Dict(props), Value::Array(children)] = structure.fields() else {
        return None;
    };
    let mut entry = Entry {
        id: *id,
        label: String::new(),
        enabled: true,
        separator: false,
        toggle: Toggle::None,
        children: Vec::new(),
    };
    let mut visible = true;
    let (mut toggle_type, mut toggle_state) = (String::new(), 0);
    for (key, value) in props.iter() {
        let (Value::Str(key), value) = (key, unwrap(value)) else {
            continue;
        };
        match (key.as_str(), value) {
            ("label", Value::Str(label)) => entry.label = without_mnemonics(label),
            ("enabled", Value::Bool(enabled)) => entry.enabled = *enabled,
            ("visible", Value::Bool(shown)) => visible = *shown,
            ("type", Value::Str(kind)) => entry.separator = kind.as_str() == "separator",
            ("toggle-type", Value::Str(kind)) => toggle_type = kind.to_string(),
            ("toggle-state", Value::I32(state)) => toggle_state = *state,
            _ => {}
        }
    }
    entry.toggle = match toggle_type.as_str() {
        "checkmark" => Toggle::Check(toggle_state == 1),
        "radio" => Toggle::Radio(toggle_state == 1),
        _ => Toggle::None,
    };
    entry.children = entries(children.iter());
    visible.then_some(entry)
}

/// A variant's value.
fn unwrap<'a>(value: &'a Value<'a>) -> &'a Value<'a> {
    match value {
        Value::Value(inner) => unwrap(inner),
        other => other,
    }
}

/// `_File` as `File`, `__` as `_`.
fn without_mnemonics(label: &str) -> String {
    let mut text = String::with_capacity(label.len());
    let mut chars = label.chars();
    while let Some(c) = chars.next() {
        if c == '_' {
            if let Some(next) = chars.next() {
                text.push(next);
            }
        } else {
            text.push(c);
        }
    }
    text
}

/// The rows the tallest level of `entries` takes (a submenu shown in place
/// of its parent adds a row to go back), and its longest label in chars:
/// what the popup is sized for.
pub fn extent(entries: &[Entry]) -> (usize, usize, usize) {
    let rows = entries.iter().filter(|e| !e.separator).count();
    let separators = entries.len() - rows;
    let widest = entries
        .iter()
        .map(|e| e.label.chars().count())
        .max()
        .unwrap_or(0);
    entries
        .iter()
        .filter(|e| !e.children.is_empty())
        .map(|e| extent(&e.children))
        .fold((rows, separators, widest), |(r, s, w), (cr, cs, cw)| {
            // A submenu's level has the back row too.
            let (r, s) = if cr + 1 + cs > r + s {
                (cr + 1, cs)
            } else {
                (r, s)
            };
            (r, s, w.max(cw))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::zvariant::{Array, Dict, Signature, Structure, StructureBuilder};

    fn layout(
        id: i32,
        props: &[(&str, Value<'static>)],
        children: Vec<Value<'static>>,
    ) -> Value<'static> {
        let mut dict = Dict::new(&Signature::Str, &Signature::Variant);
        for (key, value) in props {
            dict.append(
                Value::from(key.to_string()),
                Value::Value(Box::new(value.clone())),
            )
            .unwrap();
        }
        let mut array = Array::new(&Signature::Variant);
        for child in children {
            array.append(Value::Value(Box::new(child))).unwrap();
        }
        let structure: Structure = StructureBuilder::new()
            .add_field(id)
            .append_field(Value::Dict(dict))
            .append_field(Value::Array(array))
            .build()
            .unwrap();
        Value::Structure(structure)
    }

    #[test]
    fn a_layout_as_entries() {
        let quit = layout(4, &[("label", "_Quit".into())], vec![]);
        let hidden = layout(
            5,
            &[("label", "Debug".into()), ("visible", false.into())],
            vec![],
        );
        let separator = layout(3, &[("type", "separator".into())], vec![]);
        let pinyin = layout(
            11,
            &[
                ("label", "Pinyin".into()),
                ("toggle-type", "radio".into()),
                ("toggle-state", 1.into()),
            ],
            vec![],
        );
        let methods = layout(10, &[("label", "Input __Method".into())], vec![pinyin]);
        let check = layout(
            2,
            &[
                ("label", "Enabled".into()),
                ("toggle-type", "checkmark".into()),
                ("toggle-state", 0.into()),
                ("enabled", false.into()),
            ],
            vec![],
        );
        let children = [methods, check, separator, hidden, quit];
        let entries = entries(children.iter());
        let labels: Vec<&str> = entries.iter().map(|e| e.label.as_str()).collect();
        assert_eq!(labels, ["Input _Method", "Enabled", "", "Quit"]);
        assert_eq!(entries[0].children[0].label, "Pinyin");
        assert_eq!(entries[0].children[0].toggle, Toggle::Radio(true));
        assert_eq!(entries[1].toggle, Toggle::Check(false));
        assert!(!entries[1].enabled);
        assert!(entries[2].separator);
        assert_eq!(entries[3].id, 4);
        // Three rows and a separator; the submenu's one row and a back row.
        assert_eq!(extent(&entries), (3, 1, 13));
    }

    #[test]
    fn not_a_layout() {
        assert!(entries([&Value::from(3)]).is_empty());
        assert_eq!(without_mnemonics("a_"), "a");
    }
}
