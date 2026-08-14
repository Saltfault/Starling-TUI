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
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
#[cfg(feature = "video")]
use tokio::sync::{broadcast, mpsc};

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

#[cfg(feature = "video")]
pub struct CameraHandle {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

#[cfg(feature = "video")]
impl Drop for CameraHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Start the webcam on a background thread, sending JPEG frames to `tx`.
#[cfg(feature = "video")]
pub fn start_camera(
    camera_index: u32,
    tx: broadcast::Sender<Vec<u8>>,
    evt_tx: mpsc::UnboundedSender<crate::event::AppEvent>,
) -> anyhow::Result<CameraHandle> {
    // nokhwa::Camera is not Send on all platforms (e.g. Windows COM objects),
    // so we create the camera inside the thread rather than moving it in.
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = stop.clone();
    let thread = std::thread::Builder::new().spawn(move || {
        let mut cam = match Camera::new(
            CameraIndex::Index(camera_index),
            RequestedFormat::new::<RgbFormat>(RequestedFormatType::AbsoluteHighestFrameRate),
        ) {
            Ok(c) => c,
            Err(e) => {
                starling::logger::error(&format!("camera init failed: {e}"));
                let _ = evt_tx.send(crate::event::AppEvent::LocalVideoFailed(format!(
                    "camera initialization failed: {e}"
                )));
                return;
            }
        };
        if let Err(e) = cam.open_stream() {
            starling::logger::error(&format!("camera stream open failed: {e}"));
            let _ = evt_tx.send(crate::event::AppEvent::LocalVideoFailed(format!(
                "camera stream failed: {e}"
            )));
            return;
        }

        while !thread_stop.load(Ordering::Relaxed) {
            let Ok(frame) = cam.frame() else {
                let _ = evt_tx.send(crate::event::AppEvent::LocalVideoFailed(
                    "camera stopped producing frames".into(),
                ));
                break;
            };
            if let Ok(img) = frame.decode_image::<RgbFormat>() {
                let mut jpeg = std::io::Cursor::new(Vec::new());
                if DynamicImage::ImageRgb8(img)
                    .write_to(&mut jpeg, ImageFormat::Jpeg)
                    .is_ok()
                {
                    let jpeg = jpeg.into_inner();
                    let _ = evt_tx.send(crate::event::AppEvent::LocalVideoFrame(jpeg.clone()));
                    let _ = tx.send(jpeg);
                }
            }
        }
    })?;
    Ok(CameraHandle {
        stop,
        thread: Some(thread),
    })
}
