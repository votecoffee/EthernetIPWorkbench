using System.Text.Json;
using FeedReader;
using MQTTnet;
using MQTTnet.Exceptions;
using MQTTnet.Formatter;

namespace MqttFeedReader;

internal static class Program
{
    private static readonly TimeSpan ReconnectDelay = TimeSpan.FromMilliseconds(250);

    public static async Task<int> Main(string[] args)
    {
        if (args.Length is < 3 or > 5 || !ushort.TryParse(args[1], out var port) || port == 0)
        {
            Console.Error.WriteLine("Usage: MqttFeedReader BROKER PORT TOPIC [sample-count] [max-age-ms]");
            return 1;
        }

        using var stop = new CancellationTokenSource();
        ConsoleCancelEventHandler cancel = (_, signal) =>
        {
            signal.Cancel = true;
            stop.Cancel();
        };
        Console.CancelKeyPress += cancel;
        try
        {
            var topic = args[2];
            if (topic.Length == 0 || topic.Any(character => char.IsControl(character) || character is '#' or '+'))
                throw new ArgumentException("Enter the feed's exact MQTT topic.");
            var sampleLimit = args.Length >= 4 ? PositiveInt(args[3], "sample-count") : (int?)null;
            var maxAgeUs = args.Length == 5 ? (ulong)PositiveInt(args[4], "max-age-ms") * 1000 : 500_000UL;
            await Watch(args[0], port, topic, sampleLimit, maxAgeUs, stop.Token);
            return 0;
        }
        catch (OperationCanceledException) when (stop.IsCancellationRequested)
        {
            return 130;
        }
        catch (Exception error) when (error is ArgumentException or IOException or InvalidDataException)
        {
            Console.Error.WriteLine(error.Message);
            return 1;
        }
        finally
        {
            Console.CancelKeyPress -= cancel;
        }
    }

    private static int PositiveInt(string text, string name)
    {
        if (!int.TryParse(text, out var value) || value <= 0)
            throw new ArgumentException($"{name} requires a positive whole number.");
        return value;
    }

    private static async Task Watch(string host, ushort port, string topic, int? sampleLimit,
        ulong maxAgeUs, CancellationToken cancellation)
    {
        var factory = new MqttClientFactory();
        var clientId = "eipwb-net-" + Guid.NewGuid().ToString("N")[..12];
        var options = new MqttClientOptionsBuilder()
            .WithTcpServer(host, port)
            .WithProtocolVersion(MqttProtocolVersion.V311)
            .WithClientId(clientId)
            .WithCleanSession()
            .WithTimeout(TimeSpan.FromSeconds(3))
            .Build();
        var subscription = factory.CreateSubscribeOptionsBuilder()
            .WithTopicFilter(filter => filter.WithTopic(topic).WithAtMostOnceQoS())
            .Build();
        var cursor = new LatestSampleCursor();
        var completed = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var emitted = 0;
        while (!completed.Task.IsCompleted)
        {
            cancellation.ThrowIfCancellationRequested();
            using var client = factory.CreateMqttClient();
            var disconnected = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
            client.DisconnectedAsync += _ =>
            {
                disconnected.TrySetResult();
                return Task.CompletedTask;
            };
            client.ApplicationMessageReceivedAsync += message =>
            {
                if (completed.Task.IsCompleted || cancellation.IsCancellationRequested)
                    return Task.CompletedTask;
                try
                {
                    var sample = Decode(message.ApplicationMessage, topic);
                    if (sample?.EstimateAgeUs() is ulong age && age <= maxAgeUs &&
                        cursor.TryAdvance(sample, out var skipped))
                    {
                        Console.WriteLine(JsonSerializer.Serialize(new
                        {
                            transport = "mqtt",
                            Topic = topic,
                            EstimatedAgeUs = age,
                            SkippedExportIndexes = skipped,
                            sample
                        }, FeedJson.Options));
                        if (sampleLimit is int limit && ++emitted == limit)
                            completed.TrySetResult();
                    }
                }
                catch (IOException error)
                {
                    completed.TrySetException(error);
                }
                return Task.CompletedTask;
            };

            try
            {
                var connected = await client.ConnectAsync(options, cancellation);
                if (connected.ResultCode != MqttClientConnectResultCode.Success)
                    throw new InvalidDataException($"MQTT broker rejected the connection ({connected.ResultCode}).");
                var subscribed = await client.SubscribeAsync(subscription, cancellation);
                if (subscribed.Items.Count != 1 ||
                    subscribed.Items.Single().ResultCode != MqttClientSubscribeResultCode.GrantedQoS0)
                    throw new InvalidDataException($"MQTT broker did not grant the QoS 0 subscription to {topic}.");
                Console.Error.WriteLine($"Subscribed to {topic}.");
                await Task.WhenAny(completed.Task, disconnected.Task).WaitAsync(cancellation);
            }
            catch (MqttCommunicationException error)
            {
                Console.Error.WriteLine($"MQTT connection interrupted ({error.Message}).");
            }
            finally
            {
                await Disconnect(client);
            }

            if (!completed.Task.IsCompleted)
                await Task.Delay(ReconnectDelay, cancellation);
        }
        await completed.Task;
    }

    private static Sample? Decode(MqttApplicationMessage message, string topic)
    {
        if (message.Topic != topic || message.Retain)
            return null;
        try
        {
            using var document = JsonDocument.Parse(message.Payload);
            var root = document.RootElement;
            if (root.ValueKind != JsonValueKind.Object ||
                !root.TryGetProperty("schema_version", out var version) ||
                version.ValueKind != JsonValueKind.Number || !version.TryGetInt32(out var schema) || schema != 1)
                return null;
            var sample = root.Deserialize<Sample>(FeedJson.Options);
            return sample is { Generation: > 0, PayloadHex: not null } ? sample : null;
        }
        catch (JsonException)
        {
            return null;
        }
    }

    private static async Task Disconnect(IMqttClient client)
    {
        if (!client.IsConnected)
            return;
        using var deadline = new CancellationTokenSource(TimeSpan.FromSeconds(1));
        try
        {
            await client.DisconnectAsync(new MqttClientDisconnectOptionsBuilder().Build(), deadline.Token);
        }
        catch (Exception error) when (error is MqttCommunicationException or OperationCanceledException)
        {
            Console.Error.WriteLine($"MQTT disconnect ended early ({error.Message}).");
        }
    }
}
