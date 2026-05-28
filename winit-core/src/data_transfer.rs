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

use std::ffi::OsString;
use std::fmt::{self, Debug};
use std::io;
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
}

impl TransferType for TypeHint {
    fn hint(&self) -> Option<TypeHint> {
        Some(*self)
    }
}

impl TransferType for Option<TypeHint> {
    fn hint(&self) -> Option<TypeHint> {
        *self
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
    fn try_as_uris(&mut self) -> io::Result<Vec<String>>;

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
        type_.hint().is_some_and(|hint| {
            let mut found = false;
            self.for_each_available_type(&mut |haystack| {
                if haystack.hint() == Some(hint) {
                    found = true;
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            });

            found
        })
    }
}

impl_dyn_casting!(DataTransfer);
