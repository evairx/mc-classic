package classic;

import java.io.ByteArrayInputStream;
import java.io.ByteArrayOutputStream;
import java.io.DataInputStream;
import java.io.DataOutputStream;
import java.io.IOException;
import java.lang.instrument.ClassFileTransformer;
import java.security.ProtectionDomain;

/**
 * Rewrites string constants in classes as they load: every URL of a dead 2012 host becomes
 * {@code <server>/<host>/<path>}, the "Need account?" link goes to today's minecraft.net, the
 * pinned key name becomes the agent's own key, and the APPDATA lookup misses so the game folder
 * follows user.home. Only the constant pool changes, so no bytecode offsets need fixing.
 */
final class UrlRewriter implements ClassFileTransformer {
    /** The launcher's "Need account?" link; minecraft.net answers it with a 404 today. */
    private static final String REGISTER_PAGE = "http://www.minecraft.net/register.jsp";
    /** Where accounts are made now. The site adds the visitor's language (/es-es/login). */
    private static final String ACCOUNT_PAGE = "https://www.minecraft.net/login";

    private final String server;

    UrlRewriter(String server) {
        this.server = server;
    }

    public byte[] transform(ClassLoader loader, String className, Class<?> redefined,
            ProtectionDomain domain, byte[] bytes) {
        if (loader == null || className == null || isPlatformClass(className)) {
            return null;
        }
        try {
            return rewrite(className, bytes);
        } catch (Throwable t) {
            Agent.log("No se pudo revisar " + className + ": " + t);
            return null;
        }
    }

    private static boolean isPlatformClass(String name) {
        return name.startsWith("java/") || name.startsWith("javax/") || name.startsWith("sun/")
                || name.startsWith("com/sun/") || name.startsWith("jdk/") || name.startsWith("classic/");
    }

    /** Replacement for a constant-pool string, or null to keep it. */
    String replacement(String value) {
        if (value.equals(REGISTER_PAGE)) {
            return ACCOUNT_PAGE;
        }
        if (value.equals("minecraft.key")) {
            return "/" + Agent.PIN_RESOURCE;
        }
        if (value.equals("APPDATA")) {
            return "CLASSIC_SIN_APPDATA"; // never set, so System.getenv returns null
        }
        for (String host : Agent.HOSTS) {
            String rest = afterPrefix(value, "http://" + host + "/");
            if (rest == null) {
                rest = afterPrefix(value, "https://" + host + "/");
            }
            if (rest != null) {
                return server + "/" + host + "/" + rest;
            }
        }
        return null;
    }

    private static String afterPrefix(String value, String prefix) {
        return value.startsWith(prefix) ? value.substring(prefix.length()) : null;
    }

    private byte[] rewrite(String className, byte[] in) throws IOException {
        DataInputStream reader = new DataInputStream(new ByteArrayInputStream(in));
        if (reader.readInt() != 0xCAFEBABE) {
            return null;
        }
        ByteArrayOutputStream buffer = new ByteArrayOutputStream(in.length + 256);
        DataOutputStream writer = new DataOutputStream(buffer);
        writer.writeInt(0xCAFEBABE);
        writer.writeInt(reader.readInt()); // minor + major version
        int count = reader.readUnsignedShort();
        writer.writeShort(count);
        boolean changed = false;
        for (int i = 1; i < count; i++) {
            int tag = reader.readUnsignedByte();
            writer.writeByte(tag);
            switch (tag) {
                case 1: // Utf8; DataInput's modified UTF-8 is the class-file encoding
                    String value = reader.readUTF();
                    String replaced = replacement(value);
                    if (replaced != null) {
                        Agent.log(className + ": " + value + " -> " + replaced);
                        value = replaced;
                        changed = true;
                    }
                    writer.writeUTF(value);
                    break;
                case 7: case 8: case 16: case 19: case 20: // Class, String, MethodType, Module, Package
                    writer.writeShort(reader.readUnsignedShort());
                    break;
                case 15: // MethodHandle
                    writer.writeByte(reader.readUnsignedByte());
                    writer.writeShort(reader.readUnsignedShort());
                    break;
                case 3: case 4: case 9: case 10: case 11: case 12: case 17: case 18:
                    writer.writeInt(reader.readInt());
                    break;
                case 5: case 6: // Long and Double take two constant-pool slots
                    writer.writeLong(reader.readLong());
                    i++;
                    break;
                default:
                    throw new IOException("tag de constant pool desconocido: " + tag);
            }
        }
        if (!changed) {
            return null;
        }
        // Everything after the constant pool refers to it by index, so it is copied as is.
        byte[] rest = new byte[reader.available()];
        reader.readFully(rest);
        writer.write(rest);
        return buffer.toByteArray();
    }
}
