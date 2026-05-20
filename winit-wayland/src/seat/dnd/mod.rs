use sctk::data_device_manager::data_device::DataDeviceHandler;
use sctk::data_device_manager::data_offer::DataOfferHandler;
use sctk::data_device_manager::data_source::DataSourceHandler;
use wayland_client::protocol::wl_data_device::WlDataDevice;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, QueueHandle};

use crate::state::WinitState;

impl DataSourceHandler for WinitState {
    fn accept_mime(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        source: &wayland_client::protocol::wl_data_source::WlDataSource,
        mime: Option<String>,
    ) {
        let _ = mime;
        let _ = source;
        let _ = qh;
        let _ = conn;
        todo!()
    }

    fn send_request(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        source: &wayland_client::protocol::wl_data_source::WlDataSource,
        mime: String,
        fd: sctk::data_device_manager::WritePipe,
    ) {
        let _ = fd;
        let _ = mime;
        let _ = source;
        let _ = qh;
        let _ = conn;
        todo!()
    }

    fn cancelled(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        source: &wayland_client::protocol::wl_data_source::WlDataSource,
    ) {
        let _ = source;
        let _ = qh;
        let _ = conn;
        todo!()
    }

    fn dnd_dropped(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        source: &wayland_client::protocol::wl_data_source::WlDataSource,
    ) {
        let _ = source;
        let _ = qh;
        let _ = conn;
        todo!()
    }

    fn dnd_finished(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        source: &wayland_client::protocol::wl_data_source::WlDataSource,
    ) {
        let _ = source;
        let _ = qh;
        let _ = conn;
        todo!()
    }

    fn action(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        source: &wayland_client::protocol::wl_data_source::WlDataSource,
        action: wayland_client::protocol::wl_data_device_manager::DndAction,
    ) {
        let _ = action;
        let _ = source;
        let _ = qh;
        let _ = conn;
        todo!()
    }
}

impl DataOfferHandler for WinitState {
    fn source_actions(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        offer: &mut sctk::data_device_manager::data_offer::DragOffer,
        actions: wayland_client::protocol::wl_data_device_manager::DndAction,
    ) {
        let _ = actions;
        let _ = offer;
        let _ = qh;
        let _ = conn;
        todo!()
    }

    fn selected_action(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        offer: &mut sctk::data_device_manager::data_offer::DragOffer,
        actions: wayland_client::protocol::wl_data_device_manager::DndAction,
    ) {
        let _ = actions;
        let _ = offer;
        let _ = qh;
        let _ = conn;
        todo!()
    }
}

impl DataDeviceHandler for WinitState {
    fn enter(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        data_device: &WlDataDevice,
        x: f64,
        y: f64,
        wl_surface: &WlSurface,
    ) {
        let _ = wl_surface;
        let _ = y;
        let _ = x;
        let _ = data_device;
        let _ = qh;
        let _ = conn;
        todo!()
    }

    fn leave(&mut self, conn: &Connection, qh: &QueueHandle<Self>, data_device: &WlDataDevice) {
        let _ = data_device;
        let _ = qh;
        let _ = conn;
        todo!()
    }

    fn motion(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        data_device: &WlDataDevice,
        x: f64,
        y: f64,
    ) {
        let _ = y;
        let _ = x;
        let _ = data_device;
        let _ = qh;
        let _ = conn;
        todo!()
    }

    fn selection(&mut self, conn: &Connection, qh: &QueueHandle<Self>, data_device: &WlDataDevice) {
        let _ = data_device;
        let _ = qh;
        let _ = conn;
        todo!()
    }

    fn drop_performed(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        data_device: &WlDataDevice,
    ) {
        let _ = data_device;
        let _ = qh;
        let _ = conn;
        todo!()
    }
}
