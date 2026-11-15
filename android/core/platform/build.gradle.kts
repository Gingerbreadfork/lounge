plugins {
    alias(libs.plugins.android.library)
}

android {
    namespace = "tv.lounge.platform"
    compileSdk = 37
    defaultConfig { minSdk = 28 }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    // The tests of what is handed to other apps build real intents on the JVM.
    testOptions { unitTests { isIncludeAndroidResources = true } }
}


dependencies {
    testImplementation(libs.junit)
    testImplementation(libs.robolectric)
    implementation(project(":core:bindings"))
    implementation(libs.androidx.core.ktx)
    implementation(libs.androidx.datastore.preferences)
    implementation(libs.kotlinx.coroutines.android)
    implementation(libs.androidx.tvprovider)
}
