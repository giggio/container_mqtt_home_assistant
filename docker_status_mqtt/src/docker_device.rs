use async_trait::async_trait;
use bollard::{
    query_parameters::{
        InspectContainerOptionsBuilder, ListContainersOptionsBuilder, LogsOptionsBuilder,
        RemoveContainerOptionsBuilder, RestartContainerOptionsBuilder, StartContainerOptionsBuilder,
        StatsOptionsBuilder, StopContainerOptionsBuilder,
    },
    secret::{ContainerStatsResponse, ContainerSummary},
};
use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use hashbrown::{HashMap, HashSet};
use std::{
    fmt::Debug,
    sync::{Arc, atomic::AtomicBool},
};
use tokio::sync::RwLock;

use crate::{
    cancellation_token::CancellationToken,
    device_manager::CommandResult,
    devices::{
        Button, Device, DeviceDetails, DeviceOrigin, DeviceProvider, Devices, Entity, EntityDetails,
        EntityDetailsGetter, EntityType, Sensor,
    },
    helpers::*,
};

pub type UtcDateTime = DateTime<chrono::Utc>;

const HOST_DEVICE_METADATA: &str = "host_device";

#[cfg(test)]
pub mod docker_client {
    use bollard::{
        container::LogOutput,
        // container::StartContainerOptions,
        errors::Error,
        models::{ContainerInspectResponse, ContainerSummary},
        query_parameters::{
            InspectContainerOptions, ListContainersOptions, LogsOptions, RemoveContainerOptions,
            RestartContainerOptions, StartContainerOptions, StatsOptions, StopContainerOptions,
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
        }

        impl Clone for Docker {
            fn clone(&self) -> Self;
        }
    }

