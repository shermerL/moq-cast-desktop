use super::*;
use eframe::wgpu;
use moq_tokio::moq_net::Timestamp;
use moq_video::{Color, I420, Size, Surface};
use std::sync::Arc;

fn frame(width: u32, height: u32, generation: u64, decoder: u64, sequence: u64) -> Frame {
    let size = Size::new(width, height);
    let mut data = Vec::with_capacity(size.pixels() as usize * 3 / 2);
    for y in 0..height {
        for x in 0..width {
            data.push(32 + ((x + y * width) % 192) as u8);
        }
    }
    data.extend(vec![128; size.pixels() as usize / 2]);
    Frame::from_video(
        moq_video::Frame::new(
            Surface::I420(I420::new(size, data).unwrap().with_color(Color::Bt601Full)),
            Timestamp::ZERO,
        ),
        FrameIdentity {
            view_generation: generation,
            decoder_generation: decoder,
            sequence,
        },
        None,
        0,
        false,
    )
    .unwrap()
}

#[test]
fn orientation_coordinates_preserve_all_asymmetric_landmarks() {
    let expected = [
        [[0, 1, 2, 3, 4, 5, 6, 7], [1, 0, 3, 2, 5, 4, 7, 6]],
        [[6, 4, 2, 0, 7, 5, 3, 1], [7, 5, 3, 1, 6, 4, 2, 0]],
        [[7, 6, 5, 4, 3, 2, 1, 0], [6, 7, 4, 5, 2, 3, 0, 1]],
        [[1, 3, 5, 7, 0, 2, 4, 6], [0, 2, 4, 6, 1, 3, 5, 7]],
    ];
    for turns in 0..4 {
        for flip in [false, true] {
            let mut f = frame(2, 4, 1, 1, 1);
            f.quarter_turns = turns;
            f.flip = flip;
            let (w, h) = if turns % 2 == 0 { (2, 4) } else { (4, 2) };
            for y in 0..h {
                for x in 0..w {
                    let [u, v] =
                        f.source_uv((x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32);
                    let source = (v * 4.0).floor() as usize * 2 + (u * 2.0).floor() as usize;
                    assert_eq!(
                        source,
                        expected[turns as usize][usize::from(flip)][y * w + x]
                    );
                }
            }
        }
    }
}

async fn gpu() -> egui_wgpu::RenderState {
    let instance = wgpu::Instance::default();
    let adapter = instance
        .request_adapter(&Default::default())
        .await
        .expect("Metal GPU required");
    let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
    let renderer = egui_wgpu::Renderer::new(
        &device,
        wgpu::TextureFormat::Rgba8Unorm,
        egui_wgpu::RendererOptions {
            dithering: false,
            ..Default::default()
        },
    );
    egui_wgpu::RenderState {
        adapter,
        available_adapters: vec![],
        instance,
        device,
        queue,
        target_format: wgpu::TextureFormat::Rgba8Unorm,
        renderer: Arc::new(egui::mutex::RwLock::new(renderer)),
        surface_config: egui_wgpu::SurfaceConfig::LOW_LATENCY,
    }
}

fn draw(state: &egui_wgpu::RenderState, texture: Texture, target: &wgpu::Texture) {
    let rect = egui::Rect::from_min_size(
        egui::Pos2::ZERO,
        egui::vec2(target.width() as f32, target.height() as f32),
    );
    let jobs = [egui::ClippedPrimitive {
        clip_rect: rect,
        primitive: egui::epaint::Primitive::Mesh(texture.mesh(rect)),
    }];
    let screen = egui_wgpu::ScreenDescriptor {
        size_in_pixels: [target.width(), target.height()],
        pixels_per_point: 1.0,
    };
    let mut commands = state.device.create_command_encoder(&Default::default());
    let extra = state.renderer.write().update_buffers(
        &state.device,
        &state.queue,
        &mut commands,
        &jobs,
        &screen,
    );
    let view = target.create_view(&Default::default());
    {
        let pass = commands.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("video regression"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        state
            .renderer
            .read()
            .render(&mut pass.forget_lifetime(), &jobs, &screen);
    }
    state
        .queue
        .submit(extra.into_iter().chain([commands.finish()]));
}

fn target(state: &egui_wgpu::RenderState, width: u32, height: u32) -> wgpu::Texture {
    state.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("video regression output"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: state.target_format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

#[tokio::test]
async fn gpu_presentation_preserves_orientation_color_resize_and_restart() {
    let state = gpu().await;
    let mut p = Presenter::new(state.clone());
    let mut seq = 0;
    for (w, h) in [(2, 4), (8, 6), (2, 4)] {
        for turns in 0..4 {
            for flip in [false, true] {
                seq += 1;
                let mut f = frame(w, h, 1, 1, seq);
                f.quarter_turns = turns;
                f.flip = flip;
                let (ow, oh) = if turns % 2 == 0 { (w, h) } else { (h, w) };
                f.display_width = ow;
                f.display_height = oh;
                p.update(Some(1), Some(&f)).unwrap();
                let id = p.texture().unwrap().id;
                let output = target(&state, ow, oh);
                draw(&state, p.texture().unwrap(), &output);
                let rgba = readback(&state.device, &state.queue, &output).await;
                assert_pixels(&rgba, &f.rgba_reference());
                p.update(Some(1), None).unwrap();
                assert_eq!(
                    p.texture().unwrap().id,
                    id,
                    "no new frame retains the current picture"
                );
                p.update(Some(1), Some(&frame(4, 4, 1, 1, seq - 1)))
                    .unwrap();
                assert_eq!(
                    p.identity(),
                    Some(f.identity),
                    "late frame cannot replace a newer picture"
                );
            }
        }
    }
    for color in [
        Color::Bt601Full,
        Color::Bt601Limited,
        Color::Bt709Full,
        Color::Bt709Limited,
    ] {
        seq += 1;
        let f = Frame::from_video(
            moq_video::Frame::new(
                Surface::I420(
                    I420::new(
                        Size::new(4, 2),
                        vec![64, 128, 192, 224, 64, 128, 192, 224, 90, 90, 200, 200],
                    )
                    .unwrap()
                    .with_color(color),
                ),
                Timestamp::ZERO,
            ),
            FrameIdentity {
                view_generation: 1,
                decoder_generation: 1,
                sequence: seq,
            },
            None,
            0,
            false,
        )
        .unwrap();
        p.update(Some(1), Some(&f)).unwrap();
        let output = target(&state, 4, 2);
        draw(&state, p.texture().unwrap(), &output);
        assert_pixels(
            &readback(&state.device, &state.queue, &output).await,
            &f.rgba_reference(),
        );
    }
    let old = p.texture().unwrap().id;
    p.update(Some(1), Some(&frame(4, 4, 1, 2, 1))).unwrap();
    assert!(
        state.renderer.read().texture(&old).is_none(),
        "decoder replacement frees its registration"
    );
    let old = p.texture().unwrap().id;
    p.update(None, None).unwrap();
    assert!(p.texture().is_none() && p.renderer.is_none() && p.identity().is_none());
    assert!(
        state.renderer.read().texture(&old).is_none(),
        "stop removes the GPU binding"
    );
    p.update(Some(2), Some(&frame(4, 4, 1, 2, 999))).unwrap();
    assert!(
        p.texture().is_none(),
        "old session cannot repopulate a restarted view"
    );
    p.update(Some(2), Some(&frame(4, 4, 2, 1, 1))).unwrap();
    let old = p.texture().unwrap().id;
    p.update(Some(3), None).unwrap();
    assert!(
        state.renderer.read().texture(&old).is_none(),
        "new generation clears before its first frame"
    );
    let final_frame = frame(4, 4, 3, 1, 1);
    p.update(Some(3), Some(&final_frame)).unwrap();
    let output = target(&state, 4, 4);
    draw(&state, p.texture().unwrap(), &output);
    let last = p.texture().unwrap().id;
    drop(p);
    assert!(
        state.renderer.read().texture(&last).is_none(),
        "app drop frees its registration"
    );
    assert_pixels(
        &readback(&state.device, &state.queue, &output).await,
        &final_frame.rgba_reference(),
    );
    state
        .device
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
}

fn assert_pixels(actual: &[[u8; 4]], expected: &[u8]) {
    assert_eq!(actual.len() * 4, expected.len());
    let error = actual
        .iter()
        .flatten()
        .zip(expected)
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap();
    assert!(error <= 2, "GPU/CPU conversion error {error}");
}
async fn readback(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
) -> Vec<[u8; 4]> {
    let (width, height) = (texture.width(), texture.height());
    let row = (width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: (row * height) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);

    let (send, recv) = tokio::sync::oneshot::channel();
    buffer.map_async(wgpu::MapMode::Read, .., |result| {
        let _ = send.send(result);
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("the copy to complete");
    recv.await
        .expect("a mapping result")
        .expect("a mapped buffer");

    let view = buffer.slice(..).get_mapped_range().expect("a mapped range");
    let mut pixels = Vec::with_capacity((width * height) as usize);
    for y in 0..height as usize {
        let start = y * row as usize;
        for x in 0..width as usize {
            let px = &view[start + x * 4..start + x * 4 + 4];
            pixels.push([px[0], px[1], px[2], px[3]]);
        }
    }
    pixels
}

fn native_frames(size: Size) -> Vec<Frame> {
    use moq_video::{Rate, decode, encode};
    let mut config = encode::Config::new(size.width, size.height, Rate::new(30, 1).unwrap());
    config.kind = encode::Kind::Software;
    let mut encoder = encode::Encoder::new(&config).unwrap();
    let catalog = hang::catalog::VideoConfig::new(hang::catalog::H264 {
        inline: true,
        profile: 0x42,
        constraints: 0,
        level: 0x33,
    });
    let mut config = decode::Config::new();
    config.kind = decode::Kind::Named("videotoolbox".into());
    let mut decoder = decode::Decoder::new(&catalog, &config).unwrap();
    let mut out = Vec::new();
    for i in 0..8u64 {
        let color = [40 + i as u8 * 8, 120, 200 - i as u8 * 8, 255];
        let rgba: Vec<u8> = color
            .into_iter()
            .cycle()
            .take(size.pixels() as usize * 4)
            .collect();
        let input = moq_video::Frame::new(
            Surface::rgba(&rgba, size).unwrap(),
            Timestamp::from_micros(i * 33_333).unwrap(),
        );
        encoder.cut().unwrap();
        for packet in encoder.encode(&input).unwrap() {
            out.extend(
                decoder
                    .decode(&packet.payload, packet.timestamp, true)
                    .unwrap(),
            );
        }
    }
    for packet in encoder.finish().unwrap() {
        out.extend(
            decoder
                .decode(&packet.payload, packet.timestamp, true)
                .unwrap(),
        );
    }
    out.extend(decoder.flush().unwrap());
    assert_eq!(out.len(), 8);
    out.into_iter()
        .enumerate()
        .map(|(i, video)| {
            assert!(
                matches!(video.surface, Surface::PixelBuffer(_)),
                "native decoder output required"
            );
            Frame::from_video(
                video,
                FrameIdentity {
                    view_generation: 1,
                    decoder_generation: 1,
                    sequence: i as u64 + 1,
                },
                None,
                0,
                false,
            )
            .unwrap()
        })
        .collect()
}

#[tokio::test]
async fn gpu_native_decode_survives_decoder_drop_and_frame_reuse() {
    let state = gpu().await;
    let mut p = Presenter::new(state.clone());
    for (decoder, size) in [(1, Size::new(64, 48)), (2, Size::new(128, 72))] {
        let frames = native_frames(size);
        let output = target(&state, size.width, size.height);
        let mut reference_config = render::Config::new();
        reference_config.zero_copy = false;
        reference_config.usage = wgpu::TextureUsages::COPY_SRC;
        let mut reference =
            render::Renderer::new(&state.device, &state.queue, reference_config).unwrap();
        for (i, mut f) in frames.into_iter().enumerate() {
            f.identity.decoder_generation = decoder;
            f.identity.sequence = i as u64 + 1;
            let cpu = f.rgba_reference();
            let reference_texture = reference.render(&f.video).unwrap();
            let expected = readback(&state.device, &state.queue, &reference_texture).await;
            p.update(Some(1), Some(&f)).unwrap();
            draw(&state, p.texture().unwrap(), &output);
            drop(f); // The UI does not keep decoded buffers alive until GPU completion.
            let actual = readback(&state.device, &state.queue, &output).await;
            assert_eq!(
                actual, expected,
                "native import must match uploaded decoded samples exactly"
            );
            // CPU integer conversion and the upstream floating-point shader are
            // different algorithms. Report their delta separately from import correctness.
            let error = actual
                .iter()
                .flatten()
                .zip(&cpu)
                .map(|(a, b)| a.abs_diff(*b))
                .max()
                .unwrap();
            println!("native decoded frame {i}, size={size:?}, CPU-reference max delta={error}");
        }
    }
    p.update(None, None).unwrap();
}

/// Run explicitly in release mode; measures presentation, not decode/network or scanout latency.
#[tokio::test]
#[ignore = "release GPU benchmark; run with --release --ignored --nocapture"]
async fn benchmark_gpu_presentation() {
    use std::time::Instant;
    if cfg!(debug_assertions) {
        panic!("run the benchmark with --release");
    }
    let state = gpu().await;
    println!("GPU_BENCH adapter={:?}", state.adapter.get_info());
    println!(
        "GPU_BENCH mode,width,height,round,samples,submit_p50_us,submit_p95_us,complete_p50_us,complete_p95_us"
    );
    for size in [Size::new(1920, 1080), Size::new(3840, 2160)] {
        let mut frames = native_frames(size);
        let output = target(&state, size.width, size.height);
        for round in 0..3 {
            for gpu_mode in if round % 2 == 0 {
                [false, true]
            } else {
                [true, false]
            } {
                let mut p = Presenter::new(state.clone());
                let context = egui::Context::default();
                // No text is painted here; discard the initial empty font-atlas delta.
                let _ = context.tex_manager().write().take_delta();
                let mut old: Option<egui::TextureHandle> = None;
                let mut submit = Vec::new();
                let mut complete = Vec::new();
                for i in 0..80 {
                    let f = &mut frames[i % 8];
                    f.identity.sequence = i as u64 + 1;
                    let start = Instant::now();
                    let texture = if gpu_mode {
                        p.update(Some(1), Some(f)).unwrap();
                        p.texture().unwrap()
                    } else {
                        let rgba = f.rgba_reference();
                        let image = egui::ColorImage::from_rgba_unmultiplied(
                            [size.width as usize, size.height as usize],
                            &rgba,
                        );
                        if let Some(old) = old.as_mut() {
                            old.set(image, egui::TextureOptions::LINEAR);
                        } else {
                            old = Some(context.load_texture(
                                "baseline",
                                image,
                                egui::TextureOptions::LINEAR,
                            ));
                        }
                        let delta = context.tex_manager().write().take_delta();
                        for (id, images) in &delta.set {
                            for image in images {
                                state.renderer.write().update_texture(
                                    &state.device,
                                    &state.queue,
                                    *id,
                                    image,
                                );
                            }
                        }
                        Texture {
                            id: old.as_ref().unwrap().id(),
                            display: (size.width, size.height),
                            uv: [[0., 0.], [1., 0.], [1., 1.], [0., 1.]],
                        }
                    };
                    draw(&state, texture, &output);
                    let cpu = start.elapsed().as_secs_f64() * 1e6;
                    state
                        .device
                        .poll(wgpu::PollType::wait_indefinitely())
                        .unwrap();
                    if i >= 20 {
                        submit.push(cpu);
                        complete.push(start.elapsed().as_secs_f64() * 1e6);
                    }
                }
                submit.sort_by(f64::total_cmp);
                complete.sort_by(f64::total_cmp);
                println!(
                    "GPU_BENCH {},{},{},{},60,{:.1},{:.1},{:.1},{:.1}",
                    if gpu_mode { "gpu" } else { "cpu" },
                    size.width,
                    size.height,
                    round,
                    submit[29],
                    submit[56],
                    complete[29],
                    complete[56]
                );
                if let Some(old) = old {
                    state.renderer.write().free_texture(&old.id());
                }
            }
        }
    }
}

#[tokio::test]
async fn gpu_presenter_handles_window_surface_formats_without_double_gamma() {
    let mut state = gpu().await;
    for format in [
        wgpu::TextureFormat::Bgra8Unorm,
        wgpu::TextureFormat::Bgra8UnormSrgb,
    ] {
        state.target_format = format;
        state.renderer = Arc::new(egui::mutex::RwLock::new(egui_wgpu::Renderer::new(
            &state.device,
            format,
            egui_wgpu::RendererOptions {
                dithering: false,
                ..Default::default()
            },
        )));
        let mut p = Presenter::new(state.clone());
        let size = Size::new(4, 4);
        let video = moq_video::Frame::new(
            Surface::rgba(&[40, 120, 200, 255].repeat(16), size).unwrap(),
            Timestamp::ZERO,
        );
        let f = Frame::from_video(
            video,
            FrameIdentity {
                view_generation: 1,
                decoder_generation: 1,
                sequence: 1,
            },
            None,
            0,
            false,
        )
        .unwrap();
        p.update(Some(1), Some(&f)).unwrap();
        let output = target(&state, 4, 4);
        draw(&state, p.texture().unwrap(), &output);
        let mut rgba = readback(&state.device, &state.queue, &output).await;
        for pixel in &mut rgba {
            pixel.swap(0, 2);
        }
        assert_pixels(&rgba, &f.rgba_reference());
    }
}

#[tokio::test]
async fn gpu_reused_output_preserves_queued_draws_without_cpu_waits() {
    let state = gpu().await;
    let mut presenter = Presenter::new(state.clone());
    let mut outputs = Vec::new();
    for (i, rgba) in [[255, 0, 0, 255], [0, 0, 255, 255], [0, 255, 0, 255]]
        .into_iter()
        .enumerate()
    {
        let mut f = frame(4, 4, 1, 1, i as u64 + 1);
        f.video = moq_video::Frame::new(
            Surface::rgba(&rgba.repeat(16), Size::new(4, 4)).unwrap(),
            Timestamp::ZERO,
        );
        presenter.update(Some(1), Some(&f)).unwrap();
        let output = target(&state, 4, 4);
        draw(&state, presenter.texture().unwrap(), &output);
        outputs.push((output, f.rgba_reference()));
    }
    // All conversions and draws are already submitted before the first readback.
    drop(presenter);
    for (output, expected) in outputs {
        assert_pixels(
            &readback(&state.device, &state.queue, &output).await,
            &expected,
        );
    }
}
