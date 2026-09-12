use super::{Catalog, Column};
use std::{collections::HashSet, ops::Range};

const KEYWORDS: &str = "SELECT FROM WHERE JOIN LEFT RIGHT INNER OUTER FULL CROSS ON AS WITH RECURSIVE UNION ALL DISTINCT GROUP BY ORDER HAVING LIMIT OFFSET INSERT INTO VALUES UPDATE SET DELETE RETURNING CREATE TABLE VIEW ALTER DROP AND OR NOT NULL IS IN EXISTS CASE WHEN THEN ELSE END ASC DESC COUNT SUM AVG MIN MAX COALESCE";
pub const MAX_SQL: usize = 256 * 1024;

#[derive(Clone, Debug)]
struct Token {
    value: String,
    range: Range<usize>,
    quoted: bool,
    scope: usize,
}
impl Token {
    fn is(&self, value: &str) -> bool {
        !self.quoted && self.value.eq_ignore_ascii_case(value)
    }
    fn identifier(&self) -> bool {
        self.quoted
            || self
                .value
                .chars()
                .next()
                .is_some_and(|c| c.is_alphabetic() || c == '_')
    }
    fn name(&self) -> String {
        if self.quoted {
            self.value.clone()
        } else {
            self.value.to_ascii_lowercase()
        }
    }
}

#[derive(Clone, Debug)]
struct Source {
    scope: usize,
    alias: String,
    schema: Option<String>,
    table: String,
    derived: Option<usize>,
    columns: Vec<String>,
    cte: bool,
    lateral: bool,
}

#[derive(Debug)]
pub struct Context {
    pub replacement: Range<usize>,
    prefix: String,
    qualifier: Option<String>,
    tokens: Vec<Token>,
    parents: Vec<usize>,
    sources: Vec<Source>,
    scope: usize,
    relation_position: bool,
    case_insensitive: bool,
}

#[derive(Debug, Clone)]
pub struct Suggestion {
    pub label: String,
    pub insert: String,
    pub detail: String,
    pub keyword: bool,
}

fn word(c: char) -> bool {
    c.is_alphanumeric() || !c.is_ascii() || c == '_' || c == '$'
}

/// A tolerant lexer is needed here: the execution tokenizer rejects unfinished quotes.
/// Ranges are byte offsets; LSP conversion happens only at the editor boundary.
fn lex(sql: &str, cursor: usize) -> Option<(Vec<Token>, Vec<usize>, usize)> {
    let mut tokens = Vec::new();
    let mut parents = vec![0, 0];
    let mut scope = 1;
    let mut cursor_scope = 1;
    let mut i = 0;
    while i < sql.len() {
        if i <= cursor {
            cursor_scope = scope;
        }
        let start = i;
        let c = sql[i..].chars().next()?;
        if c.is_whitespace() {
            i += c.len_utf8();
            continue;
        }
        let mut ignored = false;
        let mut closed = true;
        let mut quoted = false;
        let value;
        if sql[i..].starts_with("--") {
            i = sql[i..].find('\n').map(|n| i + n).unwrap_or(sql.len());
            ignored = true;
            closed = i < sql.len();
            value = String::new();
        } else if sql[i..].starts_with("/*") {
            i += 2;
            let mut depth = 1;
            while i < sql.len() && depth > 0 {
                if sql[i..].starts_with("/*") {
                    depth += 1;
                    i += 2;
                } else if sql[i..].starts_with("*/") {
                    depth -= 1;
                    i += 2;
                } else {
                    i += sql[i..].chars().next()?.len_utf8();
                }
            }
            closed = depth == 0;
            ignored = true;
            value = String::new();
        } else if c == '\'' || c == '"' || c == '`' || c == '[' {
            let end = if c == '[' { ']' } else { c };
            quoted = c != '\'';
            ignored = !quoted;
            closed = false;
            i += c.len_utf8();
            let mut name = String::new();
            while i < sql.len() {
                let next = sql[i..].chars().next()?;
                i += next.len_utf8();
                if next == end {
                    if sql[i..].starts_with(end) {
                        name.push(end);
                        i += end.len_utf8();
                    } else {
                        closed = true;
                        break;
                    }
                } else if next == '\\' && c == '\'' {
                    // Conservative for PostgreSQL E strings and dialect escape modes.
                    if let Some(next) = sql[i..].chars().next() {
                        i += next.len_utf8();
                    }
                } else {
                    name.push(next);
                }
            }
            value = name;
        } else if c == '$'
            && sql[i + 1..].find('$').is_some_and(|n| {
                sql[i + 1..i + 1 + n]
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '_')
            })
        {
            let n = sql[i + 1..].find('$')?;
            let delimiter = &sql[i..i + n + 2];
            i += delimiter.len();
            if let Some(end) = sql[i..].find(delimiter) {
                i += end + delimiter.len();
            } else {
                i = sql.len();
                closed = false;
            }
            ignored = true;
            value = String::new();
        } else if word(c) {
            i += c.len_utf8();
            while i < sql.len() && sql[i..].chars().next().is_some_and(word) {
                i += sql[i..].chars().next()?.len_utf8();
            }
            value = sql[start..i].into();
        } else {
            i += c.len_utf8();
            value = c.to_string();
        }
        if ignored {
            if cursor > start && (cursor < i || (!closed && cursor == i)) {
                return None;
            }
            continue;
        }
        if value == ")" {
            scope = parents[scope];
        }
        tokens.push(Token {
            value: value.clone(),
            range: start..i,
            quoted,
            scope,
        });
        if value == "(" {
            parents.push(scope);
            scope = parents.len() - 1;
        }
        if value == ";" {
            parents.push(0);
            scope = parents.len() - 1;
        }
        if i <= cursor {
            cursor_scope = scope;
        }
        if tokens.len() > 30000 {
            return None;
        }
    }
    Some((tokens, parents, cursor_scope))
}

