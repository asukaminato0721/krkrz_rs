//! Kirikiri pad key codes. The aggregate code is a query, never an input event.
pub const ANY_KEY_CODE: u32 = 0x1df;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(u32)]
pub enum PadKey {
    Left = 0x1b5,
    Up = 0x1b6,
    Right = 0x1b7,
    Down = 0x1b8,
    Button1 = 0x1c0,
    Button2,
    Button3,
    Button4,
    Button5,
    Button6,
    Button7,
    Button8,
    Button9,
    Button10,
}

impl PadKey {
    pub fn from_key_code(code: u32) -> Option<Self> {
        use PadKey::*;
        Some(match code {
            0x1b5 => Left,
            0x1b6 => Up,
            0x1b7 => Right,
            0x1b8 => Down,
            0x1c0 => Button1,
            0x1c1 => Button2,
            0x1c2 => Button3,
            0x1c3 => Button4,
            0x1c4 => Button5,
            0x1c5 => Button6,
            0x1c6 => Button7,
            0x1c7 => Button8,
            0x1c8 => Button9,
            0x1c9 => Button10,
            _ => return None,
        })
    }

    pub const fn key_code(self) -> u32 {
        self as u32
    }
}
