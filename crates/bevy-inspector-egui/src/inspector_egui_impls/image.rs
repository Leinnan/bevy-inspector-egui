use crate::utils::pretty_type_name;
use bevy_asset::{AssetEvent, AssetId, Assets, Handle, UntypedAssetId};
use bevy_ecs::{
    event::EventReader,
    resource::Resource,
    system::{Commands, In, ResMut},
};
use bevy_image::Image;
use bevy_math::{URect, UVec2};
use bevy_reflect::DynamicTypePath;
use egui::{AtomExt, RichText, Vec2, load::SizedTexture};
use std::{any::Any, collections::HashMap};

use crate::{
    bevy_inspector::errors::{no_world_in_context, show_error},
    reflect_inspector::InspectorUi,
    restricted_world_view::RestrictedWorldView,
};

use super::InspectorPrimitive;

mod image_texture_conversion;

impl InspectorPrimitive for Handle<Image> {
    fn ui(
        &mut self,
        ui: &mut egui::Ui,
        _: &dyn Any,
        id: egui::Id,
        env: InspectorUi<'_, '_>,
    ) -> bool {
        let Some(world) = &mut env.context.world else {
            let immutable_self: &Handle<Image> = self;
            no_world_in_context(ui, immutable_self.reflect_short_type_path());
            return false;
        };
        let scaled_down_textures = world
            .get_resource_mut::<ScaledDownTextures>()
            .unwrap()
            .textures
            .clone();
        let Some(image) = ScaledDownTextures::get_or_load(self, world) else {
            let iimages = world.get_resource_mut::<Assets<Image>>().unwrap();
            let Some(current_image) = iimages.get(self.id()) else {
                ui.label("No image");
                return false;
            };
            ui.label(format!("{:?}", current_image.texture_descriptor));
            ui.label(format!("{:?}", current_image.texture_view_descriptor));
            return false;
        };

        let (asset_server, images) =
            match world.get_two_resources_mut::<bevy_asset::AssetServer, Assets<Image>>() {
                (Ok(a), Ok(b)) => (a, b),
                (a, b) => {
                    if let Err(e) = a {
                        show_error(e, ui, &pretty_type_name::<bevy_asset::AssetServer>());
                    }
                    if let Err(e) = b {
                        show_error(e, ui, &pretty_type_name::<Assets<Image>>());
                    }
                    return false;
                }
            };
        let Some(current_image) = images.get(self.id()) else {
            ui.label("No image by id");
            return false;
        };
        let label = current_image.texture_descriptor.label.map_or(
            self.path()
                .and_then(|f| {
                    Some(
                        f.path()
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_string(),
                    )
                })
                .unwrap_or_default(),
            |label| label.to_string(),
        );
        // get all loaded image paths
        let mut handles = HashMap::new();
        for image in images.iter() {
            if let Some(image_path) = asset_server.get_path(image.0) {
                handles.insert(image_path.to_string(), image.0.clone());
            }
        }

        // first, get the typed search text from a stored egui data value
        let mut selected_path = None;
        let mut image_picker_search_text = String::from("");
        ui.data_mut(|data| {
            image_picker_search_text.clone_from(
                data.get_temp_mut_or_default::<String>(id.with("image_picker_search_text")),
            );
        });
        ui.menu_button(egui::Image::new(image.info).max_height(128.0), |ui| {
            // ui.text_edit_singleline(&mut image_picker_search_text);
            for (path, id) in handles.iter() {
                if !path.contains(&image_picker_search_text) {
                    continue;
                }
                let img_id = scaled_down_textures.iter().find_map(|s| {
                    if s.base_image.id().eq(id) || s.scaled_image.id().eq(id) {
                        Some(s.info.clone())
                    } else {
                        None
                    }
                });
                let clicked = if let Some(id) = img_id {
                    ui.button((
                        egui::Image::new(id).atom_max_height(64.0),
                        RichText::new(path.as_str()),
                    ))
                    .clicked()
                } else {
                    ui.button(path.as_str()).clicked()
                };
                if clicked {
                    selected_path = Some(path.clone());
                }
            }
        });
        ui.add(egui::Label::new(label).selectable(false));
        ui.add(
            egui::Label::new(format!(
                "Size: {}x{}",
                current_image.texture_descriptor.size.width,
                current_image.texture_descriptor.size.height
            ))
            .selectable(false),
        );
        // update the typed search text
        ui.data_mut(|data| {
            *data.get_temp_mut_or_default::<String>(id.with("image_picker_search_text")) =
                image_picker_search_text;
        });

        // if the user selected an option, update the image handle
        if let Some(selected_path) = selected_path {
            *self = asset_server.load(selected_path);
        }

        false
    }

    fn ui_readonly(&self, ui: &mut egui::Ui, _: &dyn Any, _: egui::Id, env: InspectorUi<'_, '_>) {
        let Some(world) = &mut env.context.world else {
            no_world_in_context(ui, self.reflect_short_type_path());
            return;
        };

        let Some(image) = ScaledDownTextures::get_or_load(self, world) else {
            return;
        };
        ui.add(egui::Image::new(image.info).max_height(128.0));
    }
}

#[derive(Debug, Clone)]
pub struct RescaledTextureInfo {
    pub base_image: Handle<Image>,
    #[allow(dead_code)]
    pub scaled_image: Handle<Image>,
    pub info: SizedTexture,
}

#[derive(Debug, Resource)]
pub struct ScaledDownTextures {
    textures: Vec<RescaledTextureInfo>,
    trimmed_textures: HashMap<(URect, UntypedAssetId), RescaledTextureInfo>,
    max_size: UVec2,
}

