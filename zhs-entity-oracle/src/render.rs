use zhs_entity::DataError;
use zhs_entity::{CompiledDelete, CompiledInsert, CompiledQuery, CompiledUpdate, Direction, Node};
use zhs_json::Value;

#[derive(Clone, Debug, PartialEq)]
pub struct Statement {
    pub sql: String,

    pub params: Vec<Value>,
}

pub fn render(query: &CompiledQuery) -> Result<Statement, DataError> {
    let mut statement = Statement {
        sql: String::new(),
        params: Vec::new(),
    };

    statement.sql.push_str("SELECT ");
    for (index, column) in query.select.iter().enumerate() {
        if index > 0 {
            statement.sql.push_str(", ");
        }
        statement.sql.push_str(&quote_identifier(column)?);
    }

    statement.sql.push_str(" FROM ");
    statement.sql.push_str(&quote_identifier(query.table)?);

    if let Some(node) = &query.filter {
        statement.sql.push_str(" WHERE ");
        render_node(node, &mut statement)?;
    }

    if !query.order.is_empty() {
        statement.sql.push_str(" ORDER BY ");
        for (index, (column, direction)) in query.order.iter().enumerate() {
            if index > 0 {
                statement.sql.push_str(", ");
            }
            statement.sql.push_str(&quote_identifier(column)?);
            statement.sql.push(' ');
            statement.sql.push_str(match direction {
                Direction::Asc => "ASC",
                Direction::Desc => "DESC",
            });
        }
    }

    match (query.limit, query.offset) {
        (None, None) => {}
        (Some(limit), None) => {
            statement
                .sql
                .push_str(&format!(" FETCH FIRST {limit} ROWS ONLY"));
        }
        (Some(limit), Some(offset)) => {
            statement.sql.push_str(&format!(
                " OFFSET {offset} ROWS FETCH NEXT {limit} ROWS ONLY"
            ));
        }
        (None, Some(offset)) => {
            statement.sql.push_str(&format!(" OFFSET {offset} ROWS"));
        }
    }

    Ok(statement)
}

pub fn render_count(query: &CompiledQuery) -> Result<Statement, DataError> {
    let mut statement = Statement {
        sql: String::new(),
        params: Vec::new(),
    };

    statement.sql.push_str("SELECT COUNT(*) AS count FROM ");
    statement.sql.push_str(&quote_identifier(query.table)?);

    if let Some(node) = &query.filter {
        statement.sql.push_str(" WHERE ");
        render_node(node, &mut statement)?;
    }

    Ok(statement)
}

pub fn quote_identifier(name: &str) -> Result<String, DataError> {
    let valid = !name.is_empty()
        && name
            .chars()
            .all(|character| character.is_alphanumeric() || matches!(character, '_' | '$' | '#'));

    if !valid {
        return Err(DataError::identifier(name));
    }

    Ok(format!("\"{}\"", name.to_ascii_uppercase()))
}

fn render_node(node: &Node, statement: &mut Statement) -> Result<(), DataError> {
    match node {
        Node::Compare { column, op, value } => {
            statement.sql.push_str(&quote_identifier(column)?);
            statement.sql.push(' ');
            statement.sql.push_str(op.sql_symbol());
            statement.sql.push(' ');
            bind(statement, value.clone());
        }
        Node::AnyOf { column, values } => {
            statement.sql.push_str(&quote_identifier(column)?);
            statement.sql.push_str(" IN (");
            for (index, value) in values.iter().enumerate() {
                if index > 0 {
                    statement.sql.push_str(", ");
                }
                bind(statement, value.clone());
            }
            statement.sql.push(')');
        }
        Node::Between { column, low, high } => {
            statement.sql.push_str(&quote_identifier(column)?);
            statement.sql.push_str(" BETWEEN ");
            bind(statement, low.clone());
            statement.sql.push_str(" AND ");
            bind(statement, high.clone());
        }
        Node::Null { column, negated } => {
            statement.sql.push_str(&quote_identifier(column)?);
            statement
                .sql
                .push_str(if *negated { " IS NOT NULL" } else { " IS NULL" });
        }
        Node::And(nodes) => {
            statement.sql.push('(');
            for (index, node) in nodes.iter().enumerate() {
                if index > 0 {
                    statement.sql.push_str(" AND ");
                }
                render_node(node, statement)?;
            }
            statement.sql.push(')');
        }
        Node::Or(nodes) => {
            statement.sql.push('(');
            for (index, node) in nodes.iter().enumerate() {
                if index > 0 {
                    statement.sql.push_str(" OR ");
                }
                render_node(node, statement)?;
            }
            statement.sql.push(')');
        }
        Node::Not(inner) => {
            statement.sql.push_str("NOT (");
            render_node(inner, statement)?;
            statement.sql.push(')');
        }
    }

    Ok(())
}

