# ProGuard rules for rustls-platform-verifier
# The Android verifier component must not be optimized out because it looks like dead code
# to ProGuard (which cannot see JNI usage).
-keep, includedescriptorclasses class org.rustls.platformverifier.** { *; }