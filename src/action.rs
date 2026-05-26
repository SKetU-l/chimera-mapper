use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::str::FromStr;

macro_rules! define_str_enum {
    (
        $(#[$meta:meta])*
        pub enum $name:ident {
            $( $variant:ident => $canonical:literal $( | $alias:literal )* ),* $(,)?
        }
    ) => {
        $(#[$meta])*
        pub enum $name {
            $( $variant ),*
        }

        impl $name {
            pub fn from_str_lower(s: &str) -> Option<Self> {
                match s {
                    $( $canonical $( | $alias )* => Some($name::$variant), )*
                    _ => None,
                }
            }

            pub fn canonical_name(&self) -> &'static str {
                match self {
                    $( $name::$variant => $canonical, )*
                }
            }
        }
    };
}

define_str_enum! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub enum Modifier {
        Ctrl  => "ctrl" | "control",
        Shift => "shift",
        Alt   => "alt" | "option",
        Meta  => "meta" | "super" | "command" | "cmd" | "win",
    }
}

define_str_enum! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub enum Key {
        A => "a", B => "b", C => "c", D => "d", E => "e",
        F => "f", G => "g", H => "h", I => "i", J => "j",
        K => "k", L => "l", M => "m", N => "n", O => "o",
        P => "p", Q => "q", R => "r", S => "s", T => "t",
        U => "u", V => "v", W => "w", X => "x", Y => "y",
        Z => "z",
        Num0 => "0", Num1 => "1", Num2 => "2", Num3 => "3",
        Num4 => "4", Num5 => "5", Num6 => "6", Num7 => "7",
        Num8 => "8", Num9 => "9",
        F1 => "f1", F2 => "f2", F3 => "f3", F4 => "f4",
        F5 => "f5", F6 => "f6", F7 => "f7", F8 => "f8",
        F9 => "f9", F10 => "f10", F11 => "f11", F12 => "f12",
        Enter     => "enter" | "return",
        Space     => "space",
        Tab       => "tab",
        Backspace => "backspace",
        Escape    => "escape" | "esc",
        Delete    => "delete" | "del",
        Insert    => "insert" | "ins",
        Home      => "home",
        End       => "end",
        PageUp    => "pageup" | "pgup",
        PageDown  => "pagedown" | "pgdn",
        Left      => "left" | "arrowleft",
        Right     => "right" | "arrowright",
        Up        => "up" | "arrowup",
        Down      => "down" | "arrowdown",
    }
}

define_str_enum! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub enum MouseButton {
        Left    => "mouse_left" | "btn_left",
        Right   => "mouse_right" | "btn_right",
        Middle  => "mouse_middle" | "btn_middle",
        Back    => "mouse_back" | "btn_side" | "back",
        Forward => "mouse_forward" | "btn_extra" | "forward",
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    Keys {
        modifiers: Vec<Modifier>,
        key: Key,
    },
    Mouse(MouseButton),
}

impl FromStr for Action {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim().to_lowercase();
        if s.is_empty() {
            return Err("empty action".into());
        }

        if let Some(btn) = MouseButton::from_str_lower(&s) {
            return Ok(Action::Mouse(btn));
        }

        let parts: Vec<&str> = s.split('+').map(|p| p.trim()).collect();
        if parts.is_empty() {
            return Err("invalid key combination".into());
        }

        let mut modifiers = Vec::new();
        for part in &parts[..parts.len() - 1] {
            let m = Modifier::from_str_lower(part)
                .ok_or_else(|| format!("unknown modifier: {}", part))?;
            modifiers.push(m);
        }

        let key_str = parts[parts.len() - 1];
        let key = Key::from_str_lower(key_str)
            .ok_or_else(|| format!("unknown key: {}", key_str))?;

        Ok(Action::Keys { modifiers, key })
    }
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Action::Keys { modifiers, key } => {
                let mut parts: Vec<&str> = modifiers.iter().map(|m| m.canonical_name()).collect();
                parts.push(key.canonical_name());
                write!(f, "{}", parts.join("+"))
            }
            Action::Mouse(btn) => write!(f, "{}", btn.canonical_name()),
        }
    }
}

