use std::{
    ffi::c_void,
    ptr::slice_from_raw_parts,
    sync::mpsc,
    time::Duration,
};

use c_api::PacketMonitorPacketType;
use log::{debug, error, info, trace, warn};
use windows::w;

use crate::{
    CaptureBackend, Packet, PacketPayload, filter::PktMonFilter, util::install_shutdown_hook,
};

mod c_api;
mod c_filter;

#[derive(Debug)]
struct MonitorContext {
    sender: mpsc::SyncSender<Packet>,
    stats: std::sync::Arc<crate::RealtimeStats>,

    #[cfg(feature = "tokio")]
    notify: Option<std::sync::Weak<tokio::sync::Notify>>,
}

type UserContext = *mut MonitorContext;

#[derive(Debug)]
pub struct RealTimeBackend {
    api: c_api::PacketMonitorApi,

    handle: c_api::PacketMonitorHandle,
    session: c_api::PacketMonitorSession,
    stream: c_api::PacketMonitorRealTimeStream,

    receiver: mpsc::Receiver<Packet>,

    #[allow(dead_code)]
    context: Box<MonitorContext>,

    #[cfg(feature = "tokio")]
    notify: Option<std::sync::Arc<tokio::sync::Notify>>,

    loaded: bool,
}

extern "system" fn event_callback(
    _context: *mut c_void,
    event_info: *const c_api::PacketMonitorStreamEventInfo,
    event_kind: c_api::PacketMonitorStreamEventKind,
) {
    match event_kind {
        c_api::PacketMonitorStreamEventKind::PacketMonitorStreamEventStarted => {
            let info = unsafe { &(*event_info).stream_start_info };
            debug!(
                "Packet monitor stream started with buffer size {} and truncation size {}",
                info.packet_buffer_size_in_bytes, info.truncation_size
            );
        }
        c_api::PacketMonitorStreamEventKind::PacketMonitorStreamEventStopped => {
            let info = unsafe { &(*event_info).stream_stop_info };
            if info.is_fatal_error {
                error!(
                    "Fatal packet monitor stream error with reason: {}",
                    info.reason
                );
            } else {
                debug!("Packet monitor stream stopped with reason: {}", info.reason);
            }
        }
        c_api::PacketMonitorStreamEventKind::PacketMonitorStreamEventFatalError => {
            error!("Fatal packet monitor stream event");
        }
        c_api::PacketMonitorStreamEventKind::PacketMonitorStreamEventProcessInfo => {
            let info = unsafe { &(*event_info).stream_process_info };
            if info.is_warning {
                warn!(
                    "Packet monitor stream warning reason: {}, packet length: {}",
                    info.reason, info.packet_length
                );
            } else {
                error!(
                    "Packet monitor stream error reason: {}, packet length: {}",
                    info.reason, info.packet_length
                );
            }
        }
    }
}

