use std::collections::HashMap;
use std::ffi::{OsString, c_void};
use std::io;
use std::ops::ControlFlow;
use std::os::windows::ffi::OsStringExt;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};

use dpi::PhysicalPosition;
use windows_sys::Win32::Foundation::{E_ABORT, HGLOBAL, HWND, POINT, POINTL, S_OK};
use windows_sys::Win32::Graphics::Gdi::ScreenToClient;
use windows_sys::Win32::System::Com::{DVASPECT_CONTENT, FORMATETC, STGMEDIUM, TYMED_HGLOBAL};
use windows_sys::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
use windows_sys::Win32::System::Ole::{
    CF_HDROP, CF_UNICODETEXT, DROPEFFECT_COPY, DROPEFFECT_NONE, ReleaseStgMedium,
};
use windows_sys::Win32::UI::Shell::{DragQueryFileW, HDROP};
use windows_sys::core::{GUID, HRESULT};
use winit_core::data_transfer::{DataTransfer, DataTransferId, TransferType, TypeHint, TypedData};
use winit_core::event::WindowEvent;

use crate::definitions::{
    IDataObject, IDataObjectVtbl, IDropTarget, IDropTargetVtbl, IUnknown, IUnknownVtbl,
};
use crate::event_loop::EventLoopRunner;

#[derive(Default, Debug)]
pub struct FileDropDataShared {
    transfer_id: AtomicI64,
    pub accepted: AtomicBool,
}

impl FileDropDataShared {
    pub fn transfer_id(&self) -> DataTransferId {
        DataTransferId::from_raw(self.transfer_id.load(Ordering::Relaxed))
    }
}

#[derive(Debug)]
enum DataKind {
    Uris(Vec<OsString>),
    String(String),
    #[allow(dead_code, reason = "populated once more types are read eagerly")]
    Bytes(Vec<u8>),
}

// TODO: Exposing the full native API to client applications is too error-prone so long as
// winit is still manually implementing refcounting and using the win32 APIs. For now, we
// just eagerly read all the data supported by cross-platform type hints on Windows. This
// would be resolved by migrating to `windows-rs`.
#[derive(Debug)]
pub(crate) struct DataObject {
    data: HashMap<TypeHint, DataKind>,
}

impl DataObject {
    unsafe fn from_idataobject(data_obj: *const IDataObject) -> Self {
        let mut data = HashMap::new();

        if let Some(text) = unsafe { read_unicode_text(data_obj) } {
            data.insert(TypeHint::Plaintext, DataKind::String(text));
        }

        if let Some(uris) = unsafe { read_uri_list(data_obj) } {
            if !uris.is_empty() {
                data.insert(TypeHint::UriList, DataKind::Uris(uris));
            }
        }

        Self { data }
    }

    fn resolve(&self, requested: TypeHint) -> Option<TypeHint> {
        self.data.keys().copied().find(|stored| stored.matches(&requested))
    }
}

/// RAII wrapper around an STGMEDIUM returned by IDataObject::GetData, releasing it on drop.
struct StgMedium(STGMEDIUM);

impl StgMedium {
    /// Returns `None` if the object doesn't provide the format.
    unsafe fn get(data_obj: *const IDataObject, cf_format: u16) -> Option<Self> {
        let format = FORMATETC {
            cfFormat: cf_format,
            ptd: std::ptr::null_mut(),
            dwAspect: DVASPECT_CONTENT,
            lindex: -1,
            tymed: TYMED_HGLOBAL as u32,
        };

        let mut medium = unsafe { std::mem::zeroed::<STGMEDIUM>() };
        let get_data = unsafe { (*(*data_obj).cast::<IDataObjectVtbl>()).GetData };
        if unsafe { get_data(data_obj as *mut _, &format, &mut medium) } < 0 {
            return None;
        }

        Some(Self(medium))
    }

    fn hglobal(&self) -> HGLOBAL {
        unsafe { self.0.u.hGlobal }
    }
}

impl Drop for StgMedium {
    fn drop(&mut self) {
        unsafe { ReleaseStgMedium(&mut self.0) };
    }
}

