//! Types related to data transfer, used for clipboard and drag-and-drop.
//!
//! This module contains types and traits implementing cross-application data transfer.
//! While the precise implementation depends on platform, there are a set of types which
//! can be safely transferred between applications on all platforms (see [`TypeHint`]).
//!
//! On all platforms, the process looks something like this:
//!
//! - A data transfer advertises a set of types which the data can be interpreted as
//!   - For example, if you copy or drag text from a web page, the browser may advertise the text
//!     formatted using HTML, the text formatted as RTF, and the text with all formatting removed
//!     simultaneously.
//! - An application receiving a data transfer chooses one or more types that it understands and
//!   requests the data in those formats (in practice, it will usually only request a single
//!   format).
//! - The source application converts the data stored in its memory to the requested format and
//!   asynchronously sends it to the target application
//!
//! On some platforms, the data is sometimes available synchronously, but all platforms have at
//! least some method of sending the data asynchronously and some types of data that may _only_ be
//! sent using the asynchronous interface. Because of this, the API in winit must be asynchronous.
//!
//! The flow for a user application that implements drag-and-drop would look something like this:
//!
//! - The application receives a [`DragEnter`](crate::event::WindowEvent::DragEnter) event. This
//!   event supplies a [`DataTransferId`] which can be used to request information or operations on
//!   the dragged data by using methods on [`Window`](crate::window::Window).
//! - As the drag operation continues, the window will receive
//!   [`DragMoved`](crate::event::WindowEvent::DragEnter) events.
//! - While `DragMoved` events are being received, the receiving application may mark the data as
//!   being accepted or rejected. This will update the OS/compositor to display the correct UI to
//!   the user. Accepting does not "finalize" the drag operation, nor does rejecting cancel it.
//! - At any point during this operation, the receiving application may request either the available
//!   types or even the data being transferred. This may be useful in cases where the application
//!   wants to preload the data. For example, an image editor may want to display the image on the
//!   canvas during the drag operation.
//! - When the user tries to drop the data onto the window, that window will receive a
//!   [`DragDropped`](crate::event::WindowEvent::DragDropped) event. In general, the receiving
//!   application should assume that calling [`reject_drag`](crate::window::Window::reject_drag)
//!   after `DragDropped` is received ends the lifecycle of the data transfer.
//!
//! If platform-dependent behavior is required, a platform may define internal types
//! implementing the traits in this module, which can then be accessed in an application
//! using the methods defined on [`dyn AsAny`]. See each platform's documentation for details.

use std::borrow::Cow;
use std::ffi::{OsStr, OsString};
use std::fmt::{self, Debug};
use std::io;
use std::ops::Deref;
use std::sync::Arc;

use crate::as_any::AsAny;

/// Unique identifier for a data transfer.
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DataTransferId(i64);

impl DataTransferId {
    /// Convert the [`DataTransferId`] into the underlying integer.
    ///
    /// This is useful if you need to pass the ID across an FFI boundary, or store it in an atomic.
    pub const fn into_raw(self) -> i64 {
        self.0
    }

    /// Construct a [`DataTransferId`] from the underlying integer.
    ///
    /// This should only be called with integers returned from [`DataTransferId::into_raw`].
    pub const fn from_raw(id: i64) -> Self {
        Self(id)
    }
}

/// The set of types supported cross-platform.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum TypeHint {
    /// Plain UTF-8 text (see [`TypedData::try_as_plaintext`]).
    ///
    /// **Note for platform implementations**: this hint is _only_ for UTF-8 text. If the platform
    /// returns plaintext in some format other than UTF-8 by default, a [`TypedData`]
    /// implementation marked with this type hint should convert to UTF-8.
    Plaintext,
    /// A list of URIs in the format defined by the `text/uri-list` MIME type, encoded as UTF-8 (see
    /// [`TypedData::try_as_uris`]).
    ///
    /// **Note for platform implementations**: this hint is _only_ for URIs encoded precisely in the
    /// format specified above. If the platform uses a different format, a [`TypedData`]
    /// implementation marked with this type hint should convert to that format.
    UriList,
    /// A HTML-formatted string
    Html,
    /// An RTF-formatted string
    Rtf,
    /// Audio
    Audio {
        /// An optional hint for the encoding of the supplied bytes, specified using the standard
        /// file extension for that audio format, lowercase and without the leading `.`.
        extension_hint: Option<&'static str>,
    },
    /// Image data
    Image {
        /// An optional hint for the encoding of the supplied bytes, specified using the standard
        /// file extension for that audio format, lowercase and without the leading `.`.
        extension_hint: Option<&'static str>,
    },
}

