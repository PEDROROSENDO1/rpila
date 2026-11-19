//! The tray's items as the bar shows them, from what the StatusNotifierItem
//! protocol says: registered and unregistered items, and each one's
//! properties once read. Knows nothing of D-Bus (`I` is an item's icon), so
//! it's tested on its own.

/// An item's `Status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Nothing to show: hidden.
    Passive,
    Active,
    /// Asking for attention: its attention icon, where it has one.
    NeedsAttention,
}

impl Status {
    pub fn parse(text: &str) -> Status {
        match text {
            "Passive" => Status::Passive,
            "NeedsAttention" => Status::NeedsAttention,
            // "Active", and what an item gets wrong: shown.
            _ => Status::Active,
        }
    }
}

/// What the bar needs of an item's properties, its icons already found.
#[derive(Debug, Clone, PartialEq)]
pub struct Props<I> {
    pub title: String,
    pub status: Status,
    pub icon: Option<I>,
    pub attention_icon: Option<I>,
    /// Its com.canonical.dbusmenu object, if it has a menu.
    pub menu: Option<String>,
    /// It only has a menu: a click shows it rather than activating.
    pub item_is_menu: bool,
    /// Its tooltip's lines (`tooltip_lines`); none for no tooltip.
    pub tooltip: Vec<String>,
}

/// An item's menu as last read.
pub type Menu = std::sync::Arc<Vec<super::menu::Entry>>;

/// An item to show.
#[derive(Debug, Clone, PartialEq)]
pub struct Item<I> {
    /// As the watcher names it: a bus name, then maybe an object path.
    pub key: String,
    pub title: String,
    /// The attention icon while it asks for attention, else its icon.
    pub icon: Option<I>,
    pub needs_attention: bool,
    pub menu: Option<String>,
    pub item_is_menu: bool,
    pub tooltip: Vec<String>,
    /// Its menu's entries, once read.
    pub layout: Option<Menu>,
}

#[derive(Debug)]
struct Entry<I> {
    key: String,
    /// None until its properties are read.
    props: Option<Props<I>>,
    layout: Option<Menu>,
}

/// The items registered, in the order they came.
#[derive(Debug)]
pub struct Model<I> {
    entries: Vec<Entry<I>>,
}

impl<I> Default for Model<I> {
    fn default() -> Self {
        Model {
            entries: Vec::new(),
        }
    }
}

impl<I: Clone> Model<I> {
    /// A new item; false if it was there already.
    pub fn registered(&mut self, key: &str) -> bool {
        if self.entries.iter().any(|e| e.key == key) {
            return false;
        }
        self.entries.push(Entry {
            key: key.to_string(),
            props: None,
            layout: None,
        });
        true
    }

    pub fn unregistered(&mut self, key: &str) {
        self.entries.retain(|e| e.key != key);
    }

    /// Its properties, read; nothing for an item gone meanwhile.
    pub fn read(&mut self, key: &str, props: Props<I>) {
        if let Some(entry) = self.entries.iter_mut().find(|e| e.key == key) {
            entry.props = Some(props);
        }
    }

    /// Its menu's entries, read; nothing for an item gone meanwhile.
    pub fn read_layout(&mut self, key: &str, layout: Menu) {
        if let Some(entry) = self.entries.iter_mut().find(|e| e.key == key) {
            entry.layout = Some(layout);
        }
    }

    /// Its menu's object path, if it has a menu.
    pub fn menu(&self, key: &str) -> Option<&str> {
        let entry = self.entries.iter().find(|e| e.key == key)?;
        entry.props.as_ref()?.menu.as_deref()
    }

    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|e| e.key.as_str())
    }

    /// The items to show: those read and not passive, in the order they came.
    pub fn items(&self) -> Vec<Item<I>> {
        self.entries
            .iter()
            .filter_map(|e| Some((e, e.props.as_ref()?)))
            .filter(|(_, p)| p.status != Status::Passive)
            .map(|(e, p)| {
                let needs_attention = p.status == Status::NeedsAttention;
                let icon = match needs_attention {
                    true => p.attention_icon.clone().or_else(|| p.icon.clone()),
                    false => p.icon.clone(),
                };
                Item {
                    key: e.key.clone(),
                    title: p.title.clone(),
                    icon,
                    needs_attention,
                    menu: p.menu.clone(),
                    item_is_menu: p.item_is_menu,
                    tooltip: p.tooltip.clone(),
                    layout: e.layout.clone().filter(|_| p.menu.is_some()),
                }
            })
            .collect()
    }
}

/// The most lines a tooltip shows, and characters a line.
const TOOLTIP_LINES: usize = 4;
const TOOLTIP_LINE_CHARS: usize = 80;

