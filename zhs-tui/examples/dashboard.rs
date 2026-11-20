use std::io;

use zhs_tui::{
    Block, Constraint, Direction, Event, Frame, Gauge, Key, List, Paragraph, Rect, Sparkline,
    Style, Table, Terminal, Widget, split,
};

fn main() -> io::Result<()> {
    let mut terminal = Terminal::new()?;
    let mut selected = 0usize;
    let items = ["Overview", "Routes", "Health", "Logs"];
    let cpu = [3.0, 5.0, 4.0, 8.0, 6.0, 9.0, 7.0, 11.0, 8.0, 6.0];

    loop {
        terminal.draw(|frame| draw(frame, selected, &items, &cpu))?;

        match terminal.read_event()? {
            Event::Key(Key::Char('q')) | Event::Key(Key::CtrlC) => break,
            Event::Key(Key::Char('j')) | Event::Key(Key::Down) => {
                selected = (selected + 1).min(items.len() - 1);
            }
            Event::Key(Key::Char('k')) | Event::Key(Key::Up) => {
                selected = selected.saturating_sub(1);
            }
            _ => {}
        }
    }

    Ok(())
}

fn draw(frame: &mut Frame, selected: usize, items: &[&str], cpu: &[f64]) {
    let area = frame.area();
    let columns = split(
        area,
        Direction::Horizontal,
        &[Constraint::Length(22), Constraint::Min(0)],
    );

    let sidebar = columns[0];
    let main = columns[1];
    let rows = split(
        main,
        Direction::Vertical,
        &[
            Constraint::Length(9),
            Constraint::Length(8),
            Constraint::Min(0),
        ],
    );

    frame.render(
        Block::new()
            .title("zhs")
            .border_style(Style::new().fg(zhs_tui::Color::Cyan)),
        sidebar,
    );
    frame.render(
        List::new(items.iter().copied()).selected(selected),
        sidebar.inner(1),
    );

    frame.render(Block::new().title("dashboard"), rows[0]);
    frame.render(
        Paragraph::new([
            format!("selected: {}", items[selected]),
            "j/k or arrows: move".to_owned(),
            "q: quit".to_owned(),
        ]),
        rows[0].inner(2),
    );

    frame.render(Block::new().title("CPU"), rows[1]);
    frame.render(
        Gauge::new(0.42).label("42%"),
        Rect::new(rows[1].x + 2, rows[1].y + 2, 40, 1),
    );
    frame.render(
        Sparkline::new(cpu.iter().copied()),
        Rect::new(rows[1].x + 2, rows[1].y + 4, 40, 1),
    );

    let table = Table::new(["route", "method", "status"])
        .row(["/api/hello", "GET", "200"])
        .row(["/health", "GET", "200"])
        .row(["/swagger", "GET", "200"]);
    frame.render(Block::new().title("routes"), rows[2]);
    table.render(rows[2].inner(1), frame.buffer());
}
