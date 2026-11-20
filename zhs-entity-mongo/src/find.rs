use zhs_entity::DataError;
use zhs_entity::{CompiledDelete, CompiledQuery, CompiledUpdate, Direction, Node, Op};
use zhs_json::{Object, Value};

#[derive(Clone, Debug, PartialEq)]
pub struct Find {
    pub filter: Object,

    pub sort: Option<Object>,

    pub projection: Option<Object>,

    pub limit: Option<u64>,

    pub skip: Option<u64>,
}

pub fn find(query: &CompiledQuery) -> Result<Find, DataError> {
    let filter = match &query.filter {
        Some(node) => render_node(node)?,
        None => Object::new(),
    };

    let sort = if query.order.is_empty() {
        None
    } else {
        let mut sort = Object::new();
        for (column, direction) in &query.order {
            let order = match direction {
                Direction::Asc => 1_i64,
                Direction::Desc => -1_i64,
            };
            sort.insert(*column, Value::from(order));
        }
        Some(sort)
    };

    let projection = if query.select.as_slice() == query.columns {
        None
    } else {
        let mut projection = Object::new();
        for column in &query.select {
            projection.insert(*column, Value::from(1_i64));
        }
        Some(projection)
    };

    Ok(Find {
        filter,
        sort,
        projection,
        limit: query.limit,
        skip: query.offset,
    })
}

pub fn render_filter(filter: Option<&Node>) -> Result<Object, DataError> {
    match filter {
        Some(node) => render_node(node),
        None => Ok(Object::new()),
    }
}

pub fn count_command(query: &CompiledQuery) -> Result<Object, DataError> {
    let mut command = Object::new();
    command.insert("count", Value::from(query.table));

    let filter = render_filter(query.filter.as_ref())?;
    if !filter.is_empty() {
        command.insert("query", Value::Object(filter));
    }

    Ok(command)
}

fn render_node(node: &Node) -> Result<Object, DataError> {
    match node {
        Node::Compare { column, op, value } => {
            let mut object = Object::new();
            if *op == Op::Eq {
                object.insert(*column, value.clone());
            } else {
                let mut operator = Object::new();
                operator.insert(op.mongo_operator(), value.clone());
                object.insert(*column, Value::Object(operator));
            }
            Ok(object)
        }
        Node::AnyOf { column, values } => {
            let mut operator = Object::new();
            operator.insert("$in", Value::Array(values.clone()));

            let mut object = Object::new();
            object.insert(*column, Value::Object(operator));
            Ok(object)
        }
        Node::Between { column, low, high } => {
            let mut operator = Object::new();
            operator.insert("$gte", low.clone());
            operator.insert("$lte", high.clone());

            let mut object = Object::new();
            object.insert(*column, Value::Object(operator));
            Ok(object)
        }
        Node::Null { column, negated } => {
            let mut object = Object::new();
            if *negated {
                let mut operator = Object::new();
                operator.insert("$ne", Value::Null);
                object.insert(*column, Value::Object(operator));
            } else {
                object.insert(*column, Value::Null);
            }
            Ok(object)
        }
        Node::And(nodes) => merge(nodes),
        Node::Or(nodes) => {
            let mut rendered = Vec::with_capacity(nodes.len());
            for node in nodes {
                rendered.push(render_node(node)?);
            }
            Ok(logical("$or", &rendered))
        }
        Node::Not(inner) => {
            let rendered = render_node(inner)?;
            Ok(logical("$nor", &[rendered]))
        }
    }
}

fn merge(nodes: &[Node]) -> Result<Object, DataError> {
    let mut rendered = Vec::with_capacity(nodes.len());
    for node in nodes {
        rendered.push(render_node(node)?);
    }

    let mut merged = Object::new();
    let mut collision = false;
    for part in &rendered {
        for (key, value) in part.iter() {
            if merged.contains_key(key) {
                collision = true;
            } else {
                merged.insert(key, value.clone());
            }
        }
    }

    if collision {
        Ok(logical("$and", &rendered))
    } else {
        Ok(merged)
    }
}

fn logical(operator: &'static str, parts: &[Object]) -> Object {
    let items = parts
        .iter()
        .map(|part| Value::Object(part.clone()))
        .collect();
    let mut object = Object::new();
    object.insert(operator, Value::Array(items));
    object
}

pub fn insert_command(
    table: &str,
    columns: &[&'static str],
    values: &[Value],
) -> Result<Object, DataError> {
    if columns.is_empty() {
        return Err(DataError::query("`insert` needs at least one column"));
    }
    if columns.len() != values.len() {
        return Err(DataError::query(
            "`insert` columns and values must have the same length",
        ));
    }

    let mut document = Object::new();
    for (column, value) in columns.iter().zip(values) {
        document.insert(*column, value.clone());
    }

    let mut command = Object::new();
    command.insert("insert", Value::from(table));
    command.insert("documents", Value::Array(vec![Value::Object(document)]));
    Ok(command)
}

pub fn update_command(update: &CompiledUpdate) -> Result<Object, DataError> {
    if update.assignments.is_empty() {
        return Err(DataError::query("`update` needs at least one assignment"));
    }

    let mut set = Object::new();
    for (column, value) in &update.assignments {
        set.insert(*column, value.clone());
    }

    let mut statement = Object::new();
    statement.insert("q", Value::Object(render_filter(update.filter.as_ref())?));
    statement.insert(
        "u",
        Value::Object({
            let mut fields = Object::new();
            fields.insert("$set", Value::Object(set));
            fields
        }),
    );
    statement.insert("multi", Value::from(true));
    statement.insert("upsert", Value::from(false));

    let mut command = Object::new();
    command.insert("update", Value::from(update.table));
    command.insert("updates", Value::Array(vec![Value::Object(statement)]));
    Ok(command)
}

pub fn delete_command(delete: &CompiledDelete) -> Result<Object, DataError> {
    let mut statement = Object::new();
    statement.insert("q", Value::Object(render_filter(delete.filter.as_ref())?));
    statement.insert("limit", Value::from(0_u64));

    let mut command = Object::new();
    command.insert("delete", Value::from(delete.table));
    command.insert("deletes", Value::Array(vec![Value::Object(statement)]));
    Ok(command)
}
