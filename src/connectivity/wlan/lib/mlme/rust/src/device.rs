// Copyright 2021 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use {
    crate::{buffer::OutBuf, key},
    banjo_fuchsia_hardware_wlan_associnfo::*,
    banjo_fuchsia_hardware_wlan_softmac::{
        self as banjo_wlan_softmac, WlanRxPacket, WlanSoftmacInfo, WlanTxPacket,
    },
    banjo_fuchsia_wlan_common::{self as banjo_common, WlanTxStatus},
    banjo_fuchsia_wlan_ieee80211 as banjo_ieee80211,
    banjo_fuchsia_wlan_internal::BssConfig,
    fidl_fuchsia_wlan_mlme as fidl_mlme, fuchsia_zircon as zx,
    ieee80211::MacAddr,
    std::{ffi::c_void, marker::PhantomData},
    wlan_common::{mac::FrameControl, tx_vector, TimeUnit},
};

#[cfg(test)]
pub use test_utils::*;

#[derive(Debug, PartialEq)]
pub struct LinkStatus(u8);
impl LinkStatus {
    pub const DOWN: Self = Self(0);
    pub const UP: Self = Self(1);
}

impl From<fidl_mlme::ControlledPortState> for LinkStatus {
    fn from(state: fidl_mlme::ControlledPortState) -> Self {
        match state {
            fidl_mlme::ControlledPortState::Open => Self::UP,
            fidl_mlme::ControlledPortState::Closed => Self::DOWN,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct TxFlags(pub u32);
impl TxFlags {
    pub const NONE: Self = Self(0);

    pub const PROTECTED: Self = Self(1);
    pub const FAVOR_RELIABILITY: Self = Self(1 << 1);
    // TODO(fxbug.dev/29622): remove once MLME supports QoS tag.
    pub const QOS: Self = Self(1 << 2);
}

pub struct Device {
    raw_device: DeviceInterface,
    minstrel: Option<crate::MinstrelWrapper>,
    control_handle: fidl_mlme::MlmeControlHandle,
}

const REQUIRED_WLAN_HEADER_LEN: usize = 10;
const PEER_ADDR_OFFSET: usize = 4;

impl Device {
    pub fn new(
        raw_device: DeviceInterface,
        minstrel: Option<crate::MinstrelWrapper>,
        control_handle: fidl_mlme::MlmeControlHandle,
    ) -> Self {
        Self { raw_device, minstrel, control_handle }
    }
    pub fn mlme_control_handle(&self) -> &fidl_mlme::MlmeControlHandle {
        &self.control_handle
    }

    pub fn deliver_eth_frame(&self, slice: &[u8]) -> Result<(), zx::Status> {
        self.raw_device.deliver_eth_frame(slice)
    }

    pub fn send_wlan_frame(&self, buf: OutBuf, mut flags: TxFlags) -> Result<(), zx::Status> {
        if buf.as_slice().len() < REQUIRED_WLAN_HEADER_LEN {
            return Err(zx::Status::BUFFER_TOO_SMALL);
        }
        // Unwrap is safe since the byte slice is always the same size.
        let frame_control =
            zerocopy::LayoutVerified::<&[u8], FrameControl>::new(&buf.as_slice()[0..=1])
                .unwrap()
                .into_ref();
        if frame_control.protected() {
            flags.0 |= banjo_wlan_softmac::WlanTxInfoFlags::PROTECTED.0 as u32;
        }
        let mut peer_addr = [0u8; 6];
        peer_addr.copy_from_slice(&buf.as_slice()[PEER_ADDR_OFFSET..PEER_ADDR_OFFSET + 6]);
        let tx_vector_idx = self
            .minstrel
            .as_ref()
            .and_then(|minstrel| {
                let mut banjo_flags = banjo_wlan_softmac::WlanTxInfoFlags(0);
                if flags.0 & TxFlags::FAVOR_RELIABILITY.0 != 0 {
                    banjo_flags |= banjo_wlan_softmac::WlanTxInfoFlags::FAVOR_RELIABILITY;
                }
                if flags.0 & TxFlags::PROTECTED.0 != 0 {
                    banjo_flags |= banjo_wlan_softmac::WlanTxInfoFlags::PROTECTED;
                }
                if flags.0 & TxFlags::QOS.0 != 0 {
                    banjo_flags |= banjo_wlan_softmac::WlanTxInfoFlags::QOS;
                }
                minstrel.lock().get_tx_vector_idx(frame_control, &peer_addr, banjo_flags)
            })
            .unwrap_or_else(|| {
                // We either don't have minstrel, or minstrel failed to generate a tx vector.
                // Use a reasonable default value instead.
                // Note: This section has no practical effect on ath10k. It is
                // only effective if the underlying device meets both criteria below:
                // 1. Does not support tx status report.
                // 2. Honors our instruction on tx_vector to use.
                // TODO(fxbug.dev/28893): Choose an optimal MCS for management frames
                // TODO(fxbug.dev/43456): Log stats about minstrel usage vs default tx vector.
                let mcs_idx = if frame_control.is_data() { 7 } else { 3 };
                tx_vector::TxVector::new(
                    banjo_common::WlanPhyType::ERP,
                    WlanGi::G_800NS,
                    banjo_common::ChannelBandwidth::CBW20,
                    mcs_idx,
                )
                .unwrap()
                .to_idx()
            });

        let tx_info = wlan_common::tx_vector::TxVector::from_idx(tx_vector_idx)
            .to_banjo_tx_info(flags.0, self.minstrel.is_some());
        self.raw_device.queue_tx(0, buf, tx_info)
    }

    pub fn set_eth_status(&self, status: u32) {
        self.raw_device.set_eth_status(status);
    }

    pub fn set_channel(&self, channel: banjo_common::WlanChannel) -> Result<(), zx::Status> {
        self.raw_device.set_channel(channel)
    }

    pub fn set_key(&self, key: key::KeyConfig) -> Result<(), zx::Status> {
        self.raw_device.set_key(key)
    }

    pub fn start_passive_scan(
        &self,
        passive_scan_args: &banjo_wlan_softmac::WlanSoftmacPassiveScanArgs,
    ) -> Result<u64, zx::Status> {
        self.raw_device.start_passive_scan(&passive_scan_args)
    }

    pub fn start_active_scan(&self, active_scan_args: &ActiveScanArgs) -> Result<u64, zx::Status> {
        self.raw_device.start_active_scan(active_scan_args)
    }

    pub fn channel(&self) -> banjo_common::WlanChannel {
        self.raw_device.channel()
    }

    pub fn wlan_softmac_info(&self) -> WlanSoftmacInfo {
        self.raw_device.wlan_softmac_info()
    }

    pub fn configure_bss(&self, cfg: BssConfig) -> Result<(), zx::Status> {
        self.raw_device.configure_bss(cfg)
    }

    pub fn enable_beaconing(
        &self,
        buf: OutBuf,
        tim_ele_offset: usize,
        beacon_interval: TimeUnit,
    ) -> Result<(), zx::Status> {
        self.raw_device.enable_beaconing(buf, tim_ele_offset, beacon_interval)
    }

    pub fn disable_beaconing(&self) -> Result<(), zx::Status> {
        self.raw_device.disable_beaconing()
    }

    pub fn configure_beacon(&self, buf: OutBuf) -> Result<(), zx::Status> {
        self.raw_device.configure_beacon(buf)
    }

    pub fn set_eth_link(&self, status: LinkStatus) -> Result<(), zx::Status> {
        self.raw_device.set_eth_link(status)
    }

    pub fn set_eth_link_up(&self) -> Result<(), zx::Status> {
        self.raw_device.set_eth_link(LinkStatus::UP)
    }

    pub fn set_eth_link_down(&self) -> Result<(), zx::Status> {
        self.raw_device.set_eth_link(LinkStatus::DOWN)
    }

    pub fn configure_assoc(&self, assoc_ctx: WlanAssocCtx) -> Result<(), zx::Status> {
        if let Some(minstrel) = &self.minstrel {
            minstrel.lock().add_peer(&assoc_ctx);
        }
        self.raw_device.configure_assoc(assoc_ctx)
    }

    pub fn clear_assoc(&self, addr: &MacAddr) -> Result<(), zx::Status> {
        if let Some(minstrel) = &self.minstrel {
            minstrel.lock().remove_peer(addr);
        }
        self.raw_device.clear_assoc(addr)
    }
}

/// Hand-rolled Rust version of the banjo wlan_softmac_ifc_protocol for communication from the driver up.
/// Note that we copy the individual fns out of this struct into the equivalent generated struct
/// in C++. Thanks to cbindgen, this gives us a compile-time confirmation that our function
/// signatures are correct.
#[repr(C)]
pub struct WlanSoftmacIfcProtocol<'a> {
    ops: *const WlanSoftmacIfcProtocolOps,
    ctx: &'a mut crate::DriverEventSink,
}

#[repr(C)]
pub struct WlanSoftmacIfcProtocolOps {
    status: extern "C" fn(ctx: &mut crate::DriverEventSink, status: u32),
    recv: extern "C" fn(ctx: &mut crate::DriverEventSink, packet: *const WlanRxPacket),
    complete_tx: extern "C" fn(
        ctx: &'static mut crate::DriverEventSink,
        packet: *const WlanTxPacket,
        status: i32,
    ),
    report_tx_status:
        extern "C" fn(ctx: &mut crate::DriverEventSink, tx_status: *const WlanTxStatus),
    scan_complete: extern "C" fn(ctx: &mut crate::DriverEventSink, status: i32, scan_id: u64),
}

#[no_mangle]
extern "C" fn handle_status(ctx: &mut crate::DriverEventSink, status: u32) {
    let _ = ctx.0.unbounded_send(crate::DriverEvent::Status { status });
}
#[no_mangle]
extern "C" fn handle_recv(ctx: &mut crate::DriverEventSink, packet: *const WlanRxPacket) {
    // TODO(fxbug.dev/29063): C++ uses a buffer allocator for this, determine if we need one.
    let bytes =
        unsafe { std::slice::from_raw_parts((*packet).mac_frame_buffer, (*packet).mac_frame_size) }
            .into();
    let rx_info = unsafe { (*packet).info };
    let _ = ctx.0.unbounded_send(crate::DriverEvent::MacFrameRx { bytes, rx_info });
}
#[no_mangle]
extern "C" fn handle_complete_tx(
    _ctx: &mut crate::DriverEventSink,
    _packet: *const WlanTxPacket,
    _status: i32,
) {
    // TODO(fxbug.dev/85924): Implement this to support asynchronous packet delivery.
}
#[no_mangle]
extern "C" fn handle_report_tx_status(
    ctx: &mut crate::DriverEventSink,
    tx_status: *const WlanTxStatus,
) {
    if tx_status.is_null() {
        return;
    }
    let tx_status = unsafe { *tx_status };
    let _ = ctx.0.unbounded_send(crate::DriverEvent::TxStatusReport { tx_status });
}
#[no_mangle]
extern "C" fn handle_scan_complete(ctx: &mut crate::DriverEventSink, status: i32, scan_id: u64) {
    let _ = ctx.0.unbounded_send(crate::DriverEvent::ScanComplete {
        status: zx::Status::from_raw(status),
        scan_id,
    });
}

const PROTOCOL_OPS: WlanSoftmacIfcProtocolOps = WlanSoftmacIfcProtocolOps {
    status: handle_status,
    recv: handle_recv,
    complete_tx: handle_complete_tx,
    report_tx_status: handle_report_tx_status,
    scan_complete: handle_scan_complete,
};

impl<'a> WlanSoftmacIfcProtocol<'a> {
    pub(crate) fn new(sink: &'a mut crate::DriverEventSink) -> Self {
        // Const reference has 'static lifetime, so it's safe to pass down to the driver.
        let ops = &PROTOCOL_OPS;
        Self { ops, ctx: sink }
    }
}

pub struct ActiveScanArgs {
    pub min_channel_time: zx::sys::zx_duration_t,
    pub max_channel_time: zx::sys::zx_duration_t,
    pub min_home_time: zx::sys::zx_duration_t,
    pub min_probes_per_channel: u8,
    pub max_probes_per_channel: u8,
    pub ssids_list: Vec<banjo_ieee80211::CSsid>,
    pub mac_header: Vec<u8>,
    pub channels: Vec<u8>,
    pub ies: Vec<u8>,
}

// Private wrapper struct to manage the lifetime of the pointers contained in the
// banjo_fuchsia_hardware_wlan_softmac::WlanSoftmacActiveScanArgs converted
// from an ActiveScanArgs.
struct WlanSoftmacActiveScanArgs<'a, T> {
    args: banjo_wlan_softmac::WlanSoftmacActiveScanArgs,
    phantom: PhantomData<&'a T>,
}

impl<'a> From<&'a ActiveScanArgs> for WlanSoftmacActiveScanArgs<'a, ActiveScanArgs> {
    fn from(active_scan_args: &ActiveScanArgs) -> WlanSoftmacActiveScanArgs<'_, ActiveScanArgs> {
        WlanSoftmacActiveScanArgs {
            args: banjo_wlan_softmac::WlanSoftmacActiveScanArgs {
                min_channel_time: active_scan_args.min_channel_time,
                max_channel_time: active_scan_args.max_channel_time,
                min_home_time: active_scan_args.min_home_time,
                min_probes_per_channel: active_scan_args.min_probes_per_channel,
                max_probes_per_channel: active_scan_args.max_probes_per_channel,
                channels_list: active_scan_args.channels.as_ptr(),
                channels_count: active_scan_args.channels.len(),
                ssids_list: active_scan_args.ssids_list.as_ptr(),
                ssids_count: active_scan_args.ssids_list.len(),
                mac_header_buffer: active_scan_args.mac_header.as_ptr(),
                mac_header_size: active_scan_args.mac_header.len(),
                ies_buffer: active_scan_args.ies.as_ptr(),
                ies_size: active_scan_args.ies.len(),
            },
            phantom: PhantomData,
        }
    }
}

