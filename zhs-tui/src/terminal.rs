use std::io::{self, Write};

use crate::buffer::Buffer;
use crate::geometry::Rect;
use crate::widgets::Widget;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Enter,
    Esc,
    Tab,
    Backspace,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    Delete,
    CtrlC,
    F(u8),
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Key(Key),
    Resize,
}

pub struct Frame {
    area: Rect,
    buffer: Buffer,
}

impl Frame {
    pub fn area(&self) -> Rect {
        self.area
    }

    pub fn buffer(&mut self) -> &mut Buffer {
        &mut self.buffer
    }

    pub fn render<W: Widget>(&mut self, widget: W, area: Rect) {
        widget.render(area, &mut self.buffer);
    }
}

pub struct Terminal {
    raw: platform::RawMode,
    area: Rect,
}

impl Terminal {
    pub fn new() -> io::Result<Self> {
        let raw = platform::RawMode::enable()?;
        let mut stdout = io::stdout();
        stdout.write_all(b"\x1b[?1049h\x1b[?25l\x1b[2J\x1b[H")?;
        stdout.flush()?;

        let area = platform::size().unwrap_or(Rect::new(0, 0, 80, 24));
        Ok(Self { raw, area })
    }

    pub fn size(&self) -> Rect {
        self.area
    }

    pub fn draw<F>(&mut self, draw: F) -> io::Result<()>
    where
        F: FnOnce(&mut Frame),
    {
        self.area = platform::size().unwrap_or(self.area);
        let mut frame = Frame {
            area: self.area,
            buffer: Buffer::new(self.area),
        };
        draw(&mut frame);

        let mut stdout = io::stdout();
        stdout.write_all(frame.buffer.render().as_bytes())?;
        stdout.flush()
    }

    pub fn read_event(&mut self) -> io::Result<Event> {
        let key = platform::read_key(&self.raw)?;
        Ok(Event::Key(key))
    }

    pub fn clear(&mut self) -> io::Result<()> {
        let mut stdout = io::stdout();
        stdout.write_all(b"\x1b[2J\x1b[H")?;
        stdout.flush()
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let mut stdout = io::stdout();
        let _ = stdout.write_all(b"\x1b[0m\x1b[?25h\x1b[?1049l");
        let _ = stdout.flush();
        let _ = self.raw.restore();
    }
}

#[cfg(windows)]
mod platform {
    use std::ffi::c_void;
    use std::io;

    use crate::geometry::Rect;
    use crate::terminal::Key;

    type Handle = *mut c_void;

