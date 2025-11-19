use async_trait::async_trait;
use bollard::{
    query_parameters::{
        DataUsageOptions, InspectContainerOptionsBuilder, ListContainersOptionsBuilder, ListNetworksOptions,
        LogsOptionsBuilder, RemoveContainerOptionsBuilder, RestartContainerOptionsBuilder,
        StartContainerOptionsBuilder, StatsOptionsBuilder, StopContainerOptionsBuilder,
    },
    secret::{
        ContainerCpuStats, ContainerStatsResponse, ContainerSummary, ContainerSummaryStateEnum, HealthStatusEnum,
    },
};
use chrono::Duration;
use futures::TryStreamExt;
use hashbrown::{HashMap, HashSet};
use serde_json::json;
use std::{
    fmt::Debug,
    sync::{Arc, atomic::AtomicBool},
};
use tokio::sync::RwLock;

use crate::{
    cancellation_token::CancellationToken,
    device_manager::CommandResult,
    devices::{
        Button, Device, DeviceDetails, DeviceOrigin, DeviceProvider, Devices, EntityDetails, EntityDetailsGetter,
        HandlesData, Sensor,
    },
    helpers::*,
};

const HOST_DEVICE_METADATA: &str = "host_device";

#[cfg(not(test))]
pub mod docker_client {
    pub use bollard::Docker;
}

use docker_client::Docker;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error(transparent)]
    Docker(#[from] bollard::errors::Error),
    #[error("No stats available for the container")]
    NoStats,
    #[allow(dead_code)]
    #[error("Unknown error")]
    Unknown,
}

impl From<Error> for crate::devices::Error {
    fn from(val: Error) -> Self {
        crate::devices::Error::DeviceProvider(val.to_string())
    }
}

impl From<bollard::errors::Error> for crate::devices::Error {
    fn from(e: bollard::errors::Error) -> Self {
        crate::devices::Error::DeviceProvider(e.to_string())
    }
}

pub struct DockerDeviceProvider {
    docker: Docker,
    provider_name: String,
}

impl DockerDeviceProvider {
    pub fn new(provider_name: impl Into<String>) -> Result<Self> {
        Ok(Self {
            docker: Docker::connect_with_socket_defaults()?,
            provider_name: provider_name.into(),
        })
    }

    fn create_container_devices(
        &self,
        containers: Vec<ContainerSummary>,
        host_identifier: String,
        availability_topic: String,
        cancellation_token: CancellationToken,
    ) -> Vec<Device> {
        let mut device_vec = Vec::new();
        for container in containers.into_iter() {
            let container_name = if let Some(name) = container.names.as_ref().and_then(|n| n.first()) {
                name.trim_start_matches('/').to_string()
            } else {
                warn!(
                    "Container {} has no name, skipping",
                    container.id.as_deref().unwrap_or_default()
                );
                continue;
            };
            let device_identifier = slugify(&container_name);
            let should_get_logs = Arc::new(AtomicBool::new(false));
            let log_text = Box::new(Sensor::new_simple(
                device_identifier.clone(),
                "Logs",
                "mdi:script-text-outline",
            ));
            let log_text_data = Box::new(LogText {
                state_topic: log_text.details().get_topic_for_state(None),
                docker: self.docker.clone(),
                container_name: container_name.clone(),
                should_get_logs: should_get_logs.clone(),
            });
            let get_logs_button = Box::new(Button::new(
                &device_identifier,
                "Get Logs",
                "mdi:script-text-play-outline",
            ));
            let get_logs_button_data = Box::new(GetLogsButton {
                command_topic: get_logs_button.details().get_topic_for_command(None),
                should_get_logs,
            });
            let used_memory = Box::new(Sensor::new(
                &device_identifier,
                "Used Memory",
                "mdi:memory",
                "KiB",
                "data_size",
            ));
            let used_cpus = Box::new(Sensor::new_with_details(
                EntityDetails::new(&device_identifier, "Used CPUs", "mdi:chip"),
                Some("%".to_string()),
                None,
            ));
            let container_stats_data = Box::new(ContainerStats {
                used_cpu_state_topic: used_cpus.details().get_topic_for_state(None),
                used_memory_state_topic: used_memory.details().get_topic_for_state(None),
                docker: self.docker.clone(),
                container_name: container_name.clone(),
                last_cpu_stats: Arc::new(RwLock::new(None)),
            });
            let restart_button =
                Box::new(Button::new(&device_identifier, "Restart", "mdi:restart").can_be_made_unavailable());
            let restart_button_data = Box::new(RestartButton {
                command_topic: restart_button.details().get_topic_for_command(None),
                docker: self.docker.clone(),
                container_name: container_name.clone(),
            });
            let start_button = Box::new(Button::new(&device_identifier, "Start", "mdi:play").can_be_made_unavailable());
            let start_button_data = Box::new(StartButton {
                command_topic: start_button.details().get_topic_for_command(None),
                docker: self.docker.clone(),
                container_name: container_name.clone(),
            });
            let stop_button = Box::new(Button::new(&device_identifier, "Stop", "mdi:stop").can_be_made_unavailable());
            let stop_button_data = Box::new(StopButton {
                command_topic: stop_button.details().get_topic_for_command(None),
                docker: self.docker.clone(),
                container_name: container_name.clone(),
            });
            let remove_button = Box::new(Button::new(&device_identifier, "Remove", "mdi:delete"));
            let remove_button_data = Box::new(RemoveButton {
                command_topic: remove_button.details().get_topic_for_command(None),
                docker: self.docker.clone(),
                container_name: container_name.clone(),
            });
            let container_health: Box<Sensor> =
                EntityDetails::new(&device_identifier, "Health", "mdi:medication-outline").into();
            let container_status: Box<Sensor> = EntityDetails::new_without_icon(&device_identifier, "Status").into();
            let container_status_data = Box::new(ContainerStatus {
                state_topic: container_status.details().get_topic_for_state(None),
                health_topic: container_health.details().get_topic_for_state(None),
                start_button_availability_topic: start_button.details().get_topic_for_availability(None),
                restart_button_availability_topic: restart_button.details().get_topic_for_availability(None),
                stop_button_availability_topic: stop_button.details().get_topic_for_availability(None),
                docker: self.docker.clone(),
                container_name: container_name.clone(),
                cancellation_token: cancellation_token.clone(),
            });
            let container_image: Box<Sensor> = EntityDetails::new(&device_identifier, "Image", "mdi:oci").into();
            let container_static_data = Box::new(ContainerStaticData {
                image: (
                    container_image.details().get_topic_for_state(None),
                    container.image.unwrap_or_default(),
                ),
            });
            let container_device = Device::new_with_entities(
                DeviceDetails {
                    name: container_name.clone(),
                    identifier: device_identifier.clone(),
                    manufacturer: "Giovanni Bassi".to_string(),
                    sw_version: env!("CARGO_PKG_VERSION").to_string(),
                    via_device: Some(host_identifier.clone()),
                },
                DeviceOrigin {
                    name: "docker-status-mqtt".to_string(),
                    sw: env!("CARGO_PKG_VERSION").to_string(),
                    url: "https://github.com/giggio/docker-status-mqtt".to_string(),
                },
                availability_topic.clone(),
                vec![
                    log_text,
                    get_logs_button,
                    used_memory,
                    used_cpus,
                    container_status,
                    container_image,
                    restart_button,
                    start_button,
                    stop_button,
                    remove_button,
                    container_health,
                ],
                vec![
                    log_text_data,
                    get_logs_button_data,
                    container_stats_data,
                    container_status_data,
                    restart_button_data,
                    start_button_data,
                    stop_button_data,
                    remove_button_data,
                    container_static_data,
                ],
                self.id(),
                cancellation_token.clone(),
            );
            device_vec.push(container_device);
        }
        device_vec
    }
}

