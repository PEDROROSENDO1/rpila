//! Which built-in icon (`symbols.rs`) the bar shows: the battery's level by
//! tens, with a bolt inside while charging, and a clock.

use crate::{
    battery::{Battery, State},
    symbols::Symbol,
};

/// The batteries, 0% to 100% by tens: as they are, and charging.
const TENS: [Symbol; 11] = [
    Symbol::Battery0,
    Symbol::Battery10,
    Symbol::Battery20,
    Symbol::Battery30,
    Symbol::Battery40,
    Symbol::Battery50,
    Symbol::Battery60,
    Symbol::Battery70,
    Symbol::Battery80,
    Symbol::Battery90,
    Symbol::Battery100,
];
const TENS_CHARGING: [Symbol; 11] = [
    Symbol::Charging0,
    Symbol::Charging10,
    Symbol::Charging20,
    Symbol::Charging30,
    Symbol::Charging40,
    Symbol::Charging50,
    Symbol::Charging60,
    Symbol::Charging70,
    Symbol::Charging80,
    Symbol::Charging90,
    Symbol::Charging100,
];

/// The clock's, before the time.
pub const CLOCK: Symbol = Symbol::Clock;

/// The battery's one icon: its level to the nearest ten, with a bolt
/// inside while charging. Plugged in and not charging, its level alone.
pub fn battery(battery: &Battery) -> Symbol {
    let level = (usize::from(battery.percent.min(100)) + 5) / 10;
    match battery.state {
        State::Charging => TENS_CHARGING[level],
        _ => TENS[level],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_battery_icon() {
        let at = |percent, state| battery(&Battery { percent, state });
        assert_eq!(at(100, State::Discharging), Symbol::Battery100);
        assert_eq!(at(64, State::Discharging), Symbol::Battery60);
        assert_eq!(at(65, State::Discharging), Symbol::Battery70);
        assert_eq!(at(4, State::Discharging), Symbol::Battery0);
        assert_eq!(at(42, State::Charging), Symbol::Charging40);
        assert_eq!(at(100, State::Charging), Symbol::Charging100);
        assert_eq!(at(100, State::Plugged), Symbol::Battery100);
    }
}
