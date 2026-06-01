use std::ffi::OsString;
use std::fmt;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read};
use std::ops::Deref;
use std::os::fd::OwnedFd;
use std::sync::Arc;

use dpi::{LogicalPosition, PhysicalPosition};
use sctk::data_device_manager::data_device::{DataDeviceData, DataDeviceHandler};
use sctk::data_device_manager::data_offer::{DataOfferHandler, DragOffer};
use sctk::data_device_manager::data_source::DataSourceHandler;
use wayland_client::protocol::wl_data_device::WlDataDevice;
use wayland_client::protocol::wl_data_device_manager::DndAction;
use wayland_client::protocol::wl_data_offer::WlDataOffer;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Proxy, QueueHandle};
use winit_core::data_transfer::{DataTransfer, DataTransferId, TransferType, TypeHint, TypedData};
use winit_core::event::WindowEvent;
use winit_core::event_loop::{DndActionMask, DndActions};
use winit_core::window::WindowId;

use crate::state::WinitState;

impl DataSourceHandler for WinitState {
    fn accept_mime(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        source: &wayland_client::protocol::wl_data_source::WlDataSource,
        mime: Option<String>,
    ) {
        let _ = mime;
        let _ = source;
        let _ = qh;
        let _ = conn;
        // Not implemented, but required for `DataDeviceHandler`.
    }

    fn send_request(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        source: &wayland_client::protocol::wl_data_source::WlDataSource,
        mime: String,
        fd: sctk::data_device_manager::WritePipe,
    ) {
        let _ = fd;
        let _ = mime;
        let _ = source;
        let _ = qh;
        let _ = conn;
        // Not implemented, but required for `DataDeviceHandler`.
    }

    fn cancelled(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        source: &wayland_client::protocol::wl_data_source::WlDataSource,
    ) {
        let _ = source;
        let _ = qh;
        let _ = conn;
        // Not implemented, but required for `DataDeviceHandler`.
    }

    fn dnd_dropped(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        source: &wayland_client::protocol::wl_data_source::WlDataSource,
    ) {
        let _ = source;
        let _ = qh;
        let _ = conn;
        // Not implemented, but required for `DataDeviceHandler`.
    }

    fn dnd_finished(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        source: &wayland_client::protocol::wl_data_source::WlDataSource,
    ) {
        let _ = source;
        let _ = qh;
        let _ = conn;
        // Not implemented, but required for `DataDeviceHandler`.
    }

    fn action(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        source: &wayland_client::protocol::wl_data_source::WlDataSource,
        action: wayland_client::protocol::wl_data_device_manager::DndAction,
    ) {
        let _ = action;
        let _ = source;
        let _ = qh;
        let _ = conn;
        // Not implemented, but required for `DataDeviceHandler`.
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Hash)]
enum Charset {
    Utf8,
    Utf16,
}

#[derive(Debug, PartialEq, Eq, Clone, Hash)]
pub struct MimeType {
    mime: Arc<str>,
    hint: Option<TypeHint>,
}

// MIME types
// Files
const TEXT_URI_LIST: &str = "text/uri-list";
// Plaintext
const TEXT_PLAIN: &str = "text/plain";
const TEXT_PLAIN_CHARSET_UTF8: &str = "text/plain;charset=utf-8";
// HTML
const TEXT_HTML: &str = "text/html";
const TEXT_HTML_CHARSET_UTF8: &str = "text/html;charset=utf-8";
// RTF
const APPLICATION_RTF: &str = "application/rtf";
// Audio
const AUDIO_AAC: &str = "audio/aac";
const AUDIO_AIFF: &str = "audio/aiff";
const AUDIO_FLAC: &str = "audio/flac";
const AUDIO_WAV: &str = "audio/wav";
const AUDIO_WAVE: &str = "audio/wave";
const AUDIO_X_WAV: &str = "audio/x-wav";
const AUDIO_VND_WAV: &str = "audio/vnd.wav";
const AUDIO_VND_WAVE: &str = "audio/vnd.wave";
const AUDIO_MPEG: &str = "audio/mpeg";
const AUDIO_OGG: &str = "audio/ogg";
// Image
const IMAGE_BMP: &str = "image/bmp";
const IMAGE_GIF: &str = "image/gif";
const IMAGE_JPEG: &str = "image/jpeg";
const IMAGE_PJPEG: &str = "image/pjpeg";
const IMAGE_PNG: &str = "image/png";
const IMAGE_SVG: &str = "image/svg+xml";
const IMAGE_TIFF: &str = "image/tiff";
const IMAGE_WEBP: &str = "image/webp";
const IMAGE_X_ICON: &str = "image/x-icon";
const IMAGE_RAW: &str = "image/x-panasonic-raw";

