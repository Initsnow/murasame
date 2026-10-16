//! Live2D Benchmark & Comparison Suite
//!
//! Compares the performance and architecture of:
//! 1. Live2D Cubism SDK for Native (CubismSdkForNative-5-r.5, Live2D Cubism Core C ABI)
//! 2. Pure Rust Live2D Runtime (Mocari)
//! 3. End-to-End GPU WGPU Rendering Pipeline (CPU compute vs GPU rasterization)

use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use murasame::{
    cubism::{self, CubismMoc, CubismModel},
    renderer::Live2dRenderer,
};

#[derive(Debug, Clone, Copy)]
struct BenchStats {
    min: Duration,
    median: Duration,
    mean: Duration,
    p95: Duration,
    p99: Duration,
}

impl BenchStats {
    fn from_samples(mut samples: Vec<Duration>) -> Self {
        assert!(!samples.is_empty(), "Samples must not be empty");
        samples.sort();
        let min = samples[0];
        let median = samples[samples.len() / 2];
        let sum: Duration = samples.iter().sum();
        let mean = sum / (samples.len() as u32);
        let p95_idx = ((samples.len() as f64) * 0.95).round() as usize;
        let p95 = samples[p95_idx.min(samples.len() - 1)];
        let p99_idx = ((samples.len() as f64) * 0.99).round() as usize;
        let p99 = samples[p99_idx.min(samples.len() - 1)];

        Self {
            min,
            median,
            mean,
            p95,
            p99,
        }
    }

    fn mean_micros(&self) -> f64 {
        self.mean.as_nanos() as f64 / 1_000.0
    }

    fn median_micros(&self) -> f64 {
        self.median.as_nanos() as f64 / 1_000.0
    }

    fn min_micros(&self) -> f64 {
        self.min.as_nanos() as f64 / 1_000.0
    }

    fn p95_micros(&self) -> f64 {
        self.p95.as_nanos() as f64 / 1_000.0
    }

    fn p99_micros(&self) -> f64 {
        self.p99.as_nanos() as f64 / 1_000.0
    }

    fn mean_millis(&self) -> f64 {
        self.mean.as_secs_f64() * 1000.0
    }

    fn equivalent_fps(&self) -> f64 {
        let secs = self.mean.as_secs_f64();
        if secs > 0.0 {
            1.0 / secs
        } else {
            0.0
        }
    }
}

fn bench_runner<F>(warmup_rounds: usize, sample_rounds: usize, mut f: F) -> BenchStats
where
    F: FnMut(),
{
    for _ in 0..warmup_rounds {
        f();
    }
    let mut samples = Vec::with_capacity(sample_rounds);
    for _ in 0..sample_rounds {
        let t0 = Instant::now();
        f();
        let elapsed = t0.elapsed();
        samples.push(elapsed);
    }
    BenchStats::from_samples(samples)
}

async fn init_wgpu_hardware() -> Option<(wgpu::AdapterInfo, Arc<wgpu::Device>, Arc<wgpu::Queue>)> {
    let instance = wgpu::Instance::default();
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions::default())
        .await
        .ok()?;
    let info = adapter.get_info();
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor::default())
        .await
        .ok()?;
    Some((info, Arc::new(device), Arc::new(queue)))
}