#[async_trait]
impl DeviceProvider for DockerDeviceProvider {
    fn id(&self) -> String {
        self.provider_name.clone()
    }
    async fn get_devices(
        &self,
        availability_topic: String,
        cancellation_token: CancellationToken,
    ) -> crate::devices::Result<Devices> {
        let containers = self
            .docker
            .list_containers(Some(ListContainersOptionsBuilder::new().all(true).build()))
            .await?;
        // todo: improve device for the docker host
        let host_identifier = slugify(&self.provider_name);
        // let number_of_containers = Box::new(Sensor::new_simple(
        //     host_identifier.clone(),
        //     "Total containers",
        //     "mdi:truck-cargo-container",
        // ));
        // let number_of_containers_data = Box::new(NumberOfContainers {
        //     state_topic: number_of_containers.details().get_topic_for_state(None),
        //     docker: self.docker.clone(),
        // });
        let host_version = Box::new(Sensor::new_simple(
            host_identifier.clone(),
            "Host version",
            "mdi:docker",
        ));
        let host_version_data = Box::new(HostVersion {
            state_topic: host_version.details().get_topic_for_state(None),
            docker: self.docker.clone(),
        });
        let images = Box::new(Sensor::new_with_details(
            EntityDetails::new(host_identifier.clone(), "Images", "mdi:oci").has_attributes(),
            None,
            None,
        ));
        let volumes = Box::new(Sensor::new_with_details(
            EntityDetails::new(host_identifier.clone(), "Volumes", "mdi:harddisk").has_attributes(),
            None,
            None,
        ));
        let build_cache = Box::new(Sensor::new_with_details(
            EntityDetails::new(host_identifier.clone(), "Build cache", "mdi:sync-circle").has_attributes(),
            None,
            None,
        ));
        let number_of_containers = Box::new(Sensor::new_with_details(
            EntityDetails::new(host_identifier.clone(), "Total containers", "mdi:truck-cargo-container")
                .has_attributes(),
            None,
            None,
        ));
        let df_data = Box::new(
            DiskFree {
                images_state_topic: images.details().get_topic_for_state(None),
                images_attributes_state_topic: images.details().get_topic_for_state(Some("attributes")),
                volumes_state_topic: volumes.details().get_topic_for_state(None),
                volumes_attributes_state_topic: volumes.details().get_topic_for_state(Some("attributes")),
                build_cache_state_topic: build_cache.details().get_topic_for_state(None),
                build_cache_attributes_state_topic: build_cache.details().get_topic_for_state(Some("attributes")),
                number_of_containers_state_topic: number_of_containers.details().get_topic_for_state(None),
                number_of_containers_attributes_state_topic: number_of_containers
                    .details()
                    .get_topic_for_state(Some("attributes")),
                docker: self.docker.clone(),
            }
            .debounce(Duration::hours(1)),
        );
        let network_info = Box::new(Sensor::new_with_details(
            EntityDetails::new(host_identifier.clone(), "Networks", "mdi:network").has_attributes(),
            None,
            None,
        ));
        let network_info_data = Box::new(
            NetworkInfo {
                state_topic: network_info.details().get_topic_for_state(None),
                state_attributes_topic: network_info.details().get_topic_for_state(Some("attributes")),
                docker: self.docker.clone(),
            }
            .debounce(Duration::hours(1)),
        );
        let host_device = Device::new_with_entities(
            DeviceDetails {
                name: self.provider_name.clone(),
                identifier: host_identifier.clone(),
                manufacturer: "Giovanni Bassi".to_string(),
                sw_version: env!("CARGO_PKG_VERSION").to_string(),
                via_device: None,
            },
            DeviceOrigin {
                name: "docker-status-mqtt".to_string(),
                sw: env!("CARGO_PKG_VERSION").to_string(),
                url: "https://github.com/giggio/docker-status-mqtt".to_string(),
            },
            availability_topic.clone(),
            vec![
                number_of_containers,
                host_version,
                images,
                volumes,
                build_cache,
                network_info,
            ],
            vec![host_version_data, df_data, network_info_data],
            self.id(),
            cancellation_token.clone(),
        )
        .with_metadata(vec![Box::new(HOST_DEVICE_METADATA.to_string())]);
        let mut devices_vec = self.create_container_devices(
            containers,
            host_identifier,
            availability_topic,
            cancellation_token.clone(),
        );
        devices_vec.push(host_device);
        let devices = Devices::new_from_many_devices(devices_vec, cancellation_token);
        Ok(devices)
    }

    async fn remove_missing_devices(
        &self,
        devices: &Devices,
        cancellation_token: CancellationToken,
    ) -> crate::devices::Result<Vec<Arc<RwLock<Device>>>> {
        let containers = cancellation_token
            .wait_on(
                self.docker
                    .list_containers(Some(ListContainersOptionsBuilder::new().all(true).build())),
            )
            .await??
            .into_iter()
            .filter_map(|c| {
                c.names
                    .as_ref()
                    .and_then(|names| names.first())
                    .map(|name| slugify(name.trim_start_matches('/')))
            })
            .collect::<Vec<String>>();
        let devices_ids_to_remove = devices
            .iter()
            .await
            .async_map(|device_lock| async move {
                let device = device_lock.read().await;
                (device.details.identifier.clone(), device.get_metadata::<String>())
            })
            .await
            .into_iter()
            .filter(|(id, metadata)| !containers.contains(id) && !metadata.contains(&HOST_DEVICE_METADATA.to_string()))
            .map(|(id, _)| id)
            .collect::<HashSet<String>>();
        let devices_removed = devices.remove_devices(&devices_ids_to_remove).await?;
        Ok(devices_removed)
    }

    async fn add_discovered_devices(
        &self,
        devices: &Devices,
        availability_topic: String,
        cancellation_token: CancellationToken,
    ) -> crate::devices::Result<HashSet<String>> {
        let containers = cancellation_token
            .wait_on(
                self.docker
                    .list_containers(Some(ListContainersOptionsBuilder::new().all(true).build())),
            )
            .await??;
        let current_devices_id = devices.identifiers().await;
        let new_containers = containers
            .into_iter()
            .filter(|c| {
                c.names
                    .as_ref()
                    .and_then(|names| names.first())
                    .map(|name| {
                        let device_identifier = slugify(name.trim_start_matches('/'));
                        !current_devices_id.contains(&device_identifier)
                    })
                    .unwrap_or(false)
            })
            .collect::<Vec<ContainerSummary>>();
        if new_containers.is_empty() {
            return Ok(HashSet::new());
        }
        let host_identifier = slugify(&self.provider_name);
        let devices_to_add = self.create_container_devices(
            new_containers,
            host_identifier,
            availability_topic,
            cancellation_token.clone(),
        );
        let ids = devices_to_add
            .iter()
            .map(|d| d.details.identifier.clone())
            .collect::<HashSet<String>>();
        devices.add_devices(devices_to_add).await?;
        Ok(ids)
    }
}

#[derive(Clone, Debug)]
struct LogText {
    state_topic: String,
    docker: Docker,
    container_name: String,
    should_get_logs: Arc<AtomicBool>,
}
#[async_trait]
impl HandlesData for LogText {
    async fn get_entity_data(
        &self,
        _cancellation_token: CancellationToken,
    ) -> crate::devices::Result<HashMap<String, String>> {
        if !self.should_get_logs.load(std::sync::atomic::Ordering::SeqCst) {
            return Ok(HashMap::new());
        }
        self.should_get_logs.store(false, std::sync::atomic::Ordering::SeqCst);
        let logs = self.docker.logs(
            &self.container_name,
            Some(LogsOptionsBuilder::new().stdout(true).stderr(true).tail("3").build()),
        );
        let mut logs: String = logs
            .try_collect::<Vec<_>>()
            .await?
            .into_iter()
            .map(|log| log.to_string())
            .collect::<Vec<String>>()
            .join("\n");
        logs.truncate(255);
        Ok(hashmap! {self.state_topic.clone() => logs})
    }
}

#[derive(Debug)]
struct GetLogsButton {
    command_topic: String,
    should_get_logs: Arc<AtomicBool>,
}
#[async_trait]
impl HandlesData for GetLogsButton {
    fn get_command_topics(&self) -> Vec<&str> {
        vec![&self.command_topic]
    }
    async fn do_handle_command(
        &mut self,
        topic: &str,
        _payload: &str,
        _cancellation_token: CancellationToken,
    ) -> crate::devices::Result<CommandResult> {
        trace!("RestartButton received event for topic {topic}");
        self.should_get_logs.store(true, std::sync::atomic::Ordering::SeqCst);
        return Ok(CommandResult {
            handled: true,
            state_update_topics: None,
        });
    }
}