impl MimeType {
    const MIME_HINT_MAP: &[(&str, TypeHint)] = &[
        // Files
        (TEXT_URI_LIST, TypeHint::UriList),
        // Plaintext
        (TEXT_PLAIN, TypeHint::Plaintext),
        (TEXT_PLAIN_CHARSET_UTF8, TypeHint::Plaintext),
        // HTML
        (TEXT_HTML, TypeHint::Html),
        (TEXT_HTML_CHARSET_UTF8, TypeHint::Html),
        // RTF
        (APPLICATION_RTF, TypeHint::Rtf),
        // Audio
        (AUDIO_AAC, TypeHint::Audio { extension_hint: Some("aac") }),
        (AUDIO_AIFF, TypeHint::Audio { extension_hint: Some("aif") }),
        (AUDIO_FLAC, TypeHint::Audio { extension_hint: Some("flac") }),
        (AUDIO_VND_WAV, TypeHint::Audio { extension_hint: Some("wav") }),
        (AUDIO_VND_WAVE, TypeHint::Audio { extension_hint: Some("wav") }),
        (AUDIO_WAV, TypeHint::Audio { extension_hint: Some("wav") }),
        (AUDIO_WAVE, TypeHint::Audio { extension_hint: Some("wav") }),
        (AUDIO_X_WAV, TypeHint::Audio { extension_hint: Some("wav") }),
        (AUDIO_OGG, TypeHint::Audio { extension_hint: Some("ogg") }),
        (AUDIO_MPEG, TypeHint::Audio { extension_hint: Some("mp3") }),
        // Image
        (IMAGE_BMP, TypeHint::Image { extension_hint: Some("bmp") }),
        (IMAGE_GIF, TypeHint::Image { extension_hint: Some("gif") }),
        (IMAGE_JPEG, TypeHint::Image { extension_hint: Some("jpg") }),
        (IMAGE_PJPEG, TypeHint::Image { extension_hint: Some("jpg") }),
        (IMAGE_PNG, TypeHint::Image { extension_hint: Some("png") }),
        (IMAGE_RAW, TypeHint::Image { extension_hint: Some("raw") }),
        (IMAGE_SVG, TypeHint::Image { extension_hint: Some("svg") }),
        (IMAGE_TIFF, TypeHint::Image { extension_hint: Some("tiff") }),
        (IMAGE_WEBP, TypeHint::Image { extension_hint: Some("webp") }),
        (IMAGE_X_ICON, TypeHint::Image { extension_hint: Some("ico") }),
    ];

    fn charset(&self) -> Option<Charset> {
        let (_essence, options) = self.mime.split_once(';')?;

        let (_, charset) = options.split_once("charset=")?;

        if charset.starts_with("utf-8") {
            Some(Charset::Utf8)
        } else if charset.starts_with("utf-16") {
            Some(Charset::Utf16)
        } else {
            None
        }
    }

    fn parse(mime: String) -> Self {
        let hint = Self::MIME_HINT_MAP
            .iter()
            .find_map(|(haystack, hint)| (*haystack == &*mime).then_some(*hint))
            .or_else(|| {
                if mime.starts_with("image/") {
                    Some(TypeHint::Image { extension_hint: None })
                } else if mime.starts_with("audio/") {
                    Some(TypeHint::Audio { extension_hint: None })
                } else {
                    None
                }
            });

        Self { mime: mime.into(), hint }
    }
}

