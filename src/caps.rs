//! What the terminal can do, asked once at startup. One write carries
//! every query and ends with DA1, which every terminal answers, so a
//! query without an answer before DA1's is unsupported. Under tmux the
//! graphics query stays home, since tmux answers DA1 itself and the
//! terminal's late answer would reach the active pane as keys; tmux
//! reports the client terminal instead, as `kitten icat` reads it.
//! GNU screen shows APC strings in its status line, so it gets nothing.
//! Sources: [graphics query][gp], [tmux answers DA1 first][tmux],
//! [`kitten icat` under tmux][icat], [Wayst on `CSI 16 t`][wayst].
//!
//! [gp]: https://sw.kovidgoyal.net/kitty/graphics-protocol/
//! [tmux]: https://github.com/tmux/tmux/issues/5530
//! [icat]: https://github.com/kovidgoyal/kitty/blob/master/tools/tui/tmux.go
//! [wayst]: https://github.com/91861/wayst/blob/master/src/vt_core.c
use crate::Result;
use crate::decode::{ModeState, Reply};
use std::fmt;
use std::io;
use std::sync::OnceLock;
use std::time::Duration;

/// Time the terminal has to answer DA1. Answers that come later decode
/// as replies, which the reader drops, never as keys.
pub const WAIT: Duration = Duration::from_secs(1);

/// Image id of the graphics query, which stores no image. Ghostty,
/// Warp, and Rio answer only a query with an id.
const QUERY: u32 = 31;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Caps {
  pub host: Host,
  /// XTVERSION of what answered: the terminal, or tmux itself.
  pub version: Option<String>,
  /// Kitty keyboard flags the probe found, before the reader pushes
  /// its own; None without the protocol.
  pub keyboard: Option<u32>,
  /// Synchronized output, mode 2026.
  pub sync: Support,
  /// SGR pixel mouse reports, mode 1016.
  pub pixels: Support,
  /// In-band resize reports, mode 2048.
  pub resize: Support,
  /// Pointer shapes, OSC 22.
  pub pointer: Support,
  /// Drag and drop, OSC 72.
  pub drop: Support,
  /// Cell size from `CSI 16 t`, asked only when TIOCGWINSZ has none.
  pub cell: Option<Cell>,
  /// DA1's answer; None when it missed `WAIT` or was never asked.
  pub attributes: Option<Vec<u32>>,
}

/// What sits between vpdf and the terminal.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Host {
  /// The terminal itself, with its answer to the graphics query.
  Direct(Graphics),
  /// tmux; None when `tmux display` failed.
  Tmux(Option<Tmux>),
  /// GNU screen, asked nothing.
  Screen,
}

/// The answer to `a=q`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Graphics {
  Ok,
  /// An error such as `EINVAL:…`.
  Error(String),
  Silent,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Tmux {
  /// `#{version}` as numbers: `3.2a` is 3.2, `next-3.9` is 3.9; empty
  /// for a name without a tmux release, such as `openbsd-7.8`.
  pub version: Vec<u32>,
  /// The client terminal's XTVERSION, `#{client_termtype}`.
  pub client: Option<String>,
  /// The client's cell in pixels at the probe; cell sizes read it
  /// fresh through `tmux_now`.
  pub cell: Option<Cell>,
  /// `allow-passthrough` for vpdf's pane; None before tmux 3.3.
  pub passthrough: Option<Passthrough>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Passthrough {
  Off,
  On,
  /// Also from panes that are not visible.
  All,
}

/// Cell size in pixels.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Cell {
  pub width: u16,
  pub height: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Support {
  /// The terminal answered that it has it.
  Answered,
  /// The terminal's name says so; it does not answer the query.
  Named,
  Missing,
}

/// How the reader can draw on this terminal. Keyboard support is a
/// separate requirement.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Profile {
  Graphics,
  /// iTerm2 stretches a crop to the whole image unless `c,r` size it.
  CropQuirk,
  Unsupported(Lack),
}

