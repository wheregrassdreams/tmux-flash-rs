use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::process::Command;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use tmux_flash::{Match, Search};

const RESET: &str = "\x1b[0m";
const DEFAULT_LABELS: &str = "asdfghjklqwertyuiopzxcvbnm";

#[derive(Clone)]
struct Colors {
    backdrop: String,
    match_fg: String,
    match_bg: String,
    current_fg: String,
    current_bg: String,
    label_fg: String,
    label_bg: String,
    status: String,
    status_error: String,
}

impl Default for Colors {
    fn default() -> Self {
        Self {
            backdrop: "#62666f".into(),
            match_fg: "#f8fafc".into(),
            match_bg: "#385d8a".into(),
            current_fg: "#ffffff".into(),
            current_bg: "#23734f".into(),
            label_fg: "#17131d".into(),
            label_bg: "#c99af5".into(),
            status: "#b8bdc7".into(),
            status_error: "#ff6b75".into(),
        }
    }
}

struct Options {
    original: String,
    capture: String,
    labels: String,
    cursor_x: usize,
    cursor_y: usize,
    width: usize,
    height: usize,
    colors: Colors,
}

fn parse_options() -> Result<Options, String> {
    let mut options = Options {
        original: String::new(),
        capture: String::new(),
        labels: DEFAULT_LABELS.into(),
        cursor_x: 0,
        cursor_y: 0,
        width: 0,
        height: 0,
        colors: Colors::default(),
    };
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--help" || arg == "-h" {
            return Err(usage().into());
        }
        let value = args
            .next()
            .ok_or_else(|| format!("missing value for {arg}"))?;
        match arg.as_str() {
            "--orig" => options.original = value,
            "--capture" => options.capture = value,
            "--labels" => options.labels = value,
            "--cx" => options.cursor_x = parse_number(&arg, &value)?,
            "--cy" => options.cursor_y = parse_number(&arg, &value)?,
            "--width" => options.width = parse_number(&arg, &value)?,
            "--height" => options.height = parse_number(&arg, &value)?,
            "--color-backdrop" => options.colors.backdrop = value,
            "--color-match-fg" => options.colors.match_fg = value,
            "--color-match-bg" => options.colors.match_bg = value,
            "--color-current-fg" => options.colors.current_fg = value,
            "--color-current-bg" => options.colors.current_bg = value,
            "--color-label-fg" => options.colors.label_fg = value,
            "--color-label-bg" => options.colors.label_bg = value,
            "--color-status" => options.colors.status = value,
            "--color-status-error" => options.colors.status_error = value,
            _ => return Err(format!("unknown option {arg}\n{}", usage())),
        }
    }
    if options.original.is_empty()
        || options.capture.is_empty()
        || options.width == 0
        || options.height < 2
    {
        return Err(format!(
            "missing required pane geometry or capture arguments\n{}",
            usage()
        ));
    }
    Ok(options)
}

fn usage() -> &'static str {
    "usage: tmux-flash --orig PANE --capture FILE --width COLS --height ROWS [--cx N --cy N --labels ALPHABET]"
}

fn parse_number(name: &str, value: &str) -> Result<usize, String> {
    value
        .parse()
        .map_err(|_| format!("invalid value for {name}: {value}"))
}

fn rgb(color: &str, foreground: bool) -> String {
    let value = color.strip_prefix('#').unwrap_or(color);
    if value.len() != 6 {
        return String::new();
    }
    let Ok(red) = u8::from_str_radix(&value[0..2], 16) else {
        return String::new();
    };
    let Ok(green) = u8::from_str_radix(&value[2..4], 16) else {
        return String::new();
    };
    let Ok(blue) = u8::from_str_radix(&value[4..6], 16) else {
        return String::new();
    };
    format!(
        ";{};2;{red};{green};{blue}",
        if foreground { 38 } else { 48 }
    )
}

fn sgr(fg: &str, bg: &str, bold: bool) -> String {
    let mut parts = vec!["0".to_owned()];
    if bold {
        parts.push("1".into());
    }
    for color in [rgb(fg, true), rgb(bg, false)] {
        if !color.is_empty() {
            parts.push(color.trim_start_matches(';').to_owned());
        }
    }
    format!("\x1b[{}m", parts.join(";"))
}

