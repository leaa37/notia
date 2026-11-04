//! The web browser of the host computer: the Owner's agent drives a Chrome
//! or Edge window of the PC where Notia hosts the library through Playwright
//! (the app chats, Telegram and the AI tasks). This module validates the
//! tools' arguments, writes the request the Playwright process runs, reads
//! the page it answers with and writes what the model reads (the page's
//! accessibility snapshot, cut into parts) and the texts of the
//! confirmation. The adapter installs Playwright, keeps the browser process
//! and hands it the requests.
//!
//! The model points at elements by the references of the last snapshot
//! (`[ref=e12]`); they change with the page, so every action answers with a
//! new snapshot. What the snapshot does not describe (the images of a
//! captcha, a slider) is pointed at by the cells of a grid drawn over the
//! last screenshot (`C4`), which the person reads to say where to click:
//! a captcha or a verification is solved by the person, never the model.

use std::time::Duration;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::BackendError;

pub const NAVIGATE_TOOL: &str = "browser_navigate";
pub const SNAPSHOT_TOOL: &str = "browser_snapshot";
pub const CLICK_TOOL: &str = "browser_click";
pub const TYPE_TOOL: &str = "browser_type";
pub const PRESS_KEY_TOOL: &str = "browser_press_key";
pub const SELECT_OPTION_TOOL: &str = "browser_select_option";
pub const SCREENSHOT_TOOL: &str = "browser_screenshot";
pub const TABS_TOOL: &str = "browser_tabs";
pub const WAIT_TOOL: &str = "browser_wait";
pub const CLOSE_TOOL: &str = "browser_close";
pub const DRAG_TOOL: &str = "browser_drag";

pub const BROWSER_TOOLS: [&str; 11] = [
    NAVIGATE_TOOL,
    SNAPSHOT_TOOL,
    CLICK_TOOL,
    TYPE_TOOL,
    PRESS_KEY_TOOL,
    SELECT_OPTION_TOOL,
    SCREENSHOT_TOOL,
    TABS_TOOL,
    WAIT_TOOL,
    CLOSE_TOOL,
    DRAG_TOOL,
];

/// The tools that act on a page as the person would (a click may buy, send
/// or delete): each asks for confirmation.
pub const BROWSER_INTERACTION_TOOLS: [&str; 5] = [CLICK_TOOL, TYPE_TOOL, PRESS_KEY_TOOL, SELECT_OPTION_TOOL, DRAG_TOOL];

pub fn is_browser_tool(name: &str) -> bool {
    BROWSER_TOOLS.contains(&name)
}

pub fn is_browser_interaction_tool(name: &str) -> bool {
    BROWSER_INTERACTION_TOOLS.contains(&name)
}

const MAX_URL_CHARS: usize = 4_000;
const MAX_TEXT_CHARS: usize = 10_000;
const MAX_KEY_CHARS: usize = 40;
const MAX_ELEMENT_CHARS: usize = 200;
const MAX_OPTIONS: usize = 20;
const MAX_WAIT: Duration = Duration::from_secs(60);
const DEFAULT_WAIT_FOR_TEXT: Duration = Duration::from_secs(30);
/// Characters of the snapshot one result carries; the rest comes with
/// `browser_snapshot` and `offset`.
pub const SNAPSHOT_PART_CHARS: usize = 20_000;
const MAX_DIALOG_CHARS: usize = 500;
/// Cells one click may take (the images of a captcha).
const MAX_CELLS: usize = 16;
/// The grid has at most this many cells on its longer side, and cells of
/// at least this many pixels: readable on a phone, fine enough for the
/// images of a captcha when it covers only the challenge.
const GRID_MAX_CELLS: u32 = 12;
const GRID_MIN_CELL: u32 = 24;

/// The grid drawn over the last screenshot that had one, in pixels of the
/// page's viewport: rows by letter from the top (`A`), columns by number
/// from the left (`1`).
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct GridSpec {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub cell: f64,
    pub columns: u32,
    pub rows: u32,
}

impl GridSpec {
    /// The label of the bottom-right cell.
    pub fn last_label(&self) -> String {
        format!("{}{}", char::from(b'A' + self.rows.clamp(1, 26) as u8 - 1), self.columns.max(1))
    }

    /// Where a cell is clicked: the middle of its part inside the grid.
    pub fn center(&self, label: &str) -> Result<(f64, f64), BackendError> {
        let (row, column) = parse_cell(label)
            .filter(|(row, column)| *row < self.rows && (1..=self.columns).contains(column))
            .ok_or_else(|| {
                BackendError::invalid_input(format!(
                    "La casilla {} no está en la grilla de la última captura: van de A1 a {}.",
                    label.trim(),
                    self.last_label()
                ))
            })?;
        let middle = |start: f64, index: u32, end: f64| {
            let from = start + f64::from(index) * self.cell;
            (from + (from + self.cell).min(end)) / 2.0
        };
        Ok((middle(self.x, column - 1, self.x + self.width), middle(self.y, row, self.y + self.height)))
    }
}

/// `C4` (also `c 4`): the row (from 0) and the column (from 1).
fn parse_cell(label: &str) -> Option<(u32, u32)> {
    let label = label.chars().filter(|char| !char.is_whitespace()).collect::<String>().to_ascii_uppercase();
    let mut chars = label.chars();
    let row = chars.next().filter(char::is_ascii_uppercase)?;
    let column = chars.as_str();
    if column.is_empty() || column.len() > 3 || !column.chars().all(|char| char.is_ascii_digit()) {
        return None;
    }
    Some((u32::from(row) - u32::from('A'), column.parse().ok().filter(|column| *column > 0)?))
}

/// Where `browser_navigate` goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NavigateTarget {
    Url(String),
    Back,
    Forward,
    Reload,
}

/// What `browser_tabs` does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TabAction {
    List,
    New(Option<String>),
    Select(usize),
    Close(Option<usize>),
}

/// What `browser_wait` waits for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WaitCondition {
    Text { text: String, timeout: Duration },
    TextGone { text: String, timeout: Duration },
    Time(Duration),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

impl MouseButton {
    fn id(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
            Self::Middle => "middle",
        }
    }
}

/// An element of the page: the reference of the last snapshot, and how the
/// model describes it for the person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElementTarget {
    pub reference: String,
    pub element: String,
}

/// What a click points at: an element of the last snapshot, or cells of
/// the last screenshot with a grid, clicked one after the other.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClickTarget {
    Element(ElementTarget),
    Cells { cells: Vec<String>, element: String },
}

