// Copyright 2021 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use {
    crate::{
        buffer::OutBuf,
        client::{
            channel_listener::{ChannelListener, ChannelListenerSource},
            channel_scheduler::ChannelScheduler,
            convert_beacon::construct_bss_description,
            Context, TimedEvent,
        },
        ddk_converter::cssid_from_ssid_unchecked,
        device::{ActiveScanArgs, Device, TxFlags},
        error::Error,
    },
    anyhow::format_err,
    banjo_fuchsia_hardware_wlan_phyinfo as banjo_wlan_phyinfo,
    banjo_fuchsia_hardware_wlan_softmac as banjo_wlan_softmac,
    banjo_fuchsia_wlan_common as banjo_common, banjo_fuchsia_wlan_ieee80211 as banjo_ieee80211,
    fidl_fuchsia_wlan_internal as fidl_internal, fidl_fuchsia_wlan_mlme as fidl_mlme,
    fuchsia_zircon as zx,
    ieee80211::{Bssid, MacAddr},
    log::{error, warn},
    thiserror::Error,
    wlan_common::{
        mac::{self, CapabilityInfo},
        mgmt_writer,
        time::TimeUnit,
        timer::EventId,
    },
    wlan_frame_writer::{write_frame, write_frame_with_dynamic_buf},
};

// TODO(fxbug.dev/89992): Currently hardcoded until parameters supported.
const MIN_HOME_TIME: zx::Duration = zx::Duration::from_millis(0);
const MIN_PROBES_PER_CHANNEL: u8 = 0;
const MAX_PROBES_PER_CHANNEL: u8 = 0;

#[derive(Error, Debug, PartialEq, Eq)]
pub enum ScanError {
    #[error("scanner is busy")]
    Busy,
    #[error("invalid arg: empty channel list")]
    EmptyChannelList,
    #[error("invalid arg: max_channel_time < min_channel_time")]
    MaxChannelTimeLtMin,
    #[error("fail starting device scan: {}", _0)]
    StartOffloadScanFails(zx::Status),
}

impl From<ScanError> for zx::Status {
    fn from(e: ScanError) -> Self {
        match e {
            ScanError::Busy => zx::Status::UNAVAILABLE,
            ScanError::EmptyChannelList | ScanError::MaxChannelTimeLtMin => {
                zx::Status::INVALID_ARGS
            }
            ScanError::StartOffloadScanFails(status) => status,
        }
    }
}

impl From<ScanError> for fidl_mlme::ScanResultCode {
    fn from(e: ScanError) -> Self {
        match e {
            ScanError::Busy => fidl_mlme::ScanResultCode::NotSupported,
            ScanError::EmptyChannelList | ScanError::MaxChannelTimeLtMin => {
                fidl_mlme::ScanResultCode::InvalidArgs
            }
            ScanError::StartOffloadScanFails(zx::Status::NOT_SUPPORTED) => {
                fidl_mlme::ScanResultCode::NotSupported
            }
            ScanError::StartOffloadScanFails(..) => fidl_mlme::ScanResultCode::InternalError,
        }
    }
}

pub struct Scanner {
    ongoing_scan: Option<OngoingScan>,
    /// MAC address of current client interface
    iface_mac: MacAddr,
}

impl Scanner {
    pub fn new(iface_mac: MacAddr) -> Self {
        Self { ongoing_scan: None, iface_mac }
    }

    pub fn bind<'a>(&'a mut self, ctx: &'a mut Context) -> BoundScanner<'a> {
        BoundScanner { scanner: self, ctx }
    }

    pub fn is_scanning(&self) -> bool {
        self.ongoing_scan.is_some()
    }

    #[cfg(test)]
    pub fn probe_delay_timeout_id(&self) -> Option<EventId> {
        match self.ongoing_scan {
            Some(OngoingScan::MlmeScan { probe_delay_timeout_id: id, .. }) => id,
            _ => None,
        }
    }
}

pub struct BoundScanner<'a> {
    scanner: &'a mut Scanner,
    ctx: &'a mut Context,
}

enum OngoingScan {
    MlmeScan {
        /// Scan request that's currently being serviced.
        req: fidl_mlme::ScanRequest,
        /// ID of timeout event scheduled for active scan at beginning of each channel switch. At
        /// end of timeout, a probe request is sent.
        probe_delay_timeout_id: Option<EventId>,
    },
    PassiveOffloadScan {
        /// Scan txn_id that's currently being serviced.
        mlme_txn_id: u64,
        /// Unique identifier returned from the device driver when the scan began.
        in_progress_device_scan_id: u64,
    },
    ActiveOffloadScan {
        /// Scan txn_id that's currently being serviced.
        mlme_txn_id: u64,
        /// Unique identifier returned from the device driver when the scan began.
        in_progress_device_scan_id: u64,
        /// Remaining arguments to be sent to future scan requests.
        remaining_active_scan_args: Vec<ActiveScanArgs>,
    },
}

struct ChannelList {
    band: banjo_wlan_phyinfo::WlanInfoBand,
    channels: Vec<u8>,
}

impl<'a> BoundScanner<'a> {
    /// Handle scan request. Queue requested scan channels in channel scheduler.
    ///
    /// If a scan request is in progress, or the new request has invalid argument (empty channel
    /// list or larger min channel time than max), then the request is rejected.
    pub fn on_sme_scan<F, CL>(
        &'a mut self,
        req: fidl_mlme::ScanRequest,
        build_channel_listener: F,
        chan_sched: &mut ChannelScheduler,
    ) -> Result<(), Error>
    where
        F: FnOnce(&'a mut Context, &'a mut Scanner) -> CL,
        CL: ChannelListener,
    {
        if self.scanner.ongoing_scan.is_some() {
            return Err(Error::ScanError(ScanError::Busy));
        }
        if req.channel_list.is_empty() {
            return Err(Error::ScanError(ScanError::EmptyChannelList));
        }
        if req.max_channel_time < req.min_channel_time {
            return Err(Error::ScanError(ScanError::MaxChannelTimeLtMin));
        }

        let wlan_softmac_info = self.ctx.device.wlan_softmac_info();

        // The else of this branch is an "MLME scan" which is implemented by calling SetChannel
        // multiple times to visit each channel. It's only used in hw-sim tests and is not supported
        // by any SoftMAC device drivers.
        let offload_scan = (wlan_softmac_info.driver_features
            & banjo_wlan_phyinfo::WlanInfoDriverFeature::SCAN_OFFLOAD)
            .0
            > 0;
        if offload_scan {
            match req.scan_type {
                fidl_mlme::ScanTypes::Passive => self.start_passive_scan(req),
                fidl_mlme::ScanTypes::Active => self.start_active_scan(req, &wlan_softmac_info),
            }
            .map(|ongoing_scan| self.scanner.ongoing_scan = Some(ongoing_scan))
            .map_err(|e| {
                self.scanner.ongoing_scan.take();
                e
            })?;
        } else {
            let channels = req
                .channel_list
                .iter()
                .map(|c| banjo_common::WlanChannel {
                    primary: *c,
                    cbw: banjo_common::ChannelBandwidth::CBW20,
                    secondary80: 0,
                })
                .collect();
            let max_channel_time = req.max_channel_time;
            // Note: For MLME scanning case, it's important to populate this beforehand because
            //       channel scheduler may `begin_requested_channel_time` immediately, and scanner
            //       needs these information to determine whether to send probe request.
            self.scanner.ongoing_scan =
                Some(OngoingScan::MlmeScan { req, probe_delay_timeout_id: None });
            let mut listener = build_channel_listener(self.ctx, self.scanner);
            let dwell_time = TimeUnit(max_channel_time as u16).into();
            chan_sched
                .bind(&mut listener, ChannelListenerSource::Scanner)
                .queue_channels(channels, dwell_time);
        }
        Ok(())
    }