impl Default for ScaledDownTextures {
    fn default() -> Self {
        Self {
            textures: Vec::new(),
            max_size: UVec2::new(100, 100),
            trimmed_textures: HashMap::new(),
        }
    }
}

impl ScaledDownTextures {
    /// Sets the maximum size for scaled down textures.
    pub fn max_size(&mut self, new_size: impl Into<UVec2>) {
        self.max_size = new_size.into();
    }

    pub fn try_get_image(&self, id: AssetId<Image>) -> Option<RescaledTextureInfo> {
        self.textures
            .iter()
            .find(|info| info.base_image.id().eq(&id) || info.scaled_image.id().eq(&id))
            .cloned()
    }

    pub fn reload_assets(
        mut assets: EventReader<AssetEvent<Image>>,
        mut images: ResMut<Self>,
        mut images_assets: ResMut<Assets<Image>>,
        mut commands: Commands,
    ) {
        for event in assets.read() {
            if let AssetEvent::Modified { id } = event {
                if let Some(r_pos) = images
                    .textures
                    .iter_mut()
                    .position(|info| info.base_image.id().eq(id))
                {
                    eprintln!("REMOVE OLD ONE");
                    images.textures.remove(r_pos);
                    if let Some(handle) = images_assets.get_strong_handle(*id) {
                        commands.run_system_cached_with(Self::load_texture, handle);
                    }
                }
            }
        }
    }

    pub fn load_texture(
        In(img): In<Handle<Image>>,
        mut resized_images: ResMut<Self>,
        mut images: ResMut<Assets<Image>>,
        mut egui_txt: ResMut<bevy_egui::EguiUserTextures>,
    ) {
        if resized_images.try_get_image(img.id()).is_some() {
            return;
        }
        let Some(image) = images.get(img.id()) else {
            return;
        };
        if image
            .texture_descriptor
            .dimension
            .ne(&bevy_render::render_resource::TextureDimension::D2)
        {
            return;
        }
        let max_size = resized_images.max_size;
        if image
            .texture_descriptor
            .dimension
            .eq(&bevy_render::render_resource::TextureDimension::D2)
            && image.size().max_element() <= max_size.min_element()
        {
            let texture_id = egui_txt.add_image(img.clone());
            let new_info = RescaledTextureInfo {
                base_image: img.clone(),
                scaled_image: img.clone(),
                info: SizedTexture {
                    id: texture_id,
                    size: [image.size().x as f32, image.size().y as f32].into(),
                },
            };
            resized_images.textures.push(new_info);
            return;
        }
        let Some((image_gen, is_srgb)) = image_texture_conversion::try_into_dynamic(image) else {
            return;
        };
        let resized = image_gen.resize(
            max_size.x,
            max_size.y,
            image::imageops::FilterType::Triangle,
        );
        let resized = image_texture_conversion::from_dynamic(resized, is_srgb);
        let size = Vec2::new(resized.width() as f32, resized.height() as f32);
        let resized_handle = images.add(resized);
        let texture_id = egui_txt.add_image(resized_handle.clone());
        let new_info = RescaledTextureInfo {
            base_image: img.clone(),
            scaled_image: resized_handle.clone(),
            info: SizedTexture {
                id: texture_id,
                size,
            },
        };
        resized_images.textures.push(new_info);
    }

    /// Gets or loads a scaled down texture for the given image.
    pub fn get_or_load<'a>(
        image: &Handle<Image>,
        world: &mut RestrictedWorldView,
    ) -> Option<RescaledTextureInfo> {
        if let Some(res) = world.get_resource_mut::<Self>().ok().and_then(|resource| {
            resource
                .textures
                .iter()
                .find(|info| {
                    info.base_image.id().eq(&image.id()) || info.scaled_image.id().eq(&image.id())
                })
                .cloned()
        }) {
            return Some(res);
        }
        if image.path().is_none() {
            return None;
        }
        unsafe {
            world
                .world()
                .into_deferred()
                .commands()
                .run_system_cached_with(Self::load_texture, image.clone());
        }
        return None;
    }

    /// Gets or loads a scaled down texture for the given image.
    pub fn get_or_load_trimmed<'a>(
        image: &Handle<Image>,
        rect: URect,
        world: &mut RestrictedWorldView,
    ) -> Option<RescaledTextureInfo> {
        let key = (rect, image.id().untyped());
        if let Some(res) = world
            .get_resource_mut::<Self>()
            .ok()
            .and_then(|resource| resource.trimmed_textures.get(&key).cloned())
        {
            return Some(res);
        }
        let new_texture_info = {
            let (mut egui_user_textures, mut images) =
                match world.get_two_resources_mut::<bevy_egui::EguiUserTextures, Assets<Image>>() {
                    (Ok(a), Ok(b)) => (a, b),
                    _ => return None,
                };
            let original = images.get(image)?;

            let (image_gen, is_srgb) = image_texture_conversion::try_into_dynamic(original)?;
            let resized = image_gen.crop_imm(rect.min.x, rect.min.y, rect.width(), rect.height());
            let resized = image_texture_conversion::from_dynamic(resized, is_srgb);

            let size = Vec2::new(resized.width() as f32, resized.height() as f32);
            let resized_handle = images.add(resized);
            let texture_id = egui_user_textures.add_image(resized_handle.clone());
            RescaledTextureInfo {
                base_image: image.clone(),
                scaled_image: resized_handle.clone(),
                info: SizedTexture {
                    id: texture_id,
                    size,
                },
            }
        };
        if let Ok(mut resource) = world.get_resource_mut::<Self>() {
            resource
                .trimmed_textures
                .insert(key, new_texture_info.clone());
        }
        Some(new_texture_info)
    }
}
