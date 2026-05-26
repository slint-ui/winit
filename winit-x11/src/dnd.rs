use std::cell::Cell;
use std::io;
use std::marker::PhantomData;
use std::os::raw::*;
use std::str::Utf8Error;
use std::sync::{Arc, OnceLock, RwLock};
use std::thread::ThreadId;

use percent_encoding::percent_decode;
use winit_core::data_transfer::{DataTransfer, DataTransferId, TransferType, TypeHint, TypedData};
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
    Io(#[allow(dead_code)] io::Error),
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

// When `thread_id_value` is stabilized, this can become `AtomicU64`.
#[derive(Default, Debug)]
pub struct DeadlockSentinel(Arc<RwLock<Option<ThreadId>>>);

/// Read-side of `DeadlockSentinel` (to prevent accidentally guarding in a re-entrant way).
#[derive(Debug, Clone)]
pub struct DeadlockSentinelReader(Arc<RwLock<Option<ThreadId>>>);

impl DeadlockSentinelReader {
    fn get(&self) -> Option<ThreadId> {
        *self.0.read().unwrap()
    }
}

#[must_use]
#[derive(Debug)]
pub struct DeadlockSentinelGuard(Arc<RwLock<Option<ThreadId>>>);

impl Drop for DeadlockSentinelGuard {
    fn drop(&mut self) {
        *self.0.write().unwrap() = None;
    }
}

impl DeadlockSentinel {
    pub fn guard(&self) -> DeadlockSentinelGuard {
        let mut writer = self.0.write().unwrap();
        assert!(writer.is_none(), "Internal error: re-entrant `DeadlockSentinelGuard`");
        *writer = Some(std::thread::current().id());
        DeadlockSentinelGuard(self.0.clone())
    }

    pub fn reader(&self) -> DeadlockSentinelReader {
        DeadlockSentinelReader(self.0.clone())
    }
}

#[derive(Debug, Default)]
struct SharedDataInnerState {
    data: OnceLock<Result<Box<[c_uchar]>, io::ErrorKind>>,
}

impl SharedDataInnerState {
    fn has_data(&self) -> bool {
        self.data.get().is_some()
    }

    fn try_data(&self) -> io::Result<&[u8]> {
        self.data
            .get()
            .map(|data| data.as_ref().map(|data| &**data).map_err(|err| io::Error::from(*err)))
            .ok_or_else(|| io::Error::from(io::ErrorKind::WouldBlock))?
    }
}

#[derive(Clone, Debug)]
pub(crate) struct SharedDataReader {
    reader: Arc<SharedDataInnerState>,
    deadlock_sentinel: DeadlockSentinelReader,
}

impl SharedDataReader {
    fn try_data(&self) -> io::Result<&[u8]> {
        self.reader.try_data()
    }

    #[rustversion::since(1.86)]
    fn wait_internal(&self) -> io::Result<()> {
        let _ = self.reader.data.wait();

        Ok(())
    }

    #[rustversion::before(1.86)]
    fn wait_internal(&self) -> io::Result<()> {
        if self.reader.has_data() { Ok(()) } else { Err(io::ErrorKind::WouldBlock.into()) }
    }

    fn wait_for_data(&self) -> io::Result<()> {
        if !self.reader.has_data()
            && self.deadlock_sentinel.get() == Some(std::thread::current().id())
        {
            return Err(io::ErrorKind::Deadlock.into());
        }

        self.wait_internal()
    }
}

type NonSyncMarker = PhantomData<Cell<()>>;

#[derive(Debug, Default)]
pub(crate) struct SharedDataWriter {
    writer: Arc<SharedDataInnerState>,
    _non_sync: NonSyncMarker,
}

impl SharedDataWriter {
    fn reader(&self, deadlock_sentinel: DeadlockSentinelReader) -> SharedDataReader {
        SharedDataReader { reader: self.writer.clone(), deadlock_sentinel }
    }

    pub(crate) fn write(&self, value: Box<[c_uchar]>) -> Result<(), Box<[c_uchar]>> {
        // We know that we just passed `Ok`, so we can unwrap here.
        self.writer.data.set(Ok(value)).map_err(|result| result.unwrap())
    }
}

impl Drop for SharedDataWriter {
    fn drop(&mut self) {
        // Prevent `SelectionReader::wait_for_data` from deadlocking.
        let _ = self.writer.data.set(Err(io::ErrorKind::BrokenPipe));
    }
}

#[derive(Clone, Debug)]
pub struct SelectionReader {
    type_: SelectionType,
    data: SharedDataReader,
    pos: u64,
}

impl io::Read for SelectionReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.with_cursor(|cursor| cursor.read(buf))
    }

    fn read_to_end(&mut self, buf: &mut Vec<u8>) -> io::Result<usize> {
        self.with_cursor(|cursor| cursor.read_to_end(buf))
    }

    fn read_to_string(&mut self, buf: &mut String) -> io::Result<usize> {
        self.with_cursor(|cursor| cursor.read_to_string(buf))
    }

    fn read_exact(&mut self, buf: &mut [u8]) -> io::Result<()> {
        self.with_cursor(|cursor| cursor.read_exact(buf))
    }
}

