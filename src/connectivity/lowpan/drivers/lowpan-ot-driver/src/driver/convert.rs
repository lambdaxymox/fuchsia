// Copyright 2022 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use crate::convert_ext::*;
use crate::prelude::*;
use fidl_fuchsia_lowpan::*;

impl FromExt<fidl_fuchsia_lowpan_device::RoutePreference> for ot::RoutePreference {
    fn from_ext(x: fidl_fuchsia_lowpan_device::RoutePreference) -> Self {
        match x {
            fidl_fuchsia_lowpan_device::RoutePreference::Low => ot::RoutePreference::Low,
            fidl_fuchsia_lowpan_device::RoutePreference::Medium => ot::RoutePreference::Medium,
            fidl_fuchsia_lowpan_device::RoutePreference::High => ot::RoutePreference::High,
        }
    }
}

impl FromExt<ot::RoutePreference> for fidl_fuchsia_lowpan_device::RoutePreference {
    fn from_ext(x: ot::RoutePreference) -> Self {
        match x {
            ot::RoutePreference::Low => fidl_fuchsia_lowpan_device::RoutePreference::Low,
            ot::RoutePreference::Medium => fidl_fuchsia_lowpan_device::RoutePreference::Medium,
            ot::RoutePreference::High => fidl_fuchsia_lowpan_device::RoutePreference::High,
        }
    }
}

impl FromExt<ot::ActiveScanResult> for BeaconInfo {
    fn from_ext(x: ot::ActiveScanResult) -> Self {
        BeaconInfo {
            identity: Identity {
                raw_name: if x.is_native() { Some(x.network_name().to_vec()) } else { None },
                net_type: if x.is_native() {
                    Some(fidl_fuchsia_lowpan::NET_TYPE_THREAD_1_X.to_string())
                } else {
                    None
                },
                channel: Some(x.channel().into()),
                panid: Some(x.pan_id()),
                ..Identity::EMPTY
            },
            rssi: x.rssi().into(),
            lqi: x.lqi(),
            address: x.ext_address().to_vec(),
            flags: vec![],
        }
    }
}

impl FromExt<&ot::OperationalDataset> for Identity {
    fn from_ext(operational_dataset: &ot::OperationalDataset) -> Self {
        Identity {
            raw_name: operational_dataset.get_network_name().map(ot::NetworkName::to_vec),
            xpanid: operational_dataset.get_extended_pan_id().map(ot::ExtendedPanId::to_vec),
            net_type: Some(fidl_fuchsia_lowpan::NET_TYPE_THREAD_1_X.to_string()),
            channel: operational_dataset.get_channel().map(|x| x as u16),
            panid: operational_dataset.get_pan_id(),
            mesh_local_prefix: operational_dataset
                .get_mesh_local_prefix()
                .copied()
                .map(fidl_fuchsia_net::Ipv6Address::from),
            ..Identity::EMPTY
        }
    }
}

impl FromExt<ot::OperationalDataset> for Identity {
    fn from_ext(f: ot::OperationalDataset) -> Self {
        FromExt::<&ot::OperationalDataset>::from_ext(&f)
    }
}

pub trait UpdateOperationalDataset<T> {
    fn update_from(&mut self, data: &T) -> Result<(), anyhow::Error>;
}

impl UpdateOperationalDataset<ProvisioningParams> for ot::OperationalDataset {
    fn update_from(&mut self, params: &ProvisioningParams) -> Result<(), anyhow::Error> {
        self.update_from(&params.identity)?;
        if let Some(cred) = params.credential.as_ref() {
            self.update_from(cred.as_ref())?
        }
        Ok(())
    }
}

impl UpdateOperationalDataset<Identity> for ot::OperationalDataset {
    fn update_from(&mut self, ident: &Identity) -> Result<(), anyhow::Error> {
        if ident.channel.is_some() {
            self.set_channel(ident.channel.map(|x| x.try_into().unwrap()));
        }
        if ident.panid.is_some() {
            self.set_pan_id(ident.panid)
        }
        if ident.xpanid.is_some() {
            self.set_extended_pan_id(
                ident
                    .xpanid
                    .as_ref()
                    .map(|v| ot::ExtendedPanId::try_ref_from_slice(v.as_slice()))
                    .transpose()?,
            );
        }
        if ident.raw_name.is_some() {
            self.set_network_name(
                ident
                    .raw_name
                    .as_ref()
                    .map(|n| ot::NetworkName::try_from_slice(n.as_slice()))
                    .transpose()?
                    .as_ref(),
            )
        }
        if ident.mesh_local_prefix.is_some() {
            self.set_mesh_local_prefix(
                ident
                    .mesh_local_prefix
                    .clone()
                    .map(|x| std::net::Ipv6Addr::from(x.addr))
                    .map(ot::MeshLocalPrefix::from)
                    .as_ref(),
            )
        }
        Ok(())
    }
}

impl UpdateOperationalDataset<Credential> for ot::OperationalDataset {
    fn update_from(&mut self, cred: &Credential) -> Result<(), anyhow::Error> {
        match cred {
            Credential::MasterKey(key) => {
                self.set_network_key(Some(ot::NetworkKey::try_ref_from_slice(key.as_slice())?))
            }
            _ => Err(format_err!("Unknown credential type"))?,
        }
        Ok(())
    }
}
