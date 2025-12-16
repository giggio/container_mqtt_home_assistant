# Container Status MQTT Home Assistant

![Build status](https://codeberg.org/giggio/container_mqtt_home_assistant/badges/workflows/build.yaml/badge.svg)

Main repo: [codeberg.org/giggio/container_mqtt_home_assistant](https://codeberg.org/giggio/container_mqtt_home_assistant)

This app will report the status of all your containers to MQTT.

It is designed to work with the protocols of Home Assistant's MQTT integration.

## Quick Start

Run it as a container in the Docker host where the MQTT broker is, like so (replace username and password with your own):

```bash
docker run -e RUST_LOG=cmha=trace -v /var/run/docker.sock:/var/run/docker.sock --add-host=host.docker.internal=host-gateway \
  giggio/cmha run --host=host.docker.internal --port=1883 --username=mqtt_user --password=password --disable-tls --device-name='MQTT Test' --prefix=test
```

It is also possible to use environment variables to specify the same options. To view all options, run:

```bash
docker run --rm -ti giggio/cmha run --help
```

The device name is used in Home Assistant to identify the Docker host, a new device will be created with its name and
each container will show as connected through it.
Communication to MQTT goes through https or http, with Docker though the default unix socket.

Bellow is a compose configuration ([compose.yml](./compose.yml)) that will run the container in the Docker host and
assumes that the MQTT broker is on the same host.

```yaml
services:
  cmha:
    image: giggio/cmha
    container_name: container_mqtt_ha
    environment:
      RUST_LOG: cmha=trace # optional
      MQTT_DEVICE_NAME: Device my_hosthame
      MQTT_USERNAME: mqtt_user
      MQTT_PASSWORD: ${MQTT_PASSWORD?MQTT_PASSWORD is required.}
      MQTT_HOST: host.docker.internal # or your MQTT broker fqdn or ip
      MQTT_PORT: 1883 # optional, defaults to 1883
      MQTT_DISABLE_TLS: false # optional, should be avoided, defaults to false
      MQTT_PUBLISH_INTERVAL: 5000 # optional, defaults to 5000 (milliseconds)
      MQTT_DEVICE_MANAGER_ID: Containers at my_hostname # optional, defaults to 'Containers at <hostname>'
      MQTT_PREFIX: your_prefix # optional, useful if you have multiple docker hosts and you might have containers with the same name, defaults to ''
    volumes:
      - /var/run/docker.sock:/var/run/docker.sock
    extra_hosts:
      - "host.docker.internal:host-gateway" # only necessary if your MQTT broker is on the host
    restart: always
```

## Running

### Markdown card for Home Assistant

Bellow is a markdown card for Home Assistant that will show the status of all your containers.
It assumes that MQTT_DEVICE_NAME is in the format `Device my_hostname` and that you are using the `MQTT_PREFIX` option
as `my_hostname`.

```jinja
{%- for name in integration_entities('mqtt')
  | map('device_id')
  | unique
  | map('device_attr', 'name')
  | select('match', '^Device ')
  | sort() -%}
  {%- set device_name = name |regex_replace(find='Device ', replace='') %}
  ## {{ device_name }}
| Container | Status | Health | Ok |
|---|---|---|---|
  {% for container_name in integration_entities('mqtt')
    | map('device_attr', 'name')
    | unique
    | select('match', '^' ~ device_name)
    | sort() -%}
      {%- set fixed_container_name = container_name | regex_replace(find='-', replace='_') -%}
      [{{- container_name |regex_replace(find='^' ~ device_name ~ '_', replace='')
        -}}](/config/devices/device/{{ device_id(container_name) }}) |
      {{- states('sensor.' ~ fixed_container_name ~ '_run_status') -}} |
      {{- states('sensor.' ~ fixed_container_name ~ '_health') -}} |
      {{- '✔️' if is_state('sensor.' ~ fixed_container_name ~ '_run_status', 'running')
        and states('sensor.' ~ fixed_container_name ~ '_health') in ['healthy', 'unknown'] else '❌' -}} |
  {% endfor -%}
{% endfor -%}
```

### Running container automation for Home Assistant

This is an example of a running container automation for Home Assistant. It will react to the status of a container
and send a message to Telegram if the container has not been running for 1 hour.

```yaml
alias: Container stopped
description: Reacts to container not running
triggers:
  - trigger: mqtt
    options:
      topic: +/run_status/state
      payload: "1"
      value_template: "{{ '1' if value != 'running' else '0' }}"
conditions: []
actions:
  - variables:
      device_name: >-
        {{ trigger.topic | regex_replace(find='(\w+)/run_status/state', replace='\\1') }}
  - delay:
      hours: 0
      minutes: 0
      seconds: 35
      milliseconds: 0
  - if:
      - condition: template
        value_template: "{{ not is_state('sensor.' ~ device_name ~ '_run_status', 'running') }}"
        alias: Check if run state is still not running
    then:
      - action: telegram_bot.send_message
        data:
          message: >-
            Container `{{ device_name }}` is *not running*. Status: `{{ trigger.payload }}`.
        metadata: {}
  - repeat:
      count: 6
      sequence:
        - delay:
            hours: 0
            minutes: 10
            seconds: 0
            milliseconds: 0
        - if:
            - alias: Check if run state is now running
              condition: template
              value_template: >-
                {{ is_state('sensor.' ~ device_name ~ '_run_status', 'running') }}
          then:
            - action: telegram_bot.send_message
              data:
                message: Container `{{ device_name }}` is *now running*.
              metadata: {}
              alias: Telegram message saying that container is working again
            - stop: container is now running
  - action: telegram_bot.send_message
    data:
      message: >-
        Container `{{ device_name }}` is *not running* for *1 hour*. Status: `{{ trigger.payload }}`.
    metadata: {}
    alias: Telegram message that container has not restarted after a long period
mode: parallel
max: 20
```

### Detailed commands

#### run

At the moment of this writing the options are:

```
Usage: cmha run [OPTIONS] --username <USERNAME> --password <PASSWORD> --host <HOST>

Options:
  -u, --username <USERNAME>
          [env: MQTT_USERNAME=]
  -p, --password <PASSWORD>
          [env: MQTT_PASSWORD=]
  -H, --host <HOST>
          [env: MQTT_HOST=]
  -P, --port <PORT>
          [env: MQTT_PORT=] [default: 1883]
      --disable-tls
          Disable TLS [env: MQTT_DISABLE_TLS=]
  -n, --device-name <DEVICE_NAME>
          Name of the device to use in Home Assistant. If not provided, will use 'Containers at <hostname>' [env: MQTT_DEVICE_NAME=] [default: "Containers at c99a86f73434"]
  -I, --publish-interval <PUBLISH_INTERVAL>
          Publish interval (in milliseconds) [env: MQTT_PUBLISH_INTERVAL=] [default: 5000]
  -i, --device-manager-id <DEVICE_MANAGER_ID>
          Name of the Device Manager that identifies this instance to Home Assistant and group all devices (and, therefore, entities). If not provided, will use the hostname. [env: MQTT_DEVICE_MANAGER_ID=] [default: "Device c99a86f73434"]
      --prefix <PREFIX>
          Prefix to add do device name for each container. If not provided, will use only the container name. [env: MQTT_PREFIX=]
  -q, --quiet
          [env: MQTT_QUIET=]
      --verbose
          [env: MQTT_VERBOSE=]
  -h, --help
          Print help
```

#### health-check

This should not be used directly, it is used by the container health check.

### Logging

Logging is controlled by environment variable `RUST_LOG`, as is common with Rust applications. It can be set to error,
warn, info, debug or trace. Setting it will enable logs for all libraries used by the application, so it might be better
to prefix it with `cmha=`, e.g. `RUST_LOG=cmha=info`, to only get logs from this application. Setting it without prefix
will show http communication logs and logs from communication with the docker host, among others. The default is info.
Logs are output to stdout.

### Container image tags

There are tags for `amd64` and `arm64`, and also for each released versions, like `0.1.0`, which is a multiarchitecture
image, and variants for each architecture, like `0.1.0-amd64`.

The default is `latest`, and that is a multiarchitecture image too. Pull from `latest` and you will get the correct
architecture and latest updates.

### Health check

The container will be healthy if the MQTT connection can be established.

## Development

Bring up the Home Assistant server using docker compose (compose.yaml is in `cmha` directory).

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

Bug reports that include steps-to-reproduce (including code) are the best. Even better, make them in the form of pull
requests. Pull requests on Github will probably be ignored, so avoid them.

## Author

[Giovanni Bassi](https://links.giggio.net/bio)

## License

Licensed under the MIT license.