    fn start_passive_scan(&mut self, req: fidl_mlme::ScanRequest) -> Result<OngoingScan, Error> {
        // Note: WlanSoftmacPassiveScanArgs contains raw pointers and the memory pointed
        // to must remain in scope for the duration of the call to Device::start_passive_scan().
        Ok(OngoingScan::PassiveOffloadScan {
            mlme_txn_id: req.txn_id,
            in_progress_device_scan_id: self
                .ctx
                .device
                .start_passive_scan(&banjo_wlan_softmac::WlanSoftmacPassiveScanArgs {
                    channels_list: req.channel_list.as_ptr(),
                    channels_count: req.channel_list.len(),
                    // TODO(fxbug.dev/89933): A TimeUnit is generally limited to 2 octets. Conversion here
                    // is required since fuchsia.wlan.mlme/ScanRequest.min_channel_time has a width of
                    // four octets.
                    min_channel_time: zx::Duration::from(TimeUnit(req.min_channel_time as u16))
                        .into_nanos(),
                    max_channel_time: zx::Duration::from(TimeUnit(req.max_channel_time as u16))
                        .into_nanos(),
                    min_home_time: MIN_HOME_TIME.into_nanos(),
                })
                .map_err(|status| Error::ScanError(ScanError::StartOffloadScanFails(status)))?,
        })
    }

    fn start_active_scan(
        &mut self,
        req: fidl_mlme::ScanRequest,
        wlan_softmac_info: &banjo_wlan_softmac::WlanSoftmacInfo,
    ) -> Result<OngoingScan, Error> {
        let ssids_list = req
            .ssid_list
            .iter()
            .map(cssid_from_ssid_unchecked)
            .collect::<Vec<banjo_ieee80211::CSsid>>();

        let (mac_header, _) = write_frame_with_dynamic_buf!(vec![], {
            headers: {
                mac::MgmtHdr: &self.probe_request_mac_header(),
            },
        })?;

        let mut remaining_active_scan_args = active_scan_args_series(
            // TODO(fxbug.dev/89933): A TimeUnit is generally limited to 2 octets. Conversion here
            // is required since fuchsia.wlan.mlme/ScanRequest.min_channel_time has a width of
            // four octets.
            zx::Duration::from(TimeUnit(req.min_channel_time as u16)).into_nanos(),
            zx::Duration::from(TimeUnit(req.max_channel_time as u16)).into_nanos(),
            MIN_HOME_TIME.into_nanos(),
            MIN_PROBES_PER_CHANNEL,
            MAX_PROBES_PER_CHANNEL,
            ssids_list,
            mac_header,
            wlan_softmac_info,
            req.channel_list,
        )?;

        match remaining_active_scan_args.pop() {
            None => {
                error!("unexpected empty list of active scan args");
                return Err(Error::ScanError(ScanError::StartOffloadScanFails(
                    zx::Status::INVALID_ARGS,
                )));
            }
            Some(active_scan_args) => Ok(OngoingScan::ActiveOffloadScan {
                mlme_txn_id: req.txn_id,
                in_progress_device_scan_id: self
                    .start_next_active_scan(active_scan_args)
                    .map_err(|scan_error| Error::ScanError(scan_error))?,
                remaining_active_scan_args,
            }),
        }
    }

    fn start_next_active_scan(
        &mut self,
        active_scan_args: ActiveScanArgs,
    ) -> Result<u64, ScanError> {
        // Note: active_scan_args must outlive the WlanSoftmacActiveScanArgs it returns
        // because WlanSoftmacActiveScanArgs contains raw pointers that must be valid
        // for the duration of the call to Device::start_active_scan().
        self.ctx
            .device
            .start_active_scan(&active_scan_args)
            .map_err(|status| ScanError::StartOffloadScanFails(status))
    }

    /// Called when MLME receives a beacon or probe response so that scanner saves it in a BSS map.
    ///
    /// This method is a no-op if no scan request is in progress.
    pub fn handle_beacon_or_probe_response(
        &mut self,
        bssid: Bssid,
        beacon_interval: TimeUnit,
        capability_info: CapabilityInfo,
        ies: &[u8],
        rx_info: banjo_wlan_softmac::WlanRxInfo,
    ) {
        let mlme_txn_id = match self.scanner.ongoing_scan {
            Some(OngoingScan::MlmeScan { req: fidl_mlme::ScanRequest { txn_id, .. }, .. }) => {
                txn_id
            }
            Some(OngoingScan::PassiveOffloadScan { mlme_txn_id, .. }) => mlme_txn_id,
            Some(OngoingScan::ActiveOffloadScan { mlme_txn_id, .. }) => mlme_txn_id,
            None => return,
        };
        let bss_description =
            construct_bss_description(bssid, beacon_interval, capability_info, ies, rx_info);
        let bss_description = match bss_description {
            Ok(bss) => bss,
            Err(e) => {
                warn!("Failed to process beacon or probe response: {}", e);
                return;
            }
        };
        send_scan_result(mlme_txn_id, bss_description, &mut self.ctx.device);
    }

    /// Notify scanner about end of probe-delay timeout so that it sends out probe request.
    pub fn handle_probe_delay_timeout(&mut self, channel: banjo_common::WlanChannel) {
        let ssid_list = match &self.scanner.ongoing_scan {
            Some(OngoingScan::MlmeScan { req, .. }) => req.ssid_list.clone(),
            Some(OngoingScan::PassiveOffloadScan { .. }) => {
                warn!("Unexpected probe_delay_timeout during PassiveOffloadScan.");
                return;
            }
            Some(OngoingScan::ActiveOffloadScan { .. }) => {
                warn!("Unexpected probe_delay_timeout during ActiveOffloadScan.");
                return;
            }
            None => {
                warn!("Unexpected probe_delay_timeout when no scan in progress.");
                return;
            }
        };
        for ssid in ssid_list {
            if let Err(e) = self.send_probe_req(&ssid[..], channel) {
                error!("{}", e);
            }
        }
    }

