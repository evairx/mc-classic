package classic;

import java.awt.Container;
import java.awt.Dimension;
import java.awt.EventQueue;
import java.awt.Rectangle;
import java.beans.PropertyChangeEvent;
import java.beans.PropertyChangeListener;
import java.io.IOException;
import javax.swing.JEditorPane;
import javax.swing.JScrollPane;
import javax.swing.ScrollPaneConstants;
import javax.swing.ScrollPaneLayout;
import javax.swing.event.HyperlinkListener;
import javax.swing.text.Element;
import javax.swing.text.html.HTML;
import javax.swing.text.html.HTMLDocument;

/**
 * Keeps the news sidebar in place while the posts scroll. Swing's HTML has no position: fixed,
 * so the sidebar ({@code <server>/mcupdate.tumblr.com/sidebar}) goes into a pane of its own,
 * which this layout puts between the posts and the scroll bar, and the page drops its own copy.
 * If that pane cannot load, the page keeps its sidebar. When the window is too short for it, the
 * sidebar scrolls with the mouse wheel.
 */
@SuppressWarnings("serial")
final class NewsSidebar extends ScrollPaneLayout {
    /** The sidebar cell's 180px, its 10px padding on each side and the page's margins. */
    private static final int WIDTH = 216;

    private final JScrollPane sidebar;

    private NewsSidebar(JScrollPane sidebar) {
        this.sidebar = sidebar;
    }

    /** Loads the sidebar in the background, then fixes it beside the news. */
    static void show(final JScrollPane news, final JEditorPane page, final HTMLDocument document, final String url) {
        final JEditorPane pane = new JEditorPane();
        pane.setEditable(false);
        // It looks and its links open like the page's own.
        pane.setBackground(page.getBackground());
        pane.setMargin(page.getMargin());
        pane.setBorder(page.getBorder());
        for (HyperlinkListener listener : page.getHyperlinkListeners()) {
            pane.addHyperlinkListener(listener);
        }
        pane.addPropertyChangeListener("page", new PropertyChangeListener() {
            public void propertyChange(PropertyChangeEvent event) {
                EventQueue.invokeLater(new Runnable() {
                    public void run() {
                        install(news, document, pane);
                    }
                });
            }
        });
        Thread loader = new Thread("Classic news sidebar") {
            public void run() {
                try {
                    pane.setPage(url);
                } catch (IOException e) {
                    Agent.log("Noticias: la barra lateral se queda en la pagina: " + e);
                }
            }
        };
        loader.setDaemon(true);
        loader.start();
    }

    /** Called once the sidebar has loaded. */
    private static void install(JScrollPane news, HTMLDocument document, JEditorPane pane) {
        Element cell = document.getElement(document.getDefaultRootElement(), HTML.Attribute.CLASS, "sidebar");
        if (news.getLayout() instanceof NewsSidebar || cell == null) {
            return;
        }
        JScrollPane sidebar = new JScrollPane(pane, ScrollPaneConstants.VERTICAL_SCROLLBAR_AS_NEEDED,
                ScrollPaneConstants.HORIZONTAL_SCROLLBAR_NEVER);
        sidebar.setBorder(null);
        sidebar.getViewport().setBackground(pane.getBackground());
        // When the window is too short for it, the sidebar scrolls with the mouse wheel. Its own
        // scroll bar is there, which the wheel needs, but takes no room next to the news one.
        sidebar.getVerticalScrollBar().setPreferredSize(new Dimension(0, 0));
        news.add(sidebar);
        news.setLayout(new NewsSidebar(sidebar)); // JScrollPane hands it the viewport and scroll bars
        document.removeElement(cell);
        news.revalidate();
        news.repaint();
        Agent.log("Noticias: barra lateral fija.");
    }

    /** The usual layout, then the sidebar takes the right side of the viewport. */
    public void layoutContainer(Container parent) {
        super.layoutContainer(parent);
        Rectangle view = viewport.getBounds();
        int width = Math.min(WIDTH, view.width / 2);
        viewport.setBounds(view.x, view.y, view.width - width, view.height);
        sidebar.setBounds(view.x + view.width - width, view.y, width, view.height);
    }
}
