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
