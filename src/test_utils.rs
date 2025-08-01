// Licensed to the Apache Software Foundation (ASF) under one
// or more contributor license agreements.  See the NOTICE file
// distributed with this work for additional information
// regarding copyright ownership.  The ASF licenses this file
// to you under the Apache License, Version 2.0 (the
// "License"); you may not use this file except in compliance
// with the License.  You may obtain a copy of the License at
//
//   http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing,
// software distributed under the License is distributed on an
// "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied.  See the License for the
// specific language governing permissions and limitations
// under the License.

/// This module contains internal utilities used for testing the library.
/// While technically public, the library's users are not supposed to rely
/// on this module, as it will change without notice.
//
// Integration tests (i.e. everything under `tests/`) import this
// via `tests/test_utils/helpers`.

#[cfg(not(feature = "std"))]
use alloc::{
    boxed::Box,
    string::{String, ToString},
    vec,
    vec::Vec,
};
use core::fmt::Debug;

use crate::dialect::*;
use crate::parser::{Parser, ParserError};
use crate::tokenizer::{Token, Tokenizer};
use crate::{ast::*, parser::ParserOptions};

#[cfg(test)]
use pretty_assertions::assert_eq;

/// Tests use the methods on this struct to invoke the parser on one or
/// multiple dialects.
pub struct TestedDialects {
    pub dialects: Vec<Box<dyn Dialect>>,
    pub options: Option<ParserOptions>,
    pub recursion_limit: Option<usize>,
}

impl TestedDialects {
    /// Create a TestedDialects with default options and the given dialects.
    pub fn new(dialects: Vec<Box<dyn Dialect>>) -> Self {
        Self {
            dialects,
            options: None,
            recursion_limit: None,
        }
    }

    pub fn new_with_options(dialects: Vec<Box<dyn Dialect>>, options: ParserOptions) -> Self {
        Self {
            dialects,
            options: Some(options),
            recursion_limit: None,
        }
    }

    pub fn with_recursion_limit(mut self, recursion_limit: usize) -> Self {
        self.recursion_limit = Some(recursion_limit);
        self
    }

