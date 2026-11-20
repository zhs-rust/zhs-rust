use zhs_entity_sqlite::SqliteDatabase;
use zhs_framework::prelude::*;

#[derive(Debug, FromRow)]
#[table("users")]
struct User {
    #[column("id")]
    id: i64,
    #[column("user_name")]
    name: String,
}

#[test]
fn parameterized_sql_crud() {
    let database = Database::new(SqliteDatabase::in_memory().unwrap());

    database
        .execute(
            "create table users (id integer primary key autoincrement, user_name text not null)",
            &Params::default(),
        )
        .unwrap();

    let inserted = database
        .execute("insert into users (user_name) values (?)", &params!["Ada"])
        .unwrap();
    assert_eq!(inserted, 1);

    database
        .execute(
            "insert into users (user_name) values (?)",
            &params!["Grace"],
        )
        .unwrap();

    let users: Vec<User> = database
        .query_as(
            "select id, user_name from users order by id",
            &Params::default(),
        )
        .unwrap();

    assert_eq!(users.len(), 2);
    assert_eq!(users[0].id, 1);
    assert_eq!(users[0].name, "Ada");
    assert_eq!(users[1].id, 2);
    assert_eq!(users[1].name, "Grace");

    let updated = database
        .execute(
            "update users set user_name = ? where id = ?",
            &params!["Ada Lovelace", 1_i64],
        )
        .unwrap();
    assert_eq!(updated, 1);

    let one: Option<User> = database
        .query_one(
            "select id, user_name from users where id = ?",
            &params![1_i64],
        )
        .unwrap();

    assert_eq!(one.unwrap().name, "Ada Lovelace");

    let deleted = database
        .execute("delete from users where id = ?", &params![2_i64])
        .unwrap();
    assert_eq!(deleted, 1);
}

#[test]
fn empty_blob_is_not_null() {
    let database = Database::new(SqliteDatabase::in_memory().unwrap());

    database
        .execute(
            "create table blobs (id integer primary key, data blob not null)",
            &Params::default(),
        )
        .unwrap();

    database
        .execute("insert into blobs (data) values (?)", &params![Vec::<u8>::new()])
        .unwrap();

    let rows = database
        .query("select data from blobs", &Params::default())
        .unwrap();

    assert_eq!(rows[0].get::<Vec<u8>>("data").unwrap(), Vec::<u8>::new());
}
