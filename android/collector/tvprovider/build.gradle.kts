plugins {
    alias(libs.plugins.android.library)
}

android {
    namespace = "tv.lounge.collector.tvprovider"
    compileSdk = 37
    defaultConfig { minSdk = 28 }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

dependencies {
    implementation(project(":core:bindings"))
    implementation(project(":core:platform"))
    implementation(libs.androidx.core.ktx)
    implementation(libs.androidx.tvprovider)
    implementation(libs.kotlinx.coroutines.android)
}
