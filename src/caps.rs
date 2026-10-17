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
  /// Kitty keyboard flags in effect; None without the protocol.
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
  /// The client's cell in pixels.
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

impl Caps {
  /// The outer terminal's XTVERSION; under tmux, the client's.
  pub fn terminal(&self) -> Option<&str> {
    match &self.host {
      Host::Tmux(tmux) => tmux.as_ref()?.client.as_deref(),
      _ => self.version.as_deref(),
    }
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
    keyboard: replies.iter().find_map(|reply| match reply {
      Reply::Keyboard(flags) => Some(*flags),
      _ => None,
    }),
    sync: mode(2026).unwrap_or(Support::Missing),
    pixels: mode(1016).unwrap_or(Support::Missing),
    resize: mode(2048).unwrap_or(Support::Missing),
    pointer: answered(osc(22, |body| body.split(',').next() == Some("1"))),
    drop: answered(osc(72, |body| body.starts_with("t=q"))),
    cell: replies.iter().find_map(|reply| match reply {
      Reply::Window(values) => match values[..] {
        [6, height, width] => cell(width, height),
        _ => None,
      },
      _ => None,
    }),
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
    passthrough: match passthrough {
      "off" => Some(Passthrough::Off),
      "on" => Some(Passthrough::On),
      "all" => Some(Passthrough::All),
      _ => None,
    },
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
