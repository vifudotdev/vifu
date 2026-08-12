plugins {
    alias(libs.plugins.android.library) apply false
    alias(libs.plugins.kotlin.android) apply false
    alias(libs.plugins.maven.publish) apply false
}

allprojects {
    group = "dev.vifu"
    version = providers.gradleProperty("VERSION_NAME").getOrElse("0.1.12-SNAPSHOT")
}
