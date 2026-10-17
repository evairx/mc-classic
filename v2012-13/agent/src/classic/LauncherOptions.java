package classic;

import java.awt.AWTEvent;
import java.awt.BorderLayout;
import java.awt.Component;
import java.awt.Container;
import java.awt.Desktop;
import java.awt.EventQueue;
import java.awt.FlowLayout;
import java.awt.Frame;
import java.awt.GridBagConstraints;
import java.awt.GridBagLayout;
import java.awt.Insets;
import java.awt.Toolkit;
import java.awt.event.AWTEventListener;
import java.awt.event.ActionEvent;
import java.awt.event.ActionListener;
import java.awt.event.WindowEvent;
import java.io.File;
import java.io.IOException;
import java.util.ArrayList;
import java.util.List;
import javax.swing.BorderFactory;
import javax.swing.Box;
import javax.swing.BoxLayout;
import javax.swing.JButton;
import javax.swing.JCheckBox;
import javax.swing.JComboBox;
import javax.swing.JDialog;
import javax.swing.JLabel;
import javax.swing.JOptionPane;
import javax.swing.JPanel;
import javax.swing.JTextField;
import javax.swing.border.EmptyBorder;
import javax.swing.event.DocumentEvent;
import javax.swing.event.DocumentListener;

/**
 * The launcher's options window, as a profile editor in the style of Mojang's 2013 launcher:
 * profile info, version selection and Java settings, each in its own titled box, with
 * checkboxes that turn on the optional settings. Each time net.minecraft.OptionsPanel opens, its
 * content is replaced by this one; "Force game update" goes, since the version chosen here is
 * what gets installed. Profiles live in the local server ({@code <server>/classic/profiles});
 * the selected one fills the login form's username and points user.home at its folder, where a
 * game that runs inside the launcher keeps its worlds.
 */
final class LauncherOptions implements AWTEventListener {
    private static final String OPTIONS_PANEL = "net.minecraft.OptionsPanel";
    private static final String LOGIN_FORM = "net.minecraft.LoginForm";
    /** What the window shows as the game directory; "Open Game Dir" opens the real folder. */
    private static final String LOCATION = "%APPDATA%/Roaming/mc-classic/2012-13/instances";
    private static final String AUTOMATIC_JAVA = "Automatic: the Java Mojang publishes for the version";
    private static final long FIND_MILLIS = 500;
    private static final long GIVE_UP_MILLIS = 120000;

    private final String server;

    private LauncherOptions(String server) {
        this.server = server;
    }

    /** Waits on a background thread for the login form, then applies the selected profile. */
    static void start(final String server) {
        Thread finder = new Thread("Classic options") {
            public void run() {
                attach(server);
            }
        };
        finder.setDaemon(true);
        finder.start();
    }

    private static void attach(final String server) {
        final Component[] form = new Component[1];
        long deadline = System.currentTimeMillis() + GIVE_UP_MILLIS;
        try {
            while (form[0] == null && System.currentTimeMillis() < deadline) {
                Thread.sleep(FIND_MILLIS);
                EventQueue.invokeAndWait(new Runnable() {
                    public void run() {
                        for (Frame frame : Frame.getFrames()) {
                            if (form[0] == null) {
                                form[0] = find(frame, LOGIN_FORM);
                            }
                        }
                    }
                });
            }
            if (form[0] == null) {
                Agent.log("Opciones: no aparecio el formulario de login; sin perfiles.");
                return;
            }
            // The launcher has read lastlogin by now, so its own folder is settled.
            final Profiles profiles = Profiles.fetch(server);
            EventQueue.invokeAndWait(new Runnable() {
                public void run() {
                    Toolkit.getDefaultToolkit().addAWTEventListener(new LauncherOptions(server), AWTEvent.WINDOW_EVENT_MASK);
                    apply(profiles.selected());
                }
            });
            Agent.log("Opciones: perfiles activos.");
        } catch (Exception e) {
            Agent.log("Opciones: " + e);
        }
    }

