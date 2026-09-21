/// Replaces newlines that occur directly inside parentheses with carriage returns.
///
/// The lexer treats `\r` as insignificant whitespace and `\n` as a statement
/// separator. Replacing one single-byte character with another preserves byte
/// offsets, so parse error locations still refer to the original source. Braced
/// blocks nested inside parentheses retain their newlines because those newlines
/// delimit statements.
///
/// Line comments and string literals are skipped so parentheses and newlines
/// inside them do not affect layout.
pub(crate) fn normalize_parenthesized_newlines(source: &str) -> Option<String> {
    let mut bytes = source.as_bytes().to_vec();
    let mut paren_brace_depths = Vec::new();
    let mut brace_depth = 0usize;
    let mut in_line_comment = false;
    let mut in_string = false;
    let mut changed = false;
    let mut index = 0;

    while index < bytes.len() {
        let byte = bytes[index];
        if in_string {
            match byte {
                b'\\' => index += 1,
                b'"' => in_string = false,
                _ => {}
            }
            index += 1;
            continue;
        }

        match byte {
            b'\n' => {
                if paren_brace_depths.last() == Some(&brace_depth) {
                    bytes[index] = b'\r';
                    changed = true;
                }
                in_line_comment = false;
            }
            b'\r' => in_line_comment = false,
            b'/' if !in_line_comment && bytes.get(index + 1).copied() == Some(b'/') => {
                in_line_comment = true;
                index += 1;
            }
            b'"' if !in_line_comment => in_string = true,
            b'(' if !in_line_comment => paren_brace_depths.push(brace_depth),
            b')' if !in_line_comment => {
                paren_brace_depths.pop();
            }
            b'{' if !in_line_comment => brace_depth += 1,
            b'}' if !in_line_comment => brace_depth = brace_depth.saturating_sub(1),
            _ => {}
        }
        index += 1;
    }

    changed.then(|| String::from_utf8(bytes).expect("source was valid UTF-8"))
}

#[cfg(test)]
mod tests {
    use super::normalize_parenthesized_newlines;

    #[test]
    fn newline_inside_parentheses_becomes_carriage_return() {
        assert_eq!(
            normalize_parenthesized_newlines("f(\nx)"),
            Some("f(\rx)".into())
        );
    }

    #[test]
    fn newline_outside_parentheses_is_preserved() {
        assert_eq!(normalize_parenthesized_newlines("f\n(x)"), None);
    }

    #[test]
    fn parentheses_in_line_comments_do_not_affect_layout() {
        assert_eq!(
            normalize_parenthesized_newlines("f(// ) ignored\nx)\ny"),
            Some("f(// ) ignored\rx)\ny".into())
        );
    }

    #[test]
    fn parentheses_in_strings_do_not_affect_layout() {
        assert_eq!(
            normalize_parenthesized_newlines("f(\")\",\nx)"),
            Some("f(\")\",\rx)".into())
        );
    }

    #[test]
    fn newlines_inside_strings_are_preserved() {
        assert_eq!(normalize_parenthesized_newlines("\"a\nb\""), None);
        assert_eq!(normalize_parenthesized_newlines("f(\"a\nb\")"), None);
    }
}
