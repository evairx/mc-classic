package classic;

import java.awt.Component;
import java.awt.Container;
import java.awt.EventQueue;
import java.awt.Frame;
import java.io.IOException;
import java.net.URL;
import javax.swing.BoundedRangeModel;
import javax.swing.JEditorPane;
import javax.swing.JScrollPane;
import javax.swing.SwingUtilities;
import javax.swing.event.ChangeEvent;
import javax.swing.event.ChangeListener;
import javax.swing.text.Element;
import javax.swing.text.html.HTMLDocument;

/**
 * Infinite scroll for the launcher's news pane. The pane is a JEditorPane, which renders simple
 * HTML and runs no JavaScript, so this does what a script would: it watches the pane's scroll
 * bar and, near the bottom, appends the next page of posts from the local server
 * ({@code <server>/mcupdate.tumblr.com/page/<n>}) to the element with id {@code posts}, until a
 * page comes back empty.
 */
final class NewsScroller implements ChangeListener {
    /** URL path of the news page; the launcher asks for http://mcupdate.tumblr.com/. */
    private static final String NEWS = "/mcupdate.tumblr.com/";
    /** Distance from the bottom, in pixels, at which the next page is requested. */
    private static final int MARGIN = 300;
    private static final long FIND_MILLIS = 500;
    /** The launcher shows its news as it opens; after this long they are not coming. */
    private static final long GIVE_UP_MILLIS = 120000;

    private final String server;
    private final JEditorPane pane;
    private final HTMLDocument document;
    private final BoundedRangeModel scroll;
    // Only touched on the event dispatch thread.
    private int next = 2;
    private boolean loading;
    private boolean finished;

    private NewsScroller(String server, JEditorPane pane, HTMLDocument document, BoundedRangeModel scroll) {
        this.server = server;
        this.pane = pane;
        this.document = document;
        this.scroll = scroll;
    }

    /** Waits on a background thread for the news to show, then takes over their scroll bar. */
    static void start(final String server) {
        Thread finder = new Thread("Classic news") {
            public void run() {
                find(server);
            }
        };
        finder.setDaemon(true);
        finder.start();
    }

    private static void find(final String server) {
        final boolean[] attached = new boolean[1];
        long deadline = System.currentTimeMillis() + GIVE_UP_MILLIS;
        try {
            while (!attached[0] && System.currentTimeMillis() < deadline) {
                Thread.sleep(FIND_MILLIS);
                EventQueue.invokeAndWait(new Runnable() {
                    public void run() {
                        attached[0] = attach(server);
                    }
                });
            }
        } catch (Exception e) {
            Agent.log("Noticias: " + e);
        }
        if (!attached[0]) {
            Agent.log("Noticias: no aparecio el panel de noticias; sin scroll infinito.");
        }
    }

    /** Whether the news pane was found and fully loaded; then it scrolls infinitely. */
    private static boolean attach(String server) {
        for (Frame frame : Frame.getFrames()) {
            JEditorPane pane = newsPane(frame);
            if (pane == null) {
                continue;
            }
            HTMLDocument document = (HTMLDocument) pane.getDocument();
            Element posts = document.getElement("posts");
            // The page loads in the background; once something follows the posts (the sidebar),
            // they are all in.
            if (posts == null || posts.getEndOffset() >= document.getLength()) {
                return false;
            }
            JScrollPane scrollPane = (JScrollPane) SwingUtilities.getAncestorOfClass(JScrollPane.class, pane);
            if (scrollPane == null) {
                Agent.log("Noticias: el panel no tiene barra de desplazamiento; sin scroll infinito.");
                return true;
            }
            NewsScroller scroller = new NewsScroller(server, pane, document, scrollPane.getVerticalScrollBar().getModel());
            scroller.scroll.addChangeListener(scroller);
            Agent.log("Noticias: scroll infinito activo.");
            scroller.stateChanged(null);
            return true;
        }
        return false;
    }

    private static JEditorPane newsPane(Component component) {
        if (component instanceof JEditorPane) {
            JEditorPane pane = (JEditorPane) component;
            URL page = pane.getPage();
            if (page != null && NEWS.equals(page.getPath()) && pane.getDocument() instanceof HTMLDocument) {
                return pane;
            }
        }
        if (component instanceof Container) {
            for (Component child : ((Container) component).getComponents()) {
                JEditorPane pane = newsPane(child);
                if (pane != null) {
                    return pane;
                }
            }
        }
        return null;
    }

    /** The scroll bar moved or the page grew: near the bottom, fetch the next page. */
    public void stateChanged(ChangeEvent event) {
        if (loading || finished) {
            return;
        }
        if (pane.getDocument() != document) {
            finished = true; // the launcher loaded another page
            return;
        }
        if (scroll.getValue() + scroll.getExtent() < scroll.getMaximum() - MARGIN) {
            return;
        }
        loading = true;
        final int page = next;
        Thread fetcher = new Thread("Classic news page " + page) {
            public void run() {
                String html;
                try {
                    html = Http.get(server + NEWS + "page/" + page);
                } catch (IOException e) {
                    Agent.log("Noticias: no se pudo cargar la pagina " + page + ": " + e);
                    html = null;
                }
                final String result = html;
                EventQueue.invokeLater(new Runnable() {
                    public void run() {
                        append(page, result);
                    }
                });
            }
        };
        fetcher.setDaemon(true);
        fetcher.start();
    }

    /**
     * Adds a fetched page at the end of the posts. The scroll bar's maximum changes once the pane
     * lays it out, which checks again, so pages keep coming while the pane is not full.
     */
    private void append(int page, String html) {
        loading = false;
        if (html == null || html.trim().length() == 0 || pane.getDocument() != document) {
            finished = true;
            return;
        }
        try {
            document.insertBeforeEnd(document.getElement("posts"), html);
            next = page + 1;
            Agent.log("Noticias: pagina " + page + " anadida.");
        } catch (Exception e) {
            Agent.log("Noticias: no se pudo anadir la pagina " + page + ": " + e);
            finished = true;
        }
    }
}