#[derive(Debug)]
struct NetworkInfo {
    state_topic: String,
    docker: Docker,
    state_attributes_topic: String,
}
#[async_trait]
impl HandlesData for NetworkInfo {
    async fn get_entity_data(
        &self,
        cancellation_token: CancellationToken,
    ) -> crate::devices::Result<HashMap<String, String>> {
        let networks = cancellation_token
            .wait_on(self.docker.list_networks(None::<ListNetworksOptions>))
            .await??;
        let network_count = networks.len();
        Ok(hashmap! {
            self.state_topic.clone() => network_count.to_string(),
            self.state_attributes_topic.clone() => json!({
                "networks": networks.iter().filter_map(|n| {
                    let name = n.name.clone().unwrap_or_else(|| n.id.clone().unwrap_or_default());
                    if name.is_empty() {
                        None
                    } else {
                        Some(name)
                    }
                }).collect::<Vec<String>>(),
            }).to_string(),

        })
    }
}

#[derive(Debug)]
struct HostVersion {
    state_topic: String,
    docker: Docker,
}
#[async_trait]
impl HandlesData for HostVersion {
    async fn get_entity_data(
        &self,
        cancellation_token: CancellationToken,
    ) -> crate::devices::Result<HashMap<String, String>> {
        let info = cancellation_token.wait_on(self.docker.info()).await??;
        if let Some(version) = info.server_version {
            Ok(hashmap! {self.state_topic.clone() => version})
        } else {
            Ok(HashMap::new())
        }
    }
}

#[derive(Debug)]
struct DiskFree {
    docker: Docker,
    images_state_topic: String,
    images_attributes_state_topic: String,
    volumes_state_topic: String,
    volumes_attributes_state_topic: String,
    build_cache_state_topic: String,
    build_cache_attributes_state_topic: String,
    number_of_containers_state_topic: String,
    number_of_containers_attributes_state_topic: String,
}
#[async_trait]
impl HandlesData for DiskFree {
    async fn get_entity_data(
        &self,
        cancellation_token: CancellationToken,
    ) -> crate::devices::Result<HashMap<String, String>> {
        let data_usage = cancellation_token
            .wait_on(self.docker.df(None::<DataUsageOptions>))
            .await??;
        if let Some(image_summary) = data_usage.images
            && let Some(volume_summary) = data_usage.volumes
            && let Some(build_cache_summary) = data_usage.build_cache
            && let Some(containers_summary) = data_usage.containers
        {
            let mut images_active_count = 0;
            let mut image_size = 0;
            let mut image_reclaimable = 0;
            let mut number_of_images = 0;
            for img in image_summary {
                number_of_images += 1;
                image_size += img.size;
                if img.containers > 0 {
                    images_active_count += 1;
                } else {
                    image_reclaimable += img.size;
                }
            }

            let mut volumes_active_count = 0;
            let mut volume_size = 0i64;
            let mut volume_reclaimable = 0i64;
            let mut number_of_volumes = 0;
            for vol in volume_summary {
                number_of_volumes += 1;
                if let Some(usage_data) = &vol.usage_data {
                    volume_size += usage_data.size;
                    if usage_data.ref_count > 0 {
                        volumes_active_count += 1;
                    } else {
                        volume_reclaimable += usage_data.size;
                    }
                }
            }

            let mut build_cache_active_count = 0;
            let mut build_cache_size = 0;
            let mut build_cache_reclaimable = 0;
            let mut number_of_build_caches = 0;
            for bc in build_cache_summary {
                number_of_build_caches += 1;
                let size = bc.size.unwrap_or(0);
                build_cache_size += size;
                if bc.in_use.unwrap_or(false) {
                    build_cache_active_count += 1;
                } else {
                    build_cache_reclaimable += size;
                }
            }

            let mut containers_active_count = 0;
            let mut number_of_containers = 0;
            let mut containers_reclaimable = 0;
            let mut container_size = 0;
            for container in containers_summary {
                number_of_containers += 1;
                let size = container.size_rw.unwrap_or(0);
                container_size += size;
                if let Some(state) = &container.state
                    && matches!(
                        state,
                        ContainerSummaryStateEnum::RUNNING
                            | ContainerSummaryStateEnum::PAUSED
                            | ContainerSummaryStateEnum::RESTARTING
                    )
                {
                    containers_active_count += 1;
                } else {
                    containers_reclaimable += size;
                }
            }

            let data = hashmap! {
                self.images_state_topic.clone() => number_of_images.to_string(),
                self.images_attributes_state_topic.clone() => json!({
                    "images_active": images_active_count,
                    "images_size_kib": (image_size / 1024),
                    "images_reclaimable_kib": (image_reclaimable / 1024),
                }).to_string(),
                self.volumes_state_topic.clone() => number_of_volumes.to_string(),
                self.volumes_attributes_state_topic.clone() => json!({
                    "volumes_active": volumes_active_count,
                    "volumes_size_kib": (volume_size / 1024),
                    "volumes_reclaimable_kib": (volume_reclaimable / 1024),
                }).to_string(),
                self.build_cache_state_topic.clone() => number_of_build_caches.to_string(),
                self.build_cache_attributes_state_topic.clone() => json!({
                    "build_caches_active": build_cache_active_count,
                    "build_caches_size_kib": (build_cache_size / 1024),
                    "build_caches_reclaimable_kib": (build_cache_reclaimable / 1024),
                }).to_string(),
                self.number_of_containers_state_topic.clone() => number_of_containers.to_string(),
                self.number_of_containers_attributes_state_topic.clone() => json!({
                    "containers_active": containers_active_count,
                    "containers_size_kib": (container_size / 1024),
                    "containers_reclaimable_kib": (containers_reclaimable / 1024),
                }).to_string(),
            };
            trace!("Got df data from Docker: {data:?}");
            Ok(data)
        } else {
            Ok(HashMap::new())
        }
    }
}

// #[derive(Debug)]
// struct NumberOfContainers {
//     state_topic: String,
//     docker: Docker,
// }
// #[async_trait]
// impl HandlesData for NumberOfContainers {
//     async fn get_entity_data(
//         &self,
//         _cancellation_token: CancellationToken,
//     ) -> crate::devices::Result<HashMap<String, String>> {
//         let count = self
//             .docker
//             .list_containers(Some(ListContainersOptionsBuilder::new().all(true).build()))
//             .await?
//             .len();
//         Ok(hashmap! {self.state_topic.clone() => count.to_string()})
//     }
// }

#[derive(Debug)]
struct ContainerStatus {
    state_topic: String,
    docker: Docker,
    container_name: String,
    cancellation_token: CancellationToken,
    start_button_availability_topic: String,
    restart_button_availability_topic: String,
    stop_button_availability_topic: String,
    health_topic: String,
}
#[async_trait]
impl HandlesData for ContainerStatus {
    async fn get_entity_data(
        &self,
        _cancellation_token: CancellationToken,
    ) -> crate::devices::Result<HashMap<String, String>> {
        let inspect = self
            .cancellation_token
            .wait_on(self.docker.inspect_container(
                &self.container_name,
                Some(InspectContainerOptionsBuilder::new().build()),
            ))
            .await??;

        let state = if let Some(s) = inspect.state {
            s
        } else {
            warn!(
                "Container {} has no state information, skipping status update.",
                self.container_name
            );
            return Ok(HashMap::new()); // No state, so no status to report
        };

        let is_running = [state.running, state.paused, state.restarting]
            .into_iter()
            .any(|s| s == Some(true));

        let mut map = hashmap! {
            self.start_button_availability_topic.clone() => if is_running { "offline" } else { "online" }.to_string(),
            self.restart_button_availability_topic.clone() => if is_running { "online" } else { "offline" }.to_string(),
            self.stop_button_availability_topic.clone() => if is_running { "online" } else { "offline" }.to_string(),
        };

        if let Some(status) = state.status {
            map.insert(self.state_topic.clone(), status.to_string());
        } else {
            warn!(
                "Container {} has no status, skipping status topic update.",
                self.container_name
            );
        }

        let health = state
            .health
            .unwrap_or_default()
            .status
            .unwrap_or(HealthStatusEnum::EMPTY);

        if !matches!(health, HealthStatusEnum::EMPTY) {
            map.insert(self.health_topic.clone(), health.to_string());
        }
        Ok(map)
    }
}