impl From<banjo_wlan_softmac::WlanSoftmacActiveScanArgs> for ActiveScanArgs {
    fn from(banjo_args: banjo_wlan_softmac::WlanSoftmacActiveScanArgs) -> ActiveScanArgs {
        unsafe {
            ActiveScanArgs {
                min_channel_time: banjo_args.min_channel_time,
                max_channel_time: banjo_args.max_channel_time,
                min_home_time: banjo_args.min_home_time,
                min_probes_per_channel: banjo_args.min_probes_per_channel,
                max_probes_per_channel: banjo_args.max_probes_per_channel,
                ssids_list: std::slice::from_raw_parts(
                    banjo_args.ssids_list,
                    banjo_args.ssids_count,
                )
                .to_vec(),
                mac_header: std::slice::from_raw_parts(
                    banjo_args.mac_header_buffer,
                    banjo_args.mac_header_size,
                )
                .to_vec(),
                channels: std::slice::from_raw_parts(
                    banjo_args.channels_list,
                    banjo_args.channels_count,
                )
                .to_vec(),
                ies: std::slice::from_raw_parts(banjo_args.ies_buffer, banjo_args.ies_size)
                    .to_vec(),
            }
        }
    }
}

// Our device is used inside a separate worker thread, so we force Rust to allow this.
unsafe impl Send for DeviceInterface {}

