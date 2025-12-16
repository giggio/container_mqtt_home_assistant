#![cfg_attr(
    test,
    allow(clippy::unwrap_used),
    allow(clippy::panic),
    allow(clippy::similar_names),
    allow(clippy::too_many_lines)
)]

#[macro_use]
extern crate log;

use std::{process, sync::Arc};

use clap::Parser;
extern crate cmha_proc_macros;
#[macro_use]
mod macros;
mod args;
mod cancellation_token;
mod device_manager;
mod devices;
mod healthcheck;
mod helpers;
mod logger;
#[cfg(debug_assertions)]
mod sample_device;
mod shared_memory;
mod update_engine;

#[cfg(debug_assertions)]
use crate::sample_device::SampleDeviceProvider;
use crate::{
    args::{Cli, Commands},
    cancellation_token::CancellationTokenSource,
    container_device::ContainerDeviceProvider,
    device_manager::{DeviceManager, Error},
    devices::{DeviceProvider, Devices},
    healthcheck::HealthCheck,
};
mod container_device;

pub type Result<T> = std::result::Result<T, AppError>;

#[tokio::main]
async fn main() -> std::result::Result<(), String> {
    let _logger_handle = logger::start().map_err(|e| format!("Error starting logger: {e}"))?;
    run(Cli::parse()).await.map_err(|e| e.to_string())?;
    Ok(())
}

async fn run(cli: Cli) -> Result<()> {
    // this will hold the shared memory in the main process until exit and will be used to check if the main process is healthy
    let healthcheck = HealthCheck::new()?;
    match cli.command {
        Commands::Run {
            mqtt_broker_info,
            device_name,
            publish_interval,
            device_manager_id,
            #[cfg(debug_assertions)]
            sample_device,
            prefix,
        } => {
            #[cfg(not(debug_assertions))]
            #[allow(non_upper_case_globals)]
            const sample_device: bool = false;
            let mut device_providers: Vec<Box<dyn DeviceProvider>> = vec![];
            if sample_device {
                #[cfg(debug_assertions)]
                {
                    trace!("Adding sample device provider with name: {device_name}");
                    device_providers.push(Box::new(SampleDeviceProvider::new(device_name)));
                }
            } else {
                trace!("Adding Container device provider with name: {device_name}");
                device_providers.push(Box::new(ContainerDeviceProvider::new(device_name, prefix)?));
            }
            let device_providers_arc = Arc::new(device_providers);
            let (mut device_manager, eventloop, rx) = DeviceManager::new(
                mqtt_broker_info.host,
                mqtt_broker_info.port,
                device_manager_id,
                mqtt_broker_info.username,
                mqtt_broker_info.password,
                mqtt_broker_info.disable_tls,
                publish_interval,
            )?;
            let mut cancellation_token_source = CancellationTokenSource::new();
            deal_with_stop_request(cancellation_token_source.clone(), device_manager.clone());
            let devices = Devices::from_device_providers(
                device_providers_arc.clone(),
                device_manager.availability_topic(),
                cancellation_token_source.create_token().await,
            )
            .await?;
            device_manager.publish_sensor_data_periodically(rx, devices, device_providers_arc);
            info!("Configured, initiating connection and message exchange...");
            device_manager.deal_with_event_loop(eventloop).await?;
        }
        Commands::HealthCheck { .. } => {
            if healthcheck.check_if_healthy()? {
                trace!("Health check passed");
                if cli.verbose {
                    println!("Health check passed");
                }
            } else {
                trace!("Health check failed");
                eprintln!("Health check failed");
                process::exit(1);
            }
        }
    }
    if cli.verbose {
        info!("Shutting down...");
    }
    debug!("Shutting down...");
    Ok(())
}

fn deal_with_stop_request(mut cancellation_token_source: CancellationTokenSource, mut device_manager: DeviceManager) {
    use tokio::signal::unix::{SignalKind, signal};
    tokio::spawn(async move {
        let mut sigint = match signal(SignalKind::interrupt()) {
            Err(err) => {
                eprintln!("Error listening for SIGINT (Ctrl+C) signal: {err}");
                process::exit(1);
            }
            Ok(sigint) => sigint,
        };
        let mut sigterm = match signal(SignalKind::terminate()) {
            Err(err) => {
                eprintln!("Error listening for SIGTERM signal: {err}");
                process::exit(1);
            }
            Ok(sigterm) => sigterm,
        };
        tokio::select! {
            _ = sigint.recv() => {
                trace!("SIGINT (Ctrl+C) received, cancelling cancellation token source...");
            }
            _ = sigterm.recv() => {
                trace!("SIGTERM received, cancelling cancellation token source...");
            }
        }
        cancellation_token_source.cancel().await;
        trace!("Termination request received, stopping device manager...");
        tokio::select! {
            stop_result = device_manager.stop() => {
                if let Err(e) = stop_result {
                    error!("Device manager stopped with error: {e}");
                } else {
                    trace!("Device manager stopped successfully");
                }
            }
            _ = tokio::time::sleep(device_manager::DURATION_UNTIL_SHUTDOWN) => {
                trace!("Force stopping device manager...");
                if let Err(e) = device_manager.force_stop().await {
                    error!("Error force stopping device manager: {e}");
                } else {
                    trace!("Device manager force stopped successfully");
                }
            }
        }
    });
}

#[derive(thiserror::Error, Debug)]
pub enum AppError {
    #[error(transparent)]
    Logger(#[from] logger::Error),
    #[error(transparent)]
    DeviceManager(#[from] Error),
    #[error(transparent)]
    Devices(#[from] devices::Error),
    #[error(transparent)]
    ContainerDevice(#[from] container_device::Error),
    #[error(transparent)]
    SharedMemory(#[from] shared_memory::Error),
    #[error(transparent)]
    HealthCheck(#[from] healthcheck::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[ctor::ctor]
    static LOGGER: flexi_logger::LoggerHandle = {
        let logger_handle_result = logger::start();
        match logger_handle_result {
            Ok(r) => r,
            Err(e) => {
                eprintln!("Error starting logger: {e}");
                panic!("Error starting logger: {e}");
            }
        }
    };
}
