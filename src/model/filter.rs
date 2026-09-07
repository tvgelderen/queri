use super::{Column, TableMetadata};
use anyhow::{Result, ensure};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterKind {
    Text,
    Number,
    Date,
    Boolean,
}

impl Column {
    pub fn filter_kind(&self, sqlite: bool) -> FilterKind {
        let kind = self.data_type.to_ascii_lowercase();
        if !sqlite && kind.ends_with("[]") {
            return FilterKind::Text;
        }
        let base = kind.split('(').next().unwrap_or(&kind).trim();
        if matches!(
            base,
            "date" | "timestamp" | "timestamp with time zone" | "timestamp without time zone"
        ) || (sqlite && base == "datetime")
        {
            FilterKind::Date
        } else if matches!(base, "boolean" | "bool") {
            FilterKind::Boolean
        } else if sqlite && base.contains("int") {
            FilterKind::Number
        } else if sqlite
            && ["char", "clob", "text", "blob"]
                .iter()
                .any(|name| base.contains(name))
        {
            FilterKind::Text
        } else if (sqlite
            && ["real", "floa", "doub"]
                .iter()
                .any(|name| base.contains(name)))
            || matches!(
                base,
                "smallint"
                    | "integer"
                    | "bigint"
                    | "real"
                    | "double precision"
                    | "numeric"
                    | "decimal"
                    | "float"
                    | "double"
            )
        {
            FilterKind::Number
        } else {
            FilterKind::Text
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_kinds_follow_declared_scalar_types() {
        for (data_type, sqlite, expected) in [
            ("numeric(38,10)", false, FilterKind::Number),
            ("timestamp(3) with time zone", false, FilterKind::Date),
            ("numeric(38,10)[]", false, FilterKind::Text),
            ("timestamp(3) with time zone[]", false, FilterKind::Text),
            ("timestamp_custom", false, FilterKind::Text),
            ("jsonb", false, FilterKind::Text),
            ("BOOLEAN", true, FilterKind::Boolean),
            ("DATETIME", true, FilterKind::Date),
            ("FLOAT8", true, FilterKind::Number),
            ("REAL TEXT", true, FilterKind::Text),
            ("INT TEXT", true, FilterKind::Number),
            ("TEXT", true, FilterKind::Text),
            ("", true, FilterKind::Text),
        ] {
            let column = Column {
                name: "value".into(),
                data_type: data_type.into(),
                nullable: true,
                default: None,
                primary_key: false,
                generated: false,
            };
            assert_eq!(column.filter_kind(sqlite), expected, "{data_type}");
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterOperator {
    Contains,
    NotContains,
    Equals,
    NotEquals,
    StartsWith,
    EndsWith,
    GreaterThan,
    GreaterOrEqual,
    LessThan,
    LessOrEqual,
    IsNull,
    IsNotNull,
}

impl FilterKind {
    pub fn operators(self) -> &'static [FilterOperator] {
        use FilterOperator::*;
        match self {
            Self::Text => &[
                Contains,
                NotContains,
                Equals,
                NotEquals,
                StartsWith,
                EndsWith,
                IsNull,
                IsNotNull,
            ],
            Self::Number | Self::Date => &[
                Equals,
                NotEquals,
                GreaterThan,
                GreaterOrEqual,
                LessThan,
                LessOrEqual,
                IsNull,
                IsNotNull,
            ],
            Self::Boolean => &[Equals, NotEquals, IsNull, IsNotNull],
        }
    }

    pub fn default_operator(self) -> FilterOperator {
        match self {
            Self::Text => FilterOperator::Contains,
            _ => FilterOperator::Equals,
        }
    }
}

impl FilterOperator {
    pub fn label(self, kind: FilterKind) -> &'static str {
        match self {
            Self::Contains => "contains",
            Self::NotContains => "does not contain",
            Self::Equals => "equals",
            Self::NotEquals => "does not equal",
            Self::StartsWith => "starts with",
            Self::EndsWith => "ends with",
            Self::GreaterThan if kind == FilterKind::Date => "after",
            Self::GreaterOrEqual if kind == FilterKind::Date => "on or after",
            Self::LessThan if kind == FilterKind::Date => "before",
            Self::LessOrEqual if kind == FilterKind::Date => "on or before",
            Self::GreaterThan => "greater than",
            Self::GreaterOrEqual => "greater than or equal to",
            Self::LessThan => "less than",
            Self::LessOrEqual => "less than or equal to",
            Self::IsNull => "is NULL",
            Self::IsNotNull => "is not NULL",
        }
    }

    pub fn needs_value(self) -> bool {
        !matches!(self, Self::IsNull | Self::IsNotNull)
    }

    pub fn predicate(self, value: String) -> FilterPredicate {
        match self {
            Self::Contains => FilterPredicate::Contains(value),
            Self::NotContains => FilterPredicate::NotContains(value),
            Self::Equals => FilterPredicate::Equals(value),
            Self::NotEquals => FilterPredicate::NotEquals(value),
            Self::StartsWith => FilterPredicate::StartsWith(value),
            Self::EndsWith => FilterPredicate::EndsWith(value),
            Self::GreaterThan => FilterPredicate::GreaterThan(value),
            Self::GreaterOrEqual => FilterPredicate::GreaterOrEqual(value),
            Self::LessThan => FilterPredicate::LessThan(value),
            Self::LessOrEqual => FilterPredicate::LessOrEqual(value),
            Self::IsNull => FilterPredicate::IsNull,
            Self::IsNotNull => FilterPredicate::IsNotNull,
        }
    }
}

#[derive(Clone, Debug)]
pub enum FilterPredicate {
    Contains(String),
    NotContains(String),
    Equals(String),
    NotEquals(String),
    StartsWith(String),
    EndsWith(String),
    GreaterThan(String),
    GreaterOrEqual(String),
    LessThan(String),
    LessOrEqual(String),
    IsNull,
    IsNotNull,
}

impl FilterPredicate {
    pub fn operator(&self) -> FilterOperator {
        match self {
            Self::Contains(_) => FilterOperator::Contains,
            Self::NotContains(_) => FilterOperator::NotContains,
            Self::Equals(_) => FilterOperator::Equals,
            Self::NotEquals(_) => FilterOperator::NotEquals,
            Self::StartsWith(_) => FilterOperator::StartsWith,
            Self::EndsWith(_) => FilterOperator::EndsWith,
            Self::GreaterThan(_) => FilterOperator::GreaterThan,
            Self::GreaterOrEqual(_) => FilterOperator::GreaterOrEqual,
            Self::LessThan(_) => FilterOperator::LessThan,
            Self::LessOrEqual(_) => FilterOperator::LessOrEqual,
            Self::IsNull => FilterOperator::IsNull,
            Self::IsNotNull => FilterOperator::IsNotNull,
        }
    }

    pub fn value(&self) -> Option<&str> {
        match self {
            Self::Contains(value)
            | Self::NotContains(value)
            | Self::Equals(value)
            | Self::NotEquals(value)
            | Self::StartsWith(value)
            | Self::EndsWith(value)
            | Self::GreaterThan(value)
            | Self::GreaterOrEqual(value)
            | Self::LessThan(value)
            | Self::LessOrEqual(value) => Some(value),
            Self::IsNull | Self::IsNotNull => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct TableFilter {
    pub column: String,
    pub predicate: FilterPredicate,
}

impl TableFilter {
    pub fn validate<'a>(&self, metadata: &'a TableMetadata, sqlite: bool) -> Result<&'a Column> {
        let column = metadata
            .columns
            .iter()
            .find(|column| column.name == self.column)
            .ok_or_else(|| anyhow::anyhow!("Unknown filter column."))?;
        let kind = column.filter_kind(sqlite);
        ensure!(
            kind.operators().contains(&self.predicate.operator()),
            "This filter operator is not available for {}.",
            column.name
        );
        Ok(column)
    }

    pub fn summary(&self, kind: FilterKind) -> String {
        let label = self.predicate.operator().label(kind);
        match self.predicate.value() {
            Some(value) => format!("{} {label} “{value}”", self.column),
            None => format!("{} {label}", self.column),
        }
    }
}