#[derive(Debug)]
struct ContainerStats {
    used_memory_state_topic: String,
    used_cpu_state_topic: String,
    docker: Docker,
    container_name: String,
    last_cpu_stats: Arc<RwLock<Option<ContainerCpuStats>>>,
}
impl ContainerStats {
    async fn get_cpu(&self, stat: &ContainerStatsResponse) -> Result<Option<String>> {
        if let Some(cpu_stats) = &stat.cpu_stats {
            let mut last_cpu_stats_guard = self.last_cpu_stats.write().await;
            let last_cpu_stats_option = (*last_cpu_stats_guard).take();
            *last_cpu_stats_guard = Some(cpu_stats.clone());
            drop(last_cpu_stats_guard);
            if let Some(last_cpu_stats) = last_cpu_stats_option {
                if let Some(cpu_usage) = &cpu_stats.cpu_usage
                    && let Some(total_usage) = cpu_usage.total_usage
                    && let Some(system_cpu_usage) = cpu_stats.system_cpu_usage
                    && let Some(online_cpus) = cpu_stats.online_cpus
                    && let Some(last_cpu_usage) = last_cpu_stats.cpu_usage
                    && let Some(last_total_usage) = last_cpu_usage.total_usage
                    && let Some(last_system_cpu_usage) = last_cpu_stats.system_cpu_usage
                {
                    let cpu_delta = total_usage.saturating_sub(last_total_usage) as f64;
                    let system_cpu_delta = system_cpu_usage.saturating_sub(last_system_cpu_usage) as f64;
                    if system_cpu_delta == 0.0 || cpu_delta == 0.0 {
                        return Ok(Some("0".to_string()));
                    }
                    let total_usage = (100.0 * cpu_delta / system_cpu_delta) * online_cpus as f64;
                    Ok(Some(format!("{total_usage:.2}")))
                } else {
                    Err(Error::NoStats)
                }
            } else {
                Ok(None)
            }
        } else {
            Err(Error::NoStats)
        }
    }
    fn get_memory(&self, stat: &ContainerStatsResponse) -> Result<String> {
        if let Some(used_memory) = &stat.memory_stats
            && let Some(usage) = used_memory.usage
        {
            Ok((usage / 1024).to_string())
        } else {
            Err(Error::NoStats)
        }
    }
}
#[async_trait]
impl HandlesData for ContainerStats {
    async fn get_entity_data(
        &self,
        cancellation_token: CancellationToken,
    ) -> crate::devices::Result<HashMap<String, String>> {
        let inspect = cancellation_token
            .wait_on(self.docker.inspect_container(
                &self.container_name,
                Some(InspectContainerOptionsBuilder::new().build()),
            ))
            .await??;

        if !inspect.state.and_then(|s| s.running).unwrap_or(false) {
            *self.last_cpu_stats.write().await = None;
            return Ok(hashmap! {
                self.used_memory_state_topic.clone() => "".to_string(),
                self.used_cpu_state_topic.clone() => "".to_string()
            });
        }
        let stats_stream = self.docker.stats(
            &self.container_name,
            Some(StatsOptionsBuilder::new().stream(false).one_shot(true).build()),
        );
        let stats = stats_stream.try_collect::<Vec<_>>().await?;
        if let Some(stat) = stats.into_iter().next() {
            let used_memory = self.get_memory(&stat)?;
            let used_cpu = self.get_cpu(&stat).await?;
            Ok(hashmap! {
                self.used_memory_state_topic.clone() => used_memory,
                self.used_cpu_state_topic.clone() => used_cpu.unwrap_or("".to_string())
            })
        } else {
            Err(Error::NoStats.into())
        }
    }
}

#[derive(Debug)]
struct RestartButton {
    command_topic: String,
    docker: Docker,
    container_name: String,
}
#[async_trait]
impl HandlesData for RestartButton {
    fn get_command_topics(&self) -> Vec<&str> {
        vec![&self.command_topic]
    }
    async fn do_handle_command(
        &mut self,
        topic: &str,
        _payload: &str,
        cancellation_token: CancellationToken,
    ) -> crate::devices::Result<CommandResult> {
        trace!("RestartButton received event for topic {topic}");
        cancellation_token
            .wait_on(self.docker.restart_container(
                &self.container_name,
                Some(RestartContainerOptionsBuilder::new().t(5).build()),
            ))
            .await??;
        return Ok(CommandResult {
            handled: true,
            state_update_topics: None,
        });
    }
}

#[derive(Debug)]
struct StartButton {
    command_topic: String,
    docker: Docker,
    container_name: String,
}
#[async_trait]
impl HandlesData for StartButton {
    fn get_command_topics(&self) -> Vec<&str> {
        vec![&self.command_topic]
    }
    async fn do_handle_command(
        &mut self,
        topic: &str,
        _payload: &str,
        cancellation_token: CancellationToken,
    ) -> crate::devices::Result<CommandResult> {
        trace!("StartButton received event for topic {topic}");
        cancellation_token
            .wait_on(
                self.docker
                    .start_container(&self.container_name, Some(StartContainerOptionsBuilder::new().build())),
            )
            .await??;
        return Ok(CommandResult {
            handled: true,
            state_update_topics: None,
        });
    }
}

#[derive(Debug)]
struct StopButton {
    command_topic: String,
    docker: Docker,
    container_name: String,
}
#[async_trait]
impl HandlesData for StopButton {
    fn get_command_topics(&self) -> Vec<&str> {
        vec![&self.command_topic]
    }
    async fn do_handle_command(
        &mut self,
        topic: &str,
        _payload: &str,
        cancellation_token: CancellationToken,
    ) -> crate::devices::Result<CommandResult> {
        trace!("StopButton received event for topic {topic}");
        cancellation_token
            .wait_on(self.docker.stop_container(
                &self.container_name,
                Some(StopContainerOptionsBuilder::new().t(5).build()),
            ))
            .await??;
        return Ok(CommandResult {
            handled: true,
            state_update_topics: None,
        });
    }
}

#[derive(Debug)]
struct RemoveButton {
    command_topic: String,
    docker: Docker,
    container_name: String,
}
#[async_trait]
impl HandlesData for RemoveButton {
    fn get_command_topics(&self) -> Vec<&str> {
        vec![&self.command_topic]
    }
    async fn do_handle_command(
        &mut self,
        topic: &str,
        _payload: &str,
        cancellation_token: CancellationToken,
    ) -> crate::devices::Result<CommandResult> {
        trace!("RemoveButton received event for topic {topic}");
        cancellation_token
            .wait_on(self.docker.stop_container(
                &self.container_name,
                Some(StopContainerOptionsBuilder::new().t(5).build()),
            ))
            .await??;
        cancellation_token
            .wait_on(
                self.docker
                    .remove_container(&self.container_name, Some(RemoveContainerOptionsBuilder::new().build())),
            )
            .await??;
        return Ok(CommandResult {
            handled: true,
            state_update_topics: None,
        });
    }
}

#[derive(Debug)]
struct ContainerStaticData {
    image: (String, String),
}
#[async_trait]
impl HandlesData for ContainerStaticData {
    async fn get_entity_data(&self, _: CancellationToken) -> crate::devices::Result<HashMap<String, String>> {
        Ok(hashmap! {
            self.image.0.clone() => self.image.1.clone()
        })
    }
}