extern "system" fn data_callback(
    context: *mut c_void,
    data: *const c_api::PacketMonitorStreamDataDescriptor,
) {
    if context.is_null() || data.is_null() { return; }
    let context = unsafe { &*(context as UserContext) };

    let descriptor = unsafe { &(*data) };

    use std::sync::atomic::Ordering::Relaxed;
    context.stats.missed_write.fetch_add(u64::from(descriptor.missed_packet_write_count),Relaxed);
    context.stats.missed_read.fetch_add(u64::from(descriptor.missed_packet_read_count),Relaxed);
    let valid_range = |offset: u32, size: usize| (offset as usize).checked_add(size).is_some_and(|end| end <= descriptor.data_size as usize);
    if descriptor.data.is_null() || descriptor.packet_length==0 || descriptor.packet_length>131072
        || !valid_range(descriptor.metadata_offset,std::mem::size_of::<c_api::PacketMonitorStreamMetadata>())
        || !valid_range(descriptor.packet_offset,descriptor.packet_length as usize) {
        context.stats.invalid_descriptors.fetch_add(1,Relaxed); return;
    }
    let metadata = unsafe { std::ptr::read_unaligned((descriptor.data as *const u8).add(descriptor.metadata_offset as usize)
            as *const c_api::PacketMonitorStreamMetadata) };

    let packet_payload =
        unsafe { (descriptor.data as *const u8).add(descriptor.packet_offset as usize) };
    let packet_payload = slice_from_raw_parts(packet_payload, descriptor.packet_length as usize);
    let mut packet_payload_vector = Vec::new();
    packet_payload_vector.extend_from_slice(unsafe { &*packet_payload });

    // trace!("Cloned time: {:?}", start.elapsed().as_nanos());

    let packet = Packet {
        component_id: metadata.component_id,
        payload: match PacketMonitorPacketType::try_from(metadata.packet_type) {
            Ok(packet_type) => match packet_type {
                PacketMonitorPacketType::PktMonPayload_Unknown => {
                    PacketPayload::Unknown(packet_payload_vector)
                }
                PacketMonitorPacketType::PktMonPayload_Ethernet => {
                    PacketPayload::Ethernet(packet_payload_vector)
                }
                PacketMonitorPacketType::PktMonPayload_WiFi => {
                    PacketPayload::WiFi(packet_payload_vector)
                }
                PacketMonitorPacketType::PktMonPayload_IP => {
                    PacketPayload::IP(packet_payload_vector)
                }
                PacketMonitorPacketType::PktMonPayload_HTTP => {
                    PacketPayload::HTTP(packet_payload_vector)
                }
                PacketMonitorPacketType::PktMonPayload_TCP => {
                    PacketPayload::TCP(packet_payload_vector)
                }
                PacketMonitorPacketType::PktMonPayload_UDP => {
                    PacketPayload::UDP(packet_payload_vector)
                }
                PacketMonitorPacketType::PktMonPayload_ARP => {
                    PacketPayload::ARP(packet_payload_vector)
                }
                PacketMonitorPacketType::PktMonPayload_ICMP => {
                    PacketPayload::ICMP(packet_payload_vector)
                }
                PacketMonitorPacketType::PktMonPayload_ESP => {
                    PacketPayload::ESP(packet_payload_vector)
                }
                PacketMonitorPacketType::PktMonPayload_AH => {
                    PacketPayload::AH(packet_payload_vector)
                }
                PacketMonitorPacketType::PktMonPayload_L4Payload => {
                    PacketPayload::L4Payload(packet_payload_vector)
                }
            },
            Err(()) => PacketPayload::Unknown(packet_payload_vector),
        },
    };

    // let sender = context.sender.clone();
    // trace!("lock time: {:?}", start.elapsed().as_nanos());
    if let Err(mpsc::TrySendError::Full(_)) = context.sender.try_send(packet) {
        context.stats.callback_queue_drops.fetch_add(1,Relaxed);
    }
    // trace!("send time: {:?}", start.elapsed().as_nanos());

    #[cfg(feature = "tokio")]
    if let Some(ref notify) = context.notify {
        if let Some(notify) = notify.upgrade() {
            notify.notify_one();
        }
    }

}

impl RealTimeBackend {
    pub fn new() -> std::io::Result<Self> {
        Self::new_with_stats(std::sync::Arc::new(crate::RealtimeStats::default()))
    }

