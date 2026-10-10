package rastai.bakeoff;

import ai.djl.huggingface.tokenizers.HuggingFaceTokenizer;
import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.ArrayNode;
import com.fasterxml.jackson.databind.node.ObjectNode;
import com.sun.net.httpserver.HttpExchange;
import com.sun.net.httpserver.HttpServer;
import java.io.IOException;
import java.io.OutputStream;
import java.net.InetSocketAddress;
import java.net.URI;
import java.net.http.HttpClient;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;
import java.nio.charset.StandardCharsets;
import java.nio.file.Path;
import java.sql.Connection;
import java.sql.DriverManager;
import java.sql.PreparedStatement;
import java.sql.SQLException;
import java.sql.Statement;
import java.util.ArrayList;
import java.util.Iterator;
import java.util.List;
import java.util.Set;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.Executors;
import java.util.concurrent.atomic.AtomicLong;
import java.util.concurrent.locks.ReentrantLock;
import java.util.stream.Stream;

/**
 * Minimal OpenAI-compatible gateway (spec v5 §18), the part of the backend every model call goes through.
 *
 * <p>POST /v1/chat/completions (streaming):
 * <ol>
 *   <li>render the Gemma chat template and tokenize it here: the gateway owns tokenization;</li>
 *   <li>send the token IDs upstream to llama-server's /completion, streaming;</li>
 *   <li>relay each upstream event to the client as an OpenAI "chat.completion.chunk";</li>
 *   <li>log the prompt and generated token IDs to SQLite, then finish with "data: [DONE]".</li>
 * </ol>
 * GET /health answers {"status":"ok"} once the server is ready. Each request runs on its own virtual thread.
 */
final class Gateway {
    private static final String LOOPBACK = "127.0.0.1";
    private static final int CONNECTION_BACKLOG = 1024;
    private static final long BEGIN_TEXT_ID = 2;
    private static final int DEFAULT_MAX_TOKENS = 64;
    private static final String DATA_PREFIX = "data: ";
    private static final byte[] DONE = "data: [DONE]\n\n".getBytes(StandardCharsets.UTF_8);
    private static final ObjectMapper JSON = new ObjectMapper();

    private final HuggingFaceTokenizer tokenizer;
    private final URI upstreamCompletion;
    private final HttpClient client = HttpClient.newBuilder().version(HttpClient.Version.HTTP_1_1).build();
    private final PreparedStatement logInsert;
    private final ReentrantLock logLock = new ReentrantLock(); // one SQLite connection, one writer at a time
    private final AtomicLong nextRequestNumber = new AtomicLong();
    // The JDK server swallows handler exceptions silently; report each kind once instead.
    private static final Set<String> REPORTED_FAILURES = ConcurrentHashMap.newKeySet();

    private Gateway(HuggingFaceTokenizer tokenizer, URI upstreamCompletion, Connection log) throws SQLException {
        this.tokenizer = tokenizer;
        this.upstreamCompletion = upstreamCompletion;
        this.logInsert = log.prepareStatement(
                "INSERT INTO gateway_log (request_id, prompt_ids, generated_ids, created_unix_ms) VALUES (?, ?, ?, ?)");
    }

    static void command(String[] arguments) throws IOException, SQLException {
        // Must be set before the server class loads. Without it, streaming small chunks stalls ~40 ms per
        // response, where Nagle's algorithm meets the client's delayed ACKs.
        System.setProperty("sun.net.httpserver.nodelay", "true");

        int port = Main.intArgument(arguments, 0, "port");
        URI upstream = URI.create(Main.argument(arguments, 1, "upstream-url") + "/completion");
        HuggingFaceTokenizer tokenizer = TokenizeBench.loadTokenizer(Path.of(Main.argument(arguments, 2, "tokenizer.json")));
        Gateway gateway = new Gateway(tokenizer, upstream, openLog(Main.argument(arguments, 3, "log.sqlite")));

        HttpServer server = HttpServer.create(new InetSocketAddress(LOOPBACK, port), CONNECTION_BACKLOG);
        server.setExecutor(Executors.newVirtualThreadPerTaskExecutor());
        server.createContext("/health", Gateway::health);
        server.createContext("/v1/chat/completions", gateway::chatCompletions);
        server.start();
    }

    // ---- Prompt ----------------------------------------------------------------------------------------

    private static String renderChatTemplate(JsonNode messages) {
        StringBuilder prompt = new StringBuilder();
        for (JsonNode message : messages) {
            String role = message.path("role").asText();
            String turnRole = role.equals("assistant") ? "model" : role;
            prompt.append("<start_of_turn>").append(turnRole).append('\n')
                    .append(message.path("content").asText()).append("<end_of_turn>\n");
        }
        return prompt.append("<start_of_turn>model\n").toString();
    }

    private long[] promptTokenIds(JsonNode messages) {
        long[] encoded = tokenizer.encode(renderChatTemplate(messages)).getIds();
        long[] ids = new long[encoded.length + 1];
        ids[0] = BEGIN_TEXT_ID;
        System.arraycopy(encoded, 0, ids, 1, encoded.length);
        return ids;
    }

    // ---- Streaming -------------------------------------------------------------------------------------

