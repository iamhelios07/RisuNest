# Local scanner patch

Source: tauri-apps/plugins-workspace, crates.io tauri-plugin-barcode-scanner 2.5.1, MIT/Apache-2.0. Rust API and permissions remain upstream.

Android changes: finish pending invokes before destroying state; invalidate late camera/model callbacks by scan generation; remove preview even before provider readiness; close ML Kit scanner; report bounded camera errors; use the bundled barcode model for first-use offline scanning; make camera hardware optional. These address observed upstream cancel/destroy ordering and asynchronous setup paths.

iOS changes: set up and stop the capture session on a serial queue; invalidate late setup by scan generation so a cancelled scan never attaches a camera; reject a second scan while one is running; reject a missing usage description, missing permission, unsupported requested formats or an unavailable camera instead of failing silently; ignore metadata from an output that is no longer current; size the preview to the app window and remove it with the session; clear the windowed state on teardown.

Keep this patch narrow when updating the plugin. JS bindings remain the official 2.5.1 package.
