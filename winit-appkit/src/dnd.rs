use std::cell::RefCell;
use std::collections::HashMap;
use std::io;
use std::ops::Deref;
use std::sync::atomic::{AtomicI64, Ordering};

use objc2::Message;
use objc2::rc::{Retained, Weak};
use objc2_app_kit::{
    NSPasteboard, NSPasteboardType, NSPasteboardTypeFileURL, NSPasteboardTypeHTML,
    NSPasteboardTypePNG, NSPasteboardTypeSound, NSPasteboardTypeString, NSPasteboardTypeTIFF,
};
use objc2_foundation::{NSArray, NSData, NSString};
use winit_core::data_transfer::{DataTransfer, DataTransferId, TransferType, TypeHint, TypedData};
use winit_core::event_loop::AsyncRequestSerial;

#[derive(Debug, Clone)]
pub struct PasteboardType {
    hint: Option<TypeHint>,
    // We need to convert `NSString` to `str` since `NSString` isn't `Send`/`Sync`
    inner: Retained<NSPasteboardType>,
}

impl PasteboardType {
    fn from_hint(hint: TypeHint) -> Option<Self> {
        let hint_to_pasteboard_type = unsafe {
            [
                (TypeHint::UriList, NSPasteboardTypeFileURL),
                (TypeHint::Plaintext, NSPasteboardTypeString),
                (TypeHint::Html, NSPasteboardTypeHTML),
                (TypeHint::Image { extension_hint: Some("png") }, NSPasteboardTypePNG),
                (TypeHint::Image { extension_hint: Some("tiff") }, NSPasteboardTypeTIFF),
                (TypeHint::Audio { extension_hint: None }, NSPasteboardTypeSound),
            ]
        };

        hint_to_pasteboard_type.into_iter().find_map(|(haystack, inner)| {
            (haystack == hint).then(|| Self { hint: Some(hint), inner: inner.retain() })
        })
    }
}

impl Deref for PasteboardType {
    type Target = Retained<NSPasteboardType>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

pub struct InvalidPasteboardTypeError(pub TypeHint);

impl From<Retained<NSPasteboardType>> for PasteboardType {
    fn from(value: Retained<NSPasteboardType>) -> Self {
        let pasteboard_type_to_hint = unsafe {
            [
                #[expect(deprecated)]
                (objc2_app_kit::NSFilenamesPboardType, TypeHint::UriList),
                (NSPasteboardTypeFileURL, TypeHint::UriList),
                (NSPasteboardTypeString, TypeHint::Plaintext),
                (NSPasteboardTypeHTML, TypeHint::Html),
                (NSPasteboardTypePNG, TypeHint::Image { extension_hint: Some("png") }),
                (NSPasteboardTypeTIFF, TypeHint::Image { extension_hint: Some("tiff") }),
                (NSPasteboardTypeSound, TypeHint::Audio { extension_hint: None }),
            ]
        };

        let hint = pasteboard_type_to_hint
            .iter()
            .find_map(|(pb_type, hint)| (**pb_type == *value).then_some(hint));

        Self { hint: hint.copied(), inner: value }
    }
}

impl TransferType for PasteboardType {
    fn hint(&self) -> Option<winit_core::data_transfer::TypeHint> {
        self.hint
    }
}

#[derive(Clone, Debug)]
pub struct Pasteboard {
    transfer_id: DataTransferId,
    inner: Retained<NSPasteboard>,
}

impl Deref for Pasteboard {
    type Target = Retained<NSPasteboard>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl Pasteboard {
    pub(crate) fn set_pasteboard(&mut self, pasteboard: Retained<NSPasteboard>) {
        self.inner = pasteboard;
    }

    pub fn id(&self) -> DataTransferId {
        self.transfer_id
    }

    pub fn with_type(&self, dyn_type: &dyn TransferType) -> Option<PasteboardValue> {
        if self.has_type(dyn_type) {
            Some(PasteboardValue {
                type_: PasteboardTypeSpec::from_dyn(dyn_type)?,
                inner: self.clone(),
            })
        } else {
            None
        }
    }
}

impl DataTransfer for Pasteboard {
    fn available_types(&self) -> Vec<Box<dyn TransferType>> {
        self.inner
            .types()
            .map(|types| {
                types
                    .into_iter()
                    .map(|pb_type| Box::new(PasteboardType::from(pb_type)) as _)
                    .collect()
            })
            .unwrap_or_default()
    }

