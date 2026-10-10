package rastai.bakeoff;

import com.fasterxml.jackson.core.JsonProcessingException;
import com.fasterxml.jackson.core.StreamReadFeature;
import com.fasterxml.jackson.databind.DeserializationFeature;
import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.json.JsonMapper;
import com.fasterxml.jackson.databind.node.ObjectNode;
import java.io.IOException;
import java.lang.foreign.Arena;
import java.lang.foreign.MemorySegment;
import java.lang.foreign.ValueLayout;
import java.nio.ByteOrder;
import java.nio.channels.FileChannel;
import java.nio.charset.CharacterCodingException;
import java.nio.charset.CodingErrorAction;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardOpenOption;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.List;
import java.util.Map;
import java.util.OptionalLong;
import java.util.stream.Stream;
import java.util.zip.CRC32C;

/**
 * Strict safetensors reader: the same contract as the Python reader in src/rastai_spike/safetensors_reader.py.
 *
 * <ul>
 *   <li>the header is a UTF-8 JSON object with no duplicate keys, at most 100 MB;</li>
 *   <li>"__metadata__", if present, maps strings to strings;</li>
 *   <li>every other entry has a known dtype, a shape of non-negative integers, and integer data_offsets
 *       [start, end] whose length equals the shape's byte size, computed without overflow;</li>
 *   <li>the tensors tile the data section exactly: contiguous, no overlaps, no gaps, nothing left over.</li>
 * </ul>
 */
final class Safetensors {
    private static final int HEADER_LENGTH_BYTES = 8;
    private static final long MAX_HEADER_BYTES = 100L * 1024 * 1024;
    private static final String METADATA_KEY = "__metadata__";
    private static final ValueLayout.OfLong LITTLE_ENDIAN_LONG = ValueLayout.JAVA_LONG_UNALIGNED.withOrder(ByteOrder.LITTLE_ENDIAN);
    private static final Map<String, Long> BYTES_PER_ELEMENT = Map.ofEntries(
            Map.entry("F64", 8L), Map.entry("I64", 8L), Map.entry("U64", 8L),
            Map.entry("F32", 4L), Map.entry("I32", 4L), Map.entry("U32", 4L),
            Map.entry("F16", 2L), Map.entry("BF16", 2L), Map.entry("I16", 2L), Map.entry("U16", 2L),
            Map.entry("F8_E4M3", 1L), Map.entry("F8_E5M2", 1L), Map.entry("I8", 1L), Map.entry("U8", 1L), Map.entry("BOOL", 1L));

    // Duplicate keys are an error (not "last one wins"), and nothing may follow the JSON object.
    private static final ObjectMapper JSON = JsonMapper.builder()
            .enable(StreamReadFeature.STRICT_DUPLICATE_DETECTION)
            .enable(DeserializationFeature.FAIL_ON_TRAILING_TOKENS)
            .build();

    private Safetensors() {}

    /** The file is not a well-formed safetensors file. */
    static final class FormatException extends Exception {
        FormatException(String message) {
            super(message);
        }
    }

    record TensorEntry(String name, long start, long end) {}

    /** A memory-mapped safetensors file. Close it to unmap. */
    static final class SafetensorsFile implements AutoCloseable {
        private final Arena arena;
        private final MemorySegment mapping;
        private final long dataStart;
        final List<TensorEntry> entries; // sorted by position in the data section

        private SafetensorsFile(Arena arena, MemorySegment mapping, long dataStart, List<TensorEntry> entries) {
            this.arena = arena;
            this.mapping = mapping;
            this.dataStart = dataStart;
            this.entries = entries;
        }

        static SafetensorsFile open(Path path) throws IOException, FormatException {
            try (FileChannel channel = FileChannel.open(path, StandardOpenOption.READ)) {
                long fileLength = channel.size();
                if (fileLength < HEADER_LENGTH_BYTES) {
                    throw new FormatException("file is shorter than the 8-byte header length");
                }
                Arena arena = Arena.ofConfined();
                try {
                    MemorySegment mapping = channel.map(FileChannel.MapMode.READ_ONLY, 0, fileLength, arena);
                    long headerLength = readHeaderLength(mapping);
                    long dataStart = HEADER_LENGTH_BYTES + headerLength;
                    JsonNode header = parseHeader(mapping.asSlice(HEADER_LENGTH_BYTES, headerLength));
                    List<TensorEntry> entries = buildEntries(header, fileLength - dataStart);
                    return new SafetensorsFile(arena, mapping, dataStart, entries);
                } catch (IOException | FormatException | RuntimeException error) {
                    arena.close(); // a rejected file must not stay mapped
                    throw error;
                }
            }
        }

