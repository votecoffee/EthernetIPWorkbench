# C# feed reader

This .NET 8 console example reads an existing EtherNet/IP Workbench input stream through loopback HTTP or Windows shared memory. Build with the .NET 8 SDK or a later SDK with the .NET 8 targeting pack. It uses the .NET base class library.

## Configure the scanner

In the GUI enable HTTP or shared memory under **Integration** and open the verified input session. To run headlessly use an input-only profile with the same settings in its TOML file.

~~~toml
[app.integration]
http_port = 8765
shared_memory = "dotnet-demo"
~~~

For a local demonstration, copy `samples/mapped-loopback.toml` to `feed-profile.toml`, set both RPIs to 8000 µs, and add the table above. Start the software adapter and then the scanner in separate consoles from the unpacked bundle.

~~~powershell
python tools\loopback_adapter.py loopback-result.json --mode normal
.\eipmaster-cli.exe serve .\feed-profile.toml "0:12:1:305419896"
~~~

In a source checkout, build the scanner and use `.\target\release\eipmaster-cli.exe`.

## Run the reader

~~~powershell
dotnet run --project examples\dotnet\FeedReader.csproj -c Release -- http 8765 20
dotnet run --project examples\dotnet\FeedReader.csproj -c Release -- memory dotnet-demo 20
~~~

The last argument in each command requests 20 samples. Omit it to read until Ctrl+C. An optional positive age limit in milliseconds can follow the count. It defaults to 500 ms. The reader emits one JSON object per new, fresh sample from the active generation. Redirect stdout to a JSON Lines file to keep the values. It resets its index when the stream start timestamp changes. `skipped_export_indexes` counts indexes between consecutive outputs of this reader in the same stream.

HTTP uses `GET /v1/latest` with schema version 1 and the server's reported age. It preserves all mapped values as strings. Use `/v1/samples` for bounded history and `/v1/connection` for connection diagnostics. [HTTP consumer contract](../../docs/OPERATIONS.md#connect-other-software)

On Windows the memory reader opens the named mapping read-only. Each poll acquires the mutex without waiting, copies the 573-byte view, and releases the mutex before decoding. It disposes both named handles after the poll so the publisher can reuse the name on restart. It estimates age from the stream's UTC start plus elapsed microseconds and emits the raw payload. [Shared-memory layout](../../README.md#connect-other-software) · [.NET mapping API](https://learn.microsoft.com/en-us/dotnet/api/system.io.memorymappedfiles.memorymappedfile.openexisting?view=net-8.0) · [.NET mutex API](https://learn.microsoft.com/en-us/dotnet/api/system.threading.mutex.openexisting?view=net-8.0)

Both paths poll every 20 ms. For a complete packet series, use the scanner's Every packet CSV capture. [Capture and verify CSV](../../docs/OPERATIONS.md#capture-and-verify-csv)

## Verify on Windows

~~~powershell
dotnet build examples\dotnet\FeedReader.csproj -c Release
python tools\test_dotnet_consumer.py .\eipmaster-cli.exe examples\dotnet\bin\Release\net8.0\FeedReader.dll .\dotnet-reader-check
~~~

The test starts its own localhost fixture and two readers. It checks both generations of a reconnect, HTTP typed values, raw payload agreement, age and the final close results. Choose a fresh results directory outside the repository or under its ignored `.local` folder.
