//! Present due video frames on the UI's GPU without owning the playback session.

use crate::playback::{Frame, FrameIdentity};
use eframe::{egui, egui_wgpu};
use moq_video::render;

#[derive(Clone, Copy)]
pub(super) struct Texture {
    pub(super) id: egui::TextureId,
    pub(super) display: (u32, u32),
    uv: [[f32; 2]; 4],
}

impl Texture {
    pub(super) fn mesh(self, rect: egui::Rect) -> egui::Mesh {
        let mut mesh = egui::Mesh::with_texture(self.id);
        for (pos, uv) in [
            rect.left_top(),
            rect.right_top(),
            rect.right_bottom(),
            rect.left_bottom(),
        ]
        .into_iter()
        .zip(self.uv)
        {
            mesh.vertices.push(egui::epaint::Vertex {
                pos,
                uv: egui::pos2(uv[0], uv[1]),
                color: egui::Color32::WHITE,
            });
        }
        mesh.indices.extend([0, 1, 2, 0, 2, 3]);
        mesh
    }
}

pub(super) struct Presenter {
    state: egui_wgpu::RenderState,
    generation: Option<u64>,
    identity: Option<FrameIdentity>,
    renderer: Option<render::Renderer>,
    texture: Option<Texture>,
    size: Option<(u32, u32)>,
    failed: bool,
}

impl Presenter {
    pub(super) fn new(state: egui_wgpu::RenderState) -> Self {
        Self {
            state,
            generation: None,
            identity: None,
            renderer: None,
            texture: None,
            size: None,
            failed: false,
        }
    }
    pub(super) fn identity(&self) -> Option<FrameIdentity> {
        self.identity
    }
    pub(super) fn texture(&self) -> Option<Texture> {
        self.texture
    }

    pub(super) fn update(
        &mut self,
        generation: Option<u64>,
        frame: Option<&Frame>,
    ) -> anyhow::Result<()> {
        if self.generation != generation {
            self.clear();
            self.generation = generation;
            self.failed = false;
        }
        let (Some(generation), Some(frame)) = (generation, frame) else {
            return Ok(());
        };
        if self.failed
            || frame.identity.view_generation != generation
            || self
                .identity
                .is_some_and(|identity| identity >= frame.identity)
        {
            return Ok(());
        }
        if self.identity.is_some_and(|identity| {
            identity.decoder_generation != frame.identity.decoder_generation
        }) {
            self.clear();
        }
        if let Err(error) = self.present(frame) {
            self.clear();
            self.failed = true;
            return Err(error);
        }
        self.identity = Some(frame.identity);
        Ok(())
    }

    fn present(&mut self, frame: &Frame) -> anyhow::Result<()> {
        if self.renderer.is_none() {
            self.renderer = Some(render::Renderer::new(
                &self.state.device,
                &self.state.queue,
                render::Config::new(),
            )?);
        }
        // eframe submits its draw on this same queue after update. A later frame's
        // conversion is ordered after the previous UI draw even when the upstream
        // renderer reuses its output texture. Never queue ahead here.
        let output = self
            .renderer
            .as_mut()
            .expect("initialized renderer")
            .render(&frame.video)?;
        let size = (output.width(), output.height());
        let id = match self.texture {
            Some(texture) => {
                if self.size != Some(size) {
                    self.state
                        .renderer
                        .write()
                        .update_egui_texture_from_wgpu_texture(
                            &self.state.device,
                            &output.create_view(&Default::default()),
                            render::wgpu::FilterMode::Linear,
                            texture.id,
                        );
                }
                texture.id
            }
            None => self.state.renderer.write().register_native_texture(
                &self.state.device,
                &output.create_view(&Default::default()),
                render::wgpu::FilterMode::Linear,
            ),
        };
        self.size = Some(size);
        self.texture = Some(Texture {
            id,
            display: (frame.display_width, frame.display_height),
            uv: [
                frame.source_uv(0.0, 0.0),
                frame.source_uv(1.0, 0.0),
                frame.source_uv(1.0, 1.0),
                frame.source_uv(0.0, 1.0),
            ],
        });
        Ok(())
    }

    fn clear(&mut self) {
        if let Some(texture) = self.texture.take() {
            self.state.renderer.write().free_texture(&texture.id);
        }
        self.renderer = None;
        self.identity = None;
        self.size = None;
    }
}
impl Drop for Presenter {
    fn drop(&mut self) {
        self.clear();
    }
}
#[cfg(test)]
mod tests;
