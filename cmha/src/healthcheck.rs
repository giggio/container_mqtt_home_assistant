use crate::shared_memory::SharedMemory;
pub type Result<T> = std::result::Result<T, Error>;
static LINK_FILE_NAME: &str = "/dev/shm/cmha_shmem_flink";
#[derive(Copy, Clone, Default)]
enum HealthCheckStatus {
    Healthy,
    #[default]
    Unhealthy,
}

pub struct HealthCheck {
    shm: SharedMemory<HealthCheckStatus>,
}

impl HealthCheck {
    pub fn new() -> Result<Self> {
        let shm = match SharedMemory::<HealthCheckStatus>::new(LINK_FILE_NAME) {
            Err(err) => {
                error!(
                    "Error creating shared memory: {err}, will not be able to listen for SIGUSR1 signal (Health check)"
                );
                return Err(err.into());
            }
            Ok(shm) => shm,
        };
        Ok(Self { shm })
    }

    pub fn set_healthy_shared(is_healthy: bool) -> Result<()> {
        Self::new()?.set_healthy(is_healthy)
    }

    pub fn set_healthy(&self, is_healthy: bool) -> Result<()> {
        self.shm.set(if is_healthy {
            HealthCheckStatus::Healthy
        } else {
            HealthCheckStatus::Unhealthy
        })?;
        Ok(())
    }

    pub fn check_if_healthy(&self) -> Result<bool> {
        let current_value = self.shm.get()?;
        match current_value {
            HealthCheckStatus::Healthy => {
                trace!("Health check is healthy");
                Ok(true)
            }
            HealthCheckStatus::Unhealthy => {
                trace!("Health check is unhealthy");
                Ok(false)
            }
        }
    }
}

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error(transparent)]
    SharedMemory(#[from] crate::shared_memory::Error),
    #[error("Unknown")]
    Unknown,
}
