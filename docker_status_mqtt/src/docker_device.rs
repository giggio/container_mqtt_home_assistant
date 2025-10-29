use hashbrown::HashMap;
use std::fmt::Debug;
use std::fmt::Display;

use crate::devices::EntityDetailsGetter;
use crate::devices::Sensor;
use crate::{
    cancellation_token::CancellationToken,
    devices::{Device, DeviceDetails, DeviceOrigin, DeviceProvider, Devices, Entity, EntityType},
    helpers::*,
};

use async_trait::async_trait;
use bollard::query_parameters::{ListContainersOptionsBuilder, LogsOptionsBuilder, StatsOptionsBuilder};
use futures::TryStreamExt;

#[cfg(test)]
pub mod docker_client {
    use bollard::{
        container::LogOutput,
        models::ContainerSummary,
        query_parameters::{ListContainersOptions, LogsOptions, StatsOptions},
    };
    use futures::stream::Stream;
    use serde::{Deserialize, Serialize};
    use std::pin::Pin;

    #[derive(Debug, Clone, Default, Serialize, Deserialize)]
    pub struct MemoryStats {
        pub usage: Option<u64>,
    }

    #[derive(Debug, Clone, Default, Serialize, Deserialize)]
    pub struct Stats {
        pub memory_stats: Option<MemoryStats>,
    }

    mockall::mock! {
        #[derive(Debug)]
        pub Docker {
            pub fn connect_with_socket_defaults() -> Result<Self, bollard::errors::Error>;
            pub fn logs(&self, container_name: &str, options: Option<LogsOptions>) -> Pin<Box<dyn Stream<Item = Result<LogOutput, bollard::errors::Error>> + Send + 'static>>;
            pub async fn list_containers(&self, options: Option<ListContainersOptions>) -> Result<Vec<ContainerSummary>, bollard::errors::Error>;
            pub fn stats(&self, container_name: &str, options: Option<StatsOptions>) -> Pin<Box<dyn Stream<Item = Result<Stats, bollard::errors::Error>> + Send + 'static>>;
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
        Ok(DockerDeviceProvider {
            docker: Docker::connect_with_socket_defaults()?,
            provider_name: provider_name.into(),
        })
    }
}

#[async_trait]
impl DeviceProvider for DockerDeviceProvider {
    async fn get_devices(
        &self,
        availability_topic: String,
        cancellation_token: CancellationToken,
    ) -> crate::devices::Result<Devices> {
        let builder = ListContainersOptionsBuilder::new().all(true);
        let containers = self.docker.list_containers(Some(builder.build())).await?;
        let mut device_vec = Vec::new();
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
            cancellation_token.clone(),
        );
        device_vec.push(host_device);
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
                            "Log",
                            "mdi:script-text-outline",
                        )),
                        docker: self.docker.clone(),
                        container_name: container_name.clone(),
                    }),
                    Box::new(UsedMemory {
                        device_information: Box::new(Sensor::new(
                            device_identifier,
                            "Used Memory",
                            "mdi:memory",
                            "KiB",
                            "data_size",
                        )),
                        docker: self.docker.clone(),
                        container_name: container_name.clone(),
                    }),
                ],
                cancellation_token.clone(),
            );
            device_vec.push(container_device);
        }
        let devices = Devices::new_from_many_devices(device_vec, cancellation_token);
        Ok(devices)
    }
}

#[derive(Debug)]
struct LogText {
    device_information: Box<Sensor>,
    docker: Docker,
    container_name: String,
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
struct UsedMemory {
    device_information: Box<Sensor>,
    docker: Docker,
    container_name: String,
}
impl Display for UsedMemory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "UsedMemoryDataProvider")
    }
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
        let stats_stream = self.docker.stats(
            &self.container_name,
            Some(StatsOptionsBuilder::new().stream(false).one_shot(true).build()),
        );
        let stats = stats_stream.try_collect::<Vec<_>>().await?;
        if stats.is_empty() {
            return Err(Error::NoStats.into());
        }
        let memory_stats = &stats.first().unwrap().memory_stats;
        if memory_stats.is_none() {
            return Err(Error::NoStats.into());
        }
        let used_memory = memory_stats.as_ref().unwrap();
        if used_memory.usage.is_none() {
            return Err(Error::NoStats.into());
        }
        let usage = used_memory.usage.unwrap() / 1024;
        Ok(hashmap! {self.device_information.details().get_topic_for_state(None) => usage.to_string()})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bollard::{container::LogOutput, models::ContainerSummary};
    use docker_client::{MockDocker, Stats};
    use futures::stream;
    use pretty_assertions::assert_eq;

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
        };

        let data = log_text.get_entity_data(CancellationToken::default()).await.unwrap();

        assert_eq!(data.len(), 1);
        let topic = "test_container/log/state";
        assert_eq!(data.get(topic).unwrap(), "");
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
            let stats = Stats {
                memory_stats: Some(docker_client::MemoryStats {
                    usage: Some(1073741824),
                }),
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
            let stats = Stats { memory_stats: None };
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
            let stats = Stats {
                memory_stats: Some(docker_client::MemoryStats { usage: None }),
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
            let stats = Stats {
                memory_stats: Some(docker_client::MemoryStats { usage: Some(0) }),
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
        };

        let data = used_memory.get_entity_data(CancellationToken::default()).await.unwrap();

        assert_eq!(data.len(), 1);
        let topic = "test_container/used_memory/state";
        assert_eq!(data.get(topic).unwrap(), "0");
    }
}