    fn has_type(&self, type_: &dyn TransferType) -> bool {
        let Some(pb_types) = self.inner.types() else {
            return false;
        };

        if let Some(needle) = type_.cast_ref::<PasteboardType>().cloned() {
            pb_types.iter().any(|haystack| **needle == *haystack)
        } else if let Some(needle) = type_.hint() {
            pb_types.iter().any(|haystack| PasteboardType::from(haystack).hint() == Some(needle))
        } else {
            false
        }
    }
}

#[derive(Debug, Clone)]
enum PasteboardTypeSpec {
    PasteboardType(PasteboardType),
    TypeHint(TypeHint),
}

impl PasteboardTypeSpec {
    fn from_dyn(type_: &dyn TransferType) -> Option<Self> {
        match type_.cast_ref::<PasteboardType>() {
            Some(pb_type) => Some(Self::PasteboardType(pb_type.clone())),
            None => type_.hint().map(Into::into),
        }
    }
}

impl From<TypeHint> for PasteboardTypeSpec {
    fn from(value: TypeHint) -> Self {
        match PasteboardType::from_hint(value) {
            Some(pb_type) => Self::PasteboardType(pb_type),
            None => Self::TypeHint(value),
        }
    }
}

impl PasteboardTypeSpec {
    fn pasteboard_type(&self) -> Option<&PasteboardType> {
        match self {
            PasteboardTypeSpec::PasteboardType(pasteboard_type) => Some(pasteboard_type),
            PasteboardTypeSpec::TypeHint(_) => None,
        }
    }
}

#[derive(Debug)]
pub struct PasteboardValue {
    // The concept of "top-level" types for a pasteboard doesn't always make sense on macOS due to
    // the use of `pasteboardItems`, so we allow using `TypeHint` instead to preserve the user's
    // intention.
    type_: PasteboardTypeSpec,
    inner: Pasteboard,
}

impl PasteboardValue {
    fn single_file_url(&self) -> Option<String> {
        self.inner
            .stringForType(unsafe { NSPasteboardTypeFileURL })
            .map(|ns_str| ns_str.to_string())
    }
}

impl Deref for PasteboardValue {
    type Target = Retained<NSPasteboard>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl TypedData for PasteboardValue {
    fn type_(&self) -> &dyn TransferType {
        match &self.type_ {
            PasteboardTypeSpec::PasteboardType(pasteboard_type) => pasteboard_type,
            PasteboardTypeSpec::TypeHint(type_hint) => type_hint,
        }
    }

    fn try_read(&mut self) -> Option<Box<dyn std::io::BufRead>> {
        struct DataReader {
            inner: Retained<NSData>,
            offset: usize,
        }

        impl DataReader {
            fn new(data: Retained<NSData>) -> Self {
                Self { inner: data, offset: 0 }
            }
        }

        impl io::Read for DataReader {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                let end = (self.offset + buf.len()).min(self.inner.len());
                let range = self.offset..end;
                self.offset = end;
                let bytes = unsafe { self.inner.as_bytes_unchecked() };
                let src = &bytes[range];
                buf[..src.len()].copy_from_slice(src);

                Ok(src.len())
            }
        }

        impl io::BufRead for DataReader {
            fn fill_buf(&mut self) -> io::Result<&[u8]> {
                Ok(unsafe { self.inner.as_bytes_unchecked() }
                    .get(self.offset..)
                    .unwrap_or_default())
            }

            fn consume(&mut self, amount: usize) {
                self.offset = (self.offset + amount).min(self.inner.len())
            }
        }

        self.inner
            .dataForType(self.type_.pasteboard_type()?)
            .map(|data| Box::new(DataReader::new(data)) as _)
    }

    fn try_as_uris(&mut self) -> io::Result<Vec<String>> {
        // TODO: We should probably use `readObjects`, need to check how that works.
        if self.type_().hint() != Some(TypeHint::UriList) {
            return Err(io::ErrorKind::InvalidData.into());
        }

        let Some(items) = self.inner.pasteboardItems() else {
            // The pasteboard didn't expose any items, so we try with the deprecated method.
            #[expect(deprecated)]
            let property_list = match self
                .inner
                .propertyListForType(unsafe { objc2_app_kit::NSFilenamesPboardType })
            {
                Some(property_list) => property_list,
                None => {
                    return self
                        .single_file_url()
                        .map(|str| vec![str])
                        .ok_or_else(|| io::ErrorKind::InvalidData.into());
                },
            };

            let paths = property_list
                .downcast::<NSArray>()
                .unwrap()
                .into_iter()
                .map(|file| file.downcast::<NSString>().unwrap().to_string())
                .collect();

            return Ok(paths);
        };

        Ok(items
            .into_iter()
            .filter_map(|item| item.stringForType(unsafe { NSPasteboardTypeFileURL }))
            .map(|ns_str| ns_str.to_string())
            .collect())
    }

    fn try_as_string(&mut self) -> io::Result<String> {
        self.inner
            .stringForType(self.type_.pasteboard_type().ok_or(io::ErrorKind::InvalidData)?)
            .map(|ns_str| ns_str.to_string())
            .ok_or_else(|| io::ErrorKind::InvalidData.into())
    }

    fn wait_for_data(&self) -> io::Result<()> {
        // The methods on `NSPasteboard` already wait without danger of deadlock
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct DndState {
    inner: RefCell<HashMap<DataTransferId, Weak<NSPasteboard>>>,
}

impl DndState {
    pub fn remove_deloaded_pasteboards(&self) {
        self.inner.borrow_mut().retain(|_, v| v.load().is_some());
    }

    /// If the data transfer exists, update the pasteboard it points to.
    pub fn set_pasteboard(&self, id: DataTransferId, pb: &Retained<NSPasteboard>) {
        let mut inner = self.inner.borrow_mut();
        if let Some(state) = inner.get_mut(&id) {
            *state = Weak::from_retained(pb);
        }
    }

    pub fn insert(&self, pb: &Retained<NSPasteboard>) -> DataTransferId {
        static TRANSFER_ID: AtomicI64 = AtomicI64::new(0);

        let id = TRANSFER_ID.fetch_add(1, Ordering::Relaxed);
        let id = DataTransferId::from_raw(id);

        self.inner.borrow_mut().insert(id, Weak::from_retained(pb));

        id
    }

    pub fn get(&self, id: DataTransferId) -> Option<Pasteboard> {
        self.inner
            .borrow()
            .get(&id)
            .and_then(|weak| weak.load())
            .map(|pb| Pasteboard { transfer_id: id, inner: pb })
    }
}
