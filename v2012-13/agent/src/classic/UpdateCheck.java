package classic;

import java.awt.EventQueue;
import java.awt.Frame;
import java.io.IOException;
import javax.swing.JOptionPane;

/**
 * Offers a newer release of this launcher once its window is up ({@code <server>/classic/update}).
 * If the player says yes, the launcher closes and the local server installs the update and opens
 * it again; no keeps this version until the next start.
 */
final class UpdateCheck {
    private static final long FIND_MILLIS = 500;
    private static final long GIVE_UP_MILLIS = 120000;

    private UpdateCheck() {}

    static void start(final String server) {
        Thread checker = new Thread("Classic update") {
            public void run() {
                try {
                    check(server);
                } catch (Exception e) {
                    Agent.log("Actualizaciones: " + e);
                }
            }
        };
        checker.setDaemon(true);
        checker.start();
    }

    private static void check(final String server) throws Exception {
        final Frame[] window = new Frame[1];
        long deadline = System.currentTimeMillis() + GIVE_UP_MILLIS;
        while (window[0] == null && System.currentTimeMillis() < deadline) {
            Thread.sleep(FIND_MILLIS);
            EventQueue.invokeAndWait(new Runnable() {
                public void run() {
                    for (Frame frame : Frame.getFrames()) {
                        if (frame.isShowing()) {
                            window[0] = frame;
                        }
                    }
                }
            });
        }
        // The server answers once it has heard from GitHub.
        String offer = window[0] == null ? "" : Http.get(server + "/classic/update").trim();
        final String[] versions = offer.split("\t");
        if (versions.length != 2) {
            return;
        }
        Agent.log("Actualizaciones: " + versions[1] + " disponible (esta es " + versions[0] + ").");
        EventQueue.invokeLater(new Runnable() {
            public void run() {
                int answer = JOptionPane.showConfirmDialog(window[0],
                        "A new version of Minecraft Launcher Classic is available: " + versions[1]
                                + "\nYou have " + versions[0] + ".\n\nWould you like to update now?",
                        "Update available", JOptionPane.YES_NO_OPTION, JOptionPane.QUESTION_MESSAGE);
                if (answer != JOptionPane.YES_OPTION) {
                    return;
                }
                try {
                    Http.post(server + "/classic/update", "");
                    System.exit(0); // the server updates and reopens the launcher
                } catch (IOException e) {
                    JOptionPane.showMessageDialog(window[0], e.getMessage(), "Update", JOptionPane.ERROR_MESSAGE);
                }
            }
        });
    }
}
