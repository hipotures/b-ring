# UniFFI bindings reach the Rust library through JNA, by reflection: keep JNA and
# every generated binding type (structs and callbacks are mapped by field name).
-keep class com.sun.jna.** { *; }
-keepclassmembers class * extends com.sun.jna.** { public *; }
-keep class io.github.hipotures.bring.ffi.** { *; }
-dontwarn java.awt.**
