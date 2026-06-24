package com.livestar;

import android.content.Context;
import android.content.SharedPreferences;

/**
 * Shared persisted settings for the starfield renderer.
 *
 * <p>{@link MainActivity} (the configuration UI) and {@link LivestarWallpaperService}
 * (the renderer) run in separate processes, so they exchange values through
 * {@link SharedPreferences}. Stored values are in raw slider/progress units; the
 * service converts them to the float parameters the native renderer expects.
 */
final class Settings {

    static final String PREFS = "livestar_settings";

    /** Star density factor, 0..100 (percent of the area-derived base count). */
    static final String KEY_DENSITY = "density";
    /** Minimum star size, 0..100 -> /10 = 0.0..10.0 px. */
    static final String KEY_SIZE_MIN = "star_size_min";
    /** Maximum star size, 0..100 -> /10 = 0.0..10.0 px. */
    static final String KEY_SIZE_MAX = "star_size_max";
    /** Brightness multiplier, 0..100 -> /100 = 0.0..1.0. */
    static final String KEY_BRIGHTNESS = "brightness";
    /** Fraction of stars that twinkle, 0..100 percent. */
    static final String KEY_TWINKLE = "twinkle";
    /** Target render rate, 1..60 fps. */
    static final String KEY_FPS = "target_fps";
    /** Battery savings mode (drives wgpu PowerPreference). */
    static final String KEY_BATTERY = "battery_saving";

    static final int DEF_DENSITY = 100;
    static final int DEF_SIZE_MIN = 10;   // 1.0 px
    static final int DEF_SIZE_MAX = 40;   // 4.0 px
    static final int DEF_BRIGHTNESS = 100; // 1.0
    static final int DEF_TWINKLE = 35;    // 35%
    static final int DEF_FPS = 60;
    static final boolean DEF_BATTERY = false;

    static SharedPreferences prefs(Context context) {
        return context.getSharedPreferences(PREFS, Context.MODE_PRIVATE);
    }

    private Settings() {
    }
}