    pub use MockDocker as Docker;
}

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
            let container_name = container
                .names
                .clone()
                .unwrap() // todo: check these unwraps
                .first()
                .unwrap() // todo: check these unwraps
                .trim_start_matches('/')
                .to_string();
            let device_identifier = slugify(&container_name);
            let stats_cache = Arc::new(RwLock::new((
                UtcDateTime::MIN_UTC,
                ContainerStatsResponse::default(),
                UtcDateTime::MIN_UTC,
                ContainerStatsResponse::default(),
            )));
            let should_get_logs = Arc::new(AtomicBool::new(false));
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
                    Box::new(LogText {
                        device_information: Box::new(Sensor::new_simple(
                            device_identifier.clone(),
                            "Logs",
                            "mdi:script-text-outline",
                        )),
                        docker: self.docker.clone(),
                        container_name: container_name.clone(),
                        should_get_logs: should_get_logs.clone(),
                    }),
                    Box::new(GetLogsButton {
                        device_information: Box::new(Button::new(
                            &device_identifier,
                            "Get Logs",
                            "mdi:script-text-play-outline",
                        )),
                        should_get_logs,
                    }),
                    Box::new(UsedMemory {
                        device_information: Box::new(Sensor::new(
                            &device_identifier,
                            "Used Memory",
                            "mdi:memory",
                            "KiB",
                            "data_size",
                        )),
                        docker: self.docker.clone(),
                        container_name: container_name.clone(),
                        stats_cache: stats_cache.clone(),
                    }),
                    Box::new(UsedCPUs {
                        device_information: Box::new(Sensor::new_with_details(
                            EntityDetails::new(&device_identifier, "Used CPUs", "mdi:chip"),
                            Some("%".to_string()),
                            None,
                        )),
                        docker: self.docker.clone(),
                        container_name: container_name.clone(),
                        stats_cache: stats_cache.clone(),
                    }),
                    Box::new(ContainerStatus {
                        device_information: EntityDetails::new_without_icon(&device_identifier, "Status").into(),
                        docker: self.docker.clone(),
                        container_name: container_name.clone(),
                        cancellation_token: cancellation_token.clone(),
                    }),
                    Box::new(RestartButton {
                        device_information: Box::new(Button::new(&device_identifier, "Restart", "mdi:restart")),
                        docker: self.docker.clone(),
                        container_name: container_name.clone(),
                    }),
                    Box::new(StartButton {
                        device_information: Box::new(Button::new(&device_identifier, "Start", "mdi:play")),
                        docker: self.docker.clone(),
                        container_name: container_name.clone(),
                    }),
                    Box::new(StopButton {
                        device_information: Box::new(Button::new(&device_identifier, "Stop", "mdi:stop")),
                        docker: self.docker.clone(),
                        container_name: container_name.clone(),
                    }),
                    Box::new(RemoveButton {
                        device_information: Box::new(Button::new(&device_identifier, "Remove", "mdi:delete")),
                        docker: self.docker.clone(),
                        container_name: container_name.clone(),
                    }),
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
            vec![Box::new(NumberOfContainers {
                device_information: Box::new(Sensor::new_simple(
                    host_identifier.clone(),
                    "Total containers",
                    "mdi:truck-cargo-container",
                )),
                docker: self.docker.clone(),
            })],
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
            .map(|c| {
                let container_name = c
                    .names
                    .unwrap() // todo: check these unwraps
                    .first()
                    .unwrap() // todo: check these unwraps
                    .trim_start_matches('/')
                    .to_string();
                slugify(&container_name)
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
                let container_name = c
                    .names
                    .as_ref()
                    .unwrap() // todo: check these unwraps
                    .first()
                    .unwrap() // todo: check these unwraps
                    .trim_start_matches('/')
                    .to_string();
                let device_identifier = slugify(&container_name);
                !current_devices_id.contains(&device_identifier)
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

#[derive(Debug)]
struct LogText {
    device_information: Box<Sensor>,
    docker: Docker,
    container_name: String,
    should_get_logs: Arc<AtomicBool>,
}
#[async_trait]
impl Entity for LogText {
    fn get_data(&self) -> &dyn EntityType {
        self.device_information.as_ref()
    }
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
        Ok(hashmap! {self.device_information.details().get_topic_for_state(None) => logs})
    }
}

#[derive(Debug)]
struct GetLogsButton {
    device_information: Box<Button>,
    should_get_logs: Arc<AtomicBool>,
}
#[async_trait]
impl Entity for GetLogsButton {
    fn get_data(&self) -> &dyn EntityType {
        self.device_information.as_ref()
    }
    async fn do_handle_command(
        &mut self,
        topic: &str,
        _payload: &str,
        _cancellation_token: CancellationToken,
    ) -> crate::devices::Result<CommandResult> {
        trace!(
            "RestartButton received entity {} event for topic {topic}",
            self.device_information.details().name,
        );
        self.should_get_logs.store(true, std::sync::atomic::Ordering::SeqCst);
        return Ok(CommandResult {
            handled: true,
            state_update_topics: None,
        });
    }
}

#[derive(Debug)]
struct NumberOfContainers {
    device_information: Box<Sensor>,
    docker: Docker,
}
#[async_trait]
impl Entity for NumberOfContainers {
    fn get_data(&self) -> &dyn EntityType {
        self.device_information.as_ref()
    }
    async fn get_entity_data(
        &self,
        _cancellation_token: CancellationToken,
    ) -> crate::devices::Result<HashMap<String, String>> {
        let count = self
            .docker
            .list_containers(Some(ListContainersOptionsBuilder::new().all(true).build()))
            .await?
            .len();
        Ok(hashmap! {self.device_information.details().get_topic_for_state(None) => count.to_string()})
    }
}

#[derive(Debug)]
struct ContainerStatus {
    device_information: Box<Sensor>,
    docker: Docker,
    container_name: String,
    cancellation_token: CancellationToken,
}
#[async_trait]
impl Entity for ContainerStatus {
    fn get_data(&self) -> &dyn EntityType {
        self.device_information.as_ref()
    }
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
        Ok(
            hashmap! {self.device_information.details().get_topic_for_state(None) => inspect.state.unwrap().status.unwrap().to_string()},
        )
    }
}

