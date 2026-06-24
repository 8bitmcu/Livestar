package com.livestar;

import android.app.Activity;
import android.app.WallpaperManager;
import android.content.ActivityNotFoundException;
import android.content.ComponentName;
import android.content.Intent;
import android.content.SharedPreferences;
import android.os.Bundle;
import android.view.View;
import android.widget.Button;
import android.widget.CheckBox;
import android.widget.SeekBar;
import android.widget.TextView;
import android.widget.Toast;

import java.util.function.IntFunction;

/**
 * Launcher and configuration activity. Offers sliders that tune the starfield
 * renderer plus a button that jumps to the live wallpaper preview to apply it.
 *
 * <p>Slider values persist to {@link Settings} (SharedPreferences); the wallpaper
 * service reads them when it (re)creates its surface.
 */
public class MainActivity extends Activity {

    private SharedPreferences prefs;

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        setContentView(R.layout.activity_main);

        prefs = Settings.prefs(this);

        // Stars section.
        bindSlider(R.id.density_slider, R.id.density_label,
                Settings.KEY_DENSITY, Settings.DEF_DENSITY,
                value -> getString(R.string.density_label, value));
        bindSizeSliders();
        bindSlider(R.id.brightness_slider, R.id.brightness_label,
                Settings.KEY_BRIGHTNESS, Settings.DEF_BRIGHTNESS,
                value -> getString(R.string.brightness_label, value / 100f));
        bindSlider(R.id.twinkle_slider, R.id.twinkle_label,
                Settings.KEY_TWINKLE, Settings.DEF_TWINKLE,
                value -> getString(R.string.twinkle_label, value));

        // Rendering section.
        bindSlider(R.id.fps_slider, R.id.fps_label,
                Settings.KEY_FPS, Settings.DEF_FPS,
                value -> getString(R.string.fps_label, value));

        CheckBox battery = findViewById(R.id.battery_saving_checkbox);
        battery.setChecked(prefs.getBoolean(Settings.KEY_BATTERY, Settings.DEF_BATTERY));
        battery.setOnCheckedChangeListener((view, checked) ->
                prefs.edit().putBoolean(Settings.KEY_BATTERY, checked).apply());

        Button applyButton = findViewById(R.id.apply_wallpaper_button);
        applyButton.setOnClickListener(this::onApplyWallpaperClicked);
    }

    /**
     * Wires a SeekBar to its value label and persistence. {@code formatter}
     * renders the current progress into the label text. The SeekBar's bounds are
     * defined in the layout.
     */
    private void bindSlider(int seekId, int labelId, String key, int def,
                            IntFunction<String> formatter) {
        SeekBar seek = findViewById(seekId);
        TextView label = findViewById(labelId);
        int value = prefs.getInt(key, def);
        seek.setProgress(value);
        label.setText(formatter.apply(value));
        seek.setOnSeekBarChangeListener(new SeekBar.OnSeekBarChangeListener() {
            @Override
            public void onProgressChanged(SeekBar bar, int progress, boolean fromUser) {
                label.setText(formatter.apply(progress));
                prefs.edit().putInt(key, progress).apply();
            }

            @Override
            public void onStartTrackingTouch(SeekBar bar) {
            }

            @Override
            public void onStopTrackingTouch(SeekBar bar) {
            }
        });
    }

    /**
     * Binds the two star-size SeekBars with a min/max constraint: dragging the
     * minimum past the maximum pushes the maximum up to match, and dragging the
     * maximum below the minimum pulls the minimum down to match.
     */
    private void bindSizeSliders() {
        SeekBar minSeek = findViewById(R.id.size_min_slider);
        SeekBar maxSeek = findViewById(R.id.size_max_slider);
        TextView minLabel = findViewById(R.id.size_min_label);
        TextView maxLabel = findViewById(R.id.size_max_label);

        int minValue = prefs.getInt(Settings.KEY_SIZE_MIN, Settings.DEF_SIZE_MIN);
        int maxValue = prefs.getInt(Settings.KEY_SIZE_MAX, Settings.DEF_SIZE_MAX);
        minSeek.setProgress(minValue);
        maxSeek.setProgress(maxValue);
        minLabel.setText(getString(R.string.size_min_label, minValue / 10f));
        maxLabel.setText(getString(R.string.size_max_label, maxValue / 10f));

        minSeek.setOnSeekBarChangeListener(new SeekBar.OnSeekBarChangeListener() {
            @Override
            public void onProgressChanged(SeekBar bar, int progress, boolean fromUser) {
                minLabel.setText(getString(R.string.size_min_label, progress / 10f));
                prefs.edit().putInt(Settings.KEY_SIZE_MIN, progress).apply();
                if (progress > maxSeek.getProgress()) {
                    maxSeek.setProgress(progress);
                }
            }

            @Override
            public void onStartTrackingTouch(SeekBar bar) {
            }

            @Override
            public void onStopTrackingTouch(SeekBar bar) {
            }
        });

        maxSeek.setOnSeekBarChangeListener(new SeekBar.OnSeekBarChangeListener() {
            @Override
            public void onProgressChanged(SeekBar bar, int progress, boolean fromUser) {
                maxLabel.setText(getString(R.string.size_max_label, progress / 10f));
                prefs.edit().putInt(Settings.KEY_SIZE_MAX, progress).apply();
                if (progress < minSeek.getProgress()) {
                    minSeek.setProgress(progress);
                }
            }

            @Override
            public void onStartTrackingTouch(SeekBar bar) {
            }

            @Override
            public void onStopTrackingTouch(SeekBar bar) {
            }
        });
    }

    private void onApplyWallpaperClicked(View view) {
        ComponentName component =
                new ComponentName(this, LivestarWallpaperService.class);

        // Preferred path: open the live wallpaper preview directly for our service.
        Intent intent = new Intent(WallpaperManager.ACTION_CHANGE_LIVE_WALLPAPER);
        intent.putExtra(WallpaperManager.EXTRA_LIVE_WALLPAPER_COMPONENT, component);

        try {
            startActivity(intent);
            return;
        } catch (ActivityNotFoundException ignored) {
            // Some devices/OEMs don't honor the direct-preview extra; fall back below.
        }

        // Fallback: open the generic live wallpaper chooser.
        try {
            startActivity(new Intent(WallpaperManager.ACTION_LIVE_WALLPAPER_CHOOSER));
        } catch (ActivityNotFoundException e) {
            Toast.makeText(this, R.string.apply_wallpaper_unavailable, Toast.LENGTH_LONG).show();
        }
    }
}
