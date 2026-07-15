// Minimal ANSI escape-sequence stripper for `read_output` (raw=false).
//
// Removes:
// - CSI sequences: ESC '[' <params 0x30-0x3F> <intermediates 0x20-0x2F> <final 0x40-0x7E>
// - OSC sequences: ESC ']' ... terminated by BEL (0x07) or ST (ESC '\')
// - other ESC sequences: ESC + optional intermediates (0x20-0x2F) + one final byte
//
// Plain text (including \n, \r, \t) is passed through untouched.

/// Strip ANSI/VT escape sequences from `input`.
pub fn strip_ansi(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] != 0x1b {
            out.push(bytes[i]);
            i += 1;
            continue;
        }

        // ESC at end of input: drop it.
        if i + 1 >= bytes.len() {
            break;
        }

        match bytes[i + 1] {
            // CSI: ESC [ params... intermediates... final(0x40-0x7E)
            b'[' => {
                i += 2;
                while i < bytes.len() && (0x30..=0x3f).contains(&bytes[i]) {
                    i += 1; // parameter bytes
                }
                while i < bytes.len() && (0x20..=0x2f).contains(&bytes[i]) {
                    i += 1; // intermediate bytes
                }
                if i < bytes.len() {
                    i += 1; // final byte
                }
            }
            // OSC: ESC ] ... (BEL | ESC \)
            b']' => {
                i += 2;
                while i < bytes.len() {
                    if bytes[i] == 0x07 {
                        i += 1;
                        break;
                    }
                    if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'\\' {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
            }
            // Everything else: ESC + optional intermediates + one final byte
            // (covers e.g. ESC ( B charset designation, ESC 7 / ESC 8, ESC = etc.)
            _ => {
                i += 1;
                while i < bytes.len() && (0x20..=0x2f).contains(&bytes[i]) {
                    i += 1;
                }
                if i < bytes.len() {
                    i += 1;
                }
            }
        }
    }

    String::from_utf8_lossy(&out).to_string()
}

/// Fold carriage-return overwrites into the final visible state, with true
/// terminal overlay semantics: `\r` moves the cursor to column 0 and the
/// following segment OVERWRITES the previous content cell by cell — a
/// shorter overwrite leaves the tail of the longer earlier text visible.
/// Applied per `\n`-separated line; `\r\n` folds away naturally (the empty
/// segment after the final `\r` overwrites nothing).
pub fn fold_cr(text: &str) -> String {
    text.split('\n')
        .map(fold_cr_line)
        .collect::<Vec<_>>()
        .join("\n")
}

fn fold_cr_line(line: &str) -> String {
    if !line.contains('\r') {
        return line.to_string();
    }
    let mut cells: Vec<char> = Vec::new();
    for segment in line.split('\r') {
        for (i, ch) in segment.chars().enumerate() {
            if i < cells.len() {
                cells[i] = ch;
            } else {
                cells.push(ch);
            }
        }
    }
    cells.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_csi_colors() {
        assert_eq!(strip_ansi("\x1b[31mred\x1b[0m plain"), "red plain");
        assert_eq!(strip_ansi("\x1b[1;38;5;208mbold orange\x1b[m"), "bold orange");
    }

    #[test]
    fn strips_csi_cursor_moves() {
        assert_eq!(strip_ansi("\x1b[2J\x1b[Hcleared"), "cleared");
        assert_eq!(strip_ansi("a\x1b[10;20Hb\x1b[1Ac\x1b[?25ld"), "abcd");
    }

    #[test]
    fn strips_osc_title() {
        // BEL-terminated
        assert_eq!(strip_ansi("\x1b]0;my title\x07after"), "after");
        // ST (ESC \)-terminated
        assert_eq!(strip_ansi("\x1b]2;another\x1b\\after"), "after");
    }

    #[test]
    fn strips_lone_esc_sequences() {
        assert_eq!(strip_ansi("x\x1b(By"), "xy"); // charset designation
        assert_eq!(strip_ansi("x\x1b7y\x1b8z"), "xyz"); // save/restore cursor
        assert_eq!(strip_ansi("x\x1b=y"), "xy"); // keypad mode
        assert_eq!(strip_ansi("trailing\x1b"), "trailing"); // ESC at EOF
    }

    #[test]
    fn mixed_text_and_newlines_survive() {
        let input = "\x1b[32m$ \x1b[0mls -la\r\n\x1b[1mtotal 8\x1b[0m\nfile.txt\r\n\x1b]0;bash\x07";
        assert_eq!(strip_ansi(input), "$ ls -la\r\ntotal 8\nfile.txt\r\n");
    }

    #[test]
    fn plain_text_untouched() {
        let s = "no escapes here\nline2\ttabbed 日本語";
        assert_eq!(strip_ansi(s), s);
    }

    // ----- CR overlay folding -----

    #[test]
    fn fold_cr_spinner_sequence() {
        assert_eq!(
            fold_cr("Working.\rWorking..\rWorking..."),
            "Working..."
        );
    }

    #[test]
    fn fold_cr_shorter_overwrite_keeps_tail() {
        // Progress bar overwritten by a SHORTER segment: the tail of the
        // longer earlier text stays visible (overlay, not truncate).
        assert_eq!(fold_cr("[#####     ] 50%\rdone"), "done##     ] 50%");
        assert_eq!(fold_cr("abcdefgh\r123"), "123defgh");
    }

    #[test]
    fn fold_cr_plain_lines_untouched() {
        let s = "line one\nline two\nline three";
        assert_eq!(fold_cr(s), s);
        assert_eq!(fold_cr(""), "");
    }

    #[test]
    fn fold_cr_crlf_and_multiline() {
        // \r\n: the empty segment after \r overwrites nothing.
        assert_eq!(fold_cr("hello\r\nworld\r\n"), "hello\nworld\n");
        // folding is per line
        assert_eq!(fold_cr("a\rb\nc.\rc..\nplain"), "b\nc..\nplain");
    }

    #[test]
    fn strip_then_fold_typical_tui_output() {
        let raw = "\x1b[2K\x1b[36m⠋\x1b[0m Working.\r\x1b[2K\x1b[36m⠙\x1b[0m Working..\r\x1b[2K\x1b[36m⠹\x1b[0m Working...\ndone\r\n";
        let cleaned = fold_cr(&strip_ansi(raw));
        assert_eq!(cleaned, "⠹ Working...\ndone\n");
    }
}