impl ClickTarget {
    fn element(&self) -> &str {
        match self {
            Self::Element(target) => &target.element,
            Self::Cells { element, .. } => element,
        }
    }
}

/// One validated browser tool call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrowserCommand {
    Navigate(NavigateTarget),
    Snapshot { within: Option<String>, offset: usize },
    Click { target: ClickTarget, double: bool, button: MouseButton, accept_dialog: bool },
    Type { target: ElementTarget, text: String, submit: bool },
    PressKey { key: String },
    SelectOption { target: ElementTarget, values: Vec<String> },
    /// Drags between two cells of the grid (a slider).
    Drag { from: String, to: String, element: String },
    /// What is seen, of the page or of one element; `grid` draws the cells
    /// over it and `for_person` sends it to the person too.
    Screenshot { within: Option<String>, grid: bool, for_person: bool },
    Tabs(TabAction),
    Wait(WaitCondition),
    Close,
}

impl BrowserCommand {
    pub fn from_call(name: &str, arguments: &Value) -> Result<Self, BackendError> {
        Ok(match name {
            NAVIGATE_TOOL => Self::Navigate(navigate_target(arguments)?),
            SNAPSHOT_TOOL => Self::Snapshot {
                within: optional_reference(arguments, "ref")?,
                offset: arguments.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize,
            },
            CLICK_TOOL => Self::Click {
                target: click_target(arguments)?,
                double: arguments.get("doubleClick").and_then(Value::as_bool).unwrap_or(false),
                button: match optional_text(arguments, "button").as_deref() {
                    None | Some("left") => MouseButton::Left,
                    Some("right") => MouseButton::Right,
                    Some("middle") => MouseButton::Middle,
                    Some(_) => return Err(BackendError::invalid_input("button debe ser left, right o middle.")),
                },
                accept_dialog: arguments.get("acceptDialog").and_then(Value::as_bool).unwrap_or(false),
            },
            TYPE_TOOL => {
                let text = arguments
                    .get("text")
                    .and_then(Value::as_str)
                    .ok_or_else(|| BackendError::invalid_input("Falta text: lo que se escribe en el campo."))?;
                if text.chars().count() > MAX_TEXT_CHARS {
                    return Err(BackendError::invalid_input(format!("text tiene más de {MAX_TEXT_CHARS} caracteres.")));
                }
                Self::Type {
                    target: element_target(arguments)?,
                    text: text.to_string(),
                    submit: arguments.get("submit").and_then(Value::as_bool).unwrap_or(false),
                }
            }
            PRESS_KEY_TOOL => {
                let key = optional_text(arguments, "key")
                    .ok_or_else(|| BackendError::invalid_input("Falta key: la tecla, como Enter, Escape, PageDown o Control+A."))?;
                if key.chars().count() > MAX_KEY_CHARS {
                    return Err(BackendError::invalid_input("key debe ser una tecla o una combinación corta, como Control+A."));
                }
                Self::PressKey { key }
            }
            SELECT_OPTION_TOOL => {
                let values = arguments
                    .get("values")
                    .and_then(Value::as_array)
                    .map(|values| values.iter().filter_map(Value::as_str).map(str::trim).filter(|value| !value.is_empty()).map(str::to_string).collect::<Vec<_>>())
                    .unwrap_or_default();
                if values.is_empty() || values.len() > MAX_OPTIONS {
                    return Err(BackendError::invalid_input(format!("values debe tener de 1 a {MAX_OPTIONS} opciones (su valor o su texto).")));
                }
                Self::SelectOption { target: element_target(arguments)?, values }
            }
            SCREENSHOT_TOOL => Self::Screenshot {
                within: optional_reference(arguments, "ref")?,
                grid: arguments.get("grid").and_then(Value::as_bool).unwrap_or(false),
                for_person: arguments.get("forPerson").and_then(Value::as_bool).unwrap_or(false),
            },
            DRAG_TOOL => {
                let cell = |key: &str| {
                    optional_text(arguments, key)
                        .filter(|cell| parse_cell(cell).is_some())
                        .ok_or_else(|| BackendError::invalid_input(format!("Falta {key}: una casilla de la grilla de la última captura, como C4.")))
                };
                Self::Drag { from: cell("from")?, to: cell("to")?, element: element_description(arguments)? }
            }
            TABS_TOOL => Self::Tabs(tab_action(arguments)?),
            WAIT_TOOL => Self::Wait(wait_condition(arguments)?),
            CLOSE_TOOL => Self::Close,
            _ => return Err(BackendError::invalid_input(format!("{name} no es una herramienta del navegador."))),
        })
    }

    /// The element the command acts on, when it acts on one.
    pub fn target(&self) -> Option<&ElementTarget> {
        match self {
            Self::Click { target: ClickTarget::Element(target), .. } | Self::Type { target, .. } | Self::SelectOption { target, .. } => Some(target),
            _ => None,
        }
    }

    /// The cells the command points at, when it uses the grid.
    fn cells(&self) -> Vec<&str> {
        match self {
            Self::Click { target: ClickTarget::Cells { cells, .. }, .. } => cells.iter().map(String::as_str).collect(),
            Self::Drag { from, to, .. } => vec![from, to],
            _ => Vec::new(),
        }
    }

    /// Whether what the command points at is still there: its reference in
    /// the last snapshot, its cells in the last grid. Otherwise the call
    /// goes back to the model before asking the person.
    pub fn check_pointers(&self, seen: &SeenPages, grid: Option<&GridSpec>) -> Result<(), BackendError> {
        if let Some(target) = self.target() {
            check_reference(seen, &target.reference)?;
        }
        self.points(grid).map(|_| ())
    }

    /// The element as a snapshot shows it (`button "Enviar"`), for the
    /// Playwright process to find it again if the page drew it again.
    pub fn described_target(&self, seen: &SeenPages) -> Option<String> {
        self.target().and_then(|target| seen.describe(&target.reference)).filter(|described| named(described))
    }

    /// The page points of the command's cells.
    fn points(&self, grid: Option<&GridSpec>) -> Result<Vec<[f64; 2]>, BackendError> {
        let cells = self.cells();
        if cells.is_empty() {
            return Ok(Vec::new());
        }
        let grid = grid.ok_or_else(|| {
            BackendError::invalid_input("No hay una grilla: tomá browser_screenshot con grid true (y forPerson si la persona tiene que indicarte las casillas) antes de usar casillas.")
        })?;
        cells
            .into_iter()
            .map(|cell| grid.center(cell).map(|(x, y)| [(x * 10.0).round() / 10.0, (y * 10.0).round() / 10.0]))
            .collect()
    }