    pub fn new_with_stats(stats: std::sync::Arc<crate::RealtimeStats>) -> std::io::Result<Self> {
        let api = c_api::PacketMonitorApi::new()?;

        #[cfg(feature = "tokio")]
        let notify = Some(std::sync::Arc::new(tokio::sync::Notify::new()));

        let mut handle = c_api::PacketMonitorHandle::default();
        (api.initialize)(
            c_api::PACKETMONITOR_API_VERSION,
            std::ptr::null_mut(),
            &mut handle,
        )
        .ok()?;
        trace!("Initialized handle {:?}", handle);

        let mut session = c_api::PacketMonitorSession::default();
        (api.create_live_session)(handle, w!("PktMon Rust"), &mut session).ok()?;

        trace!("Created session {:?}", session);

        let (sender, receiver) = mpsc::sync_channel(8192);

        let context_box = Box::new(MonitorContext {
            sender,
            stats,

            #[cfg(feature = "tokio")]
            notify: notify.clone().map(|n| std::sync::Arc::downgrade(&n)),
        });
        let context_ptr = &*context_box as *const _;

        let stream_config = c_api::PacketMonitorRealTimeStreamConfiguration {
            user_context: context_ptr as *mut c_void,
            event_callback: event_callback as *mut _,
            data_callback: data_callback as *mut _,
            buffer_size_multiplier: 10, // Idk if this is a good default
            truncation_size: 9000,      // Max value
        };

        let mut stream = c_api::PacketMonitorRealTimeStream::default();
        (api.create_realtime_stream)(handle, &stream_config, &mut stream).ok()?;

        trace!("Created stream {:?}", stream);

        install_shutdown_hook(move || {
            if let Ok(api) = c_api::PacketMonitorApi::new() {
                (api.close_realtime_stream)(stream);
                (api.close_session_handle)(session);
                (api.uninitialize)(handle);
            }
        });

        (api.attach_output_to_session)(session, stream).ok()?;

        Ok(RealTimeBackend {
            api,

            handle,
            session,
            stream,

            context: context_box,

            receiver,

            #[cfg(feature = "tokio")]
            notify,

            loaded: true,
        })
    }

    pub fn select_network_interfaces(&mut self, component: Option<u32>) -> std::io::Result<()> {
        use std::io::{Error, ErrorKind};
        use c_api::{PacketMonitorDataSourceKind, PacketMonitorDataSourceList, PacketMonitorDataSourceSpecification};
        let kind = PacketMonitorDataSourceKind::PacketMonitorDataSourceKindNetworkInterface;
        let mut bytes_needed = 0usize;
        let first = (self.api.enum_data_sources)(self.handle,kind,false,0,&mut bytes_needed,std::ptr::null_mut());
        if bytes_needed == 0 { first.ok()?; return Err(Error::new(ErrorKind::NotFound,"No NIC sources")); }
        if bytes_needed > 16*1024*1024 { return Err(Error::new(ErrorKind::InvalidData,"Source list too large")); }
        let mut storage = vec![0u64; bytes_needed.div_ceil(8)];
        let capacity = storage.len()*8;
        let buffer = storage.as_mut_ptr() as *mut PacketMonitorDataSourceList;
        (self.api.enum_data_sources)(self.handle,kind,false,capacity,&mut bytes_needed,buffer).ok()?;
        if bytes_needed>capacity { return Err(Error::new(ErrorKind::InvalidData,"Source list changed")); }
        let count = unsafe { (*buffer).num_data_sources as usize };
        let offset = std::mem::offset_of!(PacketMonitorDataSourceList,data_sources);
        if count > capacity.saturating_sub(offset)/std::mem::size_of::<usize>() { return Err(Error::new(ErrorKind::InvalidData,"Invalid source array")); }
        let base=buffer as usize;
        let mut selected=0u64;
        for index in 0..count {
            let ptr=unsafe { *((buffer as *const u8).add(offset) as *const *const PacketMonitorDataSourceSpecification).add(index) };
            let address=ptr as usize;
            if address<base || address.checked_add(std::mem::size_of::<PacketMonitorDataSourceSpecification>()).is_none_or(|end|end>base+capacity) { return Err(Error::new(ErrorKind::InvalidData,"Invalid source pointer")); }
            let source=unsafe { std::ptr::read_unaligned(ptr) };
            if component.is_none_or(|id|id==source.id) {
                (self.api.add_single_data_source_to_session)(self.session,&source).ok()?;
                selected+=1;
            }
        }
        if selected==0 { return Err(Error::new(ErrorKind::NotFound,"Requested NIC not found")); }
        self.context.stats.selected_sources.store(selected,std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }

}

impl CaptureBackend for RealTimeBackend {
    fn start(&mut self) -> std::io::Result<()> {
        info!("Setting session active");
        (self.api.set_session_active)(self.session, true).ok()?;

        Ok(())
    }

    fn stop(&mut self) -> std::io::Result<()> {
        info!("Setting session inactive");
        (self.api.set_session_active)(self.session, false).ok()?;

        Ok(())
    }

