//! Read-only luminance information for the window's actual Windows output.
//!
//! HDR pixels use a fixed 203-nit scene reference; Windows scRGB uses 80 nits
//! per linear unit. SDR UI white follows the user's separate Windows slider.
//! Neither value is a measured guarantee of the panel's full-frame brightness.
//! API review: 2026-11-11, Microsoft DISPLAYCONFIG_SDR_WHITE_LEVEL and
//! DXGI_OUTPUT_DESC1. Keep the two normalization factors separate in rendering.

#[cfg(windows)]
use windows::{
    Win32::{
        Devices::Display::{
            DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL,
            DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_DEVICE_INFO_HEADER,
            DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_SDR_WHITE_LEVEL,
            DISPLAYCONFIG_SOURCE_DEVICE_NAME, DisplayConfigGetDeviceInfo,
            GetDisplayConfigBufferSizes, QDC_ONLY_ACTIVE_PATHS, QueryDisplayConfig,
        },
        Foundation::{ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS, HWND, LUID},
        Graphics::{
            Dxgi::{
                Common::DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020, CreateDXGIFactory1,
                IDXGIFactory1, IDXGIOutput6,
            },
            Gdi::{MONITOR_DEFAULTTONEAREST, MonitorFromWindow},
        },
        UI::Input::KeyboardAndMouse::GetActiveWindow,
    },
    core::Interface,
};

/// `(HDR peak / 203 nits, SDR UI white / 80 nits)` for a supplied window.
/// Inactive windows remain tied to their own monitor. `None` requests the SDR
/// fallback on query failure, ambiguous cloned outputs or invalid driver data.
#[cfg(windows)]
pub(super) fn display_range_for_window(window: HWND) -> Option<(f32, f32)> {
    if window.is_invalid() {
        return None;
    }
    // SAFETY: all APIs query OS-owned handles; COM wrappers retain interfaces.
    // No output configuration, calibration or system brightness is changed.
    unsafe {
        let target = MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST);
        if target.is_invalid() {
            return None;
        }
        let factory: IDXGIFactory1 = CreateDXGIFactory1().ok()?;
        for adapter_index in 0..128 {
            let Ok(adapter) = factory.EnumAdapters1(adapter_index) else {
                break;
            };
            let Ok(adapter_description) = adapter.GetDesc1() else {
                continue;
            };
            for output_index in 0..256 {
                let Ok(output) = adapter.EnumOutputs(output_index) else {
                    break;
                };
                let Ok(output) = output.cast::<IDXGIOutput6>() else {
                    continue;
                };
                let Ok(description) = output.GetDesc1() else {
                    continue;
                };
                if description.Monitor != target || !description.AttachedToDesktop.as_bool() {
                    continue;
                }
                if description.ColorSpace != DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020 {
                    return Some((1., 1.));
                }
                let white =
                    sdr_white_level(&description.DeviceName, adapter_description.AdapterLuid)?;
                return normalized_range(description.MaxLuminance, white);
            }
        }
    }
    None
}

/// Compatibility fallback for callers without a native window handle. This
/// only uses the calling thread's active window, never another app's foreground
/// window or a guessed primary monitor. Prefer display_range_for_window().
#[cfg(windows)]
pub(super) fn display_range() -> Option<(f32, f32)> {
    // SAFETY: querying the current thread's window does not transfer ownership.
    display_range_for_window(unsafe { GetActiveWindow() })
}

