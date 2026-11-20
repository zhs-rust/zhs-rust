use std::fmt;
use std::marker::PhantomData;

use crate::DataError;
use zhs_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Eq,

    Ne,

    Gt,

    Gte,

    Lt,

    Lte,
}

impl Op {
    pub const fn sql_symbol(self) -> &'static str {
        match self {
            Op::Eq => "=",
            Op::Ne => "<>",
            Op::Gt => ">",
            Op::Gte => ">=",
            Op::Lt => "<",
            Op::Lte => "<=",
        }
    }

    pub const fn mongo_operator(self) -> &'static str {
        match self {
            Op::Eq => "$eq",
            Op::Ne => "$ne",
            Op::Gt => "$gt",
            Op::Gte => "$gte",
            Op::Lt => "$lt",
            Op::Lte => "$lte",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Compare {
        column: &'static str,

        op: Op,

        value: Value,
    },

    AnyOf {
        column: &'static str,

        values: Vec<Value>,
    },

    Between {
        column: &'static str,

        low: Value,

        high: Value,
    },

    Null {
        column: &'static str,

        negated: bool,
    },

    And(Vec<Node>),

    Or(Vec<Node>),

    Not(Box<Node>),
}

pub struct Expr<M> {
    node: Node,
    marker: PhantomData<fn() -> M>,
}

impl<M> Expr<M> {
    pub(crate) fn new(node: Node) -> Self {
        Self {
            node,
            marker: PhantomData,
        }
    }

    pub fn node(&self) -> &Node {
        &self.node
    }

    pub(crate) fn into_node(self) -> Node {
        self.node
    }

    pub fn and(self, other: Expr<M>) -> Expr<M> {
        Expr::new(Node::And(vec![self.node, other.node]))
    }

    pub fn or(self, other: Expr<M>) -> Expr<M> {
        Expr::new(Node::Or(vec![self.node, other.node]))
    }

    #[allow(clippy::should_implement_trait)]
    pub fn not(self) -> Expr<M> {
        Expr::new(Node::Not(Box::new(self.node)))
    }
}

impl<M> Clone for Expr<M> {
    fn clone(&self) -> Self {
        Self {
            node: self.node.clone(),
            marker: PhantomData,
        }
    }
}

impl<M> fmt::Debug for Expr<M> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.node.fmt(formatter)
    }
}

pub(crate) fn validate(node: &Node) -> Result<(), DataError> {
    match node {
        Node::Compare { column, value, .. } => {
            if value.is_null() {
                return Err(DataError::query(format!(
                    "comparison on column `{column}` needs a value; use `is_null`"
                )));
            }
            Ok(())
        }
        Node::AnyOf { column, values } => {
            if values.is_empty() {
                return Err(DataError::query(format!(
                    "`any_of` on column `{column}` needs at least one value"
                )));
            }
            if values.iter().any(Value::is_null) {
                return Err(DataError::query(format!(
                    "`any_of` on column `{column}` cannot contain null; use `is_null`"
                )));
            }
            Ok(())
        }
        Node::Between { column, low, high } => {
            if low.is_null() || high.is_null() {
                return Err(DataError::query(format!(
                    "`between` on column `{column}` needs two values"
                )));
            }
            Ok(())
        }
        Node::Null { .. } => Ok(()),
        Node::And(nodes) | Node::Or(nodes) => {
            for node in nodes {
                validate(node)?;
            }
            Ok(())
        }
        Node::Not(inner) => validate(inner),
    }
}