#[cfg(test)]
pub mod docker_client {
    use bollard::{
        container::LogOutput,
        errors::Error,
        models::{ContainerInspectResponse, ContainerSummary, Network, SystemDataUsageResponse, SystemInfo},
        query_parameters::{
            DataUsageOptions, InspectContainerOptions, ListContainersOptions, ListNetworksOptions, LogsOptions,
            RemoveContainerOptions, RestartContainerOptions, StartContainerOptions, StatsOptions, StopContainerOptions,
        },
        secret::ContainerStatsResponse,
    };
    use futures::stream::Stream;
    use std::pin::Pin;

    mockall::mock! {
        #[derive(Debug)]
        pub Docker {
            pub fn connect_with_socket_defaults() -> std::result::Result<Self, bollard::errors::Error>;
            pub fn logs(&self, container_name: &str, options: Option<LogsOptions>) -> Pin<Box<dyn Stream<Item = std::result::Result<LogOutput, bollard::errors::Error>> + Send + 'static>>;
            pub async fn list_containers(&self, options: Option<ListContainersOptions>) -> std::result::Result<Vec<ContainerSummary>, bollard::errors::Error>;
            pub fn stats(&self, container_name: &str, options: Option<StatsOptions>) -> Pin<Box<dyn Stream<Item = std::result::Result<ContainerStatsResponse, bollard::errors::Error>> + Send + 'static>>;
            pub async fn restart_container(&self, container_name: &str, options: Option<RestartContainerOptions>) -> std::result::Result<(), Error>;
            pub async fn start_container(&self, container_name: &str, options: Option<StartContainerOptions>) -> std::result::Result<(), Error>;
            pub async fn stop_container(&self, container_name: &str, options: Option<StopContainerOptions>) -> std::result::Result<(), Error>;
            pub async fn remove_container(&self, container_name: &str, options: Option<RemoveContainerOptions>) -> std::result::Result<(), Error>;
            pub async fn inspect_container(&self, container_name: &str, options: Option<InspectContainerOptions>) -> std::result::Result<ContainerInspectResponse, Error>;
            pub async fn info(&self) -> std::result::Result<SystemInfo, Error>;
            pub async fn df(&self, options: Option<DataUsageOptions>) -> std::result::Result<SystemDataUsageResponse, Error>;
            pub async fn list_networks( &self, options: Option<ListNetworksOptions>) -> std::result::Result<Vec<Network>, Error>;
        }

        impl Clone for Docker {
            fn clone(&self) -> Self;
        }
    }

    pub use MockDocker as Docker;
}

#[cfg(test)]
mod tests {
    use super::*;
    use bollard::{
        container::LogOutput,
        models::{ContainerStateStatusEnum, ContainerSummary, SystemDataUsageResponse, SystemInfo},
        secret::{
            ContainerCpuStats, ContainerCpuUsage, ContainerInspectResponse, ContainerMemoryStats,
            ContainerSummaryStateEnum, HealthStatusEnum,
        },
    };
    use docker_client::MockDocker;
    use futures::stream;
    use pretty_assertions::assert_eq;
    use serial_test::serial;

    #[tokio::test]
    async fn test_log_text_get_entity_data_success() {
        let mut mock_docker = MockDocker::new();

        mock_docker.expect_logs().returning(|_, _| {
            Box::pin(stream::iter(vec![
                Ok(LogOutput::StdOut {
                    message: "2024-10-31 10:00:00 Starting container".into(),
                }),
                Ok(LogOutput::StdOut {
                    message: "2024-10-31 10:00:01 Container ready".into(),
                }),
                Ok(LogOutput::StdOut {
                    message: "2024-10-31 10:00:02 Processing request".into(),
                }),
            ]))
        });

        let log_text = LogText {
            state_topic: "test_container/log/state".to_string(),
            docker: mock_docker,
            container_name: "test_container".to_string(),
            should_get_logs: Arc::new(AtomicBool::new(true)),
        };

        let data = log_text.get_entity_data(CancellationToken::default()).await.unwrap();

        assert_eq!(data.len(), 1);
        let topic = "test_container/log/state";
        assert!(data.contains_key(topic));
        let log_content = data.get(topic).unwrap();
        assert!(log_content.contains("Starting container"));
        assert!(log_content.contains("Container ready"));
        assert!(log_content.contains("Processing request"));
    }

    #[tokio::test]
    async fn test_log_text_get_entity_data_empty_logs() {
        let mut mock_docker = MockDocker::new();

        mock_docker
            .expect_logs()
            .returning(|_, _| Box::pin(stream::iter(vec![])));

        let log_text = LogText {
            state_topic: "test_container/log/state".to_string(),
            docker: mock_docker,
            container_name: "test_container".to_string(),
            should_get_logs: Arc::new(AtomicBool::new(true)),
        };

        let data = log_text.get_entity_data(CancellationToken::default()).await.unwrap();

        assert_eq!(data.len(), 1);
        let topic = "test_container/log/state";
        assert_eq!(data.get(topic).unwrap(), "");
    }

    #[tokio::test]
    async fn test_log_text_get_entity_data_no_logs_when_shouldnt_get_logs() {
        let mut mock_docker = MockDocker::new();

        mock_docker
            .expect_logs()
            .returning(|_, _| Box::pin(stream::iter(vec![])));

        let log_text = LogText {
            state_topic: "test_container/log/state".to_string(),
            docker: mock_docker,
            container_name: "test_container".to_string(),
            should_get_logs: Arc::new(AtomicBool::new(false)),
        };

        let data = log_text.get_entity_data(CancellationToken::default()).await.unwrap();

        assert_eq!(data.len(), 0);
    }

    // #[tokio::test]
    // async fn test_number_of_containers_get_entity_data_success() {
    //     let mut mock_docker = MockDocker::new();
    //
    //     let containers = vec![
    //         ContainerSummary::default(),
    //         ContainerSummary::default(),
    //         ContainerSummary::default(),
    //         ContainerSummary::default(),
    //         ContainerSummary::default(),
    //     ];
    //
    //     mock_docker
    //         .expect_list_containers()
    //         .returning(move |_| Ok(containers.clone()));
    //
    //     let number_of_containers = NumberOfContainers {
    //         state_topic: "docker_host/total_containers/state".to_string(),
    //         docker: mock_docker,
    //     };
    //
    //     let data = number_of_containers
    //         .get_entity_data(CancellationToken::default())
    //         .await
    //         .unwrap();
    //
    //     assert_eq!(data.len(), 1);
    //     let topic = "docker_host/total_containers/state";
    //     assert!(data.contains_key(topic));
    //     assert_eq!(data.get(topic).unwrap(), "5");
    // }

    // #[tokio::test]
    // async fn test_number_of_containers_get_entity_data_zero_containers() {
    //     let mut mock_docker = MockDocker::new();
    //
    //     mock_docker.expect_list_containers().returning(|_| Ok(vec![]));
    //
    //     let number_of_containers = NumberOfContainers {
    //         state_topic: "docker_host/total_containers/state".to_string(),
    //         docker: mock_docker,
    //     };
    //
    //     let data = number_of_containers
    //         .get_entity_data(CancellationToken::default())
    //         .await
    //         .unwrap();
    //
    //     assert_eq!(data.len(), 1);
    //     let topic = "docker_host/total_containers/state";
    //     assert_eq!(data.get(topic).unwrap(), "0");
    // }