/// Why graphics are unsupported.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Lack {
  Screen,
  /// DA1 missed `WAIT`.
  Answer,
  /// No OK to the graphics query; the error when one came.
  Graphics(Option<String>),
  /// A named terminal lacks what the reader draws with.
  Terminal(&'static str),
  /// `tmux display` failed.
  Tmux,
  /// tmux's client terminal, when known, is not a graphics terminal.
  Client(Option<String>),
}

impl fmt::Display for Lack {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    match self {
      Self::Screen => write!(f, "GNU screen cannot pass Kitty graphics"),
      Self::Answer => write!(f, "Terminal did not answer queries"),
      Self::Graphics(None) => write!(f, "No Kitty graphics"),
      Self::Graphics(Some(error)) => {
        write!(f, "Kitty graphics failed: {error}")
      }
      Self::Terminal(reason) => write!(f, "{reason}"),
      Self::Tmux => write!(f, "Cannot query tmux"),
      Self::Client(None) => write!(f, "tmux names no client terminal"),
      Self::Client(Some(name)) => {
        write!(f, "tmux client {name} lacks graphics")
      }
    }
  }
}

impl fmt::Display for Profile {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    match self {
      Self::Graphics => write!(f, "Kitty graphics"),
      Self::CropQuirk => write!(f, "Kitty graphics, iTerm2 crop sizing"),
      Self::Unsupported(lack) => write!(f, "unsupported ({lack})"),
    }
  }
}

impl fmt::Display for Support {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    match self {
      Self::Answered => write!(f, "yes"),
      Self::Named => write!(f, "yes, by name"),
      Self::Missing => write!(f, "no"),
    }
  }
}

/// The names `allow-passthrough` takes.
impl fmt::Display for Passthrough {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    f.write_str(match self {
      Self::Off => "off",
      Self::On => "on",
      Self::All => "all",
    })
  }
}

impl fmt::Display for Cell {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    write!(f, "{}x{} px", self.width, self.height)
  }
}

/// One `Name: value` line per field, for `:info`. Values drop control
/// characters, since a terminal's answer can hold a line break.
impl fmt::Display for Caps {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    let version = self.version.as_deref().unwrap_or("none");
    let mut rows = vec![
      ("Profile", self.profile().to_string()),
      ("XTVERSION", version.into()),
    ];
    match &self.host {
      Host::Direct(graphics) => rows.push((
        "Graphics query",
        match graphics {
          Graphics::Ok => "OK".into(),
          Graphics::Error(error) => format!("failed: {error}"),
          Graphics::Silent => "no answer".into(),
        },
      )),
      Host::Tmux(None) => rows.push(("tmux", "cannot query".into())),
      Host::Tmux(Some(tmux)) => rows.extend(tmux.rows()),
      Host::Screen => rows.push(("GNU screen", "not queried".into())),
    }
    let keyboard =
      self.keyboard.map(|flags| format!("yes, flags {flags} at start"));
    rows.extend([
      ("Kitty keyboard", keyboard.unwrap_or_else(|| "no".into())),
      ("Synchronized output", self.sync.to_string()),
      ("Pixel mouse", self.pixels.to_string()),
      ("Resize reports", self.resize.to_string()),
      ("Pointer shapes", self.pointer.to_string()),
      ("Drag and drop", self.drop.to_string()),
    ]);
    // Asked only without TIOCGWINSZ pixels; absent is the usual case.
    if let Some(cell) = self.cell {
      rows.push(("Cell reply", cell.to_string()));
    }
    let attributes = self.attributes.as_deref().map(|values| join(values, ";"));
    rows.push(("DA1", attributes.unwrap_or_else(|| "no answer".into())));
    for (index, (name, value)) in rows.into_iter().enumerate() {
      let value: String = crate::text::printable(&value).collect();
      let separator = if index == 0 { "" } else { "\n" };
      write!(f, "{separator}{name}: {value}")?;
    }
    Ok(())
  }
}

