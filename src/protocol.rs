use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecResult {
    pub host: String,
    pub command_id: String,
    pub raw_log_path: PathBuf,
    pub visible_output: String,
    pub exit_code: ExitCodeState,
    pub transport: TransportState,
    pub parser: ParserState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExitCodeState {
    Code(i32),
    TimedOut,
    Interrupted,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportState {
    Ok,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParserState {
    Ok,
    MissingExitMarker,
    InvalidExitCode(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedOutput {
    pub visible_output: String,
    pub exit_code: ExitCodeState,
    pub parser: ParserState,
}

pub fn exit_marker(nonce: &str) -> String {
    format!("__RPTY_EXIT__:{nonce}:")
}

pub fn begin_marker(nonce: &str) -> String {
    format!("__RPTY_BEGIN__:{nonce}:")
}

pub fn parse_marked_output(raw: &str, nonce: &str) -> ParsedOutput {
    let exit = exit_marker(nonce);
    let begin = begin_marker(nonce);
    let mut marker_start = None;
    let mut marker_end = None;
    let mut exit_code = None;
    let mut begin_ends = Vec::new();

    for (start, line) in raw.split_inclusive('\n').scan(0, |offset, line| {
        let start = *offset;
        *offset += line.len();
        Some((start, line))
    }) {
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if let Some(code_text) = trimmed.strip_prefix(&exit) {
            marker_start = Some(start);
            marker_end = Some(start + line.len());
            exit_code = Some(code_text.to_string());
        } else if trimmed.starts_with(&begin) {
            begin_ends.push(start + line.len());
        }
    }

    let Some(code_text) = exit_code else {
        return ParsedOutput {
            visible_output: raw.to_string(),
            exit_code: ExitCodeState::Unknown,
            parser: ParserState::MissingExitMarker,
        };
    };

    // Slice between the last begin marker preceding the exit marker and the
    // exit marker itself, so echoed input and prompts are excluded. Fall back
    // to stripping only the exit-marker line when no begin marker was seen.
    let begin_end = begin_ends
        .into_iter()
        .filter(|&end| marker_start.is_some_and(|start| end <= start))
        .next_back();
    let visible_output = extract_visible(raw, begin_end, marker_start, marker_end);

    let code = match code_text.parse::<i32>() {
        Ok(code) => code,
        Err(_) => {
            return ParsedOutput {
                visible_output,
                exit_code: ExitCodeState::Unknown,
                parser: ParserState::InvalidExitCode(code_text),
            };
        }
    };

    ParsedOutput {
        visible_output,
        exit_code: ExitCodeState::Code(code),
        parser: ParserState::Ok,
    }
}

fn extract_visible(
    raw: &str,
    begin_end: Option<usize>,
    marker_start: Option<usize>,
    marker_end: Option<usize>,
) -> String {
    match (begin_end, marker_start, marker_end) {
        (Some(begin), Some(start), _) if begin <= start => raw[begin..start].to_string(),
        (None, Some(start), Some(end)) => {
            let mut output = String::with_capacity(raw.len().saturating_sub(end - start));
            output.push_str(&raw[..start]);
            output.push_str(&raw[end..]);
            output
        }
        _ => raw.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_marked_output, ExitCodeState, ParserState};

    #[test]
    fn parses_success_exit_code() {
        let parsed = parse_marked_output("hello\n__RPTY_EXIT__:abc:0\n", "abc");
        assert_eq!(parsed.visible_output, "hello\n");
        assert_eq!(parsed.exit_code, ExitCodeState::Code(0));
        assert_eq!(parsed.parser, ParserState::Ok);
    }

    #[test]
    fn parses_nonzero_exit_code() {
        let parsed = parse_marked_output("bad\n__RPTY_EXIT__:abc:17\n", "abc");
        assert_eq!(parsed.exit_code, ExitCodeState::Code(17));
        assert_eq!(parsed.parser, ParserState::Ok);
    }

    #[test]
    fn missing_marker_is_unknown_not_success() {
        let parsed = parse_marked_output("output without marker\n", "abc");
        assert_eq!(parsed.visible_output, "output without marker\n");
        assert_eq!(parsed.exit_code, ExitCodeState::Unknown);
        assert_eq!(parsed.parser, ParserState::MissingExitMarker);
    }

    #[test]
    fn invalid_exit_code_is_parser_error() {
        let parsed = parse_marked_output("x\n__RPTY_EXIT__:abc:not-an-int\n", "abc");
        assert_eq!(parsed.visible_output, "x\n");
        assert_eq!(parsed.exit_code, ExitCodeState::Unknown);
        assert_eq!(
            parsed.parser,
            ParserState::InvalidExitCode("not-an-int".to_string())
        );
    }

    #[test]
    fn ignores_different_nonce() {
        let parsed = parse_marked_output("__RPTY_EXIT__:other:0\n", "abc");
        assert_eq!(parsed.exit_code, ExitCodeState::Unknown);
        assert_eq!(parsed.parser, ParserState::MissingExitMarker);
    }

    #[test]
    fn slices_between_begin_and_exit_markers() {
        let raw = "prompt$ . /tmp/rpty-router/x.cmd\r\n__RPTY_BEGIN__:abc:\r\nreal output\r\n__RPTY_EXIT__:abc:0\r\nprompt$ \r\n";
        let parsed = parse_marked_output(raw, "abc");
        assert_eq!(parsed.visible_output, "real output\r\n");
        assert_eq!(parsed.exit_code, ExitCodeState::Code(0));
        assert_eq!(parsed.parser, ParserState::Ok);
    }

    #[test]
    fn uses_last_begin_before_exit_marker() {
        let raw = "__RPTY_BEGIN__:abc:\nstale\n__RPTY_EXIT__:abc:1\n__RPTY_BEGIN__:abc:\nfresh\n__RPTY_EXIT__:abc:0\n";
        let parsed = parse_marked_output(raw, "abc");
        assert_eq!(parsed.visible_output, "fresh\n");
        assert_eq!(parsed.exit_code, ExitCodeState::Code(0));
    }

    #[test]
    fn uses_last_matching_marker() {
        let parsed = parse_marked_output(
            "first\n__RPTY_EXIT__:abc:1\nsecond\n__RPTY_EXIT__:abc:0\n",
            "abc",
        );
        assert_eq!(
            parsed.visible_output,
            "first\n__RPTY_EXIT__:abc:1\nsecond\n"
        );
        assert_eq!(parsed.exit_code, ExitCodeState::Code(0));
        assert_eq!(parsed.parser, ParserState::Ok);
    }
}