    private static Component find(Component component, String className) {
        if (component.getClass().getName().equals(className)) {
            return component;
        }
        if (component instanceof Container) {
            for (Component child : ((Container) component).getComponents()) {
                Component found = find(child, className);
                if (found != null) {
                    return found;
                }
            }
        }
        return null;
    }

    /** The profile's username goes into the login form, and its folder becomes user.home. */
    private static void apply(Profile profile) {
        System.setProperty("user.home", profile.folder);
        if (profile.username.length() == 0) {
            return;
        }
        for (Frame frame : Frame.getFrames()) {
            Component form = find(frame, LOGIN_FORM);
            if (form == null) {
                continue;
            }
            try {
                ((JTextField) form.getClass().getField("userName").get(form)).setText(profile.username);
            } catch (Exception e) {
                Agent.log("Opciones: no se pudo poner el usuario: " + e);
            }
        }
    }

    public void eventDispatched(AWTEvent event) {
        if (event.getID() == WindowEvent.WINDOW_OPENED && event.getSource().getClass().getName().equals(OPTIONS_PANEL)) {
            try {
                new Editor((JDialog) event.getSource(), Profiles.fetch(server)).show();
            } catch (Exception e) {
                Agent.log("Opciones: se deja la ventana original: " + e);
            }
        }
    }

    /** The window's content, editing a copy of the profiles until "Save Profile" saves them. */
    private final class Editor {
        private final JDialog dialog;
        private final Profiles profiles;
        private final JComboBox<Profile> profileBox = new JComboBox<Profile>();
        private final JTextField nameField = new JTextField(30);
        private final JTextField userField = new JTextField(30);
        private final JCheckBox resolutionBox = new JCheckBox("Resolution:");
        private final JTextField widthField = new JTextField("854", 6);
        private final JTextField heightField = new JTextField("480", 6);
        private final JCheckBox snapshots = new JCheckBox("Enable experimental development versions (\"snapshots\")");
        private final JCheckBox betas = new JCheckBox("Allow use of old \"Beta\" Minecraft versions (From 2010-2011)");
        private final JCheckBox alphas = new JCheckBox("Allow use of old \"Alpha\" Minecraft versions (From 2010)");
        private final JCheckBox modded = new JCheckBox("Allow use of modded versions (\"Fabric\" mod loader)");
        private final JComboBox<Version> versionBox = new JComboBox<Version>();
        private final Toggle java = new Toggle("Executable:", AUTOMATIC_JAVA, true);
        private final Toggle gameFlags;
        private final Toggle launcherFlags;
        /** Every version the server lists; the box shows those the checkboxes allow. */
        private final List<Version> versions = new ArrayList<Version>();
        private Profile current;

        Editor(JDialog dialog, Profiles profiles) {
            this.dialog = dialog;
            this.profiles = profiles;
            gameFlags = new Toggle("JVM Arguments:", profiles.defaultGameFlags, false);
            launcherFlags = new Toggle("Launcher JVM Arguments:", profiles.defaultLauncherFlags, false);
        }