/// An item's tooltip: its `ToolTip`'s title, then its description's lines,
/// as plain text (`plain`); with neither, its `Title`; nothing for none. At
/// most `TOOLTIP_LINES` lines of `TOOLTIP_LINE_CHARS` characters, an
/// ellipsis where one is cut.
pub fn tooltip_lines(title: &str, description: &str, item_title: &str) -> Vec<String> {
    let text = format!("{}\n{}", plain(title), plain(description));
    let mut lines: Vec<String> = text
        .lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|line| !line.is_empty())
        .collect();
    if lines.is_empty() {
        lines = Some(item_title.trim())
            .filter(|t| !t.is_empty())
            .map(String::from)
            .into_iter()
            .collect();
    }
    lines.truncate(TOOLTIP_LINES);
    for line in &mut lines {
        if line.chars().count() > TOOLTIP_LINE_CHARS {
            *line = line
                .chars()
                .take(TOOLTIP_LINE_CHARS - 1)
                .collect::<String>()
                + "\u{2026}";
        }
    }
    lines
}

/// The text of a tooltip's markup (the subset of HTML the protocol allows):
/// a line break for `<br>`, no other tags, entities decoded.
pub fn plain(markup: &str) -> String {
    let mut text = String::new();
    let mut rest = markup;
    while let Some(open) = rest.find('<') {
        text.push_str(&rest[..open]);
        // A tag starts with its name or a `/`; any other `<` is text.
        let starts_tag = rest[open + 1..]
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '/');
        let close = rest[open..].find('>').filter(|_| starts_tag);
        let Some(close) = close else {
            text.push('<');
            rest = &rest[open + 1..];
            continue;
        };
        let tag = rest[open + 1..open + close].trim().to_ascii_lowercase();
        let name = tag.trim_start_matches('/').split([' ', '/']).next();
        if matches!(name, Some("br" | "p" | "div" | "li")) {
            text.push('\n');
        }
        rest = &rest[open + close + 1..];
    }
    text.push_str(rest);
    decode_entities(&text)
}

fn decode_entities(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp + 1..];
        let entity = after
            .find(';')
            .filter(|&end| end <= 10)
            .and_then(|end| Some((entity(&after[..end])?, end)));
        match entity {
            Some((c, end)) => {
                out.push(c);
                rest = &after[end + 1..];
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

fn entity(name: &str) -> Option<char> {
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some(' '),
        _ => {
            let number = name.strip_prefix('#')?;
            let code = match number.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => number.parse().ok()?,
            };
            char::from_u32(code)
        }
    }
}

/// An item's bus name and object path from its key: `service/path`, or a
/// bus name alone for the default path.
pub fn split_key(key: &str) -> (&str, &str) {
    match key.find('/') {
        Some(slash) => (&key[..slash], &key[slash..]),
        None => (key, "/StatusNotifierItem"),
    }
}

/// The key the watcher gives an item that registered with `argument`,
/// asked by `sender`: a bus name stands for itself, an object path is the
/// sender's (Ayatana's libappindicator registers that way).
pub fn item_key(argument: &str, sender: &str) -> String {
    match argument.starts_with('/') {
        true => format!("{sender}{argument}"),
        false => argument.to_string(),
    }
}

/// An `IconPixmap` (ARGB32 in network byte order) as RGBA, or None if its
/// data doesn't match its size.
pub fn rgba(width: i32, height: i32, argb: &[u8]) -> Option<Vec<u8>> {
    let pixels = usize::try_from(width).ok()? * usize::try_from(height).ok()?;
    if pixels == 0 || argb.len() != pixels * 4 {
        return None;
    }
    Some(
        argb.as_chunks::<4>()
            .0
            .iter()
            .flat_map(|[a, r, g, b]| [*r, *g, *b, *a])
            .collect(),
    )
}