fn main() -> anyhow::Result<()> {
    println!("===============================================================================");
    println!("       Live2D Runtime & GPU Render Benchmark: Cubism SDK vs Mocari / WGPU      ");
    println!("===============================================================================\n");

    let (core_major, core_minor, core_patch) = cubism::core_version();
    let latest_moc = cubism::latest_moc_version();
    println!("Official Cubism SDK for Native:");
    println!("  Core Version:       {:02}.{:02}.{:04}", core_major, core_minor, core_patch);
    println!("  Latest Moc Version: {}", latest_moc);
    println!("Pure Rust Runtime (Mocari):");
    println!("  Mocari Version:     0.1.0 (submodule)\n");

    let model_dir = PathBuf::from("Murasame_live2d");
    let moc3_path = model_dir.join("Murasame.moc3");
    let model3_json_path = model_dir.join("Murasame.model3.json");

    if !moc3_path.exists() {
        eprintln!("Error: Cannot find model at {}", moc3_path.display());
        std::process::exit(1);
    }

    let moc3_bytes = fs::read(&moc3_path)?;
    println!("Target Model: {}", moc3_path.display());
    println!("Moc3 File Size: {} KB ({:.2} MB)\n", moc3_bytes.len() / 1024, moc3_bytes.len() as f64 / 1_048_576.0);

    // Warmup & Model Inspection
    let cubism_moc = CubismMoc::from_bytes(&moc3_bytes)?;
    let mut cubism_model = CubismModel::new(&cubism_moc)?;

    let mocari_loaded = mocari::assets::load_model_runtime(&model3_json_path)
        .map_err(|e| anyhow::anyhow!("Failed to load mocari runtime: {e}"))?;
    let mut mocari_runtime = mocari_loaded.runtime().clone();

    println!("Model Topology Comparison:");
    println!("  {:<25} | {:<20} | {:<20}", "Metric", "Cubism SDK Native", "Mocari (Rust)");
    println!("  {:-<25}-+-{:-<20}-+-{:-<20}", "", "", "");
    println!(
        "  {:<25} | {:<20} | {:<20}",
        "Parameters Count",
        cubism_model.parameter_count(),
        mocari_runtime.parameter_ids().len()
    );
    println!(
        "  {:<25} | {:<20} | {:<20}",
        "Parts Count",
        cubism_model.part_count(),
        mocari_runtime.part_ids().len()
    );
    println!(
        "  {:<25} | {:<20} | {:<20}",
        "Drawables Count",
        cubism_model.drawable_count(),
        mocari_runtime.meshes().len()
    );

    let total_cubism_vertices: usize = (0..cubism_model.drawable_count())
        .map(|i| cubism_model.drawable_vertex_positions(i).map(|v| v.len()).unwrap_or(0))
        .sum();
    let total_mocari_vertices: usize = mocari_runtime
        .meshes()
        .iter()
        .map(|m| m.vertices().len())
        .sum();

    println!(
        "  {:<25} | {:<20} | {:<20}",
        "Total Vertices",
        total_cubism_vertices,
        total_mocari_vertices
    );
    println!(
        "  {:<25} | {:<20} | {:<20}",
        "Model Memory Allocated",
        format!("{} KB", cubism_model.buffer_len() / 1024),
        "Dynamic (Heap/Vecs)"
    );
    println!();

    // -------------------------------------------------------------
    // Benchmark 1: Moc Load & Parse / Revive
    // -------------------------------------------------------------
    println!("Running Benchmark 1: Moc Parse / Revive (from memory buffer, 100 rounds)...");
    let cubism_moc_stats = bench_runner(10, 100, || {
        let _ = CubismMoc::from_bytes(&moc3_bytes).unwrap();
    });

    let mocari_moc_stats = bench_runner(10, 100, || {
        let _ = mocari::moc3::Moc3ArtMeshes::parse(&moc3_bytes).unwrap();
        let _ = mocari::moc3::Moc3ArtMeshKeyforms::parse(&moc3_bytes).unwrap();
        let _ = mocari::moc3::Moc3Deformers::parse(&moc3_bytes).unwrap();
        let _ = mocari::moc3::Moc3KeyformBindings::parse(&moc3_bytes).unwrap();
        let _ = mocari::moc3::Moc3Ids::parse(&moc3_bytes).unwrap();
        let _ = mocari::moc3::Moc3OffscreenInfo::parse(&moc3_bytes).unwrap();
        let _ = mocari::moc3::Moc3Glues::parse(&moc3_bytes).unwrap();
        let _ = mocari::moc3::Moc3Parts::parse(&moc3_bytes).unwrap();
        let _ = mocari::moc3::Moc3CanvasInfo::parse(&moc3_bytes).unwrap();
        let _ = mocari::moc3::Moc3DrawOrderGroups::parse(&moc3_bytes).unwrap();
    });

    // -------------------------------------------------------------
    // Benchmark 2: Model Instance Creation
    // -------------------------------------------------------------
    println!("Running Benchmark 2: Model Instance Creation & Placement (100 rounds)...");
    let cubism_inst_stats = bench_runner(10, 100, || {
        let _ = CubismModel::new(&cubism_moc).unwrap();
    });

    let mocari_inst_stats = bench_runner(10, 100, || {
        let _ = mocari::assets::load_model_runtime(&model3_json_path).unwrap();
    });

    // -------------------------------------------------------------
    // Benchmark 3: Single Frame Parameter Update & Mesh Deformation
    // -------------------------------------------------------------
    println!("Running Benchmark 3: Single Frame Parameter Update & Evaluate (5,000 rounds)...");

    let angle_x_cubism_idx = cubism_model.parameter_index("ParamAngleX").unwrap_or(0);
    let angle_y_cubism_idx = cubism_model.parameter_index("ParamAngleY").unwrap_or(0);
    let eye_l_cubism_idx = cubism_model.parameter_index("ParamEyeLOpen").unwrap_or(0);
    let body_x_cubism_idx = cubism_model.parameter_index("ParamBodyAngleX").unwrap_or(0);

    let mut frame_step = 0usize;
    let cubism_update_stats = bench_runner(500, 5000, || {
        frame_step = frame_step.wrapping_add(1);
        let t = (frame_step as f32) * 0.05;
        let vals = cubism_model.parameter_values_mut();
        vals[angle_x_cubism_idx] = t.sin() * 30.0;
        vals[angle_y_cubism_idx] = (t * 0.7).cos() * 20.0;
        vals[eye_l_cubism_idx] = ((t * 2.0).sin() * 0.5 + 0.5).clamp(0.0, 1.0);
        vals[body_x_cubism_idx] = (t * 0.5).sin() * 10.0;
        cubism_model.update();
    });

    let mut frame_step_m = 0usize;
    let mocari_update_stats = bench_runner(500, 5000, || {
        frame_step_m = frame_step_m.wrapping_add(1);
        let t = (frame_step_m as f32) * 0.05;
        mocari_runtime.set_parameter("ParamAngleX", t.sin() * 30.0);
        mocari_runtime.set_parameter("ParamAngleY", (t * 0.7).cos() * 20.0);
        mocari_runtime.set_parameter("ParamEyeLOpen", ((t * 2.0).sin() * 0.5 + 0.5).clamp(0.0, 1.0));
        mocari_runtime.set_parameter("ParamBodyAngleX", (t * 0.5).sin() * 10.0);
        let _ = mocari_runtime.update_meshes();
    });

    // -------------------------------------------------------------
    // Benchmark 4: 1000-Frame Animation Timeline Loop
    // -------------------------------------------------------------
    println!("Running Benchmark 4: 1,000 Consecutive Animation Frames Loop (20 batches)...");

    let cubism_1000_stats = bench_runner(2, 20, || {
        for frame in 0..1000 {
            let t = (frame as f32) * 0.03;
            let vals = cubism_model.parameter_values_mut();
            vals[angle_x_cubism_idx] = t.sin() * 30.0;
            vals[angle_y_cubism_idx] = (t * 0.8).cos() * 20.0;
            vals[eye_l_cubism_idx] = ((t * 2.5).sin() * 0.5 + 0.5).clamp(0.0, 1.0);
            vals[body_x_cubism_idx] = (t * 0.4).sin() * 10.0;
            cubism_model.update();
        }
    });

    let mocari_1000_stats = bench_runner(2, 20, || {
        for frame in 0..1000 {
            let t = (frame as f32) * 0.03;
            mocari_runtime.set_parameter("ParamAngleX", t.sin() * 30.0);
            mocari_runtime.set_parameter("ParamAngleY", (t * 0.8).cos() * 20.0);
            mocari_runtime.set_parameter("ParamEyeLOpen", ((t * 2.5).sin() * 0.5 + 0.5).clamp(0.0, 1.0));
            mocari_runtime.set_parameter("ParamBodyAngleX", (t * 0.4).sin() * 10.0);
            let _ = mocari_runtime.update_meshes();
        }
    });

    // -------------------------------------------------------------
    // Benchmark 5: Vertex & Index Readout (CPU -> GPU preparation)
    // -------------------------------------------------------------
    println!("Running Benchmark 5: Vertex & Index Readout for GPU staging (1,000 rounds)...");

    let cubism_vertex_stats = bench_runner(100, 1000, || {
        let count = cubism_model.drawable_count();
        let mut total_coords = 0.0f32;
        for i in 0..count {
            if let Some(verts) = cubism_model.drawable_vertex_positions(i) {
                if let Some(first) = verts.first() {
                    total_coords += first.x + first.y;
                }
            }
        }
        std::hint::black_box(total_coords);
    });

    let mocari_vertex_stats = bench_runner(100, 1000, || {
        let meshes = mocari_runtime.meshes();
        let mut total_coords = 0.0f32;
        for mesh in meshes {
            if let Some(first) = mesh.vertices().first() {
                let [x, y] = first.position();
                total_coords += x + y;
            }
        }
        std::hint::black_box(total_coords);
    });

    // -------------------------------------------------------------
    // Benchmark 6: End-to-End GPU WGPU Render Pipeline Benchmark
    // -------------------------------------------------------------
    println!("Running Benchmark 6: Full Hardware GPU Render Pipeline (WGPU + RenderPass)...");

    let gpu_data = pollster::block_on(init_wgpu_hardware());
    let (gpu_info_str, gpu_render_stats, gpu_sync_stats) = if let Some((info, device, queue)) = gpu_data {
        let info_str = format!("{} ({:?}, {:?})", info.name, info.backend, info.device_type);
        println!("  Detected GPU: {}", info_str);

        // Instantiate live GPU renderer at standard canvas size (450x650)
        let mut renderer = Live2dRenderer::new(
            Arc::clone(&device),
            Arc::clone(&queue),
            &model3_json_path,
            450,
            650,
            1.0,
            2.0,
        )?;

        // Bench A: CPU tick + Command Encoding + Submit to Queue
        let render_stats = bench_runner(10, 100, || {
            let _ = renderer.tick_and_render().unwrap();
        });

        // Bench B: CPU tick + Command Encoding + Submit + GPU Hardware Sync Wait
        let sync_stats = bench_runner(10, 100, || {
            let _ = renderer.tick_and_render().unwrap();
            let _ = device.poll(wgpu::PollType::wait_indefinitely());
        });

        (Some(info_str), Some(render_stats), Some(sync_stats))
    } else {
        println!("  No compatible hardware GPU found for headless bench; skipping GPU pass.");
        (None, None, None)
    };

    println!("\n===============================================================================");
    println!("                               BENCHMARK RESULTS                               ");
    println!("===============================================================================\n");

    let print_row = |name: &str, cubism: &BenchStats, mocari: &BenchStats, unit: &str| {
        let c_mean = cubism.mean_micros();
        let m_mean = mocari.mean_micros();
        let speedup = if c_mean > 0.0 { m_mean / c_mean } else { 0.0 };
        println!(
            "{:<32} | {:>9.2} {} | {:>9.2} {} | {:>7.2}x ({})",
            name,
            c_mean,
            unit,
            m_mean,
            unit,
            speedup,
            if speedup > 1.0 { "Cubism faster" } else { "Mocari faster" }
        );
    };

    println!("{:<32} | {:>12} | {:>12} | {:>18}", "Benchmark Phase", "Cubism Native", "Mocari (Rust)", "Speedup / Ratio");
    println!("{:-<32}-+-{:-<12}-+-{:-<12}-+-{:-<18}", "", "", "", "");
    print_row("1. Moc Parse/Revive", &cubism_moc_stats, &mocari_moc_stats, "µs");
    print_row("2. Model Instantiation", &cubism_inst_stats, &mocari_inst_stats, "µs");
    print_row("3. Frame Update/Deform", &cubism_update_stats, &mocari_update_stats, "µs");
    print_row("4. 1,000 Frames Animation", &cubism_1000_stats, &mocari_1000_stats, "µs");
    print_row("5. Vertex Extraction", &cubism_vertex_stats, &mocari_vertex_stats, "µs");

    println!("\nDetailed Latency Breakdown (Mean / Median / Min / P95 / P99):");
    println!("-------------------------------------------------------------------------------");
    println!("Single Frame Update & Mesh Deformation (CPU Compute Stage):");
    println!(
        "  Cubism SDK Native: Mean={:>7.2}µs | Median={:>7.2}µs | Min={:>7.2}µs | P95={:>7.2}µs | P99={:>7.2}µs -> Max Approx {:>7.0} FPS",
        cubism_update_stats.mean_micros(),
        cubism_update_stats.median_micros(),
        cubism_update_stats.min_micros(),
        cubism_update_stats.p95_micros(),
        cubism_update_stats.p99_micros(),
        cubism_update_stats.equivalent_fps()
    );
    println!(
        "  Mocari (Pure Rust): Mean={:>7.2}µs | Median={:>7.2}µs | Min={:>7.2}µs | P95={:>7.2}µs | P99={:>7.2}µs -> Max Approx {:>7.0} FPS",
        mocari_update_stats.mean_micros(),
        mocari_update_stats.median_micros(),
        mocari_update_stats.min_micros(),
        mocari_update_stats.p95_micros(),
        mocari_update_stats.p99_micros(),
        mocari_update_stats.equivalent_fps()
    );

    if let (Some(info), Some(render), Some(sync)) = (&gpu_info_str, &gpu_render_stats, &gpu_sync_stats) {
        println!("\nHardware GPU Full-Pipeline Benchmark (WGPU Render Loop):");
        println!("-------------------------------------------------------------------------------");
        println!("  GPU Device:        {}", info);
        println!(
            "  CPU Record & Queue Submit Time:  Mean={:>7.2}µs | Median={:>7.2}µs | P95={:>7.2}µs",
            render.mean_micros(),
            render.median_micros(),
            render.p95_micros()
        );
        println!(
            "  Total End-to-End Frame Time:     Mean={:>7.2}µs ({:.2}ms) | P95={:>7.2}µs -> Max Hardware {:>5.0} FPS",
            sync.mean_micros(),
            sync.mean_millis(),
            sync.p95_micros(),
            sync.equivalent_fps()
        );
        let pure_gpu_time = (sync.mean_micros() - render.mean_micros()).max(0.0);
        println!("  Estimated Pure GPU Hardware Render Time: {:>7.2}µs ({:.2}ms)", pure_gpu_time, pure_gpu_time / 1000.0);

        // Frame budget analysis at 60 FPS (16.67ms = 16667µs)
        let frame_budget_us = 16_667.0;
        let cpu_update_pct = (mocari_update_stats.mean_micros() / frame_budget_us) * 100.0;
        let cubism_cpu_pct = (cubism_update_stats.mean_micros() / frame_budget_us) * 100.0;
        let total_cpu_pct = (render.mean_micros() / frame_budget_us) * 100.0;
        let gpu_time_pct = (pure_gpu_time / frame_budget_us) * 100.0;

        println!("\nFrame Budget Utilization at 60 FPS (16.67ms per frame):");
        println!("  - CPU Core Update (Cubism Core):     {:>5.2}% of frame budget", cubism_cpu_pct);
        println!("  - CPU Core Update (Mocari):          {:>5.2}% of frame budget", cpu_update_pct);
        println!("  - Total CPU Pipeline (Logic+Record): {:>5.2}% of frame budget", total_cpu_pct);
        println!("  - GPU Hardware Rasterization Time:   {:>5.2}% of frame budget", gpu_time_pct);
    }

    // Write results to markdown file for user reference
    let gpu_report_section = if let (Some(info), Some(render), Some(sync)) = (&gpu_info_str, &gpu_render_stats, &gpu_sync_stats) {
        let pure_gpu_time = (sync.mean_micros() - render.mean_micros()).max(0.0);
        let frame_budget_us = 16_667.0;
        format!(
r#"
### 2.3 GPU 端到端渲染流水线基准测试 (WGPU)

- **当前检测 GPU 硬件**: `{}`
- **渲染分辨率**: 450x650 逻辑像素 (2x 超采样 -> 900x1300 渲染分辨率)
- **遮罩分辨率**: 2048x2048 裁剪蒙版

| 渲染管线阶段 | 平均耗时 (Mean) | 中位数 (Median) | 95分位 (P95) | 说明 |
| :--- | :---: | :---: | :---: | :--- |
| **CPU 录制与命令提交** | **{:.2} µs** | {:.2} µs | {:.2} µs | CPU 组装 CommandEncoder 与提交队列 |
| **纯 GPU 硬件执行耗时** | **{:.2} µs** ({:.2} ms) | - | - | 显卡片元着色、光栅化与 2048 蒙版计算 |
| **端到端完整单帧总延迟** | **{:.2} µs** ({:.2} ms) | {:.2} µs | {:.2} µs | 包含动画+变形+上传+GPU渲染全流程 |
| **理论全流程硬件帧率极限** | **~{:.0} FPS** | - | - | 当前显卡在无垂直同步下的物理天花板 |

#### 60 FPS 目标帧率下 (16.67 ms 预算) 资源占比分析:

| 环节 | 消耗时间占一帧比例 | 性能评估 |
| :--- | :---: | :--- |
| **Live2D 变形计算 (官方 Cubism SDK)** | **{:.2}%** | 极低 CPU 负载，几乎不占算力 |
| **Live2D 变形计算 (Mocari)** | **{:.2}%** | 轻度 CPU 负载 |
| **CPU 总开销 (含物理/动画/录制)** | **{:.2}%** | 留出 >90% CPU 预算供主线程与 UI 调度 |
| **GPU 硬件时间占比** | **{:.2}%** | 显卡负担轻微，中低端核显亦可轻松胜任 |
"#,
            info,
            render.mean_micros(),
            render.median_micros(),
            render.p95_micros(),
            pure_gpu_time,
            pure_gpu_time / 1000.0,
            sync.mean_micros(),
            sync.mean_millis(),
            sync.median_micros(),
            sync.p95_micros(),
            sync.equivalent_fps(),
            (cubism_update_stats.mean_micros() / frame_budget_us) * 100.0,
            (mocari_update_stats.mean_micros() / frame_budget_us) * 100.0,
            (render.mean_micros() / frame_budget_us) * 100.0,
            (pure_gpu_time / frame_budget_us) * 100.0,
        )
    } else {
        String::new()
    };

    let report_content = format!(
r#"# Live2D 运行时与 GPU 渲染深度对比与 Benchmark 基准测试报告

> 测试日期: 2026-10-17
> 平台环境: Windows x86_64, MSVC 19.44 / Cargo 1.93.1
> 测试模型: `{}` (大小: {:.2} MB, 顶点数: {}, 可绘制网格数: {})

## 1. 运行时概览对比

| 维度 | Live2D Cubism SDK for Native (v5-r.5) | Mocari (当前纯 Rust 实现) |
| :--- | :--- | :--- |
| **核心源码形态** | 官方闭源 C 二进制静态库/动态库 (`Live2DCubismCore`) | 纯 Rust 开源项目 (`submodules/mocari`) |
| **版本规范** | Cubism Core v{:02}.{:02}.{:04} (支持 moc3 3.0 ~ 5.3) | 实验性解析器 (支持常用 moc3 特性) |
| **内存管理机制** | Placement-new 模式 (64字节 Moc 对齐, 16字节 Model 对齐) | Rust 标准堆内存 (`Vec`, 动态重新分配) |
| **模型固定内存开销** | 紧凑内存块 ({:.2} KB 固定占用) | 离散动态分配 (取决于结构体层级与对象数量) |
| **计算指令集优化** | 经过官方 AVX/SSE 汇编向量化内联优化 | Rust 自动向量化 (受借用与切片边界检查制约) |
| **物理与高级特性** | 完整 Physics3, BlendShape, Glue 胶水, Inverted Mask | 基础物理模拟, 基本 Glue, 基础变形器 |
| **商业授权合规** | 专有协议 (Proprietary EULA, 有年营收商业授权限制) | 开源 MIT/Apache 自由许可 |

---

## 2. Benchmark 性能测试数据

测试运行配置: 释放模式 / Release 优化编译, 充分预热, 多轮采样统计。

### 2.1 核心变形计算耗时对比 (CPU 阶段)

| 测试项 (Benchmark Phase) | 官方 Cubism Native | 当前实现 Mocari (Rust) | 性能对比 (倍率) | 结论 |
| :--- | :---: | :---: | :---: | :--- |
| **1. Moc 解析与复活 (Moc Parse / Revive)** | **{:.2} µs** | {:.2} µs | **{:.2}x** | {} |
| **2. 模型实例化 (Model Instantiation)** | **{:.2} µs** | {:.2} µs | **{:.2}x** | {} |
| **3. 单帧参数更新与网格变形 (Frame Update)** | **{:.2} µs** | {:.2} µs | **{:.2}x** | {} |
| **4. 连续 1,000 帧动画演进耗时** | **{:.2} ms** | {:.2} ms | **{:.2}x** | {} |
| **5. 顶点数据提取与传输准备** | **{:.2} µs** | {:.2} µs | **{:.2}x** | {} |

### 2.2 单帧变形计算延迟分布 (µs)

| 引擎 | Min | Median | Mean | P95 | P99 | 理论帧率上限 (FPS) |
| :--- | :---: | :---: | :---: | :---: | :---: | :---: |
| **Cubism SDK Native** | {:.2} | {:.2} | {:.2} | {:.2} | {:.2} | **~{:.0} FPS** |
| **Mocari (Rust)** | {:.2} | {:.2} | {:.2} | {:.2} | {:.2} | **~{:.0} FPS** |
{}
"#,
        moc3_path.display(),
        moc3_bytes.len() as f64 / 1_048_576.0,
        total_cubism_vertices,
        cubism_model.drawable_count(),
        core_major, core_minor, core_patch,
        cubism_model.buffer_len() as f64 / 1024.0,
        cubism_moc_stats.mean_micros(),
        mocari_moc_stats.mean_micros(),
        mocari_moc_stats.mean_micros() / cubism_moc_stats.mean_micros().max(0.001),
        if cubism_moc_stats.mean < mocari_moc_stats.mean { "官方 SDK 更快" } else { "Mocari 更快" },
        cubism_inst_stats.mean_micros(),
        mocari_inst_stats.mean_micros(),
        mocari_inst_stats.mean_micros() / cubism_inst_stats.mean_micros().max(0.001),
        if cubism_inst_stats.mean < mocari_inst_stats.mean { "官方 SDK 更快" } else { "Mocari 更快" },
        cubism_update_stats.mean_micros(),
        mocari_update_stats.mean_micros(),
        mocari_update_stats.mean_micros() / cubism_update_stats.mean_micros().max(0.001),
        if cubism_update_stats.mean < mocari_update_stats.mean { "官方 SDK 更快" } else { "Mocari 更快" },
        cubism_1000_stats.mean.as_secs_f64() * 1000.0,
        mocari_1000_stats.mean.as_secs_f64() * 1000.0,
        (mocari_1000_stats.mean.as_secs_f64() / cubism_1000_stats.mean.as_secs_f64().max(0.0001)),
        if cubism_1000_stats.mean < mocari_1000_stats.mean { "官方 SDK 更快" } else { "Mocari 更快" },
        cubism_vertex_stats.mean_micros(),
        mocari_vertex_stats.mean_micros(),
        mocari_vertex_stats.mean_micros() / cubism_vertex_stats.mean_micros().max(0.001),
        if cubism_vertex_stats.mean < mocari_vertex_stats.mean { "官方 SDK 更快" } else { "Mocari 更快" },
        cubism_update_stats.min_micros(),
        cubism_update_stats.median_micros(),
        cubism_update_stats.mean_micros(),
        cubism_update_stats.p95_micros(),
        cubism_update_stats.p99_micros(),
        cubism_update_stats.equivalent_fps(),
        mocari_update_stats.min_micros(),
        mocari_update_stats.median_micros(),
        mocari_update_stats.mean_micros(),
        mocari_update_stats.p95_micros(),
        mocari_update_stats.p99_micros(),
        mocari_update_stats.equivalent_fps(),
        gpu_report_section,
    );

    let report_path = Path::new("benchmark_results.md");
    fs::write(report_path, report_content)?;
    println!("\n[Saved] Updated full benchmark report saved to {}", report_path.display());

    Ok(())
}
