import java.util.Properties

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("rust")
}

val tauriProperties = Properties().apply {
    val propFile = file("tauri.properties")
    if (propFile.exists()) {
        propFile.inputStream().use { load(it) }
    }
}

// Signing, from a key that is not in this repository and must never be.
//
// An unsigned `.apk` installs on nothing: Android refuses it, Play refuses it, and a phone with
// developer mode on refuses it too. So the build has to be able to sign — and the only thing that
// can safely live here is the *plumbing*, reading a keystore from wherever the person building it
// keeps one.
//
// Two sources, in order, because they answer two different situations:
//
//   `keystore.properties` beside this file, git-ignored — someone building a release on their own
//   machine, who should not have to export four variables every time.
//
//   `ANDROID_KEYSTORE` and friends in the environment — CI, where the keystore arrives as a secret
//   and never touches a file that could be committed or uploaded as an artefact.
//
// With neither, the build produces the same unsigned `.apk` it always has, and says so. That is
// what CI does on every pull request, and it is the right default: a build that fails for want of a
// key nobody has would make the thing unbuildable for contributors.
val keystoreProperties = Properties().apply {
    val propFile = file("keystore.properties")
    if (propFile.exists()) {
        propFile.inputStream().use { load(it) }
    }
}

fun signingValue(key: String, env: String): String? =
    keystoreProperties.getProperty(key) ?: System.getenv(env)

val keystorePath = signingValue("storeFile", "ANDROID_KEYSTORE")
val keystorePassword = signingValue("storePassword", "ANDROID_KEYSTORE_PASSWORD")
val keyAliasName = signingValue("keyAlias", "ANDROID_KEY_ALIAS")
val keyPasswordValue = signingValue("keyPassword", "ANDROID_KEY_PASSWORD") ?: keystorePassword

val canSign = keystorePath != null && keystorePassword != null && keyAliasName != null &&
    file(keystorePath).exists()

android {
    compileSdk = 36
    namespace = "app.summo.mobile"
    defaultConfig {
        manifestPlaceholders["usesCleartextTraffic"] = "false"
        applicationId = "app.summo.mobile"
        minSdk = 26
        targetSdk = 36
        versionCode = tauriProperties.getProperty("tauri.android.versionCode", "1").toInt()
        versionName = tauriProperties.getProperty("tauri.android.versionName", "1.0")
    }
    signingConfigs {
        if (canSign) {
            create("release") {
                storeFile = file(keystorePath!!)
                storePassword = keystorePassword
                keyAlias = keyAliasName
                keyPassword = keyPasswordValue
            }
        }
    }
    buildTypes {
        getByName("debug") {
            manifestPlaceholders["usesCleartextTraffic"] = "true"
            isDebuggable = true
            isJniDebuggable = true
            isMinifyEnabled = false
            packaging {                jniLibs.keepDebugSymbols.add("*/arm64-v8a/*.so")
                jniLibs.keepDebugSymbols.add("*/armeabi-v7a/*.so")
                jniLibs.keepDebugSymbols.add("*/x86/*.so")
                jniLibs.keepDebugSymbols.add("*/x86_64/*.so")
            }
        }
        getByName("release") {
            if (canSign) {
                signingConfig = signingConfigs.getByName("release")
                logger.lifecycle("signing the release with $keystorePath")
            } else {
                // Said out loud. An unsigned artefact that looks like a release is the kind of
                // thing that gets uploaded, downloaded, and refused by a phone hours later.
                logger.lifecycle("no keystore — the release .apk will be unsigned and will not install")
            }
            isMinifyEnabled = true
            proguardFiles(
                *fileTree(".") { include("**/*.pro") }
                    .plus(getDefaultProguardFile("proguard-android-optimize.txt"))
                    .toList().toTypedArray()
            )
        }
    }
    kotlinOptions {
        jvmTarget = "1.8"
    }
    buildFeatures {
        buildConfig = true
    }
}

rust {
    rootDirRel = "../../../"
}

dependencies {
    implementation("androidx.webkit:webkit:1.14.0")
    implementation("androidx.appcompat:appcompat:1.7.1")
    implementation("androidx.activity:activity-ktx:1.10.1")
    implementation("com.google.android.material:material:1.12.0")
    implementation("androidx.lifecycle:lifecycle-process:2.10.0")
    testImplementation("junit:junit:4.13.2")
    androidTestImplementation("androidx.test.ext:junit:1.1.4")
    androidTestImplementation("androidx.test.espresso:espresso-core:3.5.0")
}

apply(from = "tauri.build.gradle.kts")