    private static byte[] openAiChunk(String requestId, String content) throws IOException {
        ObjectNode chunk = JSON.createObjectNode();
        chunk.put("id", requestId);
        chunk.put("object", "chat.completion.chunk");
        ObjectNode choice = chunk.putArray("choices").addObject();
        choice.put("index", 0);
        ObjectNode delta = choice.putObject("delta");
        if (content != null) {
            delta.put("content", content);
            choice.putNull("finish_reason");
        } else {
            choice.put("finish_reason", "stop");
        }
        return (DATA_PREFIX + JSON.writeValueAsString(chunk) + "\n\n").getBytes(StandardCharsets.UTF_8);
    }

    private HttpRequest upstreamRequest(long[] promptIds, int maxTokens) throws IOException {
        ObjectNode body = JSON.createObjectNode();
        ArrayNode prompt = body.putArray("prompt");
        for (long id : promptIds) {
            prompt.add(id);
        }
        body.put("n_predict", maxTokens);
        body.put("stream", true);
        body.put("return_tokens", true);
        body.put("cache_prompt", false);
        body.put("temperature", 0.0);
        body.put("top_k", 1);
        return HttpRequest.newBuilder(upstreamCompletion)
                .header("Content-Type", "application/json")
                .POST(HttpRequest.BodyPublishers.ofString(JSON.writeValueAsString(body)))
                .build();
    }

    /** Relays upstream events as OpenAI chunks; returns the generated token IDs. */
    private static List<Long> relay(Iterator<String> upstreamLines, OutputStream client, String requestId) throws IOException {
        List<Long> generatedIds = new ArrayList<>();
        while (upstreamLines.hasNext()) {
            String line = upstreamLines.next();
            if (!line.startsWith(DATA_PREFIX)) {
                continue;
            }
            JsonNode event = JSON.readTree(line.substring(DATA_PREFIX.length()));
            for (JsonNode token : event.path("tokens")) {
                generatedIds.add(token.asLong());
            }
            String content = event.path("content").asText("");
            if (!content.isEmpty()) {
                client.write(openAiChunk(requestId, content));
                client.flush();
            }
            if (event.path("stop").asBoolean(false)) {
                break;
            }
        }
        return generatedIds;
    }

    // ---- Logging ---------------------------------------------------------------------------------------

    private static Connection openLog(String path) throws SQLException {
        Connection connection = DriverManager.getConnection("jdbc:sqlite:" + path);
        try (Statement statement = connection.createStatement()) {
            statement.execute("PRAGMA journal_mode = WAL");
            statement.execute("PRAGMA synchronous = NORMAL");
            statement.execute("CREATE TABLE IF NOT EXISTS gateway_log (request_id TEXT PRIMARY KEY, prompt_ids TEXT NOT NULL, "
                    + "generated_ids TEXT NOT NULL, created_unix_ms INTEGER NOT NULL)");
        }
        return connection;
    }

    private void logTokenIds(String requestId, long[] promptIds, List<Long> generatedIds) throws IOException {
        String prompt = JSON.writeValueAsString(promptIds);
        String generated = JSON.writeValueAsString(generatedIds);
        logLock.lock();
        try {
            logInsert.setString(1, requestId);
            logInsert.setString(2, prompt);
            logInsert.setString(3, generated);
            logInsert.setLong(4, System.currentTimeMillis());
            logInsert.executeUpdate();
        } catch (SQLException error) {
            throw new IOException("could not log request " + requestId, error);
        } finally {
            logLock.unlock();
        }
    }

    // ---- HTTP ------------------------------------------------------------------------------------------

    private void chatCompletions(HttpExchange exchange) throws IOException {
        try {
            serveChat(exchange);
        } catch (IOException | RuntimeException failure) {
            String kind = failure.getClass().getName() + ": " + failure.getMessage();
            if (REPORTED_FAILURES.add(kind)) {
                System.err.println("request failed: " + kind);
            }
            throw failure;
        }
    }

    private void serveChat(HttpExchange exchange) throws IOException {
        try (exchange) {
            if (!exchange.getRequestMethod().equals("POST")) {
                exchange.sendResponseHeaders(405, -1);
                return;
            }
            JsonNode request = JSON.readTree(exchange.getRequestBody());
            long[] promptIds = promptTokenIds(request.path("messages"));
            int maxTokens = request.path("max_tokens").asInt(DEFAULT_MAX_TOKENS);
            String requestId = "chatcmpl-" + nextRequestNumber.getAndIncrement();

            HttpResponse<Stream<String>> upstream;
            try {
                upstream = client.send(upstreamRequest(promptIds, maxTokens), HttpResponse.BodyHandlers.ofLines());
            } catch (InterruptedException interrupted) {
                Thread.currentThread().interrupt();
                exchange.sendResponseHeaders(503, -1);
                return;
            }

            exchange.getResponseHeaders().set("Content-Type", "text/event-stream");
            exchange.getResponseHeaders().set("Cache-Control", "no-cache");
            exchange.sendResponseHeaders(200, 0); // 0 = streamed (chunked) body
            try (Stream<String> lines = upstream.body(); OutputStream client = exchange.getResponseBody()) {
                List<Long> generatedIds = relay(lines.iterator(), client, requestId);
                logTokenIds(requestId, promptIds, generatedIds);
                client.write(openAiChunk(requestId, null));
                client.write(DONE);
                client.flush();
            }
        }
    }

    private static void health(HttpExchange exchange) throws IOException {
        try (exchange) {
            byte[] body = "{\"status\":\"ok\"}".getBytes(StandardCharsets.UTF_8);
            exchange.getResponseHeaders().set("Content-Type", "application/json");
            exchange.sendResponseHeaders(200, body.length);
            exchange.getResponseBody().write(body);
        }
    }
}