impl fmt::Display for MimeType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.mime.fmt(f)
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct UnknownTypeHint(pub TypeHint);

impl fmt::Display for UnknownTypeHint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Unknown type hint: {:?}", self.0)
    }
}

impl TryFrom<TypeHint> for MimeType {
    type Error = UnknownTypeHint;

    fn try_from(hint: TypeHint) -> Result<Self, Self::Error> {
        let mime = Self::MIME_HINT_MAP
            .iter()
            .find_map(|(mime, haystack)| (*haystack == hint).then_some(*mime))
            .ok_or(UnknownTypeHint(hint))?;

        Ok(Self { mime: mime.to_owned().into(), hint: Some(hint) })
    }
}

impl TransferType for MimeType {
    fn hint(&self) -> Option<TypeHint> {
        self.hint
    }

    fn matches(&self, other: &dyn TransferType) -> bool {
        if let Some(other_mime) = other.cast_ref::<Self>() {
            *self == *other_mime
        } else {
            // If either hint is `None`, return false
            self.hint().is_some_and(|hint| other.hint() == Some(hint))
        }
    }
}

#[derive(Debug)]
pub struct MimeData {
    mime_type: MimeType,
    fd: Option<OwnedFd>,
}

impl MimeData {
    pub(crate) fn new(fd: OwnedFd, mime_type: MimeType) -> Self {
        Self { mime_type, fd: Some(fd) }
    }

    fn try_as_file(&mut self) -> Option<File> {
        let fd_clone =
            // TODO: Is it ok that this may only work once, depending on what the fd points to?
            if let Ok(cloned) = self.fd.as_ref()?.try_clone() { cloned } else { self.fd.take()? };
        Some(fd_clone.into())
    }
}

impl TypedData for MimeData {
    fn type_(&self) -> &dyn TransferType {
        &self.mime_type
    }

    fn try_read(&mut self) -> Option<Box<dyn io::BufRead>> {
        Some(Box::new(BufReader::new(self.try_as_file()?)))
    }

    fn try_as_uris(&mut self) -> io::Result<Vec<OsString>> {
        let Some(file) = self.try_as_file() else {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "This `MimeData` was already read, and the underlying file descriptor does not \
                 support cloning",
            ));
        };

        BufReader::new(file).lines().map(|res| res.map(OsString::from)).collect()
    }

    fn try_as_string(&mut self) -> io::Result<String> {
        let Some(mut file) = self.try_as_file() else {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "This `MimeData` was already read, and the underlying file descriptor does not \
                 support cloning",
            ));
        };

        // Default charset is UTF-16 for some reason
        let charset = self.mime_type.charset().unwrap_or(Charset::Utf16);

        match charset {
            Charset::Utf8 => {
                let mut out = String::new();
                file.read_to_string(&mut out)?;

                Ok(out)
            },
            Charset::Utf16 => {
                let mut bytes = Vec::<u8>::new();
                file.read_to_end(&mut bytes)?;

                // TODO: `from_utf16le` once it's stable
                let utf_16 = bytes
                    .chunks_exact(2)
                    .map(|chunk| {
                        let arr: [u8; 2] = chunk.try_into().unwrap();
                        u16::from_le_bytes(arr)
                    })
                    .collect::<Vec<_>>();

                String::from_utf16(&utf_16)
                    .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
            },
        }
    }
}

#[derive(Debug, Clone)]
pub struct CurrentDrag {
    mime_types: Arc<[MimeType]>,
    accepted_type: Option<MimeType>,
    data: WlDataOffer,
    transfer_id: DataTransferId,
    window_id: WindowId,
}

impl CurrentDrag {
    pub(crate) fn transfer_id(&self) -> DataTransferId {
        self.transfer_id
    }

    pub(crate) fn window_id(&self) -> WindowId {
        self.window_id
    }

    pub(crate) fn set_actions(&self, action_set: &DndActionSet) {
        self.data.set_actions(action_set.dnd_actions, action_set.preferred_action());
    }