fn tmux(args: &[&str]) -> io::Result<String> {
    let output = Command::new("tmux").args(args).output()?;
    if !output.status.success() {
        return Err(io::Error::other(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

struct PaneRestore {
    me: String,
    original: String,
    restored: bool,
}

impl PaneRestore {
    fn restore(&mut self) {
        if self.restored {
            return;
        }
        self.restored = true;
        let _ = tmux(&["swap-pane", "-s", &self.me, "-t", &self.original]);
        let _ = tmux(&["select-pane", "-t", &self.original]);
    }
}

impl Drop for PaneRestore {
    fn drop(&mut self) {
        self.restore();
    }
}

struct CursorRestore;

impl Drop for CursorRestore {
    fn drop(&mut self) {
        let _ = io::stdout().write_all(format!("{RESET}\x1b[?25h").as_bytes());
    }
}

fn jump(original: &str, row: usize, grapheme_index: usize) -> io::Result<()> {
    if tmux(&["display-message", "-p", "-t", original, "#{pane_in_mode}"])? != "1" {
        return Ok(());
    }
    tmux(&["send-keys", "-X", "-t", original, "top-line"])?;
    tmux(&["send-keys", "-X", "-t", original, "start-of-line"])?;
    if row > 0 {
        tmux(&[
            "send-keys",
            "-X",
            "-t",
            original,
            "-N",
            &row.to_string(),
            "cursor-down",
        ])?;
        tmux(&["send-keys", "-X", "-t", original, "start-of-line"])?;
    }
    if grapheme_index > 0 {
        tmux(&[
            "send-keys",
            "-X",
            "-t",
            original,
            "-N",
            &grapheme_index.to_string(),
            "cursor-right",
        ])?;
    }
    Ok(())
}

struct RawMode {
    fd: libc::c_int,
    previous: libc::termios,
}

impl RawMode {
    fn enable(fd: libc::c_int) -> io::Result<Self> {
        let mut previous = unsafe { std::mem::zeroed::<libc::termios>() };
        if unsafe { libc::tcgetattr(fd, &mut previous) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let mut raw = previous;
        raw.c_iflag &= !(libc::BRKINT | libc::ICRNL | libc::INPCK | libc::ISTRIP | libc::IXON);
        raw.c_oflag &= !libc::OPOST;
        raw.c_cflag |= libc::CS8;
        raw.c_lflag &= !(libc::ECHO | libc::ICANON | libc::IEXTEN | libc::ISIG);
        raw.c_cc[libc::VMIN] = 1;
        raw.c_cc[libc::VTIME] = 0;
        if unsafe { libc::tcsetattr(fd, libc::TCSAFLUSH, &raw) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self { fd, previous })
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        unsafe {
            libc::tcsetattr(self.fd, libc::TCSAFLUSH, &self.previous);
        }
    }
}

fn read_char(input: &mut impl Read) -> io::Result<Option<char>> {
    let mut first = [0; 1];
    if input.read(&mut first)? == 0 {
        return Ok(None);
    }
    let size = match first[0] {
        0x00..=0x7f => 1,
        0xc2..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf4 => 4,
        _ => return Ok(None),
    };
    let mut bytes = vec![first[0]];
    while bytes.len() < size {
        let mut next = [0; 1];
        if input.read(&mut next)? == 0 {
            return Ok(None);
        }
        bytes.push(next[0]);
    }
    Ok(std::str::from_utf8(&bytes)
        .ok()
        .and_then(|s| s.chars().next()))
}

fn render(
    search: &Search,
    matches: &[Match],
    labels: &[(char, Match)],
    options: &Options,
) -> String {
    let colors = &options.colors;
    let backdrop = sgr(&colors.backdrop, "", false);
    let match_style = sgr(&colors.match_fg, &colors.match_bg, false);
    let current_style = sgr(&colors.current_fg, &colors.current_bg, false);
    let label_style = sgr(&colors.label_fg, &colors.label_bg, true);
    let status_color = if matches.is_empty() && !search.pattern.is_empty() {
        &colors.status_error
    } else {
        &colors.status
    };
    let status_style = sgr(
        status_color,
        "",
        matches.is_empty() && !search.pattern.is_empty(),
    );

    let mut cells: HashMap<(usize, usize), &'static str> = HashMap::new();
    for (index, found) in matches.iter().enumerate() {
        for grapheme in found.start..found.end {
            cells.insert(
                (found.row, grapheme),
                if index == 0 { "current" } else { "match" },
            );
        }
    }
    let mut label_at = HashMap::new();
    for (label, found) in labels {
        label_at.insert((found.row, found.end), *label);
    }

    let mut output = String::from("\x1b[H\x1b[2J");
    output.push_str("\x1b[?25l");
    for (row, line) in search
        .lines
        .iter()
        .take(options.height.saturating_sub(1))
        .enumerate()
    {
        output.push_str(&format!("\x1b[{};1H", row + 1));
        let mut column = 0;
        let mut previous = "";
        for (index, cluster) in line.graphemes(true).enumerate() {
            let width = UnicodeWidthStr::width(cluster);
            if width > 0 && column + width > options.width {
                break;
            }
            let style = match cells.get(&(row, index)) {
                Some(&"current") => current_style.as_str(),
                Some(&"match") => match_style.as_str(),
                _ => backdrop.as_str(),
            };
            if style != previous {
                output.push_str(style);
                previous = style;
            }
            output.push_str(cluster);
            column += width;
            if let Some(label) = label_at.get(&(row, index + 1))
                && column < options.width
            {
                if previous != label_style {
                    output.push_str(&label_style);
                    previous = &label_style;
                }
                output.push(*label);
                column += 1;
            }
        }
        output.push_str(RESET);
    }
    let count = if search.pattern.is_empty() {
        String::new()
    } else {
        format!(" [{}]", matches.len())
    };
    output.push_str(&format!(
        "\x1b[{};1H{} flash> {}{}{}",
        options.height, status_style, search.pattern, count, RESET
    ));
    output
}

fn run(options: Options) -> Result<(), Box<dyn std::error::Error>> {
    let me = env::var("TMUX_PANE").map_err(|_| "TMUX_PANE is not set")?;
    let mut panes = PaneRestore {
        me,
        original: options.original.clone(),
        restored: false,
    };
    let capture = fs::read_to_string(&options.capture)?;
    let _ = fs::remove_file(&options.capture);
    let lines: Vec<String> = capture
        .trim_end_matches('\n')
        .split('\n')
        .map(str::to_owned)
        .take(options.height)
        .collect();
    let mut search = Search {
        lines,
        cursor_x: options.cursor_x,
        cursor_y: options.cursor_y,
        ..Search::default()
    };
    let stdin = io::stdin();
    let _raw = RawMode::enable(stdin.as_raw_fd())?;
    let _cursor = CursorRestore;
    let mut input = stdin.lock();
    let mut stdout = io::stdout().lock();

    loop {
        let matches = search.find_matches();
        let labels = if search.pattern.is_empty() {
            Vec::new()
        } else {
            search.assign_labels(&matches, &options.labels)
        };
        stdout.write_all(render(&search, &matches, &labels, &options).as_bytes())?;
        stdout.write_all(b"\x1b[?25l")?;
        stdout.flush()?;

        let Some(ch) = read_char(&mut input)? else {
            panes.restore();
            return Ok(());
        };
        match ch {
            '\u{1b}' | '\u{3}' => {
                panes.restore();
                return Ok(());
            }
            '\r' | '\n' => {
                if let Some(found) = matches.first() {
                    panes.restore();
                    jump(&options.original, found.row, found.start)?;
                } else {
                    panes.restore();
                }
                return Ok(());
            }
            '\u{7f}' | '\u{8}' => {
                let mut graphemes = search.pattern.graphemes(true).collect::<Vec<_>>();
                graphemes.pop();
                search.pattern = graphemes.concat();
            }
            _ => {
                if let Some((_, found)) = labels.iter().find(|(label, _)| *label == ch) {
                    panes.restore();
                    jump(&options.original, found.row, found.start)?;
                    return Ok(());
                }
                if !ch.is_control() {
                    search.pattern.push(ch);
                }
            }
        }
    }
}

fn main() {
    let options = match parse_options() {
        Ok(options) => options,
        Err(error) => {
            eprintln!("tmux-flash: {error}");
            std::process::exit(2);
        }
    };
    if let Err(error) = run(options) {
        eprintln!("tmux-flash: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_options(width: usize) -> Options {
        Options {
            original: "%1".into(),
            capture: String::new(),
            labels: "a".into(),
            cursor_x: 0,
            cursor_y: 0,
            width,
            height: 4,
            colors: Colors::default(),
        }
    }

    #[test]
    fn renders_one_label_after_a_match_at_end_of_line() {
        let search = Search {
            lines: vec!["x中".into()],
            pattern: "中".into(),
            ..Search::default()
        };
        let matches = search.find_matches();
        let labels = vec![("@".chars().next().unwrap(), matches[0].clone())];
        let frame = render(&search, &matches, &labels, &test_options(8));
        assert_eq!(frame.matches('@').count(), 1);
    }

    #[test]
    fn rendered_wide_graphemes_respect_the_terminal_cell_limit() {
        let search = Search {
            lines: vec!["A中🌈B".into()],
            ..Search::default()
        };
        let frame = render(&search, &[], &[], &test_options(5));
        assert!(frame.contains("A中🌈"));
        assert!(!frame.contains("A中🌈B"));

        let labels = vec![(
            "@".chars().next().unwrap(),
            Match {
                row: 0,
                start: 2,
                end: 3,
            },
        )];
        let frame = render(&search, &[], &labels, &test_options(5));
        assert!(!frame.contains('@'));
    }
}
