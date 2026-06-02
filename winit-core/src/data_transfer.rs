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

#![warn(missing_docs)]

use std::ffi::OsString;
use std::fmt;
use std::io;
use std::marker::PhantomData;
use std::ops::ControlFlow;

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
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum TypeHint {
    /// Plain UTF-8 text (see [`TypedData::try_as_string`]).
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

impl TypeHint {
    /// Check whether the two type hints "match".
    ///
    /// This is subtly different to direct equality. If one of the types is an image or audio with a
    /// `None` extension hint, then the other type just needs to match variant (i.e. image/audio),
    /// the extension does not also have to be `None`.
    pub fn matches(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Plaintext, Self::Plaintext)
            | (Self::UriList, Self::UriList)
            | (Self::Html, Self::Html)
            | (Self::Rtf, Self::Rtf) => true,

            (
                Self::Audio { extension_hint: this_ext },
                Self::Audio { extension_hint: other_ext },
            )
            | (
                Self::Image { extension_hint: this_ext },
                Self::Image { extension_hint: other_ext },
            ) => match (this_ext, other_ext) {
                (Some(this_ext), Some(other_ext)) => this_ext == other_ext,
                (None, _) | (_, None) => true,
            },

            _ => false,
        }
    }
}

/// The type of a data transfer.
///
/// [`hint`](TransferType::hint) can be called to get the type in
/// a cross-platform format (see [`TypeHint`])
pub trait TransferType: AsAny + fmt::Debug {
    /// Get the cross-platform representation of this type.
    ///
    /// If this returns `None`, then this is a platform-dependent type that has no cross-platform
    /// equivalent.
    fn hint(&self) -> Option<TypeHint>;

    /// Check whether two dynamically-typed transfer types are equivalent.
    // Can't use a `PartialEq` bound because it causes a dependency cycle.
    fn matches(&self, other: &dyn TransferType) -> bool;
}

impl TransferType for TypeHint {
    fn hint(&self) -> Option<TypeHint> {
        Some(*self)
    }

    fn matches(&self, other: &dyn TransferType) -> bool {
        other.hint() == Some(*self)
    }
}

impl_dyn_casting!(TransferType);

/// Data that has been fetched from a data transfer
///
/// ### Blocking
///
/// Note that this type provides a blocking interface. In cases where reading this type directly on
/// the event loop would cause a deadlock, the backend will make a best-effort attempt to return an
/// error with [`io::ErrorKind::Deadlock`]. For now, the only way to access the data is via blocking
/// on the event loop, so simply retrying the next time an event is received that references the
/// data transfer should be enough to ensure that the data is accessible.
pub trait TypedData: AsAny + fmt::Debug {
    /// The type of this `TypedData`.
    fn type_(&self) -> &dyn TransferType;

    /// If this value is readable as bytes, return a reader than can be used to read those bytes.
    fn try_read(&mut self) -> Option<Box<dyn io::BufRead>>;

    /// Read this value as a list of URIs.
    ///
    /// If this value is not readable as URIs, return `None`.
    ///
    /// The format of the returned URIs is simply a vector of strings. No validation is done
    /// to ensure that the URIs are valid or in the format
    fn try_as_uris(&mut self) -> io::Result<Vec<OsString>>;

    /// Read this value as a plain text string.
    ///
    /// If this value is not readable as a string, return `None`.
    fn try_as_string(&mut self) -> io::Result<String>;
}

impl_dyn_casting!(TypedData);

/// Metadata about a data transfer. This does not allow actually receiving data, as that is an
/// asynchronous operation. To fetch the data from the source application, see
/// [`Window::fetch_data_transfer`](crate::window::Window::fetch_data_transfer)
/// and [`WindowEvent::DataTransferResult`](crate::event::WindowEvent::DataTransferResult).
pub trait DataTransfer: AsAny + fmt::Debug {
    /// Iterate over each type advertized by this `DataTransfer`. This is just a minor optimization,
    /// in most cases you should probably use [`has_type`](DataTransfer::has_type) or
    /// [`available_types`](DataTransfer::available_types).
    fn for_each_available_type<'this>(
        &'this self,
        func: &'_ mut dyn FnMut(&'this dyn TransferType) -> ControlFlow<()>,
    );

    /// Display the list of all available types.
    ///
    /// This is useful if more-complex type matching is required, but for most cases
    /// [`has_type`](DataTransfer::has_type) should be used.
    fn available_types(&self) -> Vec<&'_ dyn TransferType> {
        let mut out = Vec::new();

        self.for_each_available_type(&mut |ty| {
            out.push(ty);
            ControlFlow::Continue(())
        });

        out
    }

    /// Check if the supplied type is provided by this [`DataTransfer`].
    ///
    /// Supplying a [`TypeHint`] as the type is supported on all platforms, but if some
    /// platform-specific type is required then that platform's implementation of `TransferType` can
    /// be used.
    fn has_type(&self, type_: &dyn TransferType) -> bool {
        let mut found = false;
        self.for_each_available_type(&mut |haystack| {
            if haystack.matches(type_) {
                found = true;
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        });

        found
    }
}

impl_dyn_casting!(DataTransfer);

/// Kinds of data that can be sent via a `DataTransfer`.
///
/// Some kinds of data cannot be represented by just a binary blob in a cross-platform way.
/// File URIs on Windows and macOS are represented as arrays of strings, and strings have
/// different encoding on different platforms. To allow this to be represented, we allow
/// supplying strings and URIs separately from binary blobs.
pub enum SendData {
    /// File URIs
    Uris(Vec<OsString>),
    /// String
    String(String),
    /// Binary blob
    Bytes(Vec<u8>),
}

impl From<String> for SendData {
    fn from(value: String) -> Self {
        Self::String(value)
    }
}