#[cfg(windows)]
fn active_paths() -> Option<Vec<DISPLAYCONFIG_PATH_INFO>> {
    // Display topology may change between sizing and reading. Retry boundedly
    // as documented; never pair path indices with a different topology query.
    for _ in 0..3 {
        let (mut path_count, mut mode_count) = (0, 0);
        // SAFETY: counts are writable; subsequent buffers have those lengths.
        unsafe {
            if GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut path_count, &mut mode_count)
                != ERROR_SUCCESS
                || path_count == 0
                || path_count > 1024
                || mode_count > 4096
            {
                return None;
            }
            let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); path_count as usize];
            // QueryDisplayConfig requires a non-null mode array even when empty.
            let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); (mode_count as usize).max(1)];
            let result = QueryDisplayConfig(
                QDC_ONLY_ACTIVE_PATHS,
                &mut path_count,
                paths.as_mut_ptr(),
                &mut mode_count,
                modes.as_mut_ptr(),
                None,
            );
            if result == ERROR_INSUFFICIENT_BUFFER {
                continue;
            }
            if result != ERROR_SUCCESS || path_count as usize > paths.len() {
                return None;
            }
            paths.truncate(path_count as usize);
            return Some(paths);
        }
    }
    None
}

#[cfg(windows)]
fn sdr_white_level(device_name: &[u16; 32], adapter: LUID) -> Option<u32> {
    let mut matched = None;
    for path in active_paths()? {
        if path.targetInfo.adapterId != adapter || !path.targetInfo.targetAvailable.as_bool() {
            continue;
        }
        let mut source = DISPLAYCONFIG_SOURCE_DEVICE_NAME {
            header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                r#type: DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
                size: std::mem::size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32,
                adapterId: path.sourceInfo.adapterId,
                id: path.sourceInfo.id,
            },
            ..Default::default()
        };
        // SAFETY: header is the first member of the correctly sized request.
        if unsafe { DisplayConfigGetDeviceInfo(&mut source.header) } != ERROR_SUCCESS.0 as i32
            || wide_name(&source.viewGdiDeviceName) != wide_name(device_name)
        {
            continue;
        }
        // A cloned source may drive several physical targets with different
        // white levels. HMONITOR/GDI name alone cannot safely choose between them.
        if matched.is_some() {
            return None;
        }
        matched = Some((path.targetInfo.adapterId, path.targetInfo.id));
    }
    let (adapter_id, id) = matched?;
    let mut white = DISPLAYCONFIG_SDR_WHITE_LEVEL {
        header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
            r#type: DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL,
            size: std::mem::size_of::<DISPLAYCONFIG_SDR_WHITE_LEVEL>() as u32,
            adapterId: adapter_id,
            id,
        },
        ..Default::default()
    };
    // The request uses the target ID, not the GDI source ID queried above.
    (unsafe { DisplayConfigGetDeviceInfo(&mut white.header) } == ERROR_SUCCESS.0 as i32)
        .then_some(white.SDRWhiteLevel)
}

fn wide_name(value: &[u16]) -> &[u16] {
    &value[..value.iter().position(|&c| c == 0).unwrap_or(value.len())]
}

fn normalized_range(peak_nits: f32, sdr_white_level: u32) -> Option<(f32, f32)> {
    let headroom = peak_nits / 203.;
    // DISPLAYCONFIG expresses SDR white in thousandths of an 80-nit unit.
    // This is UI scale only; HDR image pixels still use 203 / 80 for scRGB.
    let ui_white = sdr_white_level as f32 / 1000.;
    (headroom.is_finite() && headroom > 0. && ui_white > 0.).then_some((headroom.max(1.), ui_white))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_headroom_and_user_sdr_white_are_independent() {
        let (peak, white) = normalized_range(1015., 2500).unwrap();
        assert_eq!((peak, white), (5., 2.5));
        assert_eq!(normalized_range(1015., 1000).unwrap().0, peak);
        assert_eq!(normalized_range(203., 2500), Some((1., white)));
    }

    #[test]
    fn invalid_luminance_requests_the_sdr_fallback() {
        for peak in [0., -1., f32::NAN, f32::INFINITY] {
            assert_eq!(normalized_range(peak, 1000), None);
        }
        assert_eq!(normalized_range(1000., 0), None);
    }

    #[test]
    fn device_names_ignore_padding_after_the_terminator() {
        assert_eq!(wide_name(&[65, 66, 0, 99]), wide_name(&[65, 66, 0, 0]));
        assert_ne!(wide_name(&[65, 66, 0]), wide_name(&[65, 67, 0]));
    }
}
