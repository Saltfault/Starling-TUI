//! Webcam capture and half-block terminal rendering.

#[cfg(feature = "video")]
use image::{DynamicImage, ImageFormat};
use image::{RgbImage, imageops::FilterType};
#[cfg(feature = "video")]
use nokhwa::{
    Camera,
    pixel_format::RgbFormat,
    utils::{CameraIndex, RequestedFormat, RequestedFormatType},
};
use ratatui::prelude::*;
#[cfg(feature = "video")]
use tokio::sync::mpsc;

/// Convert an RGB image to terminal lines using half-block characters:
/// each cell represents two vertical pixels (top = fg, bottom = bg).
#[allow(dead_code)]
pub fn frame_to_lines(img: &RgbImage, cols: u16, rows: u16) -> Vec<Line<'static>> {
    let small = image::imageops::resize(img, cols as u32, (rows * 2) as u32, FilterType::Triangle);
    (0..rows)
        .map(|cy| {
            Line::from(
                (0..cols)
                    .map(|cx| {
                        let top = small.get_pixel(cx as u32, (cy * 2) as u32);
                        let bot = small.get_pixel(cx as u32, (cy * 2 + 1) as u32);
                        Span::styled(
                            "\u{2580}",
                            Style::new()
                                .fg(Color::Rgb(top[0], top[1], top[2]))
                                .bg(Color::Rgb(bot[0], bot[1], bot[2])),
                        )
                    })
                    .collect::<Vec<_>>(),
            )
        })
        .collect()
}

/// Start the webcam on a background thread, sending JPEG frames to `tx`.
/// Returns the thread handle so the caller can stop it by dropping the
/// channel (which causes `tx.send` to fail and the thread to exit).
#[cfg(feature = "video")]
pub fn start_camera(
    tx: mpsc::UnboundedSender<Vec<u8>>,
) -> anyhow::Result<std::thread::JoinHandle<()>> {
    // nokhwa::Camera is not Send on all platforms (e.g. Windows COM objects),
    // so we create the camera inside the thread rather than moving it in.
    Ok(std::thread::Builder::new().spawn(move || {
        let mut cam = match Camera::new(
            CameraIndex::Index(0),
            RequestedFormat::new::<RgbFormat>(RequestedFormatType::AbsoluteHighestFrameRate),
        ) {
            Ok(c) => c,
            Err(e) => {
                starling::logger::error(&format!("camera init failed: {e}"));
                return;
            }
        };
        if let Err(e) = cam.open_stream() {
            starling::logger::error(&format!("camera stream open failed: {e}"));
            return;
        }

        while let Ok(frame) = cam.frame() {
            if let Ok(img) = frame.decode_image::<RgbFormat>() {
                let mut jpeg = std::io::Cursor::new(Vec::new());
                if DynamicImage::ImageRgb8(img)
                    .write_to(&mut jpeg, ImageFormat::Jpeg)
                    .is_ok()
                {
                    if tx.send(jpeg.into_inner()).is_err() {
                        break;
                    }
                }
            }
        }
    })?)
}
