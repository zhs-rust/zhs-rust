use std::fmt;
use std::marker::PhantomData;

use zhs_json::ToJson;

use crate::expr::{Expr, Node, Op};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Asc,

    Desc,
}

pub struct Column<M, V> {
    name: &'static str,
    marker: PhantomData<fn() -> (M, V)>,
}

impl<M, V> Column<M, V> {
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            marker: PhantomData,
        }
    }

    pub const fn name(&self) -> &'static str {
        self.name
    }

    pub fn is_null(self) -> Expr<M> {
        Expr::new(Node::Null {
            column: self.name,
            negated: false,
        })
    }

    pub fn is_not_null(self) -> Expr<M> {
        Expr::new(Node::Null {
            column: self.name,
            negated: true,
        })
    }

    pub fn asc(self) -> Order<M> {
        Order::new(self.name, Direction::Asc)
    }

    pub fn desc(self) -> Order<M> {
        Order::new(self.name, Direction::Desc)
    }
}

impl<M, V> Column<M, V>
where
    V: ToJson,
{
    #[allow(clippy::should_implement_trait)]
    pub fn eq(self, value: V) -> Expr<M> {
        self.compare(Op::Eq, &value)
    }

    #[allow(clippy::should_implement_trait)]
    pub fn ne(self, value: V) -> Expr<M> {
        self.compare(Op::Ne, &value)
    }

    pub fn gt(self, value: V) -> Expr<M> {
        self.compare(Op::Gt, &value)
    }

    pub fn gte(self, value: V) -> Expr<M> {
        self.compare(Op::Gte, &value)
    }

    pub fn lt(self, value: V) -> Expr<M> {
        self.compare(Op::Lt, &value)
    }

    pub fn lte(self, value: V) -> Expr<M> {
        self.compare(Op::Lte, &value)
    }

    pub fn between(self, low: V, high: V) -> Expr<M> {
        Expr::new(Node::Between {
            column: self.name,
            low: low.to_json(),
            high: high.to_json(),
        })
    }

    pub fn any_of<I>(self, values: I) -> Expr<M>
    where
        I: IntoIterator<Item = V>,
    {
        let values = values.into_iter().map(|value| value.to_json()).collect();
        Expr::new(Node::AnyOf {
            column: self.name,
            values,
        })
    }

    fn compare(self, op: Op, value: &V) -> Expr<M> {
        Expr::new(Node::Compare {
            column: self.name,
            op,
            value: value.to_json(),
        })
    }
}

impl<M, V> Clone for Column<M, V> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<M, V> Copy for Column<M, V> {}

impl<M, V> fmt::Debug for Column<M, V> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Column({})", self.name)
    }
}

pub struct Order<M> {
    column: &'static str,
    direction: Direction,
    marker: PhantomData<fn() -> M>,
}

impl<M> Order<M> {
    pub(crate) const fn new(column: &'static str, direction: Direction) -> Self {
        Self {
            column,
            direction,
            marker: PhantomData,
        }
    }

    pub const fn column(&self) -> &'static str {
        self.column
    }

    pub const fn direction(&self) -> Direction {
        self.direction
    }
}

impl<M> Clone for Order<M> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<M> Copy for Order<M> {}

impl<M> fmt::Debug for Order<M> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Order({}, {:?})", self.column, self.direction)
    }
}
