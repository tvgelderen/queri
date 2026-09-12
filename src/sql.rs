use anyhow::{Result, ensure};
use sqlparser::{
    dialect::GenericDialect,
    tokenizer::{Token, Tokenizer},
};

pub fn quote_identifier(name: &str) -> Result<String> {
    ensure!(
        !name.is_empty() && !name.contains('\0'),
        "Invalid empty or NUL-containing database identifier."
    );
    Ok(format!("\"{}\"", name.replace('"', "\"\"")))
}

pub fn qualified(schema: &str, table: &str) -> Result<String> {
    Ok(format!(
        "{}.{}",
        quote_identifier(schema)?,
        quote_identifier(table)?
    ))
}

/// Tokenization excludes strings and comments, including PostgreSQL dollar-quoted bodies.
/// This is an execution reminder, not an SQL authorization boundary.
pub fn destructive(sql: &str) -> bool {
    let Ok(tokens) = Tokenizer::new(&GenericDialect, sql).tokenize() else {
        return true;
    };
    tokens
        .split(|token| *token == Token::SemiColon)
        .any(|statement| {
            let words: Vec<_> = statement
                .iter()
                .filter_map(|token| match token {
                    Token::Word(word) if word.quote_style.is_none() => {
                        Some(word.value.to_ascii_uppercase())
                    }
                    _ => None,
                })
                .collect();
            words
                .iter()
                .any(|word| word == "DROP" || word == "TRUNCATE")
                || (words.iter().any(|word| word == "DELETE")
                    && !words.iter().any(|word| word == "WHERE"))
        })
}

#[derive(Clone, Copy)]
pub enum SqlDialect {
    Postgres,
    Sqlite,
}

/// Formatting is accepted only when the dialect parser and significant tokens agree.
pub fn format_sql(sql: &str, dialect: SqlDialect) -> Result<String> {
    use sqlparser::{
        dialect::{Dialect, PostgreSqlDialect, SQLiteDialect},
        parser::Parser,
        tokenizer::Whitespace,
    };
    let parser: &dyn Dialect = match dialect {
        SqlDialect::Postgres => &PostgreSqlDialect {},
        SqlDialect::Sqlite => &SQLiteDialect {},
    };
    Parser::parse_sql(parser, sql)
        .map_err(|e| anyhow::anyhow!("Cannot format incomplete or unsupported SQL: {e}"))?;
    let tokens = |text: &str| -> Result<Vec<Token>> {
        Ok(Tokenizer::new(parser, text)
            .with_unescape(false)
            .tokenize()?
            .into_iter()
            .filter_map(|token| match token {
                Token::Whitespace(Whitespace::Space | Whitespace::Newline | Whitespace::Tab) => {
                    None
                }
                Token::Whitespace(Whitespace::SingleLineComment { comment, prefix }) => {
                    Some(Token::Whitespace(Whitespace::SingleLineComment {
                        comment: comment.trim_end_matches(['\r', '\n']).to_owned(),
                        prefix,
                    }))
                }
                token => Some(token),
            })
            .collect())
    };
    let options = sqlformat::FormatOptions {
        dialect: match dialect {
            SqlDialect::Postgres => sqlformat::Dialect::PostgreSql,
            SqlDialect::Sqlite => sqlformat::Dialect::Generic,
        },
        ..Default::default()
    };
    // The formatter does not understand PostgreSQL dollar-quoted bodies. Protect
    // them as opaque string literals; never format procedural SQL inside a body.
    let format = |text: &str| -> Result<String> {
        let mut protected = text.to_owned();
        let mut literals = vec![];
        for token in Tokenizer::new(parser, text)
            .with_unescape(false)
            .tokenize()?
        {
            if matches!(token, Token::DollarQuotedString(_)) {
                let raw = token.to_string();
                let marker = format!("'queri_literal_{}'", uuid::Uuid::new_v4().simple());
                protected = protected.replace(&raw, &marker);
                literals.push((marker, raw));
            }
        }
        let mut output = sqlformat::format(&protected, &sqlformat::QueryParams::None, &options);
        for (marker, raw) in literals {
            output = output.replace(&marker, &raw);
        }
        Ok(output)
    };
    // sqlformat settles VALUES indentation inside a CTE on its second pass.
    // Still require stable output below, and validate the final contents.
    let formatted = format(&format(sql)?)?;
    ensure!(
        tokens(sql)? == tokens(&formatted)?,
        "This SQL syntax cannot be formatted without changing its contents. The original text was kept."
    );
    Parser::parse_sql(parser, &formatted)
        .map_err(|e| anyhow::anyhow!("Formatted SQL did not validate: {e}"))?;
    ensure!(
        format(&formatted)? == formatted,
        "The formatter could not produce stable output. The original text was kept."
    );
    Ok(formatted)
}

#[cfg(test)]
mod format_tests {
    use super::*;
    #[test]
    fn formatting_preserves_dialect_fixtures() {
        let fixtures = [
            (
                SqlDialect::Postgres,
                "select :name::text, :name::text, :number::integer",
            ),
            (SqlDialect::Sqlite, "select ?2, ?1, ?2, :name"),
            (
                SqlDialect::Postgres,
                "-- keep this\nselect 'é;--x' as \"Odd Name\", $1::numeric;",
            ),
            (
                SqlDialect::Postgres,
                "WITH x AS (SELECT 1 AS n) SELECT n, row_number() OVER (ORDER BY n) FROM x;",
            ),
            (
                SqlDialect::Postgres,
                "CREATE FUNCTION f() RETURNS text LANGUAGE sql AS $body$ SELECT 'hello'; $body$;",
            ),
            (
                SqlDialect::Postgres,
                "SELECT payload->>'name' FROM items WHERE id=$1;",
            ),
            (
                SqlDialect::Sqlite,
                "SELECT x'00ff', :name, 'quote''s'; -- final comment",
            ),
            (
                SqlDialect::Sqlite,
                "CREATE TRIGGER changed AFTER INSERT ON items BEGIN UPDATE items SET name='changed' WHERE id=NEW.id; END;",
            ),
            (SqlDialect::Sqlite, "/* keep comment */ select 1; select 2;"),
        ];
        for (dialect, sql) in fixtures {
            let formatted = format_sql(sql, dialect).unwrap_or_else(|e| panic!("{sql}: {e:#}"));
            assert_eq!(format_sql(&formatted, dialect).unwrap(), formatted);
        }
        assert!(format_sql("SELECT (", SqlDialect::Sqlite).is_err());
    }
}
