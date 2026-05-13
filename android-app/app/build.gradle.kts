import java.util.Properties

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

// Load release-signing config from keystore.properties at the project
// root if present. CI without secrets, fresh checkouts, and the
// example flavor (which has no keystore) all silently skip — only
// real release variants get the signing config wired below.
val keystoreProps = Properties().apply {
    val f = rootProject.file("keystore.properties")
    if (f.exists()) f.inputStream().use { load(it) }
}
val hasReleaseKeystore = keystoreProps.getProperty("storeFile")?.let {
    rootProject.file(it).exists()
} ?: false

// PUBLIC build config. Defines ONE example flavor (com.example.rusterando)
// with placeholder values so the repo can be cloned and built by anyone
// without leaking tenant-specific config.
//
// To ship a real app under YOUR own applicationId + shop:
//
//   1. cp app/build.local.example.gradle.kts app/build.local.gradle.kts
//   2. Edit the local file: applicationId, START_URL, ASSOCIATED_DOMAIN,
//      app_name for every flavor you ship. Add as many as you want.
//   3. ./gradlew :app:assemble<YourFlavor>Debug
//
// build.local.gradle.kts is gitignored — your real flavors stay on
// your machine. The block at the bottom of this file picks it up
// automatically when present.
android {
    namespace = "eu.trahe.rusterando"
    compileSdk = 34

    defaultConfig {
        minSdk = 24
        targetSdk = 34
        versionCode = 1
        versionName = "0.1.0"
    }

    flavorDimensions += "shop"

    productFlavors {
        // Example flavor — placeholder bundle id + empty URL. Builds
        // out of the box (debug only; release would need a keystore).
        // Real shippable flavors are declared in flavors.local.json
        // (gitignored) and registered by the loop below.
        create("example") {
            dimension = "shop"
            applicationId = "com.example.rusterando"
            resValue("string", "app_name", "Rusterando")
            buildConfigField("String", "START_URL", "\"\"")
            buildConfigField("String", "ASSOCIATED_DOMAIN", "\"\"")
        }

        // Tenant flavors loaded from flavors.local.json. The file is
        // gitignored; flavors.local.example.json next to it is the
        // public template documenting the shape. Skipped silently
        // when the file is missing so OSS clones still build.
        val localFlavorsFile = file("flavors.local.json")
        if (localFlavorsFile.exists()) {
            val text = localFlavorsFile.readText()
            // Tiny ad-hoc parser: avoids pulling in org.json /
            // kotlinx.serialization just for ~5 keys per flavor.
            // Each entry is `{ "name": "...", "applicationId": "...",
            // "appName": "...", "startUrl": "...", "associatedDomain": "..." }`.
            val entryRegex = Regex(
                """\{[^{}]*"name"\s*:\s*"([^"]+)"[^{}]*"applicationId"\s*:\s*"([^"]+)"""" +
                    """[^{}]*"appName"\s*:\s*"([^"]+)"""" +
                    """[^{}]*"startUrl"\s*:\s*"([^"]*)"""" +
                    """[^{}]*"associatedDomain"\s*:\s*"([^"]*)"[^{}]*\}""",
                RegexOption.DOT_MATCHES_ALL,
            )
            for (m in entryRegex.findAll(text)) {
                val (name, appId, appName, startUrl, assocDomain) = m.destructured
                create(name) {
                    dimension = "shop"
                    applicationId = appId
                    resValue("string", "app_name", appName)
                    buildConfigField("String", "START_URL", "\"$startUrl\"")
                    buildConfigField("String", "ASSOCIATED_DOMAIN", "\"$assocDomain\"")
                }
            }
        }
    }

    buildFeatures {
        buildConfig = true
    }

    // Release-signing config. Populated only when keystore.properties +
    // the .jks both exist; otherwise release builds emit unsigned APKs.
    if (hasReleaseKeystore) {
        signingConfigs {
            create("release") {
                storeFile = rootProject.file(keystoreProps.getProperty("storeFile"))
                storePassword = keystoreProps.getProperty("storePassword")
                keyAlias = keystoreProps.getProperty("keyAlias")
                keyPassword = keystoreProps.getProperty("keyPassword")
            }
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro",
            )
            if (hasReleaseKeystore) {
                signingConfig = signingConfigs.getByName("release")
            }
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    kotlinOptions {
        jvmTarget = "17"
    }
}

dependencies {
    implementation("androidx.core:core-ktx:1.13.1")
    implementation("androidx.appcompat:appcompat:1.7.0")
    implementation("androidx.activity:activity-ktx:1.9.2")
    implementation("androidx.webkit:webkit:1.11.0")
    implementation("com.google.android.material:material:1.12.0")

    // Encrypted SharedPreferences for the role-password store —
    // the Android equivalent of the iOS Keychain. Backed by the
    // AndroidKeyStore-held master key.
    implementation("androidx.security:security-crypto:1.1.0-alpha06")

    // Kotlin coroutines so RoleSwitcher can do async HTTP without
    // pulling in OkHttp; the platform HttpURLConnection is enough.
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.8.1")
}