impl Tmux {
  fn rows(&self) -> [(&'static str, String); 4] {
    let release = join(&self.version, ".");
    let old = !self.version.is_empty() && self.version[..] < [3, 3][..];
    let passthrough = match self.passthrough {
      Some(passthrough) => passthrough.to_string(),
      None if old => "on, no option before tmux 3.3".into(),
      None => "unknown".into(),
    };
    [
      (
        "tmux",
        if release.is_empty() { "unknown release".into() } else { release },
      ),
      ("tmux client", self.client.clone().unwrap_or_else(|| "none".into())),
      (
        "tmux cell",
        self.cell.map_or_else(|| "none".into(), |cell| cell.to_string()),
      ),
      ("tmux passthrough at start", passthrough),
    ]
  }
}

fn join(numbers: &[u32], separator: &str) -> String {
  numbers.iter().map(u32::to_string).collect::<Vec<_>>().join(separator)
}

/// Kitty keyboard flags the reader pushes: disambiguation, which it
/// requires, and alternate keys, which serve other layouts.
const KEYBOARD: u32 = DISAMBIGUATE | ALTERNATE_KEYS;

/// The required flag: without it Esc, Ctrl-i, and Alt chords share
/// bytes with other keys.
const DISAMBIGUATE: u32 = 1;

const ALTERNATE_KEYS: u32 = 4;

/// Rounds that read the flags back before the reader gives up; a
/// refusing terminal answers each at once.
const READBACKS: usize = 3;

/// Why keys cannot be told apart.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum KeyLack {
  /// GNU screen, asked nothing.
  Screen,
  /// DA1 missed `WAIT`, so the probe cannot tell.
  Unanswered,
  /// No answer to `CSI ? u`; the XTVERSION of what answered, if any.
  Protocol(Option<String>),
  /// Flags in effect after pushing `KEYBOARD`; None without an answer.
  Refused(Option<u32>),
}

impl fmt::Display for KeyLack {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    match self {
      Self::Screen => {
        write!(f, "GNU screen lacks Kitty keyboard disambiguation")
      }
      Self::Unanswered => write!(f, "Terminal did not answer queries"),
      Self::Protocol(name) if is(name.as_deref(), &["wezterm"]) => {
        write!(f, "WezTerm needs enable_kitty_keyboard = true")
      }
      Self::Protocol(name) => {
        let name = name.as_deref().unwrap_or("Terminal");
        write!(f, "{name} lacks Kitty keyboard disambiguation")
      }
      Self::Refused(Some(flags)) => {
        write!(f, "Kitty keyboard disambiguation refused, flags {flags}")
      }
      Self::Refused(None) => {
        write!(f, "Terminal did not confirm Kitty keyboard flags")
      }
    }
  }
}

impl std::error::Error for KeyLack {}

impl Caps {
  /// Pushes the reader's keyboard flags where the protocol answered and
  /// reads back the flags in effect in the same round; `ask` sends a
  /// request and returns the replies up to DA1's. None once keys are
  /// told apart.
  pub fn negotiate(
    &self,
    mut ask: impl FnMut(&[u8]) -> Result<Vec<Reply>>,
  ) -> Result<Option<KeyLack>> {
    if self.keyboard.is_none() {
      return Ok(self.key_lack(None));
    }
    let push = format!("\x1b[>{KEYBOARD}u\x1b[?u");
    let mut request = push.as_bytes();
    let mut enabled = None;
    // Answers a child program left unread, such as its own flags and
    // DA1, can end a round before the readback; each later round
    // settles one stale DA1 and collects what came late. The last
    // flags are the readback.
    for _ in 0..READBACKS {
      let replies = ask(request)?;
      enabled = replies.iter().rev().find_map(flags).or(enabled);
      if enabled.is_some_and(|flags| flags & DISAMBIGUATE != 0) {
        break;
      }
      request = b"\x1b[?u";
    }
    Ok(self.key_lack(enabled))
  }

  /// Why keys stay ambiguous given `enabled`, the flags in effect after
  /// pushing `KEYBOARD`.
  fn key_lack(&self, enabled: Option<u32>) -> Option<KeyLack> {
    if self.host == Host::Screen {
      return Some(KeyLack::Screen);
    }
    if self.keyboard.is_none() {
      return Some(match self.attributes {
        None => KeyLack::Unanswered,
        Some(_) => KeyLack::Protocol(self.version.clone()),
      });
    }
    match enabled {
      Some(flags) if flags & DISAMBIGUATE != 0 => None,
      enabled => Some(KeyLack::Refused(enabled)),
    }
  }