/// A `Device` allows transmitting frames and MLME messages.
#[repr(C)]
pub struct DeviceInterface {
    device: *mut c_void,
    /// Start operations on the underlying device and return the SME channel.
    start: extern "C" fn(
        device: *mut c_void,
        ifc: *const WlanSoftmacIfcProtocol<'_>,
        out_sme_channel: *mut zx::sys::zx_handle_t,
    ) -> i32,
    /// Request to deliver an Ethernet II frame to Fuchsia's Netstack.
    deliver_eth_frame: extern "C" fn(device: *mut c_void, data: *const u8, len: usize) -> i32,
    /// Deliver a WLAN frame directly through the firmware.
    queue_tx: extern "C" fn(
        device: *mut c_void,
        options: u32,
        buf: OutBuf,
        tx_info: banjo_wlan_softmac::WlanTxInfo,
    ) -> i32,
    /// Reports the current status to the ethernet driver.
    set_eth_status: extern "C" fn(device: *mut c_void, status: u32),
    /// Returns the currently set WLAN channel.
    get_wlan_channel: extern "C" fn(device: *mut c_void) -> banjo_common::WlanChannel,
    /// Request the PHY to change its channel. If successful, get_wlan_channel will return the
    /// chosen channel.
    set_wlan_channel: extern "C" fn(device: *mut c_void, channel: banjo_common::WlanChannel) -> i32,
    /// Set a key on the device.
    /// |key| is mutable because the underlying API does not take a const wlan_key_config_t.
    set_key: extern "C" fn(device: *mut c_void, key: *mut banjo_wlan_softmac::WlanKeyConfig) -> i32,
    /// Make passive scan request to the driver
    start_passive_scan: extern "C" fn(
        device: *mut c_void,
        passive_scan_args: *const banjo_wlan_softmac::WlanSoftmacPassiveScanArgs,
        out_scan_id: *mut u64,
    ) -> zx::sys::zx_status_t,
    /// Make active scan request to the driver
    start_active_scan: extern "C" fn(
        device: *mut c_void,
        active_scan_args: *const banjo_wlan_softmac::WlanSoftmacActiveScanArgs,
        out_scan_id: *mut u64,
    ) -> zx::sys::zx_status_t,
    /// Get information and capabilities of this WLAN interface
    get_wlan_softmac_info: extern "C" fn(device: *mut c_void) -> WlanSoftmacInfo,
    /// Configure the device's BSS.
    /// |cfg| is mutable because the underlying API does not take a const bss_config_t.
    configure_bss: extern "C" fn(device: *mut c_void, cfg: *mut BssConfig) -> i32,
    /// Enable hardware offload of beaconing on the device.
    enable_beaconing: extern "C" fn(
        device: *mut c_void,
        buf: OutBuf,
        tim_ele_offset: usize,
        beacon_interval: u16,
    ) -> i32,
    /// Disable beaconing on the device.
    disable_beaconing: extern "C" fn(device: *mut c_void) -> i32,
    /// Reconfigure the enabled beacon on the device.
    configure_beacon: extern "C" fn(device: *mut c_void, buf: OutBuf) -> i32,
    /// Sets the link status to be UP or DOWN.
    set_link_status: extern "C" fn(device: *mut c_void, status: u8) -> i32,
    /// Configure the association context.
    /// |assoc_ctx| is mutable because the underlying API does not take a const wlan_assoc_ctx_t.
    configure_assoc: extern "C" fn(device: *mut c_void, assoc_ctx: *mut WlanAssocCtx) -> i32,
    /// Clear the association context.
    clear_assoc: extern "C" fn(device: *mut c_void, addr: &[u8; 6]) -> i32,
}

impl DeviceInterface {
    pub fn start(&self, ifc: *const WlanSoftmacIfcProtocol<'_>) -> Result<zx::Handle, zx::Status> {
        let mut out_channel = 0;
        let status = (self.start)(self.device, ifc, &mut out_channel as *mut u32);
        // Unsafe block required because we cannot pass a Rust handle over FFI. An invalid
        // handle violates the banjo API, and may be detected by the caller of this fn.
        zx::ok(status).map(|_| unsafe { zx::Handle::from_raw(out_channel) })
    }
    fn deliver_eth_frame(&self, slice: &[u8]) -> Result<(), zx::Status> {
        let status = (self.deliver_eth_frame)(self.device, slice.as_ptr(), slice.len());
        zx::ok(status)
    }

    fn queue_tx(
        &self,
        options: u32,
        buf: OutBuf,
        tx_info: banjo_wlan_softmac::WlanTxInfo,
    ) -> Result<(), zx::Status> {
        let status = (self.queue_tx)(self.device, options, buf, tx_info);
        zx::ok(status)
    }

    fn set_eth_status(&self, status: u32) {
        (self.set_eth_status)(self.device, status);
    }

    fn set_channel(&self, channel: banjo_common::WlanChannel) -> Result<(), zx::Status> {
        let status = (self.set_wlan_channel)(self.device, channel);
        zx::ok(status)
    }

    fn set_key(&self, key: key::KeyConfig) -> Result<(), zx::Status> {
        let mut banjo_key = banjo_wlan_softmac::WlanKeyConfig {
            bssid: key.bssid,
            protection: match key.protection {
                key::Protection::NONE => banjo_wlan_softmac::WlanProtection::NONE,
                key::Protection::RX => banjo_wlan_softmac::WlanProtection::RX,
                key::Protection::TX => banjo_wlan_softmac::WlanProtection::TX,
                key::Protection::RX_TX => banjo_wlan_softmac::WlanProtection::RX_TX,
                _ => return Err(zx::Status::INVALID_ARGS),
            },
            cipher_oui: key.cipher_oui,
            cipher_type: key.cipher_type,
            key_type: match key.key_type {
                key::KeyType::PAIRWISE => WlanKeyType::PAIRWISE,
                key::KeyType::GROUP => WlanKeyType::GROUP,
                key::KeyType::IGTK => WlanKeyType::IGTK,
                key::KeyType::PEER => WlanKeyType::PEER,
                _ => return Err(zx::Status::INVALID_ARGS),
            },
            peer_addr: key.peer_addr,
            key_idx: key.key_idx,
            key_len: key.key_len,
            key: key.key,
            rsc: key.rsc,
        };
        let status = (self.set_key)(self.device, &mut banjo_key);
        zx::ok(status)
    }

