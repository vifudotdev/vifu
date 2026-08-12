# Vifu release artifacts

The `release-binaries.yml` workflow builds the Vifu desktop archives and Apple
Swift package artifact from a version tag.

The `release-android-starter.yml` workflow builds the Android Starter Demo from
an `android-starter-v*` tag.

## Android Starter Demo

The workflow uses the Android debug signature and publishes:

- `vifu-android-starter.apk`
- `vifu-android-starter-baseline.apk`
- `vifu-android-starter-checksums.sha256`

Each APK uses an independent application ID. A tester can install both APKs and
compare their traces with the same Vifu project.

The Demo workflow does not publish Maven packages. Source builds use the local
Android integration through `-PvifuUseLocalCheckout=true`.

The Demo signature is only for direct device evaluation. If a later Demo uses
another signature, uninstall the old Demo before installation.

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

Use `apksigner verify` to inspect each APK before publication.