        void show() {
            JPanel info = section("Profile Info");
            row(info, new JLabel("Profile:"), profileRow());
            row(info, new JLabel("Profile Name:"), nameField);
            row(info, new JLabel("Username:"), userField);
            JTextField location = new JTextField(LOCATION);
            location.setEnabled(false);
            row(info, new JLabel("Game Directory:"), location);
            row(info, resolutionBox, resolutionRow());

            JPanel versionPanel = section("Version Selection");
            for (JCheckBox filter : new JCheckBox[] { snapshots, betas, alphas, modded }) {
                wide(versionPanel, filter);
                filter.addActionListener(new ActionListener() {
                    public void actionPerformed(ActionEvent event) {
                        refilter();
                    }
                });
            }
            row(versionPanel, new JLabel("Use version:"), versionBox);

            JPanel javaPanel = section("Java Settings (Advanced)");
            java.addTo(javaPanel);
            gameFlags.addTo(javaPanel);
            launcherFlags.addTo(javaPanel);

            userField.setToolTipText("Fills the login form. Empty keeps the last one used.");
            modded.setToolTipText("Fabric versions come with the Fabric mod loader: put mods in the profile's .minecraft\\mods.");
            gameFlags.field.setToolTipText("Such as -Xmx4G for 4 GB of memory. Versions up to 1.5.2 run inside the launcher and use its arguments.");
            launcherFlags.field.setToolTipText("They apply the next time the launcher opens.");
            resolutionBox.addActionListener(new ActionListener() {
                public void actionPerformed(ActionEvent event) {
                    widthField.setEnabled(resolutionBox.isSelected());
                    heightField.setEnabled(resolutionBox.isSelected());
                }
            });

            JPanel sections = new JPanel();
            sections.setLayout(new BoxLayout(sections, BoxLayout.Y_AXIS));
            sections.add(info);
            sections.add(versionPanel);
            sections.add(javaPanel);

            JPanel panel = new JPanel(new BorderLayout());
            panel.add(sections, BorderLayout.CENTER);
            panel.add(buttons(), BorderLayout.SOUTH);
            panel.setBorder(new EmptyBorder(8, 8, 8, 8));

            for (Profile profile : profiles.list) {
                profileBox.addItem(profile);
            }
            current = profiles.selected();
            profileBox.setSelectedItem(current);
            launcherFlags.set(profiles.launcherFlags);
            versions.add(new Version(current.version, current.version, "custom"));
            display(current);
            profileBox.addActionListener(new ActionListener() {
                public void actionPerformed(ActionEvent event) {
                    if (profileBox.getSelectedItem() != null) {
                        commit();
                        current = (Profile) profileBox.getSelectedItem();
                        display(current);
                    }
                }
            });
            // The list shows the new name as it is typed.
            nameField.getDocument().addDocumentListener(new DocumentListener() {
                public void insertUpdate(DocumentEvent event) {
                    rename();
                }

                public void removeUpdate(DocumentEvent event) {
                    rename();
                }

                public void changedUpdate(DocumentEvent event) {
                    rename();
                }
            });
            loadVersions();

            dialog.setTitle("Profile Editor");
            dialog.getContentPane().removeAll();
            dialog.getContentPane().add(panel);
            dialog.pack();
            dialog.setLocationRelativeTo(dialog.getOwner());
        }

        private JPanel section(String title) {
            JPanel section = new JPanel(new GridBagLayout());
            section.setBorder(BorderFactory.createCompoundBorder(BorderFactory.createTitledBorder(title),
                    new EmptyBorder(0, 4, 4, 4)));
            return section;
        }

        /** A label or checkbox on the left and its field filling the rest of the line. */
        private void row(JPanel panel, Component left, Component right) {
            GridBagConstraints c = new GridBagConstraints();
            c.gridx = 0;
            c.anchor = GridBagConstraints.WEST;
            c.insets = new Insets(2, 2, 2, 8);
            panel.add(left, c);
            c.gridx = 1;
            c.weightx = 1;
            c.fill = GridBagConstraints.HORIZONTAL;
            c.insets = new Insets(2, 0, 2, 2);
            panel.add(right, c);
        }

        private void wide(JPanel panel, Component component) {
            GridBagConstraints c = new GridBagConstraints();
            c.gridx = 0;
            c.gridwidth = 2;
            c.anchor = GridBagConstraints.WEST;
            c.insets = new Insets(1, 2, 1, 2);
            panel.add(component, c);
        }

