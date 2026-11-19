using System.Buffers.Binary;
using System.IO.MemoryMappedFiles;

namespace FeedReader;

internal sealed class SharedMemoryReader
{
    private const int HeaderBytes = 64;
    private const int MaxPayloadBytes = 509;
    private readonly string prefix;
    private readonly byte[] buffer = new byte[HeaderBytes + MaxPayloadBytes];

    public SharedMemoryReader(string name)
    {
        if (name.Length is < 1 or > 64 ||
            !name.All(character => char.IsAsciiLetterOrDigit(character) || character is '-' or '_'))
            throw new ArgumentException("Enter a shared-memory name using 1–64 letters, digits, hyphens or underscores.");
        prefix = $@"Local\EthernetIPWorkbench.{name}";
    }

    public Snapshot? Read()
    {
        if (!OperatingSystem.IsWindows())
            throw new PlatformNotSupportedException("Named shared memory requires Windows.");

        try
        {
            // Dispose handles every poll so a stopped publisher can reuse the name.
            using var gate = Mutex.OpenExisting(prefix + ".lock");
            using var mapping = MemoryMappedFile.OpenExisting(prefix + ".data", MemoryMappedFileRights.Read);
            using var view = mapping.CreateViewAccessor(0, buffer.Length, MemoryMappedFileAccess.Read);

            try
            {
                if (!gate.WaitOne(0))
                    return null;
            }
            catch (AbandonedMutexException)
            {
                // An abandoned wait grants ownership. Release it and discard this snapshot.
                gate.ReleaseMutex();
                return null;
            }

            try
            {
                if (view.ReadArray(0, buffer, 0, buffer.Length) != buffer.Length)
                    throw new InvalidDataException("Incomplete EIPWB001 snapshot.");
            }
            finally
            {
                gate.ReleaseMutex();
            }
        }
        catch (Exception error) when (error is WaitHandleCannotBeOpenedException or FileNotFoundException)
        {
            return null;
        }

        return Decode(buffer);
    }

    private static Snapshot? Decode(ReadOnlySpan<byte> bytes)
    {
        if (BinaryPrimitives.ReadUInt64LittleEndian(bytes) == 0)
            return null;
        if (!bytes[..8].SequenceEqual("EIPWB001"u8))
            throw new InvalidDataException("Expected the EIPWB001 shared-memory layout.");

        var index = BinaryPrimitives.ReadUInt64LittleEndian(bytes[16..]);
        if (index == 0)
            return null;
        var active = BinaryPrimitives.ReadUInt64LittleEndian(bytes[32..]);
        var startUs = BinaryPrimitives.ReadInt64LittleEndian(bytes[8..]);
        var elapsedUs = BinaryPrimitives.ReadUInt64LittleEndian(bytes[40..]);
        var length = BinaryPrimitives.ReadUInt16LittleEndian(bytes[56..]);
        if (length > MaxPayloadBytes || startUs <= 0 || elapsedUs > (ulong)(long.MaxValue - startUs))
            throw new InvalidDataException("Invalid EIPWB001 payload size or timestamp.");

        var sample = new Sample(
            startUs,
            index,
            BinaryPrimitives.ReadUInt64LittleEndian(bytes[24..]),
            elapsedUs,
            BinaryPrimitives.ReadUInt32LittleEndian(bytes[48..]),
            BinaryPrimitives.ReadUInt16LittleEndian(bytes[52..]),
            BinaryPrimitives.ReadUInt16LittleEndian(bytes[54..]),
            Convert.ToHexString(bytes.Slice(HeaderBytes, length)),
            null);
        return new Snapshot(active, sample.EstimateAgeUs(), sample);
    }
}
