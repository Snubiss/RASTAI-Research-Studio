package rastai.bakeoff;

import java.util.Arrays;

/**
 * Java side of the backend bake-off (spike experiment 07). The Rust side implements the same
 * command-line contract, so the experiment can run both and compare.
 *
 * <pre>
 *   safetensors-read &lt;file&gt;                   read and checksum every tensor, print JSON
 *   safetensors-validate &lt;folder&gt;             accept or reject every file in a folder, print JSON
 *   tokenize-bench &lt;tokenizer.json&gt; &lt;corpus.json&gt; &lt;expected.json&gt; &lt;repeats&gt;
 *   gateway &lt;port&gt; &lt;upstream-url&gt; &lt;tokenizer.json&gt; &lt;log.sqlite&gt;
 * </pre>
 */
public final class Main {
    private static final String USAGE = "usage: bakeoff-java <safetensors-read|safetensors-validate|tokenize-bench|gateway> ...";

    private Main() {}

    public static void main(String[] arguments) throws Exception {
        // DJL "calls home" with usage telemetry and probes the AWS metadata address (169.254.169.254) at
        // startup; off AWS that costs ~4 s of timeouts. Rastai promises no telemetry (spec §24), so: off.
        System.setProperty("OPT_OUT_TRACKING", "true");
        if (arguments.length == 0) {
            System.err.println(USAGE);
            System.exit(2);
        }
        String[] rest = Arrays.copyOfRange(arguments, 1, arguments.length);
        try {
            switch (arguments[0]) {
                case "safetensors-read" -> Safetensors.readCommand(rest);
                case "safetensors-validate" -> Safetensors.validateCommand(rest);
                case "tokenize-bench" -> TokenizeBench.command(rest);
                case "gateway" -> Gateway.command(rest);
                default -> throw new IllegalArgumentException("unknown command " + arguments[0] + "\n" + USAGE);
            }
        } catch (IllegalArgumentException | Safetensors.FormatException error) {
            System.err.println("error: " + error.getMessage());
            System.exit(1);
        }
    }

    /** The argument at {@code index}, or an error naming what was expected there. */
    static String argument(String[] arguments, int index, String name) {
        if (index >= arguments.length) {
            throw new IllegalArgumentException("missing argument <" + name + ">");
        }
        return arguments[index];
    }

    static int intArgument(String[] arguments, int index, String name) {
        try {
            return Integer.parseInt(argument(arguments, index, name));
        } catch (NumberFormatException error) {
            throw new IllegalArgumentException("<" + name + "> is not a valid number");
        }
    }
}