#[derive(Debug)]
struct UsedCPUs {
    device_information: Box<Sensor>,
    docker: Docker,
    container_name: String,
    stats_cache: Arc<RwLock<(UtcDateTime, ContainerStatsResponse, UtcDateTime, ContainerStatsResponse)>>,
}
#[async_trait]
impl Entity for UsedCPUs {
    fn get_data(&self) -> &dyn EntityType {
        self.device_information.as_ref()
    }
    async fn get_entity_data(
        &self,
        _cancellation_token: CancellationToken,
    ) -> crate::devices::Result<HashMap<String, String>> {
        let stats = get_container_stats(self.stats_cache.clone(), &self.docker, &self.container_name, true).await?;
        if stats.is_none() {
            return Ok(HashMap::new());
        }
        let (stat, previous_stat) = stats.unwrap();
        if let Some(cpu_stats) = stat.cpu_stats
            && let Some(cpu_usage) = cpu_stats.cpu_usage
            && let Some(total_usage) = cpu_usage.total_usage
            && let Some(system_cpu_usage) = cpu_stats.system_cpu_usage
            && let Some(online_cpus) = cpu_stats.online_cpus
            && let Some(previous_cpu_stats) = previous_stat.cpu_stats
            && let Some(previous_cpu_usage) = previous_cpu_stats.cpu_usage
            && let Some(previous_total_usage) = previous_cpu_usage.total_usage
            && let Some(previous_system_cpu_usage) = previous_cpu_stats.system_cpu_usage
        {
            let cpu_delta = total_usage.saturating_sub(previous_total_usage) as f64;
            let system_cpu_delta = system_cpu_usage.saturating_sub(previous_system_cpu_usage) as f64;
            if system_cpu_delta == 0.0 || cpu_delta == 0.0 {
                return Ok(hashmap! {self.device_information.details().get_topic_for_state(None) => "0".to_string()});
            }
            let total_usage = (100.0 * cpu_delta / system_cpu_delta) * online_cpus as f64;
            Ok(hashmap! {self.device_information.details().get_topic_for_state(None) => format!("{total_usage:.2}")})
        } else {
            Err(Error::NoStats.into())
        }
    }
}

#[derive(Debug)]
struct UsedMemory {
    device_information: Box<Sensor>,
    docker: Docker,
    container_name: String,
    stats_cache: Arc<RwLock<(UtcDateTime, ContainerStatsResponse, UtcDateTime, ContainerStatsResponse)>>,
}
#[async_trait]
impl Entity for UsedMemory {
    fn get_data(&self) -> &dyn EntityType {
        self.device_information.as_ref()
    }
    async fn get_entity_data(
        &self,
        _cancellation_token: CancellationToken,
    ) -> crate::devices::Result<HashMap<String, String>> {
        let stats = get_container_stats(self.stats_cache.clone(), &self.docker, &self.container_name, false).await?;
        if stats.is_none() {
            return Ok(HashMap::new());
        }
        let (stat, _) = stats.unwrap();
        let memory_stats = stat.memory_stats;
        if let Some(used_memory) = memory_stats
            && let Some(usage) = used_memory.usage
        {
            Ok(hashmap! {self.device_information.details().get_topic_for_state(None) => (usage / 1024).to_string()})
        } else {
            Err(Error::NoStats.into())
        }
    }
}
async fn get_container_stats(
    stats_cache: Arc<RwLock<(UtcDateTime, ContainerStatsResponse, UtcDateTime, ContainerStatsResponse)>>,
    docker: &Docker,
    container_name: &str,
    delta: bool,
) -> Result<Option<(ContainerStatsResponse, ContainerStatsResponse)>> {
    let stats_cache_guard = stats_cache.read().await;
    let (last_stats_time, cache, previous_stats_time, previous_cache) = &*stats_cache_guard;
    if Utc::now().signed_duration_since(last_stats_time).num_seconds().abs() > 10 {
        let (last_stats_time, cache) = (*last_stats_time, cache.clone());
        drop(stats_cache_guard);
        let stats_stream = docker.stats(
            container_name,
            Some(StatsOptionsBuilder::new().stream(false).one_shot(true).build()),
        );
        let stats = stats_stream.try_collect::<Vec<_>>().await?;
        if stats.is_empty() {
            return Err(Error::NoStats);
        }
        let stat = stats.into_iter().next().unwrap();
        let mut stats_cache_guard = stats_cache.write().await;
        *stats_cache_guard = (Utc::now(), stat.clone(), last_stats_time, cache.clone());
        if delta && last_stats_time == UtcDateTime::MIN_UTC {
            Ok(None)
        } else {
            Ok(Some((stat, cache)))
        }
    } else if delta && *previous_stats_time == UtcDateTime::MIN_UTC {
        Ok(None)
    } else {
        Ok(Some((cache.clone(), previous_cache.clone())))
    }
}