  /// The outer terminal's XTVERSION; under tmux, the client's.
  pub fn terminal(&self) -> Option<&str> {
    match &self.host {
      Host::Tmux(tmux) => tmux.as_ref()?.client.as_deref(),
      _ => self.version.as_deref(),
    }
  }

  /// Whether what answered keeps a title stack (`CSI 22/23 t`), so a
  /// title vpdf sets goes at exit; Ghostty, `WezTerm`, and Konsole ignore
  /// the stack. Under tmux the title is the pane's, which reaches the
  /// client only with `set-titles on`, off by default.
  pub fn title_stack(&self) -> bool {
    !is(self.version.as_deref(), &["ghostty", "wezterm", "konsole"])
  }

  pub fn profile(&self) -> Profile {
    let named = self.terminal().map_or(Named::Unknown, named);
    let lack = match (&self.host, named) {
      (Host::Screen, _) => Lack::Screen,
      (Host::Tmux(None), _) => Lack::Tmux,
      (_, Named::Lacks(reason)) => Lack::Terminal(reason),
      (Host::Direct(Graphics::Ok) | Host::Tmux(_), Named::Crop) => {
        return Profile::CropQuirk;
      }
      (Host::Direct(Graphics::Ok), _) | (Host::Tmux(_), Named::Graphics) => {
        return Profile::Graphics;
      }
      (Host::Direct(Graphics::Error(error)), _) => {
        Lack::Graphics(Some(error.clone()))
      }
      (Host::Direct(Graphics::Silent), _) if self.attributes.is_none() => {
        Lack::Answer
      }
      (Host::Direct(Graphics::Silent), _) => Lack::Graphics(None),
      (Host::Tmux(_), Named::Unknown) => {
        Lack::Client(self.terminal().map(str::to_owned))
      }
    };
    Profile::Unsupported(lack)
  }
}

/// What a terminal's XTVERSION name reveals that no query does.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Named {
  Graphics,
  Crop,
  Lacks(&'static str),
  Unknown,
}

fn named(xtversion: &str) -> Named {
  let (name, version) = identify(xtversion);
  match name.as_str() {
    "kitty" | "ghostty" | "wezterm" | "konsole" => Named::Graphics,
    "rio" if version[..] >= [0, 5, 27][..] => Named::Graphics,
    "rio" => Named::Lacks("Rio before 0.5.27 breaks Unicode placeholders"),
    "iterm2" if version[..] >= [3, 5, 6][..] => Named::Crop,
    "iterm2" => Named::Lacks("iTerm2 before 3.5.6 lacks Kitty graphics"),
    "warp" => Named::Lacks("Warp lacks image crops and cell offsets"),
    "wayst" => Named::Lacks("Wayst drops graphics keys after o="),
    "st-graphics" => Named::Lacks("st-graphics ignores image layers"),
    "xterm.js" => Named::Lacks("xterm.js ignores placement ids"),
    _ => Named::Unknown,
  }
}

/// Whether `xtversion` names one of `names`, given in lowercase.
fn is(xtversion: Option<&str>, names: &[&str]) -> bool {
  xtversion.is_some_and(|xtversion| names.contains(&&*identify(xtversion).0))
}

/// Lowercase name and version numbers of an XTVERSION answer, such as
/// `kitty(0.49.1)` or `WezTerm 20240203-110809-5046fc22`.
fn identify(xtversion: &str) -> (String, Vec<u32>) {
  let (name, version) =
    xtversion.split_once(['(', ' ']).unwrap_or((xtversion, ""));
  (name.to_ascii_lowercase(), numbers(version))
}

fn numbers(version: &str) -> Vec<u32> {
  version
    .split(|ch: char| !ch.is_ascii_digit())
    .filter(|part| !part.is_empty())
    .map(|part| part.parse().unwrap_or(u32::MAX))
    .collect()
}

/// Which way the queries go, before any answer.
#[derive(Clone, Debug, Eq, PartialEq)]
enum Route {
  Direct,
  Tmux(Option<Tmux>),
  Screen,
}

