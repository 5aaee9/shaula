//! Small query-builder extensions shared by store repositories.
//!
//! JSON paths render as SQL literals, never bound parameters, so expression
//! indexes such as `idx_workflow_observation_generation` still match.
use sea_orm::{
    sea_query::{
        Alias, Expr, Func, IntoColumnRef, IntoTableRef, Query, SelectStatement, SimpleExpr,
    },
    ColumnTrait, EntityTrait, QueryTrait, Select,
};

/// `json_extract(column, 'path')`; `path` is a fixed, trusted literal.
pub(crate) fn json_extract(column: impl IntoColumnRef, path: &'static str) -> SimpleExpr {
    Func::cust(Alias::new("json_extract"))
        .arg(Expr::col(column))
        .arg(SimpleExpr::Constant(path.into()))
        .into()
}

/// A column qualified by its entity's table name.
pub(crate) fn col(column: impl ColumnTrait) -> Expr {
    Expr::col(column.as_column_ref())
}

/// `left = right` between two qualified columns.
pub(crate) fn same(left: impl ColumnTrait, right: impl ColumnTrait) -> SimpleExpr {
    col(left).equals(right.as_column_ref())
}

/// `SELECT 1 FROM table`, the body of an `EXISTS` probe.
pub(crate) fn probe(table: impl IntoTableRef) -> SelectStatement {
    Query::select().expr(Expr::val(1)).from(table).to_owned()
}

/// Joins on an arbitrary condition, for tables without declared relations.
pub(crate) trait JoinOn: Sized {
    fn inner_join_on(self, table: impl IntoTableRef, on: SimpleExpr) -> Self;
    fn left_join_on(self, table: impl IntoTableRef, on: SimpleExpr) -> Self;
}

impl<E: EntityTrait> JoinOn for Select<E> {
    fn inner_join_on(mut self, table: impl IntoTableRef, on: SimpleExpr) -> Self {
        QueryTrait::query(&mut self).inner_join(table, on);
        self
    }

    fn left_join_on(mut self, table: impl IntoTableRef, on: SimpleExpr) -> Self {
        QueryTrait::query(&mut self).left_join(table, on);
        self
    }
}

/// `(SELECT ...)` as a scalar operand.
pub(crate) fn scalar(select: SelectStatement) -> SimpleExpr {
    SimpleExpr::SubQuery(None, Box::new(select.into_sub_query_statement()))
}
