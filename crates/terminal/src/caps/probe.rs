use super::{
  Caps, Cell, Graphics, Host, Lack, ModeState, OnceLock, Passthrough, Profile,
  QUERY, Reply, Result, Support, Tmux, WAIT, io,
};

/// `allow-passthrough` as vpdf set it for its pane, which `:info` shows
/// next to the value at start.
pub(super) static PASSTHROUGH: OnceLock<Passthrough> = OnceLock::new();
static PASSTHROUGH_NOTICE: OnceLock<Option<String>> = OnceLock::new();

/// Sets `allow-passthrough` for vpdf's pane where it falls short, as
/// `kitten icat` does; returns what the status line says about it.
pub fn allow_passthrough() -> Option<String> {
  if let Some(notice) = PASSTHROUGH_NOTICE.get() {
    return notice.clone();
  }
  let Some(Host::Tmux(Some(tmux))) = get().map(|caps| &caps.host) else {
    return None;
  };
  set_passthrough(tmux)
}

fn set_passthrough(tmux: &Tmux) -> Option<String> {
  let wanted = tmux.wanted()?;
  let value = wanted.to_string();
  if !crate::terminal::tmux_set("allow-passthrough", &value) {
    return Some(format!("Cannot set tmux allow-passthrough {value}"));
  }
  PASSTHROUGH.set(wanted).ok()?;
  Some(format!("Set tmux allow-passthrough {value}"))
}

/// Leaves `allow-passthrough on` where vpdf set it, as `kitten icat`
/// does: `all` would let what runs next draw over other windows. `on`
/// still passes the exit's deletes from a shown pane, whichever tmux
/// takes first.
pub fn reset_passthrough() {
  reset_passthrough_with(
    PASSTHROUGH.get().copied(),
    || tmux_now()?.passthrough,
    |value| {
      crate::terminal::tmux_set("allow-passthrough", &value.to_string());
    },
  );
}

pub(super) fn reset_passthrough_with(
  changed: Option<Passthrough>,
  current: impl FnOnce() -> Option<Passthrough>,
  set: impl FnOnce(Passthrough),
) {
  if changed == Some(Passthrough::All) && current() == changed {
    set(Passthrough::On);
  }
}

/// Rio releases before this draw Unicode placeholders wrongly.
const RIO_PLACEHOLDERS: &[u32] = &[0, 5, 27];

/// The first iTerm2 release with Kitty graphics and placeholders.
const ITERM2_GRAPHICS: &[u32] = &[3, 5, 6];

/// What a terminal's XTVERSION name reveals that no query does.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Named {
  Graphics,
  Crop,
  Lacks(&'static str),
  Unknown,
}

pub(super) fn named(xtversion: &str) -> Named {
  let (name, version) = identify(xtversion);
  match name.as_str() {
    "kitty" | "ghostty" | "wezterm" | "konsole" => Named::Graphics,
    "rio" if version[..] >= RIO_PLACEHOLDERS[..] => Named::Graphics,
    "rio" => Named::Lacks("Rio before 0.5.27 breaks Unicode placeholders"),
    "iterm2" if version[..] >= ITERM2_GRAPHICS[..] => Named::Crop,
    "iterm2" => Named::Lacks("iTerm2 before 3.5.6 lacks Kitty graphics"),
    "warp" => Named::Lacks("Warp lacks image crops and cell offsets"),
    "wayst" => Named::Lacks("Wayst drops graphics keys after o="),
    "st-graphics" => Named::Lacks("st-graphics ignores image layers"),
    "xterm.js" => Named::Lacks("xterm.js ignores placement ids"),
    _ => Named::Unknown,
  }
}

/// Whether the terminal `xtversion` names draws Unicode placeholders
/// (`U=1`).
pub fn placeholders(xtversion: &str) -> bool {
  let (name, version) = identify(xtversion);
  let since: &[u32] = match name.as_str() {
    "kitty" => &[0, 28],
    "ghostty" => &[1],
    "iterm2" => ITERM2_GRAPHICS,
    "rio" => RIO_PLACEHOLDERS,
    _ => return false,
  };
  version[..] >= since[..]
}

/// Whether `xtversion` names one of `names`, given in lowercase.
pub(super) fn is(xtversion: Option<&str>, names: &[&str]) -> bool {
  xtversion.is_some_and(|xtversion| names.contains(&&*identify(xtversion).0))
}

/// Lowercase name and version numbers of an XTVERSION answer, such as
/// `kitty(0.49.1)` or `WezTerm 20240203-110809-5046fc22`.
pub(super) fn identify(xtversion: &str) -> (String, Vec<u32>) {
  let (name, version) =
    xtversion.split_once(['(', ' ']).unwrap_or((xtversion, ""));
  (name.to_ascii_lowercase(), numbers(version))
}

