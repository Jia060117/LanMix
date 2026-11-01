plugins { id("com.android.application") }

android {
    namespace = "io.lanmix.receiver"
    compileSdk = 37
    ndkVersion = "30.0.14904198"

    defaultConfig {
        applicationId = "io.lanmix.receiver"
        minSdk = 26
        targetSdk = 37
        versionCode = 9
        versionName = "0.2.2"
        ndk { abiFilters += listOf("arm64-v8a", "armeabi-v7a") }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    buildTypes { release { isMinifyEnabled = false } }
    buildFeatures { buildConfig = true }
    testOptions { unitTests.isIncludeAndroidResources = true }
}

// Never silently package a UI-only APK. build.ps1 cross-compiles JNI first.
val verifyNativeLibraries = tasks.register("verifyNativeLibraries") {
    doLast {
        listOf("arm64-v8a", "armeabi-v7a").forEach { abi ->
            check(file("src/main/jniLibs/$abi/liblanmix_android.so").isFile) {
                "Missing $abi JNI library. Run android/build.ps1 first."
            }
        }
    }
}
tasks.named("preBuild") { dependsOn(verifyNativeLibraries) }

dependencies {
    testImplementation("junit:junit:4.13.2")
    testImplementation("org.robolectric:robolectric:4.17")
}