#[derive(Debug)]
struct RestartButton {
    device_information: Box<Button>,
    docker: Docker,
    container_name: String,
}
#[async_trait]
impl Entity for RestartButton {
    fn get_data(&self) -> &dyn EntityType {
        self.device_information.as_ref()
    }
    async fn do_handle_command(
        &mut self,
        topic: &str,
        _payload: &str,
        cancellation_token: CancellationToken,
    ) -> crate::devices::Result<CommandResult> {
        trace!(
            "RestartButton received entity {} event for topic {topic}",
            self.device_information.details().name,
        );
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
    device_information: Box<Button>,
    docker: Docker,
    container_name: String,
}
#[async_trait]
impl Entity for StartButton {
    fn get_data(&self) -> &dyn EntityType {
        self.device_information.as_ref()
    }
    async fn do_handle_command(
        &mut self,
        topic: &str,
        _payload: &str,
        cancellation_token: CancellationToken,
    ) -> crate::devices::Result<CommandResult> {
        trace!(
            "StartButton received entity {} event for topic {topic}",
            self.device_information.details().name,
        );
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
    device_information: Box<Button>,
    docker: Docker,
    container_name: String,
}
#[async_trait]
impl Entity for StopButton {
    fn get_data(&self) -> &dyn EntityType {
        self.device_information.as_ref()
    }
    async fn do_handle_command(
        &mut self,
        topic: &str,
        _payload: &str,
        cancellation_token: CancellationToken,
    ) -> crate::devices::Result<CommandResult> {
        trace!(
            "StopButton received entity {} event for topic {topic}",
            self.device_information.details().name,
        );
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
    device_information: Box<Button>,
    docker: Docker,
    container_name: String,
}
#[async_trait]
impl Entity for RemoveButton {
    fn get_data(&self) -> &dyn EntityType {
        self.device_information.as_ref()
    }
    async fn do_handle_command(
        &mut self,
        topic: &str,
        _payload: &str,
        cancellation_token: CancellationToken,
    ) -> crate::devices::Result<CommandResult> {
        trace!(
            "RemoveButton received entity {} event for topic {topic}",
            self.device_information.details().name,
        );
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

#[cfg(test)]
mod tests {
    use super::*;
    use bollard::{container::LogOutput, models::ContainerSummary, secret::ContainerMemoryStats};
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
            device_information: Box::new(Sensor::new_simple("test_container", "Log", "mdi:script-text-outline")),
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
            device_information: Box::new(Sensor::new_simple(
                "test_container".to_string(),
                "Log".to_string(),
                "mdi:script-text-outline".to_string(),
            )),
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
            device_information: Box::new(Sensor::new_simple(
                "test_container".to_string(),
                "Log".to_string(),
                "mdi:script-text-outline".to_string(),
            )),
            docker: mock_docker,
            container_name: "test_container".to_string(),
            should_get_logs: Arc::new(AtomicBool::new(false)),
        };

        let data = log_text.get_entity_data(CancellationToken::default()).await.unwrap();

        assert_eq!(data.len(), 0);
    }

