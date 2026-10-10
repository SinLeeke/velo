//! Read-only dealer observer alongside Spirc. No polling or second connection.
use std::sync::{Arc, Mutex};

use librespot_core::{Session, dealer::protocol::Message};
use librespot_protocol::{connect::ClusterUpdate, devices::DeviceType};
use tokio::task::JoinHandle;
use velo_domain::{Device, Result};

pub(crate) struct ConnectDevices {
    rows: Arc<Mutex<Option<Snapshot>>>,
    task: JoinHandle<()>,
}

struct Snapshot {
    timestamp: i64,
    devices: Vec<Device>,
}

impl ConnectDevices {
    /// Subscribe before Spirc registers the device, preserving the initial cluster.
    /// Dealer's SubscriberMap supports multiple observers of the same URI.
    pub fn observe(session: &Session) -> Option<Self> {
        let mut stream = session
            .dealer()
            .listen_for(
                "hm://connect-state/v1/cluster",
                Message::from_raw::<ClusterUpdate>,
            )
            .ok()?;
        let rows = Arc::new(Mutex::new(None::<Snapshot>));
        let latest = rows.clone();
        let task = tokio::spawn(async move {
            while let Some(update) = std::future::poll_fn(|cx| stream.as_mut().poll_next(cx)).await
            {
                if let Ok(update) = update {
                    let next = update.cluster.as_ref().and_then(|cluster| {
                        cluster_devices(&update).map(|devices| Snapshot {
                            timestamp: cluster.changed_timestamp_ms,
                            devices,
                        })
                    });
                    let mut current = latest.lock().unwrap_or_else(|e| e.into_inner());
                    if let (Some(old), Some(new)) = (current.as_ref(), next.as_ref()) {
                        if new.timestamp < old.timestamp {
                            continue;
                        }
                    }
                    *current = next;
                }
            }
            *latest.lock().unwrap_or_else(|e| e.into_inner()) = None;
        });
        Some(Self { rows, task })
    }

    pub fn snapshot(&self) -> Option<Vec<Device>> {
        self.rows
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(|snapshot| snapshot.devices.clone())
    }
}

impl Drop for ConnectDevices {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn cluster_devices(update: &ClusterUpdate) -> Option<Vec<Device>> {
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

/// Web API metadata wins for known IDs; dealer fills missing devices or a failed read.
pub(crate) fn with_connect(
    web: Result<Vec<Device>>,
    connect: Option<Vec<Device>>,
) -> Result<Vec<Device>> {
    match (web, connect) {
        (Ok(mut rows), Some(connect)) => {
            let web_active = rows.iter().any(|d| d.is_active);
            for mut device in connect {
                if let Some(existing) = device
                    .id
                    .as_ref()
                    .and_then(|id| rows.iter_mut().find(|r| r.id.as_ref() == Some(id)))
                {
                    if !web_active {
                        existing.is_active = device.is_active;
                    }
                    continue;
                }
                if web_active {
                    device.is_active = false;
                }
                rows.push(device);
            }
            Ok(rows)
        }
        (Err(_), Some(rows)) if !rows.is_empty() => Ok(rows),
        (result, _) => result,
    }
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
        assert!(with_connect(Err(VeloError::NotAuthenticated), Some(vec![])).is_err());
        let mut web = rows.clone();
        web[0].is_active = false;
        assert!(with_connect(Ok(web), Some(rows.clone())).unwrap()[0].is_active);
    }
}