    pub fn handle_scan_complete(&mut self, status: zx::Status, scan_id: u64) {
        macro_rules! send_on_scan_end {
            ($mlme_txn_id: ident, $code:expr) => {
                let _ = self
                    .ctx
                    .device
                    .mlme_control_handle()
                    .send_on_scan_end(&mut fidl_mlme::ScanEnd { txn_id: $mlme_txn_id, code: $code })
                    .map_err(|e| {
                        error!("error sending MLME ScanEnd: {}", e);
                    });
            };
        }

        match self.scanner.ongoing_scan.take() {
            // TODO(fxbug.dev/91045): A spurious ScanComplete should not silently cancel an
            // MlmeScan by permanently taking the contents of ongoing_scan.
            Some(OngoingScan::MlmeScan { .. }) => {
                warn!("Unexpected ScanComplete with status {:?} during MlmeScan.", status);
            }
            None => {
                warn!("Unexpected ScanComplete when no scan in progress.");
            }
            Some(OngoingScan::PassiveOffloadScan { mlme_txn_id, in_progress_device_scan_id })
                if in_progress_device_scan_id == scan_id =>
            {
                send_on_scan_end!(
                    mlme_txn_id,
                    if status == zx::Status::OK {
                        fidl_mlme::ScanResultCode::Success
                    } else {
                        error!("passive offload scan failed: {}", status);
                        fidl_mlme::ScanResultCode::InternalError
                    }
                );
            }
            Some(OngoingScan::ActiveOffloadScan {
                mlme_txn_id,
                in_progress_device_scan_id,
                mut remaining_active_scan_args,
            }) if in_progress_device_scan_id == scan_id => {
                if status != zx::Status::OK {
                    error!("active offload scan failed: {}", status);
                    send_on_scan_end!(mlme_txn_id, fidl_mlme::ScanResultCode::InternalError);
                    return;
                }

                match remaining_active_scan_args.pop() {
                    None => {
                        send_on_scan_end!(mlme_txn_id, fidl_mlme::ScanResultCode::Success);
                    }
                    Some(active_scan_args) => match self.start_next_active_scan(active_scan_args) {
                        Ok(in_progress_device_scan_id) => {
                            self.scanner.ongoing_scan = Some(OngoingScan::ActiveOffloadScan {
                                mlme_txn_id,
                                in_progress_device_scan_id,
                                remaining_active_scan_args,
                            });
                        }
                        Err(scan_error) => {
                            self.scanner.ongoing_scan.take();
                            send_on_scan_end!(mlme_txn_id, scan_error.into());
                        }
                    },
                }
            }
            Some(other) => {
                let in_progress_device_scan_id = match other {
                    OngoingScan::ActiveOffloadScan { in_progress_device_scan_id, .. } => {
                        in_progress_device_scan_id
                    }
                    OngoingScan::PassiveOffloadScan { in_progress_device_scan_id, .. } => {
                        in_progress_device_scan_id
                    }
                    _ => unreachable!(),
                };
                warn!(
                    "Unexpected scan ID upon scan completion. expected: {}, returned: {}",
                    in_progress_device_scan_id, scan_id
                );
                self.scanner.ongoing_scan.replace(other);
            }
        }
    }

    /// Called after switching to a requested channel from a scan request. It's primarily to
    /// send out, or schedule to send out, a probe request in an active scan.
    pub fn begin_requested_channel_time(&mut self, channel: banjo_common::WlanChannel) {
        let (req, probe_delay_timeout_id) = match &mut self.scanner.ongoing_scan {
            Some(OngoingScan::MlmeScan { req, probe_delay_timeout_id }) => {
                (req, probe_delay_timeout_id)
            }
            Some(OngoingScan::PassiveOffloadScan { .. }) => {
                warn!("Unexpected begin_requested_channel_time during PassiveOffloadScan.");
                return;
            }
            Some(OngoingScan::ActiveOffloadScan { .. }) => {
                warn!("Unexpected begin_requested_channel_time during ActiveOffloadScan.");
                return;
            }
            None => {
                warn!("Unexpected begin_requested_channel_time when no scan in progress.");
                return;
            }
        };
        if req.scan_type == fidl_mlme::ScanTypes::Active {
            if req.probe_delay == 0 {
                for ssid in req.ssid_list.clone() {
                    if let Err(e) = self.send_probe_req(&ssid[..], channel) {
                        error!("{}", e);
                    }
                }
            } else {
                let timeout_id = self.ctx.timer.schedule_after(
                    TimeUnit(req.probe_delay as u16).into(),
                    TimedEvent::ScannerProbeDelay(channel),
                );
                probe_delay_timeout_id.replace(timeout_id);
            }
        }
    }

    fn probe_request_mac_header(&mut self) -> mac::MgmtHdr {
        mgmt_writer::mgmt_hdr_to_ap(
            mac::FrameControl(0)
                .with_frame_type(mac::FrameType::MGMT)
                .with_mgmt_subtype(mac::MgmtSubtype::PROBE_REQ),
            Bssid(mac::BCAST_ADDR),
            self.scanner.iface_mac,
            mac::SequenceControl(0)
                .with_seq_num(self.ctx.seq_mgr.next_sns1(&mac::BCAST_ADDR) as u16),
        )
    }

    fn send_probe_req(
        &mut self,
        ssid: &[u8],
        channel: banjo_common::WlanChannel,
    ) -> Result<(), Error> {
        // TODO(fxbug.dev/91038): This is not correct. Channel numbers do not imply band.
        let band = band_from_channel_number(channel.primary);
        let supported_rates = supported_rates_for_band(&self.ctx.device.wlan_softmac_info(), band)?;
        let (buf, bytes_written) = write_frame!(&mut self.ctx.buf_provider, {
            headers: {
                mac::MgmtHdr: &self.probe_request_mac_header(),
            },
            ies: {
                ssid: ssid,
                supported_rates: supported_rates,
                extended_supported_rates: {/* continue rates */},
            }
        })?;
        let out_buf = OutBuf::from(buf, bytes_written);
        self.ctx
            .device
            .send_wlan_frame(out_buf, TxFlags::NONE)
            .map_err(|s| Error::Status(format!("error sending probe req frame"), s))
    }

    /// Called when channel scheduler has gone through all the requested channels from a scan
    /// request. The scanner submits scan results to SME.
    pub fn handle_channel_req_complete(&mut self) {
        match self.scanner.ongoing_scan.take() {
            Some(OngoingScan::MlmeScan { req, .. }) => {
                let _ = self
                    .ctx
                    .device
                    .mlme_control_handle()
                    .send_on_scan_end(&mut fidl_mlme::ScanEnd {
                        txn_id: req.txn_id,
                        code: fidl_mlme::ScanResultCode::Success,
                    })
                    .map_err(|e| {
                        error!("error sending MLME ScanEnd: {}", e);
                    });
            }
            Some(OngoingScan::PassiveOffloadScan { .. }) => {
                warn!("Unexpected channel_req_complete during PassiveOffloadScan.");
                return;
            }
            Some(OngoingScan::ActiveOffloadScan { .. }) => {
                warn!("Unexpected channel_req_complete during ActiveOffloadScan.");
                return;
            }
            None => {
                warn!("Unexpected channel_req_complete when no scan in progress.");
                return;
            }
        }
    }
}

fn band_cap_for_band(
    wlan_softmac_info: &banjo_wlan_softmac::WlanSoftmacInfo,
    band: banjo_wlan_phyinfo::WlanInfoBand,
) -> Option<&banjo_wlan_softmac::WlanSoftmacBandCapability> {
    wlan_softmac_info.band_cap_list[..wlan_softmac_info.band_cap_count as usize]
        .iter()
        .filter(|b| b.band == band)
        .next()
}

// TODO(fxbug.dev/91036): Zero should not mark a null rate.
fn supported_rates_for_band(
    wlan_softmac_info: &banjo_wlan_softmac::WlanSoftmacInfo,
    band: banjo_wlan_phyinfo::WlanInfoBand,
) -> Result<Vec<u8>, Error> {
    let band_cap = band_cap_for_band(&wlan_softmac_info, band)
        .ok_or(format_err!("no band found for band {:?}", band))?;
    Ok(band_cap.basic_rate_list[..band_cap.basic_rate_count as usize].to_vec())
}

// TODO(fxbug.dev/91038): This is not correct. Channel numbers do not imply band.
fn band_from_channel_number(channel_number: u8) -> banjo_wlan_phyinfo::WlanInfoBand {
    if channel_number > 14 {
        banjo_wlan_phyinfo::WlanInfoBand::FIVE_GHZ
    } else {
        banjo_wlan_phyinfo::WlanInfoBand::TWO_GHZ
    }
}