    /// Whether the grid of the last screenshot still matches the page after
    /// this command: another page or tab has none.
    pub fn keeps_grid(&self) -> bool {
        !matches!(self, Self::Navigate(_) | Self::Close | Self::Tabs(TabAction::New(_) | TabAction::Select(_) | TabAction::Close(_)))
    }

    /// The request the Playwright process runs, as one JSON line; cells
    /// become points of `grid`.
    pub fn agent_request(&self, id: u64, grid: Option<&GridSpec>) -> Result<Value, BackendError> {
        let points = self.points(grid)?;
        let mut request = match self {
            Self::Navigate(NavigateTarget::Url(url)) => json!({ "op": "navigate", "url": url }),
            Self::Navigate(NavigateTarget::Back) => json!({ "op": "navigate", "history": "back" }),
            Self::Navigate(NavigateTarget::Forward) => json!({ "op": "navigate", "history": "forward" }),
            Self::Navigate(NavigateTarget::Reload) => json!({ "op": "navigate", "history": "reload" }),
            Self::Snapshot { within, .. } => json!({ "op": "snapshot", "ref": within }),
            Self::Click { target: ClickTarget::Element(target), double, button, accept_dialog } => {
                json!({ "op": "click", "ref": target.reference, "double": double, "button": button.id(), "acceptDialog": accept_dialog })
            }
            Self::Click { target: ClickTarget::Cells { .. }, double, button, accept_dialog } => {
                json!({ "op": "clickPoints", "points": points, "double": double, "button": button.id(), "acceptDialog": accept_dialog })
            }
            Self::Drag { .. } => json!({ "op": "drag", "from": points[0], "to": points[1] }),
            Self::Type { target, text, submit } => json!({ "op": "type", "ref": target.reference, "text": text, "submit": submit }),
            Self::PressKey { key } => json!({ "op": "press", "key": key }),
            Self::SelectOption { target, values } => json!({ "op": "select", "ref": target.reference, "values": values }),
            Self::Screenshot { within, grid, .. } => json!({
                "op": "screenshot",
                "ref": within,
                "grid": grid.then(|| json!({ "maxCells": GRID_MAX_CELLS, "minCell": GRID_MIN_CELL })),
            }),
            Self::Tabs(TabAction::List) => json!({ "op": "tabs", "action": "list" }),
            Self::Tabs(TabAction::New(url)) => json!({ "op": "tabs", "action": "new", "url": url }),
            Self::Tabs(TabAction::Select(index)) => json!({ "op": "tabs", "action": "select", "index": index }),
            Self::Tabs(TabAction::Close(index)) => json!({ "op": "tabs", "action": "close", "index": index }),
            Self::Wait(WaitCondition::Text { text, timeout }) => json!({ "op": "wait", "text": text, "timeoutMs": timeout.as_millis() as u64 }),
            Self::Wait(WaitCondition::TextGone { text, timeout }) => json!({ "op": "wait", "textGone": text, "timeoutMs": timeout.as_millis() as u64 }),
            Self::Wait(WaitCondition::Time(time)) => json!({ "op": "wait", "timeMs": time.as_millis() as u64 }),
            Self::Close => json!({ "op": "close" }),
        };
        request["id"] = json!(id);
        Ok(request)
    }

    /// Whether the answer carries the whole page's snapshot, the one the
    /// next confirmations check references against (a screenshot has none,
    /// a section's snapshot only part).
    pub fn returns_whole_snapshot(&self) -> bool {
        !matches!(self, Self::Screenshot { .. } | Self::Close | Self::Snapshot { within: Some(_), .. })
    }

    /// How long the adapter waits for the answer: the longest step the
    /// Playwright process allows itself, and a margin for the snapshot.
    pub fn answer_timeout(&self) -> Duration {
        const MARGIN: Duration = Duration::from_secs(30);
        let step = match self {
            Self::Navigate(_) | Self::Tabs(TabAction::New(Some(_))) => Duration::from_secs(45),
            Self::Wait(WaitCondition::Text { timeout, .. } | WaitCondition::TextGone { timeout, .. }) => *timeout,
            Self::Wait(WaitCondition::Time(time)) => *time,
            _ => Duration::from_secs(20),
        };
        step + MARGIN
    }

    /// The confirmation's title.
    pub fn summary(&self) -> String {
        let element = |target: &ElementTarget| one_line(&target.element, 120);
        match self {
            Self::Click { target, double, button, .. } => {
                let how = match (double, button) {
                    (true, _) => "Doble clic",
                    (false, MouseButton::Right) => "Clic derecho",
                    (false, MouseButton::Middle) => "Clic con la rueda",
                    (false, MouseButton::Left) => "Clic",
                };
                let cells = self.cells();
                let place = if cells.is_empty() { String::new() } else { format!(" en {}", cells.join(", ")) };
                format!("{how} en el navegador{place}: {}", one_line(target.element(), 120))
            }
            Self::Drag { from, to, element } => format!("Arrastrar en el navegador de {from} a {to}: {}", one_line(element, 120)),
            Self::Type { target, submit, .. } => {
                format!("Escribir en el navegador: {}{}", element(target), if *submit { " y enviar" } else { "" })
            }
            Self::PressKey { key } => format!("Presionar {key} en el navegador"),
            Self::SelectOption { target, .. } => format!("Elegir una opción en el navegador: {}", element(target)),
            _ => "Usar el navegador de la PC host".to_string(),
        }
    }