fn bind(statement: &mut Statement, value: Value) {
    statement.params.push(value);
    let index = statement.params.len();
    statement.sql.push_str(&format!(":{index}"));
}

pub fn render_insert(insert: &CompiledInsert) -> Result<Statement, DataError> {
    if insert.columns.is_empty() {
        return Err(DataError::query("`insert` needs at least one column"));
    }
    if insert.columns.len() != insert.values.len() {
        return Err(DataError::query(
            "`insert` columns and values must have the same length",
        ));
    }

    let mut statement = Statement {
        sql: String::new(),
        params: Vec::new(),
    };

    statement.sql.push_str("INSERT INTO ");
    statement.sql.push_str(&quote_identifier(insert.table)?);
    statement.sql.push_str(" (");

    for (index, column) in insert.columns.iter().enumerate() {
        if index > 0 {
            statement.sql.push_str(", ");
        }
        statement.sql.push_str(&quote_identifier(column)?);
    }

    statement.sql.push_str(") VALUES (");
    for (index, value) in insert.values.iter().enumerate() {
        if index > 0 {
            statement.sql.push_str(", ");
        }
        bind(&mut statement, value.clone());
    }
    statement.sql.push(')');

    Ok(statement)
}

pub fn render_update(update: &CompiledUpdate) -> Result<Statement, DataError> {
    if update.assignments.is_empty() {
        return Err(DataError::query("`update` needs at least one assignment"));
    }

    let mut statement = Statement {
        sql: String::new(),
        params: Vec::new(),
    };

    statement.sql.push_str("UPDATE ");
    statement.sql.push_str(&quote_identifier(update.table)?);
    statement.sql.push_str(" SET ");

    for (index, (column, value)) in update.assignments.iter().enumerate() {
        if index > 0 {
            statement.sql.push_str(", ");
        }
        statement.sql.push_str(&quote_identifier(column)?);
        statement.sql.push_str(" = ");
        bind(&mut statement, value.clone());
    }

    if let Some(node) = &update.filter {
        statement.sql.push_str(" WHERE ");
        render_node(node, &mut statement)?;
    }

    Ok(statement)
}

pub fn render_delete(delete: &CompiledDelete) -> Result<Statement, DataError> {
    let mut statement = Statement {
        sql: String::new(),
        params: Vec::new(),
    };

    statement.sql.push_str("DELETE FROM ");
    statement.sql.push_str(&quote_identifier(delete.table)?);

    if let Some(node) = &delete.filter {
        statement.sql.push_str(" WHERE ");
        render_node(node, &mut statement)?;
    }

    Ok(statement)
}

#[cfg(test)]
mod tests {
    use super::*;
    use zhs_entity::Op;

    fn query() -> CompiledQuery {
        CompiledQuery {
            table: "users",
            columns: &["id", "name", "age"],
            select: vec!["id", "name"],
            filter: None,
            order: Vec::new(),
            limit: None,
            offset: None,
        }
    }

