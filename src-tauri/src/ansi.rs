// Minimal ANSI escape-sequence stripper for `read_output` (raw=false).
//
// Removes:
// - CSI sequences: ESC '[' <params 0x30-0x3F> <intermediates 0x20-0x2F> <final 0x40-0x7E>
// - OSC sequences: ESC ']' ... terminated by BEL (0x07) or ST (ESC '\')
// - other ESC sequences: ESC + optional intermediates (0x20-0x2F) + one final byte
//
// Plain text (including \n, \r, \t) is passed through untouched.

/// Strip ANSI/VT escape sequences from `input`.
#[cfg(test)]
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
#[cfg(test)]
pub fn fold_cr(text: &str) -> String {
    text.split('\n')
        .map(fold_cr_line)
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
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

const MAX_SCROLLBACK_LINES: usize = 2_000;

#[derive(Clone)]
struct ScreenState {
    cells: Vec<Vec<char>>,
    scrollback: Vec<String>,
    row: usize,
    col: usize,
    saved: (usize, usize),
}

impl ScreenState {
    fn new(rows: usize, cols: usize) -> Self {
        Self {
            cells: vec![vec![' '; cols]; rows],
            scrollback: Vec::new(),
            row: 0,
            col: 0,
            saved: (0, 0),
        }
    }

    fn rows(&self) -> usize {
        self.cells.len()
    }

    fn cols(&self) -> usize {
        self.cells[0].len()
    }

    fn linefeed(&mut self) {
        if self.row + 1 < self.rows() {
            self.row += 1;
            return;
        }
        let line = visible_line(&self.cells[0]);
        self.scrollback.push(line);
        if self.scrollback.len() > MAX_SCROLLBACK_LINES {
            self.scrollback.remove(0);
        }
        self.cells.remove(0);
        self.cells.push(vec![' '; self.cols()]);
    }

    fn put(&mut self, ch: char) {
        if self.col >= self.cols() {
            self.col = 0;
            self.linefeed();
        }
        self.cells[self.row][self.col] = ch;
        self.col += 1;
    }

    fn clear_screen(&mut self) {
        for line in &mut self.cells {
            line.fill(' ');
        }
    }

    fn erase_display(&mut self, mode: usize) {
        match mode {
            1 => {
                for row in 0..self.row {
                    self.cells[row].fill(' ');
                }
                for col in 0..=self.col.min(self.cols() - 1) {
                    self.cells[self.row][col] = ' ';
                }
            }
            2 | 3 => {
                self.clear_screen();
                if mode == 3 {
                    self.scrollback.clear();
                }
            }
            _ => {
                for col in self.col.min(self.cols())..self.cols() {
                    self.cells[self.row][col] = ' ';
                }
                for row in self.row + 1..self.rows() {
                    self.cells[row].fill(' ');
                }
            }
        }
    }

    fn erase_line(&mut self, mode: usize) {
        match mode {
            1 => {
                for col in 0..=self.col.min(self.cols() - 1) {
                    self.cells[self.row][col] = ' ';
                }
            }
            2 => self.cells[self.row].fill(' '),
            _ => {
                for col in self.col.min(self.cols())..self.cols() {
                    self.cells[self.row][col] = ' ';
                }
            }
        }
    }
}

fn visible_line(cells: &[char]) -> String {
    cells.iter().collect::<String>().trim_end().to_string()
}

fn csi_param(params: &[usize], index: usize, default: usize) -> usize {
    params
        .get(index)
        .copied()
        .filter(|value| *value != 0)
        .unwrap_or(default)
}

fn apply_csi(state: &mut ScreenState, private: bool, params: &[usize], final_byte: char) {
    let amount = csi_param(params, 0, 1);
    match final_byte {
        'A' => state.row = state.row.saturating_sub(amount),
        'B' => state.row = (state.row + amount).min(state.rows() - 1),
        'C' => state.col = (state.col + amount).min(state.cols() - 1),
        'D' => state.col = state.col.saturating_sub(amount),
        'E' => {
            state.row = (state.row + amount).min(state.rows() - 1);
            state.col = 0;
        }
        'F' => {
            state.row = state.row.saturating_sub(amount);
            state.col = 0;
        }
        'G' => state.col = amount.saturating_sub(1).min(state.cols() - 1),
        'H' | 'f' => {
            state.row = csi_param(params, 0, 1)
                .saturating_sub(1)
                .min(state.rows() - 1);
            state.col = csi_param(params, 1, 1)
                .saturating_sub(1)
                .min(state.cols() - 1);
        }
        'J' => state.erase_display(params.first().copied().unwrap_or(0)),
        'K' => state.erase_line(params.first().copied().unwrap_or(0)),
        'd' => state.row = amount.saturating_sub(1).min(state.rows() - 1),
        's' => state.saved = (state.row, state.col),
        'u' => {
            state.row = state.saved.0.min(state.rows() - 1);
            state.col = state.saved.1.min(state.cols() - 1);
        }
        // DEC private modes are handled by the caller when they switch the
        // alternate screen. Other mode changes only affect presentation.
        'h' | 'l' if private => {}
        _ => {}
    }
}

/// Reconstruct the terminal's current textual screen and scrollback.
///
/// Unlike `strip_ansi` + `fold_cr`, this applies the cursor movement and erase
/// operations used by alternate-screen TUIs. It intentionally implements only
/// the VT operations needed to recover text; colour and other presentation
/// modes are ignored.
pub fn render_terminal(input: &str, rows: u16, cols: u16) -> String {
    let rows = usize::from(rows.max(1));
    let cols = usize::from(cols.max(1));
    let mut state = ScreenState::new(rows, cols);
    let mut main_screen: Option<ScreenState> = None;
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '\x1b' => match chars.next() {
                Some('[') => {
                    let mut body = String::new();
                    let mut final_byte = None;
                    for next in chars.by_ref() {
                        if ('@'..='~').contains(&next) {
                            final_byte = Some(next);
                            break;
                        }
                        body.push(next);
                    }
                    let private = body.starts_with('?');
                    let parameter_body = body.trim_start_matches('?');
                    let params: Vec<usize> = if parameter_body.is_empty() {
                        Vec::new()
                    } else {
                        parameter_body
                            .split(';')
                            .map(|part| part.parse().unwrap_or(0))
                            .collect()
                    };
                    if private && params.iter().any(|p| matches!(p, 47 | 1047 | 1049)) {
                        match final_byte {
                            Some('h') if main_screen.is_none() => {
                                main_screen = Some(state.clone());
                                state = ScreenState::new(rows, cols);
                            }
                            Some('l') => {
                                if let Some(main) = main_screen.take() {
                                    state = main;
                                }
                            }
                            _ => {}
                        }
                    }
                    if let Some(final_byte) = final_byte {
                        apply_csi(&mut state, private, &params, final_byte);
                    }
                }
                Some(']') => {
                    // OSC: consume through BEL or ST.
                    let mut saw_escape = false;
                    for next in chars.by_ref() {
                        if next == '\x07' {
                            break;
                        }
                        if saw_escape && next == '\\' {
                            break;
                        }
                        saw_escape = next == '\x1b';
                    }
                }
                Some('7') => state.saved = (state.row, state.col),
                Some('8') => {
                    state.row = state.saved.0.min(state.rows() - 1);
                    state.col = state.saved.1.min(state.cols() - 1);
                }
                Some('c') => state = ScreenState::new(rows, cols),
                Some(next) if (' '..='/').contains(&next) => {
                    while matches!(chars.peek(), Some(ch) if (' '..='/').contains(ch)) {
                        chars.next();
                    }
                    chars.next();
                }
                _ => {}
            },
            '\r' => state.col = 0,
            '\n' => state.linefeed(),
            '\x08' => state.col = state.col.saturating_sub(1),
            '\t' => state.col = ((state.col / 8 + 1) * 8).min(state.cols() - 1),
            ch if !ch.is_control() => state.put(ch),
            _ => {}
        }
    }

    let mut lines = state.scrollback;
    lines.extend(state.cells.iter().map(|line| visible_line(line)));
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_csi_colors() {
        assert_eq!(strip_ansi("\x1b[31mred\x1b[0m plain"), "red plain");
        assert_eq!(
            strip_ansi("\x1b[1;38;5;208mbold orange\x1b[m"),
            "bold orange"
        );
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
        assert_eq!(fold_cr("Working.\rWorking..\rWorking..."), "Working...");
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

    #[test]
    fn render_terminal_applies_full_screen_redraws() {
        let raw = "old one\r\nold two\x1b[2J\x1b[Hnew one\x1b[2;1Hnew two";
        assert_eq!(render_terminal(raw, 4, 20), "new one\nnew two");
    }

    #[test]
    fn render_terminal_overwrites_spinner_in_place() {
        let raw = "\x1b[H⠋ Thinking\x1b[H⠙ Thinking\x1b[2;1Hdone";
        assert_eq!(render_terminal(raw, 3, 20), "⠙ Thinking\ndone");
    }

    #[test]
    fn render_terminal_keeps_only_active_alternate_screen() {
        let raw = "shell\x1b[?1049h\x1b[Hagent\x1b[2;1Hready";
        assert_eq!(render_terminal(raw, 3, 20), "agent\nready");
    }

    #[test]
    fn render_terminal_restores_main_screen_after_alternate_screen() {
        let raw = "shell\x1b[?1049h\x1b[Hagent\x1b[?1049l\r\n$ ";
        assert_eq!(render_terminal(raw, 3, 20), "shell\n$");
    }
}