/// GNU screen wins over tmux: either nesting puts screen in the path.
fn route(
  set: impl Fn(&str) -> bool,
  tmux: impl FnOnce() -> Option<Tmux>,
) -> Route {
  if set("STY") {
    Route::Screen
  } else if set("TMUX") {
    Route::Tmux(tmux())
  } else {
    Route::Direct
  }
}

/// The first round: XTVERSION, the graphics query with a full 1x1
/// payload (Konsole ignores a short one), modes 2026, 1016, and 2048,
/// keyboard flags, pointer shapes, and drag and drop.
fn request(route: &Route) -> Vec<u8> {
  let mut request = b"\x1b[>0q".to_vec();
  if *route == Route::Direct {
    let query = format!("\x1b_Gi={QUERY},s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\");
    request.extend_from_slice(query.as_bytes());
  }
  request.extend_from_slice(
    b"\x1b[?2026$p\x1b[?1016$p\x1b[?2048$p\x1b[?u\
      \x1b]22;?pointer\x1b\\\x1b]72;t=q\x1b\\",
  );
  request
}

/// Whether to ask `CSI 16 t` for the cell size: only when TIOCGWINSZ
/// has no pixels and the terminal answers DA1. Wayst resizes its
/// window to 16 rows instead, and iTerm2 answers in points.
fn asks_cell(caps: &Caps, pixels: bool) -> bool {
  // tmux answers from its own window and always has pixels.
  let direct = matches!(caps.host, Host::Direct(_));
  let wrong = is(caps.version.as_deref(), &["wayst", "iterm2"]);
  direct && !pixels && !wrong && caps.attributes.is_some()
}

fn caps(route: Route, replies: &[Reply]) -> Caps {
  let mode = |wanted| {
    replies.iter().find_map(|reply| match reply {
      Reply::Mode { mode, state } if *mode == wanted => Some(match state {
        ModeState::Set | ModeState::Reset | ModeState::AlwaysSet => {
          Support::Answered
        }
        ModeState::NotRecognized | ModeState::AlwaysReset => Support::Missing,
      }),
      _ => None,
    })
  };
  let osc = |wanted, supported: fn(&str) -> bool| {
    replies.iter().any(|reply| {
      matches!(reply, Reply::Osc { code, body }
        if *code == wanted && supported(body))
    })
  };
  let answered = |yes| if yes { Support::Answered } else { Support::Missing };
  let host = match route {
    Route::Direct => Host::Direct(graphics(replies)),
    Route::Tmux(tmux) => Host::Tmux(tmux),
    Route::Screen => Host::Screen,
  };
  let mut caps = Caps {
    host,
    version: replies.iter().find_map(|reply| match reply {
      Reply::Version(version) => Some(version.clone()),
      _ => None,
    }),
    keyboard: replies.iter().find_map(flags),
    sync: mode(2026).unwrap_or(Support::Missing),
    pixels: mode(1016).unwrap_or(Support::Missing),
    resize: mode(2048).unwrap_or(Support::Missing),
    pointer: answered(osc(22, |body| body.split(',').next() == Some("1"))),
    drop: answered(osc(72, |body| body.starts_with("t=q"))),
    cell: cell_reply(replies),
    attributes: replies.iter().find_map(|reply| match reply {
      Reply::Attributes(values) => Some(values.clone()),
      _ => None,
    }),
  };
  // Ghostty sets pointer shapes but never answers the query. Under tmux
  // `pointer` is tmux's answer, which the client's name cannot change.
  let direct = matches!(caps.host, Host::Direct(_));
  if direct
    && is(caps.version.as_deref(), &["ghostty"])
    && caps.pointer == Support::Missing
  {
    caps.pointer = Support::Named;
  }
  caps
}

/// The flags of a `CSI ? u` answer.
fn flags(reply: &Reply) -> Option<u32> {
  match reply {
    Reply::Keyboard(flags) => Some(*flags),
    _ => None,
  }
}

fn graphics(replies: &[Reply]) -> Graphics {
  let answer = replies.iter().find_map(|reply| match reply {
    Reply::Graphics { id: Some(QUERY), message, .. } => Some(message),
    _ => None,
  });
  match answer {
    Some(message) if message == "OK" => Graphics::Ok,
    Some(message) => Graphics::Error(message.clone()),
    None => Graphics::Silent,
  }
}

/// The cell size in a `CSI 16 t` answer, `CSI 6 ; height ; width t`.
fn cell_reply(replies: &[Reply]) -> Option<Cell> {
  replies.iter().find_map(|reply| match reply {
    Reply::Window(values) => match values[..] {
      [6, height, width] => cell(width, height),
      _ => None,
    },
    _ => None,
  })
}

fn cell(width: u32, height: u32) -> Option<Cell> {
  let width = u16::try_from(width).ok().filter(|&width| width > 0)?;
  let height = u16::try_from(height).ok().filter(|&height| height > 0)?;
  Some(Cell { width, height })
}

/// Formats `tmux display` expands for `Tmux`, tab-separated. Option
/// names expand to the pane's value, inherited as `show -A` does.
const TMUX_FORMAT: &str = "#{version}\t#{client_termtype}\t\
  #{client_cell_width}\t#{client_cell_height}\t#{allow-passthrough}";

fn tmux(display: &str) -> Option<Tmux> {
  let fields: Vec<&str> = display.trim_end_matches('\n').split('\t').collect();
  let [version, client, width, height, passthrough] = fields[..] else {
    return None;
  };
  if version.is_empty() {
    return None;
  }
  let release = version.strip_prefix("next-").unwrap_or(version);
  let version = if release.starts_with(|ch: char| ch.is_ascii_digit()) {
    numbers(release)
  } else {
    Vec::new()
  };
  let number = |text: &str| text.parse().unwrap_or(0);
  Some(Tmux {
    version,
    client: Some(client).filter(|client| !client.is_empty()).map(str::to_owned),
    cell: cell(number(width), number(height)),
    passthrough: [Passthrough::Off, Passthrough::On, Passthrough::All]
      .into_iter()
      .find(|value| value.to_string() == passthrough),
  })
}

static CAPS: OnceLock<Caps> = OnceLock::new();

/// The probe's result; None before `probe` ran.
pub fn get() -> Option<&'static Caps> {
  CAPS.get()
}

