package com.livestar;

import android.view.Surface;

/**
 * Thin JNI boundary to the native Rust renderer.
 *
 * <p>The Java side owns the {@link Surface} (from the wallpaper engine) and hands it
 * across to native code, which uses it to create an {@code ANativeWindow} and initialize
 * wgpu's Vulkan backend. Native state lives behind an opaque {@code long} handle.
 */
public final class NativeBridge {

    static {
        System.loadLibrary("livestar");
    }

    private NativeBridge() {
    }

    /**
     * Initializes the native renderer over the given surface.
     *
     * @param density        star count factor, 0.0..1.0 of the area-derived base count
     * @param starSizeMin    minimum star size in pixels
     * @param starSizeMax    maximum star size in pixels
     * @param brightness     brightness multiplier, 0.0..1.0
     * @param twinkle        fraction of stars that twinkle, 0.0..1.0
     * @param batterySaving  whether to request a low-power GPU adapter
     * @return an opaque handle to native state, or 0 if initialization failed.
     */
    public static native long onSurfaceCreated(
            Surface surface,
            float density,
            float starSizeMin,
            float starSizeMax,
            float brightness,
            float twinkle,
            boolean batterySaving);

    /** Reconfigures the swapchain when the surface size changes. */
    public static native void onSurfaceChanged(long handle, int width, int height);

    /** Renders a single frame; called once per vsync while the wallpaper is visible. */
    public static native void onFrame(long handle);

    /** Tears down native state and releases the surface. */
    public static native void onSurfaceDestroyed(long handle);

    /** Notifies native code of wallpaper visibility changes (drives the render loop). */
    public static native void onVisibilityChanged(long handle, boolean visible);
}
