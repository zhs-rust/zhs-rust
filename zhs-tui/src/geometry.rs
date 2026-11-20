#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

impl Rect {
    pub const fn new(x: u16, y: u16, width: u16, height: u16) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }

    pub fn inner(self, margin: u16) -> Self {
        let width = self.width.saturating_sub(margin * 2);
        let height = self.height.saturating_sub(margin * 2);
        Self::new(self.x + margin, self.y + margin, width, height)
    }

    pub fn right(self) -> u16 {
        self.x.saturating_add(self.width)
    }

    pub fn bottom(self) -> u16 {
        self.y.saturating_add(self.height)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Constraint {
    Length(u16),
    Percentage(u16),
    Min(u16),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Horizontal,
    Vertical,
}

pub fn split(area: Rect, direction: Direction, constraints: &[Constraint]) -> Vec<Rect> {
    if constraints.is_empty() || area.is_empty() {
        return Vec::new();
    }

    let total = match direction {
        Direction::Horizontal => area.width,
        Direction::Vertical => area.height,
    };

    let mut lengths = Vec::with_capacity(constraints.len());
    let mut fixed = 0u16;
    let mut percentages = 0u16;

    for constraint in constraints {
        match *constraint {
            Constraint::Length(length) => {
                fixed = fixed.saturating_add(length);
                lengths.push(length);
            }
            Constraint::Percentage(percentage) => {
                percentages = percentages.saturating_add(percentage);
                lengths.push(0);
            }
            Constraint::Min(minimum) => {
                fixed = fixed.saturating_add(minimum);
                lengths.push(minimum);
            }
        }
    }

    let remaining = total.saturating_sub(fixed);
    let mut assigned = 0u16;

    for (index, constraint) in constraints.iter().enumerate() {
        if let Constraint::Percentage(percentage) = *constraint {
            let length = remaining
                .saturating_mul(percentage)
                .checked_div(percentages)
                .unwrap_or(0);
            lengths[index] = length;
            assigned = assigned.saturating_add(length);
        }
    }

    let mut slack = remaining.saturating_sub(assigned);
    for (index, constraint) in constraints.iter().enumerate() {
        if matches!(constraint, Constraint::Min(_)) && slack > 0 {
            lengths[index] = lengths[index].saturating_add(slack);
            slack = 0;
        }
    }

    let mut offset = 0u16;
    let mut rectangles = Vec::with_capacity(constraints.len());

    for length in lengths {
        let length = length.min(total.saturating_sub(offset));
        let rectangle = match direction {
            Direction::Horizontal => Rect::new(area.x + offset, area.y, length, area.height),
            Direction::Vertical => Rect::new(area.x, area.y + offset, area.width, length),
        };
        rectangles.push(rectangle);
        offset = offset.saturating_add(length);
    }

    rectangles
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_columns() {
        let areas = split(
            Rect::new(0, 0, 100, 20),
            Direction::Horizontal,
            &[Constraint::Percentage(50), Constraint::Percentage(50)],
        );

        assert_eq!(areas[0].width, 50);
        assert_eq!(areas[1].x, 50);
    }

    #[test]
    fn inner_keeps_margins() {
        assert_eq!(Rect::new(1, 2, 10, 10).inner(1), Rect::new(2, 3, 8, 8));
    }
}
