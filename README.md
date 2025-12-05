# Container Status MQTT Home Assistant

![Build status](https://codeberg.org/giggio/container_mqtt_home_assistant/badges/workflows/build.yaml/badge.svg)

Main repo: [codeberg.org/giggio/container_mqtt_home_assistant](https://codeberg.org/giggio/container_mqtt_home_assistant)

This app will report the status of all your containers to MQTT.

It is designed to work with the protocols of Home Assistant's MQTT integration.

## Quick Start

Run it as a container in the Docker host, like so

```bash
docker run giggio/cmha run --host=<mqtt_host> --port=<mqtt_port> --username=<mqtt_user> --password=<mqtt_password> --device-name='<device name>'
```

It is also possible to use environment variables to specify the same options. To view all options, run:

```bash
docker run giggio/cmha run --help
```

The device name is used in Home Assistant to identify the Docker host, a new device will be created with its name and
each container will show as connected through it.
Communication to MQTT goes through https or http, with Docker though the default unix socket.

Bellow is a compose configuration ([compose.yml](./compose.yml)) that will run the container in the Docker host and
assumes that the MQTT broker is on the same host.

```yaml
services:
  container_mqtt_ha:
    image: giggio/cmha
    container_name: container_mqtt_ha
    environment:
      MQTT_USERNAME: <mqtt_user>
      MQTT_PASSWORD: <mqtt_password>
      MQTT_HOST: host.docker.internal
      # MQTT_PORT: 1883 # optional, defaults to 1883
      MQTT_DEVICE_NAME: <device name>
      # MQTT_DISABLE_TLS: true
      # MQTT_PUBLISH_INTERVAL: 5000
      # MQTT_DEVICE_MANAGER_ID: <device manager id>
    volumes:
      - /var/run/docker.sock:/var/run/docker.sock
    extra_hosts:
      - "host.docker.internal:host-gateway" # only necessary if your MQTT broker is on the host
    restart: always
```

### Detailed commands

TBD

### Logging

Logging is controlled by environment variable `RUST_LOG`, as is common with Rust applications. It can be set to error,
warn, info, debug or trace. Setting it will enable logs for all libraries used by the application, so it might be better
to prefix it with `cmha=`, e.g. `RUST_LOG=cmha=info`, to only get logs from this application. Setting it without prefix
will show http communication logs and logs from communication with the docker host, among others.

### Image tags

There are tags for `amd64` and `arm64`, and also for each released versions, like `0.1.0`, which is a multiarchitecture
image, and variants for each architecture, like `0.1.0-amd64`.

The default is `latest`, and that is a multiarchitecture image too. Pull from `latest` and you will get the correct
architecture and latest updates.

### Healthcheck

TBD

## Development

Bring up the Home Assistant server using docker compose (compose.yaml is in cmha directory).

Access it at <http://localhost:8123/>

Install the MQTT integration on Home Assistant.

### Helpful debug commands

Remove one entity from Home Assistant:

```bash
docker exec -ti mlogs sh -c 'mosquitto_pub --username $USER --pw $PASSWORD -t homeassistant/device/mqtt_docker/config -m '"'"'{"availability_topic":"mqtt_docker/availability","components":{"living_room_light":{"brightness_command_topic":"mqtt_docker/mqtt_test/living_room_light_brightness/command","brightness_scale":100,"brightness_state_topic":"mqtt_docker/mqtt_test/living_room_light_brightness/state","command_topic":"mqtt_docker/mqtt_test/living_room_light/command","icon":"mdi:lightbulb","name":"Living Room Light","payload_off":"OFF","payload_on":"ON","platform":"light","state_topic":"mqtt_docker/mqtt_test/living_room_light/state","unique_id":"mqtt_test_living_room_light"},"memory_usage":{"platform":"sensor"},"server_mode":{"command_topic":"mqtt_docker/mqtt_test/server_mode/command","device_class":null,"icon":"mdi:server","name":"Server Mode","payload_off":"OFF","payload_on":"ON","platform":"switch","state_topic":"mqtt_docker/mqtt_test/server_mode/state","unique_id":"mqtt_test_server_mode"},"system_uptime":{"device_class":"duration","icon":"mdi:clock-outline","name":"System Uptime","platform":"sensor","state_topic":"mqtt_docker/mqtt_test/system_uptime/state","unique_id":"mqtt_test_system_uptime","unit_of_measurement":"s"}},"device":{"identifiers":["mqtt_test"],"manufacturer":"Giovanni Bassi","name":"MQTT Test","sw_version":"0.1.0"},"origin":{"name":"cmha","sw":"0.1.0","url":"https://codeberg.org/giggio/container_mqtt_home_assistant"}}'"'"
```

Remove the whole device from Home Assistant:

```bash
docker exec -ti mlogs sh -c 'mosquitto_pub --username $USER --pw $PASSWORD -t homeassistant/device/mqtt_docker/config -m '"'"'{"availability_topic":"mqtt_docker/availability","components":{"living_room_light":{"platform":"light"},"memory_usage":{"platform":"sensor"},"server_mode":{"platform":"switch"},"system_uptime":{"platform":"sensor"}},"device":{"identifiers":["mqtt_test"],"manufacturer":"Giovanni Bassi","name":"MQTT Test","sw_version":"0.1.0"},"origin":{"name":"cmha","sw":"0.1.0","url":"https://codeberg.org/giggio/container_mqtt_home_assistant"}}'"'"
```

View Mosquitto logs:

```bash
docker logs -f mlogs
```

## Releasing

Releases are being created using Make and Nix cross compilation. See [./cross-build.nix](./cross-build.nix) and
[./flake.nix](./flake.nix) and [./Makefile](./Makefile).

To release, simply run `make`, which will build static binaries for amd64 and arm64 using musl, then create the images
and publish them to Docker Hub.

## Contributing

Questions, comments, bug reports, and pull requests are all welcome.  Submit them at
[the project on Codeberg](https://codeberg.com/giggio/container_mqtt_home_assistant/).

Bug reports that include steps-to-reproduce (including code) are the
best. Even better, make them in the form of pull requests.

## Author

[Giovanni Bassi](https://links.giggio.net/bio)

## License

Licensed under the MIT license.
