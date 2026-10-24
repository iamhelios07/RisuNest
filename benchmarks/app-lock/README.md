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
- `--phase=native-auth`: OS authentication cancellation followed by common PIN unlock. If no OS credential is enrolled, the runner temporarily sets synthetic device PIN `1234` and clears it on completion. `--exercise-auth-success` additionally enters that temporary PIN through the real OS prompt and verifies successful unlock. With `--exercise-auth-success` and a PIN enrolled by this run, Android 10 (API 29) also verifies stable cancellation, normal locking after authentication ends, and Home/Overview while a public native authentication request is pending. The pending credential task is restored through its Overview card. The OS may cancel that request after leaving; the runner records the actual outcome and requires it to stay pending until the caller returns, with no new departure episode. A direct native-command success does not itself change the shared PIN screen. Android 9 and below do not support native unlocking.
- `--phase=all` (the default): standard checks plus owned file-picker cancellation, a rapid Home/return observation, and a sustained picker departure on Android 12L and later. Picker time is excluded from the away interval. Rapid departures may remain unobserved while the task is still visible, and Android versions below 12L cannot observe task visibility during the picker. An observed departure must still lock. `--exercise-native-auth` also runs the native-auth checks.
- `--phase=review`: requires an Android 14 (API 34) synthetic AVD. Runs native select continuation, camera and notification permission deny/grant, rapid and sustained Home during a permission prompt, native-cover accessibility, and native authentication with Overview departure recorded before return. Focused subsets are `review-focus`, `review-permission`, `review-accessibility`, `review-departure`, `review-permission-home`, `review-picker`, `review-preview`, and `review-auth`. Use `--exercise-auth-success` to include the temporary OS PIN success case.
- `--phase=review-presentation`: Android 14 native frame acknowledgement and same-epoch policy changes. Requires an overlapping old request to be rejected, the current unlocked frame to succeed, and identical or theme-only policies to preserve a pending acknowledgement.
- `--phase=keyboard`: untouched gate focus, hardware Tab/Enter/Backspace and numeric PIN input, and focus after returning.
- `--phase=settings`: capture the app-lock settings through the real settings UI.
- `--phase=seed`: prepare and capture the startup gate only.

Permission prompt time is excluded from the away interval. Rapid Home/return may finish before Android reports the activity as stopped; the runner records that observation and requires any detected departure to lock. The sustained permission and picker cases first confirm that the owned task has stopped, then require its departure timestamp to precede the return request.

Preview checks identify the RisuNest task through the native Overview app menu, capture a positive content frame before leaving, and compare marker pixels only inside that task snapshot.

The output directory contains `result.json`, native screenshots, and a twelve-second busy-return video. A green synthetic marker distinguishes retained app content from native and shared covers. The standard phase first observes Home, then checks the complete untrimmed return recording for marker exposure. It verifies that the actual native return happened inside the stalled JavaScript interval and that recording continued through native handover to the PIN screen. A failed assertion produces a nonzero exit status and the failing stage in `result.json`.

The runner removes its CDP forward and any temporary OS PIN. It leaves the selected emulator running with networking disabled. Restore networking and stop the emulator using the same owner that started the AVD and ADB server. This harness is not imported by the product.