    #[tokio::test]
    async fn test_container_stats_get_entity_data_success_for_memory_cpu_empty() {
        let mut mock_docker = MockDocker::new();

        mock_docker.expect_stats().returning(|_, _| {
            let stats = ContainerStatsResponse {
                memory_stats: Some(ContainerMemoryStats {
                    usage: Some(1073741824),
                    ..ContainerMemoryStats::default()
                }),
                cpu_stats: Some(ContainerCpuStats {
                    cpu_usage: Some(ContainerCpuUsage {
                        total_usage: Some(4000000000),
                        ..ContainerCpuUsage::default()
                    }),
                    system_cpu_usage: Some(30000000000),
                    online_cpus: Some(2),
                    ..ContainerCpuStats::default()
                }),
                ..ContainerStatsResponse::default()
            };
            Box::pin(stream::iter(vec![Ok(stats)]))
        });
        mock_docker.expect_inspect_container().returning(|_, _| {
            let inspect = ContainerInspectResponse {
                state: Some(bollard::models::ContainerState {
                    running: Some(true),
                    ..bollard::models::ContainerState::default()
                }),
                ..ContainerInspectResponse::default()
            };
            Ok(inspect)
        });

        let container_stats = ContainerStats {
            used_memory_state_topic: "test_container/used_memory/state".to_string(),
            used_cpu_state_topic: "test_container/used_cpu/state".to_string(),
            docker: mock_docker,
            container_name: "test_container".to_string(),
            last_cpu_stats: Arc::new(RwLock::new(None)),
        };

        let data = container_stats
            .get_entity_data(CancellationToken::default())
            .await
            .unwrap();

        assert_eq!(data.len(), 2);
        let memory_topic = "test_container/used_memory/state";
        assert!(data.contains_key(memory_topic));
        assert_eq!(data.get(memory_topic).unwrap(), &(1073741824 / 1024).to_string());
        let cpu_topic = "test_container/used_cpu/state";
        assert!(data.contains_key(cpu_topic));
        assert_eq!(data.get(cpu_topic).unwrap(), "");
    }

    #[tokio::test]
    async fn test_container_stats_get_entity_data_success_for_memory_and_cpu() {
        let mut mock_docker = MockDocker::new();

        mock_docker.expect_stats().returning(|_, _| {
            let stats = ContainerStatsResponse {
                memory_stats: Some(ContainerMemoryStats {
                    usage: Some(1073741824),
                    ..ContainerMemoryStats::default()
                }),
                cpu_stats: Some(ContainerCpuStats {
                    cpu_usage: Some(ContainerCpuUsage {
                        total_usage: Some(4000000000),
                        ..ContainerCpuUsage::default()
                    }),
                    system_cpu_usage: Some(30000000000),
                    online_cpus: Some(2),
                    ..ContainerCpuStats::default()
                }),
                ..ContainerStatsResponse::default()
            };
            Box::pin(stream::iter(vec![Ok(stats)]))
        });
        mock_docker.expect_inspect_container().returning(|_, _| {
            let inspect = ContainerInspectResponse {
                state: Some(bollard::models::ContainerState {
                    running: Some(true),
                    ..bollard::models::ContainerState::default()
                }),
                ..ContainerInspectResponse::default()
            };
            Ok(inspect)
        });

        let container_stats = ContainerStats {
            used_memory_state_topic: "test_container/used_memory/state".to_string(),
            used_cpu_state_topic: "test_container/used_cpu/state".to_string(),
            docker: mock_docker,
            container_name: "test_container".to_string(),
            last_cpu_stats: Arc::new(RwLock::new(Some(ContainerCpuStats {
                cpu_usage: Some(ContainerCpuUsage {
                    total_usage: Some(1000000000),
                    ..ContainerCpuUsage::default()
                }),
                system_cpu_usage: Some(10000000000),
                online_cpus: Some(2),
                ..ContainerCpuStats::default()
            }))),
        };

        let data = container_stats
            .get_entity_data(CancellationToken::default())
            .await
            .unwrap();

        assert_eq!(data.len(), 2);
        assert_eq!(
            data.get("test_container/used_memory/state").unwrap(),
            &(1073741824 / 1024).to_string()
        );
        assert_eq!(data.get("test_container/used_cpu/state").unwrap(), "30.00");
    }

    #[tokio::test]
    async fn test_container_stats_get_entity_data_no_stats() {
        let mut mock_docker = MockDocker::new();

        mock_docker
            .expect_stats()
            .returning(|_, _| Box::pin(stream::iter(vec![])));
        mock_docker.expect_inspect_container().returning(|_, _| {
            let inspect = ContainerInspectResponse {
                state: Some(bollard::models::ContainerState {
                    running: Some(true),
                    ..bollard::models::ContainerState::default()
                }),
                ..ContainerInspectResponse::default()
            };
            Ok(inspect)
        });

        let container_stats = ContainerStats {
            used_memory_state_topic: "test_container/used_memory/state".to_string(),
            used_cpu_state_topic: "test_container/used_cpu/state".to_string(),
            docker: mock_docker,
            container_name: "test_container".to_string(),
            last_cpu_stats: Arc::new(RwLock::new(None)),
        };

        let result = container_stats.get_entity_data(CancellationToken::default()).await;

        assert!(result.is_err());
        let err_string = result.unwrap_err().to_string();
        assert!(err_string.contains("No stats available"));
    }

    #[tokio::test]
    async fn test_container_stats_get_entity_data_no_memory_stats() {
        let mut mock_docker = MockDocker::new();

        mock_docker.expect_stats().returning(|_, _| {
            let stats = ContainerStatsResponse {
                memory_stats: None,
                ..ContainerStatsResponse::default()
            };
            Box::pin(stream::iter(vec![Ok(stats)]))
        });
        mock_docker.expect_inspect_container().returning(|_, _| {
            let inspect = ContainerInspectResponse {
                state: Some(bollard::models::ContainerState {
                    running: Some(true),
                    ..bollard::models::ContainerState::default()
                }),
                ..ContainerInspectResponse::default()
            };
            Ok(inspect)
        });

        let container_stats = ContainerStats {
            used_memory_state_topic: "test_container/used_memory/state".to_string(),
            used_cpu_state_topic: "test_container/used_cpu/state".to_string(),
            docker: mock_docker,
            container_name: "test_container".to_string(),
            last_cpu_stats: Arc::new(RwLock::new(None)),
        };

        let result = container_stats.get_entity_data(CancellationToken::default()).await;

        assert!(result.is_err());
        let err_string = result.unwrap_err().to_string();
        assert!(err_string.contains("No stats available"));
    }

    #[tokio::test]
    async fn test_container_stats_get_entity_data_no_usage() {
        let mut mock_docker = MockDocker::new();

        mock_docker.expect_stats().returning(|_, _| {
            let stats = ContainerStatsResponse {
                memory_stats: Some(ContainerMemoryStats {
                    usage: None,
                    ..ContainerMemoryStats::default()
                }),
                ..ContainerStatsResponse::default()
            };
            Box::pin(stream::iter(vec![Ok(stats)]))
        });
        mock_docker.expect_inspect_container().returning(|_, _| {
            let inspect = ContainerInspectResponse {
                state: Some(bollard::models::ContainerState {
                    running: Some(true),
                    ..bollard::models::ContainerState::default()
                }),
                ..ContainerInspectResponse::default()
            };
            Ok(inspect)
        });

        let container_stats = ContainerStats {
            used_memory_state_topic: "test_container/used_memory/state".to_string(),
            used_cpu_state_topic: "test_container/used_cpu/state".to_string(),
            docker: mock_docker,
            container_name: "test_container".to_string(),
            last_cpu_stats: Arc::new(RwLock::new(None)),
        };

        let result = container_stats.get_entity_data(CancellationToken::default()).await;

        assert!(result.is_err());
        let err_string = result.unwrap_err().to_string();
        assert!(err_string.contains("No stats available"));
    }

    #[tokio::test]
    async fn test_container_stats_get_entity_data_zero_usage() {
        let mut mock_docker = MockDocker::new();

        mock_docker.expect_stats().returning(|_, _| {
            let stats = ContainerStatsResponse {
                memory_stats: Some(ContainerMemoryStats {
                    usage: Some(0),
                    ..ContainerMemoryStats::default()
                }),
                cpu_stats: Some(ContainerCpuStats {
                    cpu_usage: Some(ContainerCpuUsage {
                        total_usage: Some(4000000000),
                        ..ContainerCpuUsage::default()
                    }),
                    system_cpu_usage: Some(30000000000),
                    online_cpus: Some(2),
                    ..ContainerCpuStats::default()
                }),
                ..ContainerStatsResponse::default()
            };
            Box::pin(stream::iter(vec![Ok(stats)]))
        });
        mock_docker.expect_inspect_container().returning(|_, _| {
            let inspect = ContainerInspectResponse {
                state: Some(bollard::models::ContainerState {
                    running: Some(true),
                    ..bollard::models::ContainerState::default()
                }),
                ..ContainerInspectResponse::default()
            };
            Ok(inspect)
        });

        let container_stats = ContainerStats {
            used_memory_state_topic: "test_container/used_memory/state".to_string(),
            used_cpu_state_topic: "test_container/used_cpu/state".to_string(),
            docker: mock_docker,
            container_name: "test_container".to_string(),
            last_cpu_stats: Arc::new(RwLock::new(None)),
        };

        let data = container_stats
            .get_entity_data(CancellationToken::default())
            .await
            .unwrap();

        assert_eq!(data.len(), 2);
        assert_eq!(data.get("test_container/used_memory/state").unwrap(), "0");
        assert_eq!(data.get("test_container/used_cpu/state").unwrap(), "");
    }

