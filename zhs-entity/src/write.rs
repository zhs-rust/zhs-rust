use std::fmt;
use std::marker::PhantomData;

use crate::DataError;
use zhs_json::{ToJson, Value};

use crate::column::Column;
use crate::db::{Db, Transaction};
use crate::entity::Entity;
use crate::expr::{self, Expr, Node};

#[derive(Clone, Debug, PartialEq)]
pub struct CompiledInsert {
    pub table: &'static str,

    pub columns: Vec<&'static str>,

    pub values: Vec<Value>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CompiledUpdate {
    pub table: &'static str,

    pub assignments: Vec<(&'static str, Value)>,

    pub filter: Option<Node>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CompiledDelete {
    pub table: &'static str,

    pub filter: Option<Node>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WriteOutcome {
    pub affected_rows: u64,

    pub last_insert_id: Option<u64>,
}

pub struct Insert<M> {
    table: &'static str,
    columns: Vec<&'static str>,
    values: Vec<Value>,
    marker: PhantomData<fn() -> M>,
}

impl<M> Insert<M>
where
    M: Entity,
{
    pub fn new(entity: &M) -> Self {
        let pairs = entity.values();
        debug_assert_eq!(pairs.len(), M::COLUMNS.len());

        let mut columns = Vec::with_capacity(pairs.len());
        let mut values = Vec::with_capacity(pairs.len());
        for (column, value) in pairs {
            columns.push(column);
            values.push(value);
        }

        Self {
            table: M::TABLE,
            columns,
            values,
            marker: PhantomData,
        }
    }

    pub fn value<V>(mut self, column: Column<M, V>, value: V) -> Self
    where
        V: ToJson,
    {
        let name = column.name();
        match self.columns.iter().position(|column| *column == name) {
            Some(index) => self.values[index] = value.to_json(),
            None => {
                self.columns.push(name);
                self.values.push(value.to_json());
            }
        }
        self
    }

    pub fn omit<V>(self, column: Column<M, V>) -> Self {
        self.omit_column(column.name())
    }

    pub fn omit_column(mut self, name: &str) -> Self {
        if let Some(index) = self.columns.iter().position(|column| *column == name) {
            self.columns.remove(index);
            self.values.remove(index);
        }
        self
    }

    pub fn compile(&self) -> Result<CompiledInsert, DataError> {
        if self.columns.is_empty() {
            return Err(DataError::query("`insert` needs at least one column"));
        }
        if self.columns.len() != self.values.len() {
            return Err(DataError::query(
                "`insert` columns and values must have the same length",
            ));
        }

        Ok(CompiledInsert {
            table: self.table,
            columns: self.columns.clone(),
            values: self.values.clone(),
        })
    }

    pub async fn execute(self, db: &Db) -> Result<WriteOutcome, DataError> {
        db.insert_compiled(self.compile()?).await
    }

    pub fn execute_in(&self, transaction: &mut dyn Transaction) -> Result<WriteOutcome, DataError> {
        transaction.insert(&self.compile()?)
    }
}

impl<M> Clone for Insert<M> {
    fn clone(&self) -> Self {
        Self {
            table: self.table,
            columns: self.columns.clone(),
            values: self.values.clone(),
            marker: PhantomData,
        }
    }
}

impl<M> fmt::Debug for Insert<M> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Insert")
            .field("table", &self.table)
            .field("columns", &self.columns)
            .field("values", &self.values)
            .finish()
    }
}

pub struct Update<M> {
    assignments: Vec<(&'static str, Value)>,
    filters: Vec<Node>,
    marker: PhantomData<fn() -> M>,
}

impl<M> Update<M>
where
    M: Entity,
{
    pub const fn new() -> Self {
        Self {
            assignments: Vec::new(),
            filters: Vec::new(),
            marker: PhantomData,
        }
    }

    pub fn set<V>(mut self, column: Column<M, V>, value: V) -> Self
    where
        V: ToJson,
    {
        let name = column.name();
        let value = value.to_json();
        match self
            .assignments
            .iter_mut()
            .find(|(column, _)| *column == name)
        {
            Some(slot) => slot.1 = value,
            None => self.assignments.push((name, value)),
        }
        self
    }

    #[allow(dead_code)]
    pub(crate) fn set_value(mut self, column: &'static str, value: Value) -> Self {
        match self
            .assignments
            .iter_mut()
            .find(|(name, _)| *name == column)
        {
            Some(slot) => slot.1 = value,
            None => self.assignments.push((column, value)),
        }
        self
    }

    pub fn filter(mut self, condition: Expr<M>) -> Self {
        self.filters.push(condition.into_node());
        self
    }

    pub fn compile(&self) -> Result<CompiledUpdate, DataError> {
        if self.assignments.is_empty() {
            return Err(DataError::query("`update` needs at least one assignment"));
        }
        for node in &self.filters {
            expr::validate(node)?;
        }

        Ok(CompiledUpdate {
            table: M::TABLE,
            assignments: self.assignments.clone(),
            filter: combine(&self.filters),
        })
    }

    pub async fn execute(self, db: &Db) -> Result<u64, DataError> {
        db.update_compiled(self.compile()?).await
    }

    pub fn execute_in(&self, transaction: &mut dyn Transaction) -> Result<u64, DataError> {
        transaction.update(&self.compile()?)
    }
}

impl<M> Default for Update<M>
where
    M: Entity,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<M> Clone for Update<M> {
    fn clone(&self) -> Self {
        Self {
            assignments: self.assignments.clone(),
            filters: self.filters.clone(),
            marker: PhantomData,
        }
    }
}

impl<M> fmt::Debug for Update<M> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Update")
            .field("assignments", &self.assignments)
            .field("filters", &self.filters)
            .finish()
    }
}

pub struct Delete<M> {
    filters: Vec<Node>,
    marker: PhantomData<fn() -> M>,
}

impl<M> Delete<M>
where
    M: Entity,
{
    pub const fn new() -> Self {
        Self {
            filters: Vec::new(),
            marker: PhantomData,
        }
    }

    pub fn filter(mut self, condition: Expr<M>) -> Self {
        self.filters.push(condition.into_node());
        self
    }

    pub fn compile(&self) -> Result<CompiledDelete, DataError> {
        for node in &self.filters {
            expr::validate(node)?;
        }

        Ok(CompiledDelete {
            table: M::TABLE,
            filter: combine(&self.filters),
        })
    }

    pub async fn execute(self, db: &Db) -> Result<u64, DataError> {
        db.delete_compiled(self.compile()?).await
    }

    pub fn execute_in(&self, transaction: &mut dyn Transaction) -> Result<u64, DataError> {
        transaction.delete(&self.compile()?)
    }
}

impl<M> Default for Delete<M>
where
    M: Entity,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<M> Clone for Delete<M> {
    fn clone(&self) -> Self {
        Self {
            filters: self.filters.clone(),
            marker: PhantomData,
        }
    }
}

impl<M> fmt::Debug for Delete<M> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Delete")
            .field("filters", &self.filters)
            .finish()
    }
}

fn combine(filters: &[Node]) -> Option<Node> {
    match filters.len() {
        0 => None,
        1 => filters.first().cloned(),
        _ => Some(Node::And(filters.to_vec())),
    }
}