impl Context {
    pub fn at(sql: &str, cursor: usize) -> Option<Self> {
        if sql.len() > MAX_SQL || !sql.is_char_boundary(cursor) {
            return None;
        }
        let (tokens, parents, scope) = lex(sql, cursor)?;
        let active = tokens
            .iter()
            .position(|t| t.identifier() && t.range.start < cursor && cursor <= t.range.end);
        let (replacement, prefix) = if let Some(index) = active {
            let t = &tokens[index];
            let raw = &sql[t.range.start..cursor];
            (
                t.range.clone(),
                if t.quoted {
                    raw[1..]
                        .trim_end_matches(['"', '`', ']'])
                        .replace("\"\"", "\"")
                } else {
                    raw.to_lowercase()
                },
            )
        } else {
            (cursor..cursor, String::new())
        };
        let before: Vec<_> = tokens
            .iter()
            .filter(|t| t.range.end <= replacement.start)
            .collect();
        let qualifier = if before.last().is_some_and(|t| t.is(".")) {
            before
                .iter()
                .rev()
                .nth(1)
                .filter(|t| t.identifier())
                .map(|t| t.name())
        } else {
            None
        };
        let relation_position = before
            .iter()
            .rev()
            .find(|t| t.scope == scope && !t.is("."))
            .is_some_and(|t| ["FROM", "JOIN", "UPDATE", "INTO"].iter().any(|k| t.is(k)));
        let mut context = Self {
            replacement,
            prefix,
            qualifier,
            tokens,
            parents,
            sources: vec![],
            scope,
            relation_position,
            case_insensitive: false,
        };
        context.collect_sources();
        Some(context)
    }

    pub fn case_insensitive_identifiers(mut self, enabled: bool) -> Self {
        self.case_insensitive = enabled;
        self
    }

    fn same_name(&self, a: &str, b: &str) -> bool {
        if self.case_insensitive {
            a.eq_ignore_ascii_case(b)
        } else {
            a == b
        }
    }

