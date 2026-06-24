package com.livestar;

import android.content.SharedPreferences;
import android.service.wallpaper.WallpaperService;
import android.view.Choreographer;
import android.view.SurfaceHolder;

/**
 * Live wallpaper service. Each engine owns one drawing surface, which is captured here
 * and forwarded across the JNI boundary to the native wgpu renderer.
 */
public class LivestarWallpaperService extends WallpaperService {

    @Override
    public Engine onCreateEngine() {
        return new LivestarEngine();
    }

    private final class LivestarEngine extends Engine {

        /** Opaque handle to native renderer state; 0 when nothing is initialized. */
        private long nativeHandle = 0L;

        /** Whether the per-frame render loop is currently running. */
        private boolean rendering = false;

        /** Minimum time between rendered frames, derived from the target FPS setting. */
        private long frameIntervalNanos = 1_000_000_000L / Settings.DEF_FPS;

        /** Timestamp of the last rendered frame; used to throttle to the target FPS. */
        private long lastFrameNanos = 0L;

        /** Drives native renders at the target FPS, re-posting itself while active. */
        private final Choreographer.FrameCallback frameCallback = new Choreographer.FrameCallback() {
            @Override
            public void doFrame(long frameTimeNanos) {
                if (!rendering || nativeHandle == 0L || !isVisible()) {
                    return;
                }
                // Throttle to the configured target FPS to preserve battery.
                if (frameTimeNanos - lastFrameNanos >= frameIntervalNanos) {
                    lastFrameNanos = frameTimeNanos;
                    NativeBridge.onFrame(nativeHandle);
                }
                Choreographer.getInstance().postFrameCallback(this);
            }
        };

        private void startLoop() {
            if (!rendering) {
                rendering = true;
                Choreographer.getInstance().postFrameCallback(frameCallback);
            }
        }

        private void stopLoop() {
            if (rendering) {
                rendering = false;
                Choreographer.getInstance().removeFrameCallback(frameCallback);
            }
        }

        @Override
        public void onSurfaceCreated(SurfaceHolder holder) {
            super.onSurfaceCreated(holder);
            android.util.Log.i("Livestar", "onSurfaceCreated: surface=" + holder.getSurface());
        }

        @Override
        public void onSurfaceChanged(SurfaceHolder holder, int format, int width, int height) {
            super.onSurfaceChanged(holder, format, width, height);
            android.util.Log.i("Livestar", "onSurfaceChanged: " + width + "x" + height);
            if (nativeHandle == 0L) {
                SharedPreferences prefs = Settings.prefs(LivestarWallpaperService.this);
                float density = prefs.getInt(Settings.KEY_DENSITY, Settings.DEF_DENSITY) / 100f;
                float sizeMin = prefs.getInt(Settings.KEY_SIZE_MIN, Settings.DEF_SIZE_MIN) / 10f;
                float sizeMax = prefs.getInt(Settings.KEY_SIZE_MAX, Settings.DEF_SIZE_MAX) / 10f;
                float brightness = prefs.getInt(Settings.KEY_BRIGHTNESS, Settings.DEF_BRIGHTNESS) / 100f;
                float twinkle = prefs.getInt(Settings.KEY_TWINKLE, Settings.DEF_TWINKLE) / 100f;
                boolean batterySaving = prefs.getBoolean(Settings.KEY_BATTERY, Settings.DEF_BATTERY);
                int targetFps = Math.max(1, prefs.getInt(Settings.KEY_FPS, Settings.DEF_FPS));
                frameIntervalNanos = 1_000_000_000L / targetFps;
                lastFrameNanos = 0L;

                nativeHandle = NativeBridge.onSurfaceCreated(
                        holder.getSurface(), density, sizeMin, sizeMax, brightness, twinkle, batterySaving);
                android.util.Log.i("Livestar", "onSurfaceCreated: handle=" + nativeHandle);
                if (nativeHandle == 0L) {
                    drawFallback(holder);
                } else if (isVisible()) {
                    startLoop();
                }
            } else {
                NativeBridge.onSurfaceChanged(nativeHandle, width, height);
            }
        }

        private void drawFallback(SurfaceHolder holder) {
            android.graphics.Canvas canvas = holder.lockCanvas();
            if (canvas != null) {
                // Draw a dark blue/grey background as fallback
                canvas.drawColor(0xFF1A1A2E); 
                holder.unlockCanvasAndPost(canvas);
            }
        }

        @Override
        public void onVisibilityChanged(boolean visible) {
            super.onVisibilityChanged(visible);
            if (nativeHandle != 0L) {
                NativeBridge.onVisibilityChanged(nativeHandle, visible);
            }
            // Only animate while visible so the wallpaper doesn't burn battery
            // rendering frames nobody can see.
            if (visible) {
                startLoop();
            } else {
                stopLoop();
            }
        }

        @Override
        public void onSurfaceDestroyed(SurfaceHolder holder) {
            stopLoop();
            if (nativeHandle != 0L) {
                NativeBridge.onSurfaceDestroyed(nativeHandle);
                nativeHandle = 0L;
            }
            super.onSurfaceDestroyed(holder);
        }
    }
}