impl Serialize for Action {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Action {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_single_keys() {
        assert_eq!("enter".parse::<Action>().unwrap(), Action::Keys { modifiers: vec![], key: Key::Enter });
        assert_eq!("a".parse::<Action>().unwrap(), Action::Keys { modifiers: vec![], key: Key::A });
        assert_eq!("f12".parse::<Action>().unwrap(), Action::Keys { modifiers: vec![], key: Key::F12 });
        assert_eq!("esc".parse::<Action>().unwrap(), Action::Keys { modifiers: vec![], key: Key::Escape });
    }

    #[test]
    fn test_parse_combinations() {
        assert_eq!(
            "ctrl+shift+t".parse::<Action>().unwrap(),
            Action::Keys {
                modifiers: vec![Modifier::Ctrl, Modifier::Shift],
                key: Key::T
            }
        );
        assert_eq!(
            "Ctrl + Shift + t".parse::<Action>().unwrap(),
            Action::Keys {
                modifiers: vec![Modifier::Ctrl, Modifier::Shift],
                key: Key::T
            }
        );
        assert_eq!(
            "cmd+alt+delete".parse::<Action>().unwrap(),
            Action::Keys {
                modifiers: vec![Modifier::Meta, Modifier::Alt],
                key: Key::Delete
            }
        );
    }

    #[test]
    fn test_parse_mouse_buttons() {
        assert_eq!("mouse_left".parse::<Action>().unwrap(), Action::Mouse(MouseButton::Left));
        assert_eq!("mouse_back".parse::<Action>().unwrap(), Action::Mouse(MouseButton::Back));
        assert_eq!("btn_side".parse::<Action>().unwrap(), Action::Mouse(MouseButton::Back));
        assert_eq!("forward".parse::<Action>().unwrap(), Action::Mouse(MouseButton::Forward));
        assert_eq!("back".parse::<Action>().unwrap(), Action::Mouse(MouseButton::Back));
    }

    #[test]
    fn test_display_and_serde() {
        let action = Action::Keys {
            modifiers: vec![Modifier::Ctrl, Modifier::Shift],
            key: Key::T,
        };
        assert_eq!(action.to_string(), "ctrl+shift+t");

        let serialized = serde_json::to_string(&action).unwrap();
        assert_eq!(serialized, "\"ctrl+shift+t\"");

        let deserialized: Action = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized, action);
    }

    #[test]
    fn test_parse_errors() {
        assert!(""          .parse::<Action>().is_err());
        assert!("  "        .parse::<Action>().is_err());
        assert!("unknown_xyz".parse::<Action>().is_err());
        assert!("ctrl+unknown".parse::<Action>().is_err());
        assert!("fakemeta+a" .parse::<Action>().is_err());
        assert!("mouse_nope" .parse::<Action>().is_err());
    }

    #[test]
    fn test_round_trip_keys() {
        let keys = ["a", "z", "0", "9", "f1", "f12", "enter", "space",
                     "tab", "backspace", "escape", "delete", "insert",
                     "home", "end", "pageup", "pagedown",
                     "left", "right", "up", "down"];
        for k in keys {
            let action: Action = k.parse().unwrap();
            let reparsed: Action = action.to_string().parse().unwrap();
            assert_eq!(action, reparsed, "round-trip failed for key: {}", k);
        }
    }

    #[test]
    fn test_round_trip_mouse() {
        let buttons = ["mouse_left", "mouse_right", "mouse_middle", "mouse_back", "mouse_forward"];
        for b in buttons {
            let action: Action = b.parse().unwrap();
            let reparsed: Action = action.to_string().parse().unwrap();
            assert_eq!(action, reparsed, "round-trip failed for button: {}", b);
        }
    }

    #[test]
    fn test_round_trip_combos() {
        let combos = ["ctrl+a", "shift+f5", "alt+tab", "meta+space", "ctrl+shift+delete"];
        for c in combos {
            let action: Action = c.parse().unwrap();
            let reparsed: Action = action.to_string().parse().unwrap();
            assert_eq!(action, reparsed, "round-trip failed for combo: {}", c);
        }
    }
}
