use std::{cmp, sync::Arc};

use winit::{
    application::ApplicationHandler, event::*, event_loop::{ActiveEventLoop, EventLoop}, keyboard::{KeyCode, PhysicalKey}, window::Window
};

use cgmath::prelude::*;
use wgpu::{util::DeviceExt};

mod camera;
mod light;
mod model;
mod render;
mod resources;
mod texture;

#[cfg(target_arch="wasm32")]
use wasm_bindgen::prelude::*;

use crate::{camera::CameraUniform, render::create_render_pipeline};
use crate::light::LightUniform;
use crate::model::Vertex;

#[cfg(target_arch="wasm32")]
#[wasm_bindgen(start)]
pub fn run_web() -> Result<(), wasm_bindgen::JsValue> {
    wgpu::web_sys::console::log_1(&"Rust WASM run_web() starting".into());
    console_error_panic_hook::set_once();
    run().unwrap_throw();

    Ok(())
}

pub fn run() -> anyhow::Result<()> {
    #[cfg(not(target_arch="wasm32"))]
    {
        env_logger::init();
    }
    #[cfg(target_arch="wasm32")]
    {
        console_log::init_with_level(log::Level::Info).unwrap_throw();
    }

    let event_loop = EventLoop::with_user_event().build()?;
    let mut app = App::new(
        #[cfg(target_arch="wasm32")]
        &event_loop,
    );
    event_loop.run_app(&mut app)?;
    Ok(())
}

const NUM_INSTANCES_PER_ROW: u32 = 10;

struct Instance {
    position: cgmath::Vector3<f32>,
    rotation: cgmath::Quaternion<f32>,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct InstanceRaw {
    model: [[f32; 4]; 4]
}

impl InstanceRaw {
    fn desc() -> wgpu::VertexBufferLayout<'static> {
        use std::mem;
        wgpu::VertexBufferLayout {
            array_stride: mem::size_of::<InstanceRaw>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &[
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 0,
                    shader_location: 5,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: mem::size_of::<[f32; 4]>() as wgpu::BufferAddress,
                    shader_location: 6,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: mem::size_of::<[f32; 8]>() as wgpu::BufferAddress,
                    shader_location: 7,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: mem::size_of::<[f32; 12]>() as wgpu::BufferAddress,
                    shader_location: 8,
                }
            ]
        }
    }
}

impl Instance {
    fn to_raw(&self) -> InstanceRaw {
        InstanceRaw { model: (cgmath::Matrix4::from_translation(self.position) * cgmath::Matrix4::from(self.rotation)).into() }
    }
}

const INDICES: &[u16] = &[
    0, 1, 4,
    1, 2, 4,
    2, 3, 4,
];

pub struct App {
    #[cfg(target_arch="wasm32")]
    proxy: Option<winit::event_loop::EventLoopProxy<State>>,
    state: Option<State>,
}

impl App {
    pub fn new(#[cfg(target_arch="wasm32")] event_loop: &EventLoop<State>) -> Self {
        #[cfg(target_arch="wasm32")]
        let proxy = Some(event_loop.create_proxy());
        Self {
            state: None,
            #[cfg(target_arch="wasm32")]
            proxy,
        }
    }
}

