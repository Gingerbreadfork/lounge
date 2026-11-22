plugins {
    alias(libs.plugins.android.library)
}

android {
    namespace = "tv.lounge.update"
    compileSdk = 37
    defaultConfig { minSdk = 28 }
    // The wording tests read real string resources on the JVM.
    testOptions { unitTests { isIncludeAndroidResources = true } }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

dependencies {
    implementation(project(":power"))
    implementation(libs.androidx.core.ktx)
    implementation(libs.kotlinx.coroutines.android)
    testImplementation(libs.junit)
    testImplementation(libs.robolectric)
    testImplementation("org.json:json:20240303")
}