    pub(crate) fn find_type_dyn<'a>(&'a self, type_: &'a dyn TransferType) -> Option<&'a MimeType> {
        match type_.cast_ref::<MimeType>() {
            Some(mime_type) => Some(mime_type),
            None => {
                let hint = type_.hint()?;
                self.mime_types.iter().find(|mime_type| {
                    mime_type.hint().is_some_and(|haystack| haystack.matches(&hint))
                })
            },
        }
    }
}

impl Deref for CurrentDrag {
    type Target = WlDataOffer;

    fn deref(&self) -> &Self::Target {
        &self.data
    }
}

impl DataTransfer for CurrentDrag {
    fn for_each_available_type<'this>(
        &'this self,
        func: &'_ mut dyn FnMut(&'this dyn TransferType) -> std::ops::ControlFlow<()>,
    ) {
        let _ = self.mime_types.iter().map(|mime| mime as &dyn TransferType).try_for_each(func);
    }
}

#[derive(Debug, Default)]
pub struct DndState {
    current_drag: Option<CurrentDrag>,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct DndActionSet {
    /// The set of available actions.
    pub dnd_actions: DndAction,
    /// If set, the preferred action.
    preferred_action: Option<DndAction>,
}

fn guess_preferred_action(action: DndAction) -> DndAction {
    [DndAction::Move, DndAction::Copy, DndAction::Ask]
        .into_iter()
        .find(|preferred| preferred.intersects(action))
        .unwrap_or(DndAction::empty())
}

impl DndActionSet {
    pub fn empty() -> Self {
        Self { dnd_actions: DndAction::empty(), preferred_action: None }
    }

    pub(crate) fn from_dyn(mask: &dyn DndActionMask) -> Self {
        mask.cast_ref::<Self>().copied().unwrap_or_else(|| mask.hint().into())
    }

    pub fn preferred_action(&self) -> DndAction {
        self.preferred_action.unwrap_or_else(|| guess_preferred_action(self.dnd_actions))
    }

    pub fn intersection(&self, other: &Self) -> Self {
        let preferred_action = match (self.preferred_action, other.preferred_action) {
            (Some(this_pref), Some(other_pref)) if this_pref.intersects(other_pref) => {
                Some(this_pref.intersection(other_pref))
            },
            (Some(pref), None) | (None, Some(pref)) => Some(pref),
            // If the preferences do not intersect, calculate it from the actions.
            _ => None,
        };

        let dnd_actions = self.dnd_actions.intersection(other.dnd_actions);

        Self { dnd_actions, preferred_action }
    }
}

impl DndActionMask for DndActionSet {
    fn hint(&self) -> DndActions {
        if self.dnd_actions.is_all() {
            DndActions::All
        } else {
            DndActions::Flags {
                move_: self.dnd_actions.contains(DndAction::Move),
                copy: self.dnd_actions.contains(DndAction::Copy),
                link: false,
            }
        }
    }

    fn intersection(&self, other: &dyn DndActionMask) -> Box<dyn DndActionMask> {
        Box::new(self.intersection(&Self::from_dyn(other)))
    }

    fn is_empty(&self) -> bool {
        self.dnd_actions.is_empty()
    }

    fn intersects(&self, other: &dyn DndActionMask) -> bool {
        !self.intersection(&Self::from_dyn(other)).is_empty()
    }
}

impl From<DndActions> for DndActionSet {
    fn from(value: DndActions) -> Self {
        let copy_flag = if value.copy() { DndAction::Copy } else { DndAction::empty() };
        let move_flag = if value.move_() { DndAction::Move } else { DndAction::empty() };

        DndActionSet { dnd_actions: copy_flag | move_flag, preferred_action: None }
    }
}

impl DndState {
    pub(crate) fn current_drag(&self) -> Option<&CurrentDrag> {
        self.current_drag.as_ref()
    }

