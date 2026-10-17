package classic;

import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.net.HttpURLConnection;
import java.net.URL;

/** Requests to the local server. Its error replies become the exception's message. */
final class Http {
    private Http() {}

    static String get(String url) throws IOException {
        return request(url, null);
    }

    static String post(String url, String body) throws IOException {
        return request(url, body);
    }

    private static String request(String url, String body) throws IOException {
        HttpURLConnection connection = (HttpURLConnection) new URL(url).openConnection();
        connection.setConnectTimeout(5000);
        // The version list waits for Mojang's and Fabric's servers the first time.
        connection.setReadTimeout(30000);
        connection.setUseCaches(false);
        if (body != null) {
            connection.setDoOutput(true);
            connection.setRequestProperty("Content-Type", "text/plain; charset=utf-8");
            OutputStream out = connection.getOutputStream();
            try {
                out.write(body.getBytes("UTF-8"));
            } finally {
                out.close();
            }
        }
        int code = connection.getResponseCode();
        InputStream in = code < 400 ? connection.getInputStream() : connection.getErrorStream();
        String reply = in == null ? "" : read(in);
        if (code >= 400) {
            throw new IOException(reply.length() > 0 ? reply : "HTTP " + code + " de " + url);
        }
        return reply;
    }

    private static String read(InputStream in) throws IOException {
        try {
            ByteArrayOutputStream bytes = new ByteArrayOutputStream();
            byte[] chunk = new byte[4096];
            for (int n; (n = in.read(chunk)) > 0;) {
                bytes.write(chunk, 0, n);
            }
            return bytes.toString("UTF-8");
        } finally {
            in.close();
        }
    }
}
