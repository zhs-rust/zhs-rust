use crate::buffer::Buffer;
use crate::geometry::Rect;
use crate::style::{Color, Style};

pub trait Widget {
    fn render(self, area: Rect, buffer: &mut Buffer);
}

#[derive(Debug, Clone, Default)]
pub struct Block {
    title: Option<String>,
    border_style: Style,
    title_style: Style,
}

impl Block {
    pub fn new() -> Self {
        Self {
            title: None,
            border_style: Style::new().fg(Color::DarkGray),
            title_style: Style::new().bold(),
        }
    }

    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn border_style(mut self, style: Style) -> Self {
        self.border_style = style;
        self
    }

    pub fn title_style(mut self, style: Style) -> Self {
        self.title_style = style;
        self
    }

    pub fn inner(&self, area: Rect) -> Rect {
        if area.width < 2 || area.height < 2 {
            return area;
        }
        Rect::new(area.x + 1, area.y + 1, area.width - 2, area.height - 2)
    }
}

impl Widget for Block {
    fn render(self, area: Rect, buffer: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let horizontal = if area.width >= 2 {
            "─".repeat((area.width - 2) as usize)
        } else {
            String::new()
        };

        buffer.set_string(
            area.x,
            area.y,
            &format!("┌{horizontal}┐"),
            self.border_style,
        );

        for y in area.y + 1..area.bottom().saturating_sub(1) {
            buffer.set_string(area.x, y, "│", self.border_style);
            if area.width >= 2 {
                buffer.set_string(area.right() - 1, y, "│", self.border_style);
            }
        }

        if area.height >= 2 {
            buffer.set_string(
                area.x,
                area.bottom() - 1,
                &format!("└{horizontal}┘"),
                self.border_style,
            );
        }

        if let Some(title) = self.title {
            let label = format!(" {title} ");
            let width = label.chars().count() as u16;
            if width + 2 <= area.width {
                buffer.set_string(area.x + 2, area.y, &label, self.title_style);
            }
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Paragraph {
    lines: Vec<String>,
    style: Style,
}

impl Paragraph {
    pub fn new(lines: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            lines: lines.into_iter().map(Into::into).collect(),
            style: Style::new(),
        }
    }

    pub fn style(mut self, style: Style) -> Self {
        self.style = style;
        self
    }
}

impl Widget for Paragraph {
    fn render(self, area: Rect, buffer: &mut Buffer) {
        for (offset, line) in self.lines.iter().take(area.height as usize).enumerate() {
            buffer.set_string(area.x, area.y + offset as u16, line, self.style);
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct List {
    items: Vec<String>,
    selected: Option<usize>,
    style: Style,
    selected_style: Style,
}

impl List {
    pub fn new(items: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            items: items.into_iter().map(Into::into).collect(),
            selected: None,
            style: Style::new(),
            selected_style: Style::new().fg(Color::Black).bg(Color::LightCyan).bold(),
        }
    }

    pub fn selected(mut self, index: usize) -> Self {
        self.selected = Some(index);
        self
    }

    pub fn style(mut self, style: Style) -> Self {
        self.style = style;
        self
    }

    pub fn selected_style(mut self, style: Style) -> Self {
        self.selected_style = style;
        self
    }
}

impl Widget for List {
    fn render(self, area: Rect, buffer: &mut Buffer) {
        for (index, item) in self.items.iter().take(area.height as usize).enumerate() {
            let style = if Some(index) == self.selected {
                self.selected_style
            } else {
                self.style
            };
            let prefix = if Some(index) == self.selected {
                "> "
            } else {
                "  "
            };
            let text = format!("{prefix}{item}");
            buffer.set_string(area.x, area.y + index as u16, &text, style);

            if Some(index) == self.selected {
                for x in area.x..area.right() {
                    let mut cell = buffer_cell(buffer, x, area.y + index as u16);
                    cell.style = style;
                    buffer.set_cell(x, area.y + index as u16, cell);
                }
            }
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Table {
    headers: Vec<String>,
    rows: Vec<Vec<String>>,
    header_style: Style,
    style: Style,
}

impl Table {
    pub fn new(headers: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            headers: headers.into_iter().map(Into::into).collect(),
            rows: Vec::new(),
            header_style: Style::new().bold().fg(Color::Cyan),
            style: Style::new(),
        }
    }

    pub fn row(mut self, row: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.rows.push(row.into_iter().map(Into::into).collect());
        self
    }

    pub fn style(mut self, style: Style) -> Self {
        self.style = style;
        self
    }
}

impl Widget for Table {
    fn render(self, area: Rect, buffer: &mut Buffer) {
        let columns = self
            .headers
            .len()
            .max(self.rows.iter().map(Vec::len).max().unwrap_or(0));

        if columns == 0 || area.width == 0 {
            return;
        }

        let column_width = (area.width / columns as u16).max(1);
        let mut widths = vec![column_width; columns];
        if let Some(last) = widths.last_mut() {
            *last = area
                .width
                .saturating_sub(column_width * (columns as u16 - 1));
        }

        let mut y = area.y;
        if y < area.bottom() {
            for (column, header) in self.headers.iter().take(columns).enumerate() {
                let x = area.x + widths[..column].iter().sum::<u16>();
                let text = pad(header, widths[column]);
                buffer.set_string(x, y, &text, self.header_style);
            }
            y += 1;
        }

        if y < area.bottom() {
            let separator = "─".repeat(area.width as usize);
            buffer.set_string(area.x, y, &separator, Style::new().fg(Color::DarkGray));
            y += 1;
        }

        for row in self
            .rows
            .iter()
            .take(area.bottom().saturating_sub(y) as usize)
        {
            for (column, value) in row.iter().take(columns).enumerate() {
                let x = area.x + widths[..column].iter().sum::<u16>();
                let text = pad(value, widths[column]);
                buffer.set_string(x, y, &text, self.style);
            }
            y += 1;
        }
    }
}

#[derive(Debug, Clone)]
pub struct Gauge {
    ratio: f64,
    label: Option<String>,
    style: Style,
    filled_style: Style,
}

impl Gauge {
    pub fn new(ratio: f64) -> Self {
        Self {
            ratio: ratio.clamp(0.0, 1.0),
            label: None,
            style: Style::new().fg(Color::DarkGray),
            filled_style: Style::new().fg(Color::Green).bold(),
        }
    }

    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }
}

impl Widget for Gauge {
    fn render(self, area: Rect, buffer: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let label = self
            .label
            .unwrap_or_else(|| format!("{:.0}%", self.ratio * 100.0));
        let width = area.width as usize;
        let filled = ((width as f64 * self.ratio).round() as usize).min(width);
        let bar = format!(
            "{}{} {label}",
            "█".repeat(filled),
            "░".repeat(width.saturating_sub(filled))
        );
        buffer.set_string(area.x, area.y, &bar, self.filled_style);

        for x in area.x..area.right() {
            let mut cell = buffer_cell(buffer, x, area.y);
            let offset = (x - area.x) as usize;
            cell.style = if offset < filled {
                self.filled_style
            } else {
                self.style
            };
            buffer.set_cell(x, area.y, cell);
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Sparkline {
    data: Vec<f64>,
    style: Style,
}

impl Sparkline {
    pub fn new(data: impl IntoIterator<Item = f64>) -> Self {
        Self {
            data: data.into_iter().collect(),
            style: Style::new().fg(Color::LightGreen),
        }
    }
}

impl Widget for Sparkline {
    fn render(self, area: Rect, buffer: &mut Buffer) {
        if area.width == 0 || area.height == 0 || self.data.is_empty() {
            return;
        }

        const BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
        let maximum = self.data.iter().copied().fold(f64::MIN, f64::max).max(0.0);
        let minimum = self.data.iter().copied().fold(f64::MAX, f64::min).min(0.0);
        let range = (maximum - minimum).max(f64::EPSILON);

        for (index, value) in self.data.iter().take(area.width as usize).enumerate() {
            let level = (((value - minimum) / range) * (BARS.len() - 1) as f64).round() as usize;
            buffer.set_string(
                area.x + index as u16,
                area.y,
                &BARS[level.min(BARS.len() - 1)].to_string(),
                self.style,
            );
        }
    }
}

fn pad(value: &str, width: u16) -> String {
    let width = width as usize;
    let length = value.chars().count();
    if length >= width {
        value.chars().take(width).collect()
    } else {
        format!("{value}{}", " ".repeat(width - length))
    }
}

fn buffer_cell(buffer: &Buffer, x: u16, y: u16) -> crate::buffer::Cell {
    let area = buffer.area();
    if x < area.x || y < area.y || x >= area.right() || y >= area.bottom() {
        return Default::default();
    }
    let index = (y - area.y) as usize * area.width as usize + (x - area.x) as usize;
    buffer.cell(index).copied().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_renders_borders() {
        let mut buffer = Buffer::new(Rect::new(0, 0, 4, 2));
        Block::new()
            .title("x")
            .render(Rect::new(0, 0, 4, 2), &mut buffer);

        assert_eq!(buffer.cell(0).unwrap().symbol, '┌');
        assert_eq!(buffer.cell(1).unwrap().symbol, '─');
    }
}
