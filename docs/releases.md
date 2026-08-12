# Vifu release artifacts

The `release-binaries.yml` workflow builds the Vifu desktop archives, Apple
Swift package artifact, Android Maven packages, and signed Android Starter APKs
from a version tag.

## Android Starter signing

Create one Android release keystore for `dev.vifu.android.starter` and retain it
for every release. Store the keystore and passwords in the repository's GitHub
Actions secrets. Configure these names:

- `VIFU_ANDROID_STARTER_KEYSTORE_BASE64`
- `VIFU_ANDROID_STARTER_KEYSTORE_PASSWORD`
- `VIFU_ANDROID_STARTER_KEY_ALIAS`
- `VIFU_ANDROID_STARTER_KEY_PASSWORD`

The workflow checks these values before it publishes the Android Maven
packages. It never writes the keystore into a release artifact. After building,
it runs `apksigner verify` and publishes:

- `vifu-android-starter.apk`
- `vifu-android-starter-baseline.apk`
- `vifu-android-starter-checksums.sha256`

The optimized APK receives an even version code. Its matching baseline APK
receives the next version code so a user can install the compatibility build as
an update. A later release's optimized build always has a higher version code.

The Android Maven publication continues to use its independent Maven Central
credentials and signing key.

## Apple distribution

The GitHub release publishes `VifuMobileFFI.xcframework.zip` for Swift package
consumers. When an iOS Starter beta is distributed, device installation uses
TestFlight and the Apple signing configuration for
`dev.vifu.ios.embedding.demo`.

## Local checks

Before creating a tag, run the focused platform checks from the repository
root:

```bash
swift test

cd integrations/android
./gradlew :vifu-android-core:testDebugUnitTest

cd ../../examples/android-starter
./gradlew testDebugUnitTest assembleDebug -PvifuUseLocalCheckout=true
```

Use a temporary signing certificate to exercise `assembleRelease` locally.
Verify the resulting APK with the `apksigner` from Android SDK build tools. Do
not use the temporary certificate for a published release.