unsafe fn read_unicode_text(data_obj: *const IDataObject) -> Option<String> {
    let medium = unsafe { StgMedium::get(data_obj, CF_UNICODETEXT) }?;
    let hglobal = medium.hglobal();

    let ptr = unsafe { GlobalLock(hglobal) };
    if ptr.is_null() {
        return None;
    }

    // `CF_UNICODETEXT` is a NUL-terminated UTF-16 string. Cap the scan by the allocation size in
    // case the buffer isn't terminated.
    let max_units = unsafe { GlobalSize(hglobal) } / std::mem::size_of::<u16>();
    let wide = unsafe { std::slice::from_raw_parts(ptr.cast::<u16>(), max_units) };
    let len = wide.iter().position(|&c| c == 0).unwrap_or(max_units);
    let text = String::from_utf16_lossy(&wide[..len]);

    unsafe { GlobalUnlock(hglobal) };

    Some(text)
}

unsafe fn read_uri_list(data_obj: *const IDataObject) -> Option<Vec<OsString>> {
    let medium = unsafe { StgMedium::get(data_obj, CF_HDROP) }?;
    let hdrop = medium.hglobal() as HDROP;

    // The second parameter (0xFFFFFFFF) instructs the function to return the item count.
    let item_count = unsafe { DragQueryFileW(hdrop, 0xffff_ffff, std::ptr::null_mut(), 0) };

    let mut paths = Vec::with_capacity(item_count as usize);
    for i in 0..item_count {
        // Get the length of the path string NOT including the terminating null character.
        // Previously, this was using a fixed size array of MAX_PATH length, but the Windows
        // API allows longer paths under certain circumstances.
        let character_count = unsafe { DragQueryFileW(hdrop, i, std::ptr::null_mut(), 0) } as usize;
        let str_len = character_count + 1;

        let mut path_buf = Vec::<u16>::with_capacity(str_len);
        unsafe {
            DragQueryFileW(hdrop, i, path_buf.as_mut_ptr(), str_len as u32);
            path_buf.set_len(str_len);
        }

        paths.push(OsString::from_wide(&path_buf[..character_count]));
    }

    Some(paths)
}

#[derive(Debug)]
pub(crate) struct WinDataTransfer {
    data: Rc<DataObject>,
}

impl WinDataTransfer {
    pub(crate) fn new(data: Rc<DataObject>) -> Self {
        Self { data }
    }
}

impl DataTransfer for WinDataTransfer {
    fn for_each_available_type<'this>(
        &'this self,
        func: &'_ mut dyn FnMut(&'this dyn TransferType) -> ControlFlow<()>,
    ) {
        for hint in self.data.data.keys() {
            if let ControlFlow::Break(()) = func(hint) {
                break;
            }
        }
    }
}

#[derive(Debug)]
pub(crate) struct WinTypedData {
    type_: TypeHint,
    data: Rc<DataObject>,
}

impl WinTypedData {
    pub(crate) fn new(data: Rc<DataObject>, requested: TypeHint) -> Option<Self> {
        let type_ = data.resolve(requested)?;
        Some(Self { type_, data })
    }
}

impl TypedData for WinTypedData {
    fn type_(&self) -> &dyn TransferType {
        &self.type_
    }

    fn try_read(&mut self) -> Option<Box<dyn io::BufRead>> {
        match self.data.data.get(&self.type_)? {
            DataKind::Bytes(bytes) => Some(Box::new(io::Cursor::new(bytes.clone()))),
            DataKind::String(string) => {
                Some(Box::new(io::Cursor::new(string.clone().into_bytes())))
            },
            DataKind::Uris(_) => None,
        }
    }

    fn try_as_uris(&mut self) -> io::Result<Vec<OsString>> {
        match self.data.data.get(&self.type_) {
            Some(DataKind::Uris(uris)) => Ok(uris.clone()),
            _ => Err(io::ErrorKind::InvalidData.into()),
        }
    }

