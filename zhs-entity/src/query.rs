use std::fmt;
use std::marker::PhantomData;

use crate::DataError;

use crate::column::{Direction, Order};
use crate::db::{Db, Transaction};
use crate::entity::Entity;
use crate::expr::{self, Expr, Node};

#[derive(Clone, Debug, PartialEq)]
pub struct CompiledQuery {
    pub table: &'static str,

    pub columns: &'static [&'static str],

    pub select: Vec<&'static str>,

    pub filter: Option<Node>,

    pub order: Vec<(&'static str, Direction)>,

    pub limit: Option<u64>,

    pub offset: Option<u64>,
}

pub struct Query<M> {
    filters: Vec<Node>,
    order: Vec<(&'static str, Direction)>,
    limit: Option<u64>,
    offset: Option<u64>,
    select: Option<Vec<&'static str>>,
    marker: PhantomData<fn() -> M>,
}

impl<M> Query<M> {
    pub const fn new() -> Self {
        Self {
            filters: Vec::new(),
            order: Vec::new(),
            limit: None,
            offset: None,
            select: None,
            marker: PhantomData,
        }
    }

    pub fn filter(mut self, condition: Expr<M>) -> Self {
        self.filters.push(condition.into_node());
        self
    }

    pub fn order_by(mut self, order: Order<M>) -> Self {
        self.order.push((order.column(), order.direction()));
        self
    }

    pub fn limit(mut self, limit: u64) -> Self {
        self.limit = Some(limit);
        self
    }

    pub fn offset(mut self, offset: u64) -> Self {
        self.offset = Some(offset);
        self
    }

    pub fn select<I>(mut self, columns: I) -> Self
    where
        I: IntoIterator<Item = &'static str>,
    {
        self.select = Some(columns.into_iter().collect());
        self
    }
}

impl<M> Clone for Query<M> {
    fn clone(&self) -> Self {
        Self {
            filters: self.filters.clone(),
            order: self.order.clone(),
            limit: self.limit,
            offset: self.offset,
            select: self.select.clone(),
            marker: PhantomData,
        }
    }
}

impl<M> Default for Query<M> {
    fn default() -> Self {
        Self::new()
    }
}

impl<M> fmt::Debug for Query<M> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Query")
            .field("filters", &self.filters)
            .field("order", &self.order)
            .field("limit", &self.limit)
            .field("offset", &self.offset)
            .field("select", &self.select)
            .finish()
    }
}

impl<M> Query<M>
where
    M: Entity,
{
    pub fn compile(&self) -> Result<CompiledQuery, DataError> {
        for node in &self.filters {
            expr::validate(node)?;
        }

        let filter = match self.filters.len() {
            0 => None,
            1 => self.filters.first().cloned(),
            _ => Some(Node::And(self.filters.clone())),
        };

        let select = match &self.select {
            Some(columns) if columns.is_empty() => {
                return Err(DataError::query("`select` needs at least one column"));
            }
            Some(columns) => columns.clone(),
            None => M::COLUMNS.to_vec(),
        };

        Ok(CompiledQuery {
            table: M::TABLE,
            columns: M::COLUMNS,
            select,
            filter,
            order: self.order.clone(),
            limit: self.limit,
            offset: self.offset,
        })
    }

    pub async fn fetch_all(self, db: &Db) -> Result<Vec<M>, DataError> {
        let compiled = self.compile()?;
        let rows = db.fetch(compiled).await?;
        rows.iter().map(M::from_row).collect()
    }

    pub async fn first(self, db: &Db) -> Result<Option<M>, DataError> {
        let mut query = self;
        query.limit = Some(1);
        Ok(query.fetch_all(db).await?.into_iter().next())
    }

    pub async fn count(self, db: &Db) -> Result<u64, DataError> {
        db.count(self.compile()?).await
    }

    pub fn fetch_all_in(self, transaction: &mut dyn Transaction) -> Result<Vec<M>, DataError> {
        let compiled = self.compile()?;
        let rows = transaction.fetch(&compiled)?;
        rows.iter().map(M::from_row).collect()
    }

    pub fn first_in(self, transaction: &mut dyn Transaction) -> Result<Option<M>, DataError> {
        let mut query = self;
        query.limit = Some(1);
        Ok(query.fetch_all_in(transaction)?.into_iter().next())
    }
}
