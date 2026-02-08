//! An example of using the [`QueryBuilder`] fluent API to construct a query AST
//! object.
use jsongrep::query::{Query, QueryBuilder};

fn main() {
    // Construct the query "foo.bar"
    let query: Query = QueryBuilder::new().field("foo").field("bar").build();

    // We can verify that the constructed query matches what we expect
    assert_eq!("foo.bar", query.to_string());

    // Another, more complex example: "bar[2]? | foo[2:5].**.[*]"
    let subquery1 =
        QueryBuilder::new().field("bar").index(2).optional().build();
    let subquery2 = QueryBuilder::new()
        .field("foo")
        .range(2..5)
        .field_wildcard()
        .array_wildcard()
        .build();
    let query =
        QueryBuilder::new().disjunction(vec![subquery1, subquery2]).build();
    assert_eq!("bar[2]? | foo[2:5].**.[*]", query.to_string());
}