    fn try_as_string(&mut self) -> io::Result<String> {
        match self.data.data.get(&self.type_) {
            Some(DataKind::String(string)) => Ok(string.clone()),
            _ => Err(io::ErrorKind::InvalidData.into()),
        }
    }
}

#[repr(C)]
pub struct FileDropHandlerData {
    interface: IDropTarget,
    refcount: AtomicUsize,
    window: HWND,
    runner: Rc<EventLoopRunner>,
    send_event: Box<dyn Fn(WindowEvent)>,
    shared: Arc<FileDropDataShared>,
    active_data_transfer_id: Option<DataTransferId>,
}

impl FileDropHandlerData {
    fn cursor_effect(&self) -> u32 {
        if self.shared.accepted.load(Ordering::Relaxed) {
            // TODO: Handle other kinds of drop effect
            DROPEFFECT_COPY
        } else {
            DROPEFFECT_NONE
        }
    }
}

pub struct FileDropHandler {
    data: *mut FileDropHandlerData,
}

#[allow(non_snake_case)]
impl FileDropHandler {
    pub(crate) fn new(
        window: HWND,
        runner: Rc<EventLoopRunner>,
        shared: Arc<FileDropDataShared>,
        send_event: Box<dyn Fn(WindowEvent)>,
    ) -> FileDropHandler {
        let data = Box::new(FileDropHandlerData {
            interface: IDropTarget { lpVtbl: &DROP_TARGET_VTBL as *const IDropTargetVtbl },
            refcount: AtomicUsize::new(1),
            window,
            runner,
            send_event,
            active_data_transfer_id: None,
            shared,
        });
        FileDropHandler { data: Box::into_raw(data) }
    }

    pub(crate) unsafe fn interface_unchecked_mut(&mut self) -> &mut IDropTarget {
        unsafe { &mut (*self.data).interface }
    }

    // Implement IUnknown
    unsafe extern "system" fn QueryInterface(
        _this: *mut IUnknown,
        _riid: *const GUID,
        _ppvObject: *mut *mut c_void,
    ) -> HRESULT {
        // This function doesn't appear to be required for an `IDropTarget`.
        // An implementation would be nice however.
        unimplemented!();
    }

    unsafe extern "system" fn AddRef(this: *mut IUnknown) -> u32 {
        let drop_handler_data = unsafe { Self::from_interface(this) };
        let count = drop_handler_data.refcount.fetch_add(1, Ordering::Release) + 1;
        count as u32
    }

    unsafe extern "system" fn Release(this: *mut IUnknown) -> u32 {
        let drop_handler = unsafe { Self::from_interface(this) };
        let count = drop_handler.refcount.fetch_sub(1, Ordering::Release) - 1;
        if count == 0 {
            // Drop any transfer still in flight (e.g. the window was destroyed mid-drag, so no
            // `DragLeave`/`Drop` ever arrived to clean it up).
            if let Some(id) = drop_handler.active_data_transfer_id.take() {
                drop_handler.runner.remove_data_transfer(id);
            }
            // Destroy the underlying data
            drop(unsafe { Box::from_raw(drop_handler as *mut FileDropHandlerData) });
        }
        count as u32
    }

    unsafe extern "system" fn DragEnter(
        this: *mut IDropTarget,
        pDataObj: *const IDataObject,
        _grfKeyState: u32,
        pt: POINTL,
        pdwEffect: *mut u32,
    ) -> HRESULT {
        static DATA_TRANSFER_ID: AtomicI64 = AtomicI64::new(0);

        let drop_handler = unsafe { Self::from_interface(this) };
        let data_transfer_id =
            DataTransferId::from_raw(DATA_TRANSFER_ID.fetch_add(1, Ordering::Relaxed));
        drop_handler.active_data_transfer_id = Some(data_transfer_id);

        // Make the new transfer visible to `accept_drag`/`reject_drag` and reset acceptance for
        // this drag.
        drop_handler.shared.transfer_id.store(data_transfer_id.into_raw(), Ordering::Relaxed);
        drop_handler.shared.accepted.store(false, Ordering::Relaxed);

        let data = Rc::new(unsafe { DataObject::from_idataobject(pDataObj) });
        drop_handler.runner.register_data_transfer(data_transfer_id, data);

        let mut pt = POINT { x: pt.x, y: pt.y };
        unsafe {
            ScreenToClient(drop_handler.window, &mut pt);
        }
        let position = PhysicalPosition::new(pt.x as f64, pt.y as f64);
        (drop_handler.send_event)(WindowEvent::DragEntered {
            id: data_transfer_id,
            position: Some(position),
        });
        unsafe {
            *pdwEffect = DROPEFFECT_NONE;
        }

        S_OK
    }

