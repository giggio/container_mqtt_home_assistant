#[macro_use]
extern crate log;
use clap::Parser;
extern crate docker_status_mqtt_proc_macros;
#[macro_use]
mod macros;
mod args;
mod cancellation_token;
mod device_manager;
mod devices;
mod docker;
mod helpers;
mod logger;
mod sample_device;
mod update_engine;

use crate::{
    args::*,
    cancellation_token::CancellationTokenSource,
    device_manager::*,
    devices::{DeviceProvider, Devices},
    sample_device::SampleDeviceProvider,
};
mod observer;
// use docker::*;

pub type Result<T> = std::result::Result<T, AppError>;

#[tokio::main]
async fn main() -> std::result::Result<(), String> {
    let _logger_handle = logger::start().map_err(|e| format!("Error starting logger: {e}"))?;
    run(Cli::parse()).await.map_err(|e| e.to_string())?;
    Ok(())
}

async fn run(cli: Cli) -> Result<()> {
    match cli.command {
        args::Commands::Run {
            mqtt_broker_info,
            sample_device_name,
            publish_interval,
            device_manager_id: node_id,
        } => {
            let mut device_providers: Vec<Box<dyn DeviceProvider>> = vec![];
            if let Some(name) = sample_device_name {
                device_providers.push(Box::new(SampleDeviceProvider::new(
                    name,
                    "Dependent device".to_string(),
                    true,
                )));
            }
            if device_providers.is_empty() {
                eprintln!(
                    "No device providers configured, nothing to do. Use --sample-device-name to add a sample device."
                );
                return Ok(());
            }
            let mut cancellation_token_source = CancellationTokenSource::new();
            deal_with_ctrl_c(cancellation_token_source.clone());
            let (mut device_manager, eventloop, rx) = DeviceManager::new(
                mqtt_broker_info.host,
                mqtt_broker_info.port,
                node_id.clone(),
                mqtt_broker_info.username,
                mqtt_broker_info.password,
                mqtt_broker_info.disable_tls,
                publish_interval,
                cancellation_token_source.create_token().await,
            )?;
            let devices = Devices::from_device_providers(
                device_providers,
                device_manager.availability_topic(),
                cancellation_token_source.create_token().await,
            )
            .await;
            device_manager.publish_sensor_data_periodically(rx, devices)?;
            info!("Configured, initiating connection and message exchange...");
            device_manager.deal_with_event_loop(eventloop).await?;
        }
    }
    if cli.verbose {
        info!("Shutting down...");
    }
    Ok(())
}

fn deal_with_ctrl_c(mut cancellation_token_source: CancellationTokenSource) {
    tokio::spawn(async move {
        match tokio::signal::ctrl_c().await {
            Ok(_) => {
                trace!("Ctrl-C received, cancelling cancellation token source...");
                cancellation_token_source.cancel().await;
            }
            Err(e) => {
                error!("Error listening for Ctrl-C signal: {e}");
            }
        }
    });
}

#[derive(thiserror::Error, Debug)]
pub enum AppError {
    #[error(transparent)]
    Logger(#[from] logger::Error),
    #[error(transparent)]
    DeviceManager(#[from] device_manager::Error),
    #[error(transparent)]
    Devices(#[from] devices::Error),
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
