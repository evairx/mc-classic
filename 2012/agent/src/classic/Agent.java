package classic;

import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.PrintStream;
import java.lang.instrument.Instrumentation;
import java.security.KeyStore;
import java.security.cert.CertificateException;
import java.security.cert.CertificateFactory;
import java.security.cert.X509Certificate;
import java.text.SimpleDateFormat;
import java.util.Arrays;
import java.util.Date;
import java.util.jar.JarEntry;
import java.util.jar.JarFile;
import java.util.jar.JarOutputStream;
import javax.net.ssl.HttpsURLConnection;
import javax.net.ssl.SSLContext;
import javax.net.ssl.TrustManager;
import javax.net.ssl.TrustManagerFactory;
import javax.net.ssl.X509TrustManager;

/**
 * Java agent that Minecraft Launcher Classic loads into the 2012 launcher. It points the
 * launcher (and the game it runs as an applet) at the local server without changing a byte of
 * Minecraft.exe or its signed JAR.
 */
public final class Agent {
    /** Hosts the 2012 launcher and game talk to. All of them are dead today. */
    static final String[] HOSTS = {
        "login.minecraft.net", "s3.amazonaws.com", "mcupdate.tumblr.com", "session.minecraft.net"
    };
    /** Classpath resource that replaces the launcher's pinned minecraft.key. */
    static final String PIN_RESOURCE = "classic-pin.key";
    /** net.minecraft.Util.excutePost reads exactly this many bytes of the pinned key. */
    private static final int PIN_LENGTH = 294;

    private static PrintStream log = System.err;

    private Agent() {}

    public static void premain(String args, Instrumentation inst) {
        try {
            File dir = dataDir(args);
            // javaw has no console: keep the launcher's own output (stack traces included) too.
            log = new PrintStream(new FileOutputStream(new File(dir, "launcher.log"), true), true, "UTF-8");
            System.setOut(log);
            System.setErr(log);

            String server = System.getProperty("classic.server", "https://127.0.0.1:25580");
            System.setProperty("classic.server", server); // the bridge applet calls it too
            log("Agente iniciado. Servidor: " + server + ", datos: " + dir);
            isolateGameDirectory(dir);

            X509Certificate cert = readCertificate(new File(dir, "server-cert.pem"));
            HttpsURLConnection.setDefaultSSLSocketFactory(trusting(cert).getSocketFactory());
            inst.appendToSystemClassLoaderSearch(new JarFile(writePinJar(cert)));
            inst.addTransformer(new UrlRewriter(server));
            inst.addTransformer(new LauncherFixes());
            inst.addTransformer(new GameStart(server));
            NewsScroller.start(server);
            LauncherOptions.start(server);
            UpdateCheck.start(server);
        } catch (Exception e) {
            log("ERROR: el agente no pudo iniciar; el launcher intentara usar los servidores originales.");
            e.printStackTrace(log);
        }
    }

    static synchronized void log(String message) {
        log.println(new SimpleDateFormat("HH:mm:ss").format(new Date()) + " [classic] " + message);
    }

    /**
     * The launcher and the game both use %APPDATA%\.minecraft, which today holds a modern
     * install whose worlds a 2012 client could damage. The rewriter hides APPDATA, so both fall
     * back to user.home\.minecraft, and user.home points at a private folder: the launcher's own
     * (classic.home, which the exe passes), or data\ next to the agent's folder. Once the
     * launcher has settled on it, user.home moves to the selected profile's folder, where a game
     * that runs inside the launcher keeps its worlds (LauncherOptions).
     */
    private static void isolateGameDirectory(File dir) {
        File home = new File(System.getProperty("classic.home", new File(dir.getParentFile(), "data").getPath()))
                .getAbsoluteFile();
        home.mkdirs();
        System.setProperty("user.home", home.getPath());
        log("Carpeta del juego: " + new File(home, ".minecraft"));
    }

    /** The agent argument wins; otherwise the folder that holds classic-agent.jar. */
    private static File dataDir(String args) throws Exception {
        if (args != null && args.length() > 0) {
            return new File(args).getAbsoluteFile();
        }
        return new File(Agent.class.getProtectionDomain().getCodeSource().getLocation().toURI())
                .getParentFile();
    }

    private static X509Certificate readCertificate(File file) throws IOException, CertificateException {
        InputStream in = new FileInputStream(file);
        try {
            return (X509Certificate) CertificateFactory.getInstance("X.509").generateCertificate(in);
        } finally {
            in.close();
        }
    }

    /** TLS context that trusts the local server's self-signed certificate plus the normal CAs. */
    private static SSLContext trusting(final X509Certificate server) throws Exception {
        TrustManagerFactory factory = TrustManagerFactory.getInstance(TrustManagerFactory.getDefaultAlgorithm());
        factory.init((KeyStore) null);
        X509TrustManager found = null;
        for (TrustManager manager : factory.getTrustManagers()) {
            if (manager instanceof X509TrustManager) {
                found = (X509TrustManager) manager;
                break;
            }
        }
        final X509TrustManager system = found;
        X509TrustManager combined = new X509TrustManager() {
            public void checkClientTrusted(X509Certificate[] chain, String authType) throws CertificateException {
                system.checkClientTrusted(chain, authType);
            }

            public void checkServerTrusted(X509Certificate[] chain, String authType) throws CertificateException {
                if (chain != null && chain.length > 0 && chain[0].equals(server)) {
                    return;
                }
                system.checkServerTrusted(chain, authType);
            }

            public X509Certificate[] getAcceptedIssuers() {
                return system.getAcceptedIssuers();
            }
        };
        SSLContext context = SSLContext.getInstance("TLS");
        context.init(null, new TrustManager[] { combined }, null);
        return context;
    }

    /**
     * The launcher only sends the password to a server whose public key matches minecraft.key.
     * The rewriter points that lookup at this JAR, which holds the local server's key instead.
     */
    private static File writePinJar(X509Certificate cert) throws IOException {
        byte[] key = cert.getPublicKey().getEncoded();
        if (key.length > PIN_LENGTH) {
            throw new IOException("La clave del servidor (" + key.length + " bytes) no cabe en los "
                    + PIN_LENGTH + " bytes que compara el launcher; usa una clave EC P-256 o RSA-2048.");
        }
        File jar = File.createTempFile("classic-pin", ".jar");
        jar.deleteOnExit();
        JarOutputStream out = new JarOutputStream(new FileOutputStream(jar));
        try {
            out.putNextEntry(new JarEntry(PIN_RESOURCE));
            // Only the first key.length bytes are compared; the zero padding satisfies readFully(294).
            out.write(Arrays.copyOf(key, PIN_LENGTH));
            out.closeEntry();
        } finally {
            out.close();
        }
        return jar;
    }
}