fn active_scan_args_series(
    min_channel_time: zx::sys::zx_duration_t,
    max_channel_time: zx::sys::zx_duration_t,
    min_home_time: zx::sys::zx_duration_t,
    min_probes_per_channel: u8,
    max_probes_per_channel: u8,
    ssids_list: Vec<banjo_ieee80211::CSsid>,
    mac_header: Vec<u8>,
    wlan_softmac_info: &banjo_wlan_softmac::WlanSoftmacInfo,
    channel_list: Vec<u8>,
) -> Result<Vec<ActiveScanArgs>, Error> {
    // TODO(fxbug.dev/91038): The fuchsia.wlan.mlme/MLME API assumes channels numbers imply bands
    // and so partitioning channels must be done internally.
    let channel_lists: [ChannelList; 2] = channel_list.into_iter().fold(
        [
            ChannelList { band: banjo_wlan_phyinfo::WlanInfoBand::FIVE_GHZ, channels: vec![] },
            ChannelList { band: banjo_wlan_phyinfo::WlanInfoBand::TWO_GHZ, channels: vec![] },
        ],
        |mut channel_lists, c| {
            for cl in &mut channel_lists {
                if band_from_channel_number(c) == cl.band {
                    cl.channels.push(c);
                }
            }
            channel_lists
        },
    );

    let mut active_scan_args_series = vec![];
    for cl in channel_lists {
        if !cl.channels.is_empty() {
            let supported_rates = supported_rates_for_band(wlan_softmac_info, cl.band)?;
            active_scan_args_series.push(ActiveScanArgs {
                min_channel_time,
                max_channel_time,
                min_home_time,
                min_probes_per_channel,
                max_probes_per_channel,
                ssids_list: ssids_list.clone(),
                mac_header: mac_header.clone(),
                channels: cl.channels,
                // Exclude the SSID IE because the device driver will generate
                // using ssids_list.
                ies: write_frame_with_dynamic_buf!(vec![], {
                    ies: {
                        supported_rates: supported_rates,
                        extended_supported_rates: {/* continue rates */},
                    }
                })?
                .0,
            });
        }
    }
    Ok(active_scan_args_series)
}