pub(super) fn numbers(version: &str) -> Vec<u32> {
  version
    .split(|ch: char| !ch.is_ascii_digit())
    .filter(|part| !part.is_empty())
    .map(|part| part.parse().unwrap_or(u32::MAX))
    .collect()
}

/// Which way the queries go, before any answer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum Route {
  Direct,
  Tmux(Option<Tmux>),
  Screen,
}

/// GNU screen wins over tmux: either nesting puts screen in the path.
pub(super) fn route(
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
/// keyboard flags, drag and drop, pointer shapes, and mode 5522.
pub(super) fn request(route: &Route) -> Vec<u8> {
  let mut request = b"\x1b[>0q".to_vec();
  if *route == Route::Direct {
    let query = format!("\x1b_Gi={QUERY},s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\");
    request.extend_from_slice(query.as_bytes());
  }
  request.extend_from_slice(
    b"\x1b[?2026$p\x1b[?1016$p\x1b[?2048$p\x1b[?u\x1b]72;t=q\x1b\\",
  );
  // tmux drops OSC 22; wrap it.
  let query = b"\x1b]11;?\x1b\\\x1b]22;?pointer\x1b\\\x1b[?5522$p";
  if matches!(route, Route::Tmux(_)) {
    // The outer DA1 follows this same path.
    crate::terminal::passthrough(&mut request, query, true)
      .expect("Vec write cannot fail");
  } else {
    request.extend_from_slice(query);
  }
  request
}

/// Whether to ask `CSI 16 t` for the cell size: only when TIOCGWINSZ
/// has no pixels and the terminal answers DA1. Wayst resizes its
/// window to 16 rows instead, and iTerm2 answers in points.
pub(super) fn asks_cell(caps: &Caps, pixels: bool) -> bool {
  // tmux answers from its own window and always has pixels.
  let direct = matches!(caps.host, Host::Direct(_));
  let wrong = is(caps.version.as_deref(), &["wayst", "iterm2"]);
  direct && !pixels && !wrong && caps.attributes.is_some()
}

pub(super) fn caps(route: Route, replies: &[Reply]) -> Caps {
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
    background: replies.iter().find_map(|reply| match reply {
      Reply::Osc { code: 11, body } => background(body),
      _ => None,
    }),
    sync: mode(2026).unwrap_or(Support::Missing),
    pixels: mode(1016).unwrap_or(Support::Missing),
    resize: mode(2048).unwrap_or(Support::Missing),
    pointer: answered(osc(22, |body| body.split(',').next() == Some("1"))),
    drop: answered(osc(72, |body| body.starts_with("t=q"))),
    clipboard: mode(5522),
    cell: cell_reply(replies),
    attributes: replies.iter().find_map(|reply| match reply {
      Reply::Attributes(values) => Some(values.clone()),
      _ => None,
    }),
  };
  if matches!(caps.host, Host::Direct(_))
    && is(caps.version.as_deref(), &["tmux"])
  {
    caps.host = Host::Tmux(None);
  }
  // Ghostty sets shapes but never answers.
  if is(caps.terminal(), &["ghostty"]) && caps.pointer == Support::Missing {
    caps.pointer = Support::Named;
  }
  caps
}

fn background(body: &str) -> Option<[u8; 3]> {
  let mut channels = body.strip_prefix("rgb:")?.split('/');
  let mut rgb = [0; 3];
  for value in &mut rgb {
    let text = channels.next()?;
    if !(1..=4).contains(&text.len())
      || !text.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
      return None;
    }
    let channel = u32::from_str_radix(text, 16).ok()?;
    let maximum = (1_u32 << (text.len() * 4)) - 1;
    *value = u8::try_from((channel * 255 + maximum / 2) / maximum).ok()?;
  }
  channels.next().is_none().then_some(rgb)
}

/// The flags of a `CSI ? u` answer.
pub(super) fn flags(reply: &Reply) -> Option<u32> {
  match reply {
    Reply::Keyboard(flags) => Some(*flags),
    _ => None,
  }
}

