//! Parameter discovery preserves source bytes; values never become SQL text or persisted definitions.
use crate::{model::Cell, sql::SqlDialect};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sqlparser::{
    dialect::{Dialect, PostgreSqlDialect, SQLiteDialect},
    tokenizer::{Location, Token, Tokenizer},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParameterType {
    #[default]
    Text,
    Integer,
    Real,
    Blob,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Definition {
    pub name: String,
    pub kind: ParameterType,
}
#[derive(Clone, Default)]
pub struct BoundQuery {
    pub sql: String,
    pub values: Vec<Cell>,
}
#[derive(Clone, Debug)]
pub struct Parameters {
    pub sql: String,
    pub names: Vec<String>,
}

impl ParameterType {
    pub fn parse(self, input: &str, null: bool) -> Result<Cell> {
        if null {
            return Ok(Cell::Null);
        }
        Ok(match self {
            Self::Text => Cell::Text(input.into()),
            Self::Integer => Cell::Integer(
                input
                    .trim()
                    .parse()
                    .context("Enter a signed 64-bit integer, or select NULL.")?,
            ),
            Self::Real => {
                let value: f64 = input
                    .trim()
                    .parse()
                    .context("Enter a finite real number, or select NULL.")?;
                ensure!(value.is_finite(), "Enter a finite real number.");
                Cell::Real(value)
            }
            Self::Blob => {
                let input = input.trim();
                ensure!(
                    input.len().is_multiple_of(2) && input.bytes().all(|c| c.is_ascii_hexdigit()),
                    "Enter hexadecimal byte pairs (for example 00ff), without a prefix."
                );
                Cell::Blob(
                    (0..input.len())
                        .step_by(2)
                        .map(|i| u8::from_str_radix(&input[i..i + 2], 16).unwrap())
                        .collect(),
                )
            }
        })
    }
}
impl Parameters {
    pub fn definitions(&self, previous: &[Definition]) -> Vec<Definition> {
        self.names
            .iter()
            .map(|name| {
                previous
                    .iter()
                    .find(|d| &d.name == name)
                    .cloned()
                    .unwrap_or(Definition {
                        name: name.clone(),
                        kind: ParameterType::Text,
                    })
            })
            .collect()
    }
    pub fn bind(&self, values: Vec<Cell>) -> Result<BoundQuery> {
        ensure!(
            values.len() == self.names.len(),
            "Supply a value or explicit NULL for each parameter."
        );
        Ok(BoundQuery {
            sql: self.sql.clone(),
            values,
        })
    }
}

pub fn discover(sql: &str, dialect: SqlDialect) -> Result<Parameters> {
    let pg = matches!(dialect, SqlDialect::Postgres);
    let dialect: &dyn Dialect = if pg {
        &PostgreSqlDialect {}
    } else {
        &SQLiteDialect {}
    };
    let tokens = Tokenizer::new(dialect, sql)
        .with_unescape(false)
        .tokenize_with_location()
        .context("Finish the SQL literal or comment before entering parameters.")?;
    // Tokenizer columns count Unicode scalars, including tabs as one column.
    let mut locations = std::collections::HashMap::new();
    let (mut line, mut column) = (1, 1);
    for (offset, ch) in sql.char_indices() {
        locations.insert(Location::new(line, column), offset);
        if ch == '\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
    }
    locations.insert(Location::new(line, column), sql.len());
    let offset = |loc| {
        locations
            .get(&loc)
            .copied()
            .context("Unsupported SQL source location.")
    };
    let mut names = Vec::new();
    let mut replacements = Vec::new();
    let mut style = None;
    let mut statements = 0;
    let mut content = false;
    let mut i = 0;
    while i < tokens.len() {
        let token = &tokens[i];
        if token.token == Token::SemiColon {
            if content {
                statements += 1;
                content = false;
            }
            i += 1;
            continue;
        }
        if matches!(token.token, Token::Whitespace(_)) {
            i += 1;
            continue;
        }
        content = true;
        let start = offset(token.span.start)?;
        let mut end = offset(token.span.end)?;
        let found = match &token.token {
            Token::Colon => {
                if let Some(next) = tokens.get(i + 1) {
                    if let Token::Word(word) = &next.token {
                        if word.quote_style.is_none() && token.span.end == next.span.start {
                            end = offset(next.span.end)?;
                            i += 1;
                            Some((true, word.value.clone()))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            Token::Placeholder(p) if (pg && p.starts_with('$')) || (!pg && p.starts_with('?')) => {
                Some((false, p[1..].into()))
            }
            Token::Placeholder(_) if !pg => {
                bail!("Use :name, ? or ?NNN parameters in SQLite; $name and @name are unsupported.")
            }
            Token::AtSign if !pg => bail!("Use :name parameters instead of @name in SQLite."),
            _ => None,
        };
        if let Some((named, name)) = found {
            ensure!(
                style.is_none_or(|s| s == named),
                "Do not mix named and positional parameters. Use only :name or only native positional placeholders."
            );
            style = Some(named);
            let index = if named {
                if let Some(index) = names.iter().position(|n| n == &name) {
                    index + 1
                } else {
                    names.push(name);
                    names.len()
                }
            } else {
                let index = if name.is_empty() && !pg {
                    names
                        .iter()
                        .filter_map(|n: &String| n.parse::<usize>().ok())
                        .max()
                        .unwrap_or(0)
                        + 1
                } else {
                    name.parse::<usize>()
                        .context("Use positive numbered positional parameters.")?
                };
                ensure!(
                    (1..=1000).contains(&index),
                    "Parameter numbers must be between 1 and 1000."
                );
                if !names.contains(&index.to_string()) {
                    names.push(index.to_string());
                }
                index
            };
            replacements.push((start, end, format!("{}{index}", if pg { "$" } else { "?" })));
        }
        i += 1;
    }
    if content {
        statements += 1;
    }
    if !names.is_empty() {
        ensure!(
            statements == 1,
            "Parameterized SQL must contain exactly one statement. Run each statement separately; nothing was executed."
        );
        ensure!(
            names.len() <= 1000,
            "At most 1000 parameters are supported."
        );
        if style == Some(false) {
            names.sort_by_key(|n| n.parse::<usize>().unwrap());
            ensure!(
                names
                    .iter()
                    .enumerate()
                    .all(|(i, n)| n == &(i + 1).to_string()),
                "Positional parameters must be contiguous from 1; fill the numbering gaps."
            );
        }
    }
    let mut rewritten = sql.to_owned();
    for (start, end, value) in replacements.into_iter().rev() {
        rewritten.replace_range(start..end, &value);
    }
    Ok(Parameters {
        sql: rewritten,
        names,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dialect_parameters() {
        let p = discover(
            "select :名::text, :名, $body$:ignored; $1$body$, E'\\\':x', '?' -- :no\n",
            SqlDialect::Postgres,
        )
        .unwrap();
        assert_eq!(p.names, ["名"]);
        assert!(p.sql.starts_with("select $1::text, $1,"));
        assert!(
            discover(
                "select doc ? 'x', doc ?| array['y'], doc @? '$.a'",
                SqlDialect::Postgres
            )
            .unwrap()
            .names
            .is_empty()
        );
        assert_eq!(
            discover("select ?2, ?1, ?", SqlDialect::Sqlite)
                .unwrap()
                .names,
            ["1", "2", "3"]
        );
        for sql in [
            "select :a, $1",
            "select $0",
            "select $2",
            "select :a; delete from t",
            "select :a, 'unfinished",
        ] {
            assert!(discover(sql, SqlDialect::Postgres).is_err(), "{sql}");
        }
        let p = discover(
            "select [a:b], `:x`, ':x', :x, :x /* :no */",
            SqlDialect::Sqlite,
        )
        .unwrap();
        assert_eq!(p.names, ["x"]);
        assert_eq!(
            ParameterType::Text.parse("", false).unwrap(),
            Cell::Text("".into())
        );
        assert_eq!(ParameterType::Text.parse("", true).unwrap(), Cell::Null);
        assert!(
            ParameterType::Integer
                .parse("9223372036854775808", false)
                .is_err()
        );
        assert!(ParameterType::Real.parse("NaN", false).is_err());
        assert_eq!(
            ParameterType::Blob.parse("00ff", false).unwrap(),
            Cell::Blob(vec![0, 255])
        );
    }
}
