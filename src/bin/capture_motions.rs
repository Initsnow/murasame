use anyhow::{Context, Result, anyhow};
use image::{GenericImage, Rgba, RgbaImage};
use murasame::renderer::Live2dRenderer;
use std::{
    fs,
    path::PathBuf,
    sync::Arc,
};

async fn init_wgpu_hardware() -> Option<(Arc<wgpu::Device>, Arc<wgpu::Queue>)> {
    let instance = wgpu::Instance::default();
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions::default())
        .await
        .ok()?;
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor::default())
        .await
        .ok()?;
    Some((Arc::new(device), Arc::new(queue)))
}

fn main() -> Result<()> {
    let (device, queue) = pollster::block_on(init_wgpu_hardware())
        .ok_or_else(|| anyhow!("Failed to initialize WGPU hardware"))?;

    let output_dir = PathBuf::from("motion_captures");
    fs::create_dir_all(&output_dir)?;

    let model_path = PathBuf::from("Murasame_live2d/Murasame.model3.json");
    let width = 360;
    let height = 640;
    let scale = 1.0;
    let supersample = 1.0;

    let motions = vec![
        "motion01.motion3.json",
        "motion02.motion3.json",
        "motion03.motion3.json",
        "motion04.motion3.json",
        "motion05.motion3.json",
        "motion06.motion3.json",
        "motion07.motion3.json",
        "motion08.motion3.json",
        "motion09.motion3.json",
        "motion10.motion3.json",
        "motion11.motion3.json",
        "motion12.motion3.json",
    ];

    for motion_name in &motions {
        let motion_path = PathBuf::from("Murasame_live2d/motion").join(motion_name);
        let motion = mocari::motion::load_motion(&motion_path)
            .with_context(|| format!("Failed to load motion {motion_name}"))?;
        let duration = motion.meta().duration();
        println!(
            "Processing {} (duration: {:.2}s, curves: {})...",
            motion_name,
            duration,
            motion.curves().len()
        );

        // Fresh renderer for each motion to avoid lingering blend states
        let mut renderer = Live2dRenderer::new(
            Arc::clone(&device),
            Arc::clone(&queue),
            &model_path,
            width,
            height,
            scale,
            supersample,
        )?;

        renderer.play_motion(&motion_path)?;

        let num_samples = 5;
        let mut sample_images: Vec<(f32, RgbaImage)> = Vec::new();

        let sample_times: Vec<f32> = if duration <= 1.0 {
            vec![0.2, 0.5, 0.8]
        } else {
            (1..=num_samples)
                .map(|i| (i as f32) * (duration / (num_samples as f32 + 0.5)))
                .collect()
        };

        let mut current_time = 0.0f32;
        for &target_time in &sample_times {
            let step = (target_time - current_time).max(0.001);
            renderer.tick_delta(step)?;
            current_time = target_time;

            let frame = renderer.capture_frame()?;
            sample_images.push((target_time, frame));
        }

        // Stitch into a strip
        let strip_w = width * sample_images.len() as u32;
        let strip_h = height;
        let mut strip = RgbaImage::from_pixel(strip_w, strip_h, Rgba([30, 30, 35, 255]));

        for (idx, (t, img)) in sample_images.iter().enumerate() {
            let offset_x = (idx as u32) * width;
            strip.copy_from(img, offset_x, 0)?;

            // Save individual keyframe
            let frame_stem = motion_name.trim_end_matches(".motion3.json");
            let frame_file = output_dir.join(format!("{}_t{:.1}s.png", frame_stem, t));
            img.save(&frame_file)?;
        }

        let stem = motion_name.trim_end_matches(".motion3.json");
        let strip_file = output_dir.join(format!("{}_strip.png", stem));
        strip.save(&strip_file)?;
        println!("Saved strip to {}", strip_file.display());
    }

    println!("\nAll motion captures completed successfully!");
    Ok(())
}