    const STD_INPUT_HANDLE: u32 = 0xFFFF_FFF6;
    const STD_OUTPUT_HANDLE: u32 = 0xFFFF_FFF5;
    const ENABLE_PROCESSED_INPUT: u32 = 0x0001;
    const ENABLE_LINE_INPUT: u32 = 0x0002;
    const ENABLE_ECHO_INPUT: u32 = 0x0004;
    const ENABLE_VIRTUAL_TERMINAL_PROCESSING: u32 = 0x0004;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Coord {
        x: i16,
        y: i16,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct SmallRect {
        left: i16,
        top: i16,
        right: i16,
        bottom: i16,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct ConsoleScreenBufferInfo {
        size: Coord,
        cursor_position: Coord,
        attributes: u16,
        window: SmallRect,
        maximum_window_size: Coord,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct KeyEventRecord {
        key_down: i32,
        repeat_count: u16,
        virtual_key_code: u16,
        virtual_scan_code: u16,
        unicode_char: u16,
        control_key_state: u32,
    }

    #[repr(C)]
    union EventRecord {
        key: KeyEventRecord,
        padding: [u8; 16],
    }

    #[repr(C)]
    struct InputRecord {
        event_type: u16,
        _padding: u16,
        event: EventRecord,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetStdHandle(handle: u32) -> Handle;
        fn GetConsoleMode(handle: Handle, mode: *mut u32) -> i32;
        fn SetConsoleMode(handle: Handle, mode: u32) -> i32;
        fn GetConsoleScreenBufferInfo(handle: Handle, info: *mut ConsoleScreenBufferInfo) -> i32;
        fn ReadConsoleInputW(
            handle: Handle,
            records: *mut InputRecord,
            length: u32,
            read: *mut u32,
        ) -> i32;
    }

    pub struct RawMode {
        stdin: Handle,
        stdout: Handle,
        original_input: u32,
        original_output: u32,
    }

    impl RawMode {
        pub fn enable() -> io::Result<Self> {
            unsafe {
                let stdin = GetStdHandle(STD_INPUT_HANDLE);
                let stdout = GetStdHandle(STD_OUTPUT_HANDLE);
                let mut input_mode = 0u32;
                let mut output_mode = 0u32;

                if GetConsoleMode(stdin, &mut input_mode) == 0
                    || GetConsoleMode(stdout, &mut output_mode) == 0
                {
                    return Err(io::Error::last_os_error());
                }

                let raw =
                    input_mode & !(ENABLE_LINE_INPUT | ENABLE_ECHO_INPUT | ENABLE_PROCESSED_INPUT);
                if SetConsoleMode(stdin, raw) == 0 {
                    return Err(io::Error::last_os_error());
                }
                if SetConsoleMode(stdout, output_mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING) == 0 {
                    let _ = SetConsoleMode(stdin, input_mode);
                    return Err(io::Error::last_os_error());
                }

                Ok(Self {
                    stdin,
                    stdout,
                    original_input: input_mode,
                    original_output: output_mode,
                })
            }
        }

        pub fn restore(&self) -> io::Result<()> {
            unsafe {
                if SetConsoleMode(self.stdin, self.original_input) == 0
                    || SetConsoleMode(self.stdout, self.original_output) == 0
                {
                    return Err(io::Error::last_os_error());
                }
            }
            Ok(())
        }
    }

    pub fn size() -> Option<Rect> {
        unsafe {
            let mut info = ConsoleScreenBufferInfo {
                size: Coord { x: 0, y: 0 },
                cursor_position: Coord { x: 0, y: 0 },
                attributes: 0,
                window: SmallRect {
                    left: 0,
                    top: 0,
                    right: 0,
                    bottom: 0,
                },
                maximum_window_size: Coord { x: 0, y: 0 },
            };

            if GetConsoleScreenBufferInfo(GetStdHandle(STD_OUTPUT_HANDLE), &mut info) == 0 {
                return None;
            }

            let width = (info.window.right - info.window.left + 1).max(1) as u16;
            let height = (info.window.bottom - info.window.top + 1).max(1) as u16;
            Some(Rect::new(0, 0, width, height))
        }
    }

    pub fn read_key(_raw: &RawMode) -> io::Result<Key> {
        let mut record = InputRecord {
            event_type: 0,
            _padding: 0,
            event: EventRecord {
                key: KeyEventRecord {
                    key_down: 0,
                    repeat_count: 0,
                    virtual_key_code: 0,
                    virtual_scan_code: 0,
                    unicode_char: 0,
                    control_key_state: 0,
                },
            },
        };
        let mut read = 0u32;

        loop {
            unsafe {
                if ReadConsoleInputW(GetStdHandle(STD_INPUT_HANDLE), &mut record, 1, &mut read) == 0
                {
                    return Err(io::Error::last_os_error());
                }
            }

            if record.event_type != 0x0001 || read == 0 {
                continue;
            }

            let key = unsafe { record.event.key };
            if key.key_down == 0 {
                continue;
            }

            return Ok(map_key(&key));
        }
    }

    fn map_key(key: &KeyEventRecord) -> Key {
        const VK_BACK: u16 = 0x08;
        const VK_TAB: u16 = 0x09;
        const VK_RETURN: u16 = 0x0D;
        const VK_ESCAPE: u16 = 0x1B;
        const VK_END: u16 = 0x23;
        const VK_HOME: u16 = 0x24;
        const VK_LEFT: u16 = 0x25;
        const VK_UP: u16 = 0x26;
        const VK_RIGHT: u16 = 0x27;
        const VK_DOWN: u16 = 0x28;
        const VK_DELETE: u16 = 0x2E;

        if key.control_key_state & 0x0008 != 0 && key.unicode_char == 3 {
            return Key::CtrlC;
        }

        match key.virtual_key_code {
            VK_BACK => Key::Backspace,
            VK_TAB => Key::Tab,
            VK_RETURN => Key::Enter,
            VK_ESCAPE => Key::Esc,
            VK_HOME => Key::Home,
            VK_END => Key::End,
            VK_LEFT => Key::Left,
            VK_UP => Key::Up,
            VK_RIGHT => Key::Right,
            VK_DOWN => Key::Down,
            VK_DELETE => Key::Delete,
            0x70..=0x7B => Key::F((key.virtual_key_code - 0x6F) as u8),
            _ => {
                let character = char::from_u32(key.unicode_char as u32).unwrap_or('\0');
                if key.unicode_char == 3 {
                    Key::CtrlC
                } else if character != '\0' {
                    Key::Char(character)
                } else {
                    Key::Unknown
                }
            }
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use std::io::{self, Read};
    use std::process::Command;

    use crate::geometry::Rect;
    use crate::terminal::Key;

    pub struct RawMode;

    impl RawMode {
        pub fn enable() -> io::Result<Self> {
            let _ = Command::new("stty").args(["raw", "-echo"]).status();
            Ok(Self)
        }

        pub fn restore(&self) -> io::Result<()> {
            let _ = Command::new("stty").arg("sane").status();
            Ok(())
        }
    }

    pub fn size() -> Option<Rect> {
        /// Upper bound for terminal dimensions taken from the environment,
        /// so bogus COLUMNS/LINES values cannot request absurd buffers.
        const MAX_DIMENSION: u16 = 1000;

        let width = std::env::var("COLUMNS")
            .ok()
            .and_then(|value| value.parse::<u16>().ok())
            .map(|value| value.clamp(1, MAX_DIMENSION))
            .unwrap_or(80);
        let height = std::env::var("LINES")
            .ok()
            .and_then(|value| value.parse::<u16>().ok())
            .map(|value| value.clamp(1, MAX_DIMENSION))
            .unwrap_or(24);
        Some(Rect::new(0, 0, width, height))
    }

    pub fn read_key(_raw: &RawMode) -> io::Result<Key> {
        let mut buffer = [0u8; 1];
        io::stdin().read_exact(&mut buffer)?;
        let byte = buffer[0];

        Ok(match byte {
            0x03 => Key::CtrlC,
            b'\r' | b'\n' => Key::Enter,
            b'\t' => Key::Tab,
            0x7F | 0x08 => Key::Backspace,
            0x1B => {
                let mut sequence = [0u8; 2];
                if io::stdin().read_exact(&mut sequence).is_err() {
                    return Ok(Key::Esc);
                }
                match sequence {
                    [b'[', b'A'] => Key::Up,
                    [b'[', b'B'] => Key::Down,
                    [b'[', b'C'] => Key::Right,
                    [b'[', b'D'] => Key::Left,
                    [b'[', b'H'] => Key::Home,
                    [b'[', b'F'] => Key::End,
                    [b'[', b'3'] => {
                        // Consume the trailing '~' of the delete sequence, so it
                        // does not surface as a phantom '~' key press.
                        let mut tail = [0u8; 1];
                        let _ = io::stdin().read_exact(&mut tail);
                        Key::Delete
                    }
                    _ => Key::Esc,
                }
            }
            byte if byte.is_ascii_graphic() || byte == b' ' => Key::Char(byte as char),
            _ => Key::Unknown,
        })
    }
}
