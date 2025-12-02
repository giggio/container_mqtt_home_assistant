#![cfg_attr(
    test,
    allow(clippy::unwrap_used),
    allow(clippy::panic),
    allow(clippy::similar_names),
    allow(clippy::too_many_lines)
)]

#[macro_use]
extern crate log;

use std::sync::Arc;

use clap::Parser;
extern crate cmha_proc_macros;
#[macro_use]
mod macros;
mod args;
mod cancellation_token;
mod device_manager;
mod devices;
mod helpers;
mod logger;
#[cfg(debug_assertions)]
mod sample_device;
mod update_engine;

#[cfg(debug_assertions)]
use crate::sample_device::SampleDeviceProvider;
use crate::{
    args::{Cli, Commands},
    cancellation_token::CancellationTokenSource,
    device_manager::{DeviceManager, Error},
    devices::{DeviceProvider, Devices},
    container_device::ContainerDeviceProvider,
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
    match cli.command {
        Commands::Run {
            mqtt_broker_info,
            device_name,
            publish_interval,
            device_manager_id,
            #[cfg(debug_assertions)]
            sample_device,
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
                device_providers.push(Box::new(ContainerDeviceProvider::new(device_name)?));
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
            deal_with_ctrl_c(cancellation_token_source.clone(), device_manager.clone());
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
    }
    if cli.verbose {
        info!("Shutting down...");
    }
    debug!("Shutting down...");
    Ok(())
}

fn deal_with_ctrl_c(mut cancellation_token_source: CancellationTokenSource, mut device_manager: DeviceManager) {
    tokio::spawn(async move {
        if let Err(err) = tokio::signal::ctrl_c().await {
            eprintln!("Error listening for Ctrl-C signal: {err}");
            std::process::exit(1);
        }
        trace!("Ctrl-C received, cancelling cancellation token source...");
        cancellation_token_source.cancel().await;
        trace!("Ctrl-C received, stopping device manager...");
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