        MemorySegment tensorBytes(TensorEntry entry) {
            return mapping.asSlice(dataStart + entry.start(), entry.end() - entry.start());
        }

        @Override
        public void close() {
            arena.close();
        }
    }

    private static long readHeaderLength(MemorySegment mapping) throws FormatException {
        long headerLength = mapping.get(LITTLE_ENDIAN_LONG, 0);
        // The length is an unsigned 64-bit number; Java's long is signed, so compare it as unsigned.
        if (Long.compareUnsigned(headerLength, MAX_HEADER_BYTES) > 0) {
            throw new FormatException("header is " + Long.toUnsignedString(headerLength) + " bytes, over the " + MAX_HEADER_BYTES + " byte limit");
        }
        if (HEADER_LENGTH_BYTES + headerLength > mapping.byteSize()) {
            throw new FormatException("header length points past the end of the file");
        }
        return headerLength;
    }

    private static JsonNode parseHeader(MemorySegment headerBytes) throws FormatException {
        String text;
        try {
            text = StandardCharsets.UTF_8.newDecoder()
                    .onMalformedInput(CodingErrorAction.REPORT)
                    .onUnmappableCharacter(CodingErrorAction.REPORT)
                    .decode(headerBytes.asByteBuffer())
                    .toString();
        } catch (CharacterCodingException error) {
            throw new FormatException("header is not valid UTF-8");
        }
        try {
            JsonNode header = JSON.readTree(text);
            if (header == null || !header.isObject()) {
                throw new FormatException("header must be a JSON object");
            }
            return header;
        } catch (JsonProcessingException error) {
            throw new FormatException("header is not a valid JSON object: " + error.getOriginalMessage());
        }
    }

    /** A JSON integer that fits in a long and is not negative. Floats, booleans and huge numbers are rejected. */
    private static OptionalLong plainUnsigned(JsonNode value) {
        if (value != null && value.isIntegralNumber() && value.canConvertToLong() && value.longValue() >= 0) {
            return OptionalLong.of(value.longValue());
        }
        return OptionalLong.empty();
    }

    private static List<TensorEntry> buildEntries(JsonNode header, long dataLength) throws FormatException {
        List<TensorEntry> entries = new ArrayList<>();
        for (Map.Entry<String, JsonNode> field : header.properties()) {
            if (field.getKey().equals(METADATA_KEY)) {
                checkMetadata(field.getValue());
            } else {
                entries.add(parseEntry(field.getKey(), field.getValue(), dataLength));
            }
        }
        entries.sort(Comparator.comparingLong(TensorEntry::start).thenComparingLong(TensorEntry::end));
        checkExactTiling(entries, dataLength);
        return entries;
    }

    private static void checkMetadata(JsonNode metadata) throws FormatException {
        if (!metadata.isObject()) {
            throw new FormatException("__metadata__ must map strings to strings");
        }
        for (Map.Entry<String, JsonNode> field : metadata.properties()) {
            if (!field.getValue().isTextual()) {
                throw new FormatException("__metadata__ must map strings to strings");
            }
        }
    }

    private static TensorEntry parseEntry(String name, JsonNode description, long dataLength) throws FormatException {
        if (!description.isObject()) {
            throw new FormatException(name + ": entry must be an object");
        }
        JsonNode dtype = description.get("dtype");
        Long elementSize = dtype != null && dtype.isTextual() ? BYTES_PER_ELEMENT.get(dtype.asText()) : null;
        if (elementSize == null) {
            throw new FormatException(name + ": unsupported dtype " + dtype);
        }

        long expectedBytes = shapeByteSize(name, description.get("shape"), elementSize);
        long[] offsets = dataOffsets(name, description.get("data_offsets"));
        long start = offsets[0];
        long end = offsets[1];
        if (!(start <= end && end <= dataLength)) {
            throw new FormatException(name + ": data_offsets [" + start + ", " + end + "] fall outside the data section");
        }
        if (end - start != expectedBytes) {
            throw new FormatException(name + ": shape needs " + expectedBytes + " bytes but offsets cover " + (end - start));
        }
        return new TensorEntry(name, start, end);
    }

