use crate::utils::pretty_type_name;
use bevy_asset::{Assets, Handle};
use bevy_ecs::resource::Resource;
use bevy_image::Image;
use bevy_math::UVec2;
use bevy_reflect::DynamicTypePath;
use egui::{Vec2, load::SizedTexture};
use std::{any::Any, collections::HashMap};

use crate::{
    bevy_inspector::errors::{no_world_in_context, show_error},
    dropdown::DropDownBox,
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

        update_and_show_image(self, world, ui);
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

        // get all loaded image paths
        let mut image_paths = Vec::with_capacity(images.len());
        let mut handles = HashMap::new();
        for image in images.iter() {
            if let Some(image_path) = asset_server.get_path(image.0) {
                image_paths.push(image_path.to_string());
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

        // build and show the dropdown
        let dropdown = DropDownBox::from_iter(
            image_paths.iter(),
            id.with("image_picker"),
            &mut image_picker_search_text,
            |ui, path| {
                let response = ui
                    .selectable_label(
                        self.path()
                            .is_some_and(|p| p.path().as_os_str().to_string_lossy().eq(path)),
                        path,
                    )
                    .on_hover_ui_at_pointer(|ui| {
                        if let Some(id) = handles.get(path) {
                            let s: Option<SizedTexture> =
                                ui.data(|d| d.get_temp(format!("image:{}", id).into()));
                            if let Some(id) = s {
                                ui.image(id);
                            }
                        }
                    });
                if response.clicked() {
                    selected_path = Some(path.to_string());
                }
                response
            },
        )
        .hint_text("Select image asset");
        ui.add_enabled(!image_paths.is_empty(), dropdown)
            .on_disabled_hover_text("No image assets are available");

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

        update_and_show_image(self, world, ui);
    }
}

fn update_and_show_image(
    image: &Handle<Image>,
    world: &mut RestrictedWorldView,
    ui: &mut egui::Ui,
) {
    let Some(image) = ScaledDownTextures::get_or_load(image, world) else {
        return;
    };
    if image.info.size.max_elem() >= 128.0 {
        let _response = egui::CollapsingHeader::new("Texture").show(ui, |ui| ui.image(image.info));
    } else {
        let _response = ui.image(image.info);
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
    max_size: UVec2,
}

impl Default for ScaledDownTextures {
    fn default() -> Self {
        Self {
            textures: Vec::new(),
            max_size: UVec2::new(100, 100),
        }
    }
}

impl ScaledDownTextures {
    /// Sets the maximum size for scaled down textures.
    pub fn max_size(&mut self, new_size: impl Into<UVec2>) {
        self.max_size = new_size.into();
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
                .find(|info| info.base_image.id().eq(&image.id()))
                .cloned()
        }) {
            return Some(res);
        }
        let max_size = world
            .get_resource_mut::<Self>()
            .ok()
            .map(|res| res.max_size.clone())
            .unwrap_or(UVec2::splat(100));
        let new_texture_info = {
            let (mut egui_user_textures, mut images) =
                match world.get_two_resources_mut::<bevy_egui::EguiUserTextures, Assets<Image>>() {
                    (Ok(a), Ok(b)) => (a, b),
                    _ => return None,
                };
            let original = images.get(image)?;

            let (image_gen, is_srgb) = image_texture_conversion::try_into_dynamic(original)?;
            let resized = image_gen.resize(
                max_size.x,
                max_size.y,
                image::imageops::FilterType::Triangle,
            );
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
            resource.textures.push(new_texture_info.clone());
        }
        Some(new_texture_info)
    }
}
