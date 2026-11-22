//! Print prepared SVG pages through WebView2 with an explicit Windows paper size.
use crate::{Res, e};
use serde::{Deserialize, Serialize};
use std::mem::size_of;
use tauri::async_runtime::{Sender, channel, spawn_blocking};
use tauri::{WebviewWindow, webview::PlatformWebview};
use webview2_com::{Microsoft::Web::WebView2::Win32::*, PrintCompletedHandler};
use windows::{
    Win32::{
        Foundation::POINT,
        Graphics::Printing::{
            ClosePrinter, EnumPrintersW, GetDefaultPrinterW, GetPrinterW, OpenPrinterW,
            PRINTER_ENUM_CONNECTIONS, PRINTER_ENUM_LOCAL, PRINTER_HANDLE, PRINTER_INFO_4W,
            PRINTER_INFO_5W,
        },
        Storage::Xps::{DC_PAPERSIZE, DeviceCapabilitiesW},
    },
    core::{HSTRING, Interface, PCWSTR, PWSTR},
};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Printers {
    names: Vec<String>,
    default_name: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PrintJob {
    printer: String,
    paper_mm: [f64; 2],
    copies: i32,
    page_ranges: String,
}

/// The spooler fills both structures and strings inside a caller-owned buffer.
fn spooler_buffer(bytes: u32) -> Vec<usize> {
    vec![0; (bytes as usize).div_ceil(size_of::<usize>())]
}

fn printer_names() -> Res<Vec<String>> {
    let flags = PRINTER_ENUM_LOCAL | PRINTER_ENUM_CONNECTIONS;
    let (mut needed, mut count) = (0, 0);
    // Level 4 lists installed queues from the local spooler, without querying
    // each remote printer.
    unsafe {
        let initial = EnumPrintersW(flags, PCWSTR::null(), 4, None, &mut needed, &mut count);
        if needed == 0 {
            return initial.map(|()| Vec::new()).map_err(e);
        }
        let mut buffer = spooler_buffer(needed);
        let bytes = std::slice::from_raw_parts_mut(buffer.as_mut_ptr().cast(), needed as usize);
        EnumPrintersW(
            flags,
            PCWSTR::null(),
            4,
            Some(bytes),
            &mut needed,
            &mut count,
        )
        .map_err(e)?;
        let printers =
            std::slice::from_raw_parts(buffer.as_ptr().cast::<PRINTER_INFO_4W>(), count as usize);
        let mut names = printers
            .iter()
            .filter(|p| !p.pPrinterName.is_null())
            .map(|p| p.pPrinterName.to_string())
            .collect::<Result<Vec<_>, _>>()
            .map_err(e)?;
        names.sort_unstable();
        names.dedup();
        Ok(names)
    }
}

fn default_printer() -> Option<String> {
    let mut length = 0;
    unsafe {
        let _ = GetDefaultPrinterW(None, &mut length);
        if length == 0 {
            return None;
        }
        let mut buffer = vec![0; length as usize];
        let ptr = PWSTR(buffer.as_mut_ptr());
        GetDefaultPrinterW(Some(ptr), &mut length)
            .as_bool()
            .then(|| ptr.to_string().ok())
            .flatten()
    }
}

fn printer_port(printer: &HSTRING) -> windows::core::Result<HSTRING> {
    unsafe {
        let mut handle = PRINTER_HANDLE::default();
        OpenPrinterW(printer, &mut handle, None)?;
        let port = (|| {
            let mut needed = 0;
            let initial = GetPrinterW(handle, 5, None, &mut needed);
            if needed == 0 {
                return initial.map(|()| HSTRING::new());
            }
            let mut buffer = spooler_buffer(needed);
            let bytes = std::slice::from_raw_parts_mut(buffer.as_mut_ptr().cast(), needed as usize);
            GetPrinterW(handle, 5, Some(bytes), &mut needed)?;
            let info = &*buffer.as_ptr().cast::<PRINTER_INFO_5W>();
            if info.pPortName.is_null() {
                return Ok(HSTRING::new());
            }
            Ok(HSTRING::from_wide(info.pPortName.as_wide()))
        })();
        let _ = ClosePrinter(handle);
        port
    }
}

fn supported_paper(printer: &str, [width, height]: [f64; 2]) -> Res<[f64; 2]> {
    if !printer_names()?.iter().any(|name| name == printer) {
        return Err("This printer is no longer installed. Choose another printer.".into());
    }
    let printer = HSTRING::from(printer);
    let port = printer_port(&printer).map_err(e)?;
    let mut expected = [
        (width * 10.0).round() as i32,
        (height * 10.0).round() as i32,
    ];
    expected.sort_unstable();

    // DC_PAPERSIZE supplies portrait dimensions in tenths of a millimetre.
    // Match either order for drivers that list Ledger as a landscape form.
    unsafe {
        let count = DeviceCapabilitiesW(&printer, &port, DC_PAPERSIZE, None, None);
        if count <= 0 {
            return Err("Windows could not read the paper sizes for this printer.".into());
        }
        let mut sizes = vec![POINT::default(); count as usize];
        let data = PWSTR(sizes.as_mut_ptr().cast());
        let count = DeviceCapabilitiesW(&printer, &port, DC_PAPERSIZE, Some(data), None);
        if count <= 0 {
            return Err("Windows could not read the paper sizes for this printer.".into());
        }
        sizes.truncate(count as usize);
        let closest = sizes
            .iter()
            .filter_map(|size| {
                let actual = [size.x.min(size.y), size.x.max(size.y)];
                let short_diff = actual[0].abs_diff(expected[0]);
                let long_diff = actual[1].abs_diff(expected[1]);
                // Some drivers round imperial sizes to a whole millimetre. Use the
                // driver's exact dimensions for the matching form in that case.
                (short_diff <= 5 && long_diff <= 5).then_some((short_diff + long_diff, actual))
            })
            .min_by_key(|(distance, _)| *distance);
        if let Some((_, actual)) = closest {
            return Ok(actual.map(|tenths| f64::from(tenths) / 10.0));
        }
    }
    Err(
        "This printer does not support the selected paper size. Choose a printer that supports it."
            .into(),
    )
}

fn submit(
    webview: PlatformWebview,
    job: PrintJob,
    [paper_width, paper_height]: [f64; 2],
    completed: Sender<Res<()>>,
) -> windows::core::Result<()> {
    let [width, height] = job.paper_mm;
    let orientation = if width > height {
        COREWEBVIEW2_PRINT_ORIENTATION_LANDSCAPE
    } else {
        COREWEBVIEW2_PRINT_ORIENTATION_PORTRAIT
    };
    // Tauri supplies the controller and environment on the WebView2 thread.
    unsafe {
        let printer = webview
            .controller()
            .CoreWebView2()?
            .cast::<ICoreWebView2_16>()?;
        let environment = webview.environment().cast::<ICoreWebView2Environment6>()?;
        let settings = environment.CreatePrintSettings()?;
        let media = settings.cast::<ICoreWebView2PrintSettings2>()?;
        media.SetPrinterName(&HSTRING::from(job.printer))?;
        media.SetMediaSize(COREWEBVIEW2_PRINT_MEDIA_SIZE_CUSTOM)?;
        settings.SetPageWidth(paper_width / 25.4)?;
        settings.SetPageHeight(paper_height / 25.4)?;
        settings.SetOrientation(orientation)?;
        settings.SetScaleFactor(1.0)?;
        settings.SetMarginTop(0.0)?;
        settings.SetMarginBottom(0.0)?;
        settings.SetMarginLeft(0.0)?;
        settings.SetMarginRight(0.0)?;
        settings.SetShouldPrintBackgrounds(true)?;
        settings.SetShouldPrintHeaderAndFooter(false)?;
        media.SetCopies(job.copies)?;
        media.SetPagesPerSide(1)?;
        if !job.page_ranges.is_empty() {
            media.SetPageRanges(&HSTRING::from(job.page_ranges))?;
        }
        let handler = PrintCompletedHandler::create(Box::new(move |error, status| {
            let result = match (error, status) {
                (Err(error), _) => Err(e(error)),
                (_, COREWEBVIEW2_PRINT_STATUS_SUCCEEDED) => Ok(()),
                (_, COREWEBVIEW2_PRINT_STATUS_PRINTER_UNAVAILABLE) => {
                    Err("The selected printer is unavailable.".into())
                }
                _ => Err("Windows could not send the sheets to the selected printer.".into()),
            };
            let _ = completed.try_send(result);
            Ok(())
        }));
        printer.Print(&settings, &handler)
    }
}

#[tauri::command]
pub(super) async fn windows_print_printers() -> Res<Printers> {
    spawn_blocking(|| {
        Ok(Printers {
            names: printer_names()?,
            default_name: default_printer(),
        })
    })
    .await
    .map_err(e)?
}

#[tauri::command]
pub(super) async fn windows_print_document(window: WebviewWindow, job: PrintJob) -> Res<()> {
    if job.printer.is_empty() || job.printer.contains('\0') || !(1..=999).contains(&job.copies) {
        return Err("Choose a printer and enter a number of copies from 1 to 999.".into());
    }
    if !job.paper_mm.iter().all(|&mm| mm.is_finite() && mm > 0.0) {
        return Err("The sheet has an invalid paper size.".into());
    }
    let name = job.printer.clone();
    let paper = job.paper_mm;
    let media = spawn_blocking(move || supported_paper(&name, paper))
        .await
        .map_err(e)??;

    let (sender, mut receiver) = channel(1);
    window
        .with_webview(move |webview| {
            if let Err(error) = submit(webview, job, media, sender.clone()) {
                let _ = sender.try_send(Err(format!("Could not start Windows printing: {error}")));
            }
        })
        .map_err(e)?;
    receiver
        .recv()
        .await
        .ok_or_else(|| "Windows printing ended without a result.".to_owned())?
}