    fn start_passive_scan(
        &self,
        passive_scan_args: &banjo_wlan_softmac::WlanSoftmacPassiveScanArgs,
    ) -> Result<u64, zx::Status> {
        let mut out_scan_id = 0;
        let status = (self.start_passive_scan)(
            self.device,
            passive_scan_args as *const banjo_wlan_softmac::WlanSoftmacPassiveScanArgs,
            &mut out_scan_id as *mut u64,
        );
        zx::ok(status).map(|_| out_scan_id)
    }

    fn start_active_scan(&self, active_scan_args: &ActiveScanArgs) -> Result<u64, zx::Status> {
        let mut out_scan_id = 0;
        let status = (self.start_active_scan)(
            self.device,
            &WlanSoftmacActiveScanArgs::<'_>::from(active_scan_args).args
                as *const banjo_wlan_softmac::WlanSoftmacActiveScanArgs,
            &mut out_scan_id as *mut u64,
        );
        zx::ok(status).map(|_| out_scan_id)
    }

    fn channel(&self) -> banjo_common::WlanChannel {
        (self.get_wlan_channel)(self.device)
    }

    pub fn wlan_softmac_info(&self) -> WlanSoftmacInfo {
        (self.get_wlan_softmac_info)(self.device)
    }

    fn configure_bss(&self, mut cfg: BssConfig) -> Result<(), zx::Status> {
        let status = (self.configure_bss)(self.device, &mut cfg as *mut BssConfig);
        zx::ok(status)
    }

    fn enable_beaconing(
        &self,
        buf: OutBuf,
        tim_ele_offset: usize,
        beacon_interval: TimeUnit,
    ) -> Result<(), zx::Status> {
        let status =
            (self.enable_beaconing)(self.device, buf, tim_ele_offset, beacon_interval.into());
        zx::ok(status)
    }

    fn disable_beaconing(&self) -> Result<(), zx::Status> {
        let status = (self.disable_beaconing)(self.device);
        zx::ok(status)
    }

    fn configure_beacon(&self, buf: OutBuf) -> Result<(), zx::Status> {
        let status = (self.configure_beacon)(self.device, buf);
        zx::ok(status)
    }

    fn set_eth_link(&self, status: LinkStatus) -> Result<(), zx::Status> {
        let status = (self.set_link_status)(self.device, status.0);
        zx::ok(status)
    }

    fn configure_assoc(&self, mut assoc_ctx: WlanAssocCtx) -> Result<(), zx::Status> {
        let status = (self.configure_assoc)(self.device, &mut assoc_ctx as *mut WlanAssocCtx);
        zx::ok(status)
    }

    fn clear_assoc(&self, addr: &MacAddr) -> Result<(), zx::Status> {
        let status = (self.clear_assoc)(self.device, addr);
        zx::ok(status)
    }
}

#[cfg(test)]
macro_rules! arr {
    ($slice:expr, $size:expr) => {{
        assert!($slice.len() < $size);
        let mut a = [0; $size];
        a[..$slice.len()].clone_from_slice(&$slice);
        a
    }};
}

#[cfg(test)]
mod test_utils {
    use {
        super::*,
        crate::{
            buffer::{BufferProvider, FakeBufferProvider},
            error::Error,
            test_utils::fake_control_handle,
        },
        banjo_ddk_hw_wlan_ieee80211::*,
        banjo_fuchsia_hardware_wlan_phyinfo::*,
        banjo_fuchsia_wlan_common as banjo_common, fuchsia_async as fasync,
        fuchsia_zircon as zircon,
        std::convert::TryInto,
    };

    pub struct CapturedWlanSoftmacPassiveScanArgs {
        pub channels: Vec<u8>,
        pub min_channel_time: zircon::sys::zx_duration_t,
        pub max_channel_time: zircon::sys::zx_duration_t,
        pub min_home_time: zircon::sys::zx_duration_t,
    }

    impl CapturedWlanSoftmacPassiveScanArgs {
        pub fn from_banjo(
            banjo_args_ptr: *const banjo_wlan_softmac::WlanSoftmacPassiveScanArgs,
        ) -> CapturedWlanSoftmacPassiveScanArgs {
            unsafe {
                let banjo_args = *banjo_args_ptr;
                CapturedWlanSoftmacPassiveScanArgs {
                    channels: std::slice::from_raw_parts(
                        banjo_args.channels_list,
                        banjo_args.channels_count,
                    )
                    .to_vec(),
                    min_channel_time: banjo_args.min_channel_time,
                    max_channel_time: banjo_args.max_channel_time,
                    min_home_time: banjo_args.min_home_time,
                }
            }
        }
    }

    pub struct FakeDevice {
        pub eth_queue: Vec<Vec<u8>>,
        pub wlan_queue: Vec<(Vec<u8>, u32)>,
        pub sme_sap: (fidl_mlme::MlmeControlHandle, zx::Channel),
        pub wlan_channel: banjo_common::WlanChannel,
        pub keys: Vec<banjo_wlan_softmac::WlanKeyConfig>,
        pub next_scan_id: u64,
        pub captured_passive_scan_args: Option<CapturedWlanSoftmacPassiveScanArgs>,
        pub captured_active_scan_args: Option<ActiveScanArgs>,
        pub info: WlanSoftmacInfo,
        pub bss_cfg: Option<BssConfig>,
        pub bcn_cfg: Option<(Vec<u8>, usize, TimeUnit)>,
        pub link_status: LinkStatus,
        pub assocs: std::collections::HashMap<MacAddr, WlanAssocCtx>,
        pub buffer_provider: BufferProvider,
        pub set_key_results: Vec<zx::Status>,
    }

    impl FakeDevice {
        pub fn new(executor: &fasync::TestExecutor) -> Self {
            let sme_sap = fake_control_handle(&executor);
            Self {
                eth_queue: vec![],
                wlan_queue: vec![],
                sme_sap,
                wlan_channel: banjo_common::WlanChannel {
                    primary: 0,
                    cbw: banjo_common::ChannelBandwidth::CBW20,
                    secondary80: 0,
                },
                next_scan_id: 0,
                captured_passive_scan_args: None,
                captured_active_scan_args: None,
                info: fake_wlan_softmac_info(),
                keys: vec![],
                bss_cfg: None,
                bcn_cfg: None,
                link_status: LinkStatus::DOWN,
                assocs: std::collections::HashMap::new(),
                buffer_provider: FakeBufferProvider::new(),
                set_key_results: vec![],
            }
        }

        pub extern "C" fn start(
            _device: *mut c_void,
            _ifc: *const WlanSoftmacIfcProtocol<'_>,
            _out_sme_channel: *mut zx::sys::zx_handle_t,
        ) -> i32 {
            // TODO(fxbug.dev/45464): Implement when AP tests are ported to Rust.
            zx::sys::ZX_OK
        }

        pub extern "C" fn deliver_eth_frame(
            device: *mut c_void,
            data: *const u8,
            len: usize,
        ) -> i32 {
            assert!(!device.is_null());
            assert!(!data.is_null());
            assert_ne!(len, 0);
            // safe here because slice will not outlive data
            let slice = unsafe { std::slice::from_raw_parts(data, len) };
            // safe here because device_ptr alwyas points to self
            unsafe {
                (*(device as *mut Self)).eth_queue.push(slice.to_vec());
            }
            zx::sys::ZX_OK
        }

