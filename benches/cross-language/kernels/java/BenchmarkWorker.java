import java.io.BufferedReader;
import java.io.InputStreamReader;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.util.HexFormat;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

/** Persistent newline-delimited JSON worker for the CalcKernel benchmark suite. */
public final class BenchmarkWorker {
    private static final long WARMUP_NS = 500_000_000L;
    private static final Pattern STRING_FIELD = Pattern.compile("\\\"([^\\\"]+)\\\"\\s*:\\s*\\\"([^\\\"]*)\\\"");
    private static final Pattern NUMBER_FIELD = Pattern.compile("\\\"([^\\\"]+)\\\"\\s*:\\s*(-?\\d+)");

    private enum Case { MATMUL, CONVOLVE }

    @FunctionalInterface
    private interface Kernel {
        void run();
    }

    private static Case activeCase;
    private static int size;
    private static double[] left;
    private static double[] right;
    private static double[] output;
    private static Kernel ordinary;
    private static Kernel tuned;

    private BenchmarkWorker() {}

    public static void main(String[] args) throws Exception {
        try (BufferedReader input = new BufferedReader(new InputStreamReader(System.in, StandardCharsets.UTF_8))) {
            String line;
            while ((line = input.readLine()) != null) {
                try {
                    System.out.println(handle(line));
                } catch (Exception error) {
                    System.out.println("{\"error\":" + quote(error.getMessage() == null ? error.toString() : error.getMessage()) + "}");
                }
                System.out.flush();
            }
        }
    }

    private static String handle(String json) throws Exception {
        String command = stringField(json, "cmd");
        if ("init".equals(command)) {
            initialize(stringField(json, "case"), intField(json, "size"));
            long ordinaryWarmup = warm(ordinary);
            long tunedWarmup = warm(tuned);
            long totalWarmupMs = (ordinaryWarmup + tunedWarmup) / 1_000_000L;
            return "{\"ready\":true,\"case\":" + quote(activeCase.name().toLowerCase())
                    + ",\"size\":" + size
                    + ",\"warmupMs\":" + totalWarmupMs
                    + ",\"javaVersion\":" + quote(System.getProperty("java.version")) + "}";
        }
        if ("run".equals(command)) {
            if (activeCase == null) throw new IllegalArgumentException("send init before run");
            String mode = stringField(json, "mode");
            int repeat = intField(json, "repeat");
            if (repeat < 1) throw new IllegalArgumentException("repeat must be an integer >= 1");
            Kernel kernel;
            if ("ordinary".equals(mode)) kernel = ordinary;
            else if ("tuned".equals(mode)) kernel = tuned;
            else throw new IllegalArgumentException("mode must be ordinary or tuned");

            long started = System.nanoTime();
            for (int index = 0; index < repeat; index++) kernel.run();
            long elapsed = System.nanoTime() - started;
            return "{\"durationNs\":" + elapsed + ",\"sha256\":" + quote(hashOutput()) + "}";
        }
        throw new IllegalArgumentException("cmd must be init or run");
    }

    private static void initialize(String caseName, int n) {
        if ("matmul".equals(caseName)) activeCase = Case.MATMUL;
        else if ("convolve".equals(caseName)) activeCase = Case.CONVOLVE;
        else throw new IllegalArgumentException("case must be matmul or convolve");
        if (n < 1 || (activeCase == Case.CONVOLVE && n < 3)) {
            throw new IllegalArgumentException("size must be positive (at least 3 for convolve)");
        }
        size = n;
        int count = Math.multiplyExact(n, n);
        left = new double[count];
        right = activeCase == Case.MATMUL ? new double[count] : null;
        output = new double[count];

        if (activeCase == Case.MATMUL) {
            for (int row = 0; row < n; row++) {
                int offset = row * n;
                for (int col = 0; col < n; col++) {
                    int index = offset + col;
                    left[index] = (((row * 17 + col * 13) % 31) - 15) / 32.0;
                    right[index] = (((row * 11 + col * 7) % 29) - 14) / 32.0;
                }
            }
            ordinary = () -> matmulOrdinary(left, right, output, size);
            tuned = () -> matmulTuned(left, right, output, size);
        } else {
            for (int index = 0; index < count; index++) {
                left[index] = (((index * 17) % 37) - 18) / 32.0;
            }
            ordinary = () -> convolveOrdinary(left, output, size);
            tuned = () -> convolveTuned(left, output, size);
        }
    }

    /** Warm one route for at least 500 ms, including JIT compilation and optimization. */
    private static long warm(Kernel kernel) {
        long started = System.nanoTime();
        do {
            kernel.run();
        } while (System.nanoTime() - started < WARMUP_NS);
        return System.nanoTime() - started;
    }

