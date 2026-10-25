use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::{self, Write};
use std::os::fd::AsRawFd;
use std::process::Command;
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::{Duration, Instant};
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
        let _ = tmux(&[
            "set-hook",
            "-p",
            "-u",
            "-t",
            &self.me,
            "after-copy-mode[tmux-flash-rs-guard]",
        ]);
        let _ = tmux(&[
            "set-option",
            "-p",
            "-u",
            "-t",
            &self.me,
            "@tmux-flash-active",
        ]);
        let _ = tmux(&["select-pane", "-t", &self.original]);
    }
}

impl Drop for PaneRestore {
    fn drop(&mut self) {
        self.restore();
    }
}

fn cancel_after_resize(panes: &mut PaneRestore, original: &str) {
    panes.restore();
    let _ = tmux(&[
        "display-message",
        "-d",
        "2500",
        "-t",
        original,
        "tmux-flash: pane resized; search cancelled",
    ]);
}

struct CursorRestore;

impl Drop for CursorRestore {
    fn drop(&mut self) {
        let _ = io::stdout().write_all(format!("{RESET}\x1b[?25h").as_bytes());
    }
}

fn grapheme_index_at_column(line: &str, column: usize) -> usize {
    let mut cell = 0;
    for (index, cluster) in line.graphemes(true).enumerate() {
        let width = UnicodeWidthStr::width(cluster);
        if width > 0 && column < cell + width {
            return index;
        }
        cell += width;
        if column == cell {
            return index + 1;
        }
    }
    line.graphemes(true).count()
}