    /// What the confirmation shows whole: the page, the element as the last
    /// snapshot shows it and what goes into it.
    pub fn detail(&self, seen: &SeenPages) -> String {
        let page = seen.last.as_ref();
        let mut lines = Vec::new();
        if let Some(target) = self.target() {
            lines.push(format!("Elemento: {}", target.element.trim()));
            if let Some(described) = seen.describe(&target.reference) {
                lines.push(format!("En la página: {described}"));
            }
        }
        match self {
            Self::Type { text, submit, .. } => {
                lines.push(format!("Texto: {text}"));
                if *submit {
                    lines.push("Después presiona Enter.".to_string());
                }
            }
            Self::SelectOption { values, .. } => lines.push(format!("Opciones: {}", values.join(", "))),
            Self::PressKey { key } => lines.push(format!("Tecla: {key}")),
            Self::Click { target: ClickTarget::Cells { cells, element }, .. } => {
                lines.push(format!("Elemento: {}", element.trim()));
                lines.push(format!("Casillas de la grilla de la última captura: {}", cells.join(", ")));
            }
            Self::Drag { from, to, element } => {
                lines.push(format!("Elemento: {}", element.trim()));
                lines.push(format!("Desde la casilla {from} hasta la {to} de la grilla de la última captura"));
            }
            _ => {}
        }
        if matches!(self, Self::Click { accept_dialog: true, .. }) {
            lines.push("Si la página pide confirmar, acepta.".to_string());
        }
        if let Some(page) = page {
            lines.push(format!("Página: {} ({})", one_line(&page.title, 120), page.url));
        }
        lines.join("\n")
    }
}

fn optional_text(arguments: &Value, key: &str) -> Option<String> {
    arguments.get(key).and_then(Value::as_str).map(str::trim).filter(|value| !value.is_empty()).map(str::to_string)
}

/// Snapshot references: `e12`, or `f1e3` inside a frame.
fn is_reference(value: &str) -> bool {
    (2..=16).contains(&value.len()) && value.starts_with(['e', 'f']) && value.chars().all(|char| char.is_ascii_alphanumeric())
}

fn optional_reference(arguments: &Value, key: &str) -> Result<Option<String>, BackendError> {
    match optional_text(arguments, key) {
        None => Ok(None),
        Some(reference) if is_reference(&reference) => Ok(Some(reference)),
        Some(_) => Err(BackendError::invalid_input(format!("{key} debe ser una referencia de la última instantánea, como e12."))),
    }
}

fn element_description(arguments: &Value) -> Result<String, BackendError> {
    let element = optional_text(arguments, "element")
        .ok_or_else(|| BackendError::invalid_input("Falta element: qué elemento es, en palabras, para que la persona lo confirme."))?;
    if element.chars().count() > MAX_ELEMENT_CHARS {
        return Err(BackendError::invalid_input(format!("element tiene más de {MAX_ELEMENT_CHARS} caracteres: describilo corto.")));
    }
    Ok(element)
}

fn element_target(arguments: &Value) -> Result<ElementTarget, BackendError> {
    let reference = optional_reference(arguments, "ref")?
        .ok_or_else(|| BackendError::invalid_input("Falta ref: la referencia del elemento en la última instantánea, como e12."))?;
    Ok(ElementTarget { reference, element: element_description(arguments)? })
}

fn click_target(arguments: &Value) -> Result<ClickTarget, BackendError> {
    let cells = arguments
        .get("cells")
        .and_then(Value::as_array)
        .map(|cells| cells.iter().filter_map(Value::as_str).map(str::trim).filter(|cell| !cell.is_empty()).collect::<Vec<_>>())
        .unwrap_or_default();
    if cells.is_empty() {
        return Ok(ClickTarget::Element(element_target(arguments)?));
    }
    if arguments.get("ref").is_some_and(|reference| !reference.is_null()) {
        return Err(BackendError::invalid_input("Pasá ref o cells, no los dos."));
    }
    if cells.len() > MAX_CELLS {
        return Err(BackendError::invalid_input(format!("cells admite hasta {MAX_CELLS} casillas por clic.")));
    }
    let cells = cells
        .into_iter()
        .map(|cell| {
            parse_cell(cell)
                .map(|(row, column)| format!("{}{column}", char::from_u32(u32::from('A') + row).unwrap_or('A')))
                .ok_or_else(|| BackendError::invalid_input(format!("{cell} no es una casilla de la grilla: usá una letra y un número, como C4.")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ClickTarget::Cells { cells, element: element_description(arguments)? })
}

/// A web address: `http`/`https` only (a bare domain gets `https://`), so
/// the agent never opens files of the PC nor the browser's own pages.
pub fn web_url(value: &str) -> Result<String, BackendError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(BackendError::invalid_input("Falta url: la dirección de la página."));
    }
    if value.chars().count() > MAX_URL_CHARS || value.chars().any(char::is_whitespace) {
        return Err(BackendError::invalid_input("url debe ser una dirección web sin espacios."));
    }
    let lowered = value.to_ascii_lowercase();
    if lowered.starts_with("http://") || lowered.starts_with("https://") {
        return Ok(value.to_string());
    }
    // Any other scheme (`file:`, `chrome:`, `javascript:`); `localhost:5173`
    // names a host and its port, not a scheme.
    let other_scheme = value.contains("://")
        || value.split_once(':').is_some_and(|(scheme, rest)| {
            !scheme.is_empty()
                && scheme.chars().all(|char| char.is_ascii_alphanumeric() || matches!(char, '+' | '-' | '.'))
                && !rest.starts_with(|char: char| char.is_ascii_digit())
        });
    if other_scheme {
        return Err(BackendError::invalid_input("El navegador solo abre páginas web (http o https)."));
    }
    // A server of this PC rarely has a certificate.
    let local = ["localhost", "127.0.0.1"].iter().any(|host| lowered == *host || lowered.starts_with(&format!("{host}:")) || lowered.starts_with(&format!("{host}/")));
    Ok(format!("{}://{value}", if local { "http" } else { "https" }))
}

fn navigate_target(arguments: &Value) -> Result<NavigateTarget, BackendError> {
    match (optional_text(arguments, "url"), optional_text(arguments, "history").as_deref()) {
        (Some(url), None) => Ok(NavigateTarget::Url(web_url(&url)?)),
        (None, Some("back")) => Ok(NavigateTarget::Back),
        (None, Some("forward")) => Ok(NavigateTarget::Forward),
        (None, Some("reload")) => Ok(NavigateTarget::Reload),
        (None, Some(_)) => Err(BackendError::invalid_input("history debe ser back, forward o reload.")),
        (Some(_), Some(_)) => Err(BackendError::invalid_input("Pasá url o history, no los dos.")),
        (None, None) => Err(BackendError::invalid_input("Falta url: la dirección de la página (o history para volver, avanzar o recargar).")),
    }
}

fn tab_index(arguments: &Value) -> Option<usize> {
    arguments.get("index").and_then(Value::as_u64).map(|index| index as usize)
}

fn tab_action(arguments: &Value) -> Result<TabAction, BackendError> {
    match optional_text(arguments, "action").as_deref() {
        None | Some("list") => Ok(TabAction::List),
        Some("new") => Ok(TabAction::New(optional_text(arguments, "url").map(|url| web_url(&url)).transpose()?)),
        Some("select") => tab_index(arguments)
            .map(TabAction::Select)
            .ok_or_else(|| BackendError::invalid_input("Falta index: el número de la pestaña, como lo devuelve tabs.")),
        Some("close") => Ok(TabAction::Close(tab_index(arguments))),
        Some(_) => Err(BackendError::invalid_input("action debe ser list, new, select o close.")),
    }
}

fn wait_condition(arguments: &Value) -> Result<WaitCondition, BackendError> {
    let seconds = arguments.get("seconds").and_then(Value::as_f64).filter(|seconds| *seconds > 0.0);
    let limit = |seconds: f64| Duration::from_secs_f64(seconds).min(MAX_WAIT);
    match (optional_text(arguments, "text"), optional_text(arguments, "textGone")) {
        (Some(text), None) => Ok(WaitCondition::Text { text, timeout: seconds.map_or(DEFAULT_WAIT_FOR_TEXT, limit) }),
        (None, Some(text)) => Ok(WaitCondition::TextGone { text, timeout: seconds.map_or(DEFAULT_WAIT_FOR_TEXT, limit) }),
        (None, None) => seconds
            .map(|seconds| WaitCondition::Time(limit(seconds)))
            .ok_or_else(|| BackendError::invalid_input("Pasá text (esperar que aparezca), textGone (que desaparezca) o seconds.")),
        (Some(_), Some(_)) => Err(BackendError::invalid_input("Pasá text o textGone, no los dos.")),
    }
}

/// The first non-empty line of `text`, cut at `max` characters.
fn one_line(text: &str, max: usize) -> String {
    let line = text.lines().map(str::trim).find(|line| !line.is_empty()).unwrap_or_default();
    let mut shown = line.chars().take(max).collect::<String>();
    if line.chars().count() > max {
        shown.push('…');
    }
    shown
}

// ---------------------------------------------------------------------------
// The page
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserTab {
    pub index: usize,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub active: bool,
}

/// The page after an action, as the Playwright process reports it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserPage {
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub tabs: Vec<BrowserTab>,
    /// The accessibility snapshot of the page (or of the element asked for),
    /// with the references the actions take.
    #[serde(default)]
    pub snapshot: String,
    /// Dialogs the page opened since the last action (an alert, a confirm)
    /// and what was done with them.
    #[serde(default)]
    pub dialogs: Vec<String>,
    /// What the process did on its own during the action: an element found
    /// again after the page drew it again, accessibility turned on.
    #[serde(default)]
    pub notes: Vec<String>,
    /// The page shows a password field: the site asks to sign in, so the
    /// session of Notia's profile is not active there.
    #[serde(default)]
    pub sign_in: bool,
}

