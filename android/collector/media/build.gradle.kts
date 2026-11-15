plugins {
    alias(libs.plugins.android.library)
}

android {
    namespace = "tv.lounge.collector.media"
    compileSdk = 37
    defaultConfig {
        minSdk = 28
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    // The tests of what a notification says build real ones on the JVM.
    testOptions { unitTests { isIncludeAndroidResources = true } }
}

dependencies {
    testImplementation(libs.junit)
    testImplementation(libs.robolectric)
    androidTestImplementation(libs.junit)
    androidTestImplementation(libs.androidx.test.runner)
    implementation(project(":core:bindings"))
    implementation(project(":core:platform"))
    implementation(libs.androidx.core.ktx)
    implementation(libs.kotlinx.coroutines.android)
}
