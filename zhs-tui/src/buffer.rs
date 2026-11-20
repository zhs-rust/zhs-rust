use std::fmt::Write;

use crate::geometry::Rect;
use crate::style::Style;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub symbol: char,
    pub style: Style,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            symbol: ' ',
            style: Style::new(),
        }
    }
}

/// Hard cap on the number of cells a single buffer may allocate. Requests
/// above this (or products that would overflow `usize` on 32-bit targets)
/// are clamped by reducing the buffer height.
const MAX_CELLS: usize = 1_000_000;

#[derive(Debug, Clone)]
pub struct Buffer {
    area: Rect,
    cells: Vec<Cell>,
}

impl Buffer {
    pub fn new(area: Rect) -> Self {
        let area = clamp_area(area);
        Self {
            area,
            cells: vec![Cell::default(); area.width as usize * area.height as usize],
        }
    }

    pub fn area(&self) -> Rect {
        self.area
    }

    pub fn cell(&self, index: usize) -> Option<&Cell> {
        self.cells.get(index)
    }

    pub fn clear(&mut self) {
        self.cells.fill(Cell::default());
    }

    pub fn set_cell(&mut self, x: u16, y: u16, cell: Cell) {
        if x < self.area.x || y < self.area.y || x >= self.area.right() || y >= self.area.bottom() {
            return;
        }

        let index =
            (y - self.area.y) as usize * self.area.width as usize + (x - self.area.x) as usize;
        if let Some(slot) = self.cells.get_mut(index) {
            *slot = cell;
        }
    }

    pub fn set_string(&mut self, x: u16, y: u16, text: &str, style: Style) -> u16 {
        let mut cursor = x;
        for symbol in text.chars() {
            if cursor >= self.area.right() {
                break;
            }
            self.set_cell(
                cursor,
                y,
                Cell {
                    symbol: sanitize_symbol(symbol),
                    style,
                },
            );
            cursor = cursor.saturating_add(1);
        }
        cursor
    }

    pub fn render(&self) -> String {
        let mut output =
            String::with_capacity(self.cells.len() * 2 + self.area.height as usize * 10 + 16);
        output.push_str("\x1b[H");

        let mut current_style = Style::new();

        for y in 0..self.area.height {
            for x in 0..self.area.width {
                let cell = self.cells[y as usize * self.area.width as usize + x as usize];
                if cell.style != current_style {
                    output.push_str("\x1b[0m");
                    output.push_str(&cell.style.ansi_prefix());
                    current_style = cell.style;
                }
                output.push(sanitize_symbol(cell.symbol));
            }
            if y + 1 < self.area.height {
                output.push_str("\r\n");
            }
        }

        output.push_str("\x1b[0m");
        let _ = write!(output, "\x1b[{};{}H", self.area.height, 1);
        output
    }
}

/// Clamps `area` so that `width * height` cannot overflow `usize` and never
/// exceeds [`MAX_CELLS`] cells; the height is reduced to fit when needed.
fn clamp_area(area: Rect) -> Rect {
    let cells = (area.width as usize).checked_mul(area.height as usize);
    match cells {
        Some(cells) if cells <= MAX_CELLS => area,
        _ => {
            let width = area.width as usize;
            let height = MAX_CELLS
                .checked_div(width)
                .unwrap_or(area.height as usize)
                .min(area.height as usize);
            Rect::new(area.x, area.y, area.width, height as u16)
        }
    }
}

fn sanitize_symbol(symbol: char) -> char {
    if symbol.is_control() {
        '\u{fffd}'
    } else {
        symbol
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::Color;

    #[test]
    fn writes_text_into_bounds() {
        let mut buffer = Buffer::new(Rect::new(0, 0, 5, 1));
        buffer.set_string(0, 0, "hello world", Style::new().fg(Color::Green));

        assert_eq!(buffer.cells[0].symbol, 'h');
        assert_eq!(buffer.cells[4].symbol, 'o');
    }

    #[test]
    fn sanitizes_control_characters() {
        let mut buffer = Buffer::new(Rect::new(0, 0, 5, 1));
        buffer.set_string(0, 0, "a\x1bb\nc", Style::new());

        assert_eq!(buffer.cells[0].symbol, 'a');
        assert_eq!(buffer.cells[1].symbol, '\u{fffd}');
        assert_eq!(buffer.cells[2].symbol, 'b');
        assert_eq!(buffer.cells[3].symbol, '\u{fffd}');
        assert_eq!(buffer.cells[4].symbol, 'c');
    }
}
