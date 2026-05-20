use std::borrow::Cow;
use std::ffi::{OsStr, OsString};
use std::fmt::{self, Debug};
use std::io;
use std::ops::Deref;
use std::sync::Arc;

use crate::as_any::AsAny;

/// Identifier of a data transfer.
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

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum TypeHint {
    Plaintext,
    UriList,
    Html,
    Rtf,
    Audio { extension_hint: Option<&'static str> },
    Image { extension_hint: Option<&'static str> },
}

pub trait TransferType: AsAny + Send + Sync + fmt::Debug {
    fn hint(&self) -> Option<TypeHint>;
}

impl TransferType for TypeHint {
    fn hint(&self) -> Option<TypeHint> {
        Some(*self)
    }
}

impl_dyn_casting!(TransferType);

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

pub trait DataTransfer: AsAny + Send + Sync + fmt::Debug {
    /// Display the list of all available MIME types.
    ///
    /// This is useful if more-complex MIME type matching is required, but for most cases
    /// [`has_type`](DataTransfer::has_type) should be used.
    // TODO: We should be able to do `&dyn TransferType`, but some implementation details in
    // the platforms make that unnecessarily difficult right now.
    fn available_types(&self) -> Box<dyn Iterator<Item = Box<dyn TransferType>> + '_>;

    /// Check if the supplied MIME type is provided by this [`DataTransfer`].
    fn has_type(&self, type_: &dyn TransferType) -> bool {
        type_.hint().is_some_and(|hint| {
            self.available_types().any(|haystack| haystack.hint() == Some(hint))
        })
    }
}

impl_dyn_casting!(DataTransfer);
