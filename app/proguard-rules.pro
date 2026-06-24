# Keep the JNI bridge entry points so native method lookups resolve.
-keepclasseswithmembernames class * {
    native <methods>;
}
-keep class com.livestar.NativeBridge { *; }