/// The type of a data transfer.
///
/// [`hint`](TransferType::hint) can be called to get the type in
/// a cross-platform format (see [`TypeHint`])
pub trait TransferType: AsAny + Send + Sync + fmt::Debug {
    /// Get the cross-platform representation of this type.
    ///
    /// If this returns `None`, then this is a platform-dependent type that has no cross-platform
    /// equivalent.
    fn hint(&self) -> Option<TypeHint>;
}

impl TransferType for TypeHint {
    fn hint(&self) -> Option<TypeHint> {
        Some(*self)
    }
}

impl_dyn_casting!(TransferType);

/// Data that has been fetched from a data transfer
pub trait TypedData: AsAny + Send + Sync + fmt::Debug {
    fn type_(&self) -> &dyn TransferType;
    fn try_read(&mut self) -> Option<Box<dyn io::BufRead + '_>>;

    fn try_as_uris(&mut self) -> Option<Vec<Cow<'_, OsStr>>> {
        if self.type_().hint() != Some(TypeHint::UriList) {
            return None;
        }

        let mut reader = self.try_read()?;
        let mut out = String::new();
        reader.read_to_string(&mut out).ok()?;

        let uris = out.split(|c| c == '\n' || c == '\r');

        Some(uris.map(|str| OsString::from(str).into()).collect())
    }

    fn try_as_plaintext(&mut self) -> Option<String> {
        if self.type_().hint() != Some(TypeHint::UriList) {
            return None;
        }

        let mut reader = self.try_read()?;
        let mut out = String::new();
        reader.read_to_string(&mut out).ok()?;

        Some(out)
    }
}

#[derive(Debug, Clone)]
pub struct DynTypedData(pub Arc<dyn TypedData>);

impl PartialEq for DynTypedData {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::addr_eq(&**self, &**other)
    }
}

impl Deref for DynTypedData {
    type Target = dyn TypedData;

    fn deref(&self) -> &Self::Target {
        &*self.0
    }
}

impl_dyn_casting!(TypedData);

/// Metadata about a data transfer. This does not allow actually receiving data, as that is an
/// asynchronous operation. To fetch the data from the source application, see
/// [`Window::fetch_data_transfer`](crate::window::Window::fetch_data_transfer)
/// and [`WindowEvent::DataTransferResult`](crate::event::WindowEvent::DataTransferResult).
pub trait DataTransfer: AsAny + Send + Sync + fmt::Debug {
    /// Display the list of all available types.
    ///
    /// This is useful if more-complex type matching is required, but for most cases
    /// [`has_type`](DataTransfer::has_type) should be used.
    // TODO: We should be able to do `&dyn TransferType`, but some implementation details in
    // the platforms make that unnecessarily difficult right now. Specifically, use of `RwLock`.
    fn available_types(&self) -> Box<dyn Iterator<Item = Box<dyn TransferType>> + '_>;

    /// Check if the supplied type is provided by this [`DataTransfer`].
    ///
    /// Supplying a [`TypeHint`] as the type is supported on all platforms, but if some
    /// platform-specific type is required then that platform's implementation of `TransferType` can
    /// be used.
    fn has_type(&self, type_: &dyn TransferType) -> bool {
        type_.hint().is_some_and(|hint| {
            self.available_types().any(|haystack| haystack.hint() == Some(hint))
        })
    }
}

impl_dyn_casting!(DataTransfer);