    fn unload(&mut self) -> std::io::Result<()> {
        if self.loaded {
            debug!("Closing stream");
            (self.api.close_realtime_stream)(self.stream);

            debug!("Closing session");
            (self.api.close_session_handle)(self.session);

            debug!("Uninitializing");
            (self.api.uninitialize)(self.handle);

            self.loaded = false;
        }

        Ok(())
    }

    fn add_filter(&mut self, filter: PktMonFilter) -> std::io::Result<()> {
        info!("Adding filter");
        (self.api.add_capture_constraint)(self.session, &filter.into()).ok()?;

        Ok(())
    }

    fn next_packet(&self) -> Result<Packet, mpsc::RecvError> {
        debug!("Receiving packet");
        self.receiver.recv()
    }

    fn next_packet_timeout(&self, timeout: Duration) -> Result<Packet, mpsc::RecvTimeoutError> {
        debug!("Receiving packet with timeout");
        self.receiver.recv_timeout(timeout)
    }

    fn try_next_packet(&self) -> Result<Packet, mpsc::TryRecvError> {
        self.receiver.try_recv()
    }

    #[cfg(feature = "tokio")]
    fn notify(&self) -> Option<std::sync::Arc<tokio::sync::Notify>> {
        self.notify.clone()
    }
}

impl Drop for RealTimeBackend {
    fn drop(&mut self) {
        self.unload().unwrap();
    }
}

#[cfg(test)]
mod vision_tests {
    use super::*;
    use std::sync::{Arc, atomic::Ordering::Relaxed};

    #[test]
    fn callback_bounds_loss_counters_bounded_queue_and_closed_receiver() {
        let (sender, receiver)=mpsc::sync_channel(1);
        let stats=Arc::new(crate::RealtimeStats::default());
        let context=MonitorContext {sender,stats:stats.clone(), #[cfg(feature="tokio")] notify:None};
        let context_ptr=(&context as *const MonitorContext) as *mut c_void;
        let metadata_size=std::mem::size_of::<c_api::PacketMonitorStreamMetadata>();
        let mut bytes=vec![0u8;metadata_size+3];
        let mut metadata: c_api::PacketMonitorStreamMetadata=unsafe {std::mem::zeroed()};
        metadata.component_id=42; metadata.packet_type=1;
        unsafe {std::ptr::write_unaligned(bytes.as_mut_ptr().cast(),metadata)};
        bytes[metadata_size..].copy_from_slice(&[1,2,3]);
        let mut descriptor=c_api::PacketMonitorStreamDataDescriptor {
            data:bytes.as_ptr().cast(),data_size:bytes.len() as u32,metadata_offset:0,
            packet_offset:metadata_size as u32,packet_length:3,missed_packet_write_count:2,missed_packet_read_count:3,
        };
        data_callback(context_ptr,&descriptor); data_callback(context_ptr,&descriptor);
        assert_eq!(stats.missed_write.load(Relaxed),4); assert_eq!(stats.missed_read.load(Relaxed),6);
        assert_eq!(stats.callback_queue_drops.load(Relaxed),1);
        let packet=receiver.try_recv().unwrap(); assert_eq!(packet.component_id,42);
        assert_eq!(packet.payload.to_vec(),&vec![1,2,3]);
        descriptor.packet_offset=u32::MAX;
        data_callback(context_ptr,&descriptor);
        assert_eq!(stats.invalid_descriptors.load(Relaxed),1);
        descriptor.packet_offset=metadata_size as u32;
        drop(receiver); data_callback(context_ptr,&descriptor); // must not panic across FFI
    }

    #[test]
    #[cfg(target_pointer_width="64")]
    fn windows_x64_structure_layouts() {
        assert_eq!(std::mem::size_of::<c_api::PacketMonitorStreamDataDescriptor>(),32);
        assert_eq!(std::mem::size_of::<c_api::PacketMonitorStreamMetadata>(),48);
        assert_eq!(std::mem::offset_of!(c_api::PacketMonitorStreamMetadata,timestamp),40);
        assert_eq!(std::mem::offset_of!(c_api::PacketMonitorDataSourceSpecification,detail),408);
        assert_eq!(std::mem::size_of::<c_api::PacketMonitorDataSourceSpecification>(),424);
    }
}
