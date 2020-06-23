use crate::{
    draw::{Draw, RenderCommand},
    pass::{PassDescriptor, TextureAttachment},
    pipeline::PipelineDescriptor,
    render_graph::{Node, ResourceSlotInfo, ResourceSlots},
    render_resource::{BindGroupId, BufferId, RenderResourceBindings, RenderResourceType},
    renderer::RenderContext, ActiveCameras, VisibleEntities,
};
use bevy_asset::{Assets, Handle};
use legion::prelude::*;

pub struct MainPassNode {
    descriptor: PassDescriptor,
    inputs: Vec<ResourceSlotInfo>,
    cameras: Vec<String>,
    color_attachment_input_indices: Vec<Option<usize>>,
    depth_stencil_attachment_input_index: Option<usize>,
}

impl MainPassNode {
    pub fn new(descriptor: PassDescriptor) -> Self {
        let mut inputs = Vec::new();
        let mut color_attachment_input_indices = Vec::new();
        for color_attachment in descriptor.color_attachments.iter() {
            if let TextureAttachment::Input(ref name) = color_attachment.attachment {
                inputs.push(ResourceSlotInfo::new(
                    name.to_string(),
                    RenderResourceType::Texture,
                ));
                color_attachment_input_indices.push(Some(inputs.len() - 1));
            } else {
                color_attachment_input_indices.push(None);
            }
        }

        let mut depth_stencil_attachment_input_index = None;
        if let Some(ref depth_stencil_attachment) = descriptor.depth_stencil_attachment {
            if let TextureAttachment::Input(ref name) = depth_stencil_attachment.attachment {
                inputs.push(ResourceSlotInfo::new(
                    name.to_string(),
                    RenderResourceType::Texture,
                ));
                depth_stencil_attachment_input_index = Some(inputs.len() - 1);
            }
        }

        MainPassNode {
            descriptor,
            inputs,
            cameras: Vec::new(),
            color_attachment_input_indices,
            depth_stencil_attachment_input_index,
        }
    }

    pub fn add_camera(&mut self, camera_name: &str) {
        self.cameras.push(camera_name.to_string());
    }
}

impl Node for MainPassNode {
    fn input(&self) -> &[ResourceSlotInfo] {
        &self.inputs
    }

    fn update(
        &mut self,
        world: &World,
        resources: &Resources,
        render_context: &mut dyn RenderContext,
        input: &ResourceSlots,
        _output: &mut ResourceSlots,
    ) {
        let render_resource_bindings = resources.get::<RenderResourceBindings>().unwrap();
        let pipelines = resources.get::<Assets<PipelineDescriptor>>().unwrap();
        let active_cameras= resources.get::<ActiveCameras>().unwrap();

        for (i, color_attachment) in self.descriptor.color_attachments.iter_mut().enumerate() {
            if let Some(input_index) = self.color_attachment_input_indices[i] {
                color_attachment.attachment =
                    TextureAttachment::Id(input.get(input_index).unwrap().get_texture().unwrap());
            }
        }

        if let Some(input_index) = self.depth_stencil_attachment_input_index {
            self.descriptor
                .depth_stencil_attachment
                .as_mut()
                .unwrap()
                .attachment =
                TextureAttachment::Id(input.get(input_index).unwrap().get_texture().unwrap());
        }

        for camera_name in self.cameras.iter() {
            let visible_entities = if let Some(camera_entity) = active_cameras.get(camera_name) {
                world.get_component::<VisibleEntities>(camera_entity).unwrap()
            } else {
                continue;
            };

            render_context.begin_pass(
                &self.descriptor,
                &render_resource_bindings,
                &mut |render_pass| {
                    let mut draw_state = DrawState::default();
                    for visible_entity in visible_entities.iter() {
                        let draw = if let Some(draw) = world.get_component::<Draw>(visible_entity.entity) {
                            draw
                        } else {
                            continue;
                        };

                        if !draw.is_visible {
                            continue;
                        }
    
                        for render_command in draw.render_commands.iter() {
                            match render_command {
                                RenderCommand::SetPipeline { pipeline } => {
                                    // TODO: Filter pipelines
                                    render_pass.set_pipeline(*pipeline);
                                    let descriptor = pipelines.get(pipeline).unwrap();
                                    draw_state.set_pipeline(*pipeline, descriptor);
                                }
                                RenderCommand::DrawIndexed {
                                    base_vertex,
                                    indices,
                                    instances,
                                } => {
                                    if draw_state.can_draw_indexed() {
                                        render_pass.draw_indexed(
                                            indices.clone(),
                                            *base_vertex,
                                            instances.clone(),
                                        );
                                    } else {
                                        log::info!("Could not draw indexed because the pipeline layout wasn't fully set for pipeline: {:?}", draw_state.pipeline);
                                    }
                                }
                                RenderCommand::SetVertexBuffer {
                                    buffer,
                                    offset,
                                    slot,
                                } => {
                                    render_pass.set_vertex_buffer(*slot, *buffer, *offset);
                                    draw_state.set_vertex_buffer(*slot, *buffer);
                                }
                                RenderCommand::SetIndexBuffer { buffer, offset } => {
                                    render_pass.set_index_buffer(*buffer, *offset);
                                    draw_state.set_index_buffer(*buffer)
                                }
                                RenderCommand::SetBindGroup {
                                    index,
                                    bind_group,
                                    dynamic_uniform_indices,
                                } => {
                                    let pipeline = pipelines.get(&draw_state.pipeline.unwrap()).unwrap();
                                    let layout = pipeline.get_layout().unwrap();
                                    let bind_group_descriptor = layout.get_bind_group(*index).unwrap();
                                    render_pass.set_bind_group(
                                        *index,
                                        bind_group_descriptor.id,
                                        *bind_group,
                                        dynamic_uniform_indices
                                            .as_ref()
                                            .map(|indices| indices.as_slice()),
                                    );
                                    draw_state.set_bind_group(*index, *bind_group);
                                }
                            }
                        }
                    }
                },
            );
        }
    }
}

/// Tracks the current pipeline state to ensure draw calls are valid.
#[derive(Default)]
struct DrawState {
    pipeline: Option<Handle<PipelineDescriptor>>,
    bind_groups: Vec<Option<BindGroupId>>,
    vertex_buffers: Vec<Option<BufferId>>,
    index_buffer: Option<BufferId>,
}

impl DrawState {
    pub fn set_bind_group(&mut self, index: u32, bind_group: BindGroupId) {
        self.bind_groups[index as usize] = Some(bind_group);
    }

    pub fn set_vertex_buffer(&mut self, index: u32, buffer: BufferId) {
        self.vertex_buffers[index as usize] = Some(buffer);
    }

    pub fn set_index_buffer(&mut self, buffer: BufferId) {
        self.index_buffer = Some(buffer);
    }

    pub fn can_draw_indexed(&self) -> bool {
        self.bind_groups.iter().all(|b| b.is_some())
            && self.vertex_buffers.iter().all(|v| v.is_some())
            && self.index_buffer.is_some()
    }

    pub fn set_pipeline(
        &mut self,
        handle: Handle<PipelineDescriptor>,
        descriptor: &PipelineDescriptor,
    ) {
        self.bind_groups.clear();
        self.vertex_buffers.clear();
        self.index_buffer = None;

        self.pipeline = Some(handle);
        let layout = descriptor.get_layout().unwrap();
        self.bind_groups.resize(layout.bind_groups.len(), None);
        self.vertex_buffers
            .resize(layout.vertex_buffer_descriptors.len(), None);
    }
}