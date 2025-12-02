use std::time::Duration;

use clap::{Args, Parser, Subcommand};
use const_format::formatc;

use crate::helpers::{hostname, slugify};

const ENV_PREFIX: &str = "MQTT_";

#[derive(Parser, Debug, PartialEq)]
#[command(version, about, long_about = None)]
pub struct Cli {
    #[arg(display_order = 102, global = true, short, long, env = formatc!("{ENV_PREFIX}QUIET"))]
    pub quiet: bool,

    #[arg(display_order = 103, global = true, long, env = formatc!("{ENV_PREFIX}VERBOSE"))]
    pub verbose: bool,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand, PartialEq)]
pub enum Commands {
    #[command(about = "Run the MQTT device.", long_about = None)]
    Run {
        #[command(flatten)]
        mqtt_broker_info: MqttBrokerInfo,

        #[arg(short = 'n', long, env = formatc!("{ENV_PREFIX}DEVICE_NAME"), default_value_t=device_name_fn(), help = "Name of the device to use in Home Assistant. If not provided, will use 'Containers at <hostname>'")]
        device_name: String,

        #[cfg(debug_assertions)]
        #[arg(long, env = formatc!("{ENV_PREFIX}SAMPLE_DEVICE"), help = "Enable sample device (only in debug builds)")]
        sample_device: bool,

        #[arg(short = 'I', long, env = formatc!("{ENV_PREFIX}PUBLISH_INTERVAL"), value_parser = parse_duration, default_value = "5000", help = "Publish interval (in milliseconds)")]
        publish_interval: Duration,

        #[arg(short = 'i', long, env = formatc!("{ENV_PREFIX}DEVICE_MANAGER_ID"), default_value_t=device_manager_id_fn(), value_parser=parse_slug, help = "Name of the Device Manager that identifies this instance to Home Assistant and group all devices (and, therefore, entities). If not provided, will use the hostname.")]
        device_manager_id: String,
    },
}

#[derive(Args, Debug, PartialEq)]
pub struct MqttBrokerInfo {
    // MQTT_HOST=localhost MQTT_PORT=1883 MQTT_USERNAME=mqtt_user MQTT_PASSWORD=password DISABLE_TLS=true
    #[arg(short, long, env = formatc!("{ENV_PREFIX}USERNAME"))]
    pub username: String,

    #[arg(short, long, env = formatc!("{ENV_PREFIX}PASSWORD"))]
    pub password: String,

    #[arg(short = 'H', long, env = formatc!("{ENV_PREFIX}HOST"))]
    pub host: String,

    #[arg(short = 'P', long, env = formatc!("{ENV_PREFIX}PORT"), default_value_t = 1883)]
    pub port: u16,

    #[arg(long, env = formatc!("{ENV_PREFIX}DISABLE_TLS"), help = "Disable TLS")]
    pub disable_tls: bool,
}

fn parse_duration(arg: &str) -> Result<Duration, String> {
    Ok(Duration::from_millis(
        arg.parse::<u64>().map_err(|e| format!("Invalid duration: {e}"))?,
    ))
}

#[allow(clippy::unnecessary_wraps)] // this is used by Clap, it needs to return Result
fn parse_slug(arg: &str) -> Result<String, String> {
    Ok(slugify(arg))
}

fn device_manager_id_fn() -> String {
    format!("Device {}", hostname())
}

