package com.artcraft.ime;

/**
 * Direct native JNI bridge to wake the Android event loop instantly when a key is typed or deleted.
 */
public final class ArtCraftIme {
    private ArtCraftIme() {}

    static {
        try {
            System.loadLibrary("__LIB_NAME__");
        } catch (Throwable ignored) {}
    }

    public static native void wake();
}

