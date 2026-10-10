package rastai.bakeoff;

import ai.djl.huggingface.tokenizers.HuggingFaceTokenizer;
import com.fasterxml.jackson.core.type.TypeReference;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.ObjectNode;
import java.io.IOException;
import java.nio.file.Path;
import java.util.Arrays;
import java.util.List;
import java.util.Map;

/**
 * Tokenizer parity and speed: encode the stress corpus, compare with the expected IDs, then time
 * {@code repeats} passes over the whole corpus on one thread.
 */
final class TokenizeBench {
    private static final ObjectMapper JSON = new ObjectMapper();
    // The gateway adds <bos> itself, so the tokenizer must not. DJL also truncates long inputs by default
    // (silently, to a few hundred tokens), which a gateway that must log exact token IDs cannot allow.
    private static final Map<String, String> TOKENIZER_OPTIONS = Map.of("addSpecialTokens", "false", "truncation", "false");

    private TokenizeBench() {}

    static HuggingFaceTokenizer loadTokenizer(Path tokenizerPath) throws IOException {
        return HuggingFaceTokenizer.newInstance(tokenizerPath, TOKENIZER_OPTIONS);
    }

    static void command(String[] arguments) throws IOException {
        Path tokenizerPath = Path.of(Main.argument(arguments, 0, "tokenizer.json"));
        List<String> corpus = JSON.readValue(Path.of(Main.argument(arguments, 1, "corpus.json")).toFile(), new TypeReference<>() {});
        List<long[]> expected = JSON.readValue(Path.of(Main.argument(arguments, 2, "expected.json")).toFile(), new TypeReference<>() {});
        int repeats = Main.intArgument(arguments, 3, "repeats");

        long loadStart = System.nanoTime();
        try (HuggingFaceTokenizer tokenizer = loadTokenizer(tokenizerPath)) {
            double loadMilliseconds = (System.nanoTime() - loadStart) / 1e6;

            int mismatches = 0;
            long tokenCount = 0;
            for (int index = 0; index < corpus.size(); index++) {
                long[] ids = tokenizer.encode(corpus.get(index)).getIds();
                tokenCount += ids.length;
                if (!Arrays.equals(ids, expected.get(index))) {
                    mismatches++;
                    System.err.println("mismatch in case " + index + ": " + ids.length + " ids, expected " + expected.get(index).length);
                }
            }

            long timingStart = System.nanoTime();
            long checksum = 0; // consumed below, so the JIT can't skip the work
            for (int repeat = 0; repeat < repeats; repeat++) {
                for (String text : corpus) {
                    checksum += tokenizer.encode(text).getIds().length;
                }
            }
            double seconds = (System.nanoTime() - timingStart) / 1e9;

            ObjectNode output = JSON.createObjectNode();
            output.put("cases", corpus.size());
            output.put("tokens", tokenCount);
            output.put("mismatches", mismatches);
            output.put("load_ms", loadMilliseconds);
            output.put("tokens_per_second", tokenCount * repeats / seconds);
            output.put("checksum", checksum);
            System.out.println(JSON.writeValueAsString(output));
        }
    }
}