    #[tokio::test]
    async fn test_number_of_containers_get_entity_data_success() {
        let mut mock_docker = MockDocker::new();

        let containers = vec![
            ContainerSummary::default(),
            ContainerSummary::default(),
            ContainerSummary::default(),
            ContainerSummary::default(),
            ContainerSummary::default(),
        ];

        mock_docker
            .expect_list_containers()
            .returning(move |_| Ok(containers.clone()));

        let number_of_containers = NumberOfContainers {
            device_information: Box::new(Sensor::new_simple(
                "docker_host".to_string(),
                "Total containers".to_string(),
                "mdi:truck-cargo-container".to_string(),
            )),
            docker: mock_docker,
        };

        let data = number_of_containers
            .get_entity_data(CancellationToken::default())
            .await
            .unwrap();

        assert_eq!(data.len(), 1);
        let topic = "docker_host/total_containers/state";
        assert!(data.contains_key(topic));
        assert_eq!(data.get(topic).unwrap(), "5");
    }

    #[tokio::test]
    async fn test_number_of_containers_get_entity_data_zero_containers() {
        let mut mock_docker = MockDocker::new();

        mock_docker.expect_list_containers().returning(|_| Ok(vec![]));

        let number_of_containers = NumberOfContainers {
            device_information: Box::new(Sensor::new_simple(
                "docker_host".to_string(),
                "Total containers".to_string(),
                "mdi:truck-cargo-container".to_string(),
            )),
            docker: mock_docker,
        };

        let data = number_of_containers
            .get_entity_data(CancellationToken::default())
            .await
            .unwrap();

        assert_eq!(data.len(), 1);
        let topic = "docker_host/total_containers/state";
        assert_eq!(data.get(topic).unwrap(), "0");
    }

    #[tokio::test]
    async fn test_used_memory_get_entity_data_success() {
        let mut mock_docker = MockDocker::new();

        mock_docker.expect_stats().returning(|_, _| {
            let stats = ContainerStatsResponse {
                memory_stats: Some(ContainerMemoryStats {
                    usage: Some(1073741824),
                    ..ContainerMemoryStats::default()
                }),
                ..ContainerStatsResponse::default()
            };
            Box::pin(stream::iter(vec![Ok(stats)]))
        });

        let used_memory = UsedMemory {
            device_information: Box::new(Sensor::new_simple(
                "test_container".to_string(),
                "Used Memory".to_string(),
                "mdi:memory".to_string(),
            )),
            docker: mock_docker,
            container_name: "test_container".to_string(),
            stats_cache: Arc::new(RwLock::new((
                UtcDateTime::MIN_UTC,
                ContainerStatsResponse::default(),
                UtcDateTime::MIN_UTC,
                ContainerStatsResponse::default(),
            ))),
        };

        let data = used_memory.get_entity_data(CancellationToken::default()).await.unwrap();

        assert_eq!(data.len(), 1);
        let topic = "test_container/used_memory/state";
        assert!(data.contains_key(topic));
        assert_eq!(data.get(topic).unwrap(), &(1073741824 / 1024).to_string());
    }

    #[tokio::test]
    async fn test_used_memory_get_entity_data_no_stats() {
        let mut mock_docker = MockDocker::new();

        mock_docker
            .expect_stats()
            .returning(|_, _| Box::pin(stream::iter(vec![])));

        let used_memory = UsedMemory {
            device_information: Box::new(Sensor::new_simple(
                "test_container".to_string(),
                "Used Memory".to_string(),
                "mdi:memory".to_string(),
            )),
            docker: mock_docker,
            container_name: "test_container".to_string(),
            stats_cache: Arc::new(RwLock::new((
                UtcDateTime::MIN_UTC,
                ContainerStatsResponse::default(),
                UtcDateTime::MIN_UTC,
                ContainerStatsResponse::default(),
            ))),
        };

        let result = used_memory.get_entity_data(CancellationToken::default()).await;

        assert!(result.is_err());
        let err_string = result.unwrap_err().to_string();
        assert!(err_string.contains("No stats available"));
    }

