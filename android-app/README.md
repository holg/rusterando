# Android shell — Rusterando

Thin WebView app pointed at the live shop. Counterpart to `ios-app/`.

One Gradle project; every product flavor shares the same Kotlin source
tree. The public repo ships one `example` flavor (placeholder bundle id,
empty start URL); the flavors you actually ship (e.g. `myshop`) come from
`app/flavors.local.json` (gitignored; template:
`app/flavors.local.example.json`). Each flavor sets its own
`applicationId`, `app_name`, `BuildConfig.START_URL`, and the
associated-domain string used by the deep-link intent filter and
(later) App Links verification.

## First-time setup

The Gradle wrapper JAR is not checked in. Generate it from any
machine with a system Gradle installed:

```bash
cd android-app
gradle wrapper --gradle-version 8.9
```

That writes `gradle/wrapper/gradle-wrapper.jar` plus the `gradlew`
and `gradlew.bat` scripts. From there:

```bash
./gradlew :app:assembleExampleDebug      # build the brand-neutral example APK
./gradlew :app:assembleMyshopDebug       # a flavor from flavors.local.json (here `myshop`)
./gradlew :app:installMyshopDebug        # install + launch on a connected device
```

Open in Android Studio for ergonomic editing. Configure the SDK path
in `local.properties` (untracked):

```
sdk.dir=/Users/you/Library/Android/sdk
```

## Layout

```
android-app/
├── settings.gradle.kts         single :app module
├── build.gradle.kts            root: AGP + Kotlin plugin versions
├── gradle.properties           AndroidX, JVM heap
├── gradle/wrapper/             wrapper props (JAR generated locally)
└── app/
    ├── build.gradle.kts        productFlavors: example (+ flavors.local.json)
    ├── proguard-rules.pro      minimal — nothing to keep yet
    └── src/
        ├── main/
        │   ├── AndroidManifest.xml
        │   ├── kotlin/eu/trahe/rusterando/MainActivity.kt
        │   └── res/            shared theme, launcher icon, base strings
        ├── example/res/        placeholder strings for the example flavor
        └── <flavor>/res/       per-tenant resources (gitignored, e.g. myshop/)
```

## What's in this slice (#134)

- Gradle project with `:app` module + two flavors.
- AndroidManifest with a single `MainActivity` and a deep-link intent
  filter that resolves to each flavor's associated domain.
- WebView pointed at `BuildConfig.START_URL`, with:
  - JavaScript + DOM storage enabled.
  - Persistent cookies via `CookieManager` (flushed on `onPause`)
    so role sessions survive cold starts.
  - System back gesture → WebView history.
  - Off-domain links handed to the OS browser / dialer.
  - Saved instance state restores WebView history on rotation /
    process kill.
- Adaptive-icon placeholder (white "R" disc).
- `Theme.Rusterando` with the red status-bar colour matching the
  site header.

## What lands next (#135)

- `RoleStore` (EncryptedSharedPreferences) for saved Admin / Küche /
  Fahrer credentials.
- Gear FAB → `SettingsActivity` mirroring the iOS settings sheet.
- POST-to-server role switching.

## What lands later (#132 → #133)

- Firebase Cloud Messaging registration + token POST.
- Server-side dispatcher unifying APNs + FCM sends.

## App Links

For the deep-link intent filter to verify automatically (so a
shop.example.com URL opens the app instead of the browser), the
shop has to serve a Digital Asset Links file:

```
https://shop.example.com/.well-known/assetlinks.json
```

Format and signing-cert fingerprint generation are out of scope for
this task; revisit alongside the Play Store release.
