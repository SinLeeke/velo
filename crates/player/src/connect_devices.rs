//! Dealer observer installed before Spirc registers the device. No polling or second connection.
use std::sync::Arc;

use librespot_core::{Session, dealer::protocol::Message};
use librespot_protocol::{connect::ClusterUpdate, devices::DeviceType};
use tokio::{sync::watch, task::JoinHandle};
use velo_domain::{Device, Result};

use crate::queue::connect_snapshot::{self, ClusterState};

pub(crate) struct ConnectDevices {
    rows: Arc<watch::Sender<Option<ClusterState>>>,
    task: JoinHandle<()>,
}

impl ConnectDevices {
    /// Subscribe before Spirc registers the device, preserving the initial cluster.
    pub fn observe(session: &Session) -> Option<Self> {
        let mut stream = session
            .dealer()
            .listen_for(
                "hm://connect-state/v1/cluster",
                Message::from_raw::<ClusterUpdate>,
            )
            .ok()?;
        let rows = Arc::new(watch::channel(None).0);
        connect_snapshot::register(session, &rows);
        let latest = rows.clone();
        let local_id = session.device_id().to_owned();
        let task = tokio::spawn(async move {
            while let Some(update) = std::future::poll_fn(|cx| stream.as_mut().poll_next(cx)).await
            {
                if let Ok(update) = update {
                    connect_snapshot::publish_cluster(&latest, &update, &local_id);
                }
            }
            latest.send_replace(None);
        });
        Some(Self { rows, task })
    }

    pub fn snapshot(&self) -> Option<Vec<Device>> {
        self.rows.borrow().as_ref().map(|s| s.devices.clone())
    }
}

impl Drop for ConnectDevices {
    fn drop(&mut self) {
        self.task.abort();
        self.rows.send_replace(None);
    }
}

pub(crate) fn cluster_devices(update: &ClusterUpdate) -> Option<Vec<Device>> {
    let cluster = update.cluster.as_ref()?;
    let mut rows: Vec<_> = cluster
        .device
        .iter()
        .filter_map(|(id, info)| {
            // Offline/observer entries are not playback endpoints; restricted phones remain visible.
            if info.is_offline
                || info.name.is_empty()
                || info.device_type.enum_value_or_default() == DeviceType::OBSERVER
            {
                return None;
            }
            let supports_volume = info
                .capabilities
                .as_ref()
                .is_some_and(|c| !c.disable_volume && c.volume_steps > 0);
            Some(Device {
                id: (!id.is_empty()).then(|| id.clone()),
                name: info.name.clone(),
                kind: match info.device_type.enum_value_or_default() {
                    DeviceType::COMPUTER | DeviceType::CHROMEBOOK => "Computer",
                    DeviceType::SMARTPHONE => "Smartphone",
                    DeviceType::TABLET => "Tablet",
                    DeviceType::SPEAKER => "Speaker",
                    DeviceType::TV => "TV",
                    DeviceType::AVR => "AVR",
                    DeviceType::STB => "STB",
                    DeviceType::AUDIO_DONGLE => "AudioDongle",
                    DeviceType::GAME_CONSOLE => "GameConsole",
                    DeviceType::CAST_VIDEO => "CastVideo",
                    DeviceType::CAST_AUDIO => "CastAudio",
                    _ => "Unknown",
                }
                .into(),
                is_active: !id.is_empty() && *id == cluster.active_device_id,
                volume_percent: supports_volume
                    .then(|| crate::volume_to_percent(info.volume.min(u16::MAX as u32) as u16)),
                supports_volume,
            })
        })
        .collect();
    rows.sort_by(|a, b| {
        b.is_active
            .cmp(&a.is_active)
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.id.cmp(&b.id))
    });
    Some(rows)
}

/// Cluster is authoritative for membership and the active endpoint. Web metadata only fills
/// unknown types/volume; stale Web rows never resurrect an endpoint absent from the cluster.
pub(crate) fn with_connect(
    web: Result<Vec<Device>>,
    connect: Option<Vec<Device>>,
) -> Result<Vec<Device>> {
    let Some(mut rows) = connect else { return web };
    if let Ok(web) = web {
        for row in &mut rows {
            if let Some(metadata) = web.iter().find(|d| row.id.is_some() && d.id == row.id) {
                if row.kind == "Unknown" {
                    row.kind.clone_from(&metadata.kind);
                }
                if row.volume_percent.is_none() && metadata.supports_volume {
                    row.volume_percent = metadata.volume_percent;
                    row.supports_volume = metadata.supports_volume;
                }
            }
        }
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use librespot_protocol::connect::{Capabilities, Cluster, DeviceInfo};
    use protobuf::{EnumOrUnknown, MessageField};
    use velo_domain::VeloError;

    fn update() -> ClusterUpdate {
        let mut cluster = Cluster {
            active_device_id: "phone".into(),
            ..Default::default()
        };
        for (id, name, kind) in [
            ("phone", "Pixel", DeviceType::SMARTPHONE),
            ("pc", "Velo", DeviceType::COMPUTER),
        ] {
            cluster.device.insert(
                id.into(),
                DeviceInfo {
                    name: name.into(),
                    device_type: EnumOrUnknown::new(kind),
                    volume: 32768,
                    capabilities: MessageField::some(Capabilities {
                        volume_steps: 64,
                        ..Default::default()
                    }),
                    disallow_transfer_reasons: vec!["restricted".into()],
                    ..Default::default()
                },
            );
        }
        ClusterUpdate {
            cluster: MessageField::some(cluster),
            ..Default::default()
        }
    }

    #[test]
    fn cluster_keeps_active_smartphone_restrictions_and_optional_volume() {
        let mut update = update();
        update
            .cluster
            .as_mut()
            .unwrap()
            .device
            .get_mut("phone")
            .unwrap()
            .capabilities = MessageField::none();
        let rows = cluster_devices(&update).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(
            (
                rows[0].name.as_str(),
                rows[0].kind.as_str(),
                rows[0].is_active
            ),
            ("Pixel", "Smartphone", true)
        );
        assert_eq!(rows[0].volume_percent, None);
        assert!(!rows[0].supports_volume);
        assert_eq!(rows[1].volume_percent, Some(50));
        assert!(cluster_devices(&ClusterUpdate::default()).is_none());
        update.cluster.as_mut().unwrap().device.clear();
        assert!(cluster_devices(&update).unwrap().is_empty());
    }

    #[test]
    fn connect_fills_empty_or_failed_web_reads_without_duplicating_ids() {
        let rows = cluster_devices(&update()).unwrap();
        for web in [
            Ok(vec![]),
            Err(VeloError::NotAuthenticated),
            Err(VeloError::RateLimited(std::time::Duration::from_secs(30))),
        ] {
            assert_eq!(with_connect(web, Some(rows.clone())).unwrap(), rows);
        }
        assert_eq!(
            with_connect(Ok(rows.clone()), Some(rows.clone())).unwrap(),
            rows
        );
        assert!(
            with_connect(Err(VeloError::NotAuthenticated), Some(vec![]))
                .unwrap()
                .is_empty()
        );
        let mut web = rows.clone();
        web[0].is_active = false;
        assert!(with_connect(Ok(web), Some(rows.clone())).unwrap()[0].is_active);
    }
}