    #[tokio::test]
    async fn test_used_memory_get_entity_data_no_memory_stats() {
        let mut mock_docker = MockDocker::new();

        mock_docker.expect_stats().returning(|_, _| {
            let stats = ContainerStatsResponse {
                memory_stats: None,
                ..ContainerStatsResponse::default()
            };
            Box::pin(stream::iter(vec![Ok(stats)]))
        });

        let used_memory = UsedMemory {
            device_information: Box::new(Sensor::new_simple(
                "test_container".to_string(),
                "Used Memory".to_string(),
                "mdi:memory".to_string(),
            )),
            docker: mock_docker,
            container_name: "test_container".to_string(),
            stats_cache: Arc::new(RwLock::new((
                UtcDateTime::MIN_UTC,
                ContainerStatsResponse::default(),
                UtcDateTime::MIN_UTC,
                ContainerStatsResponse::default(),
            ))),
        };

        let result = used_memory.get_entity_data(CancellationToken::default()).await;

        assert!(result.is_err());
        let err_string = result.unwrap_err().to_string();
        assert!(err_string.contains("No stats available"));
    }

    #[tokio::test]
    async fn test_used_memory_get_entity_data_no_usage() {
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

        let used_memory = UsedMemory {
            device_information: Box::new(Sensor::new_simple(
                "test_container".to_string(),
                "Used Memory".to_string(),
                "mdi:memory".to_string(),
            )),
            docker: mock_docker,
            container_name: "test_container".to_string(),
            stats_cache: Arc::new(RwLock::new((
                UtcDateTime::MIN_UTC,
                ContainerStatsResponse::default(),
                UtcDateTime::MIN_UTC,
                ContainerStatsResponse::default(),
            ))),
        };

        let result = used_memory.get_entity_data(CancellationToken::default()).await;

        assert!(result.is_err());
        let err_string = result.unwrap_err().to_string();
        assert!(err_string.contains("No stats available"));
    }

    #[tokio::test]
    async fn test_used_memory_get_entity_data_zero_usage() {
        let mut mock_docker = MockDocker::new();

        mock_docker.expect_stats().returning(|_, _| {
            let stats = ContainerStatsResponse {
                memory_stats: Some(ContainerMemoryStats {
                    usage: Some(0),
                    ..ContainerMemoryStats::default()
                }),
                ..ContainerStatsResponse::default()
            };
            Box::pin(stream::iter(vec![Ok(stats)]))
        });

        let used_memory = UsedMemory {
            device_information: Box::new(Sensor::new_simple(
                "test_container".to_string(),
                "Used Memory".to_string(),
                "mdi:memory".to_string(),
            )),
            docker: mock_docker,
            container_name: "test_container".to_string(),
            stats_cache: Arc::new(RwLock::new((
                UtcDateTime::MIN_UTC,
                ContainerStatsResponse::default(),
                UtcDateTime::MIN_UTC,
                ContainerStatsResponse::default(),
            ))),
        };

        let data = used_memory.get_entity_data(CancellationToken::default()).await.unwrap();

        assert_eq!(data.len(), 1);
        let topic = "test_container/used_memory/state";
        assert_eq!(data.get(topic).unwrap(), "0");
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
                    vec![Box::new(NumberOfContainers {
                        device_information: Box::new(Sensor::new_simple(
                            host_identifier.clone(),
                            "Total containers",
                            "mdi:truck-cargo-container",
                        )),
                        docker: MockDocker::default(),
                    })],
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
                    vec![Box::new(LogText {
                        device_information: Box::new(Sensor::new_simple(
                            "container1".to_string(),
                            "Log".to_string(),
                            "mdi:script-text-outline".to_string(),
                        )),
                        docker: MockDocker::default(),
                        container_name: "container1".to_string(),
                        should_get_logs: Arc::new(AtomicBool::new(false)),
                    })],
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
}
