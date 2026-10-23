# App-lock Android verification

Run only against a disposable Android AVD containing synthetic data. The runner verifies the explicit AVD name, disables device networking, and uses the existing guarded CDP connection. Build an agent APK before running it. `ANDROID_HOME` or `--adb=<executable>` must identify the Android SDK tools.

```sh
node benchmarks/app-lock/android-smoke.mjs \
  --serial=<emulator-serial> --avd=<disposable-avd-name> --adb-port=<owned-adb-port> \
  --cdp-port=<unused-local-port> --output=<artifact-directory> \
  --apk=<agent-apk> --ffmpeg=<ffmpeg-executable> \
  --reset-synthetic-profile --phase=standard
```

`--reset-synthetic-profile` installs the APK, clears this app's data on the selected AVD, grants its notification permission, and seeds a synthetic library with app PIN `123456`. The runner accepts only RisuNest's own terms dialog in that profile. Subsequent phases can reuse the profile by omitting `--reset-synthetic-profile` and `--apk`.

- `--phase=standard`: startup PIN, delayed WebView return frames, gated Back with and without WebView history, recent-app preview on/off, ordinary screenshots, and launch-only return.
- `--phase=timed`: return before and after a real one-minute background interval.
- `--phase=native-auth`: OS authentication cancellation followed by common PIN unlock. If no OS credential is enrolled, the runner temporarily sets synthetic device PIN `1234` and clears it on completion. `--exercise-auth-success` additionally enters that temporary PIN through the real OS prompt and verifies successful unlock.
- `--phase=all` (the default): standard checks plus owned file-picker cancellation and a rapid Home/return transition. `--exercise-native-auth` also runs the native-auth checks.
- `--phase=keyboard`: untouched gate focus, hardware Tab/Enter/Backspace and numeric PIN input, and focus after returning.
- `--phase=settings`: capture the app-lock settings through the real settings UI.
- `--phase=seed`: prepare and capture the startup gate only.

The output directory contains `result.json`, native screenshots, and a five-second busy-return video. A green synthetic marker distinguishes retained app content from native and shared covers. The standard phase checks the complete recorded return for marker exposure. A failed assertion produces a nonzero exit status and the failing stage in `result.json`.

The runner removes its CDP forward and any temporary OS PIN. It leaves the selected emulator running with networking disabled. Restore networking and stop the emulator using the same owner that started the AVD and ADB server. This harness is not imported by the product.