        private JPanel profileRow() {
            JButton add = new JButton("New");
            add.addActionListener(new ActionListener() {
                public void actionPerformed(ActionEvent event) {
                    commit();
                    Profile profile = current.copy(unusedName());
                    profiles.list.add(profile);
                    profileBox.addItem(profile);
                    profileBox.setSelectedItem(profile);
                    nameField.requestFocusInWindow();
                    nameField.selectAll();
                }
            });
            JButton delete = new JButton("Delete");
            delete.addActionListener(new ActionListener() {
                public void actionPerformed(ActionEvent event) {
                    if (profiles.list.size() > 1) {
                        Profile gone = current;
                        profiles.list.remove(gone);
                        profileBox.setSelectedItem(profiles.list.get(0));
                        profileBox.removeItem(gone);
                    }
                }
            });
            JPanel row = new JPanel(new BorderLayout(4, 0));
            row.add(profileBox, BorderLayout.CENTER);
            JPanel buttons = new JPanel(new FlowLayout(FlowLayout.RIGHT, 4, 0));
            buttons.add(add);
            buttons.add(delete);
            row.add(buttons, BorderLayout.EAST);
            return row;
        }

        private JPanel resolutionRow() {
            JPanel row = new JPanel();
            row.setLayout(new BoxLayout(row, BoxLayout.X_AXIS));
            row.add(widthField);
            row.add(Box.createHorizontalStrut(6));
            row.add(new JLabel("x"));
            row.add(Box.createHorizontalStrut(6));
            row.add(heightField);
            return row;
        }

        /** Cancel on the left; "Open Game Dir" and "Save Profile" on the right. */
        private JPanel buttons() {
            JButton cancel = new JButton("Cancel");
            cancel.addActionListener(new ActionListener() {
                public void actionPerformed(ActionEvent event) {
                    dialog.setVisible(false);
                    dialog.dispose();
                }
            });
            JButton open = new JButton("Open Game Dir");
            open.addActionListener(new ActionListener() {
                public void actionPerformed(ActionEvent event) {
                    try {
                        File folder = new File(profiles.instances);
                        folder.mkdirs();
                        Desktop.getDesktop().open(folder);
                    } catch (Exception e) {
                        Agent.log("Opciones: no se pudo abrir la carpeta: " + e);
                    }
                }
            });
            JButton save = new JButton("Save Profile");
            save.addActionListener(new ActionListener() {
                public void actionPerformed(ActionEvent event) {
                    save();
                }
            });
            JPanel right = new JPanel(new FlowLayout(FlowLayout.RIGHT, 6, 0));
            right.add(open);
            right.add(save);
            JPanel buttons = new JPanel(new BorderLayout());
            buttons.add(cancel, BorderLayout.WEST);
            buttons.add(right, BorderLayout.EAST);
            buttons.setBorder(new EmptyBorder(8, 0, 0, 0));
            return buttons;
        }

        private String unusedName() {
            for (int n = profiles.list.size() + 1;; n++) {
                String name = "Profile " + n;
                if (profiles.named(name) == null) {
                    return name;
                }
            }
        }

        private void display(Profile profile) {
            nameField.setText(profile.name);
            userField.setText(profile.username);
            String[] size = profile.resolution.split("x");
            boolean sized = size.length == 2;
            resolutionBox.setSelected(sized);
            widthField.setText(sized ? size[0] : "854");
            heightField.setText(sized ? size[1] : "480");
            widthField.setEnabled(sized);
            heightField.setEnabled(sized);
            java.set(profile.java);
            gameFlags.set(profile.gameFlags);
            selectVersion(profile.version);
        }

        private void rename() {
            current.name = clean(nameField.getText());
            profileBox.repaint();
        }

        /** The fields back into the profile being edited. */
        private void commit() {
            current.name = clean(nameField.getText());
            current.username = clean(userField.getText());
            current.resolution = resolutionBox.isSelected()
                    ? clean(widthField.getText()) + "x" + clean(heightField.getText()) : "";
            current.java = java.get();
            current.gameFlags = gameFlags.get();
            Version version = (Version) versionBox.getSelectedItem();
            if (version != null) {
                current.version = version.spec;
            }
            profileBox.repaint();
        }