    fn new_parser<'a>(&self, dialect: &'a dyn Dialect) -> Parser<'a> {
        let parser = Parser::new(dialect);
        let parser = if let Some(options) = &self.options {
            parser.with_options(options.clone())
        } else {
            parser
        };

        let parser = if let Some(recursion_limit) = &self.recursion_limit {
            parser.with_recursion_limit(*recursion_limit)
        } else {
            parser
        };

        parser
    }

    /// Run the given function for all of `self.dialects`, assert that they
    /// return the same result, and return that result.
    pub fn one_of_identical_results<F, T: Debug + PartialEq>(&self, f: F) -> T
    where
        F: Fn(&dyn Dialect) -> T,
    {
        let parse_results = self.dialects.iter().map(|dialect| (dialect, f(&**dialect)));
        parse_results
            .fold(None, |s, (dialect, parsed)| {
                if let Some((prev_dialect, prev_parsed)) = s {
                    assert_eq!(
                        prev_parsed, parsed,
                        "Parse results with {prev_dialect:?} are different from {dialect:?}"
                    );
                }
                Some((dialect, parsed))
            })
            .expect("tested dialects cannot be empty")
            .1
    }

    pub fn run_parser_method<F, T: Debug + PartialEq>(&self, sql: &str, f: F) -> T
    where
        F: Fn(&mut Parser) -> T,
    {
        self.one_of_identical_results(|dialect| {
            let mut parser = self.new_parser(dialect).try_with_sql(sql).unwrap();
            f(&mut parser)
        })
    }

    /// Parses a single SQL string into multiple statements, ensuring
    /// the result is the same for all tested dialects.
    pub fn parse_sql_statements(&self, sql: &str) -> Result<Vec<Statement>, ParserError> {
        self.one_of_identical_results(|dialect| {
            let mut tokenizer = Tokenizer::new(dialect, sql);
            if let Some(options) = &self.options {
                tokenizer = tokenizer.with_unescape(options.unescape);
            }
            let tokens = tokenizer.tokenize()?;
            self.new_parser(dialect)
                .with_tokens(tokens)
                .parse_statements()
        })
        // To fail the `ensure_multiple_dialects_are_tested` test:
        // Parser::parse_sql(&**self.dialects.first().unwrap(), sql)
    }

    /// Ensures that `sql` parses as a single [Statement] for all tested
    /// dialects.
    ///
    /// In general, the canonical SQL should be the same (see crate
    /// documentation for rationale) and you should prefer the `verified_`
    /// variants in testing, such as  [`verified_statement`] or
    /// [`verified_query`].
    ///
    /// If `canonical` is non empty,this function additionally asserts
    /// that:
    ///
    /// 1. parsing `sql` results in the same [`Statement`] as parsing
    ///    `canonical`.
    ///
    /// 2. re-serializing the result of parsing `sql` produces the same
    ///    `canonical` sql string
    ///
    ///  For multiple statements, use [`statements_parse_to`].
    pub fn one_statement_parses_to(&self, sql: &str, canonical: &str) -> Statement {
        let mut statements = self.parse_sql_statements(sql).expect(sql);
        assert_eq!(statements.len(), 1);
        if !canonical.is_empty() && sql != canonical {
            assert_eq!(self.parse_sql_statements(canonical).unwrap(), statements);
        }

        let only_statement = statements.pop().unwrap();

        if !canonical.is_empty() {
            assert_eq!(canonical, only_statement.to_string())
        }
        only_statement
    }

    /// The same as [`one_statement_parses_to`] but it works for a multiple statements
    pub fn statements_parse_to(&self, sql: &str, canonical: &str) -> Vec<Statement> {
        let statements = self.parse_sql_statements(sql).expect(sql);
        if !canonical.is_empty() && sql != canonical {
            assert_eq!(self.parse_sql_statements(canonical).unwrap(), statements);
        } else {
            assert_eq!(
                sql,
                statements
                    .iter()
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>()
                    .join("; ")
            );
        }
        statements
    }

    /// Ensures that `sql` parses as an [`Expr`], and that
    /// re-serializing the parse result produces canonical
    pub fn expr_parses_to(&self, sql: &str, canonical: &str) -> Expr {
        let ast = self
            .run_parser_method(sql, |parser| parser.parse_expr())
            .unwrap();
        assert_eq!(canonical, &ast.to_string());
        ast
    }

    /// Ensures that `sql` parses as a single [Statement], and that
    /// re-serializing the parse result produces the same `sql`
    /// string (is not modified after a serialization round-trip).
    pub fn verified_stmt(&self, sql: &str) -> Statement {
        self.one_statement_parses_to(sql, sql)
    }

    /// Ensures that `sql` parses as a single [Query], and that
    /// re-serializing the parse result produces the same `sql`
    /// string (is not modified after a serialization round-trip).
    pub fn verified_query(&self, sql: &str) -> Query {
        match self.verified_stmt(sql) {
            Statement::Query(query) => *query,
            _ => panic!("Expected Query"),
        }
    }

    /// Ensures that `sql` parses as a single [Query], and that
    /// re-serializing the parse result matches the given canonical
    /// sql string.
    pub fn verified_query_with_canonical(&self, query: &str, canonical: &str) -> Query {
        match self.one_statement_parses_to(query, canonical) {
            Statement::Query(query) => *query,
            _ => panic!("Expected Query"),
        }
    }

    /// Ensures that `sql` parses as a single [Select], and that
    /// re-serializing the parse result produces the same `sql`
    /// string (is not modified after a serialization round-trip).
    pub fn verified_only_select(&self, query: &str) -> Select {
        match *self.verified_query(query).body {
            SetExpr::Select(s) => *s,
            _ => panic!("Expected SetExpr::Select"),
        }
    }

    /// Ensures that `sql` parses as a single [`Select`], and that additionally:
    ///
    /// 1. parsing `sql` results in the same [`Statement`] as parsing
    ///    `canonical`.
    ///
    /// 2. re-serializing the result of parsing `sql` produces the same
    ///    `canonical` sql string
    pub fn verified_only_select_with_canonical(&self, query: &str, canonical: &str) -> Select {
        let q = match self.one_statement_parses_to(query, canonical) {
            Statement::Query(query) => *query,
            _ => panic!("Expected Query"),
        };
        match *q.body {
            SetExpr::Select(s) => *s,
            _ => panic!("Expected SetExpr::Select"),
        }
    }

    /// Ensures that `sql` parses as an [`Expr`], and that
    /// re-serializing the parse result produces the same `sql`
    /// string (is not modified after a serialization round-trip).
    pub fn verified_expr(&self, sql: &str) -> Expr {
        self.expr_parses_to(sql, sql)
    }

    /// Check that the tokenizer returns the expected tokens for the given SQL.
    pub fn tokenizes_to(&self, sql: &str, expected: Vec<Token>) {
        if self.dialects.is_empty() {
            panic!("No dialects to test");
        }

        self.dialects.iter().for_each(|dialect| {
            let mut tokenizer = Tokenizer::new(&**dialect, sql);
            if let Some(options) = &self.options {
                tokenizer = tokenizer.with_unescape(options.unescape);
            }
            let tokens = tokenizer.tokenize().unwrap();
            assert_eq!(expected, tokens, "Tokenized differently for {dialect:?}");
        });
    }
}

