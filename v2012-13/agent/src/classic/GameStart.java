package classic;

import java.io.IOException;
import java.lang.instrument.ClassFileTransformer;
import java.security.ProtectionDomain;

/**
 * Tells the local server when a game starts inside the launcher (versions up to 1.5.2, which the
 * launcher runs as an applet in its own window). From then on the game needs nothing from the
 * server, so it exits and only the game is left. Versions that run apart close the launcher
 * instead (bridge/MinecraftApplet.java).
 */
final class GameStart implements ClassFileTransformer {
    private static final String GAME_CLASS = "net/minecraft/client/Minecraft";

    private final String server;
    private boolean told;

    GameStart(String server) {
        this.server = server;
    }

    public synchronized byte[] transform(ClassLoader loader, String className, Class<?> redefined,
            ProtectionDomain domain, byte[] bytes) {
        if (!told && loader != null && GAME_CLASS.equals(className)) {
            told = true;
            Thread teller = new Thread("Classic game start") {
                public void run() {
                    try {
                        Http.post(server + "/classic/started", "");
                    } catch (IOException e) {
                        Agent.log("No se pudo avisar del inicio del juego: " + e.getMessage());
                    }
                }
            };
            teller.setDaemon(true);
            teller.start();
        }
        return null;
    }
}