        pub extern "C" fn queue_tx(
            device: *mut c_void,
            _options: u32,
            buf: OutBuf,
            _tx_info: banjo_wlan_softmac::WlanTxInfo,
        ) -> i32 {
            assert!(!device.is_null());
            unsafe {
                (*(device as *mut Self)).wlan_queue.push((buf.as_slice().to_vec(), 0));
            }
            buf.free();
            zx::sys::ZX_OK
        }

        pub extern "C" fn set_eth_status(_device: *mut c_void, _status: u32) {}

        pub extern "C" fn set_link_status(device: *mut c_void, status: u8) -> i32 {
            assert!(!device.is_null());
            // safe here because device_ptr always points to Self
            unsafe {
                (*(device as *mut Self)).link_status = LinkStatus(status);
            }
            zx::sys::ZX_OK
        }

        pub extern "C" fn queue_tx_with_failure(
            _: *mut c_void,
            _: u32,
            buf: OutBuf,
            _: banjo_wlan_softmac::WlanTxInfo,
        ) -> i32 {
            buf.free();
            zx::sys::ZX_ERR_IO
        }

        pub extern "C" fn get_wlan_channel(device: *mut c_void) -> banjo_common::WlanChannel {
            unsafe { (*(device as *const Self)).wlan_channel }
        }

        pub extern "C" fn set_wlan_channel(
            device: *mut c_void,
            wlan_channel: banjo_common::WlanChannel,
        ) -> i32 {
            unsafe {
                (*(device as *mut Self)).wlan_channel = wlan_channel;
            }
            zx::sys::ZX_OK
        }

        pub extern "C" fn set_key(
            device: *mut c_void,
            key: *mut banjo_wlan_softmac::WlanKeyConfig,
        ) -> i32 {
            let device = unsafe { &mut *(device as *mut Self) };
            device.keys.push(unsafe { (*key).clone() });
            if device.set_key_results.is_empty() {
                zx::sys::ZX_OK
            } else {
                device.set_key_results.remove(0).into_raw()
            }
        }

        pub extern "C" fn start_passive_scan(
            device: *mut c_void,
            passive_scan_args: *const banjo_wlan_softmac::WlanSoftmacPassiveScanArgs,
            out_scan_id: *mut u64,
        ) -> zx::sys::zx_status_t {
            unsafe {
                let self_ = &mut *(device as *mut Self);
                *out_scan_id = self_.next_scan_id;
                self_.next_scan_id += 1;
                self_.captured_passive_scan_args =
                    Some(CapturedWlanSoftmacPassiveScanArgs::from_banjo(passive_scan_args));
            }
            zx::sys::ZX_OK
        }

        pub extern "C" fn start_active_scan(
            device: *mut c_void,
            active_scan_args: *const banjo_wlan_softmac::WlanSoftmacActiveScanArgs,
            out_scan_id: *mut u64,
        ) -> zx::sys::zx_status_t {
            unsafe {
                let self_ = &mut *(device as *mut Self);
                *out_scan_id = self_.next_scan_id;
                self_.next_scan_id += 1;
                self_.captured_active_scan_args = Some(ActiveScanArgs::from(*active_scan_args));
            }
            zx::sys::ZX_OK
        }

        pub extern "C" fn start_passive_scan_fails(
            _device: *mut c_void,
            _passive_scan_args: *const banjo_wlan_softmac::WlanSoftmacPassiveScanArgs,
            _out_scan_id: *mut u64,
        ) -> zx::sys::zx_status_t {
            zx::sys::ZX_ERR_NOT_SUPPORTED
        }

        pub extern "C" fn start_active_scan_fails(
            _device: *mut c_void,
            _active_scan_args: *const banjo_wlan_softmac::WlanSoftmacActiveScanArgs,
            _out_scan_id: *mut u64,
        ) -> zx::sys::zx_status_t {
            zx::sys::ZX_ERR_NOT_SUPPORTED
        }

        pub extern "C" fn get_wlan_softmac_info(device: *mut c_void) -> WlanSoftmacInfo {
            unsafe { (*(device as *const Self)).info }
        }

        pub extern "C" fn configure_bss(device: *mut c_void, cfg: *mut BssConfig) -> i32 {
            unsafe {
                (*(device as *mut Self)).bss_cfg.replace((*cfg).clone());
            }
            zx::sys::ZX_OK
        }

        pub extern "C" fn enable_beaconing(
            device: *mut c_void,
            buf: OutBuf,
            tim_ele_offset: usize,
            beacon_interval: u16,
        ) -> i32 {
            unsafe {
                (*(device as *mut Self)).bcn_cfg =
                    Some((buf.as_slice().to_vec(), tim_ele_offset, TimeUnit(beacon_interval)));
                buf.free();
            }
            zx::sys::ZX_OK
        }

        pub extern "C" fn disable_beaconing(device: *mut c_void) -> i32 {
            unsafe {
                (*(device as *mut Self)).bcn_cfg = None;
            }
            zx::sys::ZX_OK
        }

        pub extern "C" fn configure_beacon(device: *mut c_void, buf: OutBuf) -> i32 {
            unsafe {
                if let Some((_, tim_ele_offset, beacon_interval)) = (*(device as *mut Self)).bcn_cfg
                {
                    (*(device as *mut Self)).bcn_cfg =
                        Some((buf.as_slice().to_vec(), tim_ele_offset, beacon_interval));
                    buf.free();
                    zx::sys::ZX_OK
                } else {
                    zx::sys::ZX_ERR_BAD_STATE
                }
            }
        }

        pub extern "C" fn configure_assoc(device: *mut c_void, cfg: *mut WlanAssocCtx) -> i32 {
            unsafe {
                (*(device as *mut Self)).assocs.insert((*cfg).bssid, (*cfg).clone());
            }
            zx::sys::ZX_OK
        }

        pub extern "C" fn clear_assoc(device: *mut c_void, addr: &MacAddr) -> i32 {
            unsafe {
                (*(device as *mut Self)).assocs.remove(addr);
                (*(device as *mut Self)).bss_cfg = None;
            }
            zx::sys::ZX_OK
        }

        pub fn next_mlme_msg<T: fidl::encoding::Decodable>(&mut self) -> Result<T, Error> {
            use fidl::encoding::{decode_transaction_header, Decodable, Decoder};

            let mut buf = zx::MessageBuf::new();
            let () =
                self.sme_sap.1.read(&mut buf).map_err(|status| {
                    Error::Status(format!("error reading MLME message"), status)
                })?;

            let (header, tail): (_, &[u8]) = decode_transaction_header(buf.bytes())?;
            let mut msg = Decodable::new_empty();
            Decoder::decode_into(&header, tail, &mut [], &mut msg)
                .expect("error decoding MLME message");
            Ok(msg)
        }

        pub fn reset(&mut self) {
            self.eth_queue.clear();
        }

