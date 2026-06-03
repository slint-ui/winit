use std::cell::{OnceCell, RefCell};
use std::collections::HashMap;
use std::ffi::OsString;
use std::io;
use std::ops::{ControlFlow, Deref};
use std::rc::Rc;

use objc2::rc::{Retained, Weak};
use objc2::runtime::AnyObject;
use objc2::{AnyThread, DefinedClass as _, Message, define_class, msg_send};
use objc2_app_kit::{
    NSDragOperation, NSPasteboard, NSPasteboardType, NSPasteboardTypeFileURL, NSPasteboardTypeHTML,
    NSPasteboardTypePNG, NSPasteboardTypeSound, NSPasteboardTypeString, NSPasteboardTypeTIFF,
    NSPasteboardWriting, NSPasteboardWritingOptions,
};
use objc2_foundation::{NSArray, NSData, NSObject, NSObjectProtocol, NSString};
use winit_core::data_transfer::{
    DataTransfer, DataTransferId, DataTransferSend, SendData, TransferType, TypeHint, TypedData,
};
use winit_core::event_loop::{DndActionMask, DndActions};

/// A thin wrapper around [`NSPasteboardType`], implementing [`TransferType`].
#[derive(PartialEq, Eq, Debug, Clone)]
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
            (haystack.matches(&hint)).then(|| Self { hint: Some(hint), inner: inner.retain() })
        })
    }
}

