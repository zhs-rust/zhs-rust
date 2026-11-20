pub mod buffer;
pub mod geometry;
pub mod style;
pub mod terminal;
pub mod widgets;

pub use buffer::{Buffer, Cell};
pub use geometry::{Constraint, Direction, Rect, split};
pub use style::{Color, Style};
pub use terminal::{Event, Frame, Key, Terminal};
pub use widgets::{Block, Gauge, List, Paragraph, Sparkline, Table, Widget};

use std::io;

pub type Result<T> = std::result::Result<T, io::Error>;