fn send_scan_result(txn_id: u64, bss: fidl_internal::BssDescription, device: &mut Device) {
    let result = device.mlme_control_handle().send_on_scan_result(&mut fidl_mlme::ScanResult {
        txn_id,
        timestamp_nanos: zx::Time::get_monotonic().into_nanos(),
        bss,
    });
    if let Err(e) = result {
        error!("error sending MLME ScanResult: {}", e);
    }
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::{
            buffer::FakeBufferProvider,
            client::{
                channel_listener::{LEvent, MockListenerState},
                channel_scheduler, ClientConfig,
            },
            device::FakeDevice,
            test_utils::MockWlanRxInfo,
        },
        fidl_fuchsia_wlan_common as fidl_common, fuchsia_async as fasync,
        ieee80211::Ssid,
        lazy_static::lazy_static,
        std::{cell::RefCell, convert::TryFrom, rc::Rc},
        test_case::test_case,
        wlan_common::{
            assert_variant,
            sequence::SequenceManager,
            timer::{create_timer, TimeStream, Timer},
        },
    };

    const BSSID_FOO: Bssid = Bssid([6u8; 6]);
    const CAPABILITY_INFO_FOO: CapabilityInfo = CapabilityInfo(1);
    const BEACON_INTERVAL_FOO: u16 = 100;
    #[rustfmt::skip]
    static BEACON_IES_FOO: &'static [u8] = &[
        // SSID: "ssid"
        0x00, 0x03, b'f', b'o', b'o',
        // Supported rates: 24(B), 36, 48, 54
        0x01, 0x04, 0xb0, 0x48, 0x60, 0x6c,
        // TIM - DTIM count: 0, DTIM period: 1, PVB: 2
        0x05, 0x04, 0x00, 0x01, 0x00, 0x02,
    ];
    lazy_static! {
        static ref RX_INFO_FOO: banjo_wlan_softmac::WlanRxInfo =
            MockWlanRxInfo { rssi_dbm: -30, ..Default::default() }.into();
        static ref BSS_DESCRIPTION_FOO: fidl_internal::BssDescription =
            fidl_internal::BssDescription {
                bssid: BSSID_FOO.0,
                bss_type: fidl_internal::BssType::Infrastructure,
                beacon_period: BEACON_INTERVAL_FOO,
                capability_info: CAPABILITY_INFO_FOO.0,
                ies: BEACON_IES_FOO.to_vec(),
                rssi_dbm: RX_INFO_FOO.rssi_dbm,
                channel: fidl_common::WlanChannel {
                    primary: RX_INFO_FOO.channel.primary,
                    cbw: fidl_common::ChannelBandwidth::Cbw20,
                    secondary80: 0,
                },
                snr_db: 0,
            };
    }

    const BSSID_BAR: Bssid = Bssid([1u8; 6]);
    const CAPABILITY_INFO_BAR: CapabilityInfo = CapabilityInfo(33);
    const BEACON_INTERVAL_BAR: u16 = 150;
    #[rustfmt::skip]
    static BEACON_IES_BAR: &'static [u8] = &[
        // SSID: "ss"
        0x00, 0x03, b'b', b'a', b'r',
        // Supported rates: 24(B), 36, 48, 54
        0x01, 0x04, 0xb0, 0x48, 0x60, 0x6c,
        // TIM - DTIM count: 0, DTIM period: 1, PVB: 2
        0x05, 0x04, 0x00, 0x01, 0x00, 0x02,
    ];
    lazy_static! {
        static ref RX_INFO_BAR: banjo_wlan_softmac::WlanRxInfo =
            MockWlanRxInfo { rssi_dbm: -60, ..Default::default() }.into();
        static ref BSS_DESCRIPTION_BAR: fidl_internal::BssDescription =
            fidl_internal::BssDescription {
                bssid: BSSID_BAR.0,
                bss_type: fidl_internal::BssType::Infrastructure,
                beacon_period: BEACON_INTERVAL_BAR,
                capability_info: CAPABILITY_INFO_BAR.0,
                ies: BEACON_IES_BAR.to_vec(),
                rssi_dbm: RX_INFO_BAR.rssi_dbm,
                channel: fidl_common::WlanChannel {
                    primary: RX_INFO_BAR.channel.primary,
                    cbw: fidl_common::ChannelBandwidth::Cbw20,
                    secondary80: 0,
                },
                snr_db: 0,
            };
    }

    const IFACE_MAC: MacAddr = [7u8; 6];
    // Original channel set by FakeDevice
    const ORIGINAL_CHAN: banjo_common::WlanChannel = channel(0);

    fn passive_scan_req() -> fidl_mlme::ScanRequest {
        fidl_mlme::ScanRequest {
            txn_id: 1337,
            scan_type: fidl_mlme::ScanTypes::Passive,
            channel_list: vec![6],
            ssid_list: vec![],
            probe_delay: 0,
            min_channel_time: 100,
            max_channel_time: 300,
        }
    }

    fn active_scan_req(channel_list: &[u8]) -> fidl_mlme::ScanRequest {
        fidl_mlme::ScanRequest {
            txn_id: 1337,
            scan_type: fidl_mlme::ScanTypes::Active,
            channel_list: Vec::from(channel_list),
            ssid_list: vec![
                Ssid::try_from("foo").unwrap().into(),
                Ssid::try_from("bar").unwrap().into(),
            ],
            probe_delay: 3,
            min_channel_time: 100,
            max_channel_time: 300,
        }
    }

    #[test]
    fn test_handle_scan_req_queues_channels() {
        let exec = fasync::TestExecutor::new().expect("failed to create an executor");
        let mut m = MockObjects::new(&exec);
        let mut ctx = m.make_ctx();
        let mut scanner = Scanner::new(IFACE_MAC);

        scanner
            .bind(&mut ctx)
            .on_sme_scan(
                passive_scan_req(),
                m.listener_state.create_channel_listener_fn(),
                &mut m.chan_sched,
            )
            .expect("expect scan req accepted");
        let req_id = channel_scheduler::RequestId(1, ChannelListenerSource::Scanner);
        assert_eq!(
            m.listener_state.drain_events(),
            vec![
                LEvent::PreSwitch { from: ORIGINAL_CHAN, to: channel(6), req_id },
                LEvent::PostSwitch { from: ORIGINAL_CHAN, to: channel(6), req_id },
            ]
        );
    }

    #[test]
    fn test_active_scan_probe_req_sent_with_no_delay() {
        let exec = fasync::TestExecutor::new().expect("failed to create an executor");
        let mut m = MockObjects::new(&exec);
        let mut ctx = m.make_ctx();
        let mut scanner = Scanner::new(IFACE_MAC);

        let scan_req = fidl_mlme::ScanRequest { probe_delay: 0, ..active_scan_req(&[6]) };
        scanner
            .bind(&mut ctx)
            .on_sme_scan(scan_req, m.listener_state.create_channel_listener_fn(), &mut m.chan_sched)
            .expect("expect scan req accepted");
        let req_id = channel_scheduler::RequestId(1, ChannelListenerSource::Scanner);
        assert_eq!(
            m.listener_state.drain_events(),
            vec![
                LEvent::PreSwitch { from: ORIGINAL_CHAN, to: channel(6), req_id },
                LEvent::PostSwitch { from: ORIGINAL_CHAN, to: channel(6), req_id },
            ]
        );

        // On post-switch announcement, the listener would call `begin_requested_channel_time`
        scanner.bind(&mut ctx).begin_requested_channel_time(channel(6));
        assert_eq!(m.fake_device.wlan_queue.len(), 2);
        #[rustfmt::skip]
        assert_eq!(&m.fake_device.wlan_queue[0].0[..], &[
            // Mgmt header:
            0b0100_00_00, 0b00000000, // FC
            0, 0, // Duration
            0xff, 0xff, 0xff, 0xff, 0xff, 0xff, // addr1
            7, 7, 7, 7, 7, 7, // addr2
            0xff, 0xff, 0xff, 0xff, 0xff, 0xff, // addr3
            0x10, 0, // Sequence Control
            // IEs
            0, 3, // SSID id and length
            b'f', b'o', b'o', // SSID
            1, 8, // supp_rates id and length
            0x0C, 0x12, 0x18, 0x24, 0x30, 0x48, 0x60, 0x6C // supp_rates
        ][..]);
        #[rustfmt::skip]
        assert_eq!(&m.fake_device.wlan_queue[1].0[..], &[
            // Mgmt header:
            0b0100_00_00, 0b00000000, // FC
            0, 0, // Duration
            0xff, 0xff, 0xff, 0xff, 0xff, 0xff, // addr1
            7, 7, 7, 7, 7, 7, // addr2
            0xff, 0xff, 0xff, 0xff, 0xff, 0xff, // addr3
            0x20, 0, // Sequence Control
            // IEs
            0, 3, // SSID id and length
            b'b', b'a', b'r', // SSID
            1, 8, // supp_rates id and length
            0x0C, 0x12, 0x18, 0x24, 0x30, 0x48, 0x60, 0x6C // supp_rates
        ][..]);
    }

    fn get_timed_event(id: EventId, time_stream: &mut TimeStream<TimedEvent>) -> TimedEvent {
        loop {
            let (_, event) = time_stream.try_next().unwrap().unwrap();
            if event.id == id {
                return event.event;
            }
        }
    }

    #[test]
    fn test_active_scan_probe_req_sent_with_delay() {
        let exec = fasync::TestExecutor::new().expect("failed to create an executor");
        let mut m = MockObjects::new(&exec);
        let mut ctx = m.make_ctx();
        let mut scanner = Scanner::new(IFACE_MAC);

        let scan_req = fidl_mlme::ScanRequest { probe_delay: 5, ..active_scan_req(&[6]) };
        scanner
            .bind(&mut ctx)
            .on_sme_scan(scan_req, m.listener_state.create_channel_listener_fn(), &mut m.chan_sched)
            .expect("expect scan req accepted");
        scanner.bind(&mut ctx).begin_requested_channel_time(channel(6));

        // Verify nothing is sent yet, but timeout is scheduled
        assert!(m.fake_device.wlan_queue.is_empty());
        assert!(scanner.probe_delay_timeout_id().is_some());
        let timeout_id = scanner.probe_delay_timeout_id().unwrap();

        assert_eq!(
            get_timed_event(timeout_id, &mut m.time_stream),
            TimedEvent::ScannerProbeDelay(channel(6))
        );

        // Check that telling scanner to handle timeout would send probe request frame
        scanner.bind(&mut ctx).handle_probe_delay_timeout(channel(6));
        assert_eq!(m.fake_device.wlan_queue.len(), 2);
        #[rustfmt::skip]
        assert_eq!(&m.fake_device.wlan_queue[0].0[..], &[
            // Mgmt header:
            0b0100_00_00, 0b00000000, // FC
            0, 0, // Duration
            0xff, 0xff, 0xff, 0xff, 0xff, 0xff, // addr1
            7, 7, 7, 7, 7, 7, // addr2
            0xff, 0xff, 0xff, 0xff, 0xff, 0xff, // addr3
            0x10, 0, // Sequence Control
            // IEs
            0, 3, // SSID id and length
            b'f', b'o', b'o', // SSID
            1, 8, // supp_rates id and length
            0x0C, 0x12, 0x18, 0x24, 0x30, 0x48, 0x60, 0x6C // supp_rates
        ][..]);
        #[rustfmt::skip]
        assert_eq!(&m.fake_device.wlan_queue[1].0[..], &[
            // Mgmt header:
            0b0100_00_00, 0b00000000, // FC
            0, 0, // Duration
            0xff, 0xff, 0xff, 0xff, 0xff, 0xff, // addr1
            7, 7, 7, 7, 7, 7, // addr2
            0xff, 0xff, 0xff, 0xff, 0xff, 0xff, // addr3
            0x20, 0, // Sequence Control
            // IEs
            0, 3, // SSID id and length
            b'b', b'a', b'r', // SSID
            1, 8, // supp_rates id and length
            0x0C, 0x12, 0x18, 0x24, 0x30, 0x48, 0x60, 0x6C // supp_rates
        ][..]);
    }

    #[test]
    fn test_handle_scan_req_reject_if_busy() {
        let exec = fasync::TestExecutor::new().expect("failed to create an executor");
        let mut m = MockObjects::new(&exec);
        let mut ctx = m.make_ctx();
        let mut scanner = Scanner::new(IFACE_MAC);

        scanner
            .bind(&mut ctx)
            .on_sme_scan(
                passive_scan_req(),
                m.listener_state.create_channel_listener_fn(),
                &mut m.chan_sched,
            )
            .expect("expect scan req accepted");
        let scan_req = fidl_mlme::ScanRequest { txn_id: 1338, ..passive_scan_req() };
        let result = scanner.bind(&mut ctx).on_sme_scan(
            scan_req,
            m.listener_state.create_channel_listener_fn(),
            &mut m.chan_sched,
        );
        assert_variant!(result, Err(Error::ScanError(ScanError::Busy)));
        m.fake_device
            .next_mlme_msg::<fidl_mlme::ScanEnd>()
            .expect_err("unexpected MLME ScanEnd from BoundScanner");
    }

    #[test]
    fn test_handle_scan_req_empty_channel_list() {
        let exec = fasync::TestExecutor::new().expect("failed to create an executor");
        let mut m = MockObjects::new(&exec);
        let mut ctx = m.make_ctx();
        let mut scanner = Scanner::new(IFACE_MAC);

        let scan_req = fidl_mlme::ScanRequest { channel_list: vec![], ..passive_scan_req() };
        let result = scanner.bind(&mut ctx).on_sme_scan(
            scan_req,
            m.listener_state.create_channel_listener_fn(),
            &mut m.chan_sched,
        );
        assert_variant!(result, Err(Error::ScanError(ScanError::EmptyChannelList)));
        m.fake_device
            .next_mlme_msg::<fidl_mlme::ScanEnd>()
            .expect_err("unexpected MLME ScanEnd from BoundScanner");
    }

    #[test]
    fn test_handle_scan_req_invalid_channel_time() {
        let exec = fasync::TestExecutor::new().expect("failed to create an executor");
        let mut m = MockObjects::new(&exec);
        let mut ctx = m.make_ctx();
        let mut scanner = Scanner::new(IFACE_MAC);

        let scan_req = fidl_mlme::ScanRequest {
            min_channel_time: 101,
            max_channel_time: 100,
            ..passive_scan_req()
        };
        let result = scanner.bind(&mut ctx).on_sme_scan(
            scan_req,
            m.listener_state.create_channel_listener_fn(),
            &mut m.chan_sched,
        );
        assert_variant!(result, Err(Error::ScanError(ScanError::MaxChannelTimeLtMin)));
        m.fake_device
            .next_mlme_msg::<fidl_mlme::ScanEnd>()
            .expect_err("unexpected MLME ScanEnd from BoundScanner");
    }

    #[test]
    fn test_start_offload_passive_scan_success() {
        let exec = fasync::TestExecutor::new().expect("failed to create an executor");
        let mut m = MockObjects::new(&exec);
        let mut ctx = m.make_ctx();
        m.fake_device.info.driver_features |=
            banjo_wlan_phyinfo::WlanInfoDriverFeature::SCAN_OFFLOAD;
        let mut scanner = Scanner::new(IFACE_MAC);
        let test_start_timestamp_nanos = zx::Time::get_monotonic().into_nanos();

        scanner
            .bind(&mut ctx)
            .on_sme_scan(
                passive_scan_req(),
                m.listener_state.create_channel_listener_fn(),
                &mut m.chan_sched,
            )
            .expect("expect scan req accepted");

        // Verify that passive offload scan is requested
        assert_variant!(
            m.fake_device.captured_passive_scan_args,
            Some(ref passive_scan_args) => {
                assert_eq!(passive_scan_args.channels.len(), 1);
                assert_eq!(passive_scan_args.channels, &[6]);
                assert_eq!(passive_scan_args.min_channel_time, 102_400_000);
                assert_eq!(passive_scan_args.max_channel_time, 307_200_000);
                assert_eq!(passive_scan_args.min_home_time, 0);
            },
            "passive offload scan not initiated"
        );
        let expected_scan_id = m.fake_device.next_scan_id - 1;

        // Mock receiving a beacon
        handle_beacon_foo(&mut scanner, &mut ctx);
        let scan_result = m
            .fake_device
            .next_mlme_msg::<fidl_mlme::ScanResult>()
            .expect("error reading ScanResult");
        assert_eq!(scan_result.txn_id, 1337);
        assert!(scan_result.timestamp_nanos > test_start_timestamp_nanos);
        assert_eq!(scan_result.bss, *BSS_DESCRIPTION_FOO);

        // Verify ScanEnd sent after handle_scan_complete
        scanner.bind(&mut ctx).handle_scan_complete(zx::Status::OK, expected_scan_id);
        let scan_end = m
            .fake_device
            .next_mlme_msg::<fidl_mlme::ScanEnd>()
            .expect("error reading MLME ScanEnd");
        assert_eq!(
            scan_end,
            fidl_mlme::ScanEnd { txn_id: 1337, code: fidl_mlme::ScanResultCode::Success }
        );
    }

    struct ExpectedDynamicActiveScanArgs {
        channels: Vec<u8>,
        ies: Vec<u8>,
    }

    #[test_case(&[6],
                Some(ExpectedDynamicActiveScanArgs {
                    channels: vec![6],
                    ies: vec![ 0x01, // Element ID for Supported Rates
                               0x08, // Length
                               0x0C, 0x12, 0x18, 0x24, 0x30, 0x48, 0x60, 0x6C // Supported Rates
                    ]}),
                None; "single channel")]
    #[test_case(&[1, 2, 3, 4, 5],
                Some(ExpectedDynamicActiveScanArgs {
                    channels: vec![1, 2, 3, 4, 5],
                    ies: vec![ 0x01, // Element ID for Supported Rates
                               0x08, // Length
                               0x0C, 0x12, 0x18, 0x24, 0x30, 0x48, 0x60, 0x6C // Supported Rates
                ]}),
                None; "multiple channels 2.4GHz band")]
    #[test_case(&[36, 40, 100, 108],
                None,
                Some(ExpectedDynamicActiveScanArgs {
                    channels: vec![36, 40, 100, 108],
                    ies: vec![ 0x01, // Element ID for Supported Rates
                               0x02, // Length
                               0x7E, 0x7F // Supported Rates
                    ],
                }); "multiple channels 5GHz band")]
    #[test_case(&[1, 2, 3, 4, 5, 36, 40, 100, 108],
                Some(ExpectedDynamicActiveScanArgs {
                    channels: vec![1, 2, 3, 4, 5],
                    ies: vec![ 0x01, // Element ID for Supported Rates
                               0x08, // Length
                               0x0C, 0x12, 0x18, 0x24, 0x30, 0x48, 0x60, 0x6C // Supported Rates
                    ]}),
                Some(ExpectedDynamicActiveScanArgs {
                    channels: vec![36, 40, 100, 108],
                    ies: vec![ 0x01, // Element ID for Supported Rates
                               0x02, // Length
                               0x7E, 0x7F, // Supported Rates
                    ],
                }); "multiple bands")]
    fn test_start_active_scan_success(
        channel_list: &[u8],
        expected_two_ghz_dynamic_args: Option<ExpectedDynamicActiveScanArgs>,
        expected_five_ghz_dynamic_args: Option<ExpectedDynamicActiveScanArgs>,
    ) {
        let exec = fasync::TestExecutor::new().expect("failed to create an executor");
        let mut m = MockObjects::new(&exec);
        let mut ctx = m.make_ctx();
        m.fake_device.info.driver_features |=
            banjo_wlan_phyinfo::WlanInfoDriverFeature::SCAN_OFFLOAD;
        let mut scanner = Scanner::new(IFACE_MAC);
        let test_start_timestamp_nanos = zx::Time::get_monotonic().into_nanos();

        scanner
            .bind(&mut ctx)
            .on_sme_scan(
                active_scan_req(channel_list),
                m.listener_state.create_channel_listener_fn(),
                &mut m.chan_sched,
            )
            .expect("expect scan req accepted");

        for probe_request_ies in &[expected_two_ghz_dynamic_args, expected_five_ghz_dynamic_args] {
            match probe_request_ies {
                None => {}
                Some(ExpectedDynamicActiveScanArgs { channels, ies, .. }) => {
                    // Verify that active offload scan is requested
                    assert_variant!(
                        m.fake_device.captured_active_scan_args,
                        Some(ref active_scan_args) => {
                            assert_eq!(active_scan_args.min_channel_time, 102_400_000);
                            assert_eq!(active_scan_args.max_channel_time, 307_200_000);
                            assert_eq!(active_scan_args.min_home_time, 0);
                            assert_eq!(active_scan_args.min_probes_per_channel, 0);
                            assert_eq!(active_scan_args.max_probes_per_channel, 0);
                            assert_eq!(active_scan_args.channels, *channels);
                            assert_eq!(active_scan_args.ssids_list,
                                       vec![
                                           cssid_from_ssid_unchecked(&Ssid::try_from("foo").unwrap().into()),
                                           cssid_from_ssid_unchecked(&Ssid::try_from("bar").unwrap().into()),
                                       ]);
                            assert_eq!(active_scan_args.mac_header,
                                       vec![
                                           0x40, 0x00, // Frame Control
                                           0x00, 0x00, // Duration
                                           0xff, 0xff, 0xff, 0xff, 0xff, 0xff, // Address 1
                                           0x07, 0x07, 0x07, 0x07, 0x07, 0x07, // Address 2
                                           0xff, 0xff, 0xff, 0xff, 0xff, 0xff, // Address 3
                                           0x10, 0x00, // Sequence Control
                                       ]);
                            assert_eq!(active_scan_args.ies,
                                       ies[..]);
                        },
                        "active offload scan not initiated"
                    );
                    let expected_scan_id = m.fake_device.next_scan_id - 1;

                    // Mock receiving beacons
                    handle_beacon_foo(&mut scanner, &mut ctx);
                    let scan_result = m
                        .fake_device
                        .next_mlme_msg::<fidl_mlme::ScanResult>()
                        .expect("error reading ScanResult");
                    assert_eq!(scan_result.txn_id, 1337);
                    assert!(scan_result.timestamp_nanos > test_start_timestamp_nanos);
                    assert_eq!(scan_result.bss, *BSS_DESCRIPTION_FOO);

                    handle_beacon_bar(&mut scanner, &mut ctx);
                    let scan_result = m
                        .fake_device
                        .next_mlme_msg::<fidl_mlme::ScanResult>()
                        .expect("error reading ScanResult");
                    assert_eq!(scan_result.txn_id, 1337);
                    assert!(scan_result.timestamp_nanos > test_start_timestamp_nanos);
                    assert_eq!(scan_result.bss, *BSS_DESCRIPTION_BAR);

                    // Verify ScanEnd sent after handle_scan_complete
                    scanner.bind(&mut ctx).handle_scan_complete(zx::Status::OK, expected_scan_id);
                }
            }
        }
        let scan_end = m
            .fake_device
            .next_mlme_msg::<fidl_mlme::ScanEnd>()
            .expect("error reading MLME ScanEnd");
        assert_eq!(
            scan_end,
            fidl_mlme::ScanEnd { txn_id: 1337, code: fidl_mlme::ScanResultCode::Success }
        );
    }

    #[test]
    fn test_start_passive_scan_fails() {
        let exec = fasync::TestExecutor::new().expect("failed to create an executor");
        let mut m = MockObjects::new(&exec);
        let device = m.fake_device.as_device_fail_start_passive_scan();
        let mut ctx = m.make_ctx_with_device(device);
        m.fake_device.info.driver_features |=
            banjo_wlan_phyinfo::WlanInfoDriverFeature::SCAN_OFFLOAD;
        let mut scanner = Scanner::new(IFACE_MAC);

        let result = scanner.bind(&mut ctx).on_sme_scan(
            passive_scan_req(),
            m.listener_state.create_channel_listener_fn(),
            &mut m.chan_sched,
        );
        assert_variant!(
            result,
            Err(Error::ScanError(ScanError::StartOffloadScanFails(zx::Status::NOT_SUPPORTED)))
        );
        m.fake_device
            .next_mlme_msg::<fidl_mlme::ScanEnd>()
            .expect_err("unexpected MLME ScanEnd from BoundScanner");
    }

    #[test]
    fn test_start_active_scan_fails() {
        let exec = fasync::TestExecutor::new().expect("failed to create an executor");
        let mut m = MockObjects::new(&exec);
        let device = m.fake_device.as_device_fail_start_active_scan();
        let mut ctx = m.make_ctx_with_device(device);
        m.fake_device.info.driver_features |=
            banjo_wlan_phyinfo::WlanInfoDriverFeature::SCAN_OFFLOAD;
        let mut scanner = Scanner::new(IFACE_MAC);

        let result = scanner.bind(&mut ctx).on_sme_scan(
            active_scan_req(&[6]),
            m.listener_state.create_channel_listener_fn(),
            &mut m.chan_sched,
        );
        assert_variant!(
            result,
            Err(Error::ScanError(ScanError::StartOffloadScanFails(zx::Status::NOT_SUPPORTED)))
        );
        m.fake_device
            .next_mlme_msg::<fidl_mlme::ScanEnd>()
            .expect_err("unexpected MLME ScanEnd from BoundScanner");
    }

    #[test]
    fn test_start_passive_scan_canceled() {
        let exec = fasync::TestExecutor::new().expect("failed to create an executor");
        let mut m = MockObjects::new(&exec);
        let mut ctx = m.make_ctx();
        m.fake_device.info.driver_features |=
            banjo_wlan_phyinfo::WlanInfoDriverFeature::SCAN_OFFLOAD;
        let mut scanner = Scanner::new(IFACE_MAC);
        let test_start_timestamp_nanos = zx::Time::get_monotonic().into_nanos();

        scanner
            .bind(&mut ctx)
            .on_sme_scan(
                passive_scan_req(),
                m.listener_state.create_channel_listener_fn(),
                &mut m.chan_sched,
            )
            .expect("expect scan req accepted");

        // Verify that passive offload scan is requested
        assert_variant!(
            m.fake_device.captured_passive_scan_args,
            Some(_),
            "passive offload scan not initiated"
        );
        let expected_scan_id = m.fake_device.next_scan_id - 1;

        // Mock receiving a beacon
        handle_beacon_foo(&mut scanner, &mut ctx);
        let scan_result = m
            .fake_device
            .next_mlme_msg::<fidl_mlme::ScanResult>()
            .expect("error reading ScanResult");
        assert_eq!(scan_result.txn_id, 1337);
        assert!(scan_result.timestamp_nanos > test_start_timestamp_nanos);
        assert_eq!(scan_result.bss, *BSS_DESCRIPTION_FOO);

        // Verify ScanEnd sent after handle_scan_complete
        scanner.bind(&mut ctx).handle_scan_complete(zx::Status::CANCELED, expected_scan_id);
        let scan_end = m
            .fake_device
            .next_mlme_msg::<fidl_mlme::ScanEnd>()
            .expect("error reading MLME ScanEnd");
        assert_eq!(
            scan_end,
            fidl_mlme::ScanEnd { txn_id: 1337, code: fidl_mlme::ScanResultCode::InternalError }
        );
    }

    #[test]
    fn test_start_active_scan_canceled() {
        let exec = fasync::TestExecutor::new().expect("failed to create an executor");
        let mut m = MockObjects::new(&exec);
        let mut ctx = m.make_ctx();
        m.fake_device.info.driver_features |=
            banjo_wlan_phyinfo::WlanInfoDriverFeature::SCAN_OFFLOAD;
        let mut scanner = Scanner::new(IFACE_MAC);
        let test_start_timestamp_nanos = zx::Time::get_monotonic().into_nanos();

        scanner
            .bind(&mut ctx)
            .on_sme_scan(
                active_scan_req(&[6]),
                m.listener_state.create_channel_listener_fn(),
                &mut m.chan_sched,
            )
            .expect("expect scan req accepted");

        // Verify that active offload scan is requested
        assert_variant!(
            m.fake_device.captured_active_scan_args,
            Some(_),
            "active offload scan not initiated"
        );
        let expected_scan_id = m.fake_device.next_scan_id - 1;

        // Mock receiving a beacon
        handle_beacon_foo(&mut scanner, &mut ctx);
        let scan_result = m
            .fake_device
            .next_mlme_msg::<fidl_mlme::ScanResult>()
            .expect("error reading ScanResult");
        assert_eq!(scan_result.txn_id, 1337);
        assert!(scan_result.timestamp_nanos > test_start_timestamp_nanos);
        assert_eq!(scan_result.bss, *BSS_DESCRIPTION_FOO);

        // Verify ScanEnd sent after handle_scan_complete
        scanner.bind(&mut ctx).handle_scan_complete(zx::Status::CANCELED, expected_scan_id);
        let scan_end = m
            .fake_device
            .next_mlme_msg::<fidl_mlme::ScanEnd>()
            .expect("error reading MLME ScanEnd");
        assert_eq!(
            scan_end,
            fidl_mlme::ScanEnd { txn_id: 1337, code: fidl_mlme::ScanResultCode::InternalError }
        );
    }

    #[test]
    fn test_handle_beacon_or_probe_response() {
        let exec = fasync::TestExecutor::new().expect("failed to create an executor");
        let mut m = MockObjects::new(&exec);
        let mut ctx = m.make_ctx();
        let mut scanner = Scanner::new(IFACE_MAC);
        let test_start_timestamp_nanos = zx::Time::get_monotonic().into_nanos();

        scanner
            .bind(&mut ctx)
            .on_sme_scan(
                passive_scan_req(),
                m.listener_state.create_channel_listener_fn(),
                &mut m.chan_sched,
            )
            .expect("expect scan req accepted");
        handle_beacon_foo(&mut scanner, &mut ctx);
        scanner.bind(&mut ctx).handle_channel_req_complete();

        let scan_result = m
            .fake_device
            .next_mlme_msg::<fidl_mlme::ScanResult>()
            .expect("error reading MLME ScanResult");
        assert_eq!(scan_result.txn_id, 1337);
        assert!(scan_result.timestamp_nanos > test_start_timestamp_nanos);
        assert_eq!(scan_result.bss, *BSS_DESCRIPTION_FOO);

        let scan_end = m
            .fake_device
            .next_mlme_msg::<fidl_mlme::ScanEnd>()
            .expect("error reading MLME ScanEnd");
        assert_eq!(
            scan_end,
            fidl_mlme::ScanEnd { txn_id: 1337, code: fidl_mlme::ScanResultCode::Success }
        );
    }

    #[test]
    fn test_handle_beacon_or_probe_response_multiple() {
        let exec = fasync::TestExecutor::new().expect("failed to create an executor");
        let mut m = MockObjects::new(&exec);
        let mut ctx = m.make_ctx();
        let mut scanner = Scanner::new(IFACE_MAC);
        let test_start_timestamp_nanos = zx::Time::get_monotonic().into_nanos();

        scanner
            .bind(&mut ctx)
            .on_sme_scan(
                passive_scan_req(),
                m.listener_state.create_channel_listener_fn(),
                &mut m.chan_sched,
            )
            .expect("expect scan req accepted");

        handle_beacon_foo(&mut scanner, &mut ctx);
        handle_beacon_bar(&mut scanner, &mut ctx);
        scanner.bind(&mut ctx).handle_channel_req_complete();

        // Verify that one scan result is sent for each beacon
        let foo_scan_result = m
            .fake_device
            .next_mlme_msg::<fidl_mlme::ScanResult>()
            .expect("error reading MLME ScanResult");
        assert_eq!(foo_scan_result.txn_id, 1337);
        assert!(foo_scan_result.timestamp_nanos > test_start_timestamp_nanos);
        assert_eq!(foo_scan_result.bss, *BSS_DESCRIPTION_FOO);

        let bar_scan_result = m
            .fake_device
            .next_mlme_msg::<fidl_mlme::ScanResult>()
            .expect("error reading MLME ScanResult");
        assert_eq!(bar_scan_result.txn_id, 1337);
        assert!(bar_scan_result.timestamp_nanos > foo_scan_result.timestamp_nanos);
        assert_eq!(bar_scan_result.bss, *BSS_DESCRIPTION_BAR);

        let scan_end = m
            .fake_device
            .next_mlme_msg::<fidl_mlme::ScanEnd>()
            .expect("error reading MLME ScanEnd");
        assert_eq!(
            scan_end,
            fidl_mlme::ScanEnd { txn_id: 1337, code: fidl_mlme::ScanResultCode::Success }
        );
    }

    #[test]
    fn not_scanning_vs_scanning() {
        let exec = fasync::TestExecutor::new().expect("failed to create an executor");
        let mut m = MockObjects::new(&exec);
        let mut ctx = m.make_ctx();
        let mut scanner = Scanner::new(IFACE_MAC);
        assert_eq!(false, scanner.is_scanning());

        scanner
            .bind(&mut ctx)
            .on_sme_scan(
                passive_scan_req(),
                m.listener_state.create_channel_listener_fn(),
                &mut m.chan_sched,
            )
            .expect("expect scan req accepted");
        assert_eq!(true, scanner.is_scanning());
    }

    fn handle_beacon_foo(scanner: &mut Scanner, ctx: &mut Context) {
        scanner.bind(ctx).handle_beacon_or_probe_response(
            BSSID_FOO,
            TimeUnit(BEACON_INTERVAL_FOO),
            CAPABILITY_INFO_FOO,
            BEACON_IES_FOO,
            RX_INFO_FOO.clone(),
        );
    }

    fn handle_beacon_bar(scanner: &mut Scanner, ctx: &mut Context) {
        scanner.bind(ctx).handle_beacon_or_probe_response(
            BSSID_BAR,
            TimeUnit(BEACON_INTERVAL_BAR),
            CAPABILITY_INFO_BAR,
            BEACON_IES_BAR,
            RX_INFO_BAR.clone(),
        );
    }

    const fn channel(primary: u8) -> banjo_common::WlanChannel {
        banjo_common::WlanChannel {
            primary,
            cbw: banjo_common::ChannelBandwidth::CBW20,
            secondary80: 0,
        }
    }

    struct MockObjects {
        fake_device: FakeDevice,
        time_stream: TimeStream<TimedEvent>,
        timer: Option<Timer<TimedEvent>>,
        listener_state: MockListenerState,
        chan_sched: ChannelScheduler,
    }

    impl MockObjects {
        fn new(exec: &fasync::TestExecutor) -> Self {
            let (timer, time_stream) = create_timer();
            Self {
                fake_device: FakeDevice::new(exec),
                time_stream,
                timer: Some(timer),
                listener_state: MockListenerState { events: Rc::new(RefCell::new(vec![])) },
                chan_sched: ChannelScheduler::new(),
            }
        }

        fn make_ctx(&mut self) -> Context {
            let device = self.fake_device.as_device();
            self.make_ctx_with_device(device)
        }

        fn make_ctx_with_device(&mut self, device: Device) -> Context {
            Context {
                config: ClientConfig { ensure_on_channel_time: 0 },
                device,
                buf_provider: FakeBufferProvider::new(),
                timer: self.timer.take().unwrap(),
                seq_mgr: SequenceManager::new(),
            }
        }
    }
}