impl ApplicationHandler<State> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        #[allow(unused_mut)]
        let mut window_attributes = Window::default_attributes();

        #[cfg(target_arch="wasm32")]
        {
            use wasm_bindgen::JsCast;
            use winit::platform::web::WindowAttributesExtWebSys;

            const CANVAS_ID: &str = "canvas";

            let window = wgpu::web_sys::window().unwrap_throw();
            let document = window.document().unwrap_throw();
            let canvas = document.get_element_by_id(CANVAS_ID).unwrap_throw();
            let html_canvas_element = canvas.unchecked_into();
            window_attributes = window_attributes.with_canvas(Some(html_canvas_element));
        }

        let window = Arc::new(event_loop.create_window(window_attributes).unwrap());

        #[cfg(not(target_arch="wasm32"))]
        {
            self.state = Some(pollster::block_on(State::new(window)).unwrap());
        }

        #[cfg(target_arch="wasm32")]
        {
            if let Some(proxy) = self.proxy.take() {
                wasm_bindgen_futures::spawn_local(async move {
                    assert!(proxy
                        .send_event(
                            State::new(window)
                                .await
                                .expect("Unable to create canvas")
                        )
                        .is_ok())
                });
            }
        }
    }

    #[allow(unused_mut)]
    fn user_event(&mut self, _event_loop: &ActiveEventLoop, mut event: State) {
        #[cfg(target_arch="wasm32")]
        {
            event.window.request_redraw();
            event.resize(
                event.window.inner_size().width,
                event.window.inner_size().height
            );
        }
        self.state = Some(event);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        let state = match &mut self.state {
            Some(canvas) => canvas,
            None => return,
        };

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => state.resize(size.width, size.height),
            WindowEvent::RedrawRequested => {
                state.update();
                match state.render() {
                    Ok(_) => {},
                    Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                        let size = state.window.inner_size();
                        state.resize(size.width, size.height);
                    }
                    Err(e) => {
                        log::error!("Unable to render {}", e);
                    }
                }
            }
            WindowEvent::KeyboardInput { 
                event:
                    KeyEvent {
                        physical_key: PhysicalKey::Code(code),
                        state: key_state,
                        ..
                    }, 
                ..
            } => state.handle_key(event_loop, code, key_state.is_pressed()),
            WindowEvent::CursorMoved {
                position,
                ..
            } => state.handle_mouse_moved(&position),
            _ => {},
        }
    }
}

pub struct State {
    camera: camera::Camera,
    camera_bind_group: wgpu::BindGroup,
    camera_buffer: wgpu::Buffer,
    camera_controller: camera::CameraController,
    camera_uniform: CameraUniform,
    color: wgpu::Color,
    config: wgpu::SurfaceConfiguration,
    depth_texture: texture::Texture,
    device: wgpu::Device,
    diffuse_bind_group: wgpu::BindGroup,
    diffuse_texture: texture::Texture,
    instances: Vec<Instance>,
    instance_buffer: wgpu::Buffer,
    is_surface_configured: bool,
    obj_model: model::Model,
    queue: wgpu::Queue,
    light_uniform: LightUniform,
    light_bind_group: wgpu::BindGroup,
    light_bind_group_layout: wgpu::BindGroupLayout,
    light_buffer: wgpu::Buffer,
    light_render_pipeline: wgpu::RenderPipeline,
    max_dim: u32, // Prevent panics in wgpu due to exceeded texture size
    render_pipeline: wgpu::RenderPipeline,
    surface: wgpu::Surface<'static>,
    window: Arc<Window>,
}

impl State {
    pub async fn new(window: Arc<Window>) -> anyhow::Result<Self> {
        let size = window.inner_size();

        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor{
            #[cfg(not(target_arch="wasm32"))]
            backends: wgpu::Backends::PRIMARY,
            #[cfg(target_arch="wasm32")]
            backends: wgpu::Backends::GL,
            ..Default::default()
        });

        let surface = instance.create_surface(window.clone()).unwrap();

        let adapter = instance.request_adapter(&wgpu::RequestAdapterOptions{
            power_preference: wgpu::PowerPreference::default(),
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }).await?;

        let (device, queue) = adapter.request_device(&wgpu::DeviceDescriptor {
            label: None,
            required_features: wgpu::Features::empty(),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            required_limits: if cfg!(target_arch="wasm32") {
                wgpu::Limits::downlevel_webgl2_defaults()
            } else {
                wgpu::Limits::defaults()
            },
            memory_hints: Default::default(),
            trace: wgpu::Trace::Off,
        }).await?;

        let surface_caps = surface.get_capabilities(&adapter);

        let surface_format = surface_caps.formats.iter()
            .find(|f| f.is_srgb())
            .copied()
            .unwrap_or(surface_caps.formats[0]);