    fn collect_sources(&mut self) {
        let t = &self.tokens;
        for i in 0..t.len() {
            // WITH name [(columns)] AS (query), including comma-separated CTEs.
            if t[i].identifier()
                && (i > 0 && (t[i - 1].is("WITH") || t[i - 1].is("RECURSIVE") || t[i - 1].is(",")))
            {
                let mut j = i + 1;
                let mut columns = vec![];
                if t.get(j).is_some_and(|t| t.is("(")) {
                    j += 1;
                    while j < t.len() && !t[j].is(")") {
                        if t[j].identifier() {
                            columns.push(t[j].name());
                        }
                        j += 1;
                    }
                    j += 1;
                }
                if t.get(j).is_some_and(|t| t.is("AS")) && t.get(j + 1).is_some_and(|t| t.is("(")) {
                    self.sources.push(Source {
                        scope: t[i].scope,
                        alias: t[i].name(),
                        table: t[i].name(),
                        schema: None,
                        derived: t.get(j + 2).map(|t| t.scope),
                        columns,
                        cte: true,
                        lateral: false,
                    });
                }
            }
            let source_start = ["FROM", "JOIN", "UPDATE", "INTO"]
                .iter()
                .any(|k| t[i].is(k))
                || (t[i].is(",")
                    && t[..i]
                        .iter()
                        .rev()
                        .find(|x| {
                            x.scope == t[i].scope
                                && ["FROM", "WHERE", "SELECT", "ON", "GROUP", "ORDER"]
                                    .iter()
                                    .any(|k| x.is(k))
                        })
                        .is_some_and(|x| x.is("FROM")));
            if !source_start {
                continue;
            }
            let mut j = i + 1;
            let lateral = t.get(j).is_some_and(|t| t.is("LATERAL"));
            if lateral {
                j += 1;
            }
            let Some(first) = t.get(j) else {
                continue;
            };
            let mut derived = None;
            let mut schema = None;
            let table;
            if first.is("(") {
                derived = t.get(j + 1).map(|t| t.scope);
                table = String::new();
                j += 1;
                while j < t.len() && !(t[j].is(")") && t[j].scope == first.scope) {
                    j += 1;
                }
                j += 1;
            } else if first.identifier() {
                table = if t.get(j + 1).is_some_and(|t| t.is("."))
                    && t.get(j + 2).is_some_and(|t| t.identifier())
                {
                    schema = Some(first.name());
                    j += 2;
                    t[j].name()
                } else {
                    first.name()
                };
                j += 1;
            } else {
                continue;
            }
            if t.get(j).is_some_and(|t| t.is("AS")) {
                j += 1;
            }
            let alias = t
                .get(j)
                .filter(|t| {
                    t.identifier() && (t.quoted || !KEYWORDS.split_whitespace().any(|k| t.is(k)))
                })
                .map(|t| t.name())
                .unwrap_or_else(|| table.clone());
            self.sources.push(Source {
                scope: t[i].scope,
                alias,
                schema,
                table,
                derived,
                columns: vec![],
                cte: false,
                lateral,
            });
        }
    }

    fn ancestors(&self, mut scope: usize) -> Vec<usize> {
        let mut result = vec![scope];
        while scope != 0 {
            scope = self.parents[scope];
            result.push(scope);
        }
        result
    }

    fn visible_sources(&self, scope: usize) -> Vec<&Source> {
        let mut seen = HashSet::new();
        let mut result = vec![];
        for parent in self.ancestors(scope) {
            for source in self.sources.iter().filter(|s| s.scope == parent && !s.cte) {
                let name = if self.case_insensitive {
                    source.alias.to_ascii_lowercase()
                } else {
                    source.alias.clone()
                };
                if seen.insert(name) {
                    result.push(source);
                }
            }
            if self
                .sources
                .iter()
                .any(|s| s.derived == Some(parent) && (s.cte || !s.lateral))
            {
                break;
            }
        }
        result
    }

    fn cte(&self, source: &Source) -> Option<&Source> {
        if source.schema.is_some() {
            return None;
        }
        for scope in self.ancestors(source.scope) {
            if let Some(cte) = self
                .sources
                .iter()
                .find(|s| s.cte && s.scope == scope && self.same_name(&s.alias, &source.table))
            {
                return Some(cte);
            }
        }
        None
    }

    fn columns(&self, source: &Source, catalog: &Catalog, depth: usize) -> Vec<Column> {
        if depth > 8 {
            return vec![];
        }
        if !source.columns.is_empty() {
            return source
                .columns
                .iter()
                .map(|n| Column {
                    name: n.clone(),
                    detail: "CTE column".into(),
                })
                .collect();
        }
        if let Some(scope) = source.derived {
            return self.projection(scope, catalog, depth + 1);
        }
        if let Some(cte) = self.cte(source) {
            return self.columns(cte, catalog, depth + 1);
        }
        catalog
            .relations
            .iter()
            .find(|r| {
                self.same_name(&r.name, &source.table)
                    && source
                        .schema
                        .as_ref()
                        .map_or(r.visible, |s| self.same_name(s, &r.schema))
            })
            .and_then(|r| r.columns.clone())
            .unwrap_or_default()
    }

