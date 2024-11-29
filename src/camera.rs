use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use opencv::{
    core::{Mat, Vector},
    imgcodecs,
    prelude::*,
    videoio,
};
use std::time::Duration;
use tokio::sync::mpsc;

pub struct CameraServer {
    frame_sender: mpsc::UnboundedSender<String>,
}

impl CameraServer {
    pub fn new(frame_sender: mpsc::UnboundedSender<String>) -> Self {
        Self { frame_sender }
    }

    fn configure_camera(cap: &mut videoio::VideoCapture) -> Result<()> {
        // Set camera format to MJPEG if available (more efficient)
        let _ = cap.set(
            videoio::CAP_PROP_FOURCC,
            videoio::VideoWriter::fourcc('M', 'J', 'P', 'G')? as f64,
        );

        // Lower resolution for better performance
        cap.set(videoio::CAP_PROP_FRAME_WIDTH, 320.0)?;
        cap.set(videoio::CAP_PROP_FRAME_HEIGHT, 240.0)?;

        // Lower FPS for stability
        cap.set(videoio::CAP_PROP_FPS, 15.0)?;

        // Minimal buffer size to reduce latency
        cap.set(videoio::CAP_PROP_BUFFERSIZE, 1.0)?;

        // No auto exposure or focus (can cause timeouts)
        let _ = cap.set(videoio::CAP_PROP_AUTO_EXPOSURE, 0.0); // Manual exposure
        let _ = cap.set(videoio::CAP_PROP_AUTOFOCUS, 0.0); // Manual focus

        Ok(())
    }

    fn try_open_camera() -> Result<videoio::VideoCapture> {
        let mut cap = videoio::VideoCapture::new(0, videoio::CAP_V4L2)?;

        if !cap.is_opened()? {
            return Err(opencv::Error::new(
                core::StsError,
                "Failed to open camera".to_string(),
            ));
        }

        Self::configure_camera(&mut cap)?;

        // Warm up the camera by reading a few frames
        let mut frame = core::Mat::default();
        for _ in 0..5 {
            cap.read(&mut frame)?;
            std::thread::sleep(Duration::from_millis(100));
        }

        Ok(cap)
    }

    pub async fn start_capture(&self) -> Result<()> {
        let mut cam = Self::try_open_camera()?;
        let mut frame = core::Mat::default();
        let mut consecutive_failures = 0;
        const MAX_FAILURES: i32 = 3;

        loop {
            let read_result = cam.read(&mut frame);
            match read_result {
                Ok(true) => {
                    if frame.empty() {
                        println!("Empty frame received");
                        consecutive_failures += 1;
                    } else {
                        consecutive_failures = 0; // Reset on success

                        // Convert frame to jpg with lower quality for better performance
                        let mut buffer = core::Vector::new();
                        let mut params = core::Vector::new();
                        params.push(opencv::imgcodecs::IMWRITE_JPEG_QUALITY);
                        params.push(60); // Lower quality for better performance

                        if let Ok(_) =
                            opencv::imgcodecs::imencode(".jpg", &frame, &mut buffer, &params)
                        {
                            let frame_data = BASE64.encode(&buffer);
                            if self.frame_sender.send(frame_data).is_err() {
                                println!("Frame receiver disconnected");
                                break;
                            }
                        }
                    }
                }
                Ok(false) | Err(_) => {
                    println!("Failed to read frame");
                    consecutive_failures += 1;
                }
            }

            if consecutive_failures >= MAX_FAILURES {
                println!("Too many consecutive failures, reinitializing camera...");
                // Close current camera explicitly
                drop(cam);

                // Wait before reopening
                tokio::time::sleep(Duration::from_secs(2)).await;

                match Self::try_open_camera() {
                    Ok(new_cam) => {
                        cam = new_cam;
                        consecutive_failures = 0;
                        println!("Camera reinitialized successfully");
                    }
                    Err(e) => {
                        println!("Failed to reinitialize camera: {}", e);
                        break;
                    }
                }
            }

            // Longer delay between frames for stability
            tokio::time::sleep(Duration::from_millis(66)).await; // ~15 FPS
        }

        Ok(())
    }
}