/// Which of an item's pixmaps (width, height) to draw at `size` pixels:
/// the smallest at least that big, else the biggest.
pub fn pick_pixmap(sizes: &[(i32, i32)], size: i32) -> Option<usize> {
    let side = |&(w, h): &(i32, i32)| w.max(h);
    let big_enough = sizes
        .iter()
        .enumerate()
        .filter(|(_, s)| side(s) >= size)
        .min_by_key(|(_, s)| side(s));
    big_enough
        .or_else(|| sizes.iter().enumerate().max_by_key(|(_, s)| side(s)))
        .map(|(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn props(title: &str, status: Status, icon: u8) -> Props<u8> {
        Props {
            title: title.into(),
            status,
            icon: Some(icon),
            attention_icon: None,
            menu: None,
            item_is_menu: false,
            tooltip: Vec::new(),
        }
    }

    fn titles(model: &Model<u8>) -> Vec<String> {
        model.items().into_iter().map(|i| i.title).collect()
    }

    #[test]
    fn items_show_once_read_and_not_passive() {
        let mut model = Model::default();
        assert!(model.registered(":1.5/org/ayatana/NotificationItem/a"));
        assert!(model.registered("org.kde.StatusNotifierItem-9-1"));
        assert!(!model.registered("org.kde.StatusNotifierItem-9-1"));
        assert!(titles(&model).is_empty(), "nothing read yet");
        model.read(
            "org.kde.StatusNotifierItem-9-1",
            props("b", Status::Active, 2),
        );
        model.read(
            ":1.5/org/ayatana/NotificationItem/a",
            props("a", Status::Active, 1),
        );
        // In the order they came, not the order they were read.
        assert_eq!(titles(&model), ["a", "b"]);
        model.read(
            "org.kde.StatusNotifierItem-9-1",
            props("b", Status::Passive, 2),
        );
        assert_eq!(titles(&model), ["a"]);
        model.unregistered(":1.5/org/ayatana/NotificationItem/a");
        assert!(titles(&model).is_empty());
        // Read after it went: nothing.
        model.read(
            ":1.5/org/ayatana/NotificationItem/a",
            props("a", Status::Active, 1),
        );
        assert!(titles(&model).is_empty());
    }

    #[test]
    fn attention_shows_the_attention_icon() {
        let mut model = Model::default();
        model.registered("a");
        let mut asking = props("a", Status::NeedsAttention, 1);
        model.read("a", asking.clone());
        assert_eq!(model.items()[0].icon, Some(1), "none of its own: the icon");
        assert!(model.items()[0].needs_attention);
        asking.attention_icon = Some(7);
        model.read("a", asking);
        assert_eq!(model.items()[0].icon, Some(7));
    }

    #[test]
    fn keys_and_registrations() {
        assert_eq!(
            split_key(":1.5/org/ayatana/NotificationItem/a"),
            (":1.5", "/org/ayatana/NotificationItem/a")
        );
        assert_eq!(
            split_key("org.kde.StatusNotifierItem-9-1"),
            ("org.kde.StatusNotifierItem-9-1", "/StatusNotifierItem")
        );
        assert_eq!(item_key("/org/x", ":1.7"), ":1.7/org/x");
        assert_eq!(item_key("org.foo", ":1.7"), "org.foo");
        assert_eq!(Status::parse("NeedsAttention"), Status::NeedsAttention);
        assert_eq!(Status::parse("whatever"), Status::Active);
    }

    #[test]
    fn pixmaps() {
        // One pixel, ARGB: half-transparent red.
        assert_eq!(
            rgba(1, 1, &[0x80, 0xff, 0, 0]),
            Some(vec![0xff, 0, 0, 0x80])
        );
        assert_eq!(rgba(2, 1, &[0; 4]), None, "too short");
        assert_eq!(rgba(0, 0, &[]), None);
        assert_eq!(rgba(-1, -1, &[0; 4]), None);
        let sizes = [(16, 16), (64, 64), (32, 32)];
        assert_eq!(pick_pixmap(&sizes, 24), Some(2));
        assert_eq!(pick_pixmap(&sizes, 16), Some(0));
        assert_eq!(pick_pixmap(&sizes, 100), Some(1), "none big enough");
        assert_eq!(pick_pixmap(&[], 24), None);
    }

    #[test]
    fn a_tooltip_is_its_title_and_description_as_text() {
        assert_eq!(
            tooltip_lines(
                "Wired",
                "Connected to <b>home</b><br/>Speed:  1&nbsp;Gb/s &amp; up",
                "nm-applet",
            ),
            ["Wired", "Connected to home", "Speed: 1 Gb/s & up"]
        );
        // A description alone, in paragraphs.
        assert_eq!(
            tooltip_lines("", "<p>One</p><p>Two &#8212; &#x41;</p>", "x"),
            ["One", "Two \u{2014} A"]
        );
        // Neither: the item's title; nothing at all, no tooltip.
        assert_eq!(tooltip_lines(" ", "<i></i>", "Steam"), ["Steam"]);
        assert!(tooltip_lines("", "", "").is_empty());
    }

    #[test]
    fn a_tooltip_is_kept_short() {
        let long = "word ".repeat(40);
        let lines = tooltip_lines("t", &format!("1<br>{long}<br>3<br>4<br>5"), "");
        assert_eq!(lines, ["t", "1", &lines[2], "3"]);
        assert_eq!(lines[2].chars().count(), TOOLTIP_LINE_CHARS);
        assert!(lines[2].ends_with('\u{2026}'));
    }

    #[test]
    fn markup_that_isnt_markup_stays_text() {
        assert_eq!(plain("a < b & c > d"), "a < b & c > d");
        assert_eq!(plain("x &bogus; y &amp"), "x &bogus; y &amp");
        assert_eq!(plain("1 <2"), "1 <2");
    }
}