    private static void matmulOrdinary(double[] a, double[] b, double[] out, int n) {
        for (int row = 0; row < n; row++) {
            int rowOffset = row * n;
            for (int col = 0; col < n; col++) {
                double sum = 0.0;
                for (int k = 0; k < n; k++) {
                    sum = sum + a[rowOffset + k] * b[k * n + col];
                }
                out[rowOffset + col] = sum;
            }
        }
    }

    /** Cache-friendly row -> k -> col traversal; each dot product keeps ascending-k order. */
    private static void matmulTuned(double[] a, double[] b, double[] out, int n) {
        java.util.Arrays.fill(out, 0.0);
        for (int row = 0; row < n; row++) {
            int rowOffset = row * n;
            for (int k = 0; k < n; k++) {
                double aValue = a[rowOffset + k];
                int bRowOffset = k * n;
                for (int col = 0; col < n; col++) {
                    int index = rowOffset + col;
                    out[index] = out[index] + aValue * b[bRowOffset + col];
                }
            }
        }
    }

    private static void convolveOrdinary(double[] input, double[] out, int n) {
        for (int row = 0; row < n; row++) {
            int rowOffset = row * n;
            for (int col = 0; col < n; col++) {
                int outputIndex = rowOffset + col;
                if (row == 0 || col == 0 || row == n - 1 || col == n - 1) {
                    out[outputIndex] = 0.0;
                    continue;
                }
                int top = (row - 1) * n;
                int bottom = (row + 1) * n;
                double sum = 0.0;
                sum = sum + 1.0 * input[top + col - 1];
                sum = sum + 2.0 * input[top + col];
                sum = sum + input[top + col + 1];
                sum = sum + 2.0 * input[rowOffset + col - 1];
                sum = sum + 4.0 * input[rowOffset + col];
                sum = sum + 2.0 * input[rowOffset + col + 1];
                sum = sum + input[bottom + col - 1];
                sum = sum + 2.0 * input[bottom + col];
                sum = sum + input[bottom + col + 1];
                out[outputIndex] = sum / 16.0;
            }
        }
    }

    /** Zero-fill once and visit only the valid interior. */
    private static void convolveTuned(double[] input, double[] out, int n) {
        java.util.Arrays.fill(out, 0.0);
        for (int row = 1; row < n - 1; row++) {
            int top = (row - 1) * n;
            int middle = row * n;
            int bottom = (row + 1) * n;
            for (int col = 1; col < n - 1; col++) {
                double sum = 0.0;
                sum = sum + 1.0 * input[top + col - 1];
                sum = sum + 2.0 * input[top + col];
                sum = sum + input[top + col + 1];
                sum = sum + 2.0 * input[middle + col - 1];
                sum = sum + 4.0 * input[middle + col];
                sum = sum + 2.0 * input[middle + col + 1];
                sum = sum + input[bottom + col - 1];
                sum = sum + 2.0 * input[bottom + col];
                sum = sum + input[bottom + col + 1];
                out[middle + col] = sum / 16.0;
            }
        }
    }

    private static String hashOutput() throws Exception {
        ByteBuffer bytes = ByteBuffer.allocate(Math.multiplyExact(output.length, Double.BYTES))
                .order(ByteOrder.LITTLE_ENDIAN);
        for (double value : output) bytes.putDouble(value);
        byte[] digest = MessageDigest.getInstance("SHA-256").digest(bytes.array());
        return HexFormat.of().formatHex(digest);
    }

    private static String stringField(String json, String key) {
        Matcher matcher = STRING_FIELD.matcher(json);
        while (matcher.find()) if (key.equals(matcher.group(1))) return matcher.group(2);
        throw new IllegalArgumentException("missing string field: " + key);
    }

    private static int intField(String json, String key) {
        Matcher matcher = NUMBER_FIELD.matcher(json);
        while (matcher.find()) {
            if (key.equals(matcher.group(1))) return Integer.parseInt(matcher.group(2));
        }
        throw new IllegalArgumentException("missing integer field: " + key);
    }

    private static String quote(String value) {
        StringBuilder result = new StringBuilder(value.length() + 2).append('"');
        for (int index = 0; index < value.length(); index++) {
            char ch = value.charAt(index);
            switch (ch) {
                case '"' -> result.append("\\\"");
                case '\\' -> result.append("\\\\");
                case '\n' -> result.append("\\n");
                case '\r' -> result.append("\\r");
                case '\t' -> result.append("\\t");
                default -> {
                    if (ch < 0x20) result.append(String.format("\\u%04x", (int) ch));
                    else result.append(ch);
                }
            }
        }
        return result.append('"').toString();
    }
}