/// One answer of the Playwright process: the page, or why the action failed.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentAnswer {
    pub id: u64,
    pub ok: bool,
    #[serde(default)]
    pub page: Option<BrowserPage>,
    /// The JPEG of `screenshot`, as base64.
    #[serde(default)]
    pub image: Option<String>,
    /// The grid drawn over that screenshot.
    #[serde(default)]
    pub grid: Option<GridSpec>,
    #[serde(default)]
    pub error: Option<String>,
}

impl AgentAnswer {
    /// The failure as the model reads it: a missing element or a timeout
    /// is something it can fix with a new snapshot or another step.
    pub fn failure(&self) -> BackendError {
        let message = self.error.as_deref().map(|error| one_line(error, 600)).unwrap_or_default();
        let message = if message.is_empty() { "El navegador no pudo hacer la acción.".to_string() } else { message };
        BackendError::new(crate::error::BackendErrorCode::Conflict, format!("El navegador no pudo hacer la acción: {message}"), true)
    }
}

/// The line of `snapshot` with `reference`, without its list marker and
/// its references: `link "Ver más"`.
pub fn describe_reference(snapshot: &str, reference: &str) -> Option<String> {
    let tag = format!("[ref={reference}]");
    let line = snapshot.lines().find(|line| line.contains(&tag))?;
    let mut described = line.trim().trim_start_matches("- ").replace(&tag, "");
    while let Some(start) = described.find(" [cursor=") {
        let end = described[start..].find(']').map_or(described.len(), |end| start + end + 1);
        described.replace_range(start..end, "");
    }
    let described = described.split_whitespace().collect::<Vec<_>>().join(" ");
    let described = described.trim_end_matches(':').trim();
    (!described.is_empty()).then(|| one_line(described, 200))
}

/// Earlier snapshots kept besides the last one: a reference of one of them
/// whose element the page drew again is found by its role and name.
const EARLIER_SNAPSHOTS: usize = 3;

/// The pages the model saw: the last whole snapshot and a few before it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SeenPages {
    pub last: Option<BrowserPage>,
    pub earlier: Vec<String>,
}

impl SeenPages {
    /// A new whole snapshot; the one it replaces becomes an earlier one.
    pub fn remember(&mut self, page: BrowserPage) {
        if let Some(previous) = self.last.take() {
            self.earlier.insert(0, previous.snapshot);
            self.earlier.truncate(EARLIER_SNAPSHOTS);
        }
        self.last = Some(page);
    }

    /// How the newest snapshot that has `reference` shows its element.
    pub fn describe(&self, reference: &str) -> Option<String> {
        self.last
            .iter()
            .map(|page| page.snapshot.as_str())
            .chain(self.earlier.iter().map(String::as_str))
            .find_map(|snapshot| describe_reference(snapshot, reference))
    }
}

/// Whether the element is named (`button "Enviar"`): only a named one can
/// be found again by its role and name once the page drew it again.
fn named(described: &str) -> bool {
    described.contains('"')
}

/// Whether `reference` can be acted on: it is in the last snapshot, or an
/// earlier one shows a named element, which the Playwright process finds
/// again on the page by its role and name. Otherwise the action goes back
/// to the model before asking the person.
pub fn check_reference(seen: &SeenPages, reference: &str) -> Result<(), BackendError> {
    let Some(page) = seen.last.as_ref() else {
        return Err(BackendError::invalid_input("El navegador no tiene una página abierta: abrí una con browser_navigate."));
    };
    if page.snapshot.contains(&format!("[ref={reference}]")) || seen.describe(reference).is_some_and(|described| named(&described)) {
        Ok(())
    } else {
        Err(BackendError::invalid_input(format!(
            "La referencia {reference} no está en la última instantánea de la página: pedí browser_snapshot y usá una de las que devuelve."
        )))
    }
}

