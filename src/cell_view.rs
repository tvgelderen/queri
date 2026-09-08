//! A JSON presentation tree preserves object order, duplicate keys and scalar spelling.
use anyhow::{Result, ensure};
use serde::{
    Deserialize, Deserializer,
    de::{MapAccess, Visitor},
};
use serde_json::value::RawValue;
use std::fmt;

#[derive(Debug)]
pub enum JsonNode {
    Scalar(String),
    Array(Vec<JsonNode>),
    Object(Vec<(String, JsonNode)>),
}
struct Object(Vec<(String, Box<RawValue>)>);
impl<'de> Deserialize<'de> for Object {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        struct ObjectVisitor;
        impl<'de> Visitor<'de> for ObjectVisitor {
            type Value = Object;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a JSON object")
            }
            fn visit_map<M: MapAccess<'de>>(
                self,
                mut map: M,
            ) -> std::result::Result<Object, M::Error> {
                let mut pairs = vec![];
                while let Some(pair) = map.next_entry()? {
                    pairs.push(pair);
                }
                Ok(Object(pairs))
            }
        }
        d.deserialize_map(ObjectVisitor)
    }
}
impl JsonNode {
    pub fn parse(text: &str) -> Result<Self> {
        ensure!(
            text.len() <= 2 * 1024 * 1024,
            "JSON preview is limited to 2 MiB. The raw value remains available."
        );
        let raw: Box<RawValue> = serde_json::from_str(text)?;
        Self::parse_node(raw.get(), 0, &mut 20_000)
    }
    fn parse_node(text: &str, depth: usize, budget: &mut usize) -> Result<Self> {
        ensure!(
            depth <= 64 && *budget > 0,
            "JSON is too deeply nested or has too many nodes for a tree preview."
        );
        *budget -= 1;
        Ok(match text.as_bytes().first() {
            Some(b'{') => {
                let Object(pairs) = serde_json::from_str(text)?;
                Self::Object(
                    pairs
                        .into_iter()
                        .map(|(key, value)| {
                            Ok((key, Self::parse_node(value.get(), depth + 1, budget)?))
                        })
                        .collect::<Result<_>>()?,
                )
            }
            Some(b'[') => {
                let values: Vec<Box<RawValue>> = serde_json::from_str(text)?;
                Self::Array(
                    values
                        .into_iter()
                        .map(|value| Self::parse_node(value.get(), depth + 1, budget))
                        .collect::<Result<_>>()?,
                )
            }
            _ => Self::Scalar(text.to_owned()),
        })
    }
    pub fn formatted(&self) -> String {
        fn write(node: &JsonNode, depth: usize, out: &mut String) {
            let indent = "  ".repeat(depth);
            match node {
                JsonNode::Scalar(value) => out.push_str(value),
                JsonNode::Array(values) => {
                    out.push('[');
                    for (i, value) in values.iter().enumerate() {
                        if i > 0 {
                            out.push(',');
                        }
                        out.push('\n');
                        out.push_str(&indent);
                        out.push_str("  ");
                        write(value, depth + 1, out);
                    }
                    if !values.is_empty() {
                        out.push('\n');
                        out.push_str(&indent);
                    }
                    out.push(']');
                }
                JsonNode::Object(values) => {
                    out.push('{');
                    for (i, (key, value)) in values.iter().enumerate() {
                        if i > 0 {
                            out.push(',');
                        }
                        out.push('\n');
                        out.push_str(&indent);
                        out.push_str("  ");
                        // Serializing a string cannot fail.
                        if let Ok(key) = serde_json::to_string(key) {
                            out.push_str(&key);
                        }
                        out.push_str(": ");
                        write(value, depth + 1, out);
                    }
                    if !values.is_empty() {
                        out.push('\n');
                        out.push_str(&indent);
                    }
                    out.push('}');
                }
            }
        }
        let mut out = String::new();
        write(self, 0, &mut out);
        out
    }
}
pub fn hex_line(bytes: &[u8], line: usize) -> String {
    let offset = line.saturating_mul(16);
    let chunk = bytes
        .get(offset..bytes.len().min(offset.saturating_add(16)))
        .unwrap_or_default();
    let hex = chunk
        .iter()
        .map(|v| format!("{v:02x}"))
        .collect::<Vec<_>>()
        .join(" ");
    let ascii: String = chunk
        .iter()
        .map(|b| {
            if b.is_ascii_graphic() || *b == b' ' {
                char::from(*b)
            } else {
                '.'
            }
        })
        .collect();
    format!("{offset:08x}  {hex:47}  {ascii}")
}
/// Decode PostgreSQL's documented hex and escape bytea output for a typed cell.
pub fn postgres_binary(text: &str) -> Result<Vec<u8>> {
    if let Some(hex) = text.strip_prefix(r"\x") {
        ensure!(
            hex.len().is_multiple_of(2) && hex.is_ascii(),
            "Invalid bytea hex value."
        );
        return hex
            .as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| Ok(u8::from_str_radix(std::str::from_utf8(pair)?, 16)?))
            .collect();
    }
    ensure!(text.is_ascii(), "Invalid bytea escape value.");
    let mut bytes = text.bytes();
    let mut output = Vec::new();
    while let Some(byte) = bytes.next() {
        if byte != b'\\' {
            output.push(byte);
            continue;
        }
        match bytes.next() {
            Some(b'\\') => output.push(b'\\'),
            Some(first @ b'0'..=b'3') => {
                let second = bytes
                    .next()
                    .filter(|b| (b'0'..=b'7').contains(b))
                    .ok_or_else(|| anyhow::anyhow!("Invalid bytea escape."))?;
                let third = bytes
                    .next()
                    .filter(|b| (b'0'..=b'7').contains(b))
                    .ok_or_else(|| anyhow::anyhow!("Invalid bytea escape."))?;
                output.push((first - b'0') * 64 + (second - b'0') * 8 + (third - b'0'));
            }
            _ => anyhow::bail!("Invalid bytea escape."),
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn json_preserves_duplicate_keys_and_numeric_lexemes() {
        let raw = r#"{"x":123456789012345678901234567890,"x":1.2300e+40,"a":[null,true,"é"]}"#;
        let pretty = JsonNode::parse(raw).unwrap().formatted();
        assert!(pretty.contains("123456789012345678901234567890"));
        assert!(pretty.contains("1.2300e+40"));
        assert_eq!(pretty.matches("\"x\"").count(), 2);
        assert_eq!(JsonNode::parse(&pretty).unwrap().formatted(), pretty);
        assert!(JsonNode::parse("{broken").is_err());
    }
    #[test]
    fn postgres_binary_supports_hex_and_escape_without_changing_raw() {
        assert_eq!(postgres_binary(r"\x0041ff").unwrap(), vec![0, 65, 255]);
        assert_eq!(
            postgres_binary(r"\000A\377\\").unwrap(),
            vec![0, 65, 255, 92]
        );
        assert!(postgres_binary(r"\x0").is_err());
        assert!(postgres_binary(r"\400").is_err());
    }
    #[test]
    fn hex_preserves_offsets_and_bytes() {
        assert_eq!(
            hex_line(&[0, 65, 255], 0),
            "00000000  00 41 ff                                         .A."
        );
    }
}