    #[tokio::test]
    #[serial]
    async fn test_removed_devices() {
        let c = MockDocker::connect_with_socket_defaults_context();
        c.expect().return_once(|| {
            let mut docker = MockDocker::default();
            docker
                .expect_list_containers()
                .returning(move |_| {
                    Ok(vec![ContainerSummary {
                        names: Some(vec!["/container1".to_string()]),
                        ..ContainerSummary::default()
                    }])
                })
                .times(1);
            Ok(docker)
        });
        let provider_name = "My Machine";
        let host_identifier = slugify(provider_name);
        let existing_devices = Devices::new_from_many_devices(
            vec![
                Device::new_with_entities(
                    DeviceDetails {
                        name: provider_name.to_string(),
                        identifier: host_identifier.clone(),
                        manufacturer: "Giovanni Bassi".to_string(),
                        sw_version: env!("CARGO_PKG_VERSION").to_string(),
                        via_device: None,
                    },
                    DeviceOrigin {
                        name: "docker-status-mqtt".to_string(),
                        sw: env!("CARGO_PKG_VERSION").to_string(),
                        url: "https://github.com/giggio/docker-status-mqtt".to_string(),
                    },
                    "availability/topic".to_string(),
                    vec![Box::new(Sensor::new_simple(
                        host_identifier.clone(),
                        "Total containers",
                        "mdi:truck-cargo-container",
                    ))],
                    vec![],
                    "docker_device_provider".to_string(),
                    CancellationToken::default(),
                )
                .with_metadata(vec![Box::new(HOST_DEVICE_METADATA.to_string())]),
                Device::new_with_entities(
                    DeviceDetails {
                        name: "container1".to_string(),
                        identifier: "container1".to_string(),
                        manufacturer: "Giovanni Bassi".to_string(),
                        sw_version: env!("CARGO_PKG_VERSION").to_string(),
                        via_device: None,
                    },
                    DeviceOrigin {
                        name: "docker-status-mqtt".to_string(),
                        sw: env!("CARGO_PKG_VERSION").to_string(),
                        url: "x".to_string(),
                    },
                    "availability/topic".to_string(),
                    vec![Box::new(Sensor::new_simple(
                        "container1".to_string(),
                        "Log".to_string(),
                        "mdi:script-text-outline".to_string(),
                    ))],
                    vec![],
                    "docker_device_provider".to_string(),
                    CancellationToken::default(),
                ),
            ],
            CancellationToken::default(),
        );
        let device_provider = DockerDeviceProvider::new(provider_name).unwrap();
        let removed = device_provider
            .remove_missing_devices(&existing_devices, CancellationToken::default())
            .await
            .unwrap();
        assert_eq!(0, removed.len());
    }

    #[tokio::test]
    async fn test_disk_free_get_entity_data_success() {
        let mut mock_docker = MockDocker::new();

        mock_docker.expect_df().returning(|_| {
            Ok(SystemDataUsageResponse {
                images: Some(vec![
                    bollard::models::ImageSummary {
                        size: 1024 * 1024, // 1MB
                        containers: 1,
                        ..Default::default()
                    },
                    bollard::models::ImageSummary {
                        size: 2048 * 1024, // 2MB
                        containers: 0,
                        ..Default::default()
                    },
                ]),
                volumes: Some(vec![
                    bollard::models::Volume {
                        usage_data: Some(bollard::models::VolumeUsageData {
                            size: 4096 * 1024, // 4MB
                            ref_count: 1,
                        }),
                        ..Default::default()
                    },
                    bollard::models::Volume {
                        usage_data: Some(bollard::models::VolumeUsageData {
                            size: 8192 * 1024, // 8MB
                            ref_count: 0,
                        }),
                        ..Default::default()
                    },
                ]),
                build_cache: Some(vec![
                    bollard::models::BuildCache {
                        size: Some(16384 * 1024), // 16MB
                        in_use: Some(true),
                        ..Default::default()
                    },
                    bollard::models::BuildCache {
                        size: Some(32768 * 1024), // 32MB
                        in_use: Some(false),
                        ..Default::default()
                    },
                ]),
                containers: Some(vec![
                    ContainerSummary {
                        size_rw: Some(65536 * 1024), // 64MB
                        state: Some(ContainerSummaryStateEnum::RUNNING),
                        ..Default::default()
                    },
                    ContainerSummary {
                        size_rw: Some(131072 * 1024), // 128MB
                        state: Some(ContainerSummaryStateEnum::EXITED),
                        ..Default::default()
                    },
                ]),
                ..Default::default()
            })
        });

        let disk_free = DiskFree {
            docker: mock_docker,
            images_state_topic: "images/state".to_string(),
            images_attributes_state_topic: "images/attributes".to_string(),
            volumes_state_topic: "volumes/state".to_string(),
            volumes_attributes_state_topic: "volumes/attributes".to_string(),
            build_cache_state_topic: "build_cache/state".to_string(),
            build_cache_attributes_state_topic: "build_cache/attributes".to_string(),
            number_of_containers_state_topic: "containers/state".to_string(),
            number_of_containers_attributes_state_topic: "containers/attributes".to_string(),
        };

        let data = disk_free.get_entity_data(CancellationToken::default()).await.unwrap();

        assert_eq!(data.get("images/state").unwrap(), "2");
        let images_attrs: serde_json::Value = serde_json::from_str(data.get("images/attributes").unwrap()).unwrap();
        assert_eq!(images_attrs["images_active"], 1);
        assert_eq!(images_attrs["images_size_kib"], (3072 * 1024 / 1024));
        assert_eq!(images_attrs["images_reclaimable_kib"], (2048 * 1024 / 1024));

        assert_eq!(data.get("volumes/state").unwrap(), "2");
        let volumes_attrs: serde_json::Value = serde_json::from_str(data.get("volumes/attributes").unwrap()).unwrap();
        assert_eq!(volumes_attrs["volumes_active"], 1);
        assert_eq!(volumes_attrs["volumes_size_kib"], (12288 * 1024 / 1024));
        assert_eq!(volumes_attrs["volumes_reclaimable_kib"], (8192 * 1024 / 1024));

        assert_eq!(data.get("build_cache/state").unwrap(), "2");
        let build_cache_attrs: serde_json::Value =
            serde_json::from_str(data.get("build_cache/attributes").unwrap()).unwrap();
        assert_eq!(build_cache_attrs["build_caches_active"], 1);
        assert_eq!(build_cache_attrs["build_caches_size_kib"], (49152 * 1024 / 1024));
        assert_eq!(build_cache_attrs["build_caches_reclaimable_kib"], (32768 * 1024 / 1024));

        assert_eq!(data.get("containers/state").unwrap(), "2");
        let containers_attrs: serde_json::Value =
            serde_json::from_str(data.get("containers/attributes").unwrap()).unwrap();
        assert_eq!(containers_attrs["containers_active"], 1);
        assert_eq!(containers_attrs["containers_size_kib"], (196608 * 1024 / 1024));
        assert_eq!(containers_attrs["containers_reclaimable_kib"], (131072 * 1024 / 1024));
    }