    #[test]
    fn renders_select_with_every_clause() {
        let mut query = query();
        query.filter = Some(Node::Compare {
            column: "age",
            op: Op::Gt,
            value: Value::from(18),
        });
        query.order = vec![("id", Direction::Desc)];
        query.limit = Some(10);
        query.offset = Some(5);

        let statement = render(&query).unwrap();
        assert_eq!(
            statement.sql,
            "SELECT \"ID\", \"NAME\" FROM \"USERS\" WHERE \"AGE\" > :1 ORDER BY \"ID\" DESC OFFSET 5 ROWS FETCH NEXT 10 ROWS ONLY"
        );
        assert_eq!(statement.params, vec![Value::from(18)]);
    }

    #[test]
    fn renders_limit_without_offset() {
        let mut query = query();
        query.limit = Some(3);
        assert!(
            render(&query)
                .unwrap()
                .sql
                .ends_with(" FETCH FIRST 3 ROWS ONLY")
        );
    }

    #[test]
    fn renders_offset_without_limit() {
        let mut query = query();
        query.offset = Some(7);
        assert!(render(&query).unwrap().sql.ends_with(" OFFSET 7 ROWS"));
    }

    #[test]
    fn renders_nested_conditions() {
        let mut query = query();
        query.filter = Some(Node::And(vec![
            Node::AnyOf {
                column: "id",
                values: vec![Value::from(1), Value::from(2)],
            },
            Node::Or(vec![
                Node::Null {
                    column: "name",
                    negated: false,
                },
                Node::Between {
                    column: "age",
                    low: Value::from(18),
                    high: Value::from(30),
                },
            ]),
        ]));

        let statement = render(&query).unwrap();
        assert_eq!(
            statement.sql,
            "SELECT \"ID\", \"NAME\" FROM \"USERS\" WHERE (\"ID\" IN (:1, :2) AND (\"NAME\" IS NULL OR \"AGE\" BETWEEN :3 AND :4))"
        );
        assert_eq!(statement.params.len(), 4);
    }

    #[test]
    fn renders_insert_update_delete() {
        let insert = CompiledInsert {
            table: "users",
            columns: vec!["name", "age"],
            values: vec![Value::from("ada"), Value::from(36)],
        };
        let statement = render_insert(&insert).unwrap();
        assert_eq!(
            statement.sql,
            "INSERT INTO \"USERS\" (\"NAME\", \"AGE\") VALUES (:1, :2)"
        );
        assert_eq!(statement.params.len(), 2);

        let update = CompiledUpdate {
            table: "users",
            assignments: vec![("age", Value::from(37))],
            filter: Some(Node::Compare {
                column: "id",
                op: Op::Eq,
                value: Value::from(7),
            }),
        };
        let statement = render_update(&update).unwrap();
        assert_eq!(
            statement.sql,
            "UPDATE \"USERS\" SET \"AGE\" = :1 WHERE \"ID\" = :2"
        );
        assert_eq!(statement.params.len(), 2);

        let delete = CompiledDelete {
            table: "users",
            filter: Some(Node::Compare {
                column: "id",
                op: Op::Eq,
                value: Value::from(7),
            }),
        };
        let statement = render_delete(&delete).unwrap();
        assert_eq!(statement.sql, "DELETE FROM \"USERS\" WHERE \"ID\" = :1");
    }

    #[test]
    fn quotes_and_uppercases_identifiers() {
        assert_eq!(quote_identifier("users").unwrap(), "\"USERS\"");
        assert_eq!(quote_identifier("user_name").unwrap(), "\"USER_NAME\"");
        assert!(quote_identifier("user-name").is_err());
        assert!(quote_identifier("").is_err());
    }

    #[test]
    fn rejects_write_statements_without_columns_or_values() {
        let insert = CompiledInsert {
            table: "users",
            columns: Vec::new(),
            values: Vec::new(),
        };
        assert!(render_insert(&insert).is_err());

        let update = CompiledUpdate {
            table: "users",
            assignments: Vec::new(),
            filter: None,
        };
        assert!(render_update(&update).is_err());
    }
}
