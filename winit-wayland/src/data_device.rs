impl DataOfferHandler for WinitState {
    fn source_actions(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        offer: &mut DragOffer,
        actions: DndAction,
    ) {
        let seat = offer.winit_data().seat();
        let seat_state = match self.seats.get(&seat.id()) {
            Some(seat_state) => seat_state,
            None => {
                warn!("Received pointer event without seat");
                return;
            },
        };

        let themed_pointer = match seat_state.pointer.as_ref() {
            Some(pointer) => pointer,
            None => {
                warn!("Received pointer event without pointer");
                return;
            },
        };

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