impl io::BufRead for SelectionReader {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        // `io::Cursor::split` takes `&self` instead of `self`, so we need to reimplement it here.
        let data = self.data.try_data()?;

        Ok(&data[self.pos.min(data.len() as u64) as usize..])
    }

    fn consume(&mut self, amount: usize) {
        // `io::Cursor::consume` doesn't require a buffer, so we skip the `try_data` check implied
        // by `with_cursor`.
        self.pos += amount as u64;
    }

    fn read_line(&mut self, buf: &mut String) -> io::Result<usize> {
        self.with_cursor(|cursor| cursor.read_line(buf))
    }
}

impl SelectionReader {
    pub(crate) fn new(type_: SelectionType, data: SharedDataReader) -> Self {
        Self { type_, data, pos: 0 }
    }

    // Instead of reimplementing `io::Cursor`, we have a maximally-conservative
    // implementation that just synchronizes state in order to prevent the chance
    // of misimplementation.
    fn with_cursor<F, O>(&mut self, func: F) -> io::Result<O>
    where
        F: FnOnce(&mut io::Cursor<&[u8]>) -> io::Result<O>,
    {
        let data = self.data.try_data()?;

        let mut cursor = io::Cursor::new(data);
        cursor.set_position(self.pos);
        let result = func(&mut cursor)?;
        let new_pos = cursor.position();
        self.pos = new_pos;

        Ok(result)
    }
}

impl TypedData for SelectionReader {
    fn try_read(&mut self) -> Option<Box<dyn io::BufRead>> {
        Some(Box::new(self.clone()))
    }

    fn type_(&self) -> &dyn TransferType {
        &self.type_
    }

    fn try_as_string(&mut self) -> io::Result<String> {
        fn invalid_data<E>(err: E) -> io::Error
        where
            E: Into<Box<dyn std::error::Error + Send + Sync>>,
        {
            io::Error::new(io::ErrorKind::InvalidData, err)
        }

        fn decode_utf16_bytes(bytes: &[u8]) -> io::Result<String> {
            let utf16 = bytes
                .chunks_exact(2)
                .map(|chunk| {
                    let bytes: &[u8; 2] = chunk.try_into().unwrap();
                    u16::from_ne_bytes(*bytes)
                })
                .collect::<Vec<_>>();
            String::from_utf16(&utf16).map_err(invalid_data)
        }

        match self.type_.hint() {
            Some(TypeHint::Plaintext) | Some(TypeHint::Html) => {
                let data = self.data.try_data()?;

                // Bad way to detect UTF-16 - some applications (confirmed to at least happen with
                // Firefox) don't emit a BOM when passing HTML, so we need to check:
                // A) Does the string contain a null
                // B) Can the string be decoded as UTF-8
                if data.contains(&0) {
                    decode_utf16_bytes(data)
                        // Even if we guess that it's utf-16, we'll still try utf-8 just in case
                        .or_else(|_| {
                            std::str::from_utf8(data)
                                .map(|str| str.to_owned())
                                .map_err(invalid_data)
                        })
                } else {
                    std::str::from_utf8(data)
                        .map(|str| str.to_owned())
                        .map_err(invalid_data)
                        .or_else(|_| decode_utf16_bytes(data))
                }
            },
            Some(TypeHint::UriList) => {
                let data = self.data.try_data()?;

                percent_decode(data).decode_utf8().map(Into::into).map_err(invalid_data)
            },
            _ => Err(io::ErrorKind::InvalidData.into()),
        }
    }