        /** Selects the version, turning on the checkbox that hides it if need be. */
        private void selectVersion(String spec) {
            Version found = null;
            for (Version version : versions) {
                if (version.spec.equals(spec)) {
                    found = version;
                }
            }
            if (found == null) {
                found = new Version(spec, spec, "custom");
                versions.add(0, found);
            }
            JCheckBox filter = filter(found.type);
            if (filter != null) {
                filter.setSelected(true);
            }
            refilter();
            versionBox.setSelectedItem(found);
        }

        private JCheckBox filter(String type) {
            if (type.equals("snapshot")) {
                return snapshots;
            } else if (type.equals("old_beta")) {
                return betas;
            } else if (type.equals("old_alpha")) {
                return alphas;
            } else if (type.equals("fabric")) {
                return modded;
            }
            return null;
        }

        /** Fills the box with the versions the checkboxes allow, keeping the selection if shown. */
        private void refilter() {
            Version chosen = (Version) versionBox.getSelectedItem();
            versionBox.removeAllItems();
            Version keep = null;
            for (Version version : versions) {
                JCheckBox filter = filter(version.type);
                if (filter == null || filter.isSelected()) {
                    versionBox.addItem(version);
                    if (version == chosen) {
                        keep = version;
                    }
                }
            }
            if (keep != null) {
                versionBox.setSelectedItem(keep);
            }
        }

        /** Mojang's and Fabric's lists take a moment the first time, so they fill in later. */
        private void loadVersions() {
            Thread loader = new Thread("Classic versions") {
                public void run() {
                    final List<Version> loaded = new ArrayList<Version>();
                    try {
                        for (String line : Http.get(server + "/classic/versions").split("\n")) {
                            String[] fields = line.split("\t");
                            if (fields.length == 3) {
                                loaded.add(new Version(fields[0], fields[1], fields[2]));
                            }
                        }
                    } catch (IOException e) {
                        Agent.log("Opciones: sin lista de versiones: " + e.getMessage());
                        return;
                    }
                    EventQueue.invokeLater(new Runnable() {
                        public void run() {
                            Version chosen = (Version) versionBox.getSelectedItem();
                            versions.clear();
                            versions.addAll(loaded);
                            selectVersion(chosen != null ? chosen.spec : current.version);
                        }
                    });
                }
            };
            loader.setDaemon(true);
            loader.start();
        }

        /** "Save Profile": saves the profiles, applies the selected one, and closes the window. */
        private void save() {
            commit();
            profiles.selected = current.name;
            profiles.launcherFlags = launcherFlags.get();
            try {
                Http.post(server + "/classic/profiles", profiles.toText());
                // The server names the folders, renamed profiles' included.
                apply(Profiles.fetch(server).selected());
            } catch (IOException e) {
                JOptionPane.showMessageDialog(dialog, e.getMessage(), "Profile Editor", JOptionPane.ERROR_MESSAGE);
                return;
            }
            dialog.setVisible(false);
            dialog.dispose();
        }
    }

    /**
     * A checkbox that turns on a setting, and its field. Unticked, the field is greyed out and
     * shows what applies instead, and the setting is saved empty.
     */
    private static final class Toggle {
        final JCheckBox box;
        final JTextField field = new JTextField(30);
        private final String fallback;
        /** Whether the fallback is a description rather than a value to start editing from. */
        private final boolean described;

        Toggle(String label, String fallback, boolean described) {
            this.box = new JCheckBox(label);
            this.fallback = fallback;
            this.described = described;
            box.addActionListener(new ActionListener() {
                public void actionPerformed(ActionEvent event) {
                    changed();
                }
            });
        }

        private void changed() {
            field.setEnabled(box.isSelected());
            if (!box.isSelected()) {
                field.setText(fallback);
            } else if (described && field.getText().equals(fallback)) {
                field.setText("");
            }
            field.setCaretPosition(0);
        }

        void addTo(JPanel panel) {
            GridBagConstraints c = new GridBagConstraints();
            c.gridx = 0;
            c.anchor = GridBagConstraints.WEST;
            c.insets = new Insets(2, 2, 2, 8);
            panel.add(box, c);
            c.gridx = 1;
            c.weightx = 1;
            c.fill = GridBagConstraints.HORIZONTAL;
            c.insets = new Insets(2, 0, 2, 2);
            panel.add(field, c);
        }