        pub fn as_device(&mut self) -> Device {
            let raw_device = DeviceInterface {
                device: self as *mut Self as *mut c_void,
                start: Self::start,
                deliver_eth_frame: Self::deliver_eth_frame,
                queue_tx: Self::queue_tx,
                set_eth_status: Self::set_eth_status,
                get_wlan_channel: Self::get_wlan_channel,
                set_wlan_channel: Self::set_wlan_channel,
                set_key: Self::set_key,
                start_passive_scan: Self::start_passive_scan,
                start_active_scan: Self::start_active_scan,
                get_wlan_softmac_info: Self::get_wlan_softmac_info,
                configure_bss: Self::configure_bss,
                enable_beaconing: Self::enable_beaconing,
                disable_beaconing: Self::disable_beaconing,
                configure_beacon: Self::configure_beacon,
                set_link_status: Self::set_link_status,
                configure_assoc: Self::configure_assoc,
                clear_assoc: Self::clear_assoc,
            };
            Device { raw_device, minstrel: None, control_handle: self.sme_sap.0.clone() }
        }

        pub fn as_device_fail_wlan_tx(&mut self) -> Device {
            let mut dev = self.as_device();
            dev.raw_device.queue_tx = Self::queue_tx_with_failure;
            dev
        }

        pub fn as_device_fail_start_passive_scan(&mut self) -> Device {
            let mut dev = self.as_device();
            dev.raw_device.start_passive_scan = Self::start_passive_scan_fails;
            dev
        }

        pub fn as_device_fail_start_active_scan(&mut self) -> Device {
            let mut dev = self.as_device();
            dev.raw_device.start_active_scan = Self::start_active_scan_fails;
            dev
        }
    }

    pub fn fake_wlan_softmac_info() -> WlanSoftmacInfo {
        let bands_count = 2;
        let mut bands = [default_band_info(); WLAN_INFO_MAX_BANDS as usize];
        bands[0] = WlanInfoBandInfo {
            band: WlanInfoBand::TWO_GHZ,
            rates: arr!(
                [0x0C, 0x12, 0x18, 0x24, 0x30, 0x48, 0x60, 0x6C],
                WLAN_INFO_BAND_INFO_MAX_RATES as usize
            ),
            supported_channels: WlanInfoChannelList {
                base_freq: 2407,
                channels: arr!(
                    [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14],
                    WLAN_INFO_CHANNEL_LIST_MAX_CHANNELS as usize
                ),
            },
            ht_supported: true,
            ht_caps: ht_cap(),
            vht_supported: false,
            vht_caps: Ieee80211VhtCapabilities {
                vht_capability_info: 0,
                supported_vht_mcs_and_nss_set: 0,
            },
        };
        bands[1] = WlanInfoBandInfo {
            band: WlanInfoBand::FIVE_GHZ,
            rates: arr!([0x7E, 0x7F], WLAN_INFO_BAND_INFO_MAX_RATES as usize),
            supported_channels: WlanInfoChannelList {
                base_freq: 5000,
                channels: arr!(
                    [36, 40, 44, 48, 149, 153, 157, 161],
                    WLAN_INFO_CHANNEL_LIST_MAX_CHANNELS as usize
                ),
            },
            ht_supported: true,
            ht_caps: ht_cap(),
            vht_supported: false,
            vht_caps: Ieee80211VhtCapabilities {
                vht_capability_info: 0x0f805032,
                supported_vht_mcs_and_nss_set: 0x0000fffe0000fffe,
            },
        };

        let supported_phys_list = [
            banjo_common::WlanPhyType::DSSS,
            banjo_common::WlanPhyType::HR,
            banjo_common::WlanPhyType::OFDM,
            banjo_common::WlanPhyType::ERP,
            banjo_common::WlanPhyType::HT,
            banjo_common::WlanPhyType::VHT,
        ];
        // Convert to u8 for use in WlanSoftmacInfo.supported_phys_count.
        let supported_phys_count: u8 = supported_phys_list.len().try_into().unwrap();

        // The Banjo transport requires this array to have a
        // size of banjo_common::MAX_SUPPORTED_PHY_TYPES.
        let supported_phys_list = {
            let mut initialized_supported_phys_list =
                [banjo_common::WlanPhyType(0); banjo_common::MAX_SUPPORTED_PHY_TYPES as usize];
            initialized_supported_phys_list[..supported_phys_count as usize]
                .copy_from_slice(&supported_phys_list);
            initialized_supported_phys_list
        };

        WlanSoftmacInfo {
            sta_addr: [7u8; 6],
            mac_role: banjo_common::WlanMacRole::CLIENT,
            supported_phys_list,
            supported_phys_count,
            driver_features: WlanInfoDriverFeature(0),
            caps: WlanInfoHardwareCapability(0),
            bands,
            bands_count,
        }
    }

    fn ht_cap() -> Ieee80211HtCapabilities {
        Ieee80211HtCapabilities {
            ht_capability_info: 0x0063,
            ampdu_params: 0x17,
            supported_mcs_set: Ieee80211HtCapabilitiesSupportedMcsSet {
                bytes: [
                    // Rx MCS bitmask, Supported MCS values: 0-7
                    0xff, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                    // Tx parameters
                    0x01, 0x00, 0x00, 0x00,
                ],
            },
            ht_ext_capabilities: 0,
            tx_beamforming_capabilities: 0,
            asel_capabilities: 0,
        }
    }