    fn try_as_uris(&mut self) -> io::Result<Vec<String>> {
        if self.type_().hint() != Some(TypeHint::UriList) {
            return Err(io::ErrorKind::InvalidData.into());
        }

        Ok(self
            .try_as_string()?
            .split(['\n', '\r'])
            .filter(|s| !s.is_empty())
            .map(Into::into)
            .collect())
    }

    fn wait_for_data(&self) -> io::Result<()> {
        self.data.wait_for_data()
    }
}

#[derive(Debug)]
pub(crate) struct SelectionFetchState {
    type_: SelectionType,
    // Populated by SelectionNotify event handler
    value: SharedDataWriter,
}

impl SelectionFetchState {
    pub(crate) fn new(type_: SelectionType) -> Self {
        Self { type_, value: Default::default() }
    }

    pub(crate) fn type_(&self) -> &SelectionType {
        &self.type_
    }

    pub(crate) fn as_reader(&self, sentinel: DeadlockSentinelReader) -> SelectionReader {
        SelectionReader::new(self.type_().clone(), self.value.reader(sentinel))
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
    pub deadlock_sentinel: DeadlockSentinel,
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

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
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

impl TransferType for SelectionType {
    fn hint(&self) -> Option<TypeHint> {
        self.hint
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
    pub fn new(xconn: Arc<XConnection>, sentinel: DeadlockSentinel) -> Self {
        Self::with_id(xconn, sentinel, DataTransferId::from_raw(0))
    }

    pub fn find_type_by_hint(&self, hint: TypeHint) -> Option<&SelectionType> {
        self.types.as_ref()?.iter().find(|haystack| haystack.hint() == Some(hint))
    }

    fn with_id(
        xconn: Arc<XConnection>,
        deadlock_sentinel: DeadlockSentinel,
        transfer_id: DataTransferId,
    ) -> Self {
        Dnd {
            xconn,
            transfer_id,
            accepted: None,
            version: None,
            types: None,
            source_window: None,
            target_window: None,
            last_fetched_selection: None,
            deadlock_sentinel,
        }
    }

    pub fn transfer_id(&self) -> DataTransferId {
        self.transfer_id
    }

    pub fn reset(&mut self) {
        let xconn = self.xconn.clone();
        let sentinel = std::mem::take(&mut self.deadlock_sentinel);
        let new_id = DataTransferId::from_raw(self.transfer_id.into_raw().wrapping_add(1));
        *self = Self::with_id(xconn, sentinel, new_id);
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

    pub unsafe fn read_data(&self, window: xproto::Window) -> Result<(), util::GetPropertyError> {
        // Never fetched
        let data =
            self.last_fetched_selection.as_ref().ok_or_else(|| util::GetPropertyError::Unknown)?;

        let atoms = self.xconn.atoms();
        let type_ = self
            .last_fetched_selection
            .as_ref()
            .map(|state| state.type_.atom())
            .ok_or(util::GetPropertyError::Unknown)?;
        let bytes = self.xconn.get_property(window, atoms[XdndSelection], type_)?;

        data.value.write(bytes.into()).map_err(|_| util::GetPropertyError::Unknown)?;

        Ok(())
    }
}