/// Returns all available dialects.
pub fn all_dialects() -> TestedDialects {
    TestedDialects::new(vec![
        Box::new(GenericDialect {}),
        Box::new(PostgreSqlDialect {}),
        Box::new(MsSqlDialect {}),
        Box::new(AnsiDialect {}),
        Box::new(SnowflakeDialect {}),
        Box::new(HiveDialect {}),
        Box::new(RedshiftSqlDialect {}),
        Box::new(MySqlDialect {}),
        Box::new(BigQueryDialect {}),
        Box::new(SQLiteDialect {}),
        Box::new(DuckDbDialect {}),
        Box::new(DatabricksDialect {}),
        Box::new(ClickHouseDialect {}),
    ])
}

// Returns all available dialects with the specified parser options
pub fn all_dialects_with_options(options: ParserOptions) -> TestedDialects {
    TestedDialects::new_with_options(all_dialects().dialects, options)
}

/// Returns all dialects matching the given predicate.
pub fn all_dialects_where<F>(predicate: F) -> TestedDialects
where
    F: Fn(&dyn Dialect) -> bool,
{
    let mut dialects = all_dialects();
    dialects.dialects.retain(|d| predicate(&**d));
    dialects
}

/// Returns available dialects. The `except` predicate is used
/// to filter out specific dialects.
pub fn all_dialects_except<F>(except: F) -> TestedDialects
where
    F: Fn(&dyn Dialect) -> bool,
{
    all_dialects_where(|d| !except(d))
}

pub fn assert_eq_vec<T: ToString>(expected: &[&str], actual: &[T]) {
    assert_eq!(
        expected,
        actual.iter().map(ToString::to_string).collect::<Vec<_>>()
    );
}

pub fn only<T>(v: impl IntoIterator<Item = T>) -> T {
    let mut iter = v.into_iter();
    if let (Some(item), None) = (iter.next(), iter.next()) {
        item
    } else {
        panic!("only called on collection without exactly one item")
    }
}

pub fn expr_from_projection(item: &SelectItem) -> &Expr {
    match item {
        SelectItem::UnnamedExpr(expr) => expr,
        _ => panic!("Expected UnnamedExpr"),
    }
}

pub fn alter_table_op_with_name(stmt: Statement, expected_name: &str) -> AlterTableOperation {
    match stmt {
        Statement::AlterTable {
            name,
            if_exists,
            only: is_only,
            operations,
            on_cluster: _,
            location: _,
            iceberg,
        } => {
            assert_eq!(name.to_string(), expected_name);
            assert!(!if_exists);
            assert!(!is_only);
            assert!(!iceberg);
            only(operations)
        }
        _ => panic!("Expected ALTER TABLE statement"),
    }
}

pub fn alter_table_op(stmt: Statement) -> AlterTableOperation {
    alter_table_op_with_name(stmt, "tab")
}

/// Creates a `Value::Number`, panic'ing if n is not a number
pub fn number(n: &str) -> Value {
    Value::Number(n.parse().unwrap(), false)
}

/// Creates a [Value::SingleQuotedString]
pub fn single_quoted_string(s: impl Into<String>) -> Value {
    Value::SingleQuotedString(s.into())
}

