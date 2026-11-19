using System.Globalization;
using System.Net;
using System.Net.Http.Json;
using System.Text.Json;

namespace FeedReader;

internal sealed record Sample(
    long StreamStartUtcUs,
    ulong SampleIndex,
    ulong Generation,
    ulong ElapsedUs,
    uint PacketSequence,
    ushort CipSequence,
    ushort InputAssembly,
    string PayloadHex,
    Dictionary<string, string?>? Values);

internal interface IFeedResponse
{
    int SchemaVersion { get; }
}

internal sealed record Snapshot(ulong ActiveGeneration, ulong? SampleAgeUs, Sample? Sample) : IFeedResponse
{
    public int SchemaVersion { get; init; }
}

internal sealed record HistoryBatch(
    int SchemaVersion,
    long StreamStartUtcUs,
    ulong ActiveGeneration,
    ulong? OldestSampleIndex,
    ulong? LatestSampleIndex,
    Sample[] Samples) : IFeedResponse;

internal static class Program
{
    private static readonly JsonSerializerOptions Json = new()
    {
        PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower
    };

    public static async Task<int> Main(string[] args)
    {
        if (args.Length is < 2 or > 4 || args[0] is not ("http" or "memory" or "history") ||
            (args[0] == "history" && args.Length > 3))
        {
            Console.Error.WriteLine("Usage: FeedReader <http PORT | memory NAME> [sample-count] [max-age-ms]");
            Console.Error.WriteLine("       FeedReader history PORT [sample-count]");
            return 1;
        }

        try
        {
            var sampleLimit = args.Length >= 3 ? PositiveInt(args[2], "sample-count") : (int?)null;
            var maxAgeUs = args.Length == 4 ? (ulong)PositiveInt(args[3], "max-age-ms") * 1000 : 500_000UL;
            using var stop = new CancellationTokenSource();
            ConsoleCancelEventHandler cancel = (_, signal) =>
            {
                signal.Cancel = true;
                stop.Cancel();
            };

            Console.CancelKeyPress += cancel;
            try
            {
                return args[0] == "history"
                    ? await WatchHistory(args[1], sampleLimit, stop.Token)
                    : await Watch(args[0], args[1], sampleLimit, maxAgeUs, stop.Token);
            }
            catch (OperationCanceledException) when (stop.IsCancellationRequested)
            {
                return 130;
            }
            finally
            {
                Console.CancelKeyPress -= cancel;
            }
        }
        catch (Exception error) when (error is ArgumentException or InvalidDataException or
            HttpRequestException or JsonException or IOException or UnauthorizedAccessException or
            PlatformNotSupportedException)
        {
            Console.Error.WriteLine(error.Message);
            return 1;
        }
    }

    private static int PositiveInt(string text, string name)
    {
        if (!int.TryParse(text, out var result) || result <= 0)
            throw new ArgumentException($"{name} requires a positive whole number.");
        return result;
    }

    private static async Task<int> Watch(string transport, string target, int? sampleLimit,
        ulong maxAgeUs, CancellationToken cancellation)
    {
        using var http = transport == "http" ? OpenHttp(target) : null;
        var memory = http is null ? new SharedMemoryReader(target) : null;
        using var timer = new PeriodicTimer(TimeSpan.FromMilliseconds(20));
        long? stream = null;
        ulong previousIndex = 0;
        var emitted = 0;
        Console.Error.WriteLine($"Waiting for {transport} input.");
        do
        {
            cancellation.ThrowIfCancellationRequested();
            var snapshot = http is null ? memory!.Read() : await ReadHttp<Snapshot>(http, "/v1/latest", cancellation);
            var sample = snapshot?.Sample;
            if (sample is null || snapshot!.ActiveGeneration == 0 ||
                sample.Generation != snapshot.ActiveGeneration || snapshot.SampleAgeUs is not ulong ageUs ||
                ageUs > maxAgeUs || sample.SampleIndex == 0)
                continue;

            if (stream != sample.StreamStartUtcUs)
            {
                stream = sample.StreamStartUtcUs;
                previousIndex = 0;
            }
            if (sample.SampleIndex <= previousIndex)
                continue;

            var skipped = previousIndex == 0 ? 0 : sample.SampleIndex - previousIndex - 1;
            Console.WriteLine(JsonSerializer.Serialize(new
            {
                transport,
                snapshot.ActiveGeneration,
                snapshot.SampleAgeUs,
                SkippedExportIndexes = skipped,
                sample
            }, Json));
            previousIndex = sample.SampleIndex;
            if (sampleLimit is int limit && ++emitted == limit)
                return 0;
        } while (await timer.WaitForNextTickAsync(cancellation));

        return 0;
    }