    fn projection(&self, scope: usize, catalog: &Catalog, depth: usize) -> Vec<Column> {
        let Some(start) = self
            .tokens
            .iter()
            .position(|t| t.scope == scope && t.is("SELECT"))
        else {
            return vec![];
        };
        let tail = &self.tokens[start + 1..];
        let end = tail
            .iter()
            .position(|t| {
                (t.scope == scope && t.is("FROM")) || (t.is(")") && t.scope == self.parents[scope])
            })
            .unwrap_or(tail.len());
        let mut result = vec![];
        for expression in tail[..end].split(|t| t.scope == scope && t.is(",")) {
            if expression.iter().any(|t| t.scope == scope && t.is("*")) {
                let qualifier = expression
                    .iter()
                    .position(|t| t.is("."))
                    .and_then(|i| i.checked_sub(1).map(|i| expression[i].name()));
                for source in self.visible_sources(scope).into_iter().filter(|s| {
                    s.scope == scope
                        && qualifier
                            .as_ref()
                            .is_none_or(|q| self.same_name(q, &s.alias))
                }) {
                    result.extend(self.columns(source, catalog, depth + 1));
                }
            } else if let Some(last) = expression
                .last()
                .filter(|t| t.identifier() && t.scope == scope)
            {
                let value = if let Some(alias) = expression
                    .iter()
                    .position(|t| t.scope == scope && t.is("AS"))
                {
                    &expression[..alias]
                } else if expression.len() >= 2 && expression[expression.len() - 2].identifier() {
                    &expression[..expression.len() - 1]
                } else {
                    expression
                };
                let direct = value.len() == 1 || (value.len() == 3 && value[1].is("."));
                let detail = if direct {
                    value.last().and_then(|column| {
                        self.visible_sources(scope)
                            .into_iter()
                            .filter(|s| {
                                value.len() == 1 || self.same_name(&s.alias, &value[0].name())
                            })
                            .flat_map(|s| self.columns(s, catalog, depth + 1))
                            .find(|c| self.same_name(&c.name, &column.name()))
                            .map(|c| c.detail)
                    })
                } else {
                    None
                };
                result.push(Column {
                    name: last.name(),
                    detail: detail.unwrap_or_else(|| "Query output".into()),
                });
            }
        }
        result
    }

    fn collect_wanted(
        &self,
        source: &Source,
        catalog: &Catalog,
        depth: usize,
        wanted: &mut Vec<(String, String)>,
    ) {
        if depth > 8 || wanted.len() >= 16 {
            return;
        }
        if let Some(scope) = source.derived {
            for inner in self
                .visible_sources(scope)
                .into_iter()
                .filter(|s| s.scope == scope)
            {
                self.collect_wanted(inner, catalog, depth + 1, wanted);
            }
        } else if let Some(cte) = self.cte(source) {
            self.collect_wanted(cte, catalog, depth + 1, wanted);
        } else if let Some(relation) = catalog.relations.iter().find(|r| {
            self.same_name(&r.name, &source.table)
                && source
                    .schema
                    .as_ref()
                    .map_or(r.visible, |s| self.same_name(s, &r.schema))
        }) {
            let key = (relation.schema.clone(), relation.name.clone());
            if !wanted.contains(&key) {
                wanted.push(key);
            }
        }
    }

    pub fn wanted(&self, catalog: &Catalog) -> Vec<(String, String)> {
        // Resolve only visible sources and their derived-query dependencies.
        let mut wanted = vec![];
        for source in self.visible_sources(self.scope) {
            self.collect_wanted(source, catalog, 0, &mut wanted);
        }
        if let Some(q) = &self.qualifier
            && let Some(r) = catalog
                .relations
                .iter()
                .find(|r| r.visible && self.same_name(q, &r.name))
        {
            let key = (r.schema.clone(), r.name.clone());
            if !wanted.contains(&key) && wanted.len() < 16 {
                wanted.push(key);
            }
        }
        wanted
    }