impl From<Vec<u8>> for SendData {
    fn from(value: Vec<u8>) -> Self {
        Self::Bytes(value)
    }
}

// We monomorphize these `From` implementations instead of making them generic, in order to
// prevent accidentally casting to the wrong type.
impl From<Vec<OsString>> for SendData {
    fn from(value: Vec<OsString>) -> Self {
        Self::Uris(value)
    }
}

/// Trait for sending data via a data transfer.
///
/// See [`StartDrag`](crate::event_loop::StartDrag) for where this is used. To build an
/// implementation of this trait dynamically in a cross-platform way, use [`DataTransferSendBuilder`].
pub trait DataTransferSend: DataTransfer {
    /// Get the data for the specified type, or `None` if this value does not supply the given data type.
    fn data_for_type(&self, type_: &dyn TransferType) -> Option<SendData>;

    /// If `true`, this data transfer is only valid for the application sending the data.
    ///
    /// This is useful on Wayland and macOS, which allow expressing internal drag-and-drop in the API.
    /// On platforms which make no distinction between internal and external drag-and-drop, this is
    /// ignored.
    fn is_internal_only(&self) -> bool;
}

impl_dyn_casting!(DataTransferSend);

/// Marker for a [`DataTransferSendBuilder`] which is internal-only.
pub enum InternalTransferMarker {}
/// Marker for a [`DataTransferSendBuilder`] which is external.
pub enum ExternalTransferMarker {}

type SendDataCallback<T> = Box<dyn Fn(&T) -> SendData>;

/// Dynamic builder for an implementation of [`DataTransferSend`].
///
/// On all platforms, inter-application data transfer (i.e. clipboard and drag-and-drop) works like so:
///
/// - The source advertises a set of types that it can transfer.
/// - The destination picks one or more of those types to receive.
/// - The source sends the data for that type.
///
/// This type abstracts that in a way that allows data to be sent cross-platform. `T` is an optional
/// state value, which allows the user to have a single source of truth for their data, converting
/// it lazily to the requested type.
pub struct DataTransferSendBuilder<T, M = ExternalTransferMarker> {
    state: T,
    types: Vec<(Box<dyn TransferType>, SendDataCallback<T>)>,
    ///
    _is_internal: PhantomData<M>,
}

impl<T, M> fmt::Debug for DataTransferSendBuilder<T, M>
where
    T: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NewDataTransferBuilder").field("state", &self.state).finish_non_exhaustive()
    }
}

impl<T, M> DataTransfer for DataTransferSendBuilder<T, M>
where
    M: 'static,
    T: fmt::Debug + 'static,
{
    fn for_each_available_type<'this>(
        &'this self,
        func: &'_ mut dyn FnMut(&'this dyn TransferType) -> ControlFlow<()>,
    ) {
        let _ = self.types.iter().try_for_each(|(ty, _)| func(&**ty));
    }
}

impl<T> DataTransferSend for DataTransferSendBuilder<T, ExternalTransferMarker>
where
    T: fmt::Debug + 'static,
{
    fn data_for_type(&self, type_: &dyn TransferType) -> Option<SendData> {
        self.data_for_type(type_)
    }

    fn is_internal_only(&self) -> bool {
        false
    }
}

impl<T> DataTransferSend for DataTransferSendBuilder<T, InternalTransferMarker>
where
    T: fmt::Debug + 'static,
{
    fn data_for_type(&self, type_: &dyn TransferType) -> Option<SendData> {
        self.data_for_type(type_)
    }

    fn is_internal_only(&self) -> bool {
        true
    }
}

impl<T> DataTransferSendBuilder<T, ExternalTransferMarker> {
    /// Create a new [`DataTransferSendBuilder`], with a state value which acts as
    /// the single source of truth for the underlying data.
    pub fn new(state: T) -> Self {
        Self { state, types: vec![], _is_internal: PhantomData }
    }
}

impl<T> DataTransferSendBuilder<T, InternalTransferMarker> {
    /// Create a new [`DataTransferSendBuilder`], with a state value which acts as
    /// the single source of truth for the underlying data.
    pub fn new_internal(state: T) -> Self {
        Self { state, types: vec![], _is_internal: PhantomData }
    }
}

impl<T, M> DataTransferSendBuilder<T, M> {
    fn data_for_type(&self, type_: &dyn TransferType) -> Option<SendData> {
        let (_, func) = self.types.iter().find(|(ty, _)| ty.matches(type_))?;

        Some(func(&self.state))
    }

    /// Add a callback which converts the builder's state to the given type. In
    /// most cases, `type_` will be [`TypeHint`].
    pub fn add_type<Ty, F>(&mut self, type_: Ty, func: F) -> &mut Self
    where
        Ty: TransferType,
        F: Fn(&T) -> SendData + 'static,
    {
        self.types.push((Box::new(type_), Box::new(func)));
        self
    }

    /// Return a new builder, adding a callback which converts the builder's state
    /// to the given type. In most cases, `type_` will be [`TypeHint`].
    pub fn with_type<Ty, F>(mut self, type_: Ty, func: F) -> Self
    where
        Ty: TransferType,
        F: Fn(&T) -> SendData + 'static,
    {
        self.add_type(type_, func);
        self
    }
}

impl<T, M> DataTransferSendBuilder<T, M>
where
    T: fmt::Debug + 'static,
    Self: DataTransferSend,
{
    /// Consume the builder, returning an implementation of [`DataTransferSend`].
    ///
    /// Note that this is only provided for explicitness and ergonomics. [`DataTransferSendBuilder`]
    /// implements [`DataTransferSend`] and this method is equivalent to [`Box::new`].
    pub fn build(self) -> Box<dyn DataTransferSend> {
        Box::new(self)
    }
}