    /// Placeholder for default initialization of WlanInfoBandInfo, used for case where we don't
    /// care about the exact information, but the type demands it.
    pub const fn default_band_info() -> WlanInfoBandInfo {
        WlanInfoBandInfo {
            band: WlanInfoBand(0),
            ht_supported: false,
            ht_caps: Ieee80211HtCapabilities {
                ht_capability_info: 0,
                ampdu_params: 0,
                supported_mcs_set: Ieee80211HtCapabilitiesSupportedMcsSet { bytes: [0; 16] },
                ht_ext_capabilities: 0,
                tx_beamforming_capabilities: 0,
                asel_capabilities: 0,
            },
            vht_supported: false,
            vht_caps: Ieee80211VhtCapabilities {
                vht_capability_info: 0,
                supported_vht_mcs_and_nss_set: 0,
            },
            rates: [0; WLAN_INFO_BAND_INFO_MAX_RATES as usize],
            supported_channels: WlanInfoChannelList {
                base_freq: 0,
                channels: [0; WLAN_INFO_CHANNEL_LIST_MAX_CHANNELS as usize],
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::ddk_converter::{self, cssid_from_ssid_unchecked},
        banjo_ddk_hw_wlan_ieee80211::*,
        banjo_fuchsia_hardware_wlan_phyinfo::*,
        banjo_fuchsia_wlan_internal as banjo_internal, fidl_fuchsia_wlan_common as fidl_common,
        fidl_fuchsia_wlan_ieee80211 as fidl_ieee80211, fuchsia_async as fasync,
        ieee80211::Bssid,
        ieee80211::Ssid,
        std::convert::TryFrom,
        wlan_common::assert_variant,
    };

    fn make_auth_confirm_msg() -> fidl_mlme::AuthenticateConfirm {
        fidl_mlme::AuthenticateConfirm {
            peer_sta_address: [1; 6],
            auth_type: fidl_mlme::AuthenticationTypes::SharedKey,
            result_code: fidl_ieee80211::StatusCode::RejectedSequenceTimeout,
        }
    }

    #[test]
    fn send_mlme_message() {
        let exec = fasync::TestExecutor::new().expect("failed to create an executor");
        let mut fake_device = FakeDevice::new(&exec);
        let dev = fake_device.as_device();
        dev.mlme_control_handle()
            .send_authenticate_conf(&mut make_auth_confirm_msg())
            .expect("error sending MLME message");

        // Read message from channel.
        let msg = fake_device
            .next_mlme_msg::<fidl_mlme::AuthenticateConfirm>()
            .expect("error reading message from channel");
        assert_eq!(msg, make_auth_confirm_msg());
    }

    #[test]
    fn send_mlme_message_peer_already_closed() {
        let exec = fasync::TestExecutor::new().expect("failed to create an executor");
        let mut fake_device = FakeDevice::new(&exec);
        let dev = fake_device.as_device();

        drop(fake_device.sme_sap.1);

        let result = dev.mlme_control_handle().send_authenticate_conf(&mut make_auth_confirm_msg());
        assert!(result.unwrap_err().is_closed());
    }

    #[test]
    fn fake_device_deliver_eth_frame() {
        let exec = fasync::TestExecutor::new().expect("failed to create an executor");
        let mut fake_device = FakeDevice::new(&exec);
        let dev = fake_device.as_device();
        assert_eq!(fake_device.eth_queue.len(), 0);
        let first_frame = [5; 32];
        let second_frame = [6; 32];
        assert_eq!(dev.deliver_eth_frame(&first_frame[..]), Ok(()));
        assert_eq!(dev.deliver_eth_frame(&second_frame[..]), Ok(()));
        assert_eq!(fake_device.eth_queue.len(), 2);
        assert_eq!(&fake_device.eth_queue[0], &first_frame);
        assert_eq!(&fake_device.eth_queue[1], &second_frame);
    }

    #[test]
    fn get_set_channel() {
        let exec = fasync::TestExecutor::new().expect("failed to create an executor");
        let mut fake_device = FakeDevice::new(&exec);
        let dev = fake_device.as_device();
        dev.set_channel(banjo_common::WlanChannel {
            primary: 2,
            cbw: banjo_common::ChannelBandwidth::CBW80P80,
            secondary80: 4,
        })
        .expect("set_channel failed?");
        // Check the internal state.
        assert_eq!(
            fake_device.wlan_channel,
            banjo_common::WlanChannel {
                primary: 2,
                cbw: banjo_common::ChannelBandwidth::CBW80P80,
                secondary80: 4
            }
        );
        // Check the external view of the internal state.
        assert_eq!(
            dev.channel(),
            banjo_common::WlanChannel {
                primary: 2,
                cbw: banjo_common::ChannelBandwidth::CBW80P80,
                secondary80: 4
            }
        );
    }

    #[test]
    fn set_key() {
        let exec = fasync::TestExecutor::new().expect("failed to create an executor");
        let mut fake_device = FakeDevice::new(&exec);
        let dev = fake_device.as_device();
        dev.set_key(key::KeyConfig {
            bssid: 1,
            protection: key::Protection::NONE,
            cipher_oui: [3, 4, 5],
            cipher_type: 6,
            key_type: key::KeyType::PAIRWISE,
            peer_addr: [8; 6],
            key_idx: 9,
            key_len: 10,
            key: [11; 32],
            rsc: 12,
        })
        .expect("error setting key");
        assert_eq!(fake_device.keys.len(), 1);
    }

    #[test]
    fn start_passive_scan() {
        let exec = fasync::TestExecutor::new().expect("failed to create an executor");
        let mut fake_device = FakeDevice::new(&exec);
        let dev = fake_device.as_device();

        let result = dev.start_passive_scan(&banjo_wlan_softmac::WlanSoftmacPassiveScanArgs {
            channels_list: &[1u8, 2, 3] as *const u8,
            channels_count: 3,
            min_channel_time: zx::Duration::from_millis(0).into_nanos(),
            max_channel_time: zx::Duration::from_millis(200).into_nanos(),
            min_home_time: 0,
        });
        assert!(result.is_ok());
        assert_variant!(fake_device.captured_passive_scan_args, Some(passive_scan_args) => {
            assert_eq!(passive_scan_args.channels.len(), 3);
            assert_eq!(passive_scan_args.channels, vec![1, 2, 3]);
            assert_eq!(passive_scan_args.min_channel_time, 0);
            assert_eq!(passive_scan_args.max_channel_time, 200_000_000);
            assert_eq!(passive_scan_args.min_home_time, 0);
        }, "No passive scan argument available.");
    }

    #[test]
    fn start_active_scan() {
        let exec = fasync::TestExecutor::new().expect("failed to create an executor");
        let mut fake_device = FakeDevice::new(&exec);
        let dev = fake_device.as_device();

        let result = dev.start_active_scan(&ActiveScanArgs {
            min_channel_time: zx::Duration::from_millis(0).into_nanos(),
            max_channel_time: zx::Duration::from_millis(200).into_nanos(),
            min_home_time: 0,
            min_probes_per_channel: 1,
            max_probes_per_channel: 3,
            channels: vec![1u8, 2, 3],
            ssids_list: vec![
                cssid_from_ssid_unchecked(&Ssid::try_from("foo").unwrap().into()),
                cssid_from_ssid_unchecked(&Ssid::try_from("bar").unwrap().into()),
            ],
            #[rustfmt::skip]
                mac_header: vec![
                    0x40u8, 0x00, // Frame Control
                    0x00, 0x00, // Duration
                    0xff, 0xff, 0xff, 0xff, 0xff, 0xff, // Address 1
                    0x66, 0x66, 0x66, 0x66, 0x66, 0x66, // Address 2
                    0xff, 0xff, 0xff, 0xff, 0xff, 0xff, // Address 3
                    0x70, 0xdc, // Sequence Control
                ],
            #[rustfmt::skip]
                ies: vec![
                    0x01u8, // Element ID for Supported Rates
                    0x08, // Length
                    0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08 // Supported Rates
                ],
        });
        assert!(result.is_ok());
        assert_variant!(fake_device.captured_active_scan_args, Some(active_scan_args) => {
            assert_eq!(active_scan_args.min_channel_time, 0);
            assert_eq!(active_scan_args.max_channel_time, 200_000_000);
            assert_eq!(active_scan_args.min_home_time, 0);
            assert_eq!(active_scan_args.min_probes_per_channel, 1);
            assert_eq!(active_scan_args.max_probes_per_channel, 3);
            assert_eq!(active_scan_args.channels.len(), 3);
            assert_eq!(active_scan_args.channels, vec![1, 2, 3]);
            assert_eq!(active_scan_args.ssids_list.len(), 2);
            assert_eq!(active_scan_args.ssids_list,
                       vec![
                           cssid_from_ssid_unchecked(&Ssid::try_from("foo").unwrap().into()),
                           cssid_from_ssid_unchecked(&Ssid::try_from("bar").unwrap().into()),
                       ]);
            assert_eq!(active_scan_args.mac_header.len(), 24);
            assert_eq!(active_scan_args.mac_header, vec![
                0x40, 0x00, // Frame Control
                0x00, 0x00, // Duration
                0xff, 0xff, 0xff, 0xff, 0xff, 0xff, // Address 1
                0x66, 0x66, 0x66, 0x66, 0x66, 0x66, // Address 2
                0xff, 0xff, 0xff, 0xff, 0xff, 0xff, // Address 3
                0x70, 0xdc, // Sequence Control
            ]);
            assert_eq!(active_scan_args.ies.len(), 10);
            assert_eq!(active_scan_args.ies, vec![
                0x01u8, // Element ID for Supported Rates
                0x08, // Length
                0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08 // Supported Rates
            ][..]);
        }, "No active scan argument available.");
    }

    #[test]
    fn get_wlan_softmac_info() {
        let exec = fasync::TestExecutor::new().expect("failed to create an executor");
        let mut fake_device = FakeDevice::new(&exec);
        let dev = fake_device.as_device();
        let info = dev.wlan_softmac_info();
        assert_eq!(info.sta_addr, [7u8; 6]);
        assert_eq!(info.mac_role, banjo_common::WlanMacRole::CLIENT);
        assert_eq!(info.driver_features, WlanInfoDriverFeature(0));
        assert_eq!(info.caps, WlanInfoHardwareCapability(0));
        assert_eq!(info.bands_count, 2);
    }

    #[test]
    fn configure_bss() {
        let exec = fasync::TestExecutor::new().expect("failed to create an executor");
        let mut fake_device = FakeDevice::new(&exec);
        let dev = fake_device.as_device();
        dev.configure_bss(BssConfig {
            bssid: [1, 2, 3, 4, 5, 6],
            bss_type: banjo_internal::BssType::PERSONAL,
            remote: true,
        })
        .expect("error configuring bss");
        assert!(fake_device.bss_cfg.is_some());
    }

    #[test]
    fn enable_disable_beaconing() {
        let exec = fasync::TestExecutor::new().expect("failed to create an executor");
        let mut fake_device = FakeDevice::new(&exec);
        let dev = fake_device.as_device();

        let mut in_buf = fake_device.buffer_provider.get_buffer(4).expect("error getting buffer");
        in_buf.as_mut_slice().copy_from_slice(&[1, 2, 3, 4][..]);

        dev.enable_beaconing(OutBuf::from(in_buf, 4), 1, TimeUnit(2))
            .expect("error enabling beaconing");
        assert_variant!(
        fake_device.bcn_cfg.as_ref(),
        Some((buf, tim_ele_offset, beacon_interval)) => {
            assert_eq!(&buf[..], &[1, 2, 3, 4][..]);
            assert_eq!(*tim_ele_offset, 1);
            assert_eq!(*beacon_interval, TimeUnit(2));
        });
        dev.disable_beaconing().expect("error disabling beaconing");
        assert_variant!(fake_device.bcn_cfg.as_ref(), None);
    }

    #[test]
    fn configure_beacon() {
        let exec = fasync::TestExecutor::new().expect("failed to create an executor");
        let mut fake_device = FakeDevice::new(&exec);
        let dev = fake_device.as_device();

        {
            let mut in_buf =
                fake_device.buffer_provider.get_buffer(4).expect("error getting buffer");
            in_buf.as_mut_slice().copy_from_slice(&[1, 2, 3, 4][..]);
            dev.enable_beaconing(OutBuf::from(in_buf, 4), 1, TimeUnit(2))
                .expect("error enabling beaconing");
            assert_variant!(fake_device.bcn_cfg.as_ref(), Some((buf, _, _)) => {
                assert_eq!(&buf[..], &[1, 2, 3, 4][..]);
            });
        }

        {
            let mut in_buf =
                fake_device.buffer_provider.get_buffer(4).expect("error getting buffer");
            in_buf.as_mut_slice().copy_from_slice(&[1, 2, 3, 5][..]);
            dev.configure_beacon(OutBuf::from(in_buf, 4)).expect("error enabling beaconing");
            assert_variant!(fake_device.bcn_cfg.as_ref(), Some((buf, _, _)) => {
                assert_eq!(&buf[..], &[1, 2, 3, 5][..]);
            });
        }
    }

    #[test]
    fn set_link_status() {
        let exec = fasync::TestExecutor::new().expect("failed to create an executor");
        let mut fake_device = FakeDevice::new(&exec);
        let dev = fake_device.as_device();

        dev.set_eth_link_up().expect("failed setting status");
        assert_eq!(fake_device.link_status, LinkStatus::UP);

        dev.set_eth_link_down().expect("failed setting status");
        assert_eq!(fake_device.link_status, LinkStatus::DOWN);
    }

    #[test]
    fn configure_assoc() {
        let exec = fasync::TestExecutor::new().expect("failed to create an executor");
        let mut fake_device = FakeDevice::new(&exec);
        let dev = fake_device.as_device();
        dev.configure_assoc(WlanAssocCtx {
            bssid: [1, 2, 3, 4, 5, 6],
            aid: 1,
            listen_interval: 2,
            channel: banjo_common::WlanChannel {
                primary: 3,
                cbw: banjo_common::ChannelBandwidth::CBW20,
                secondary80: 0,
            },
            qos: false,
            wmm_params: ddk_converter::blank_wmm_params(),

            rates_cnt: 4,
            rates: [0; WLAN_MAC_MAX_RATES as usize],
            capability_info: 0x0102,

            has_ht_cap: false,
            // Safe: This is not read by the driver.
            ht_cap: unsafe { std::mem::zeroed::<Ieee80211HtCapabilities>() },
            has_ht_op: false,
            // Safe: This is not read by the driver.
            ht_op: unsafe { std::mem::zeroed::<WlanHtOp>() },

            has_vht_cap: false,
            // Safe: This is not read by the driver.
            vht_cap: unsafe { std::mem::zeroed::<Ieee80211VhtCapabilities>() },
            has_vht_op: false,
            // Safe: This is not read by the driver.
            vht_op: unsafe { std::mem::zeroed::<WlanVhtOp>() },
        })
        .expect("error configuring assoc");
        assert!(fake_device.assocs.contains_key(&[1, 2, 3, 4, 5, 6]));
    }
    #[test]
    fn clear_assoc() {
        let exec = fasync::TestExecutor::new().expect("failed to create an executor");
        let mut fake_device = FakeDevice::new(&exec);
        let dev = fake_device.as_device();
        dev.configure_bss(BssConfig {
            bssid: [1, 2, 3, 4, 5, 6],
            bss_type: banjo_internal::BssType::PERSONAL,
            remote: true,
        })
        .expect("error configuring bss");
        let assoc_ctx = ddk_converter::build_ddk_assoc_ctx(
            Bssid([1, 2, 3, 4, 5, 6]),
            1,
            fidl_mlme::NegotiatedCapabilities {
                channel: fidl_common::WlanChannel {
                    primary: 149,
                    cbw: fidl_common::ChannelBandwidth::Cbw40,
                    secondary80: 42,
                },
                capability_info: 0,
                rates: vec![],
                wmm_param: None,
                ht_cap: None,
                vht_cap: None,
            },
            None,
            None,
        );
        assert!(fake_device.bss_cfg.is_some());
        dev.configure_assoc(assoc_ctx).expect("error configuring assoc");
        assert_eq!(fake_device.assocs.len(), 1);
        dev.clear_assoc(&[1, 2, 3, 4, 5, 6]).expect("error clearing assoc");
        assert_eq!(fake_device.assocs.len(), 0);
        assert!(fake_device.bss_cfg.is_none());
    }
}