        let max_dim = if cfg!(target_arch = "wasm32") {
            // TODO: Should be able to fetch this dynamically
            // For now hard clamping to the lowest value.
            2048
        } else {
            adapter.limits().max_texture_dimension_2d
        };

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: size.width.min(max_dim),
            height: size.height.min(max_dim),
            present_mode: surface_caps.present_modes[0],
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };

        let diffuse_bytes = include_bytes!("happy-tree.png");
        let diffuse_texture = texture::Texture::from_bytes(&device, &queue, diffuse_bytes, "happy-tree.png")?;

        let texture_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("texture bind group layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { 
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                }
            ]
        });

        let diffuse_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bind group"),
            layout: &texture_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&diffuse_texture.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&diffuse_texture.sampler),
                }
            ]
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor{
            label: Some("shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });

        let camera_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("camera bind group layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer { 
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None, 
                    },
                    count: None,
                }
            ],
        });

        let light_uniform = LightUniform {
            position: [2.0, 2.0, 2.0],
            _padding_1: 0,
            color: [1.0, 1.0, 1.0],
            _padding_2: 0,
        };

        let light_buffer = device.create_buffer_init(
            &wgpu::util::BufferInitDescriptor {
                label: Some("light VB"),
                contents: bytemuck::cast_slice(&[light_uniform]),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            }
        );

        let light_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor{
            label: Some("light bind group layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { 
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None
                    },
                    count: None,
                },
            ]
        });

        let light_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor{
            label: Some("light bind group"),
            layout: &light_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: light_buffer.as_entire_binding(),
                }
            ],
        });

        let render_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor{
                label: Some("render pipeline layout"),
                bind_group_layouts: &[
                    &texture_bind_group_layout,
                    &camera_bind_group_layout,
                    &light_bind_group_layout,
                ],
                push_constant_ranges: &[],
            });

        let render_pipeline = {
            let shader = wgpu::ShaderModuleDescriptor{
                label: Some("Normal Shader"),
                source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into())
            };

            render::create_render_pipeline(
                &device,
                &render_pipeline_layout,
                config.format,
                Some(texture::Texture::DEPTH_FORMAT),
                &[model::ModelVertex::desc(), InstanceRaw::desc()],
                shader,
            )
        };

        let light_render_pipeline = {
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor{
                label: Some("light render pipeline layout"),
                bind_group_layouts: &[&camera_bind_group_layout, &light_bind_group_layout],
                push_constant_ranges: &[],
            });

            let shader = wgpu::ShaderModuleDescriptor{
                label: Some("light shader"),
                source: wgpu::ShaderSource::Wgsl(include_str!("shader_light.wgsl").into())
            };

            create_render_pipeline(
                &device, 
                &layout, 
                config.format, 
                Some(texture::Texture::DEPTH_FORMAT), 
                &[model::ModelVertex::desc()], 
                shader
            )
        };

        let cam = camera::Camera::new(
            (0.0, 1.0, 2.0).into(),
            (0.0, 0.0, 0.0).into(),
            cgmath::Vector3::unit_y(),
            config.width as f32 / config.height as f32,
            45.0,
            0.1,
            100.0,
        );

        let mut camera_uniform = camera::CameraUniform::new();
        camera_uniform.update_view_proj(&cam);

        let camera_buffer = device.create_buffer_init(
            &wgpu::util::BufferInitDescriptor {
                label: Some("camera buffer"),
                contents: bytemuck::cast_slice(&[camera_uniform]),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            }
        );

        let camera_bind_group = device.create_bind_group(
            &wgpu::BindGroupDescriptor {
                label: Some("camera bind group"),
                layout: &camera_bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: camera_buffer.as_entire_binding(),
                    }
                ]
            }
        );

        let camera_controller = camera::CameraController::new(0.2);

        const INSTANCE_DISPLACEMENT: cgmath::Vector3<f32> = cgmath::Vector3::new(NUM_INSTANCES_PER_ROW as f32 * 0.5, 0.0, NUM_INSTANCES_PER_ROW as f32 * 0.5);
        let instances = (0..NUM_INSTANCES_PER_ROW).flat_map(|z| {
            (0..NUM_INSTANCES_PER_ROW).map(move |x| {
                let position = 3.0 * (cgmath::Vector3::new(x as f32, 0.0, z as f32) - INSTANCE_DISPLACEMENT);

                let rotation = if position.is_zero() {
                    cgmath::Quaternion::from_axis_angle(cgmath::Vector3::unit_z(), cgmath::Deg(0.0))
                } else {
                    cgmath::Quaternion::from_axis_angle(position.normalize(), cgmath::Deg(45.0))
                };

                Instance { position, rotation }
            })
        }).collect::<Vec<_>>();

        let instance_data = instances.iter().map(Instance::to_raw).collect::<Vec<_>>();
        let instance_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("instance buffer"),
            contents: bytemuck::cast_slice(&instance_data),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let depth_texture = texture::Texture::new_depth_texture(&device, &config, "depth texture");

        let obj_model = resources::load_model("cube.obj", &device, &queue, &texture_bind_group_layout)
            .await
            .unwrap();

        Ok(Self {
            camera: cam,
            camera_bind_group,
            camera_buffer,
            camera_controller,
            camera_uniform,
            color: wgpu::Color {
                r: 0.1,
                g: 0.2,
                b: 0.3,
                a: 1.0,
            },
            config,
            depth_texture,
            device,
            diffuse_bind_group,
            diffuse_texture,
            instances,
            instance_buffer,
            is_surface_configured: false,
            obj_model,
            queue,
            light_bind_group,
            light_bind_group_layout,
            light_uniform,
            light_buffer,
            light_render_pipeline,
            max_dim,
            render_pipeline: render_pipeline,
            surface,
            window,
        })
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width > 0 && height > 0 {
            self.config.width = cmp::min(width, self.max_dim);
            self.config.height = cmp::min(height, self.max_dim);
            self.surface.configure(&self.device, &self.config);
            self.is_surface_configured = true;
            self.depth_texture = texture::Texture::new_depth_texture(&self.device, &self.config, "depth texture");
        }
    }

    fn handle_mouse_moved(&mut self, position: &winit::dpi::PhysicalPosition<f64>) {
        if self.config.width > 0 && self.config.height > 0 {
            let r = position.x / self.config.width as f64;
            let g = position.y / self.config.height as f64;
            
            self.color = wgpu::Color{
                r: r,
                g: g,
                b: 0.3,
                a: 1.0,
            }
        }
    }

    fn handle_key(&mut self, event_loop: &ActiveEventLoop, code: KeyCode, is_pressed: bool) {
        match (code, is_pressed) {
            (KeyCode::Escape, true) => event_loop.exit(),
            _ => self.camera_controller.handle_key(code, is_pressed),
        }
    }

    fn update(&mut self) {
        self.camera_controller.update_camera(&mut self.camera);
        self.camera_uniform.update_view_proj(&self.camera);
        self.queue.write_buffer(&self.camera_buffer, 0, bytemuck::cast_slice(&[self.camera_uniform]));
        let old_position: cgmath::Vector3<_> = self.light_uniform.position.into();
        self.light_uniform.position = (cgmath::Quaternion::from_axis_angle((0.0, 1.0, 0.0).into(), cgmath::Deg(1.0))
            * old_position)
            .into();
        self.queue.write_buffer(&self.light_buffer, 0, bytemuck::cast_slice(&[self.light_uniform]));
    }

    pub fn render(&mut self) -> Result<(), wgpu::SurfaceError> {
        self.window.request_redraw();

        if !self.is_surface_configured {
            return Ok(());
        }

        let output = self.surface.get_current_texture()?;

        let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());
        
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor{
            label: Some("Render Encoder"),
        });

    
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor{
            label: Some("Render Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment{
                view: &view,
                resolve_target: None,
                ops: wgpu::Operations{
                    load: wgpu::LoadOp::Clear(self.color),
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &self.depth_texture.view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            occlusion_query_set: None,
            timestamp_writes: None,
        });

        render_pass.set_pipeline(&self.render_pipeline);
        render_pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
        use render::DrawModel;
        render_pass.draw_model_instanced(&self.obj_model, 0..self.instances.len() as u32, &self.camera_bind_group, &self.light_bind_group);

        use render::DrawLight;
        render_pass.set_pipeline(&self.light_render_pipeline);
        render_pass.draw_light_model(&self.obj_model, &self.camera_bind_group, &self.light_bind_group);
        drop(render_pass);

        self.queue.submit(std::iter::once(encoder.finish()));
        output.present();

        Ok(())
    }
}