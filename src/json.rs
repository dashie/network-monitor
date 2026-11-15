//! The JSON text this binary writes.
//!
//! The project ships no JSON crate: the escaping and the assembly are these functions, and
//! both the `query` renderer and the HTTP API use them, so a cause message is quoted the
//! same way wherever it leaves the process.

/// A string as a JSON value, with every character that JSON requires escaped.
pub fn string(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('"');
    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            control if (control as u32) < 0x20 => {
                escaped.push_str(&format!("\\u{:04x}", control as u32))
            }
            other => escaped.push(other),
        }
    }
    escaped.push('"');
    escaped
}

/// An optional string as a JSON value, `null` when absent.
pub fn option_string(value: &Option<String>) -> String {
    match value {
        Some(value) => string(value),
        None => "null".to_string(),
    }
}

/// An optional integer as a JSON value, `null` when absent.
pub fn option_i64(value: Option<i64>) -> String {
    match value {
        Some(value) => value.to_string(),
        None => "null".to_string(),
    }
}

/// An array of already rendered values, one per line so a diff of an answer stays readable.
pub fn array(items: Vec<String>) -> String {
    if items.is_empty() {
        "[]".to_string()
    } else {
        format!("[\n{}\n]", items.join(",\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_what_json_requires() {
        assert_eq!(string("plain"), "\"plain\"");
        assert_eq!(string("a\"b"), "\"a\\\"b\"");
        assert_eq!(string("a\\b"), "\"a\\\\b\"");
        assert_eq!(string("a\nb\tc\rd"), "\"a\\nb\\tc\\rd\"");
        assert_eq!(string("a\u{1}b"), "\"a\\u0001b\"");
    }

    #[test]
    fn an_absent_value_is_null() {
        assert_eq!(option_string(&None), "null");
        assert_eq!(option_string(&Some("eth1".to_string())), "\"eth1\"");
        assert_eq!(option_i64(None), "null");
        assert_eq!(option_i64(Some(-3)), "-3");
    }

    #[test]
    fn an_empty_array_is_one_line_and_a_filled_one_has_a_line_per_item() {
        assert_eq!(array(Vec::new()), "[]");
        assert_eq!(array(vec!["1".to_string(), "2".to_string()]), "[\n1,\n2\n]");
    }
}
