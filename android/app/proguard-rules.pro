# Generated UniFFI bindings and JNA are reached reflectively.
-keep class tv.lounge.core.** { *; }
-keep class com.sun.jna.** { *; }
-keep class * implements com.sun.jna.** { *; }
-dontwarn java.awt.**