        void set(String value) {
            box.setSelected(value.length() > 0);
            field.setEnabled(value.length() > 0);
            field.setText(value.length() > 0 ? value : fallback);
            field.setCaretPosition(0);
        }

        String get() {
            return box.isSelected() ? clean(field.getText()) : "";
        }
    }

    /** Tabs and line breaks separate the fields the server reads. */
    private static String clean(String text) {
        return text.replace('\t', ' ').replace('\n', ' ').replace('\r', ' ').trim();
    }

    private static final class Profile {
        String name;
        String username;
        String version;
        String gameFlags;
        /** {@code <width>x<height>}, or empty. */
        String resolution;
        String java;
        /** Its folder in instances, as the server names it. */
        String folder;

        Profile copy(String newName) {
            Profile copy = new Profile();
            copy.name = newName;
            copy.username = username;
            copy.version = version;
            copy.gameFlags = gameFlags;
            copy.resolution = resolution;
            copy.java = java;
            copy.folder = "";
            return copy;
        }

        public String toString() {
            return name;
        }
    }

    private static final class Version {
        final String spec;
        final String label;
        /** latest, release, snapshot, old_beta, old_alpha, fabric, or custom. */
        final String type;

        Version(String spec, String label, String type) {
            this.spec = spec;
            this.label = label;
            this.type = type;
        }

        public String toString() {
            return label;
        }
    }

    /** The server's tab-separated lines: defaults, launcher, instances, selected, and profiles. */
    private static final class Profiles {
        String defaultGameFlags = "";
        String defaultLauncherFlags = "";
        String launcherFlags = "";
        String instances = "";
        String selected = "";
        final List<Profile> list = new ArrayList<Profile>();

        static Profiles fetch(String server) throws IOException {
            Profiles profiles = new Profiles();
            for (String line : Http.get(server + "/classic/profiles").split("\n")) {
                String[] fields = line.split("\t", -1);
                if (fields[0].equals("defaults") && fields.length > 2) {
                    profiles.defaultGameFlags = fields[1];
                    profiles.defaultLauncherFlags = fields[2];
                } else if (fields[0].equals("launcher") && fields.length > 1) {
                    profiles.launcherFlags = fields[1];
                } else if (fields[0].equals("instances") && fields.length > 1) {
                    profiles.instances = fields[1];
                } else if (fields[0].equals("selected") && fields.length > 1) {
                    profiles.selected = fields[1];
                } else if (fields[0].equals("profile") && fields.length > 7) {
                    Profile profile = new Profile();
                    profile.name = fields[1];
                    profile.username = fields[2];
                    profile.version = fields[3];
                    profile.gameFlags = fields[4];
                    profile.resolution = fields[5];
                    profile.java = fields[6];
                    profile.folder = fields[7];
                    profiles.list.add(profile);
                }
            }
            if (profiles.list.isEmpty()) {
                throw new IOException("el servidor no devolvio perfiles");
            }
            return profiles;
        }

        Profile named(String name) {
            for (Profile profile : list) {
                if (profile.name.equals(name)) {
                    return profile;
                }
            }
            return null;
        }

        Profile selected() {
            Profile profile = named(selected);
            return profile != null ? profile : list.get(0);
        }

        String toText() {
            StringBuilder text = new StringBuilder();
            text.append("launcher\t").append(launcherFlags).append('\n');
            text.append("selected\t").append(selected).append('\n');
            for (Profile p : list) {
                text.append("profile\t").append(p.name).append('\t').append(p.username).append('\t')
                        .append(p.version).append('\t').append(p.gameFlags).append('\t').append(p.resolution)
                        .append('\t').append(p.java).append('\t').append(p.folder).append('\n');
            }
            return text.toString();
        }
    }
}
