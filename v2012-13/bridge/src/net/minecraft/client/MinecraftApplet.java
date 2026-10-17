package net.minecraft.client;

import java.applet.Applet;
import java.awt.Color;
import java.awt.Font;
import java.awt.FontMetrics;
import java.awt.Graphics;
import java.awt.Image;
import java.awt.image.VolatileImage;
import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.net.HttpURLConnection;
import java.net.URL;
import java.net.URLEncoder;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import javax.imageio.ImageIO;

/**
 * Stands in for the game applet of the versions the 2012 launcher cannot run itself (1.6 and
 * later, and the oldest ones). The launcher downloads and starts it like the real game. It asks
 * the local server to install and open the real version as a separate process, and meanwhile
 * draws the server's progress as the launcher's own "Updating Minecraft" screen, the same way
 * net.minecraft.Launcher paints it. Once the game window is up, it closes the launcher.
 */
@SuppressWarnings("serial")
public class MinecraftApplet extends Applet implements Runnable {
    private static final int POLL_MILLIS = 250;

    private Image background;
    private VolatileImage buffer;
    private volatile String description = "Contacting the local server";
    private volatile String subtask = "";
    private volatile int percent;
    private volatile boolean failed;
    private Thread worker;

    public void init() {
        try {
            background = ImageIO.read(MinecraftApplet.class.getResource("/net/minecraft/dirt.png"))
                    .getScaledInstance(32, 32, Image.SCALE_AREA_AVERAGING);
        } catch (Exception e) {
            background = null; // painted plain grey instead
        }
    }

    public synchronized void start() {
        if (worker == null) {
            worker = new Thread(this, "Classic bridge");
            worker.setDaemon(true);
            worker.start();
        }
    }

    public void run() {
        try {
            String server = System.getProperty("classic.server", "https://127.0.0.1:25580");
            request(server + "/classic/play", "user=" + encode(getParameter("username"))
                    + "&session=" + encode(getParameter("sessionid")));
            while (true) {
                Map<String, String> status = parse(request(server + "/classic/status", null));
                description = value(status, "description");
                subtask = value(status, "subtask");
                try {
                    percent = Integer.parseInt(value(status, "percent"));
                } catch (NumberFormatException e) {
                    // keep the last one
                }
                String state = status.get("state");
                if ("running".equals(state)) {
                    System.exit(0); // the game window is up: close the launcher
                }
                if ("failed".equals(state)) {
                    fail(description);
                    return;
                }
                repaint();
                Thread.sleep(POLL_MILLIS);
            }
        } catch (Exception e) {
            fail(e.getMessage() != null ? e.getMessage() : e.toString());
        }
    }

    private void fail(String message) {
        description = message;
        subtask = "";
        failed = true;
        repaint();
    }

    public void update(Graphics g) {
        paint(g);
    }

    /** The update screen of net.minecraft.Launcher: drawn at half size, then scaled up twice. */
    public void paint(Graphics screen) {
        int w = getWidth() / 2;
        int h = getHeight() / 2;
        if (w <= 0 || h <= 0) {
            return;
        }
        if (buffer == null || buffer.getWidth() != w || buffer.getHeight() != h) {
            buffer = createVolatileImage(w, h);
            if (buffer == null) {
                return; // not on screen yet
            }
        }
        Graphics g = buffer.getGraphics();
        if (background != null) {
            for (int x = 0; x <= w / 32; x++) {
                for (int y = 0; y <= h / 32; y++) {
                    g.drawImage(background, x * 32, y * 32, this);
                }
            }
        } else {
            g.setColor(Color.DARK_GRAY);
            g.fillRect(0, 0, w, h);
        }
        g.setColor(Color.LIGHT_GRAY);
        g.setFont(new Font(null, Font.BOLD, 20));
        center(g, failed ? "Failed to launch" : "Updating Minecraft", w, h / 2 - g.getFontMetrics().getHeight() * 2);
        g.setFont(new Font(null, Font.PLAIN, 12));
        int line = g.getFontMetrics().getHeight();
        if (failed) {
            List<String> lines = wrap(g.getFontMetrics(), description, w - 32);
            for (int i = 0; i < lines.size(); i++) {
                center(g, lines.get(i), w, h / 2 + line * (i + 1));
            }
        } else {
            center(g, description, w, h / 2 + line);
            center(g, subtask, w, h / 2 + line * 2);
            g.setColor(Color.black);
            g.fillRect(64, h - 64, w - 128 + 1, 5);
            g.setColor(new Color(32768));
            g.fillRect(64, h - 64, percent * (w - 128) / 100, 4);
            g.setColor(new Color(2138144));
            g.fillRect(65, h - 64 + 1, percent * (w - 128) / 100 - 2, 1);
        }
        g.dispose();
        screen.drawImage(buffer, 0, 0, w * 2, h * 2, null);
    }

    private static void center(Graphics g, String text, int width, int y) {
        g.drawString(text, width / 2 - g.getFontMetrics().stringWidth(text) / 2, y);
    }

    private static List<String> wrap(FontMetrics metrics, String text, int width) {
        List<String> lines = new ArrayList<String>();
        StringBuilder line = new StringBuilder();
        for (String word : text.split(" ")) {
            if (line.length() > 0 && metrics.stringWidth(line + " " + word) > width) {
                lines.add(line.toString());
                line.setLength(0);
            }
            if (line.length() > 0) {
                line.append(' ');
            }
            line.append(word);
        }
        lines.add(line.toString());
        return lines;
    }

    /** POSTs {@code form} when it is not null, otherwise GETs. Error replies become the exception message. */
    private static String request(String url, String form) throws IOException {
        HttpURLConnection connection = (HttpURLConnection) new URL(url).openConnection();
        connection.setConnectTimeout(5000);
        connection.setReadTimeout(15000);
        connection.setUseCaches(false);
        if (form != null) {
            connection.setDoOutput(true);
            connection.setRequestProperty("Content-Type", "application/x-www-form-urlencoded");
            OutputStream out = connection.getOutputStream();
            try {
                out.write(form.getBytes("UTF-8"));
            } finally {
                out.close();
            }
        }
        int code = connection.getResponseCode();
        InputStream in = code < 400 ? connection.getInputStream() : connection.getErrorStream();
        String body = in == null ? "" : read(in);
        if (code >= 400) {
            throw new IOException(body.length() > 0 ? body : "HTTP " + code + " de " + url);
        }
        return body;
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

    private static Map<String, String> parse(String lines) {
        Map<String, String> values = new HashMap<String, String>();
        for (String line : lines.split("\n")) {
            int equals = line.indexOf('=');
            if (equals > 0) {
                values.put(line.substring(0, equals), line.substring(equals + 1));
            }
        }
        return values;
    }

    private static String value(Map<String, String> values, String key) {
        String value = values.get(key);
        return value != null ? value : "";
    }

    private static String encode(String value) throws IOException {
        return URLEncoder.encode(value != null ? value : "", "UTF-8");
    }
}