    unsafe extern "system" fn DragOver(
        this: *mut IDropTarget,
        _grfKeyState: u32,
        pt: POINTL,
        pdwEffect: *mut u32,
    ) -> HRESULT {
        let drop_handler = unsafe { Self::from_interface(this) };
        let Some(data_transfer_id) = drop_handler.active_data_transfer_id else {
            unsafe {
                *pdwEffect = DROPEFFECT_NONE;
            }

            return E_ABORT;
        };

        let mut pt = POINT { x: pt.x, y: pt.y };
        unsafe {
            ScreenToClient(drop_handler.window, &mut pt);
        }
        let position = PhysicalPosition::new(pt.x as f64, pt.y as f64);
        (drop_handler.send_event)(WindowEvent::DragPosition { id: data_transfer_id, position });
        unsafe {
            *pdwEffect = drop_handler.cursor_effect();
        }

        S_OK
    }

    unsafe extern "system" fn DragLeave(this: *mut IDropTarget) -> HRESULT {
        let drop_handler = unsafe { Self::from_interface(this) };
        let Some(data_transfer_id) = drop_handler.active_data_transfer_id.take() else {
            return E_ABORT;
        };

        (drop_handler.send_event)(WindowEvent::DragLeft { id: data_transfer_id });
        drop_handler.runner.remove_data_transfer(data_transfer_id);

        S_OK
    }

    unsafe extern "system" fn Drop(
        this: *mut IDropTarget,
        _pDataObj: *const IDataObject,
        _grfKeyState: u32,
        pt: POINTL,
        pdwEffect: *mut u32,
    ) -> HRESULT {
        let drop_handler = unsafe { Self::from_interface(this) };
        let Some(data_transfer_id) = drop_handler.active_data_transfer_id.take() else {
            unsafe {
                *pdwEffect = DROPEFFECT_NONE;
            }

            return E_ABORT;
        };

        let mut pt = POINT { x: pt.x, y: pt.y };
        unsafe {
            ScreenToClient(drop_handler.window, &mut pt);
        }

        let position = PhysicalPosition::new(pt.x as f64, pt.y as f64);
        (drop_handler.send_event)(WindowEvent::DragPosition { id: data_transfer_id, position });
        (drop_handler.send_event)(WindowEvent::DragDropped { id: data_transfer_id });
        unsafe {
            *pdwEffect = drop_handler.cursor_effect();
        }

        // The application has had a chance to read the data while handling `DragDropped`; the
        // transfer's lifecycle ends here.
        drop_handler.runner.remove_data_transfer(data_transfer_id);

        S_OK
    }

    unsafe fn from_interface<'a, InterfaceT>(this: *mut InterfaceT) -> &'a mut FileDropHandlerData {
        unsafe { &mut *(this as *mut _) }
    }
}

impl Drop for FileDropHandler {
    fn drop(&mut self) {
        unsafe {
            FileDropHandler::Release(self.data as *mut IUnknown);
        }
    }
}

static DROP_TARGET_VTBL: IDropTargetVtbl = IDropTargetVtbl {
    parent: IUnknownVtbl {
        QueryInterface: FileDropHandler::QueryInterface,
        AddRef: FileDropHandler::AddRef,
        Release: FileDropHandler::Release,
    },
    DragEnter: FileDropHandler::DragEnter,
    DragOver: FileDropHandler::DragOver,
    DragLeave: FileDropHandler::DragLeave,
    Drop: FileDropHandler::Drop,
};
