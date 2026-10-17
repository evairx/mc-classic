package classic;

import java.io.ByteArrayInputStream;
import java.io.DataInputStream;
import java.io.IOException;
import java.lang.instrument.ClassFileTransformer;
import java.security.ProtectionDomain;

/**
 * Two fixes to the 2012 launcher, each a same-length change to one call, so no bytecode offset
 * moves:
 * <ul>
 * <li>LogoPanel calls setOpaque(true) but paints only logo.png, which has transparent pixels.
 * Swing trusts opaque components to cover their area, so it never repaints what lies behind the
 * logo, and stale pixels show through it: after a failed login, copies of the "Try again" and
 * "Play offline" buttons. The call becomes setOpaque(false): iconst_1 to iconst_0.</li>
 * <li>GameUpdater.extractNatives checks that each native library is signed with the launcher's
 * own 2012 certificate. On the Java 8 that runs the launcher now, that signature is still
 * readable, and the natives the local server repackages from Mojang's current files are not
 * signed by it, so every applet version would fail with "Unable to validate certificate chain".
 * The call to validateCertificateChain becomes pop2 nop nop, which drops its two arguments.</li>
 * </ul>
 */
final class LauncherFixes implements ClassFileTransformer {
    private static final byte NOP = 0x00;
    private static final byte ICONST_0 = 0x03;
    private static final byte ICONST_1 = 0x04;
    private static final byte POP2 = 0x58;
    private static final byte INVOKEVIRTUAL = (byte) 0xB6;
    private static final byte INVOKESTATIC = (byte) 0xB8;

    public byte[] transform(ClassLoader loader, String className, Class<?> redefined,
            ProtectionDomain domain, byte[] bytes) {
        try {
            if ("net/minecraft/LogoPanel".equals(className)) {
                return patch(className, bytes, "setOpaque", "(Z)V",
                        new byte[] { ICONST_1, INVOKEVIRTUAL }, new byte[] { ICONST_0, INVOKEVIRTUAL, -1, -1 },
                        "setOpaque(true) -> setOpaque(false)");
            }
            if ("net/minecraft/GameUpdater".equals(className)) {
                return patch(className, bytes, "validateCertificateChain",
                        "([Ljava/security/cert/Certificate;[Ljava/security/cert/Certificate;)V",
                        new byte[] { INVOKESTATIC }, new byte[] { POP2, NOP, NOP },
                        "sin validateCertificateChain para las natives");
            }
        } catch (Throwable t) {
            Agent.log("No se pudo corregir " + className + ": " + t);
        }
        return null;
    }

    /**
     * Replaces the one call to {@code name desc}, which {@code before} plus the method's
     * constant-pool index must match, with {@code after}, where -1 keeps the original byte.
     * Returns null, leaving the class as it is, unless exactly one such call exists.
     */
    private static byte[] patch(String className, byte[] in, String name, String desc, byte[] before,
            byte[] after, String what) throws IOException {
        ConstantPool pool = new ConstantPool(in);
        int method = pool.methodref(name, desc);
        byte[] call = new byte[before.length + 2];
        System.arraycopy(before, 0, call, 0, before.length);
        call[before.length] = (byte) (method >> 8);
        call[before.length + 1] = (byte) method;
        int at = -1;
        for (int i = pool.end; method > 0 && i + call.length <= in.length; i++) {
            if (matches(in, i, call)) {
                if (at >= 0) {
                    at = -2;
                    break;
                }
                at = i;
            }
        }
        if (at < 0) {
            Agent.log(className + ": no se encontro una unica llamada a " + name + "; se deja igual");
            return null;
        }
        byte[] out = in.clone();
        for (int i = 0; i < after.length; i++) {
            if (after[i] != -1) {
                out[at + i] = after[i];
            }
        }
        Agent.log(className + ": " + what);
        return out;
    }

    private static boolean matches(byte[] in, int at, byte[] call) {
        for (int i = 0; i < call.length; i++) {
            if (in[at + i] != call[i]) {
                return false;
            }
        }
        return true;
    }

    /** The parts of a class file's constant pool these patches need. */
    private static final class ConstantPool {
        final int count;
        final int[] tags;
        final int[] first;
        final int[] second;
        final String[] utf8;
        /** Offset of the first byte after the pool. */
        final int end;

        ConstantPool(byte[] in) throws IOException {
            DataInputStream reader = new DataInputStream(new ByteArrayInputStream(in));
            if (reader.readInt() != 0xCAFEBABE) {
                throw new IOException("no es un archivo .class");
            }
            reader.readInt(); // minor + major version
            count = reader.readUnsignedShort();
            tags = new int[count];
            first = new int[count];
            second = new int[count];
            utf8 = new String[count];
            for (int i = 1; i < count; i++) {
                tags[i] = reader.readUnsignedByte();
                switch (tags[i]) {
                    case 1: utf8[i] = reader.readUTF(); break;
                    case 7: case 8: case 16: case 19: case 20: reader.readUnsignedShort(); break;
                    case 15: reader.readUnsignedByte(); reader.readUnsignedShort(); break;
                    case 9: case 10: case 11: case 12:
                        first[i] = reader.readUnsignedShort();
                        second[i] = reader.readUnsignedShort();
                        break;
                    case 3: case 4: case 17: case 18: reader.readInt(); break;
                    case 5: case 6: reader.readLong(); i++; break; // two constant-pool slots
                    default: throw new IOException("tag de constant pool desconocido: " + tags[i]);
                }
            }
            end = in.length - reader.available();
        }

        /** Index of the Methodref {@code name desc}, or -1. */
        int methodref(String name, String desc) {
            for (int i = 1; i < count; i++) {
                if (tags[i] == 10 && name.equals(utf8[first[second[i]]]) && desc.equals(utf8[second[second[i]]])) {
                    return i;
                }
            }
            return -1;
        }
    }
}
