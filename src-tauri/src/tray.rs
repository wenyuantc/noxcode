use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    App, AppHandle, Manager, Runtime,
};

pub fn show_main_window_handle<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let Some(window) = app.get_webview_window("main") else {
        return Err("主窗口不存在".to_string());
    };

    let _ = window.unminimize();
    window
        .show()
        .map_err(|error| format!("显示主窗口失败: {error}"))?;
    window
        .set_focus()
        .map_err(|error| format!("聚焦主窗口失败: {error}"))?;
    Ok(())
}

#[tauri::command]
pub fn show_main_window<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    show_main_window_handle(&app)
}

pub fn create_tray<R: Runtime>(app: &App<R>) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(target_os = "macos")]
    create_app_menu(app)?;
    let show_i = MenuItem::with_id(app, "show", "显示窗口", true, None::<&str>)?;
    let quit_i = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show_i, &quit_i])?;

    let _tray = TrayIconBuilder::new()
        .icon(app.default_window_icon().unwrap().clone())
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            use tauri::tray::{MouseButton, MouseButtonState, TrayIconEvent};
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let _ = show_main_window_handle(tray.app_handle());
            }
        })
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => {
                let _ = show_main_window_handle(app);
            }
            "quit" => {
                let _ =
                    crate::app::lifecycle::request(app, crate::app::lifecycle::ExitIntent::Quit(0));
            }
            _ => {}
        })
        .build(app)?;

    Ok(())
}

#[cfg(target_os = "macos")]
fn create_app_menu<R: Runtime>(app: &App<R>) -> tauri::Result<()> {
    use tauri::menu::{AboutMetadata, PredefinedMenuItem, Submenu};
    let package = app.package_info();
    let quit = MenuItem::with_id(app, "app-quit", "退出", true, Some("CmdOrCtrl+Q"))?;
    let application_menu = Submenu::with_items(
        app,
        &package.name,
        true,
        &[
            &PredefinedMenuItem::about(
                app,
                None,
                Some(AboutMetadata {
                    name: Some(package.name.clone()),
                    version: Some(package.version.to_string()),
                    copyright: app.config().bundle.copyright.clone(),
                    authors: app
                        .config()
                        .bundle
                        .publisher
                        .clone()
                        .map(|publisher| vec![publisher]),
                    ..Default::default()
                }),
            )?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::services(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::hide(app, None)?,
            &PredefinedMenuItem::hide_others(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;
    let menu = Menu::default(app.handle())?;
    // The default macOS application menu uses NSApplication.terminate for Quit,
    // which bypasses ExitRequested. Keep the other default menus intact.
    menu.remove_at(0)?;
    menu.prepend(&application_menu)?;
    app.set_menu(menu)?;
    app.on_menu_event(|app, event| {
        if event.id.as_ref() == "app-quit" {
            let _ = crate::app::lifecycle::request(app, crate::app::lifecycle::ExitIntent::Quit(0));
        }
    });
    Ok(())
}