/// What the model reads after an action: the page, its tabs and one part
/// of its snapshot from `offset`, with where the next part starts.
pub fn tool_result(page: &BrowserPage, offset: usize) -> Value {
    let total = page.snapshot.chars().count();
    let start = offset.min(total);
    let part = page.snapshot.chars().skip(start).take(SNAPSHOT_PART_CHARS).collect::<String>();
    let end = start + part.chars().count();
    let mut result = json!({
        "url": page.url,
        "title": page.title,
        "tabs": page.tabs.iter().map(|tab| json!({ "index": tab.index, "title": tab.title, "url": tab.url, "active": tab.active })).collect::<Vec<_>>(),
        "snapshot": part,
    });
    if start > 0 || end < total {
        result["snapshotPart"] = json!({
            "from": start,
            "to": end,
            "total": total,
            "nextOffset": (end < total).then_some(end),
            "note": "La instantánea llega por partes: para seguir leyendo pedí browser_snapshot con offset = nextOffset, o con ref para ver solo una sección.",
        });
    }
    if !page.dialogs.is_empty() {
        result["dialogs"] = json!(page.dialogs.iter().map(|dialog| one_line(dialog, MAX_DIALOG_CHARS)).collect::<Vec<_>>());
    }
    if !page.notes.is_empty() {
        result["notes"] = json!(page.notes.iter().map(|note| one_line(note, MAX_DIALOG_CHARS)).collect::<Vec<_>>());
    }
    if page.sign_in {
        result["session"] = json!(SIGN_IN_NOTE);
    }
    result
}

/// What a page with a password field tells the model.
const SIGN_IN_NOTE: &str = "La página pide iniciar sesión (muestra un campo de contraseña): la sesión del perfil de Notia no está activa en este sitio. No vuelvas a navegar ni reintentes: pedile a la persona los datos o que inicie sesión ella en la ventana.";

/// What the model reads after a screenshot: the page, its tabs, the grid
/// and whether the person gets it too; the image (JPEG, base64) goes to the
/// model beside the result.
pub fn screenshot_result(page: &BrowserPage, image_base64: String, grid: Option<&GridSpec>, for_person: bool) -> Value {
    let mut result = tool_result(page, 0);
    if let Some(result) = result.as_object_mut() {
        result.remove("snapshot");
        result.insert("screenshot".into(), json!("La captura de lo que se ve llega como imagen."));
        if let Some(grid) = grid {
            result.insert(
                "grid".into(),
                json!({
                    "from": "A1",
                    "to": grid.last_label(),
                    "note": "Filas con letras (A arriba) y columnas con números (1 a la izquierda). Tocá o arrastrá solo las casillas que indique la persona: browser_click con cells o browser_drag.",
                }),
            );
        }
        if for_person {
            result.insert(
                "sentToPerson".into(),
                json!("La persona recibe esta captura antes de tu próxima pregunta con request_user_clarification, o con tu respuesta."),
            );
        }
        result.insert(crate::agent::MODEL_IMAGES_KEY.into(), json!([image_base64]));
    }
    result
}