    private static long shapeByteSize(String name, JsonNode shape, long elementSize) throws FormatException {
        if (shape == null || !shape.isArray()) {
            throw new FormatException(name + ": shape must be a list of non-negative integers");
        }
        long total = elementSize;
        for (JsonNode size : shape) {
            OptionalLong value = plainUnsigned(size);
            if (value.isEmpty()) {
                throw new FormatException(name + ": shape must be a list of non-negative integers");
            }
            try {
                // multiplyExact: a shape like [2^32, 2^32] must not wrap around and "match" empty offsets.
                total = Math.multiplyExact(total, value.getAsLong());
            } catch (ArithmeticException overflow) {
                throw new FormatException(name + ": shape's byte size overflows 64 bits");
            }
        }
        return total;
    }

    private static long[] dataOffsets(String name, JsonNode offsets) throws FormatException {
        if (offsets == null || !offsets.isArray() || offsets.size() != 2) {
            throw new FormatException(name + ": data_offsets must be [start, end]");
        }
        OptionalLong start = plainUnsigned(offsets.get(0));
        OptionalLong end = plainUnsigned(offsets.get(1));
        if (start.isEmpty() || end.isEmpty()) {
            throw new FormatException(name + ": data_offsets must be [start, end]");
        }
        return new long[] {start.getAsLong(), end.getAsLong()};
    }

    private static void checkExactTiling(List<TensorEntry> entries, long dataLength) throws FormatException {
        long expectedStart = 0;
        for (TensorEntry entry : entries) {
            if (entry.start() < expectedStart) {
                throw new FormatException(entry.name() + " overlaps the tensor before it");
            }
            if (entry.start() > expectedStart) {
                throw new FormatException("gap in the data section before " + entry.name());
            }
            expectedStart = entry.end();
        }
        if (expectedStart != dataLength) {
            throw new FormatException((dataLength - expectedStart) + " bytes at the end of the data section belong to no tensor");
        }
    }

    // ---- Commands ------------------------------------------------------------------------------------

    static void readCommand(String[] arguments) throws IOException, FormatException {
        Path path = Path.of(Main.argument(arguments, 0, "file"));

        long headerStart = System.nanoTime();
        try (SafetensorsFile file = SafetensorsFile.open(path)) {
            double headerMilliseconds = (System.nanoTime() - headerStart) / 1e6;

            long readStart = System.nanoTime();
            CRC32C checksum = new CRC32C();
            long totalBytes = 0;
            for (TensorEntry entry : file.entries) {
                MemorySegment bytes = file.tensorBytes(entry);
                checksum.update(bytes.asByteBuffer());
                totalBytes += bytes.byteSize();
            }
            double readMilliseconds = (System.nanoTime() - readStart) / 1e6;

            ObjectNode output = JSON.createObjectNode();
            output.put("tensors", file.entries.size());
            output.put("bytes", totalBytes);
            output.put("crc32c", checksum.getValue());
            output.put("header_ms", headerMilliseconds);
            output.put("read_ms", readMilliseconds);
            System.out.println(JSON.writeValueAsString(output));
        }
    }

    static void validateCommand(String[] arguments) throws IOException {
        Path folder = Path.of(Main.argument(arguments, 0, "folder"));
        List<Path> paths;
        try (Stream<Path> listing = Files.list(folder)) {
            paths = listing.sorted().toList();
        }

        ObjectNode verdicts = JSON.createObjectNode();
        for (Path path : paths) {
            ObjectNode verdict = verdicts.putObject(path.getFileName().toString());
            try (SafetensorsFile file = SafetensorsFile.open(path)) {
                verdict.put("accepted", true);
                verdict.put("tensors", file.entries.size());
            } catch (FormatException error) {
                verdict.put("accepted", false);
                verdict.put("error", error.getMessage());
            } catch (RuntimeException unexpected) {
                // Reported separately: an exception here is a bug in the reader, not a clean rejection.
                verdict.put("accepted", false);
                verdict.put("error", "UNEXPECTED " + unexpected);
            }
        }
        System.out.println(JSON.writeValueAsString(verdicts));
    }
}