    private static async Task<int> WatchHistory(string port, int? sampleLimit, CancellationToken cancellation)
    {
        using var http = OpenHttp(port);
        long? stream = null;
        ulong cursor = 0;
        var emitted = 0;
        Console.Error.WriteLine("Waiting for history input.");
        while (true)
        {
            cancellation.ThrowIfCancellationRequested();
            var path = "/v1/samples?after=" + cursor.ToString(CultureInfo.InvariantCulture);
            var batch = await ReadHttp<HistoryBatch>(http, path, cancellation);
            if (batch is null)
            {
                await Task.Delay(20, cancellation);
                continue;
            }

            if (stream != batch.StreamStartUtcUs)
            {
                stream = batch.StreamStartUtcUs;
                if (cursor != 0)
                {
                    cursor = 0;
                    continue;
                }
            }

            if (batch.Samples is null || batch.Samples.Length > 64)
                throw new InvalidDataException("Invalid EtherNet/IP Workbench history batch.");
            foreach (var sample in batch.Samples)
            {
                if (sample is null || sample.StreamStartUtcUs != stream || sample.SampleIndex <= cursor)
                    throw new InvalidDataException("History samples must have increasing indexes in the current stream.");

                var skipped = sample.SampleIndex - cursor - 1;
                var beforeHistory = batch.OldestSampleIndex is ulong oldest && oldest > cursor
                    ? Math.Min(skipped, oldest - cursor - 1)
                    : 0;
                Console.WriteLine(JsonSerializer.Serialize(new
                {
                    transport = "history",
                    batch.ActiveGeneration,
                    batch.OldestSampleIndex,
                    SkippedExportIndexes = skipped,
                    IndexesBeforeRetainedHistory = beforeHistory,
                    sample
                }, Json));
                cursor = sample.SampleIndex;
                if (sampleLimit is int limit && ++emitted == limit)
                    return 0;
            }
            if (batch.Samples.Length == 0 || batch.LatestSampleIndex is not ulong latest || cursor >= latest)
                await Task.Delay(20, cancellation);
        }
    }

    private static HttpClient OpenHttp(string port)
    {
        if (!ushort.TryParse(port, out var value) || value == 0)
            throw new ArgumentException("Enter a loopback HTTP port from 1 to 65535.");
        return new HttpClient(new HttpClientHandler { AllowAutoRedirect = false, UseProxy = false })
        {
            BaseAddress = new Uri($"http://127.0.0.1:{value}"),
            Timeout = TimeSpan.FromSeconds(1)
        };
    }

    private static async Task<T?> ReadHttp<T>(HttpClient http, string path, CancellationToken cancellation)
        where T : class, IFeedResponse
    {
        try
        {
            using var response = await http.GetAsync(path, cancellation);
            if (response.StatusCode == HttpStatusCode.ServiceUnavailable)
                return null;
            response.EnsureSuccessStatusCode();
            var data = await response.Content.ReadFromJsonAsync<T>(Json, cancellation);
            if (data?.SchemaVersion != 1)
                throw new InvalidDataException("Expected EtherNet/IP Workbench HTTP schema version 1.");
            return data;
        }
        catch (HttpRequestException error) when (error.StatusCode is null)
        {
            return null;
        }
        catch (OperationCanceledException) when (!cancellation.IsCancellationRequested)
        {
            return null;
        }
    }
}