pub(super) fn graphics(replies: &[Reply]) -> Graphics {
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

pub(super) fn cell(width: u32, height: u32) -> Option<Cell> {
  let width = u16::try_from(width).ok().filter(|&width| width > 0)?;
  let height = u16::try_from(height).ok().filter(|&height| height > 0)?;
  Some(Cell { width, height })
}

/// Formats `tmux display` expands for `Tmux`, tab-separated. Option
/// names expand to the pane's value, inherited as `show -A` does.
pub(crate) const TMUX_FORMAT: &str = "#{version}\t#{client_termtype}\t\
  #{client_cell_width}\t#{client_cell_height}\t#{allow-passthrough}\t\
  #{extended-keys}\t#{focus-events}\t#{pane_left}\t#{pane_top}\t\
  #{window_offset_x}\t#{window_offset_y}\t#{status-position}\t#{status}";

pub(crate) fn tmux(display: &str) -> Option<Tmux> {
  let fields: Vec<&str> = display.trim_end_matches('\n').split('\t').collect();
  let [
    version,
    client,
    width,
    height,
    passthrough,
    extended,
    focus,
    left,
    top,
    x,
    y,
    position,
    status,
  ] = fields[..]
  else {
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
  let small = |text: &str| text.parse().unwrap_or(0_u16);
  // A status line on top pushes panes down by its rows.
  let status = match (position, status) {
    ("top", "on") => 1,
    ("top", rows) => small(rows),
    _ => 0,
  };
  let origin = (
    small(left).saturating_sub(small(x)),
    small(top).saturating_sub(small(y)).saturating_add(status),
  );
  Some(Tmux {
    version,
    client: Some(client).filter(|client| !client.is_empty()).map(str::to_owned),
    cell: cell(number(width), number(height)),
    passthrough: [Passthrough::Off, Passthrough::On, Passthrough::All]
      .into_iter()
      .find(|value| value.to_string() == passthrough),
    extended_keys: matches!(extended, "on" | "always"),
    focus_events: focus == "1",
    origin,
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
  let notice = match &route {
    Route::Tmux(Some(tmux)) => set_passthrough(tmux),
    _ => None,
  };
  let _ = PASSTHROUGH_NOTICE.set(notice);
  let outer = matches!(route, Route::Tmux(_));
  let pixels = crossterm::terminal::window_size()
    .is_ok_and(|size| size.width > 0 && size.height > 0);
  let mut out = io::stdout();
  let probed = rounds(route, pixels, |request| {
    if outer {
      crate::input::ask_outer(&mut out, request, WAIT)
    } else {
      crate::input::ask(&mut out, request, WAIT)
    }
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

/// Whether output goes through tmux, as `probe` routes it: GNU screen
/// wins over tmux. Before the probe, from the environment, read once.
pub fn under_tmux() -> bool {
  static ENVIRONMENT: OnceLock<bool> = OnceLock::new();
  match get() {
    Some(caps) => matches!(caps.host, Host::Tmux(_)),
    None => *ENVIRONMENT.get_or_init(|| {
      let set = |name: &str| std::env::var_os(name).is_some();
      matches!(route(set, || None), Route::Tmux(_))
    }),
  }
}

/// tmux's view now, read as `probe` reads it; None outside tmux.
pub fn tmux_now() -> Option<Tmux> {
  tmux(&crate::terminal::tmux_display(TMUX_FORMAT)?)
}

/// Asks the first round, then `CSI 16 t` when `asks_cell` says so;
/// `ask` sends a request and returns the replies up to DA1's.
pub(super) fn rounds(
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

impl Caps {
  pub fn palette(&self) -> vpdf_model::theme::Palette {
    use vpdf_model::theme::{Palette, is_light};
    if self.background.is_some_and(is_light) {
      Palette::light()
    } else {
      Palette::dark()
    }
  }

  /// The outer terminal's XTVERSION; under tmux, the client's.
  pub fn terminal(&self) -> Option<&str> {
    match &self.host {
      Host::Tmux(tmux) => tmux.as_ref()?.client.as_deref(),
      _ => self.version.as_deref(),
    }
  }

  /// Whether negative layers paint per cell.
  pub fn cell_offsets(&self) -> bool {
    is(self.terminal(), &["wezterm"])
  }

  /// Whether Ctrl-i reaches vpdf as Tab: tmux asks its client for
  /// modifyOtherKeys, which Kitty ignores by design.
  pub fn merges_ctrl_i(&self) -> bool {
    matches!(self.host, Host::Tmux(_)) && is(self.terminal(), &["kitty"])
  }

  /// Whether 1016 reports TIOCGWINSZ pixels; iTerm2 sends points.
  pub fn pixel_mouse(&self) -> bool {
    self.pixels == Support::Answered
      && is(self.version.as_deref(), &["kitty", "ghostty"])
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

pub(super) fn join(numbers: &[u32], separator: &str) -> String {
  numbers.iter().map(u32::to_string).collect::<Vec<_>>().join(separator)
}
