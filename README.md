# Container Status MQTT Home Assistant

This app will report the status of all your containers to MQTT.

It is designed to work with the protocols of Home Assistant's MQTT integration.

## Quick Start

TBD

````bash
docker run --rm -ti ...
````

### Detailed commands

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
