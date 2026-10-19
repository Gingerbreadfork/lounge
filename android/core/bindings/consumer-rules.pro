# JNA and the generated UniFFI bindings are reached reflectively.
-keep class com.sun.jna.** { *; }
-keep class * implements com.sun.jna.** { *; }
-keep class tv.lounge.core.** { *; }
-dontwarn java.awt.**
