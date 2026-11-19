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
    Dictionary<string, string?>? Values)
{
    public ulong? EstimateAgeUs()
    {
        if (StreamStartUtcUs <= 0 || ElapsedUs > (ulong)(long.MaxValue - StreamStartUtcUs))
            return null;

        var sampleUs = StreamStartUtcUs + (long)ElapsedUs;
        var nowUs = (DateTime.UtcNow.Ticks - DateTime.UnixEpoch.Ticks) / TimeSpan.TicksPerMicrosecond;
        return nowUs >= sampleUs ? (ulong)(nowUs - sampleUs) : null;
    }
}

internal static class FeedJson
{
    public static readonly JsonSerializerOptions Options = new()
    {
        PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower
    };
}

internal sealed class LatestSampleCursor
{
    private long? stream;
    private ulong lastIndex;

    public bool TryAdvance(Sample sample, out ulong skipped)
    {
        skipped = 0;
        if (sample.SampleIndex == 0)
            return false;
        if (stream != sample.StreamStartUtcUs)
        {
            stream = sample.StreamStartUtcUs;
            lastIndex = 0;
        }
        if (sample.SampleIndex <= lastIndex)
            return false;

        skipped = lastIndex == 0 ? 0 : sample.SampleIndex - lastIndex - 1;
        lastIndex = sample.SampleIndex;
        return true;
    }
}