pub fn table_alias(name: impl Into<String>) -> Option<TableAlias> {
    Some(TableAlias {
        name: Ident::new(name),
        columns: vec![],
    })
}

pub fn table(name: impl Into<String>) -> TableFactor {
    TableFactor::Table {
        name: ObjectName::from(vec![Ident::new(name.into())]),
        alias: None,
        args: None,
        with_hints: vec![],
        version: None,
        partitions: vec![],
        with_ordinality: false,
        json_path: None,
        sample: None,
        index_hints: vec![],
    }
}

pub fn table_from_name(name: ObjectName) -> TableFactor {
    TableFactor::Table {
        name,
        alias: None,
        args: None,
        with_hints: vec![],
        version: None,
        partitions: vec![],
        with_ordinality: false,
        json_path: None,
        sample: None,
        index_hints: vec![],
    }
}

pub fn table_with_alias(name: impl Into<String>, alias: impl Into<String>) -> TableFactor {
    TableFactor::Table {
        name: ObjectName::from(vec![Ident::new(name)]),
        alias: Some(TableAlias {
            name: Ident::new(alias),
            columns: vec![],
        }),
        args: None,
        with_hints: vec![],
        version: None,
        partitions: vec![],
        with_ordinality: false,
        json_path: None,
        sample: None,
        index_hints: vec![],
    }
}

pub fn join(relation: TableFactor) -> Join {
    Join {
        relation,
        global: false,
        join_operator: JoinOperator::Join(JoinConstraint::Natural),
    }
}

pub fn call(function: &str, args: impl IntoIterator<Item = Expr>) -> Expr {
    Expr::Function(Function {
        name: ObjectName::from(vec![Ident::new(function)]),
        uses_odbc_syntax: false,
        parameters: FunctionArguments::None,
        args: FunctionArguments::List(FunctionArgumentList {
            duplicate_treatment: None,
            args: args
                .into_iter()
                .map(|arg| FunctionArg::Unnamed(FunctionArgExpr::Expr(arg)))
                .collect(),
            clauses: vec![],
        }),
        filter: None,
        null_treatment: None,
        over: None,
        within_group: vec![],
    })
}

/// Gets the first index column (mysql calls it a key part) of the first index found in a
/// [`Statement::CreateIndex`], [`Statement::CreateTable`], or [`Statement::AlterTable`].
pub fn index_column(stmt: Statement) -> Expr {
    match stmt {
        Statement::CreateIndex(CreateIndex { columns, .. }) => {
            columns.first().unwrap().column.expr.clone()
        }
        Statement::CreateTable(CreateTable { constraints, .. }) => {
            match constraints.first().unwrap() {
                TableConstraint::Index { columns, .. } => {
                    columns.first().unwrap().column.expr.clone()
                }
                TableConstraint::Unique { columns, .. } => {
                    columns.first().unwrap().column.expr.clone()
                }
                TableConstraint::PrimaryKey { columns, .. } => {
                    columns.first().unwrap().column.expr.clone()
                }
                TableConstraint::FulltextOrSpatial { columns, .. } => {
                    columns.first().unwrap().column.expr.clone()
                }
                _ => panic!("Expected an index, unique, primary, full text, or spatial constraint (foreign key does not support general key part expressions)"),
            }
        }
        Statement::AlterTable { operations, .. } => match operations.first().unwrap() {
            AlterTableOperation::AddConstraint { constraint, .. } => {
                match constraint {
                    TableConstraint::Index { columns, .. } => {
                        columns.first().unwrap().column.expr.clone()
                    }
                    TableConstraint::Unique { columns, .. } => {
                        columns.first().unwrap().column.expr.clone()
                    }
                    TableConstraint::PrimaryKey { columns, .. } => {
                        columns.first().unwrap().column.expr.clone()
                    }
                    TableConstraint::FulltextOrSpatial {
                        columns,
                        ..
                    } => columns.first().unwrap().column.expr.clone(),
                    _ => panic!("Expected an index, unique, primary, full text, or spatial constraint (foreign key does not support general key part expressions)"),
                }
            }
            _ => panic!("Expected a constraint"),
        },
        _ => panic!("Expected CREATE INDEX, ALTER TABLE, or CREATE TABLE, got: {stmt:?}"),
    }
}