    pub fn suggestions(&self, catalog: &Catalog) -> Vec<Suggestion> {
        let mut result = vec![];
        let mut add = |name: &str, detail: &str, keyword: bool| {
            if name.to_lowercase().starts_with(&self.prefix.to_lowercase()) {
                let insert = if keyword {
                    name.to_owned()
                } else {
                    let Ok(quoted) = crate::sql::quote_identifier(name) else {
                        return;
                    };
                    quoted
                };
                result.push(Suggestion {
                    label: name.into(),
                    insert,
                    detail: detail.into(),
                    keyword,
                });
            }
        };
        if let Some(q) = &self.qualifier {
            if let Some(source) = self
                .visible_sources(self.scope)
                .into_iter()
                .find(|s| self.same_name(&s.alias, q))
            {
                for column in self.columns(source, catalog, 0) {
                    add(&column.name, &column.detail, false);
                }
            } else {
                for relation in catalog
                    .relations
                    .iter()
                    .filter(|r| self.same_name(&r.schema, q))
                {
                    add(&relation.name, &relation.schema, false);
                }
                if let Some(relation) = catalog
                    .relations
                    .iter()
                    .find(|r| self.same_name(&r.name, q) && r.visible)
                {
                    for c in relation.columns.iter().flatten() {
                        add(&c.name, &c.detail, false);
                    }
                }
            }
        } else {
            for source in self.visible_sources(self.scope) {
                if !self.relation_position {
                    for c in self.columns(source, catalog, 0) {
                        add(&c.name, &format!("{} · {}", source.alias, c.detail), false);
                    }
                    add(&source.alias, "Table alias", false);
                }
            }
            for scope in self.ancestors(self.scope) {
                for cte in self.sources.iter().filter(|s| s.cte && s.scope == scope) {
                    add(&cte.alias, "CTE", false);
                }
            }
            for r in catalog.relations.iter().filter(|r| r.visible) {
                add(&r.name, &r.schema, false);
            }
            for s in &catalog.schemas {
                add(s, "Schema", false);
            }
            for keyword in KEYWORDS.split_whitespace() {
                add(keyword, "SQL", true);
            }
        }
        // Scope columns precede relations and keywords. Stable order makes acceptance predictable.
        let mut seen = HashSet::new();
        result.retain(|s| seen.insert(s.insert.clone()));
        result.truncate(100);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::completion::Relation;
    fn catalog() -> Catalog {
        Catalog {
            schemas: vec!["public".into()],
            relations: vec![Relation {
                schema: "public".into(),
                name: "users".into(),
                visible: true,
                columns: Some(vec![
                    Column {
                        name: "id".into(),
                        detail: "integer".into(),
                    },
                    Column {
                        name: "Mixed name".into(),
                        detail: "text · comment".into(),
                    },
                    Column {
                        name: "日本語".into(),
                        detail: "text".into(),
                    },
                    Column {
                        name: "select".into(),
                        detail: "text".into(),
                    },
                ]),
            }],
            status: String::new(),
        }
    }
    fn complete(sql: &str) -> Vec<String> {
        let offset = sql.find('|').unwrap();
        let sql = sql.replace('|', "");
        Context::at(&sql, offset)
            .map(|c| {
                c.suggestions(&catalog())
                    .into_iter()
                    .map(|s| s.label)
                    .collect()
            })
            .unwrap_or_default()
    }
    #[test]
    fn completion_aliases_incomplete_joins_and_unicode() {
        assert!(complete("SELECT u.| FROM users AS u").contains(&"id".into()));
        assert!(
            complete("SELECT u.日| FROM users u LEFT JOIN users v ON u.id = v.id")
                .contains(&"日本語".into())
        );
        assert!(complete("SELECT * FROM users u WHERE u.|").contains(&"Mixed name".into()));
        assert!(complete("SELECT v.| FROM users u, users v").contains(&"id".into()));
        assert!(complete("SELECT * FROM public.us|").contains(&"users".into()));
    }
    #[test]
    fn completion_nested_scopes_shadowing_and_statements() {
        assert!(
            complete("SELECT u.| FROM users u WHERE EXISTS (SELECT 1 FROM missing u)")
                .contains(&"id".into())
        );
        assert!(
            !complete("SELECT * FROM users u WHERE EXISTS (SELECT u.| FROM missing u)")
                .contains(&"id".into())
        );
        assert!(complete("SELECT * FROM users u WHERE EXISTS (SELECT u.|)").contains(&"id".into()));
        assert!(!complete("SELECT * FROM users u; SELECT u.|").contains(&"id".into()));
        assert!(
            !complete("SELECT v.| FROM users u WHERE EXISTS (SELECT 1 FROM users v)")
                .contains(&"id".into())
        );
    }
    #[test]
    fn completion_ctes_and_derived_projections() {
        assert!(
            complete("WITH c AS (SELECT id AS value FROM users) SELECT c.| FROM c")
                .contains(&"value".into())
        );
        assert!(
            complete("WITH c(a,b) AS (SELECT id, id FROM users) SELECT c.| FROM c")
                .contains(&"b".into())
        );
        assert!(
            complete("WITH c AS (SELECT * FROM users), d AS (SELECT * FROM c) SELECT d.| FROM d")
                .contains(&"id".into())
        );
        assert!(
            complete("SELECT q.| FROM (SELECT id AS renamed FROM users) q")
                .contains(&"renamed".into())
        );
        assert!(complete("SELECT q.| FROM (SELECT u.* FROM users u) q").contains(&"id".into()));
        assert!(
            !complete("WITH RECURSIVE c AS (SELECT * FROM c) SELECT c.| FROM c")
                .contains(&"id".into())
        );
    }
    #[test]
    fn completion_excludes_literals_and_comments() {
        for sql in [
            "SELECT 'u.|",
            "SELECT 'x''u.|'",
            "-- u.|",
            "SELECT /* outer /* inner */ u.| */",
            "DO $$ BEGIN u.| END $$",
            "DO $body$ u.| $body$",
            "SELECT E'escaped\\' u.|'",
        ] {
            assert!(complete(sql).is_empty(), "{sql}");
        }
        assert!(complete("SELECT 'ok', u.| FROM users u").contains(&"id".into()));
        assert!(complete("-- comment\nSELECT u.| FROM users u").contains(&"id".into()));
    }
    #[test]
    fn completion_replaces_entire_quoted_unicode_token() {
        for (text, offset, expected) in [
            ("SELECT u.\"Mixed na\"", 17, 9..19),
            ("SELECT u.日本語", 12, 9..18),
        ] {
            let c = Context::at(text, offset).unwrap();
            assert_eq!(c.replacement, expected);
        }
        let sql = "SELECT u.\"Mi FROM users u";
        let c = Context::at(sql, 12).unwrap();
        assert_eq!(c.replacement.start, 9);
        let c = Context::at("SELECT u.se FROM users u", 11).unwrap();
        assert_eq!(c.suggestions(&catalog())[0].insert, "\"select\"");
    }
    #[test]
    fn completion_scope_boundaries_and_dialect_case() {
        assert!(!complete("WITH c AS (SELECT u.|) SELECT * FROM users u").contains(&"id".into()));
        assert!(
            !complete("SELECT * FROM users u JOIN (SELECT u.|) q ON true").contains(&"id".into())
        );
        assert!(
            complete("SELECT * FROM users u JOIN LATERAL (SELECT u.|) q ON true")
                .contains(&"id".into())
        );
        let sql = "SELECT u. FROM \"USERS\" u";
        let context = Context::at(sql, 9).unwrap();
        assert!(context.suggestions(&catalog()).is_empty());
        let context = context.case_insensitive_identifiers(true);
        assert!(
            context
                .suggestions(&catalog())
                .iter()
                .any(|s| s.label == "id")
        );
        assert_eq!(
            context.wanted(&catalog()),
            vec![("public".into(), "users".into())]
        );
        let sql = "SELECT * FROM users; SELECT 1";
        assert!(
            Context::at(sql, sql.len())
                .unwrap()
                .wanted(&catalog())
                .is_empty()
        );
    }

    #[test]
    fn completion_bounds_large_inputs_and_results() {
        assert!(Context::at(&"x".repeat(MAX_SQL + 1), 1).is_none());
        let mut catalog = catalog();
        for i in 0..1000 {
            catalog.schemas.push(format!("schema{i}"));
        }
        assert!(Context::at("", 0).unwrap().suggestions(&catalog).len() <= 100);
    }
}
