use std::ffi::{OsString, c_void};
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;
use std::ptr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};

use dpi::PhysicalPosition;
use tracing::debug;
use windows_sys::Win32::Foundation::{DV_E_FORMATETC, E_ABORT, HWND, POINT, POINTL, S_OK};
use windows_sys::Win32::Graphics::Gdi::ScreenToClient;
use windows_sys::Win32::System::Com::{DVASPECT_CONTENT, FORMATETC, TYMED_HGLOBAL};
use windows_sys::Win32::System::Ole::{CF_HDROP, DROPEFFECT_COPY, DROPEFFECT_NONE};
use windows_sys::Win32::UI::Shell::{DragQueryFileW, HDROP};
use windows_sys::core::{GUID, HRESULT};
use winit_core::data_transfer::DataTransferId;
use winit_core::event::WindowEvent;

use crate::definitions::{
    IDataObject, IDataObjectVtbl, IDropTarget, IDropTargetVtbl, IUnknown, IUnknownVtbl,
};

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

#[allow(dead_code, reason = "TODO")]
#[repr(C)]
pub struct DataObjectData {
    pub interface: IDataObject,
    refcount: AtomicUsize,
}

#[allow(dead_code, reason = "TODO")]
pub struct DataObject {
    data: *mut DataObjectData,
}

#[repr(C)]
pub struct FileDropHandlerData {
    interface: IDropTarget,
    refcount: AtomicUsize,
    window: HWND,
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
        shared: Arc<FileDropDataShared>,
        send_event: Box<dyn Fn(WindowEvent)>,
    ) -> FileDropHandler {
        let data = Box::new(FileDropHandlerData {
            interface: IDropTarget { lpVtbl: &DROP_TARGET_VTBL as *const IDropTargetVtbl },
            refcount: AtomicUsize::new(1),
            window,
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
            // Destroy the underlying data
            drop(unsafe { Box::from_raw(drop_handler as *mut FileDropHandlerData) });
        }
        count as u32
    }

    unsafe extern "system" fn DragEnter(
        this: *mut IDropTarget,
        _pDataObj: *const IDataObject,
        _grfKeyState: u32,
        pt: POINTL,
        pdwEffect: *mut u32,
    ) -> HRESULT {
        static DATA_TRANSFER_ID: AtomicI64 = AtomicI64::new(0);

        let drop_handler = unsafe { Self::from_interface(this) };
        let data_transfer_id =
            DataTransferId::from_raw(DATA_TRANSFER_ID.fetch_add(1, Ordering::Relaxed));
        drop_handler.active_data_transfer_id = Some(data_transfer_id);
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
        let Some(data_transfer_id) = drop_handler.active_data_transfer_id else {
            return E_ABORT;
        };

        (drop_handler.send_event)(WindowEvent::DragLeft { id: data_transfer_id });

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
        (drop_handler.send_event)(WindowEvent::DragDropped { id: data_transfer_id });
        unsafe {
            *pdwEffect = drop_handler.cursor_effect();
        }

        S_OK
    }

    unsafe fn from_interface<'a, InterfaceT>(this: *mut InterfaceT) -> &'a mut FileDropHandlerData {
        unsafe { &mut *(this as *mut _) }
    }

    #[allow(dead_code, reason = "TODO")]
    unsafe fn iterate_filenames<F>(data_obj: *const IDataObject, mut callback: F) -> Option<HDROP>
    where
        F: FnMut(PathBuf),
    {
        let drop_format = FORMATETC {
            cfFormat: CF_HDROP,
            ptd: ptr::null_mut(),
            dwAspect: DVASPECT_CONTENT,
            lindex: -1,
            tymed: TYMED_HGLOBAL as u32,
        };

        let mut medium = unsafe { std::mem::zeroed() };
        let get_data_fn = unsafe { (*(*data_obj).cast::<IDataObjectVtbl>()).GetData };
        let get_data_result = unsafe { get_data_fn(data_obj as *mut _, &drop_format, &mut medium) };
        if get_data_result >= 0 {
            let hdrop = unsafe { medium.u.hGlobal as HDROP };

            // The second parameter (0xFFFFFFFF) instructs the function to return the item count
            let item_count = unsafe { DragQueryFileW(hdrop, 0xffffffff, ptr::null_mut(), 0) };

            for i in 0..item_count {
                // Get the length of the path string NOT including the terminating null character.
                // Previously, this was using a fixed size array of MAX_PATH length, but the
                // Windows API allows longer paths under certain circumstances.
                let character_count =
                    unsafe { DragQueryFileW(hdrop, i, ptr::null_mut(), 0) as usize };
                let str_len = character_count + 1;

                // Fill path_buf with the null-terminated file name
                let mut path_buf = Vec::with_capacity(str_len);
                unsafe {
                    DragQueryFileW(hdrop, i, path_buf.as_mut_ptr(), str_len as u32);
                    path_buf.set_len(str_len);
                }

                callback(OsString::from_wide(&path_buf[0..character_count]).into());
            }

            Some(hdrop)
        } else if get_data_result == DV_E_FORMATETC {
            // If the dropped item is not a file this error will occur.
            // In this case it is OK to return without taking further action.
            debug!("Error occurred while processing dropped/hovered item: item is not a file.");
            None
        } else {
            debug!("Unexpected error occurred while processing dropped/hovered item.");
            None
        }
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