    #[tokio::test]
    async fn test_disk_free_get_entity_data_empty() {
        let mut mock_docker = MockDocker::new();

        mock_docker
            .expect_df()
            .returning(|_| Ok(SystemDataUsageResponse::default()));

        let disk_free = DiskFree {
            docker: mock_docker,
            images_state_topic: "images/state".to_string(),
            images_attributes_state_topic: "images/attributes".to_string(),
            volumes_state_topic: "volumes/state".to_string(),
            volumes_attributes_state_topic: "volumes/attributes".to_string(),
            build_cache_state_topic: "build_cache/state".to_string(),
            build_cache_attributes_state_topic: "build_cache/attributes".to_string(),
            number_of_containers_state_topic: "containers/state".to_string(),
            number_of_containers_attributes_state_topic: "containers/attributes".to_string(),
        };

        let data = disk_free.get_entity_data(CancellationToken::default()).await.unwrap();

        assert!(data.is_empty());
    }

    #[tokio::test]
    async fn test_container_status_get_entity_data_running() {
        let mut mock_docker = MockDocker::new();

        mock_docker.expect_inspect_container().returning(|_, _| {
            Ok(ContainerInspectResponse {
                state: Some(bollard::models::ContainerState {
                    running: Some(true),
                    status: Some(ContainerStateStatusEnum::RUNNING),
                    health: Some(bollard::models::Health {
                        status: Some(HealthStatusEnum::HEALTHY),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            })
        });

        let container_status = ContainerStatus {
            state_topic: "status/state".to_string(),
            docker: mock_docker,
            container_name: "test_container".to_string(),
            cancellation_token: CancellationToken::default(),
            start_button_availability_topic: "start/avail".to_string(),
            restart_button_availability_topic: "restart/avail".to_string(),
            stop_button_availability_topic: "stop/avail".to_string(),
            health_topic: "health/state".to_string(),
        };

        let data = container_status
            .get_entity_data(CancellationToken::default())
            .await
            .unwrap();

        assert_eq!(data.get("status/state").unwrap(), "running");
        assert_eq!(data.get("health/state").unwrap(), "healthy");
        assert_eq!(data.get("start/avail").unwrap(), "offline");
        assert_eq!(data.get("restart/avail").unwrap(), "online");
        assert_eq!(data.get("stop/avail").unwrap(), "online");
    }

    #[tokio::test]
    async fn test_container_status_get_entity_data_stopped() {
        let mut mock_docker = MockDocker::new();

        mock_docker.expect_inspect_container().returning(|_, _| {
            Ok(ContainerInspectResponse {
                state: Some(bollard::models::ContainerState {
                    running: Some(false),
                    status: Some(ContainerStateStatusEnum::EXITED),
                    ..Default::default()
                }),
                ..Default::default()
            })
        });

        let container_status = ContainerStatus {
            state_topic: "status/state".to_string(),
            docker: mock_docker,
            container_name: "test_container".to_string(),
            cancellation_token: CancellationToken::default(),
            start_button_availability_topic: "start/avail".to_string(),
            restart_button_availability_topic: "restart/avail".to_string(),
            stop_button_availability_topic: "stop/avail".to_string(),
            health_topic: "health/state".to_string(),
        };

        let data = container_status
            .get_entity_data(CancellationToken::default())
            .await
            .unwrap();

        assert_eq!(data.get("status/state").unwrap(), "exited");
        assert!(!data.contains_key("health/state"));
        assert_eq!(data.get("start/avail").unwrap(), "online");
        assert_eq!(data.get("restart/avail").unwrap(), "offline");
        assert_eq!(data.get("stop/avail").unwrap(), "offline");
    }

    #[tokio::test]
    async fn test_network_info_get_entity_data_success() {
        let mut mock_docker = MockDocker::new();

        mock_docker.expect_list_networks().returning(|_| {
            Ok(vec![
                bollard::models::Network {
                    name: Some("bridge".to_string()),
                    ..Default::default()
                },
                bollard::models::Network {
                    name: Some("host".to_string()),
                    ..Default::default()
                },
            ])
        });

        let network_info = NetworkInfo {
            state_topic: "network/state".to_string(),
            state_attributes_topic: "network/attributes".to_string(),
            docker: mock_docker,
        };

        let data = network_info
            .get_entity_data(CancellationToken::default())
            .await
            .unwrap();

        assert_eq!(data.get("network/state").unwrap(), "2");
        let attrs: serde_json::Value = serde_json::from_str(data.get("network/attributes").unwrap()).unwrap();
        let networks = attrs["networks"].as_array().unwrap();
        assert_eq!(networks.len(), 2);
        assert!(networks.contains(&serde_json::Value::String("bridge".to_string())));
        assert!(networks.contains(&serde_json::Value::String("host".to_string())));
    }

    #[tokio::test]
    async fn test_host_version_get_entity_data_success() {
        let mut mock_docker = MockDocker::new();

        mock_docker.expect_info().returning(|| {
            Ok(SystemInfo {
                server_version: Some("20.10.7".to_string()),
                ..Default::default()
            })
        });

        let host_version = HostVersion {
            state_topic: "version/state".to_string(),
            docker: mock_docker,
        };

        let data = host_version
            .get_entity_data(CancellationToken::default())
            .await
            .unwrap();

        assert_eq!(data.get("version/state").unwrap(), "20.10.7");
    }

    #[tokio::test]
    async fn test_restart_button_handle_command() {
        let mut mock_docker = MockDocker::new();

        mock_docker
            .expect_restart_container()
            .with(mockall::predicate::eq("test_container"), mockall::predicate::always())
            .returning(|_, _| Ok(()));

        let mut restart_button = RestartButton {
            command_topic: "restart/command".to_string(),
            docker: mock_docker,
            container_name: "test_container".to_string(),
        };

        let result = restart_button
            .do_handle_command("restart/command", "PRESS", CancellationToken::default())
            .await
            .unwrap();

        assert!(result.handled);
    }

    #[tokio::test]
    async fn test_start_button_handle_command() {
        let mut mock_docker = MockDocker::new();

        mock_docker
            .expect_start_container()
            .with(mockall::predicate::eq("test_container"), mockall::predicate::always())
            .returning(|_, _| Ok(()));

        let mut start_button = StartButton {
            command_topic: "start/command".to_string(),
            docker: mock_docker,
            container_name: "test_container".to_string(),
        };

        let result = start_button
            .do_handle_command("start/command", "PRESS", CancellationToken::default())
            .await
            .unwrap();

        assert!(result.handled);
    }

    #[tokio::test]
    async fn test_stop_button_handle_command() {
        let mut mock_docker = MockDocker::new();

        mock_docker
            .expect_stop_container()
            .with(mockall::predicate::eq("test_container"), mockall::predicate::always())
            .returning(|_, _| Ok(()));

        let mut stop_button = StopButton {
            command_topic: "stop/command".to_string(),
            docker: mock_docker,
            container_name: "test_container".to_string(),
        };

        let result = stop_button
            .do_handle_command("stop/command", "PRESS", CancellationToken::default())
            .await
            .unwrap();

        assert!(result.handled);
    }

    #[tokio::test]
    async fn test_remove_button_handle_command() {
        let mut mock_docker = MockDocker::new();

        mock_docker
            .expect_stop_container()
            .with(mockall::predicate::eq("test_container"), mockall::predicate::always())
            .returning(|_, _| Ok(()));

        mock_docker
            .expect_remove_container()
            .with(mockall::predicate::eq("test_container"), mockall::predicate::always())
            .returning(|_, _| Ok(()));

        let mut remove_button = RemoveButton {
            command_topic: "remove/command".to_string(),
            docker: mock_docker,
            container_name: "test_container".to_string(),
        };

        let result = remove_button
            .do_handle_command("remove/command", "PRESS", CancellationToken::default())
            .await
            .unwrap();

        assert!(result.handled);
    }

    #[tokio::test]
    async fn test_get_logs_button_handle_command() {
        let should_get_logs = Arc::new(AtomicBool::new(false));
        let mut get_logs_button = GetLogsButton {
            command_topic: "logs/command".to_string(),
            should_get_logs: should_get_logs.clone(),
        };

        let result = get_logs_button
            .do_handle_command("logs/command", "PRESS", CancellationToken::default())
            .await
            .unwrap();

        assert!(result.handled);
        assert!(should_get_logs.load(std::sync::atomic::Ordering::SeqCst));
    }
}
