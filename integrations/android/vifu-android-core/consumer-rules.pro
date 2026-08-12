# JNA includes optional desktop helpers that are unreachable on Android. R8
# still resolves their signatures while shrinking a consumer application.
-dontwarn java.awt.Component
-dontwarn java.awt.GraphicsEnvironment
-dontwarn java.awt.HeadlessException
-dontwarn java.awt.Window
