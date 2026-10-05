use notify_rust::{Notification, NotificationResponse};
use tauri::{Manager, WebviewWindow};

trait ExistingWindow {
    fn show(&self);
    fn unminimize(&self);
    fn focus(&self);
}

impl ExistingWindow for WebviewWindow {
    fn show(&self) {
        let _ = WebviewWindow::show(self);
    }
    fn unminimize(&self) {
        let _ = WebviewWindow::unminimize(self);
    }
    fn focus(&self) {
        let _ = self.set_focus();
    }
}

fn activate(response: &NotificationResponse, window: Option<&impl ExistingWindow>) {
    if !matches!(response, NotificationResponse::Default) {
        return;
    }
    if let Some(window) = window {
        window.show();
        window.unminimize();
        window.focus();
    }
}

#[tauri::command]
pub(crate) fn desktop_notify(window: WebviewWindow, body: String) -> Result<(), String> {
    crate::native_log::logged_without_detail(
        "desktop_notify",
        (|| {
            if window.label() != "main" {
                return Err("Notifications are limited to the main window".into());
            }
            let app = window.app_handle().clone();
            let mut notification = Notification::new();
            notification.summary("RisuNest").body(&body).auto_icon();
            #[cfg(target_os = "linux")]
            notification.action("default", "");
            #[cfg(windows)]
            {
                let exe = tauri::utils::platform::current_exe()
                    .map_err(|_| "Notification executable unavailable")?;
                let directory = exe.parent().ok_or("Notification executable unavailable")?;
                // Preserve the plugin's installed-app identity and development fallback.
                if !directory.ends_with("target/debug") && !directory.ends_with("target/release") {
                    notification.app_id(&app.config().identifier);
                }
            }
            #[cfg(target_os = "macos")]
            let _ = notify_rust::set_application(if tauri::is_dev() {
                "com.apple.Terminal"
            } else {
                &app.config().identifier
            });

            // One native response owner per delivered notification, independent of renderer reloads.
            std::thread::Builder::new()
                .name("notification-response".into())
                .spawn(move || {
                    let result = notification.show().and_then(|handle| {
                        handle.wait_for_response(move |response: &NotificationResponse| {
                            if !matches!(response, NotificationResponse::Default) {
                                return;
                            }
                            let response = response.clone();
                            let target = app.clone();
                            let _ = app.run_on_main_thread(move || {
                                activate(&response, target.get_webview_window("main").as_ref());
                            });
                        })
                    });
                    if result.is_err() {
                        crate::nlog!("warn", "Desktop notification response unavailable");
                    }
                })
                .map_err(|_| "Notification worker unavailable".to_string())?;
            Ok(())
        })(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    struct Window(RefCell<Vec<&'static str>>);
    impl ExistingWindow for Window {
        fn show(&self) {
            self.0.borrow_mut().push("show");
        }
        fn unminimize(&self) {
            self.0.borrow_mut().push("unminimize");
        }
        fn focus(&self) {
            self.0.borrow_mut().push("focus");
        }
    }

    #[test]
    fn only_explicit_body_activation_restores_the_existing_window() {
        let window = Window::default();
        for reason in [
            notify_rust::CloseReason::Dismissed,
            notify_rust::CloseReason::Expired,
            notify_rust::CloseReason::CloseAction,
            notify_rust::CloseReason::Other(0),
        ] {
            activate(&NotificationResponse::Closed(reason), Some(&window));
        }
        activate(
            &NotificationResponse::Action("unknown".into()),
            Some(&window),
        );
        activate(
            &NotificationResponse::Reply("synthetic".into()),
            Some(&window),
        );
        assert!(window.0.borrow().is_empty());
        activate(&NotificationResponse::Default, None::<&Window>);
        for _ in 0..2 {
            activate(&NotificationResponse::Default, Some(&window));
        }
        assert_eq!(
            *window.0.borrow(),
            ["show", "unminimize", "focus", "show", "unminimize", "focus"]
        );
    }
}
