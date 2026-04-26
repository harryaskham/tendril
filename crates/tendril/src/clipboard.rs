use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::error::TendrilError;
use crate::platform::{AdapterContext, AdapterInfo, DesktopSession, PlatformKind};

pub const DEFAULT_CLIPBOARD_TIMEOUT_MS: u64 = 3_000;
pub const DEFAULT_CLIPBOARD_SERVE_MS: u64 = 5_000;
const CLIPBOARD_POLL_INTERVAL: Duration = Duration::from_millis(20);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ClipboardSelection {
    Clipboard,
    Primary,
}

impl ClipboardSelection {
    pub fn parse(value: Option<&str>) -> Result<Self, TendrilError> {
        match value
            .unwrap_or("clipboard")
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "clipboard" => Ok(Self::Clipboard),
            "primary" => Ok(Self::Primary),
            other => Err(TendrilError::validation(format!(
                "unsupported clipboard selection `{other}`; expected `clipboard` or `primary`"
            ))
            .with_code("invalid_clipboard_input")
            .with_field("selection")),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Clipboard => "clipboard",
            Self::Primary => "primary",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipboardGetInput {
    pub selection: ClipboardSelection,
    pub timeout_ms: u64,
}

impl ClipboardGetInput {
    pub fn validate(&self) -> Result<(), TendrilError> {
        if self.timeout_ms == 0 {
            return Err(
                TendrilError::validation("timeout_ms must be greater than zero")
                    .with_code("invalid_clipboard_input")
                    .with_field("timeout_ms"),
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipboardSetInput {
    pub selection: ClipboardSelection,
    pub text: String,
    pub serve_ms: u64,
}

impl ClipboardSetInput {
    pub fn validate(&self) -> Result<(), TendrilError> {
        if self.text.is_empty() {
            return Err(TendrilError::validation("clipboard text cannot be empty")
                .with_code("invalid_clipboard_input")
                .with_field("text"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipboardGetOutput {
    pub adapter: AdapterInfo,
    pub selection: ClipboardSelection,
    pub text: String,
    pub text_len: usize,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipboardSetOutput {
    pub adapter: AdapterInfo,
    pub selection: ClipboardSelection,
    pub text_len: usize,
    pub serve_ms: u64,
    pub served_requests: usize,
    pub notes: Vec<String>,
}

pub fn execute_clipboard_get(
    input: &ClipboardGetInput,
) -> Result<ClipboardGetOutput, TendrilError> {
    input.validate()?;
    let context = AdapterContext::detect();
    ensure_x11_clipboard_supported(&context)?;
    let (text, notes) =
        x11_get_selection(input.selection, Duration::from_millis(input.timeout_ms))?;
    Ok(ClipboardGetOutput {
        adapter: AdapterInfo::from_context(&context),
        selection: input.selection,
        text_len: text.len(),
        text,
        notes,
    })
}

pub fn execute_clipboard_set(
    input: &ClipboardSetInput,
) -> Result<ClipboardSetOutput, TendrilError> {
    input.validate()?;
    let context = AdapterContext::detect();
    ensure_x11_clipboard_supported(&context)?;
    let (served_requests, notes) = x11_set_selection(
        input.selection,
        &input.text,
        Duration::from_millis(input.serve_ms),
    )?;
    Ok(ClipboardSetOutput {
        adapter: AdapterInfo::from_context(&context),
        selection: input.selection,
        text_len: input.text.len(),
        serve_ms: input.serve_ms,
        served_requests,
        notes,
    })
}

#[must_use]
pub fn render_clipboard_get_human(output: &ClipboardGetOutput) -> String {
    let notes = if output.notes.is_empty() {
        String::from("none")
    } else {
        output.notes.join(" ")
    };
    format!(
        "clipboard selection: {:?}\nplatform: {:?} / {:?}\ntext_len: {}\ntext: {}\nnotes: {}\n",
        output.selection,
        output.adapter.platform,
        output.adapter.session,
        output.text_len,
        output.text,
        notes,
    )
}

#[must_use]
pub fn render_clipboard_set_human(output: &ClipboardSetOutput) -> String {
    let notes = if output.notes.is_empty() {
        String::from("none")
    } else {
        output.notes.join(" ")
    };
    format!(
        "clipboard selection: {:?}\nplatform: {:?} / {:?}\ntext_len: {}\nserve_ms: {}\nserved_requests: {}\nnotes: {}\n",
        output.selection,
        output.adapter.platform,
        output.adapter.session,
        output.text_len,
        output.serve_ms,
        output.served_requests,
        notes,
    )
}

fn ensure_x11_clipboard_supported(context: &AdapterContext) -> Result<(), TendrilError> {
    if context.platform == PlatformKind::Linux && context.session == DesktopSession::X11 {
        return Ok(());
    }

    Err(TendrilError::unsupported_capability(
        "clipboard_not_supported",
        "explicit clipboard get/set is currently implemented for Linux/X11 selections only",
        Some(json!({
            "platform": context.platform,
            "session": context.session,
            "supported": [{"platform": "linux", "session": "x11"}],
            "suggested_action": "Use the headless X11 micro-environment for deterministic browser↔OS clipboard smokes. For Wayland/macOS/Windows, use the platform clipboard tool directly or file a platform-specific Tendril clipboard backend bead."
        })),
    ))
}

#[cfg(target_os = "linux")]
fn x11_get_selection(
    selection: ClipboardSelection,
    timeout: Duration,
) -> Result<(String, Vec<String>), TendrilError> {
    x11_impl::get_selection(selection, timeout)
}

#[cfg(not(target_os = "linux"))]
fn x11_get_selection(
    selection: ClipboardSelection,
    _timeout: Duration,
) -> Result<(String, Vec<String>), TendrilError> {
    Err(TendrilError::unsupported_capability(
        "clipboard_not_supported",
        "X11 clipboard support is only compiled on Linux",
        Some(json!({ "selection": selection.as_str() })),
    ))
}

#[cfg(target_os = "linux")]
fn x11_set_selection(
    selection: ClipboardSelection,
    text: &str,
    serve_for: Duration,
) -> Result<(usize, Vec<String>), TendrilError> {
    x11_impl::set_selection(selection, text, serve_for)
}

#[cfg(not(target_os = "linux"))]
fn x11_set_selection(
    selection: ClipboardSelection,
    _text: &str,
    _serve_for: Duration,
) -> Result<(usize, Vec<String>), TendrilError> {
    Err(TendrilError::unsupported_capability(
        "clipboard_not_supported",
        "X11 clipboard support is only compiled on Linux",
        Some(json!({ "selection": selection.as_str() })),
    ))
}

#[cfg(target_os = "linux")]
mod x11_impl {
    use super::{CLIPBOARD_POLL_INTERVAL, ClipboardSelection};
    use crate::error::TendrilError;
    use serde_json::json;
    use std::time::{Duration, Instant};
    use x11rb::CURRENT_TIME;
    use x11rb::atom_manager;
    use x11rb::connection::Connection;
    use x11rb::protocol::Event;
    use x11rb::protocol::xproto::{
        Atom, AtomEnum, ConnectionExt as _, CreateWindowAux, EventMask, PropMode,
        SelectionNotifyEvent, SelectionRequestEvent, Window, WindowClass,
    };
    use x11rb::wrapper::ConnectionExt as _;

    atom_manager! {
        Atoms: AtomsCookie {
            CLIPBOARD,
            TARGETS,
            TEXT,
            UTF8_STRING,
            TENDRIL_CLIPBOARD,
        }
    }

    struct X11ClipboardConnection {
        conn: x11rb::rust_connection::RustConnection,
        screen_num: usize,
        atoms: Atoms,
    }

    impl X11ClipboardConnection {
        fn connect() -> Result<Self, TendrilError> {
            let (conn, screen_num) = x11rb::connect(None).map_err(|error| {
                TendrilError::execution_failure(
                    "clipboard_x11_connect_failed",
                    format!(
                        "failed to connect to the active X11 display for clipboard access: {error}"
                    ),
                    None,
                )
            })?;
            let atoms = Atoms::new(&conn)
                .map_err(|error| {
                    TendrilError::execution_failure(
                        "clipboard_x11_atom_failed",
                        format!("failed to request X11 clipboard atoms: {error}"),
                        None,
                    )
                })?
                .reply()
                .map_err(|error| {
                    TendrilError::execution_failure(
                        "clipboard_x11_atom_failed",
                        format!("failed to intern X11 clipboard atoms: {error}"),
                        None,
                    )
                })?;
            Ok(Self {
                conn,
                screen_num,
                atoms,
            })
        }

        fn root(&self) -> Result<Window, TendrilError> {
            self.conn
                .setup()
                .roots
                .get(self.screen_num)
                .map(|screen| screen.root)
                .ok_or_else(|| {
                    TendrilError::execution_failure(
                        "clipboard_x11_screen_missing",
                        format!(
                            "X11 screen index `{}` was not present in setup",
                            self.screen_num
                        ),
                        None,
                    )
                })
        }

        fn create_helper_window(&self) -> Result<Window, TendrilError> {
            let window = self.conn.generate_id().map_err(|error| {
                TendrilError::execution_failure(
                    "clipboard_x11_window_failed",
                    format!("failed to allocate an X11 helper window id: {error}"),
                    None,
                )
            })?;
            self.conn
                .create_window(
                    0,
                    window,
                    self.root()?,
                    0,
                    0,
                    1,
                    1,
                    0,
                    WindowClass::COPY_FROM_PARENT,
                    0,
                    &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE),
                )
                .map_err(|error| {
                    TendrilError::execution_failure(
                        "clipboard_x11_window_failed",
                        format!("failed to create an X11 clipboard helper window: {error}"),
                        None,
                    )
                })?;
            Ok(window)
        }

        fn selection_atom(&self, selection: ClipboardSelection) -> Atom {
            match selection {
                ClipboardSelection::Clipboard => self.atoms.CLIPBOARD,
                ClipboardSelection::Primary => AtomEnum::PRIMARY.into(),
            }
        }
    }

    pub fn get_selection(
        selection: ClipboardSelection,
        timeout: Duration,
    ) -> Result<(String, Vec<String>), TendrilError> {
        let x11 = X11ClipboardConnection::connect()?;
        let selection_atom = x11.selection_atom(selection);
        let owner = x11
            .conn
            .get_selection_owner(selection_atom)
            .map_err(|error| clipboard_error("clipboard_query_failed", selection, error))?
            .reply()
            .map_err(|error| clipboard_error("clipboard_query_failed", selection, error))?
            .owner;
        if owner == 0 {
            return Err(TendrilError::execution_failure(
                "clipboard_selection_unowned",
                format!(
                    "X11 {} selection has no owner; copy text in the source application first or use `tendril clipboard set --text ...`",
                    selection.as_str()
                ),
                None,
            )
            .with_detail_entry("selection", json!(selection.as_str()))
            .with_detail_entry(
                "suggested_action",
                json!("Copy from the browser with Ctrl+C, then immediately run `tendril clipboard get --json`; X11 selection ownership is process-owned and may disappear when the source exits."),
            ));
        }

        let window = x11.create_helper_window()?;
        let property = x11.atoms.TENDRIL_CLIPBOARD;
        let deadline = Instant::now() + timeout;
        let mut attempted_targets = Vec::new();
        for (target_atom, target_name) in [
            (x11.atoms.UTF8_STRING, "UTF8_STRING"),
            (AtomEnum::STRING.into(), "STRING"),
            (x11.atoms.TEXT, "TEXT"),
        ] {
            attempted_targets.push(target_name.to_owned());
            match read_target(
                &x11,
                window,
                selection,
                selection_atom,
                target_atom,
                property,
                deadline,
            ) {
                Ok(Some(text)) => {
                    let notes = vec![format!(
                        "Read X11 {} selection from owner 0x{owner:x} using target {target_name}.",
                        selection.as_str()
                    )];
                    let _ = x11.conn.destroy_window(window);
                    let _ = x11.conn.flush();
                    return Ok((text, notes));
                }
                Ok(None) => {}
                Err(error) => {
                    let _ = x11.conn.destroy_window(window);
                    let _ = x11.conn.flush();
                    return Err(error);
                }
            }
        }

        let _ = x11.conn.destroy_window(window);
        let _ = x11.conn.flush();
        Err(TendrilError::execution_failure(
            "clipboard_conversion_failed",
            format!(
                "X11 {} selection owner 0x{owner:x} did not provide UTF-8 or STRING text data",
                selection.as_str()
            ),
            None,
        )
        .with_detail_entry("selection", json!(selection.as_str()))
        .with_detail_entry("owner", json!(format!("0x{owner:x}")))
        .with_detail_entry("attempted_targets", json!(attempted_targets))
        .with_detail_entry(
            "suggested_action",
            json!("The source application may own a non-text selection or use a large INCR transfer not yet supported by Tendril; retry with a plain text selection or file a backend-specific clipboard bead."),
        ))
    }

    fn read_target(
        x11: &X11ClipboardConnection,
        window: Window,
        selection_kind: ClipboardSelection,
        selection: Atom,
        target: Atom,
        property: Atom,
        deadline: Instant,
    ) -> Result<Option<String>, TendrilError> {
        x11.conn
            .delete_property(window, property)
            .map_err(|error| {
                clipboard_error("clipboard_property_delete_failed", selection_kind, error)
            })?;
        x11.conn
            .convert_selection(window, selection, target, property, CURRENT_TIME)
            .map_err(|error| clipboard_error("clipboard_convert_failed", selection_kind, error))?;
        x11.conn.flush().map_err(|error| {
            TendrilError::execution_failure(
                "clipboard_x11_flush_failed",
                format!("failed to flush X11 clipboard conversion request: {error}"),
                None,
            )
        })?;

        let notify = wait_for_selection_notify(&x11.conn, window, selection, target, deadline)?;
        if notify.property == 0 {
            return Ok(None);
        }

        let reply = x11
            .conn
            .get_property(true, window, notify.property, AtomEnum::ANY, 0, u32::MAX)
            .map_err(|error| {
                clipboard_error("clipboard_property_read_failed", selection_kind, error)
            })?
            .reply()
            .map_err(|error| {
                clipboard_error("clipboard_property_read_failed", selection_kind, error)
            })?;

        if reply.type_ == x11.atoms.TARGETS {
            return Ok(None);
        }
        if atom_name(&x11.conn, reply.type_).as_deref() == Some("INCR") {
            return Err(TendrilError::execution_failure(
                "clipboard_incr_not_supported",
                "X11 clipboard owner requested an incremental INCR transfer; Tendril currently supports direct text selections only",
                None,
            )
            .with_detail_entry(
                "suggested_action",
                json!("Retry with a smaller plain-text clipboard payload or file a follow-up for X11 INCR clipboard transfers."),
            ));
        }

        Ok(Some(String::from_utf8_lossy(&reply.value).to_string()))
    }

    fn wait_for_selection_notify(
        conn: &x11rb::rust_connection::RustConnection,
        window: Window,
        selection: Atom,
        target: Atom,
        deadline: Instant,
    ) -> Result<SelectionNotifyEvent, TendrilError> {
        loop {
            if Instant::now() >= deadline {
                return Err(TendrilError::timeout(
                    "clipboard_timeout",
                    "timed out waiting for the X11 selection owner to provide clipboard data",
                    Some(json!({
                        "target": atom_name(conn, target),
                        "suggested_action": "Verify the source application still owns the clipboard and retry with a plain text selection."
                    })),
                ));
            }
            if let Some(event) = conn.poll_for_event().map_err(|error| {
                TendrilError::execution_failure(
                    "clipboard_x11_event_failed",
                    format!("failed while polling X11 clipboard events: {error}"),
                    None,
                )
            })? {
                if let Event::SelectionNotify(event) = event
                    && event.requestor == window
                    && event.selection == selection
                    && event.target == target
                {
                    return Ok(event);
                }
            } else {
                std::thread::sleep(CLIPBOARD_POLL_INTERVAL);
            }
        }
    }

    pub fn set_selection(
        selection: ClipboardSelection,
        text: &str,
        serve_for: Duration,
    ) -> Result<(usize, Vec<String>), TendrilError> {
        let x11 = X11ClipboardConnection::connect()?;
        let selection_atom = x11.selection_atom(selection);
        let window = x11.create_helper_window()?;
        x11.conn
            .set_selection_owner(window, selection_atom, CURRENT_TIME)
            .map_err(|error| clipboard_error("clipboard_set_owner_failed", selection, error))?;
        x11.conn.flush().map_err(|error| {
            TendrilError::execution_failure(
                "clipboard_x11_flush_failed",
                format!("failed to flush X11 clipboard ownership request: {error}"),
                None,
            )
        })?;
        let owner = x11
            .conn
            .get_selection_owner(selection_atom)
            .map_err(|error| clipboard_error("clipboard_query_failed", selection, error))?
            .reply()
            .map_err(|error| clipboard_error("clipboard_query_failed", selection, error))?
            .owner;
        if owner != window {
            return Err(TendrilError::execution_failure(
                "clipboard_set_owner_failed",
                format!(
                    "failed to own X11 {} selection; current owner is 0x{owner:x}, expected helper window 0x{window:x}",
                    selection.as_str()
                ),
                None,
            )
            .with_detail_entry("selection", json!(selection.as_str())));
        }

        let mut notes = vec![format!(
            "Owned X11 {} selection as helper window 0x{window:x}; serving requests for {}ms.",
            selection.as_str(),
            serve_for.as_millis()
        )];
        if serve_for.is_zero() {
            notes.push(
                "serve_ms=0 releases the selection when this process exits; consumers must request before exit for a persistent paste.".to_owned(),
            );
        }
        let deadline = Instant::now() + serve_for;
        let mut served_requests = 0;
        while Instant::now() < deadline {
            if let Some(event) = x11.conn.poll_for_event().map_err(|error| {
                TendrilError::execution_failure(
                    "clipboard_x11_event_failed",
                    format!("failed while polling X11 clipboard events: {error}"),
                    None,
                )
            })? {
                match event {
                    Event::SelectionRequest(request) => {
                        if request.owner == window && request.selection == selection_atom {
                            serve_selection_request(&x11, &request, text)?;
                            served_requests += 1;
                        }
                    }
                    Event::SelectionClear(event) if event.owner == window => {
                        notes.push(
                            "Selection ownership was replaced by another X11 client before serve_ms elapsed.".to_owned(),
                        );
                        break;
                    }
                    _ => {}
                }
            } else {
                std::thread::sleep(CLIPBOARD_POLL_INTERVAL);
            }
        }

        let _ = x11.conn.destroy_window(window);
        let _ = x11.conn.flush();
        Ok((served_requests, notes))
    }

    fn serve_selection_request(
        x11: &X11ClipboardConnection,
        request: &SelectionRequestEvent,
        text: &str,
    ) -> Result<(), TendrilError> {
        let property = if request.property == 0 {
            request.target
        } else {
            request.property
        };
        let mut response_property = property;

        if request.target == x11.atoms.TARGETS {
            let targets = [
                x11.atoms.UTF8_STRING,
                AtomEnum::STRING.into(),
                x11.atoms.TEXT,
                x11.atoms.TARGETS,
            ];
            x11.conn
                .change_property32(
                    PropMode::REPLACE,
                    request.requestor,
                    property,
                    AtomEnum::ATOM,
                    &targets,
                )
                .map_err(|error| {
                    clipboard_error(
                        "clipboard_serve_failed",
                        ClipboardSelection::Clipboard,
                        error,
                    )
                })?;
        } else if request.target == x11.atoms.UTF8_STRING || request.target == x11.atoms.TEXT {
            x11.conn
                .change_property8(
                    PropMode::REPLACE,
                    request.requestor,
                    property,
                    x11.atoms.UTF8_STRING,
                    text.as_bytes(),
                )
                .map_err(|error| {
                    clipboard_error(
                        "clipboard_serve_failed",
                        ClipboardSelection::Clipboard,
                        error,
                    )
                })?;
        } else if request.target == u32::from(AtomEnum::STRING) {
            x11.conn
                .change_property8(
                    PropMode::REPLACE,
                    request.requestor,
                    property,
                    AtomEnum::STRING,
                    text.as_bytes(),
                )
                .map_err(|error| {
                    clipboard_error(
                        "clipboard_serve_failed",
                        ClipboardSelection::Clipboard,
                        error,
                    )
                })?;
        } else {
            response_property = AtomEnum::NONE.into();
        }

        let notify = SelectionNotifyEvent {
            response_type: x11rb::protocol::xproto::SELECTION_NOTIFY_EVENT,
            sequence: 0,
            time: request.time,
            requestor: request.requestor,
            selection: request.selection,
            target: request.target,
            property: response_property,
        };
        x11.conn
            .send_event(false, request.requestor, EventMask::NO_EVENT, notify)
            .map_err(|error| {
                clipboard_error(
                    "clipboard_serve_notify_failed",
                    ClipboardSelection::Clipboard,
                    error,
                )
            })?;
        x11.conn.flush().map_err(|error| {
            TendrilError::execution_failure(
                "clipboard_x11_flush_failed",
                format!("failed to flush X11 clipboard response: {error}"),
                None,
            )
        })?;
        Ok(())
    }

    fn clipboard_error(
        code: &'static str,
        selection: ClipboardSelection,
        error: impl std::fmt::Display,
    ) -> TendrilError {
        TendrilError::execution_failure(
            code,
            format!(
                "X11 {} clipboard operation failed: {error}",
                selection.as_str()
            ),
            None,
        )
        .with_detail_entry("selection", json!(selection.as_str()))
    }

    fn atom_name(conn: &x11rb::rust_connection::RustConnection, atom: Atom) -> Option<String> {
        if atom == 0 {
            return None;
        }
        conn.get_atom_name(atom)
            .ok()?
            .reply()
            .ok()
            .and_then(|reply| String::from_utf8(reply.name).ok())
            .filter(|name| !name.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_clipboard_selection_names() {
        assert_eq!(
            ClipboardSelection::parse(None).expect("default selection"),
            ClipboardSelection::Clipboard
        );
        assert_eq!(
            ClipboardSelection::parse(Some("primary")).expect("primary selection"),
            ClipboardSelection::Primary
        );
        assert!(ClipboardSelection::parse(Some("secondary")).is_err());
    }

    #[test]
    fn validates_clipboard_inputs() {
        assert!(
            ClipboardGetInput {
                selection: ClipboardSelection::Clipboard,
                timeout_ms: 1,
            }
            .validate()
            .is_ok()
        );
        assert!(
            ClipboardGetInput {
                selection: ClipboardSelection::Clipboard,
                timeout_ms: 0,
            }
            .validate()
            .is_err()
        );
        assert!(
            ClipboardSetInput {
                selection: ClipboardSelection::Clipboard,
                text: "hello".to_owned(),
                serve_ms: 0,
            }
            .validate()
            .is_ok()
        );
        assert!(
            ClipboardSetInput {
                selection: ClipboardSelection::Clipboard,
                text: String::new(),
                serve_ms: 0,
            }
            .validate()
            .is_err()
        );
    }
}