pub fn closed_result(was_open: bool) -> Value {
    if was_open {
        json!({ "closed": true, "note": "El navegador se cerró; las sesiones iniciadas quedan guardadas en su perfil." })
    } else {
        json!({ "closed": true, "note": "El navegador ya estaba cerrado." })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(snapshot: &str) -> BrowserPage {
        BrowserPage { url: "https://example.com/".into(), title: "Example Domain".into(), snapshot: snapshot.into(), ..BrowserPage::default() }
    }

    const SNAPSHOT: &str = "- generic [active] [ref=e1]:\n  - heading \"Example Domain\" [level=1] [ref=e3]\n  - link \"Learn more\" [ref=e13] [cursor=pointer]:\n    - /url: https://iana.org/help/example-domains";

    #[test]
    fn only_web_addresses_open_and_a_bare_domain_gets_https() {
        assert_eq!(web_url("https://example.com/a?b=1").expect("https"), "https://example.com/a?b=1");
        assert_eq!(web_url("HTTP://example.com").expect("http"), "HTTP://example.com");
        assert_eq!(web_url("example.com/precios").expect("bare"), "https://example.com/precios");
        assert_eq!(web_url("localhost:5173").expect("port"), "http://localhost:5173");
        assert_eq!(web_url("example.com/a:b").expect("colon in the path"), "https://example.com/a:b");
        for blocked in ["file:///C:/Users/secreto.txt", "chrome://settings", "javascript:alert(1)", "about:blank", "data:text/html,hola", "edge://flags", ""] {
            assert!(web_url(blocked).is_err(), "{blocked}");
        }
        assert!(web_url("https://example.com/con espacio").is_err());
    }

    #[test]
    fn reads_each_tool_with_its_defaults_and_limits() {
        assert_eq!(
            BrowserCommand::from_call(NAVIGATE_TOOL, &json!({ "url": "mercadolibre.com.ar" })).expect("navigate"),
            BrowserCommand::Navigate(NavigateTarget::Url("https://mercadolibre.com.ar".into()))
        );
        assert_eq!(BrowserCommand::from_call(NAVIGATE_TOOL, &json!({ "history": "back" })).expect("back"), BrowserCommand::Navigate(NavigateTarget::Back));
        assert!(BrowserCommand::from_call(NAVIGATE_TOOL, &json!({})).is_err());
        assert!(BrowserCommand::from_call(NAVIGATE_TOOL, &json!({ "url": "a.com", "history": "back" })).is_err());
        let click = BrowserCommand::from_call(CLICK_TOOL, &json!({ "ref": "e13", "element": "Enlace «Learn more»" })).expect("click");
        assert_eq!(
            click,
            BrowserCommand::Click {
                target: ClickTarget::Element(ElementTarget { reference: "e13".into(), element: "Enlace «Learn more»".into() }),
                double: false,
                button: MouseButton::Left,
                accept_dialog: false,
            }
        );
        assert!(BrowserCommand::from_call(CLICK_TOOL, &json!({ "ref": "e13" })).is_err(), "element is required");
        assert!(BrowserCommand::from_call(CLICK_TOOL, &json!({ "ref": "button.submit", "element": "Enviar" })).is_err());
        assert!(BrowserCommand::from_call(CLICK_TOOL, &json!({ "ref": "f1e4", "element": "Botón en un iframe" })).is_ok());
        assert!(BrowserCommand::from_call(TYPE_TOOL, &json!({ "ref": "e2", "element": "Buscar" })).is_err(), "text is required");
        assert!(BrowserCommand::from_call(TYPE_TOOL, &json!({ "ref": "e2", "element": "Buscar", "text": "" })).is_ok(), "an empty text clears");
        assert!(BrowserCommand::from_call(PRESS_KEY_TOOL, &json!({ "key": "  " })).is_err());
        assert!(BrowserCommand::from_call(SELECT_OPTION_TOOL, &json!({ "ref": "e5", "element": "Provincia", "values": [] })).is_err());
        assert_eq!(
            BrowserCommand::from_call(WAIT_TOOL, &json!({ "seconds": 600 })).expect("wait"),
            BrowserCommand::Wait(WaitCondition::Time(MAX_WAIT))
        );
        assert!(BrowserCommand::from_call(WAIT_TOOL, &json!({})).is_err());
        assert_eq!(BrowserCommand::from_call(TABS_TOOL, &json!({})).expect("tabs"), BrowserCommand::Tabs(TabAction::List));
        assert!(BrowserCommand::from_call(TABS_TOOL, &json!({ "action": "select" })).is_err());
        assert!(BrowserCommand::from_call(TABS_TOOL, &json!({ "action": "new", "url": "file:///etc/passwd" })).is_err());
        assert!(BrowserCommand::from_call("run_terminal_command", &json!({})).is_err());
    }

    #[test]
    fn the_request_names_the_operation_and_its_arguments() {
        let request = BrowserCommand::from_call(TYPE_TOOL, &json!({ "ref": "e2", "element": "Buscar", "text": "zapatillas", "submit": true }))
            .expect("type")
            .agent_request(7, None)
            .expect("request");
        assert_eq!(request, json!({ "id": 7, "op": "type", "ref": "e2", "text": "zapatillas", "submit": true }));
        let wait = BrowserCommand::from_call(WAIT_TOOL, &json!({ "text": "Listo", "seconds": 5 })).expect("wait");
        assert_eq!(wait.agent_request(1, None).expect("request")["timeoutMs"], 5_000);
        assert_eq!(wait.answer_timeout(), Duration::from_secs(35));
    }

    #[test]
    fn interactions_ask_for_confirmation_and_the_rest_do_not() {
        for name in BROWSER_TOOLS {
            assert!(is_browser_tool(name));
        }
        assert!(BROWSER_INTERACTION_TOOLS.iter().all(|name| is_browser_tool(name)));
        assert!(is_browser_interaction_tool(DRAG_TOOL));
        for name in [NAVIGATE_TOOL, SNAPSHOT_TOOL, SCREENSHOT_TOOL, TABS_TOOL, WAIT_TOOL, CLOSE_TOOL] {
            assert!(!is_browser_interaction_tool(name), "{name}");
        }
    }

    #[test]
    fn the_confirmation_shows_the_element_as_the_page_has_it() {
        assert_eq!(describe_reference(SNAPSHOT, "e13").as_deref(), Some("link \"Learn more\""));
        assert_eq!(describe_reference(SNAPSHOT, "e3").as_deref(), Some("heading \"Example Domain\" [level=1]"));
        assert_eq!(describe_reference(SNAPSHOT, "e1").as_deref(), Some("generic [active]"));
        assert_eq!(describe_reference(SNAPSHOT, "e99"), None);
        let page = SeenPages { last: Some(page(SNAPSHOT)), earlier: Vec::new() };
        let click = BrowserCommand::from_call(CLICK_TOOL, &json!({ "ref": "e13", "element": "Enlace «Learn more»", "acceptDialog": true })).expect("click");
        assert_eq!(click.summary(), "Clic en el navegador: Enlace «Learn more»");
        assert_eq!(
            click.detail(&page),
            "Elemento: Enlace «Learn more»\nEn la página: link \"Learn more\"\nSi la página pide confirmar, acepta.\nPágina: Example Domain (https://example.com/)"
        );
        let typed = BrowserCommand::from_call(TYPE_TOOL, &json!({ "ref": "e13", "element": "Buscador", "text": "hola", "submit": true })).expect("type");
        assert_eq!(typed.summary(), "Escribir en el navegador: Buscador y enviar");
        assert!(typed.detail(&page).contains("Texto: hola\nDespués presiona Enter."));
    }

    #[test]
    fn an_unknown_reference_or_no_page_goes_back_to_the_model() {
        let mut seen = SeenPages::default();
        seen.remember(page(SNAPSHOT));
        assert!(check_reference(&seen, "e13").is_ok());
        assert_eq!(check_reference(&seen, "e1").map_err(|error| error.code), Ok(()));
        let unknown = check_reference(&seen, "e130").expect_err("unknown");
        assert_eq!(unknown.code, crate::error::BackendErrorCode::InvalidInput);
        assert_eq!(check_reference(&SeenPages::default(), "e13").expect_err("no page").code, crate::error::BackendErrorCode::InvalidInput);

        // The page drew the link again with another reference: the old one,
        // named in an earlier snapshot, is still accepted and goes with its
        // role and name so the process finds it again.
        seen.remember(page("- link \"Learn more\" [ref=e40]"));
        assert!(check_reference(&seen, "e13").is_ok());
        assert!(check_reference(&seen, "e1").is_err(), "an unnamed element is not found again");
        let click = BrowserCommand::from_call(CLICK_TOOL, &json!({ "ref": "e13", "element": "Learn more" })).expect("click");
        assert_eq!(click.described_target(&seen).as_deref(), Some("link \"Learn more\""));
        assert!(click.detail(&seen).contains("En la página: link \"Learn more\""));
    }

    #[test]
    fn a_long_snapshot_comes_in_parts() {
        let long = format!("{}\n- link \"fin\" [ref=e999]", "- paragraph [ref=e2]: ñandú\n".repeat(2_000));
        let mut page = page(&long);
        page.dialogs = vec!["confirm: ¿Borrar? (cancelado)".into()];
        let first = tool_result(&page, 0);
        assert_eq!(first["snapshot"].as_str().expect("part").chars().count(), SNAPSHOT_PART_CHARS);
        assert_eq!(first["snapshotPart"]["nextOffset"], SNAPSHOT_PART_CHARS);
        assert_eq!(first["dialogs"][0], "confirm: ¿Borrar? (cancelado)");
        let total = long.chars().count();
        let last = tool_result(&page, total - 10);
        assert!(last["snapshot"].as_str().expect("part").ends_with("[ref=e999]"));
        assert_eq!(last["snapshotPart"]["nextOffset"], Value::Null);
        let short = tool_result(&BrowserPage { snapshot: SNAPSHOT.into(), ..BrowserPage::default() }, 0);
        assert!(short.get("snapshotPart").is_none() && short.get("dialogs").is_none() && short.get("session").is_none());
        let login: BrowserPage = serde_json::from_value(json!({ "url": "https://www.mercadolibre.com.ar/jms/arg/lgz/login", "signIn": true })).expect("page");
        assert!(tool_result(&login, 0)["session"].as_str().expect("note").contains("pide iniciar sesión"));
    }

    #[test]
    fn a_screenshot_goes_as_an_image_and_keeps_the_last_snapshot() {
        let result = screenshot_result(&page(""), "anBn".into(), None, false);
        assert_eq!(result[crate::agent::MODEL_IMAGES_KEY], json!(["anBn"]));
        assert!(result.get("snapshot").is_none() && result.get("grid").is_none() && result.get("sentToPerson").is_none());
        assert_eq!(result["url"], "https://example.com/");
        let shared = screenshot_result(&page(""), "anBn".into(), Some(&grid()), true);
        assert_eq!(shared["grid"]["to"], "C4");
        assert!(shared["sentToPerson"].is_string());
        assert!(!BrowserCommand::Screenshot { within: None, grid: true, for_person: true }.returns_whole_snapshot());
        assert!(!BrowserCommand::Snapshot { within: Some("e3".into()), offset: 0 }.returns_whole_snapshot());
        assert!(BrowserCommand::Snapshot { within: None, offset: 20_000 }.returns_whole_snapshot());
        assert!(BrowserCommand::PressKey { key: "PageDown".into() }.returns_whole_snapshot());
    }

    /// A 4 × 3 grid of 100 px cells from (10, 20), the last column cut at 50 px.
    fn grid() -> GridSpec {
        GridSpec { x: 10.0, y: 20.0, width: 350.0, height: 300.0, cell: 100.0, columns: 4, rows: 3 }
    }

    #[test]
    fn a_cell_is_a_row_letter_and_a_column_number_clicked_in_its_middle() {
        let grid = grid();
        assert_eq!(grid.last_label(), "C4");
        assert_eq!(grid.center("A1").expect("A1"), (60.0, 70.0));
        assert_eq!(grid.center(" b 2 ").expect("b2"), (160.0, 170.0));
        // The last column is cut by the edge: its middle is inside.
        assert_eq!(grid.center("C4").expect("C4"), (335.0, 270.0));
        for outside in ["D1", "A5", "A0", "1A", "AA1", ""] {
            let error = grid.center(outside).expect_err(outside);
            assert_eq!(error.code, crate::error::BackendErrorCode::InvalidInput, "{outside}");
        }
        assert!(grid.center("Z9").expect_err("outside").message.contains("van de A1 a C4"));
    }

    #[test]
    fn cells_are_clicked_or_dragged_only_on_the_last_grid() {
        let click = BrowserCommand::from_call(CLICK_TOOL, &json!({ "cells": ["a1", "B2"], "element": "Imágenes con semáforos" })).expect("cells");
        assert_eq!(click.summary(), "Clic en el navegador en A1, B2: Imágenes con semáforos");
        assert!(click.detail(&SeenPages::default()).contains("Casillas de la grilla de la última captura: A1, B2"));
        let missing = click.check_pointers(&SeenPages::default(), None).expect_err("no grid");
        assert_eq!(missing.code, crate::error::BackendErrorCode::InvalidInput);
        assert!(click.check_pointers(&SeenPages::default(), Some(&grid())).is_ok());
        assert_eq!(
            click.agent_request(3, Some(&grid())).expect("request"),
            json!({ "id": 3, "op": "clickPoints", "points": [[60.0, 70.0], [160.0, 170.0]], "double": false, "button": "left", "acceptDialog": false })
        );
        let drag = BrowserCommand::from_call(DRAG_TOOL, &json!({ "from": "A1", "to": "A4", "element": "Deslizador del captcha" })).expect("drag");
        assert_eq!(drag.summary(), "Arrastrar en el navegador de A1 a A4: Deslizador del captcha");
        assert_eq!(drag.agent_request(4, Some(&grid())).expect("request"), json!({ "id": 4, "op": "drag", "from": [60.0, 70.0], "to": [335.0, 70.0] }));
        assert!(drag.check_pointers(&SeenPages::default(), Some(&GridSpec { columns: 2, ..grid() })).is_err());
        assert!(BrowserCommand::from_call(DRAG_TOOL, &json!({ "from": "A1", "element": "x" })).is_err());
        assert!(BrowserCommand::from_call(CLICK_TOOL, &json!({ "cells": ["A1"], "ref": "e3", "element": "x" })).is_err());
        assert!(BrowserCommand::from_call(CLICK_TOOL, &json!({ "cells": ["casilla"], "element": "x" })).is_err());
        assert!(BrowserCommand::from_call(CLICK_TOOL, &json!({ "cells": vec!["A1"; MAX_CELLS + 1], "element": "x" })).is_err());
        let shot = BrowserCommand::from_call(SCREENSHOT_TOOL, &json!({ "ref": "e25", "grid": true, "forPerson": true })).expect("screenshot");
        assert_eq!(
            shot.agent_request(5, None).expect("request"),
            json!({ "id": 5, "op": "screenshot", "ref": "e25", "grid": { "maxCells": GRID_MAX_CELLS, "minCell": GRID_MIN_CELL } })
        );
        assert!(shot.keeps_grid() && click.keeps_grid());
        assert!(!BrowserCommand::from_call(NAVIGATE_TOOL, &json!({ "url": "a.com" })).expect("navigate").keeps_grid());
    }

    #[test]
    fn a_failure_reads_as_a_step_to_fix() {
        let answer = AgentAnswer { id: 1, ok: false, page: None, image: None, grid: None, error: Some("locator.click: Timeout 15000ms exceeded.\nCall log: …".into()) };
        let error = answer.failure();
        assert_eq!(error.code, crate::error::BackendErrorCode::Conflict);
        assert_eq!(error.message, "El navegador no pudo hacer la acción: locator.click: Timeout 15000ms exceeded.");
    }
}
