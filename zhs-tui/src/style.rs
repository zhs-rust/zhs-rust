#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Color {
    #[default]
    Reset,
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    Gray,
    DarkGray,
    LightRed,
    LightGreen,
    LightYellow,
    LightBlue,
    LightMagenta,
    LightCyan,
    White,
    Rgb(u8, u8, u8),
}

impl Color {
    pub fn foreground_code(self) -> String {
        match self {
            Color::Reset => "39".to_owned(),
            Color::Black => "30".to_owned(),
            Color::Red => "31".to_owned(),
            Color::Green => "32".to_owned(),
            Color::Yellow => "33".to_owned(),
            Color::Blue => "34".to_owned(),
            Color::Magenta => "35".to_owned(),
            Color::Cyan => "36".to_owned(),
            Color::Gray => "37".to_owned(),
            Color::DarkGray => "90".to_owned(),
            Color::LightRed => "91".to_owned(),
            Color::LightGreen => "92".to_owned(),
            Color::LightYellow => "93".to_owned(),
            Color::LightBlue => "94".to_owned(),
            Color::LightMagenta => "95".to_owned(),
            Color::LightCyan => "96".to_owned(),
            Color::White => "97".to_owned(),
            Color::Rgb(red, green, blue) => format!("38;2;{red};{green};{blue}"),
        }
    }

    pub fn background_code(self) -> String {
        match self {
            Color::Reset => "49".to_owned(),
            Color::Black => "40".to_owned(),
            Color::Red => "41".to_owned(),
            Color::Green => "42".to_owned(),
            Color::Yellow => "43".to_owned(),
            Color::Blue => "44".to_owned(),
            Color::Magenta => "45".to_owned(),
            Color::Cyan => "46".to_owned(),
            Color::Gray => "47".to_owned(),
            Color::DarkGray => "100".to_owned(),
            Color::LightRed => "101".to_owned(),
            Color::LightGreen => "102".to_owned(),
            Color::LightYellow => "103".to_owned(),
            Color::LightBlue => "104".to_owned(),
            Color::LightMagenta => "105".to_owned(),
            Color::LightCyan => "106".to_owned(),
            Color::White => "107".to_owned(),
            Color::Rgb(red, green, blue) => format!("48;2;{red};{green};{blue}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Style {
    pub fg: Color,
    pub bg: Color,
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
}

impl Style {
    pub const fn new() -> Self {
        Self {
            fg: Color::Reset,
            bg: Color::Reset,
            bold: false,
            dim: false,
            italic: false,
            underline: false,
        }
    }

    pub const fn fg(mut self, color: Color) -> Self {
        self.fg = color;
        self
    }

    pub const fn bg(mut self, color: Color) -> Self {
        self.bg = color;
        self
    }

    pub const fn bold(mut self) -> Self {
        self.bold = true;
        self
    }

    pub const fn dim(mut self) -> Self {
        self.dim = true;
        self
    }

    pub const fn italic(mut self) -> Self {
        self.italic = true;
        self
    }

    pub const fn underline(mut self) -> Self {
        self.underline = true;
        self
    }

    pub fn ansi_prefix(self) -> String {
        let mut codes = Vec::new();

        if self.bold {
            codes.push("1".to_owned());
        }
        if self.dim {
            codes.push("2".to_owned());
        }
        if self.italic {
            codes.push("3".to_owned());
        }
        if self.underline {
            codes.push("4".to_owned());
        }
        if self.fg != Color::Reset {
            codes.push(self.fg.foreground_code());
        }
        if self.bg != Color::Reset {
            codes.push(self.bg.background_code());
        }

        if codes.is_empty() {
            String::new()
        } else {
            format!("\x1b[{}m", codes.join(";"))
        }
    }
}
