//! Extraction of readable error messages from latexmk/TeX output.

const MAX_LINES: usize = 20;
const FALLBACK_TAIL: usize = 15;

/// Pick the lines of `output` most useful for explaining a failed run.
///
/// With `-file-line-error`, TeX reports errors as `file.tex:LINE: message`
/// followed by an `l.LINE` context line. Classic `! message` errors are also
/// recognised. When nothing matches, the last few non-empty lines are used.
pub fn error_excerpt(output: &str) -> Vec<String> {
    let lines: Vec<&str> = output.lines().collect();
    let mut excerpt: Vec<String> = Vec::new();

    for (index, line) in lines.iter().enumerate() {
        if !(is_file_line_error(line) || line.starts_with("! ")) || line.contains("==> Fatal error")
        {
            continue;
        }
        push_unique(&mut excerpt, line.trim_end());
        if let Some(context) = lines.get(index + 1).filter(|next| next.starts_with("l.")) {
            push_unique(&mut excerpt, context.trim_end());
        }
    }

    if excerpt.is_empty() {
        let non_empty: Vec<&str> = lines
            .iter()
            .copied()
            .filter(|l| !l.trim().is_empty())
            .collect();
        let start = non_empty.len().saturating_sub(FALLBACK_TAIL);
        excerpt = non_empty[start..]
            .iter()
            .map(|l| l.trim_end().to_string())
            .collect();
    }
    excerpt.truncate(MAX_LINES);
    excerpt
}

fn push_unique(excerpt: &mut Vec<String>, line: &str) {
    if !excerpt.iter().any(|existing| existing == line) {
        excerpt.push(line.to_string());
    }
}

/// Matches `path:NUMBER: message` without allowing spaces in the path prefix.
fn is_file_line_error(line: &str) -> bool {
    let mut parts = line.splitn(3, ':');
    let (Some(path), Some(number), Some(rest)) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    !path.is_empty()
        && !path.contains(char::is_whitespace)
        && !number.is_empty()
        && number.bytes().all(|b| b.is_ascii_digit())
        && rest.starts_with(' ')
}

#[cfg(test)]
mod tests {
    use super::error_excerpt;

    #[test]
    fn extracts_file_line_errors_with_context() {
        let output = "(./bad.tex\n./bad.tex:4: Undefined control sequence.\nl.4 \\undefinedcommand\n                     \n./bad.tex:4:  ==> Fatal error occurred, no output PDF file produced!\nTranscript written";
        assert_eq!(
            error_excerpt(output),
            vec![
                "./bad.tex:4: Undefined control sequence.",
                "l.4 \\undefinedcommand"
            ]
        );
    }

    #[test]
    fn recognises_classic_bang_errors() {
        let output = "stuff\n! Missing $ inserted.\nl.9 x_1\nmore";
        assert_eq!(
            error_excerpt(output),
            vec!["! Missing $ inserted.", "l.9 x_1"]
        );
    }

    #[test]
    fn falls_back_to_the_output_tail() {
        let output = (1..=30)
            .map(|n| format!("line {n}"))
            .collect::<Vec<_>>()
            .join("\n");
        let excerpt = error_excerpt(&output);
        assert_eq!(excerpt.len(), 15);
        assert_eq!(excerpt.last().unwrap(), "line 30");
    }

    #[test]
    fn urls_and_timestamps_are_not_mistaken_for_errors() {
        assert!(error_excerpt("see https://example.com:8080/x\nat 12:30: done").len() <= 2);
    }
}
