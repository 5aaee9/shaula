//! Bound recursive syntax before entering the HCL parser. This scanner extracts no values.

use shaula_core::error::CoreResult;

use super::invalid;

const MAX_NESTING: usize = 64;
const MAX_OPERATORS: usize = 64;

enum Mode<'a> {
    Code(Option<u8>),
    Quoted,
    Heredoc(&'a [u8]),
}

fn limited() -> shaula_core::error::CoreError {
    invalid("Terraform source exceeds the syntax complexity limit")
}

fn identifier(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-') || !byte.is_ascii()
}

fn heredoc(bytes: &[u8], mut at: usize) -> CoreResult<(&[u8], usize)> {
    if bytes.get(at) == Some(&b'-') {
        at += 1;
    }
    let start = at;
    while bytes.get(at).is_some_and(|byte| identifier(*byte)) {
        at += 1;
    }
    let delimiter = &bytes[start..at];
    if bytes.get(at) == Some(&b'\r') {
        at += 1;
    }
    if delimiter.is_empty() || bytes.get(at) != Some(&b'\n') {
        return Err(invalid("Terraform source contains invalid HCL"));
    }
    Ok((delimiter, at + 1))
}

fn heredoc_end(bytes: &[u8], at: usize, delimiter: &[u8]) -> Option<usize> {
    if at != 0 && bytes.get(at - 1) != Some(&b'\n') {
        return None;
    }
    let mut start = at;
    while bytes
        .get(start)
        .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
    {
        start += 1;
    }
    let end = start + delimiter.len();
    (bytes.get(start..end) == Some(delimiter)
        && !bytes.get(end).is_some_and(|byte| identifier(*byte)))
    .then_some(end)
}

pub(super) fn check(text: &str) -> CoreResult<()> {
    let bytes = text.as_bytes();
    let mut modes = vec![Mode::Code(None)];
    let mut operators = 0;
    let mut at = 0;
    while let Some(mode) = modes.last() {
        if modes.len() > MAX_NESTING || operators > MAX_OPERATORS {
            return Err(limited());
        }
        if at >= bytes.len() {
            break;
        }
        let rest = &bytes[at..];
        match mode {
            Mode::Code(end) => {
                if rest.starts_with(b"//") || rest.starts_with(b"#") {
                    at += rest
                        .iter()
                        .position(|byte| *byte == b'\n')
                        .unwrap_or(rest.len());
                    continue;
                }
                if rest.starts_with(b"/*") {
                    let length = rest
                        .windows(2)
                        .position(|pair| pair == b"*/")
                        .ok_or_else(|| invalid("Terraform source contains invalid HCL"))?;
                    at += length + 2;
                    continue;
                }
                if rest.starts_with(b"<<") {
                    let (delimiter, next) = heredoc(bytes, at + 2)?;
                    modes.push(Mode::Heredoc(delimiter));
                    at = next;
                    continue;
                }
                match bytes[at] {
                    b'"' => modes.push(Mode::Quoted),
                    b'(' => modes.push(Mode::Code(Some(b')'))),
                    b'[' => modes.push(Mode::Code(Some(b']'))),
                    b'{' => modes.push(Mode::Code(Some(b'}'))),
                    b')' | b']' | b'}' if Some(bytes[at]) == *end => {
                        modes.pop();
                    }
                    b')' | b']' | b'}' => {
                        return Err(invalid("Terraform source contains invalid HCL"));
                    }
                    // Unary/binary/conditional chains and template directives also
                    // build recursive AST nodes without increasing bracket depth.
                    b'!' | b'?' | b'+' | b'-' | b'*' | b'/' | b'%' | b'&' | b'|' | b'<' | b'>' => {
                        operators += 1;
                    }
                    b'=' if rest.starts_with(b"==") => operators += 1,
                    _ => {}
                }
            }
            Mode::Quoted | Mode::Heredoc(_) => {
                if let Mode::Heredoc(delimiter) = mode {
                    if let Some(end) = heredoc_end(bytes, at, delimiter) {
                        modes.pop();
                        at = end;
                        continue;
                    }
                } else if bytes[at] == b'\\' {
                    at += 2;
                    continue;
                } else if bytes[at] == b'"' {
                    modes.pop();
                    at += 1;
                    continue;
                }
                if rest.starts_with(b"$${") || rest.starts_with(b"%%{") {
                    at += 3;
                    continue;
                }
                if rest.starts_with(b"${") || rest.starts_with(b"%{") {
                    operators += 1;
                    modes.push(Mode::Code(Some(b'}')));
                    at += 2;
                    continue;
                }
            }
        }
        at += 1;
    }
    Ok(())
}
