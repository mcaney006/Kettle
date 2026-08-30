use gpui::{Window, WindowAppearance};

#[derive(Clone, Copy)]
pub struct Theme {
    pub background: u32,
    pub chrome: u32,
    pub sidebar: u32,
    pub line: u32,
    pub text: u32,
    pub muted: u32,
    pub faint: u32,
    pub selection: u32,
    pub stale: u32,
    pub failure: u32,
}

impl Theme {
    pub fn for_window(window: &Window) -> Self {
        match window.appearance() {
            WindowAppearance::Light | WindowAppearance::VibrantLight => Self {
                background: 0xF4F4F4,
                chrome: 0xE9E9E9,
                sidebar: 0xEEEEEE,
                line: 0xD0D0D0,
                text: 0x202020,
                muted: 0x606060,
                faint: 0x8A8A8A,
                selection: 0x3478D4,
                stale: 0xA85A00,
                failure: 0xB42318,
            },
            WindowAppearance::Dark | WindowAppearance::VibrantDark => Self {
                background: 0x1E1E1E,
                chrome: 0x2A2A2A,
                sidebar: 0x252525,
                line: 0x383838,
                text: 0xE4E4E4,
                muted: 0x9A9A9A,
                faint: 0x686868,
                selection: 0x2C5FA8,
                stale: 0xD9903A,
                failure: 0xD4726A,
            },
        }
    }
}
