use std::{error::Error, mem, ptr};

use tray_icon::{
    Icon, TrayIconBuilder,
    menu::{Menu, MenuEvent, MenuItem},
};
use windows_sys::Win32::{
    System::Threading::GetCurrentThreadId,
    UI::{
        Shell::ShellExecuteW,
        WindowsAndMessaging::{
            DispatchMessageW, GetMessageW, MB_ICONERROR, MB_OK, MSG, MessageBoxW, PM_NOREMOVE,
            PeekMessageW, PostThreadMessageW, SW_SHOWNORMAL, TranslateMessage, WM_APP, WM_USER,
        },
    },
};

use crate::{
    bridge::{Bootstrap, BridgeOwner},
    discovery::DiscoveryOwner,
    registry::{Lifecycle, Snapshot},
};

const DISCOVERY_UPDATE: u32 = WM_APP + 1;

pub(crate) fn run() -> Result<(), Box<dyn Error>> {
    let mut message = unsafe { mem::zeroed::<MSG>() };
    unsafe {
        PeekMessageW(&mut message, ptr::null_mut(), WM_USER, WM_USER, PM_NOREMOVE);
    }
    let thread_id = unsafe { GetCurrentThreadId() };

    let status = MenuItem::new("Status: Starting discovery", false, None);
    let count = MenuItem::new("Devices found: 0", false, None);
    let open = MenuItem::new("Open MoQTCast", true, None);
    let exit = MenuItem::new("Exit", true, None);
    let open_id = open.id().clone();
    let exit_id = exit.id().clone();
    let menu = Menu::with_items(&[&status, &count, &open, &exit])?;
    let tray = TrayIconBuilder::new()
        .with_tooltip("MoQTCast Lite: discovering online devices")
        .with_menu(Box::new(menu))
        .with_icon(app_icon()?)
        .build()?;

    let (discovery, view) = DiscoveryOwner::start(move || unsafe {
        PostThreadMessageW(thread_id, DISCOVERY_UPDATE, 0, 0);
    });
    let (_bridge, bootstrap) = BridgeOwner::start(view.clone())?;
    let mut shown_revision = 0;

    loop {
        let result = unsafe { GetMessageW(&mut message, ptr::null_mut(), 0, 0) };
        if result == -1 {
            return Err(std::io::Error::last_os_error().into());
        }
        if result == 0 {
            break;
        }
        unsafe {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }

        let snapshot = view.latest();
        if snapshot.revision != shown_revision {
            shown_revision = snapshot.revision;
            update_status(&tray, &status, &count, &snapshot)?;
        }

        while let Ok(event) = MenuEvent::receiver().try_recv() {
            if event.id == open_id {
                if !open_connect_page(&bootstrap) {
                    status.set_text("Status: Browser could not be opened");
                }
            } else if event.id == exit_id {
                discovery.stop();
                return Ok(());
            }
        }
    }

    discovery.stop();
    Ok(())
}

pub(crate) fn show_startup_error() {
    let title = wide("MoQTCast Lite");
    let message = wide("MoQTCast Lite could not start.");
    unsafe {
        MessageBoxW(
            ptr::null_mut(),
            message.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}

fn update_status(
    tray: &tray_icon::TrayIcon,
    status: &MenuItem,
    count: &MenuItem,
    snapshot: &Snapshot,
) -> Result<(), tray_icon::Error> {
    let status_text = match snapshot.lifecycle {
        Lifecycle::Starting => "Status: Starting discovery",
        Lifecycle::Browsing => "Status: Online presence only",
        Lifecycle::Degraded => "Status: Discovery is partially available",
        Lifecycle::Stopping => "Status: Stopping",
        Lifecycle::Stopped => "Status: Stopped",
        Lifecycle::Failed => "Status: Discovery unavailable",
    };
    status.set_text(status_text);
    count.set_text(format!("Devices found: {}", snapshot.devices.len()));
    tray.set_tooltip(Some(format!(
        "MoQTCast Lite: {} online device(s)",
        snapshot.devices.len()
    )))
}

fn open_connect_page(bootstrap: &Bootstrap) -> bool {
    let operation = wide("open");
    let url = wide(bootstrap.open_url());
    let result = unsafe {
        ShellExecuteW(
            ptr::null_mut(),
            operation.as_ptr(),
            url.as_ptr(),
            ptr::null(),
            ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    result as isize > 32
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

fn app_icon() -> Result<Icon, tray_icon::BadIcon> {
    const SIDE: usize = 32;
    let mut rgba = vec![0_u8; SIDE * SIDE * 4];
    for y in 0..SIDE {
        for x in 0..SIDE {
            let offset = (y * SIDE + x) * 4;
            let border = !(2..SIDE - 2).contains(&x) || !(2..SIDE - 2).contains(&y);
            let mark = (8..24).contains(&y)
                && ((7..11).contains(&x)
                    || (21..25).contains(&x)
                    || x.abs_diff(16) == y.saturating_sub(8) / 2);
            let color = if border || mark {
                [242, 247, 255, 255]
            } else {
                [0, 103, 192, 255]
            };
            rgba[offset..offset + 4].copy_from_slice(&color);
        }
    }
    Icon::from_rgba(rgba, SIDE as u32, SIDE as u32)
}
