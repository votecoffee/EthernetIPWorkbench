# C# MQTT feed reader

This .NET 8 console example subscribes to one EtherNet/IP Workbench telemetry topic and emits fresh samples as JSON Lines. Build with the .NET 8 SDK or a later SDK with the .NET 8 targeting pack. The dependency lock pins MQTTnet 5.2.0.1603. [MQTTnet package](https://www.nuget.org/packages/MQTTnet/5.2.0.1603) · [Upstream license](https://github.com/dotnet/MQTTnet/blob/v5.2.0/LICENSE)

## Configure the publisher

Use a trusted broker with an MQTT 3.1.1 TCP listener. Enable MQTT in the scanner's Integration tab or add the equivalent block to a read-only profile before starting `eipmaster-cli serve`.

~~~toml
[app.integration.mqtt]
broker = "127.0.0.1"
port = 1883
topic = "bench/dotnet/samples"
period_ms = 25
~~~

Give each scanner feed a distinct topic. The example connects to the same broker and subscribes at QoS 0. [Scanner MQTT and serve setup](../../docs/OPERATIONS.md#connect-other-software)

## Build and subscribe

~~~powershell
dotnet restore examples\dotnet-mqtt\MqttFeedReader.csproj --locked-mode
dotnet run --project examples\dotnet-mqtt\MqttFeedReader.csproj -c Release --no-restore -- 127.0.0.1 1883 bench/dotnet/samples 50
~~~

The final argument requests 50 samples. Omit it to run until Ctrl+C. An optional age limit in milliseconds can follow the count and defaults to 500 ms. Progress goes to stderr. Stdout contains one JSON line per accepted sample.

The subscriber reconnects and subscribes again after a broker transport outage. It preserves the previous sample index within the same publisher stream and reports `skipped_export_indexes` between emitted samples. The reader discards retained publications, stale samples and unrecognized schemas. `estimated_age_us` compares the sample's UTC stream start and elapsed time with the reader's local clock. Use synchronized clocks for a reader on another host. The payload preserves sample generation, packet sequences, raw bytes and all mapped values as strings. Use the HTTP diagnostics endpoint for active connection state. [HTTP connection diagnostics](../../docs/OPERATIONS.md#inspect-a-live-class-1-connection)

## Verify on localhost

~~~powershell
dotnet build examples\dotnet-mqtt\MqttFeedReader.csproj -c Release --no-restore
python tools\test_dotnet_mqtt.py .\eipmaster-cli.exe examples\dotnet-mqtt\bin\Release\net8.0\MqttFeedReader.dll .\mqtt-reader-check
~~~

The test starts its own localhost MQTT 3.1.1 broker fixture and software adapter. It forces a subscriber disconnect and a Class 1 reconnect, then checks every emitted sample against the scanner's retained HTTP history. From a source checkout use `.\target\release\eipmaster-cli.exe` and a fresh results directory under `.local`. For HTTP and shared-memory clients use the [C# feed reader](../dotnet/README.md).
