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
use crate::atoms::{
    Atoms, TextUriList, XdndActionPrivate, XdndFinished, XdndSelection, XdndStatus, XdndTypeList,
};
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

    fn try_as_plaintext(&mut self) -> Option<String> {
        // We don't check that the type of this data is plaintext, as other types (e.g. HTML, URI
        // list) are valid to read as plaintext
        str::from_utf8(&self.data).ok().map(Into::into)
    }

    fn try_as_uris(&mut self) -> Option<Vec<String>> {
        if self.type_().hint() != Some(TypeHint::UriList) {
            return None;
        }

        Some(
            self.try_as_plaintext()?
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
    // Populated by SelectionNotify event handler
    pub value: Option<io::Result<Box<SelectionReader>>>,
}

impl SelectionFetchState {
    pub fn new() -> Self {
        Self { serial: AsyncRequestSerial::get(), value: None }
    }
}

#[derive(Debug)]
pub struct Dnd {
    xconn: Arc<XConnection>,
    transfer_id: DataTransferId,
    // Populated by XdndEnter event handler
    pub version: Option<c_long>,
    pub type_infos: Option<Vec<SelectionType>>,
    // Populated by XdndPosition event handler
    pub source_window: Option<xproto::Window>,
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
    pub(crate) fn from_dyn(atoms: &Atoms, type_: &dyn TransferType) -> Option<Self> {
        type_.cast_ref().cloned().or_else(|| {
            let hint = type_.hint()?;

            match hint {
                TypeHint::UriList => Some(Self { hint: Some(hint), atom: atoms[TextUriList] }),
                _ => None,
            }
        })
    }

    pub(crate) fn new(atoms: &Atoms, atom: xproto::Atom) -> Self {
        let hint = if atom == atoms[TextUriList] { Some(TypeHint::UriList) } else { None };

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
            .type_infos
            .as_ref()
            .into_iter()
            .flat_map(|types| types.iter().map(|val| Box::new(val.clone()) as _))
            .collect()
    }

    fn has_type(&self, type_: &dyn TransferType) -> bool {
        let dnd = self.dnd.read().unwrap();

        let Some(types) = dnd.type_infos.as_ref() else {
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

    fn with_id(xconn: Arc<XConnection>, transfer_id: DataTransferId) -> Self {
        Dnd {
            xconn,
            transfer_id,
            version: None,
            type_infos: None,
            source_window: None,
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
            .convert_selection(window, atoms[XdndSelection], new_type, atoms[XdndSelection], time)
            .expect_then_ignore_error("Failed to send XdndSelection event")
    }

    pub unsafe fn read_data(
        &self,
        window: xproto::Window,
    ) -> Result<Vec<c_uchar>, util::GetPropertyError> {
        let atoms = self.xconn.atoms();
        self.xconn.get_property(window, atoms[XdndSelection], atoms[TextUriList])
    }
}