    pub(crate) fn accept_type(&mut self, mime_type: MimeType) {
        if let Some(cur) = self.current_drag.as_mut() {
            cur.accepted_type = Some(mime_type);
        }
    }
}

impl DataOfferHandler for WinitState {
    fn source_actions(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        offer: &mut DragOffer,
        actions: DndAction,
    ) {
        let _ = actions;
        let _ = offer;
        let _ = qh;
        let _ = conn;
        // Not implemented, but required for `DataDeviceHandler`.
    }

    fn selected_action(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        offer: &mut DragOffer,
        actions: DndAction,
    ) {
        let _ = actions;
        let _ = offer;
        let _ = qh;
        let _ = conn;
        // Not implemented, but required for `DataDeviceHandler`.
    }
}

impl DataDeviceHandler for WinitState {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        data_device: &WlDataDevice,
        x: f64,
        y: f64,
        wl_surface: &WlSurface,
    ) {
        let Some(data) = data_device.data::<DataDeviceData>() else {
            return;
        };
        let Some(drag) = data.drag_offer() else {
            // Selections are not yet implemented
            return;
        };

        let window_id = crate::make_wid(wl_surface);

        let current_drag = drag.with_mime_types(|types| CurrentDrag {
            mime_types: types
                .iter()
                .map(|str| MimeType::parse(str.clone()))
                .collect::<Vec<_>>()
                .into(),
            transfer_id: DataTransferId::from_raw(drag.serial as i64),
            data: drag.inner().clone(),
            accepted_type: None,
            window_id,
        });

        current_drag.set_actions(&DndActionSet::empty());

        self.dnd_state.current_drag = Some(current_drag);

        let scale_factor = self
            .windows
            .borrow()
            .get(&window_id)
            .map(|window| window.lock().unwrap().scale_factor())
            .unwrap_or(1.);
        let position: PhysicalPosition<f64> = LogicalPosition::new(x, y).to_physical(scale_factor);

        self.events_sink.push_window_event(
            WindowEvent::DragEntered {
                id: DataTransferId::from_raw(drag.serial.into()),
                position: Some(position),
            },
            window_id,
        );
    }

    fn leave(&mut self, _: &Connection, _: &QueueHandle<Self>, data_device: &WlDataDevice) {
        let Some(data) = data_device.data::<DataDeviceData>() else {
            return;
        };

        if let Some(current_drag) = self.dnd_state.current_drag() {
            self.events_sink.push_window_event(
                WindowEvent::DragLeft { id: current_drag.transfer_id() },
                current_drag.window_id(),
            );

            self.dnd_state.current_drag = None;
        }

        if let Some(drag) = data.drag_offer() {
            drag.destroy();
        }
        if let Some(selection) = data.selection_offer() {
            selection.destroy();
        }
    }

    fn motion(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        data_device: &WlDataDevice,
        x: f64,
        y: f64,
    ) {
        let Some(data) = data_device.data::<DataDeviceData>() else {
            return;
        };
        let Some(drag) = data.drag_offer() else {
            // Selections (copy/paste) are not yet implemented
            return;
        };

        let window_id = crate::make_wid(&drag.surface);

        let scale_factor = self
            .windows
            .borrow()
            .get(&window_id)
            .map(|window| window.lock().unwrap().scale_factor())
            .unwrap_or(1.);
        let position: PhysicalPosition<f64> = LogicalPosition::new(x, y).to_physical(scale_factor);

        self.events_sink.push_window_event(
            WindowEvent::DragPosition {
                id: DataTransferId::from_raw(drag.serial.into()),
                position,
            },
            window_id,
        );
    }

    fn selection(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {
        // We don't handle selections right now.
    }

    fn drop_performed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        data_device: &WlDataDevice,
    ) {
        let Some(data) = data_device.data::<DataDeviceData>() else {
            return;
        };
        let Some(drag) = data.drag_offer() else {
            // Selections (copy/paste) are not yet implemented
            return;
        };

        let window_id = crate::make_wid(&drag.surface);

        self.events_sink.push_window_event(
            WindowEvent::DragDropped { id: DataTransferId::from_raw(drag.serial.into()) },
            window_id,
        );

        self.dnd_state.current_drag = None;
    }
}

sctk::delegate_data_device!(WinitState);