fn device_name_fn() -> String {
    format!("Containers at {}", hostname())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use pretty_assertions::assert_eq;
    use std::time::Duration;

    #[test]
    fn test_parse_duration_valid() {
        assert_eq!(parse_duration("1000").unwrap(), Duration::from_millis(1000));
        assert_eq!(parse_duration("5000").unwrap(), Duration::from_millis(5000));
        assert_eq!(parse_duration("0").unwrap(), Duration::from_millis(0));
    }

    #[test]
    fn test_parse_duration_invalid() {
        assert!(parse_duration("invalid").is_err());
        assert!(parse_duration("-100").is_err());
        assert!(parse_duration("12.5").is_err());
    }

    #[test]
    fn test_parse_slug_basic() {
        assert_eq!(parse_slug("mqtt_cmha").unwrap(), "mqtt_cmha");
        assert_eq!(parse_slug("My Device").unwrap(), "my_device");
        assert_eq!(parse_slug("Test-Device-123").unwrap(), "test_device_123");
    }

    #[test]
    fn test_parse_slug_special_chars() {
        assert_eq!(parse_slug("nõde@123").unwrap(), "node_123");
        assert_eq!(parse_slug("tést#device").unwrap(), "test_device");
    }

    const BASIC_ARGS: [&str; 10] = [
        "cmha",
        "run",
        "--host",
        "localhost",
        "--port",
        "1883",
        "--username",
        "test_user",
        "--password",
        "test_pass",
    ];

    #[test]
    fn test_cli_basic_parsing() {
        let args = Vec::from(BASIC_ARGS);

        let cli = Cli::parse_from(args);

        match cli.command {
            Commands::Run { mqtt_broker_info, .. } => {
                assert_eq!(mqtt_broker_info.host, "localhost");
                assert_eq!(mqtt_broker_info.port, 1883);
                assert_eq!(mqtt_broker_info.username, "test_user");
                assert_eq!(mqtt_broker_info.password, "test_pass");
                assert!(!mqtt_broker_info.disable_tls);
            }
        }
    }

    #[cfg(debug_assertions)]
    #[test]
    fn test_cli_with_sample_device_name() {
        let args = Vec::from(BASIC_ARGS)
            .into_iter()
            .chain(vec!["--device-name", "My Custom Device", "--sample-device"])
            .collect::<Vec<_>>();
        let cli = Cli::parse_from(args);

        match cli.command {
            Commands::Run {
                device_name,
                sample_device,
                ..
            } => {
                assert_eq!(device_name, "My Custom Device".to_string());
                assert!(sample_device);
            }
        }
    }

    #[test]
    fn test_cli_with_publish_interval() {
        let args = Vec::from(BASIC_ARGS)
            .into_iter()
            .chain(vec!["--publish-interval", "10000"])
            .collect::<Vec<_>>();
        let cli = Cli::parse_from(args);
        match cli.command {
            Commands::Run { publish_interval, .. } => {
                assert_eq!(publish_interval, Duration::from_millis(10000));
            }
        }
    }

    #[test]
    fn test_cli_with_device_manager_id() {
        let args = Vec::from(BASIC_ARGS)
            .into_iter()
            .chain(vec!["--device-manager-id", "My Custom Device Manager"])
            .collect::<Vec<_>>();
        let cli = Cli::parse_from(args);
        match cli.command {
            Commands::Run { device_manager_id, .. } => {
                assert_eq!(device_manager_id, "my_custom_device_manager");
            }
        }
    }

    #[test]
    fn test_cli_disable_tls() {
        let args = Vec::from(BASIC_ARGS)
            .into_iter()
            .chain(vec!["--disable-tls"])
            .collect::<Vec<_>>();
        let cli = Cli::parse_from(args);
        match cli.command {
            Commands::Run { mqtt_broker_info, .. } => {
                assert!(mqtt_broker_info.disable_tls);
            }
        }
    }

    #[test]
    fn test_cli_verbose_flag() {
        let args = Vec::from(BASIC_ARGS)
            .into_iter()
            .chain(vec!["--verbose"])
            .collect::<Vec<_>>();
        let cli = Cli::parse_from(args);
        assert!(cli.verbose);
    }

    #[test]
    fn test_cli_quiet_flag() {
        let args = Vec::from(BASIC_ARGS)
            .into_iter()
            .chain(vec!["--quiet"])
            .collect::<Vec<_>>();
        let cli = Cli::parse_from(args);
        assert!(cli.quiet);
    }

    #[cfg(debug_assertions)]
    #[test]
    fn test_cli_all_options_together() {
        let args = Vec::from(BASIC_ARGS)
            .into_iter()
            .chain(vec![
                "--device-name",
                "Production Server",
                "--publish-interval",
                "15000",
                "--device-manager-id",
                "prod-device-01",
                "--verbose",
            ])
            .collect::<Vec<_>>();
        let cli = Cli::parse_from(args);
        assert_eq!(
            cli,
            Cli {
                quiet: false,
                verbose: true,
                command: Commands::Run {
                    mqtt_broker_info: MqttBrokerInfo {
                        username: "test_user".to_string(),
                        password: "test_pass".to_string(),
                        host: "localhost".to_string(),
                        port: 1883,
                        disable_tls: false,
                    },
                    device_name: "Production Server".to_string(),
                    publish_interval: Duration::from_millis(15000),
                    device_manager_id: "prod_device_01".to_string(),
                    sample_device: false,
                },
            }
        );
    }

    #[test]
    fn test_device_manager_id_slugification() {
        let test_cases = vec![
            ("simple", "simple"),
            ("with spaces", "with_spaces"),
            ("with-dashes", "with_dashes"),
            ("MixedCase", "mixedcase"),
            ("special@chars#123", "special_chars_123"),
        ];
        for (input, expected) in test_cases {
            let args = Vec::from(BASIC_ARGS)
                .into_iter()
                .chain(vec!["--device-manager-id", input])
                .collect::<Vec<_>>();
            let cli = Cli::parse_from(args);
            match cli.command {
                Commands::Run { device_manager_id, .. } => {
                    assert_eq!(
                        device_manager_id, expected,
                        "Failed to slugify '{input}' to '{expected}'"
                    );
                }
            }
        }
    }
}
