use std::io;
use std::os::raw::*;
use std::path::{Path, PathBuf};
use std::str::Utf8Error;
use std::sync::{Arc, RwLock};

use percent_encoding::percent_decode;
use winit_core::data_transfer::{DataTransfer, DataTransferId, TransferType, TypeHint, TypedData};
use winit_core::event_loop::AsyncRequestSerial;
use x11rb::protocol::xproto::{self, ConnectionExt};

use crate::atoms::AtomName::None as DndNone;
use crate::atoms::*;
use crate::event_loop::{CookieResultExt, X11Error};
use crate::util;
use crate::xdisplay::XConnection;

#[derive(Debug, Clone, Copy)]
pub enum DndState {
    Accepted,
    Rejected,
}

#[derive(Debug)]
pub enum UriListParseError {
    EmptyData,
    InvalidUtf8(#[allow(dead_code)] Utf8Error),
    HostnameSpecified(#[allow(dead_code)] String),
    UnexpectedProtocol(#[allow(dead_code)] String),
    UnresolvablePath(#[allow(dead_code)] io::Error),
}

impl From<Utf8Error> for UriListParseError {
    fn from(e: Utf8Error) -> Self {
        UriListParseError::InvalidUtf8(e)
    }
}

impl From<io::Error> for UriListParseError {
    fn from(e: io::Error) -> Self {
        UriListParseError::UnresolvablePath(e)
    }
}

#[derive(Clone, Debug)]
pub struct SelectionReader {
    type_: SelectionType,
    data: Arc<[c_uchar]>,
}

impl SelectionReader {
    pub(crate) fn new(type_: SelectionType, data: Arc<[c_uchar]>) -> Self {
        Self { type_, data }
    }

    pub fn parse_path_list(&self) -> Result<Vec<PathBuf>, UriListParseError> {
        if !self.data.is_empty() {
            let mut path_list = Vec::new();
            let decoded = percent_decode(&self.data).decode_utf8()?.into_owned();
            for uri in decoded.split("\r\n").filter(|u| !u.is_empty()) {
                // The format is specified as protocol://host/path
                // However, it's typically simply protocol:///path
                let path_str = if uri.starts_with("file://") {
                    let path_str = uri.replace("file://", "");
                    if !path_str.starts_with('/') {
                        // A hostname is specified
                        // Supporting this case is beyond the scope of my mental health
                        return Err(UriListParseError::HostnameSpecified(path_str));
                    }
                    path_str
                } else {
                    // Only the file protocol is supported
                    return Err(UriListParseError::UnexpectedProtocol(uri.to_owned()));
                };

                let path = Path::new(&path_str).canonicalize()?;
                path_list.push(path);
            }
            Ok(path_list)
        } else {
            Err(UriListParseError::EmptyData)
        }
    }
}

impl TypedData for SelectionReader {
    fn try_read(&mut self) -> Option<Box<dyn io::BufRead + '_>> {
        Some(Box::new(io::Cursor::new(&self.data)))
    }

    fn type_(&self) -> &dyn TransferType {
        &self.type_
    }

    fn try_as_string(&mut self) -> Option<String> {
        fn decode_utf16_bytes(bytes: &[u8]) -> Option<String> {
            let utf16 = bytes
                .chunks_exact(2)
                .into_iter()
                .map(|chunk| {
                    let bytes: &[u8; 2] = chunk.try_into().unwrap();
                    u16::from_ne_bytes(*bytes)
                })
                .collect::<Vec<_>>();
            String::from_utf16(&utf16).ok()
        }

        match self.type_.hint() {
            Some(TypeHint::Plaintext) | Some(TypeHint::Html) => {
                // Bad way to detect UTF-16 - some applications (confirmed to at least happen with
                // Firefox) don't emit a BOM when passing HTML, so we need to check:
                // A) Does the string contain a null
                // B) Can the string be decoded as UTF-8
                if self.data.contains(&0) {
                    decode_utf16_bytes(&self.data)
                        // Even if we guess that it's utf-16, we'll still try utf-8 just in case
                        .or_else(|| str::from_utf8(&self.data).ok().map(|str| str.to_owned()))
                } else {
                    str::from_utf8(&self.data)
                        .map(|str| str.to_owned())
                        .ok()
                        .or_else(|| decode_utf16_bytes(&self.data))
                }
            },
            Some(TypeHint::UriList) => {
                percent_decode(&self.data).decode_utf8().ok().map(Into::into)
            },
            _ => None,
        }
    }

    fn try_as_uris(&mut self) -> Option<Vec<String>> {
        if self.type_().hint() != Some(TypeHint::UriList) {
            return None;
        }

        Some(
            self.try_as_string()?
                .split(|c| c == '\n' || c == '\r')
                .filter(|s| !s.is_empty())
                .map(Into::into)
                .collect(),
        )
    }
}

#[derive(Debug)]
pub struct SelectionFetchState {
    pub serial: AsyncRequestSerial,
    pub type_: xproto::Atom,
    // Populated by SelectionNotify event handler
    pub value: Option<io::Result<Box<SelectionReader>>>,
}

impl SelectionFetchState {
    pub fn new(type_: xproto::Atom) -> Self {
        Self { serial: AsyncRequestSerial::get(), type_, value: None }
    }
}

#[derive(Debug)]
pub struct Dnd {
    xconn: Arc<XConnection>,
    transfer_id: DataTransferId,
    /// Whether the drag operation is accepted (or `None` if the user never indicated that it's
    /// accepted or rejected)
    // Populated by `Window::accept_drag`/`Window::reject_drag`.
    pub accepted: Option<bool>,
    // Populated by XdndEnter event handler
    pub version: Option<c_long>,
    pub types: Option<Vec<SelectionType>>,
    // Populated by Xdnd* event handlers
    pub source_window: Option<xproto::Window>,
    // Populated by Xdnd* event handlers
    pub target_window: Option<xproto::Window>,
    // Populated by `fetch_data_transfer`
    pub last_fetched_selection: Option<SelectionFetchState>,
}

#[derive(Debug)]
pub struct Selection {
    dnd: Arc<RwLock<Dnd>>,
}

impl Selection {
    pub(crate) fn new(dnd: Arc<RwLock<Dnd>>) -> Selection {
        Selection { dnd }
    }
}

#[derive(Clone, Debug)]
pub struct SelectionType {
    hint: Option<TypeHint>,
    atom: xproto::Atom,
}

impl SelectionType {
    pub(crate) fn new(atoms: &Atoms, atom: xproto::Atom) -> Self {
        let atom_to_hint = [
            // Files
            (atoms[TextUriList], TypeHint::UriList),
            (atoms[TARGETS], TypeHint::UriList),
            (atoms[SAVE_TARGETS], TypeHint::UriList),
            // Plaintext
            (atoms[STRING], TypeHint::Plaintext),
            (atoms[UTF8_STRING], TypeHint::Plaintext),
            (atoms[TextPlain], TypeHint::Plaintext),
            (atoms[TextPlainCharsetUtf8], TypeHint::Plaintext),
            // HTML
            (atoms[TextHtml], TypeHint::Html),
            (atoms[TextHtmlCharsetUtf8], TypeHint::Html),
            // RTF
            (atoms[ApplicationRtf], TypeHint::Rtf),
            // Audio
            (atoms[AudioAac], TypeHint::Audio { extension_hint: Some("aac") }),
            (atoms[AudioAiff], TypeHint::Audio { extension_hint: Some("aif") }),
            (atoms[AudioFlac], TypeHint::Audio { extension_hint: Some("flac") }),
            (atoms[AudioVndWav], TypeHint::Audio { extension_hint: Some("wav") }),
            (atoms[AudioVndWave], TypeHint::Audio { extension_hint: Some("wav") }),
            (atoms[AudioWav], TypeHint::Audio { extension_hint: Some("wav") }),
            (atoms[AudioWave], TypeHint::Audio { extension_hint: Some("wav") }),
            (atoms[AudioXWav], TypeHint::Audio { extension_hint: Some("wav") }),
            (atoms[AudioOgg], TypeHint::Audio { extension_hint: Some("ogg") }),
            (atoms[AudioMpeg], TypeHint::Audio { extension_hint: Some("mp3") }),
            // Image
            (atoms[ImageBmp], TypeHint::Image { extension_hint: Some("bmp") }),
            (atoms[ImageGif], TypeHint::Image { extension_hint: Some("gif") }),
            (atoms[ImageJpeg], TypeHint::Image { extension_hint: Some("jpg") }),
            (atoms[ImagePjpeg], TypeHint::Image { extension_hint: Some("jpg") }),
            (atoms[ImagePng], TypeHint::Image { extension_hint: Some("png") }),
            (atoms[ImageRaw], TypeHint::Image { extension_hint: Some("raw") }),
            (atoms[ImageSvg], TypeHint::Image { extension_hint: Some("svg") }),
            (atoms[ImageTiff], TypeHint::Image { extension_hint: Some("tiff") }),
            (atoms[ImageWebp], TypeHint::Image { extension_hint: Some("webp") }),
            (atoms[ImageXIcon], TypeHint::Image { extension_hint: Some("ico") }),
        ];
        let hint =
            atom_to_hint.iter().find_map(|(haystack, hint)| (*haystack == atom).then_some(*hint));

        Self { hint, atom }
    }

    pub fn atom(&self) -> xproto::Atom {
        self.atom
    }
}

impl PartialEq for SelectionType {
    fn eq(&self, other: &Self) -> bool {
        self.atom == other.atom
    }
}

impl TransferType for SelectionType {
    fn hint(&self) -> Option<TypeHint> {
        self.hint.clone()
    }
}

impl DataTransfer for Selection {
    fn available_types(&self) -> Vec<Box<dyn TransferType>> {
        self.dnd
            .read()
            .unwrap()
            .types
            .as_ref()
            .into_iter()
            .flat_map(|types| types.iter().map(|val| Box::new(val.clone()) as _))
            .collect()
    }

    fn has_type(&self, type_: &dyn TransferType) -> bool {
        let dnd = self.dnd.read().unwrap();

        let Some(types) = dnd.types.as_ref() else {
            return false;
        };

        if let Some(x11_type) = type_.cast_ref() {
            types.iter().any(|haystack| haystack == x11_type)
        } else {
            let Some(hint) = type_.hint() else {
                return false;
            };

            types.iter().any(|haystack| haystack.hint() == Some(hint))
        }
    }
}

impl Dnd {
    pub fn new(xconn: Arc<XConnection>) -> Self {
        Self::with_id(xconn, DataTransferId::from_raw(0))
    }

    pub fn find_type_by_hint(&self, hint: TypeHint) -> Option<&SelectionType> {
        self.types.as_ref()?.iter().find(|haystack| haystack.hint() == Some(hint))
    }

    fn with_id(xconn: Arc<XConnection>, transfer_id: DataTransferId) -> Self {
        Dnd {
            xconn,
            transfer_id,
            accepted: None,
            version: None,
            types: None,
            source_window: None,
            target_window: None,
            last_fetched_selection: None,
        }
    }

    pub fn transfer_id(&self) -> DataTransferId {
        self.transfer_id
    }

    pub fn reset(&mut self) {
        let xconn = self.xconn.clone();
        let new_id = DataTransferId::from_raw(self.transfer_id.into_raw().wrapping_add(1));
        *self = Self::with_id(xconn, new_id);
    }

    pub unsafe fn send_status(
        &self,
        this_window: xproto::Window,
        target_window: xproto::Window,
        state: DndState,
    ) -> Result<(), X11Error> {
        let atoms = self.xconn.atoms();
        let (accepted, action) = match state {
            DndState::Accepted => (1, atoms[XdndActionPrivate]),
            DndState::Rejected => (0, atoms[DndNone]),
        };
        self.xconn
            .send_client_msg(target_window, target_window, atoms[XdndStatus] as _, None, [
                this_window,
                accepted,
                0,
                0,
                action as _,
            ])?
            .ignore_error();

        Ok(())
    }

    pub unsafe fn send_finished(
        &self,
        this_window: xproto::Window,
        target_window: xproto::Window,
        state: DndState,
    ) -> Result<(), X11Error> {
        let atoms = self.xconn.atoms();
        let (accepted, action) = match state {
            DndState::Accepted => (1, atoms[XdndActionPrivate]),
            DndState::Rejected => (0, atoms[DndNone]),
        };
        self.xconn
            .send_client_msg(target_window, target_window, atoms[XdndFinished] as _, None, [
                this_window,
                accepted,
                action as _,
                0,
                0,
            ])?
            .ignore_error();

        Ok(())
    }

    pub unsafe fn get_type_list(
        &self,
        source_window: xproto::Window,
    ) -> Result<Vec<xproto::Atom>, util::GetPropertyError> {
        let atoms = self.xconn.atoms();
        self.xconn.get_property(
            source_window,
            atoms[XdndTypeList],
            xproto::Atom::from(xproto::AtomEnum::ATOM),
        )
    }

    pub unsafe fn convert_selection(
        &self,
        window: xproto::Window,
        time: xproto::Timestamp,
        new_type: xproto::Atom,
    ) {
        let atoms = self.xconn.atoms();
        self.xconn
            .xcb_connection()
            // TODO: We store the converted selection back to `XdndSelection`. We should store to
            // some new place so that `XdndSelection` remains untouched.
            .convert_selection(window, atoms[XdndSelection], new_type, atoms[XdndSelection], time)
            .expect_then_ignore_error("Failed to send XdndSelection event")
    }

    pub unsafe fn read_data(
        &self,
        window: xproto::Window,
    ) -> Result<(xproto::Atom, Vec<c_uchar>), util::GetPropertyError> {
        let atoms = self.xconn.atoms();
        let type_ = self
            .last_fetched_selection
            .as_ref()
            .map(|state| state.type_)
            .ok_or(util::GetPropertyError::Unknown)?;
        Ok((type_, self.xconn.get_property(window, atoms[XdndSelection], type_)?))
    }
}