/// Asks the terminal on the first call, which needs raw mode and
/// `input::start`; later calls return the first answer. Keys typed
/// meanwhile stay queued as input.
pub fn probe() -> Result<&'static Caps> {
  if let Some(caps) = CAPS.get() {
    return Ok(caps);
  }
  let route = route(
    |name| std::env::var_os(name).is_some(),
    || tmux(&crate::terminal::tmux_display(TMUX_FORMAT)?),
  );
  let pixels = crossterm::terminal::window_size()
    .is_ok_and(|size| size.width > 0 && size.height > 0);
  let mut out = io::stdout();
  let probed = rounds(route, pixels, |request| {
    crate::input::ask(&mut out, request, WAIT)
  })?;
  Ok(CAPS.get_or_init(|| probed))
}

/// Asks `CSI 16 t` again where the probe would without pixel sizes,
/// as after a resize; None when it may not ask or gets no answer.
pub fn ask_cell() -> Option<Cell> {
  let probed = get()?;
  if !asks_cell(probed, false) {
    return None;
  }
  let replies = crate::input::ask(&mut io::stdout(), b"\x1b[16t", WAIT).ok()?;
  cell_reply(&replies)
}

/// tmux's view now, read as `probe` reads it; None outside tmux.
pub fn tmux_now() -> Option<Tmux> {
  tmux(&crate::terminal::tmux_display(TMUX_FORMAT)?)
}

/// Asks the first round, then `CSI 16 t` when `asks_cell` says so;
/// `ask` sends a request and returns the replies up to DA1's.
fn rounds(
  route: Route,
  pixels: bool,
  mut ask: impl FnMut(&[u8]) -> Result<Vec<Reply>>,
) -> Result<Caps> {
  let mut replies =
    if route == Route::Screen { Vec::new() } else { ask(&request(&route))? };
  let first = caps(route.clone(), &replies);
  if !asks_cell(&first, pixels) {
    return Ok(first);
  }
  replies.extend(ask(b"\x1b[16t")?);
  Ok(caps(route, &replies))
}

#[cfg(test)]
mod tests;