fn jump(
    original: &str,
    row: usize,
    cursor_y: usize,
    line: &str,
    grapheme_index: usize,
) -> io::Result<()> {
    if tmux(&["display-message", "-p", "-t", original, "#{pane_in_mode}"])? != "1" {
        return Ok(());
    }
    if row > cursor_y {
        tmux(&[
            "send-keys",
            "-X",
            "-t",
            original,
            "-N",
            &(row - cursor_y).to_string(),
            "cursor-down",
        ])?;
    } else if cursor_y > row {
        tmux(&[
            "send-keys",
            "-X",
            "-t",
            original,
            "-N",
            &(cursor_y - row).to_string(),
            "cursor-up",
        ])?;
    }
    // `start-of-line` jumps to the start of the whole wrapped line in tmux,
    // which can move the cursor up a visual row. Keep the viewport row and
    // move horizontally from the cursor's actual column instead.
    let cursor_x = tmux(&["display-message", "-p", "-t", original, "#{copy_cursor_x}"])?
        .parse::<usize>()
        .map_err(|error| io::Error::other(format!("invalid copy cursor column: {error}")))?;
    let cursor_index = grapheme_index_at_column(line, cursor_x);
    if grapheme_index > cursor_index {
        tmux(&[
            "send-keys",
            "-X",
            "-t",
            original,
            "-N",
            &(grapheme_index - cursor_index).to_string(),
            "cursor-right",
        ])?;
    } else if cursor_index > grapheme_index {
        tmux(&[
            "send-keys",
            "-X",
            "-t",
            original,
            "-N",
            &(cursor_index - grapheme_index).to_string(),
            "cursor-left",
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

static RESIZE_SIGNAL_FD: AtomicI32 = AtomicI32::new(-1);

extern "C" fn handle_terminal_resize(_: libc::c_int) {
    let fd = RESIZE_SIGNAL_FD.load(Ordering::Relaxed);
    if fd >= 0 {
        let byte = 1u8;
        unsafe {
            libc::write(fd, &byte as *const u8 as *const libc::c_void, 1);
        }
    }
}

struct ResizeSignal {
    previous: libc::sigaction,
    read_fd: libc::c_int,
    write_fd: libc::c_int,
}

impl ResizeSignal {
    fn install() -> io::Result<Self> {
        let mut pipe_fds = [-1; 2];
        if unsafe { libc::pipe(pipe_fds.as_mut_ptr()) } != 0 {
            return Err(io::Error::last_os_error());
        }
        for fd in pipe_fds {
            let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
            let fd_flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
            if flags < 0
                || fd_flags < 0
                || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
                || unsafe { libc::fcntl(fd, libc::F_SETFD, fd_flags | libc::FD_CLOEXEC) } < 0
            {
                let error = io::Error::last_os_error();
                unsafe {
                    libc::close(pipe_fds[0]);
                    libc::close(pipe_fds[1]);
                }
                return Err(error);
            }
        }

        if RESIZE_SIGNAL_FD
            .compare_exchange(-1, pipe_fds[1], Ordering::Relaxed, Ordering::Relaxed)
            .is_err()
        {
            unsafe {
                libc::close(pipe_fds[0]);
                libc::close(pipe_fds[1]);
            }
            return Err(io::Error::other(
                "resize signal handler is already installed",
            ));
        }

        let mut action = unsafe { std::mem::zeroed::<libc::sigaction>() };
        action.sa_sigaction = handle_terminal_resize as *const () as usize;
        action.sa_flags = 0;
        unsafe {
            libc::sigemptyset(&mut action.sa_mask);
            libc::sigaddset(&mut action.sa_mask, libc::SIGWINCH);
        }
        let mut previous = unsafe { std::mem::zeroed::<libc::sigaction>() };
        if unsafe { libc::sigaction(libc::SIGWINCH, &action, &mut previous) } != 0 {
            let error = io::Error::last_os_error();
            RESIZE_SIGNAL_FD.store(-1, Ordering::Relaxed);
            unsafe {
                libc::close(pipe_fds[0]);
                libc::close(pipe_fds[1]);
            }
            return Err(error);
        }
        Ok(Self {
            previous,
            read_fd: pipe_fds[0],
            write_fd: pipe_fds[1],
        })
    }
}

impl Drop for ResizeSignal {
    fn drop(&mut self) {
        unsafe {
            let mut set = std::mem::zeroed::<libc::sigset_t>();
            let mut previous_mask = std::mem::zeroed::<libc::sigset_t>();
            libc::sigemptyset(&mut set);
            libc::sigaddset(&mut set, libc::SIGWINCH);
            libc::sigprocmask(libc::SIG_BLOCK, &set, &mut previous_mask);
            libc::sigaction(libc::SIGWINCH, &self.previous, std::ptr::null_mut());
            RESIZE_SIGNAL_FD.store(-1, Ordering::Relaxed);
            libc::close(self.read_fd);
            libc::close(self.write_fd);
            libc::sigprocmask(libc::SIG_SETMASK, &previous_mask, std::ptr::null_mut());
        }
    }
}

fn terminal_size(fd: libc::c_int) -> io::Result<(usize, usize)> {
    let mut size = unsafe { std::mem::zeroed::<libc::winsize>() };
    if unsafe { libc::ioctl(fd, libc::TIOCGWINSZ, &mut size) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((size.ws_col as usize, size.ws_row as usize))
}

fn drain_resize_signal(fd: libc::c_int) -> io::Result<()> {
    let mut bytes = [0u8; 64];
    loop {
        let count = unsafe { libc::read(fd, bytes.as_mut_ptr() as *mut libc::c_void, bytes.len()) };
        if count > 0 {
            continue;
        }
        if count == 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        if error.kind() == io::ErrorKind::Interrupted {
            continue;
        }
        if error.kind() == io::ErrorKind::WouldBlock {
            return Ok(());
        }
        return Err(error);
    }
}

fn resize_signal_pending(fd: libc::c_int) -> io::Result<bool> {
    loop {
        let mut descriptor = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let ready = unsafe { libc::poll(&mut descriptor, 1, 0) };
        if ready < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
        if ready == 0 {
            return Ok(false);
        }
        drain_resize_signal(fd)?;
        return Ok(true);
    }
}

fn resize_detected(
    terminal_fd: libc::c_int,
    resize_fd: libc::c_int,
    expected: (usize, usize),
) -> io::Result<bool> {
    if resize_signal_pending(resize_fd)? {
        return Ok(true);
    }
    let size_changed = terminal_size(terminal_fd)? != expected;
    Ok(size_changed || resize_signal_pending(resize_fd)?)
}

fn wait_for_initial_size(
    fd: libc::c_int,
    resize_fd: libc::c_int,
    expected: (usize, usize),
) -> io::Result<bool> {
    let deadline = Instant::now() + Duration::from_secs(1);
    let mut stable_since = None;
    loop {
        if terminal_size(fd)? == expected {
            let stable = stable_since.get_or_insert_with(Instant::now);
            if stable.elapsed() >= Duration::from_millis(40) {
                drain_resize_signal(resize_fd)?;
                return Ok(terminal_size(fd)? == expected);
            }
        } else {
            stable_since = None;
        }
        let now = Instant::now();
        if now >= deadline {
            return Ok(false);
        }
        let remaining = deadline.saturating_duration_since(now);
        let stable_remaining =
            stable_since.map(|stable| Duration::from_millis(40).saturating_sub(stable.elapsed()));
        let timeout = stable_remaining.map_or(remaining, |stable| stable.min(remaining));
        let timeout_ms = timeout.as_millis().clamp(1, i32::MAX as u128) as i32;
        let mut descriptor = libc::pollfd {
            fd: resize_fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let ready = unsafe { libc::poll(&mut descriptor, 1, timeout_ms) };
        if ready < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
        if ready > 0 {
            drain_resize_signal(resize_fd)?;
            stable_since = None;
        }
    }
}

enum InputEvent {
    Char(char),
    Ignore,
    Eof,
    Resized,
}

enum ByteEvent {
    Byte(u8),
    Eof,
    Resized,
}

fn read_byte(fd: libc::c_int, resize_fd: libc::c_int) -> io::Result<ByteEvent> {
    loop {
        let mut descriptors = [
            libc::pollfd {
                fd: resize_fd,
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd,
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        let ready = unsafe { libc::poll(descriptors.as_mut_ptr(), descriptors.len() as _, -1) };
        if ready < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
        if descriptors[0].revents & (libc::POLLIN | libc::POLLERR | libc::POLLHUP) != 0 {
            drain_resize_signal(resize_fd)?;
            return Ok(ByteEvent::Resized);
        }
        if descriptors[1].revents & libc::POLLIN == 0 {
            continue;
        }
        let mut byte = 0;
        let count = unsafe { libc::read(fd, &mut byte as *mut u8 as *mut libc::c_void, 1) };
        if count == 0 {
            return Ok(ByteEvent::Eof);
        }
        if count < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted
                || error.kind() == io::ErrorKind::WouldBlock
            {
                continue;
            }
            return Err(error);
        }
        return Ok(ByteEvent::Byte(byte));
    }
}

fn read_char(fd: libc::c_int, resize_fd: libc::c_int) -> io::Result<InputEvent> {
    let first = match read_byte(fd, resize_fd)? {
        ByteEvent::Byte(byte) => byte,
        ByteEvent::Eof => return Ok(InputEvent::Eof),
        ByteEvent::Resized => return Ok(InputEvent::Resized),
    };
    let size = match first {
        0x00..=0x7f => 1,
        0xc2..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf4 => 4,
        _ => return Ok(InputEvent::Ignore),
    };
    let mut bytes = [0; 4];
    bytes[0] = first;
    for slot in bytes.iter_mut().take(size).skip(1) {
        match read_byte(fd, resize_fd)? {
            ByteEvent::Byte(byte) => *slot = byte,
            ByteEvent::Eof => return Ok(InputEvent::Eof),
            ByteEvent::Resized => return Ok(InputEvent::Resized),
        }
    }
    Ok(std::str::from_utf8(&bytes[..size])
        .ok()
        .and_then(|s| s.chars().next())
        .map_or(InputEvent::Ignore, InputEvent::Char))
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
    for (row, _) in search
        .iter_lines()
        .take(options.height.saturating_sub(1))
        .enumerate()
    {
        output.push_str(&format!("\x1b[{};1H", row + 1));
        let grapheme_count = search.grapheme_count(row);
        let mut column = 0;
        let mut previous = "";
        let mut index = 0;
        while index < grapheme_count {
            let cluster = search.grapheme(row, index);
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

                // A label replaces the next grapheme instead of shifting the
                // rest of the line. Preserve wide graphemes' cell width with
                // padding so following text remains in its original columns.
                if index + 1 < grapheme_count {
                    let replaced_width = UnicodeWidthStr::width(search.grapheme(row, index + 1));
                    let padding = replaced_width.saturating_sub(1);
                    let available = options.width.saturating_sub(column);
                    let padding_to_emit = padding.min(available);
                    output.push_str(&" ".repeat(padding_to_emit));
                    column += padding_to_emit;
                    index += 1;
                    if padding_to_emit < padding {
                        break;
                    }
                }
            }
            index += 1;
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
    let mut search = Search::new(lines);
    search.cursor_x = options.cursor_x;
    search.cursor_y = options.cursor_y;
    let stdin = io::stdin();
    let fd = stdin.as_raw_fd();
    let _raw = RawMode::enable(fd)?;
    let _resize_signal = ResizeSignal::install()?;
    let expected_size = (options.width, options.height);
    if !wait_for_initial_size(fd, _resize_signal.read_fd, expected_size)?
        || resize_detected(fd, _resize_signal.read_fd, expected_size)?
    {
        cancel_after_resize(&mut panes, &options.original);
        return Ok(());
    }
    let _cursor = CursorRestore;
    let mut stdout = io::stdout().lock();

    loop {
        if resize_detected(fd, _resize_signal.read_fd, expected_size)? {
            cancel_after_resize(&mut panes, &options.original);
            return Ok(());
        }
        let matches = search.find_matches();
        let labels = if search.pattern.is_empty() {
            Vec::new()
        } else {
            search.assign_labels(&matches, &options.labels)
        };
        stdout.write_all(render(&search, &matches, &labels, &options).as_bytes())?;
        stdout.write_all(b"\x1b[?25l")?;
        stdout.flush()?;

        let event = read_char(fd, _resize_signal.read_fd)?;
        if resize_detected(fd, _resize_signal.read_fd, expected_size)? {
            cancel_after_resize(&mut panes, &options.original);
            return Ok(());
        }
        let ch = match event {
            InputEvent::Char(ch) => ch,
            InputEvent::Ignore => continue,
            InputEvent::Eof => {
                panes.restore();
                return Ok(());
            }
            InputEvent::Resized => {
                cancel_after_resize(&mut panes, &options.original);
                return Ok(());
            }
        };
        match ch {
            '\u{1b}' | '\u{3}' => {
                panes.restore();
                return Ok(());
            }
            '\r' | '\n' => {
                if resize_detected(fd, _resize_signal.read_fd, expected_size)? {
                    cancel_after_resize(&mut panes, &options.original);
                    return Ok(());
                }
                if let Some(found) = matches.first() {
                    let line = search.line(found.row);
                    panes.restore();
                    jump(
                        &options.original,
                        found.row,
                        options.cursor_y,
                        line,
                        found.start,
                    )?;
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
                    if resize_detected(fd, _resize_signal.read_fd, expected_size)? {
                        cancel_after_resize(&mut panes, &options.original);
                        return Ok(());
                    }
                    let line = search.line(found.row);
                    panes.restore();
                    jump(
                        &options.original,
                        found.row,
                        options.cursor_y,
                        line,
                        found.start,
                    )?;
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
        let mut search = Search::new(vec!["x中".into()]);
        search.pattern = "中".into();
        let matches = search.find_matches();
        let labels = vec![("@".chars().next().unwrap(), matches[0].clone())];
        let frame = render(&search, &matches, &labels, &test_options(8));
        assert_eq!(frame.matches('@').count(), 1);
    }

    #[test]
    fn rendered_wide_graphemes_respect_the_terminal_cell_limit() {
        let search = Search::new(vec!["A中🌈B".into()]);
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