impl Deref for PasteboardType {
    type Target = Retained<NSPasteboardType>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl From<Retained<NSPasteboardType>> for PasteboardType {
    fn from(value: Retained<NSPasteboardType>) -> Self {
        let pasteboard_type_to_hint = unsafe {
            [
                // Just in case the source application uses the deprecated method, we handle it
                // here
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

    fn matches(&self, other: &dyn TransferType) -> bool {
        if let Some(other_pb_type) = other.cast_ref::<Self>() {
            *self == *other_pb_type
        } else {
            // If either hint is `None`, return false
            self.hint().is_some_and(|hint| other.hint() == Some(hint))
        }
    }
}

/// A thin wrapper around [`NSPasteboard`], implementing [`DataTransfer`].
#[derive(Clone, Debug)]
pub struct Pasteboard<PB = Retained<NSPasteboard>> {
    transfer_id: DataTransferId,
    inner: PB,
    types: OnceCell<Rc<[PasteboardType]>>,
}

impl Deref for Pasteboard {
    type Target = Retained<NSPasteboard>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl Pasteboard {
    fn new(transfer_id: DataTransferId, pasteboard: Retained<NSPasteboard>) -> Self {
        Self { transfer_id, inner: pasteboard, types: Default::default() }
    }

    /// Get the array of [`PasteboardType`]s advertized by this [`Pasteboard`].
    pub fn types(&self) -> &[PasteboardType] {
        self.types.get_or_init(|| {
            self.inner
                .types()
                .map(|types| types.into_iter().map(PasteboardType::from).collect::<Vec<_>>())
                .unwrap_or_default()
                .into()
        })
    }

    /// Get the `DataTransferId` of this pasteboard.
    pub fn id(&self) -> DataTransferId {
        self.transfer_id
    }

    /// Get a typed reader for this pasteboard. This is only necessary in the cross-platform case,
    /// as a user downcasting to the platform-specific type can just access the `NSPasteboard`
    /// directly.
    pub(crate) fn with_type(&self, dyn_type: &dyn TransferType) -> Option<PasteboardValue> {
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
    fn for_each_available_type<'this>(
        &'this self,
        func: &'_ mut dyn FnMut(&'this dyn TransferType) -> std::ops::ControlFlow<()>,
    ) {
        let _ = self.types().iter().map(|mime| mime as &dyn TransferType).try_for_each(func);
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

/// A thin wrapper around [`NSDragOperation`], implementing [`DndActionMask`].
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct DragOperation(pub NSDragOperation);

impl DragOperation {
    /// An empty set of drag operations
    pub fn empty() -> Self {
        Self(NSDragOperation::empty())
    }

    pub(crate) fn from_dyn(actions: &dyn DndActionMask) -> Self {
        if let Some(op) = actions.cast_ref::<Self>() {
            *op
        } else {
            match actions.hint() {
                DndActions::Flags { move_, copy, link } => {
                    let move_flag =
                        if move_ { NSDragOperation::Move } else { NSDragOperation::empty() };
                    let copy_flag =
                        if copy { NSDragOperation::Copy } else { NSDragOperation::empty() };
                    let link_flag =
                        if link { NSDragOperation::Link } else { NSDragOperation::empty() };
                    Self(move_flag | copy_flag | link_flag)
                },
                DndActions::All => Self(NSDragOperation::all()),
            }
        }
    }

    fn intersection(&self, other: &Self) -> Self {
        Self(self.0.intersection(other.0))
    }

    fn intersects(&self, other: &Self) -> bool {
        self.0.intersects(other.0)
    }
}

impl DndActionMask for DragOperation {
    fn hint(&self) -> DndActions {
        if self.0.is_all() {
            DndActions::All
        } else {
            DndActions::Flags {
                move_: self.0.contains(NSDragOperation::Move),
                copy: self.0.contains(NSDragOperation::Copy),
                link: self.0.contains(NSDragOperation::Link),
            }
        }
    }

    fn intersection(&self, other: &dyn DndActionMask) -> Box<dyn DndActionMask> {
        Box::new(self.intersection(&Self::from_dyn(other)))
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn intersects(&self, other: &dyn DndActionMask) -> bool {
        self.intersects(&Self::from_dyn(other))
    }
}

/// A thin wrapper around [`NSPasteboard`], implementing [`TypedValue`].
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

    fn try_as_uris(&mut self) -> io::Result<Vec<OsString>> {
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
                        .map(|str| vec![str.into()])
                        .ok_or_else(|| io::ErrorKind::InvalidData.into());
                },
            };

            let paths = property_list
                .downcast::<NSArray>()
                .unwrap()
                .into_iter()
                .map(|file| file.downcast::<NSString>().unwrap().to_string().into())
                .collect();

            return Ok(paths);
        };

        Ok(items
            .into_iter()
            .filter_map(|item| item.stringForType(unsafe { NSPasteboardTypeFileURL }))
            .map(|ns_str| ns_str.to_string().into())
            .collect())
    }

    fn try_as_string(&mut self) -> io::Result<String> {
        self.inner
            .stringForType(self.type_.pasteboard_type().ok_or(io::ErrorKind::InvalidData)?)
            .map(|ns_str| ns_str.to_string())
            .ok_or_else(|| io::ErrorKind::InvalidData.into())
    }
}

#[derive(Debug, Default)]
pub struct Pasteboards {
    inner: RefCell<HashMap<DataTransferId, Weak<NSPasteboard>>>,
}

impl Pasteboards {
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

    pub fn insert(&self, transfer_id: DataTransferId, pb: &Retained<NSPasteboard>) {
        self.inner.borrow_mut().insert(transfer_id, Weak::from_retained(pb));
    }

    pub fn get(&self, id: DataTransferId) -> Option<Pasteboard> {
        self.inner.borrow().get(&id).and_then(|weak| weak.load()).map(|pb| Pasteboard::new(id, pb))
    }
}

pub(crate) struct PasteboardWriterState {
    data: Box<dyn DataTransferSend>,
    // The macOS drag-and-drop API has some confusing aspects when handling multi-drag. The best
    // we can really do is have the first element contain all the cross-platform items, and
    // any further items are file paths only.
    uri: Option<Retained<NSString>>,
    writable_types: Retained<NSArray<NSPasteboardType>>,
}

impl PasteboardWriter {
    pub(crate) fn new(
        value: Box<dyn DataTransferSend>,
        uri: Option<Retained<NSString>>,
    ) -> Retained<Self> {
        let mut writable_types = Vec::<Retained<NSPasteboardType>>::new();
        value.for_each_available_type(&mut |type_| {
            let Some(spec) = PasteboardTypeSpec::from_dyn(type_) else {
                return ControlFlow::Continue(());
            };

            let Some(pb_type) = spec.pasteboard_type() else {
                return ControlFlow::Continue(());
            };

            writable_types.push((**pb_type).clone());

            ControlFlow::Continue(())
        });

        let pb_writer = Self::alloc().set_ivars(PasteboardWriterState {
            data: value,
            uri,
            writable_types: NSArray::from_retained_slice(&writable_types),
        });

        // Unsure if there's an easier way to do this, but this is how `WindowDelegate` does it.
        unsafe { msg_send![super(pb_writer), init] }
    }
}

impl PasteboardWriterState {
    fn data_for_pasteboard_type(
        &self,
        pasteboard_type: &NSPasteboardType,
    ) -> Option<Retained<AnyObject>> {
        if pasteboard_type == unsafe { NSPasteboardTypeFileURL } {
            if let Some(out) = self.uri.clone().map(Into::into) {
                return Some(out);
            }
        }
        let pb_type = PasteboardType::from(pasteboard_type.retain());

        let mut out = None;

        self.data.for_each_available_type(&mut |haystack| {
            if haystack.matches(&pb_type) {
                out = self.data.data_for_type(haystack);
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        });

        match out? {
            // This should be handled separately
            // TODO: Is there a better way to do this?
            SendData::Uris(_) => None,
            SendData::String(string) => Some(NSString::from_str(&string).into()),
            SendData::Bytes(binary) => Some(NSData::from_vec(binary).into()),
        }
    }
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = AnyThread]
    #[name = "WinitPasteboardWriter"]
    #[ivars = PasteboardWriterState]
    pub(crate) struct PasteboardWriter;

    unsafe impl NSObjectProtocol for PasteboardWriter {}

    unsafe impl NSPasteboardWriting for PasteboardWriter {
        #[unsafe(method_id(writableTypesForPasteboard:))]
        fn writable_types_for_pasteboard(
            &self,
            _: &NSPasteboard,
        ) -> Retained<NSArray<NSPasteboardType>> {
            let vars = self.ivars();
            vars.writable_types.clone()
        }

        #[unsafe(method(writingOptionsForType:pasteboard:))]
        fn writing_options_for_type(
            &self,
            type_: &NSPasteboardType,
            pasteboard: &NSPasteboard,
        ) -> NSPasteboardWritingOptions {
            let _ = type_;
            let _ = pasteboard;
            // TODO: Not necessarily ideal to always use `Promised`, but
            // it's good enough for now.
            NSPasteboardWritingOptions::empty()
        }

        #[unsafe(method_id(pasteboardPropertyListForType:))]
        fn pasteboard_property_list_for_type(
            &self,
            type_: &NSPasteboardType,
        ) -> Option<Retained<AnyObject>> {
            let vars = self.ivars();
            vars.data_for_pasteboard_type(type_)
        }
    }
);
